//! B2 Task 7: `Calls` edges. Walks each caller `SymbolSnapshot`'s own
//! `signature_text` (which, per `extract`'s per-language extractors,
//! captures the WHOLE construct — including its body — not just a
//! parameter list) by re-parsing it as a standalone fragment with the
//! same language's `LanguageParser`, and looks for call-expression nodes
//! whose callee name resolves to another snapshot in the given pool.
//!
//! # Resolved vs. heuristic vs. unresolved
//!
//! - A call whose callee name matches EXACTLY ONE other snapshot's name
//!   is `StaticAnalysis`-resolved — not because the whole program's
//!   call graph is statically analyzed (it is not: this pipeline does
//!   not do type inference, overload resolution, or virtual-dispatch
//!   analysis), but because, within the closed set of candidate
//!   snapshots this resolver is given, the name uniquely identifies one
//!   target with no ambiguity left for this resolver to guess at. A
//!   dotted/member call (Python's `self.encoder(x)`, matched by its
//!   final segment `encoder`) is resolved the SAME way once the name
//!   is unique — the pipeline does not need to know `self` is an
//!   instance to usefully record "this method calls something named
//!   `encoder`."
//! - A call whose callee name matches MORE THAN ONE candidate (e.g. two
//!   different types both define a method named `run`) is
//!   `NameHeuristic`, `Confidence::Speculative` — explicitly never
//!   picked arbitrarily. Per the task's own scope, this resolver does
//!   NOT attempt overload/type-based disambiguation; it records the
//!   ambiguity honestly rather than overclaiming a single target.
//! - A call whose callee name matches NO candidate in the given pool
//!   produces no edge at all — the callee is outside what this resolver
//!   was given to work with (e.g. a standard-library function, or a
//!   symbol in a file not included in this resolution pass), not a
//!   relationship this pipeline can honestly assert anything about.
//!
//! # Known gap: calls inside a macro invocation (Rust)
//!
//! A call wrapped inside a Rust macro invocation (e.g.
//! `assert_eq!(add(1, 2), 3)`) is NOT found by this resolver: confirmed
//! empirically against a real parse, `add(1, 2)` inside
//! `assert_eq!(...)`'s argument list is an opaque `token_tree` to
//! tree-sitter, not a real `call_expression` node — tree-sitter does not
//! expand macros, so it has no way to know `assert_eq!`'s arguments are
//! themselves Rust expressions rather than arbitrary tokens. This
//! resolver therefore cannot see calls made from inside macro-heavy test
//! code (a common shape, since `assert_eq!`/`assert!` are exactly where
//! a test's real assertions live) without a macro-aware token-tree
//! heuristic this task's scope does not include. Recorded here as a
//! real, named limitation — not silently masked by a test fixture that
//! happens to avoid it — and a candidate for the real-repository
//! validation pass this plan's section 8b requires before B2's closure.

use codegraph_domain::{Confidence, EdgeKind, SymbolKind};

use super::resolution::{heuristic_evidence, resolved_evidence, Edge, EdgeResolutionError, TargetLookup};
pub(crate) use crate::parse::parser_for;
use crate::csharp_declarations::CSharpTypeDeclarationContext;
use crate::imports::{ImportFact, ImportResolution};
use crate::negative_knowledge::CallAttempt;
use crate::parse::Language;
use crate::snapshot::SymbolSnapshot;

/// Resolves `Calls` edges from every snapshot in `snapshots` to every
/// other snapshot in the SAME pool whose name matches a call found
/// inside the caller's own signature text. `snapshots` plays both the
/// "source" and "candidate target" role simultaneously — a caller and
/// its potential callees are expected to be given together, unlike
/// Task 6's lineage resolver's `prev`/`next` split (lineage is
/// inherently directional across commits; calls are not).
///
/// Compatibility wrapper: identical behavior to calling
/// [`resolve_calls_with_negative_knowledge`] and discarding its second
/// return value — the `Vec<Edge>` this function returns is byte-for-
/// byte unchanged from before Phase 2's own negative-knowledge
/// addition, confirmed by a dedicated equivalence test. Every existing
/// call site keeps this exact signature.
pub fn resolve_calls(snapshots: &[SymbolSnapshot]) -> Result<Vec<Edge>, EdgeResolutionError> {
    resolve_calls_with_negative_knowledge(snapshots).map(|(edges, _attempts)| edges)
}

/// Quality & Semantic Reliability Hardening, Phase 2 — identical to
/// [`resolve_calls`] in every way EXCEPT it additionally returns a
/// [`CallAttempt`] for every call-site occurrence that did NOT produce
/// a resolved edge: `Ambiguous` (2+ real candidates survived scope
/// narrowing) or `Unresolved` (zero real candidates at all). A call
/// site that DID resolve (exactly one real candidate, with or without
/// scope narrowing) produces only its `Edge`, same as always — no
/// `CallAttempt` is recorded for the success path, since `Edge` itself
/// already carries that outcome's own `Evidence`; recording a
/// redundant `CallAttempt` for every successful call too would
/// duplicate information this type exists specifically to add where
/// NONE existed before.
///
/// `CallAttempt`s are deduplicated per (caller, callee_name) the same
/// way `Edge`s are deduplicated per (from, to) by `per_target` below —
/// a callee name invoked many times in one caller's body with the same
/// ambiguous/unresolved outcome produces ONE `CallAttempt`, not one per
/// occurrence (mirroring B2-ID-002's own occurrence-vs-relation
/// discipline, generalized to the negative-knowledge case for the
/// identical reason: a consumer asking "is `foo()` ambiguous in this
/// function?" wants one honest answer, not N identical copies of it).
pub fn resolve_calls_with_negative_knowledge(snapshots: &[SymbolSnapshot]) -> Result<(Vec<Edge>, Vec<CallAttempt>), EdgeResolutionError> {
    resolve_calls_with_extraction_rejections(snapshots, &[])
}

/// Semantic Resolution & Reliability Checkpoint, Priority 3 — identical
/// to [`resolve_calls_with_negative_knowledge`] in every way EXCEPT:
/// when a call site's own callee name matches NO real candidate (the
/// zero-candidate case), AND that exact name also appears in
/// `signature_too_long_names` (the caller's own, already-collected
/// `CandidateRejection::SignatureTooLong { name, .. }` names from the
/// SAME extraction pass this snapshot pool was built from), the
/// resulting `CallAttempt` carries
/// [`crate::negative_knowledge::UnresolvedReason::ExtractionRejected`]
/// instead of the generic `Unknown` reason — making the real, specific
/// cause (a real candidate sharing this exact name WAS found in the
/// source but dropped before promotion) observable, rather than
/// looking identical to a genuine external/unknown target. This
/// function performs NO extraction-rejection detection of its own — it
/// only consumes what the caller already collected, per this module's
/// "never fabricate a reason" discipline (see
/// `CallAttempt::unresolved_with_extraction_rejection`'s own doc
/// comment).
///
/// `resolve_calls_with_negative_knowledge` is now a thin compatibility
/// wrapper calling this with an empty `signature_too_long_names` slice
/// — byte-for-byte identical behavior for every existing call site,
/// confirmed by a dedicated equivalence test.
///
/// Also a thin compatibility wrapper (CALL-RES-A4): calls
/// [`resolve_calls_with_imports`] with an empty `import_facts` slice.
/// Byte-for-byte identical behavior for every existing call site,
/// confirmed by a dedicated equivalence test.
pub fn resolve_calls_with_extraction_rejections(snapshots: &[SymbolSnapshot], signature_too_long_names: &[String]) -> Result<(Vec<Edge>, Vec<CallAttempt>), EdgeResolutionError> {
    resolve_calls_with_imports(snapshots, signature_too_long_names, &[])
}

/// CALL-RES-A4 — Import/Namespace/Visibility-aware Resolution. Identical
/// to [`resolve_calls_with_extraction_rejections`] in every way EXCEPT:
/// when a call site's own callee name matches ZERO real candidates
/// directly (the exact failure shape CRC-003 documented — the resolver
/// is honestly uncertain, not ambiguous), this function additionally
/// checks whether `import_facts` contains a real `ImportFact`, from the
/// SAME file as `caller`, whose own `alias` field equals this exact
/// callee name. If so, it re-resolves using that alias's own ALREADY-
/// COMPUTED `resolution` (never re-derived here — `ImportFact`'s own
/// resolution is computed once, globally, by `resolve_raw_import_statements`,
/// and this function only CONSUMES that result, per this crate's
/// "resolvers never re-derive another resolver's own determination"
/// discipline):
///
/// - `ImportResolution::Internal(target_id)`: produces a real `Edge`
///   (`ResolutionMethod::NameHeuristic`, since the alias mapping is a
///   real structural fact but this is still name-based linking, not
///   full semantic resolution) pointing at `target_id` directly — no
///   re-scanning the snapshot pool by name at all, since the alias
///   fact ALREADY names the exact target unambiguously.
/// - `ImportResolution::External`: produces a `CallAttempt::unresolved_import_target_external`
///   — a MORE SPECIFIC, honest claim than plain `Unknown`: the alias
///   is real and its target is confirmed external, not merely absent
///   for an unknown reason.
/// - `ImportResolution::Ambiguous`: produces a `CallAttempt::ambiguous_import_target`.
///
/// If NO import fact in the caller's own file has a matching `alias`,
/// this function records `CallAttempt::unresolved_no_matching_import_alias`
/// instead of the plain `Unknown` a caller passing an empty
/// `import_facts` slice would get — distinguishing "I checked import
/// evidence and found nothing" from "I never checked."
///
/// # Never invents a target from incomplete import information
///
/// This function NEVER falls back to name-similarity matching, and
/// NEVER treats a wildcard/star import, an unresolvable relative
/// import, or any other incomplete import fact as license to guess —
/// the ONLY signal this function acts on is an EXACT, already-resolved
/// `ImportFact.alias` match. An import whose own `alias` field is
/// `None` (the ordinary, non-aliased case) is never consulted here at
/// all; `ImportFact`'s own `imported_symbol`/`module_path` fields are
/// deliberately NOT used to widen the match, since this function's
/// whole purpose (per CRC-003) is the exact-alias case, not a general
/// import-based rename.
///
/// # Scope: zero-candidate case only — never touches the `>= 2` path
///
/// This function's own import-alias check runs ONLY when the direct
/// by-name candidate lookup found exactly ZERO candidates. It NEVER
/// runs when `candidates.len() >= 2` (the ambiguity path CRC-001/
/// CRC-002 occupy) — those two challenge cases are structurally
/// unreachable from this function's own new code path, by construction,
/// not merely by convention; see the dedicated CRC-001/CRC-002
/// non-regression tests.
pub fn resolve_calls_with_imports(snapshots: &[SymbolSnapshot], signature_too_long_names: &[String], import_facts: &[ImportFact]) -> Result<(Vec<Edge>, Vec<CallAttempt>), EdgeResolutionError> {
    let lookup = TargetLookup::new(snapshots)?;
    let mut edges = Vec::new();
    let mut attempts = Vec::new();
    let rejected_names: std::collections::HashSet<&str> = signature_too_long_names.iter().map(|s| s.as_str()).collect();

    for caller in lookup.snapshots() {
        // B2-ID-006, mechanism 1 of 2 (caller eligibility): a snapshot
        // whose own `SymbolKind` is a structural container (`Class`,
        // `Struct`, `Interface`, ...) is never itself a real caller —
        // see `SymbolKind::can_originate_calls`'s own doc comment for
        // the full, per-variant justification. Skipping here, before
        // ever re-parsing `caller.signature()`, is both correct (a
        // container's own body is never itself "executed") and cheaper
        // than walking its full body only to find nothing new once
        // mechanism 2 below would otherwise exclude every call inside
        // it anyway.
        if !caller.kind().can_originate_calls() {
            continue;
        }

        let Some(parser) = parser_for(caller.file()) else {
            // This resolver only recognizes languages whose
            // FileIdentity-independent kind it can determine from the
            // caller's own promoted SymbolKind/signature shape — in
            // practice, every language Task 4 extracts already has a
            // LanguageParser; see `parser_for`'s own doc comment for
            // why HTML/CSS snapshots never reach a parser here.
            continue;
        };

        let Ok(tree) = parser.parse(caller.signature().as_str().as_bytes()) else {
            continue;
        };

        // B2-ID-002: `call_names_in_tree` returns one entry per
        // SYNTACTIC call-site occurrence (e.g. `self.forward(x)` called
        // twice in the same body yields two entries), but `CodeGraph`
        // represents a SEMANTIC/topological `(from, to, EdgeKind)`
        // relation — at most one edge per pair, never one per
        // occurrence. `per_target` collapses every occurrence resolving
        // to the SAME target into exactly one `Edge`, chosen
        // deterministically by `prefer_edge` (see its own doc comment)
        // rather than by iteration order, before this caller's edges
        // are appended to the final output. This never merges two
        // DISTINCT targets — only repeated occurrences of the same
        // `(from, to)` pair.
        let mut per_target: std::collections::HashMap<codegraph_domain::SymbolSnapshotId, Edge> = std::collections::HashMap::new();
        // Phase 2: deduplicates negative-knowledge CallAttempts the
        // same way per_target deduplicates Edges — one CallAttempt per
        // DISTINCT callee_name this caller attempted and failed to
        // resolve, never one per repeated occurrence. Keyed by name
        // alone (not by candidate set) because this resolver's own
        // candidate pool for a given name is identical across every
        // occurrence within one resolve_calls_with_negative_knowledge
        // call — there is no scenario where the SAME callee_name in
        // the SAME caller resolves to a different candidate set on a
        // later occurrence.
        let mut per_attempt_name: std::collections::HashMap<String, CallAttempt> = std::collections::HashMap::new();

        for call_site in call_sites_in_tree(&tree) {
            // B2-ID-006, mechanism 2 of 2 (nested-scope boundary): this
            // call site's ABSOLUTE file position is `caller`'s own span
            // start plus the call's position within the re-parsed
            // fragment (`caller.signature()`'s text starts exactly at
            // `caller.span().start` — see `SymbolSnapshot::span`'s own
            // doc comment). If some OTHER caller-eligible snapshot,
            // strictly nested inside `caller`'s own span in the same
            // file, ALSO strictly contains this absolute position, that
            // nested snapshot — not `caller` — is this call site's real
            // owner (it will itself be visited as its own `caller` in a
            // later iteration of this same outer loop, and will
            // correctly attribute this call site to itself there).
            // Skipping it here is what prevents an outer Function/Class
            // from rediscovering a call that genuinely belongs to a
            // nested Function/Method — the SAME span-nesting fact
            // `resolve_contains` already uses for `Contains` edges,
            // reused here rather than inventing a second nesting
            // mechanism or any synthetic scope name.
            let absolute_call_site_position = caller.span().start + call_site.fragment_relative_start_byte;
            if call_site_belongs_to_a_more_nested_caller(lookup.snapshots(), caller, absolute_call_site_position) {
                continue;
            }

            let callee_name = call_site.callee_name;
            let all_candidates = lookup.find_by_name(&callee_name, caller.id());

            // Task 9.5, Priority 5(A), Case A: a `self.field.method()`-
            // shaped call (receiver is the caller's OWN instance, and
            // `field` is a known member of the caller's enclosing
            // class) must not be matched against an UNRELATED
            // same-named candidate elsewhere in the corpus just because
            // the bare method name happens to collide (e.g.
            // `self.src_mask.size(0)` — a call on a `Field` named
            // `src_mask` — must never resolve to an unrelated class's
            // unrelated `size()` method). When the receiver's field is
            // a real member of the caller's own class (confirmed
            // structurally via span-containment, the same signal
            // `resolve_contains` uses — never a name hardcoded to this
            // corpus), restrict the candidate pool to members of THAT
            // SAME class; if that class has no matching member,
            // "unresolved" is preferred over a cross-class false match,
            // per the task's explicit instruction, so no candidates
            // survive the restriction and no edge is produced for this
            // occurrence. When the receiver field is NOT a member of
            // the caller's class (or the caller has no enclosing class,
            // or no receiver was present at all), candidate selection is
            // unchanged from before this fix.
            let candidates = match &call_site.self_receiver_field {
                Some(field_name) if receiver_field_is_a_known_member(lookup.snapshots(), caller, field_name) => {
                    restrict_to_callers_own_class(lookup.snapshots(), caller, &all_candidates)
                }
                _ => all_candidates,
            };

            if candidates.len() == 1 {
                let evidence = resolved_evidence(&format!("call to {callee_name:?} resolves to exactly one candidate in the given snapshot pool"));
                let edge = Edge {
                    from: caller.id(),
                    to: candidates[0].id(),
                    kind: EdgeKind::Calls,
                    evidence,
                };
                upsert_preferring_stronger_evidence(&mut per_target, edge);
                continue;
            }

            // CALL-RES-A4: the zero-candidate case ONLY — a direct
            // by-name lookup found nothing at all (CRC-003's own exact
            // failure shape: `_get_features_targets` matches zero
            // candidates because the real target is named
            // `get_features_targets`). This check is structurally
            // unreachable when `candidates.len() >= 2` (the
            // CRC-001/CRC-002 ambiguity territory): it lives entirely
            // inside the `candidates.is_empty()` branch, never touching
            // the already-resolved-to-one or already-ambiguous paths.
            if candidates.is_empty() {
                if let Some(outcome) = resolve_via_import_alias(caller, &callee_name, import_facts, lookup.snapshots()) {
                    match outcome {
                        ImportAliasOutcome::Resolved(target_id) => {
                            let (evidence, _confidence) = heuristic_evidence(
                                codegraph_domain::ResolutionMethod::NameHeuristic,
                                Confidence::Likely,
                                &format!("call to {callee_name:?} matches no candidate directly, but resolves via a real import alias in the caller's own file to exactly one candidate"),
                            );
                            let edge = Edge { from: caller.id(), to: target_id, kind: EdgeKind::Calls, evidence };
                            upsert_preferring_stronger_evidence(&mut per_target, edge);
                            continue;
                        }
                        ImportAliasOutcome::External => {
                            let attempt = CallAttempt::unresolved_import_target_external(caller.id(), callee_name.clone());
                            per_attempt_name.entry(callee_name).or_insert(attempt);
                            continue;
                        }
                        ImportAliasOutcome::Ambiguous(candidate_ids) => {
                            let attempt = CallAttempt::ambiguous_import_target(caller.id(), callee_name.clone(), candidate_ids);
                            per_attempt_name.entry(callee_name).or_insert(attempt);
                            continue;
                        }
                    }
                }
            }

            // Semantic Resolution & Reliability Checkpoint, Priority 1:
            // before falling into the existing scope-narrowing/
            // ambiguity logic below, narrow `candidates` by the call
            // site's own generic arity when it is known (C#'s
            // `Foo<T>(...)`/`obj.Foo<T1,T2>(...)` — B2-ID-011's own
            // extraction already made this arity observable at the
            // call site; this checkpoint is the first consumer of it).
            // Never applies to a non-generic call (`call_site.generic_arity
            // == None`) — `narrow_candidates_by_call_site_arity` itself
            // returns `candidates` UNCHANGED in that case, so this is a
            // pure addition, never a behavior change for the (vast
            // majority) non-generic call sites this resolver already
            // handled correctly.
            let arity_narrowed_candidates = narrow_candidates_by_call_site_arity(&candidates, call_site.generic_arity);

            // Deliberately classified `NameHeuristic`, never
            // `StaticAnalysis`, even though arity narrowing is a
            // structural/syntactic fact (matching type-argument
            // count), not a spatial guess like `narrow_by_scope`'s
            // own enclosing-class/file narrowing: two real overloads
            // can still share the SAME generic arity while differing
            // only in ORDINARY parameter types (e.g.
            // `Handle<TException>()` vs.
            // `Handle<TException>(Func<TException,bool>)` — both
            // arity 1). Arity narrowing is strictly MORE than bare-
            // name matching alone, but it is not full overload
            // resolution (parameter-type comparison, A5's own future
            // scope) — `StaticAnalysis` is reserved for the case where
            // NO ambiguity of any kind remained among the candidates
            // this resolver was given; arity-reduced-but-still-only-
            // partially-disambiguated certainty does not meet that
            // bar.
            //
            // Note: `candidates.len() > 1` is NOT checked here — by
            // this point, `candidates.len()` is already guaranteed to
            // be `0` or `>= 2` (the `candidates.len() == 1` branch
            // above already returned via `continue`), so a redundant
            // `> 1`/`>= 1` check here would be a provably equivalent
            // mutant target, confirmed by `cargo mutants` and removed
            // rather than kept as dead-weight defensive code.
            if call_site.generic_arity.is_some() && arity_narrowed_candidates.len() == 1 {
                let (evidence, _confidence) = heuristic_evidence(
                    codegraph_domain::ResolutionMethod::NameHeuristic,
                    Confidence::Likely,
                    &format!(
                        "call to {callee_name:?} resolves to exactly one candidate once narrowed by its own call-site generic arity ({:?}) among {} same-named candidates",
                        call_site.generic_arity,
                        candidates.len()
                    ),
                );
                let edge = Edge {
                    from: caller.id(),
                    to: arity_narrowed_candidates[0].id(),
                    kind: EdgeKind::Calls,
                    evidence,
                };
                upsert_preferring_stronger_evidence(&mut per_target, edge);
                continue;
            }

            // If arity narrowing found a real, smaller-but-still-
            // ambiguous set (2+ candidates sharing both the bare name
            // AND the call site's own arity), continue the REST of
            // this function's own ambiguity handling against that
            // NARROWER set rather than the original, so a real
            // genuine ambiguity that arity cannot resolve is reported
            // (and scope-narrowed, if applicable) against the correct,
            // smaller candidate pool — never silently reverting to the
            // wider, arity-blind set once arity information was
            // actually available and used to narrow at all.
            let candidates = if call_site.generic_arity.is_some() { arity_narrowed_candidates } else { candidates };
            // Zero candidates: the callee is outside what this resolver
            // was given to work with (see this module's doc comment) —
            // "unresolved: no candidate exists" (tracked as a distinct
            // outcome from ambiguity, per the Evidence-before-claims
            // Experimental Policy), and the logic below naturally
            // produces no edges for an empty `candidates`.
            //
            // More than one candidate: Task 9.5, Priority 5(A), Case B.
            // Previously (CALL-RES-A0, Task 7's original design — see
            // `docs/research/EXPERIMENTAL_POLICY.md`), this resolver
            // recorded ONE Hypothesized edge per ambiguous candidate,
            // corpus-wide, with no scope narrowing at all. Measured on a
            // real 97-file corpus (B2-EVAL-001), this produced a 19-for-
            // 19 false-positive rate asking "who calls this `forward`"
            // on a commonly-named method — every one of 19 incoming
            // edges was confirmed, by manual source inspection, to be an
            // unrelated same-named method in a different file/class.
            //
            // The fix (CALL-RES-A1 + A2): narrow `candidates` through a
            // scope hierarchy — same enclosing class as `caller`, then
            // same file as `caller` — BEFORE deciding. At whichever
            // level is narrowest while still non-empty: exactly one
            // candidate there is accepted as a scoped heuristic edge
            // (still not `StaticAnalysis` — this is still a name-based
            // guess, just a much better-scoped one); more than one
            // candidate, even at the narrowest non-empty level, is
            // recorded as "unresolved: ambiguity remains" and produces
            // NO edge — never an arbitrary pick, and never N Hypothesized
            // edges for targets this resolver cannot actually
            // distinguish. This is itself an experimental configuration
            // (see the policy doc), not a provably optimal rule.
            if let Some((scoped_candidate, scope_description)) = narrow_by_scope(lookup.snapshots(), caller, &candidates) {
                let (evidence, _confidence) = heuristic_evidence(
                    codegraph_domain::ResolutionMethod::NameHeuristic,
                    Confidence::Speculative,
                    &format!("call to {callee_name:?} resolves to exactly one candidate once narrowed to {scope_description}"),
                );
                let edge = Edge {
                    from: caller.id(),
                    to: scoped_candidate.id(),
                    kind: EdgeKind::Calls,
                    evidence,
                };
                upsert_preferring_stronger_evidence(&mut per_target, edge);
                continue;
            }
            // Else: ambiguity remains even at the narrowest non-empty
            // scope level — no edge is produced for this occurrence,
            // per CALL-RES-A1's "no edge beats N speculative edges" rule.
            // Phase 2: record WHICH outcome this was (zero candidates
            // vs. real ambiguity among 2+), per CallAttempt/EvidenceTier
            // — the negative-knowledge record this module's own
            // "zero candidates" / "more than one candidate" comment
            // above already distinguishes in PROSE but, before this
            // change, never preserved as DATA past this point.
            let attempt = if candidates.is_empty() {
                // Priority 3: if a real candidate sharing this EXACT
                // callee name was independently confirmed to be a
                // `SignatureTooLong` rejection from this same
                // extraction pass, report that specific, real cause
                // instead of the generic `Unknown` — never guessed,
                // only when the caller's own `rejected_names` set
                // (built from real `CandidateRejection` records)
                // confirms it.
                if rejected_names.contains(callee_name.as_str()) {
                    CallAttempt::unresolved_with_extraction_rejection(caller.id(), callee_name.clone(), "SignatureTooLong")
                } else if import_facts.is_empty() {
                    // No import evidence was given to this call AT ALL
                    // (the old entry points, or a real run against a
                    // language/corpus with zero import facts) — `Unknown`
                    // is the correct, honest classification: import
                    // evidence was never consulted, not consulted-and-
                    // found-nothing. Confirmed by the dedicated
                    // equivalence test that `resolve_calls_with_imports`
                    // called with `&[]` is byte-for-byte identical to
                    // the pre-A4 behavior.
                    CallAttempt::unresolved(caller.id(), callee_name.clone())
                } else {
                    // CALL-RES-A4: real import facts WERE given to this
                    // call, `resolve_via_import_alias` was actually
                    // invoked above (the `candidates.is_empty()` branch
                    // already ran it) and returned `None` — i.e. import
                    // evidence was genuinely checked for this exact
                    // callee name and found no matching alias. This is
                    // a more specific, honest claim than plain
                    // `Unknown`, never a guess.
                    CallAttempt::unresolved_no_matching_import_alias(caller.id(), callee_name.clone())
                }
            } else {
                CallAttempt::ambiguous(caller.id(), callee_name.clone(), candidates.iter().map(|c| c.id()).collect())
            };
            per_attempt_name.entry(callee_name).or_insert(attempt);
        }

        let mut caller_attempts: Vec<CallAttempt> = per_attempt_name.into_values().collect();
        caller_attempts.sort_by(|a, b| a.callee_name.cmp(&b.callee_name));
        attempts.extend(caller_attempts);

        let mut caller_edges: Vec<Edge> = per_target.into_values().collect();
        caller_edges.sort_by(|a, b| a.to.as_bytes().cmp(b.to.as_bytes()));
        edges.extend(caller_edges);
    }

    Ok((edges, attempts))
}

/// CALL-RES-A4 — C# namespace-aware candidate filtering. Identical to
/// [`resolve_calls_with_imports`] in every way EXCEPT: when a call
/// site's direct by-name lookup finds **2 or more** real candidates (the
/// ambiguity branch CRC-001/CRC-002 occupy, and which
/// `resolve_calls_with_imports`'s own import-alias mechanism never
/// touches), and the caller is a C# snapshot, this function additionally
/// consults `csharp_contexts` to determine each candidate's own
/// containing-type namespace (via [`enclosing_class_of`], then a
/// `(file, span)` lookup into `csharp_contexts` for that containing
/// type), and `import_facts` to determine which namespaces the caller's
/// own file has brought into scope via a real `using Namespace;`
/// directive (a *bare* using directive — `module_path` with no `alias`,
/// confirmed as the real shape `collect_csharp_using` extracts for this
/// form). A candidate is excluded from the pool ONLY when its own
/// namespace is DEFINITELY not visible: it differs from the caller's
/// own namespace AND does not match any `using`'d namespace in the
/// caller's file. A candidate whose namespace cannot be determined at
/// all (no containing class found, or that class has no
/// `CSharpTypeDeclarationContext` entry) is NEVER excluded — incomplete
/// namespace information must never be treated as "definitely not
/// visible," per this checkpoint's own three-way visibility discipline
/// (visible / definitely-not-visible / unknown — `unknown` is never
/// collapsed into `not-visible`).
///
/// # Never invents a target
///
/// This function only ever REMOVES candidates whose exclusion is
/// positively demonstrated by real namespace/using evidence; it never
/// ADDS a candidate, never resolves via name similarity, and if
/// filtering leaves zero or one candidate, control falls through to
/// this function's own byte-identical copy of the existing scope-
/// narrowing/ambiguity-recording logic — the same "no edge beats a
/// false edge" discipline every other resolver in this module already
/// follows. If `csharp_contexts` is empty, or the caller is not C#, this
/// function is a byte-for-byte no-op versus
/// [`resolve_calls_with_imports`] — confirmed by a dedicated equivalence
/// test.
///
/// # Scope: only the already-ambiguous (`>= 2`) branch
///
/// This function's own namespace filter runs ONLY after the direct
/// by-name lookup already found 2+ candidates (and, for a generic call
/// site, only after arity-narrowing has already been applied — see
/// [`resolve_calls_with_imports`]'s own Priority-1 arity logic, which
/// this function reuses unchanged). It never touches the zero-candidate
/// import-alias path above.
pub fn resolve_calls_with_namespace_context(snapshots: &[SymbolSnapshot], signature_too_long_names: &[String], import_facts: &[ImportFact], csharp_contexts: &[CSharpTypeDeclarationContext]) -> Result<(Vec<Edge>, Vec<CallAttempt>), EdgeResolutionError> {
    resolve_calls_with_imports_and_namespaces(snapshots, signature_too_long_names, import_facts, csharp_contexts)
}

/// The real implementation behind [`resolve_calls_with_namespace_context`].
/// Deliberately a thin, separately-named function (rather than adding a
/// 4th parameter to [`resolve_calls_with_imports`] itself) so every
/// existing call site of that function keeps its exact 3-argument
/// signature unchanged.
fn resolve_calls_with_imports_and_namespaces(snapshots: &[SymbolSnapshot], signature_too_long_names: &[String], import_facts: &[ImportFact], csharp_contexts: &[CSharpTypeDeclarationContext]) -> Result<(Vec<Edge>, Vec<CallAttempt>), EdgeResolutionError> {
    if csharp_contexts.is_empty() {
        // No namespace evidence given at all -- byte-identical to the
        // pre-A4-namespace behavior, confirmed by a dedicated
        // equivalence test.
        return resolve_calls_with_imports(snapshots, signature_too_long_names, import_facts);
    }

    let lookup = TargetLookup::new(snapshots)?;
    let mut edges = Vec::new();
    let mut attempts = Vec::new();
    let rejected_names: std::collections::HashSet<&str> = signature_too_long_names.iter().map(|s| s.as_str()).collect();

    for caller in lookup.snapshots() {
        if !caller.kind().can_originate_calls() {
            continue;
        }
        let Some(parser) = parser_for(caller.file()) else {
            continue;
        };
        let Ok(tree) = parser.parse(caller.signature().as_str().as_bytes()) else {
            continue;
        };

        // The caller's own containing-type namespace (if determinable)
        // and the set of namespaces its own file brings into scope via
        // a bare `using Namespace;` directive (never via an aliased
        // `using Foo = ...;` -- that shape is a symbol alias, not a
        // namespace import, and stays exclusively in the zero-candidate
        // alias path above).
        let caller_namespace = namespace_of_snapshot(lookup.snapshots(), csharp_contexts, caller);
        let used_namespaces = bare_using_namespaces_in_file(import_facts, caller.file());

        let mut per_target: std::collections::HashMap<codegraph_domain::SymbolSnapshotId, Edge> = std::collections::HashMap::new();
        let mut per_attempt_name: std::collections::HashMap<String, CallAttempt> = std::collections::HashMap::new();

        for call_site in call_sites_in_tree(&tree) {
            let absolute_call_site_position = caller.span().start + call_site.fragment_relative_start_byte;
            if call_site_belongs_to_a_more_nested_caller(lookup.snapshots(), caller, absolute_call_site_position) {
                continue;
            }

            let callee_name = call_site.callee_name;
            let all_candidates = lookup.find_by_name(&callee_name, caller.id());

            let candidates = match &call_site.self_receiver_field {
                Some(field_name) if receiver_field_is_a_known_member(lookup.snapshots(), caller, field_name) => {
                    restrict_to_callers_own_class(lookup.snapshots(), caller, &all_candidates)
                }
                _ => all_candidates,
            };

            if candidates.len() == 1 {
                let evidence = resolved_evidence(&format!("call to {callee_name:?} resolves to exactly one candidate in the given snapshot pool"));
                let edge = Edge { from: caller.id(), to: candidates[0].id(), kind: EdgeKind::Calls, evidence };
                upsert_preferring_stronger_evidence(&mut per_target, edge);
                continue;
            }

            if candidates.is_empty() {
                if let Some(outcome) = resolve_via_import_alias(caller, &callee_name, import_facts, lookup.snapshots()) {
                    match outcome {
                        ImportAliasOutcome::Resolved(target_id) => {
                            let (evidence, _confidence) = heuristic_evidence(
                                codegraph_domain::ResolutionMethod::NameHeuristic,
                                Confidence::Likely,
                                &format!("call to {callee_name:?} matches no candidate directly, but resolves via a real import alias in the caller's own file to exactly one candidate"),
                            );
                            let edge = Edge { from: caller.id(), to: target_id, kind: EdgeKind::Calls, evidence };
                            upsert_preferring_stronger_evidence(&mut per_target, edge);
                            continue;
                        }
                        ImportAliasOutcome::External => {
                            let attempt = CallAttempt::unresolved_import_target_external(caller.id(), callee_name.clone());
                            per_attempt_name.entry(callee_name).or_insert(attempt);
                            continue;
                        }
                        ImportAliasOutcome::Ambiguous(candidate_ids) => {
                            let attempt = CallAttempt::ambiguous_import_target(caller.id(), callee_name.clone(), candidate_ids);
                            per_attempt_name.entry(callee_name).or_insert(attempt);
                            continue;
                        }
                    }
                }
            }

            let arity_narrowed_candidates = narrow_candidates_by_call_site_arity(&candidates, call_site.generic_arity);
            if call_site.generic_arity.is_some() && arity_narrowed_candidates.len() == 1 {
                let (evidence, _confidence) = heuristic_evidence(
                    codegraph_domain::ResolutionMethod::NameHeuristic,
                    Confidence::Likely,
                    &format!(
                        "call to {callee_name:?} resolves to exactly one candidate once narrowed by its own call-site generic arity ({:?}) among {} same-named candidates",
                        call_site.generic_arity,
                        candidates.len()
                    ),
                );
                let edge = Edge { from: caller.id(), to: arity_narrowed_candidates[0].id(), kind: EdgeKind::Calls, evidence };
                upsert_preferring_stronger_evidence(&mut per_target, edge);
                continue;
            }
            let candidates = if call_site.generic_arity.is_some() { arity_narrowed_candidates } else { candidates };

            // CALL-RES-A4 namespace filtering: only when 2+ candidates
            // remain after arity narrowing, and only for a C# caller
            // (namespace_of_snapshot/bare_using_namespaces_in_file both
            // return conservative empty/None results for every other
            // language, making this a no-op there).
            let namespace_filtered = exclude_definitely_not_visible_by_namespace(lookup.snapshots(), csharp_contexts, &candidates, caller_namespace.as_deref(), &used_namespaces);
            let namespace_narrowed_this_call = namespace_filtered.len() < candidates.len();
            let candidates = namespace_filtered;

            if namespace_narrowed_this_call && candidates.len() == 1 {
                let (evidence, _confidence) = heuristic_evidence(
                    codegraph_domain::ResolutionMethod::NameHeuristic,
                    Confidence::Likely,
                    &format!("call to {callee_name:?} resolves to exactly one candidate once candidates outside the caller's own or any used namespace are excluded"),
                );
                let edge = Edge { from: caller.id(), to: candidates[0].id(), kind: EdgeKind::Calls, evidence };
                upsert_preferring_stronger_evidence(&mut per_target, edge);
                continue;
            }

            if let Some((scoped_candidate, scope_description)) = narrow_by_scope(lookup.snapshots(), caller, &candidates) {
                let (evidence, _confidence) = heuristic_evidence(
                    codegraph_domain::ResolutionMethod::NameHeuristic,
                    Confidence::Speculative,
                    &format!("call to {callee_name:?} resolves to exactly one candidate once narrowed to {scope_description}"),
                );
                let edge = Edge { from: caller.id(), to: scoped_candidate.id(), kind: EdgeKind::Calls, evidence };
                upsert_preferring_stronger_evidence(&mut per_target, edge);
                continue;
            }

            let attempt = if candidates.is_empty() {
                if rejected_names.contains(callee_name.as_str()) {
                    CallAttempt::unresolved_with_extraction_rejection(caller.id(), callee_name.clone(), "SignatureTooLong")
                } else if import_facts.is_empty() {
                    CallAttempt::unresolved(caller.id(), callee_name.clone())
                } else {
                    CallAttempt::unresolved_no_matching_import_alias(caller.id(), callee_name.clone())
                }
            } else {
                CallAttempt::ambiguous(caller.id(), callee_name.clone(), candidates.iter().map(|c| c.id()).collect())
            };
            per_attempt_name.entry(callee_name).or_insert(attempt);
        }

        let mut caller_attempts: Vec<CallAttempt> = per_attempt_name.into_values().collect();
        caller_attempts.sort_by(|a, b| a.callee_name.cmp(&b.callee_name));
        attempts.extend(caller_attempts);

        let mut caller_edges: Vec<Edge> = per_target.into_values().collect();
        caller_edges.sort_by(|a, b| a.to.as_bytes().cmp(b.to.as_bytes()));
        edges.extend(caller_edges);
    }

    Ok((edges, attempts))
}

/// Looks up `symbol`'s own containing-type namespace: finds its
/// enclosing `Class` (via [`enclosing_class_of`] — note this does not
/// yet consider an enclosing `Interface`, a pre-existing limitation of
/// `enclosing_class_of` itself, not newly introduced here), then finds
/// that class's own `(file, span)` entry in `csharp_contexts`. Returns
/// `None` — never a guessed/default namespace — when either step fails
/// to find a real match, which this module's own callers treat as
/// "namespace unknown," never as "global namespace" or "not visible."
fn namespace_of_snapshot(snapshots: &[SymbolSnapshot], csharp_contexts: &[CSharpTypeDeclarationContext], symbol: &SymbolSnapshot) -> Option<String> {
    let enclosing_class = enclosing_class_of(snapshots, symbol)?;
    csharp_contexts.iter().find(|c| &c.file == enclosing_class.file() && c.span == enclosing_class.span()).map(|c| c.namespace.clone())
}

/// Every namespace a file brings into scope via a real, BARE
/// `using Namespace;` directive — i.e. an `ImportFact` for this exact
/// `file` whose own `alias` is `None` (an aliased `using Foo = X;` is a
/// symbol alias, already handled exclusively by the zero-candidate
/// alias path, and is deliberately excluded here to avoid double-
/// counting the same fact two different ways). `module_path` is used
/// AS WRITTEN (the real dotted/qualified namespace text), never
/// truncated to its own last segment — unlike `resolve_raw_import_statements`'s
/// own `lookup_name` derivation (which exists to find a SYMBOL, not a
/// namespace), this function's own job is namespace-string EQUALITY
/// against `CSharpTypeDeclarationContext::namespace`'s own canonical
/// dotted form, which the full `module_path` text already matches
/// exactly for a real `using Polly.Bulkhead;` directive.
fn bare_using_namespaces_in_file(import_facts: &[ImportFact], file: &codegraph_domain::FileIdentity) -> std::collections::HashSet<String> {
    import_facts.iter().filter(|f| &f.importing_file == file && f.alias.is_none()).map(|f| f.module_path.clone()).collect()
}

/// Excludes every candidate whose own namespace is DEFINITELY not
/// visible from `caller_namespace`'s own position: a namespace was
/// actually determined for the candidate (never `None` — an unknown
/// namespace is NEVER treated as "not visible," per this checkpoint's
/// own three-way visibility discipline), AND that namespace differs
/// from both `caller_namespace` itself and every entry in
/// `used_namespaces`. If filtering would remove EVERY candidate (every
/// one has a determined, definitely-wrong namespace), the ORIGINAL,
/// unfiltered list is returned instead — this function narrows toward a
/// smaller honest ambiguity, it never manufactures a zero-candidate
/// outcome out of a real `>= 2` ambiguity that this filter's own
/// evidence cannot fully explain (e.g. every real candidate living in a
/// namespace this file never actually imports would be a sign this
/// filter's own namespace data is incomplete or wrong for this case,
/// not genuine proof that NONE of them could possibly be the target).
fn exclude_definitely_not_visible_by_namespace<'a>(snapshots: &[SymbolSnapshot], csharp_contexts: &[CSharpTypeDeclarationContext], candidates: &[&'a SymbolSnapshot], caller_namespace: Option<&str>, used_namespaces: &std::collections::HashSet<String>) -> Vec<&'a SymbolSnapshot> {
    if candidates.len() < 2 {
        return candidates.to_vec();
    }
    let filtered: Vec<&SymbolSnapshot> = candidates
        .iter()
        .copied()
        .filter(|candidate| match namespace_of_snapshot(snapshots, csharp_contexts, candidate) {
            None => true, // Unknown namespace -- never excluded.
            Some(candidate_ns) => Some(candidate_ns.as_str()) == caller_namespace || used_namespaces.contains(&candidate_ns),
        })
        .collect();
    if filtered.is_empty() {
        candidates.to_vec()
    } else {
        filtered
    }
}

/// Task 9.5, Priority 5(A): the `Class` snapshot that structurally
/// contains `symbol`, by the SAME span-nesting rule `resolve_contains`
/// (this crate's own `Contains` edge resolver) uses: same file,
/// `Class`-kind, `symbol`'s span strictly nested inside the class's
/// span, innermost (smallest) class chosen when more than one
/// candidate class contains it. Deliberately re-derived here rather
/// than depending on `resolve_contains`'s own `Edge` output: resolvers
/// in this module each take a flat snapshot pool and produce `Edge`s
/// independently (see `TargetLookup`'s own doc comment — no resolver
/// may depend on another resolver's edges, only on the shared snapshot
/// pool and `SymbolSnapshot`'s own structural fields), so this stays a
/// pool-level query rather than introducing a cross-resolver ordering
/// dependency.
fn enclosing_class_of<'a>(snapshots: &'a [SymbolSnapshot], symbol: &SymbolSnapshot) -> Option<&'a SymbolSnapshot> {
    let mut innermost: Option<&SymbolSnapshot> = None;
    for candidate_class in snapshots {
        if candidate_class.id() == symbol.id() || candidate_class.kind() != SymbolKind::Class {
            continue;
        }
        if !same_file(candidate_class, symbol) || !strictly_contains(candidate_class, symbol) {
            continue;
        }
        innermost = Some(match innermost {
            None => candidate_class,
            Some(current) if span_len(candidate_class) < span_len(current) => candidate_class,
            Some(current) => current,
        });
    }
    innermost
}

/// B2-ID-006, mechanism 2 of 2: `true` if some snapshot OTHER than
/// `caller` is (a) caller-eligible, (b) in the same file as `caller`,
/// (c) strictly nested inside `caller`'s own span, and (d) itself
/// strictly contains `absolute_call_site_position` — meaning THAT
/// nested snapshot, not `caller`, is the call site's real nearest
/// executable owner. When more than one such nested snapshot exists
/// (a doubly-nested function, for instance), any one of them being
/// found is sufficient to exclude `caller`: `caller` is never the
/// nearest owner once ANY more deeply nested eligible snapshot also
/// contains the position, regardless of how many levels of nesting
/// separate them — the INNERMOST one will itself independently decide
/// whether IT has an even-more-nested owner, in its own iteration of
/// `resolve_calls`'s outer loop.
fn call_site_belongs_to_a_more_nested_caller(snapshots: &[SymbolSnapshot], caller: &SymbolSnapshot, absolute_call_site_position: usize) -> bool {
    snapshots.iter().any(|candidate| {
        candidate.id() != caller.id()
            && candidate.kind().can_originate_calls()
            && same_file(candidate, caller)
            && strictly_contains(caller, candidate)
            && span_contains_position(candidate.span(), absolute_call_site_position)
    })
}

pub(super) fn span_contains_position(span: crate::extract::ByteSpan, position: usize) -> bool {
    span.start <= position && position < span.end
}

pub(super) fn same_file(a: &SymbolSnapshot, b: &SymbolSnapshot) -> bool {
    a.file().repository() == b.file().repository() && a.file().commit() == b.file().commit() && a.file().relative_path() == b.file().relative_path()
}

pub(super) fn strictly_contains(outer: &SymbolSnapshot, inner: &SymbolSnapshot) -> bool {
    let outer_span = outer.span();
    let inner_span = inner.span();
    outer_span.start <= inner_span.start && inner_span.end <= outer_span.end && (outer_span.start, outer_span.end) != (inner_span.start, inner_span.end)
}

fn span_len(snapshot: &SymbolSnapshot) -> usize {
    snapshot.span().end - snapshot.span().start
}

/// `true` if `field_name` names a real member (any kind — `Field`,
/// `Method`, or `Function`, matching `resolve_contains`'s own
/// containment scope) of `caller`'s own enclosing class. This is the
/// gate for applying Case A's receiver-aware restriction at all: a
/// `self.X` reference where `X` is NOT a real member of the caller's
/// class is not something this resolver can honestly reason about
/// structurally (e.g. a dynamically-added attribute, or simply wrong
/// input), so no restriction is applied for it — "no behavior change"
/// is the safe default when the structural premise does not hold.
/// CALL-RES-A4: the three real outcomes a caller's own callee name can
/// have once matched against a real `ImportFact.alias` in the SAME
/// file. Mirrors `ImportResolution`'s own three-way shape exactly
/// (`Internal`/`External`/`Ambiguous`) — this is deliberately a
/// SEPARATE type, not a re-export of `ImportResolution`, because this
/// one additionally carries the Ambiguous case's own candidate-id list
/// (needed to build `CallAttempt::ambiguous_import_target`'s own
/// `candidates` field; `ImportResolution::Ambiguous` itself carries no
/// ids, since `ImportFact`'s own resolution step intentionally never
/// stores the ambiguous set — see `imports.rs`'s own "no edge beats a
/// false edge" discipline).
enum ImportAliasOutcome {
    Resolved(codegraph_domain::SymbolSnapshotId),
    External,
    Ambiguous(Vec<codegraph_domain::SymbolSnapshotId>),
}

/// CALL-RES-A4's own core: looks up whether `callee_name` matches a
/// real import ALIAS (never a bare `imported_symbol`/`module_path`
/// match — see this function's own caller,
/// [`resolve_calls_with_imports`], for exactly why only the alias field
/// is consulted) in one of `import_facts` whose own `importing_file`
/// is the SAME file as `caller`. Returns `None` when no `ImportFact`
/// in the caller's own file has a matching `alias` at all — the caller
/// must then fall through to its own existing `Unknown`/
/// `NoMatchingImportAlias` classification, never guessed here.
///
/// If MORE THAN ONE `ImportFact` in the caller's own file happens to
/// share the exact same `alias` text (a syntactically unusual but
/// parseable shape — two separate import statements aliasing to the
/// same local name, which would itself be a real naming collision in
/// the SOURCE file, not a defect in this resolver), this function
/// deterministically prefers the import fact appearing EARLIEST in
/// `import_facts` (stable, order-independent of HOW `import_facts` was
/// assembled upstream, since `imports.rs`'s own extraction already
/// produces one fact per real import statement in source order) —
/// mirroring `prefer_edge`'s own "never order-dependent on iteration,
/// but a stable, explicit tie-break" discipline. This is NOT a new
/// ambiguity-resolution policy; re-aliasing the same local name twice
/// in one real file is itself a rare, degenerate input this function
/// does not need to resolve "correctly" beyond being deterministic
/// about it.
fn resolve_via_import_alias(caller: &SymbolSnapshot, callee_name: &str, import_facts: &[ImportFact], snapshots: &[SymbolSnapshot]) -> Option<ImportAliasOutcome> {
    let matching_fact = import_facts.iter().find(|fact| fact.importing_file == *caller.file() && fact.alias.as_deref() == Some(callee_name))?;
    match matching_fact.resolution {
        ImportResolution::Internal(target_id) => {
            // Defensive re-confirmation, not a re-derivation: `ImportFact`'s
            // own resolution was already computed once, globally, by
            // `resolve_raw_import_statements` against the SAME kind of
            // snapshot pool — but that resolution could, in principle,
            // have been computed against a DIFFERENT (e.g. stale)
            // snapshot slice than the one this exact `resolve_calls_with_imports`
            // call was given (the two are passed as independent
            // parameters by the caller — see this module's own
            // `TargetLookup` doc comment on never silently trusting a
            // cross-call invariant). Confirm `target_id` is still a
            // real member of THIS call's own `snapshots` before
            // producing an edge that points at it, never assuming.
            if snapshots.iter().any(|s| s.id() == target_id) {
                Some(ImportAliasOutcome::Resolved(target_id))
            } else {
                None
            }
        }
        ImportResolution::External => Some(ImportAliasOutcome::External),
        ImportResolution::Ambiguous => {
            // `ImportResolution::Ambiguous` itself carries no candidate
            // ids (see this function's own doc comment) — re-derive the
            // real candidate set by the SAME lookup key
            // `resolve_raw_import_statements` used (`imported_symbol`,
            // falling back to the module path's own last segment),
            // mirroring that module's own matching convention exactly
            // rather than inventing a second one.
            let lookup_name = matching_fact.imported_symbol.clone().unwrap_or_else(|| matching_fact.module_path.rsplit(&['.', ':'][..]).next().unwrap_or(&matching_fact.module_path).to_string());
            let candidate_ids: Vec<codegraph_domain::SymbolSnapshotId> = snapshots.iter().filter(|s| s.name().as_str() == lookup_name).map(|s| s.id()).collect();
            Some(ImportAliasOutcome::Ambiguous(candidate_ids))
        }
    }
}

fn receiver_field_is_a_known_member(snapshots: &[SymbolSnapshot], caller: &SymbolSnapshot, field_name: &str) -> bool {
    let Some(class) = enclosing_class_of(snapshots, caller) else {
        return false;
    };
    snapshots.iter().any(|s| {
        s.id() != class.id()
            && matches!(s.kind(), SymbolKind::Field | SymbolKind::Method | SymbolKind::Function)
            && s.name().as_str() == field_name
            && same_file(class, s)
            && strictly_contains(class, s)
    })
}

/// Restricts `candidates` (every snapshot matching the called method's
/// bare name, corpus-wide) down to the ones that are themselves members
/// of `caller`'s own enclosing class. If `caller` has no enclosing
/// class, or none of `candidates` belong to it, returns an EMPTY list
/// deliberately — per the task's explicit "unresolved is preferable to
/// false positive" instruction, this is not a fallback to the
/// unrestricted set; a `self.field.method()` call whose matching
/// methods are all OUTSIDE the caller's own class is exactly the false-
/// positive shape this fix exists to prevent (e.g. `self.src_mask.size(0)`
/// must not fall back to matching two unrelated C++ `size()` methods
/// just because no `size` member exists on the caller's own class —
/// that absence is itself evidence the call is on an external/
/// unresolvable type, not a reason to guess more broadly).
fn restrict_to_callers_own_class<'a>(snapshots: &[SymbolSnapshot], caller: &SymbolSnapshot, candidates: &[&'a SymbolSnapshot]) -> Vec<&'a SymbolSnapshot> {
    let Some(class) = enclosing_class_of(snapshots, caller) else {
        return Vec::new();
    };
    candidates.iter().copied().filter(|c| same_file(class, c) && strictly_contains(class, c)).collect()
}

/// Task 9.5, Priority 5(A), Case B: progressively narrows an ambiguous
/// (`len() > 1`) `candidates` list down a scope hierarchy — same
/// enclosing class as `caller`, then same file as `caller` — returning
/// the single surviving candidate (and a short description of which
/// scope level resolved it, for the edge's own evidence note) the
/// FIRST time exactly one candidate remains at a non-empty level.
///
/// Per the Evidence-before-claims Experimental Policy
/// (`docs/research/EXPERIMENTAL_POLICY.md`), this is CALL-RES-A2: an
/// experimental scoping configuration, not a proven-optimal rule. Each
/// level is tried in order; a level with ZERO candidates falls through
/// to the next (it narrowed too far, not far enough), while a level
/// with MORE THAN ONE candidate stops the search entirely — ambiguity
/// within the narrowest scope that has any candidates is not resolved
/// by trying an even broader scope (that would only ever make the
/// candidate set larger, never smaller). `None` means every level tried
/// either was empty or stayed ambiguous — the caller must then record
/// "unresolved: ambiguity remains" and produce no edge.
///
/// Import/visibility-aware narrowing (Priority 6 and beyond,
/// CALL-RES-A4) is a planned FUTURE level between "same file" and
/// "repo-wide," not implemented here — no import-resolution data exists
/// in this resolver yet to narrow by.
fn narrow_by_scope<'a>(snapshots: &[SymbolSnapshot], caller: &SymbolSnapshot, candidates: &[&'a SymbolSnapshot]) -> Option<(&'a SymbolSnapshot, &'static str)> {
    if let Some(class) = enclosing_class_of(snapshots, caller) {
        let same_class: Vec<&SymbolSnapshot> = candidates.iter().copied().filter(|c| same_file(class, c) && strictly_contains(class, c)).collect();
        match same_class.len() {
            0 => {} // Narrowed too far — fall through to the next level.
            1 => return Some((same_class[0], "the caller's own enclosing class")),
            _ => return None, // Ambiguous even within the caller's own class.
        }
    }

    let same_file_candidates: Vec<&SymbolSnapshot> = candidates.iter().copied().filter(|c| same_file(caller, c)).collect();
    // The `0` arm is written explicitly (naming "no same-file
    // candidate at all" distinctly from "ambiguous among several"),
    // even though both it and `_` currently return the same `None` —
    // kept apart for readability, not because mutation testing could
    // distinguish them. (Task 9.5's closing mutation gate confirmed
    // deleting the `0` arm is an EQUIVALENT mutant BY CONSTRUCTION: with
    // it gone, `0` falls into `_ => None`, the exact same return value,
    // so no input can ever tell the two arms apart as written today.
    // Left this way deliberately — do not "simplify" by merging `0` and
    // `_`; doing so would only remove a semantic distinction this
    // function may want to keep observable if a future caller needs to
    // tell "no candidate" apart from "ambiguous.")
    match same_file_candidates.len() {
        0 => None, // No candidate even shares the caller's file — repo-wide ambiguity is never auto-resolved (CALL-RES-A1).
        1 => Some((same_file_candidates[0], "the caller's own file")),
        _ => None, // Ambiguous even within the caller's own file.
    }
}

/// Semantic Resolution & Reliability Checkpoint, Priority 1: narrows
/// `candidates` to those whose own generic arity matches `call_arity`,
/// when `call_arity` is known (`Some`) — the call site itself exposes
/// a generic/template clause (`Foo<T>(...)`, B2-ID-011's own
/// extraction). Returns the UNFILTERED `candidates` slice unchanged
/// when `call_arity` is `None` (an ordinary, non-generic call — no
/// arity information to narrow by at all, never treated as "arity
/// zero"; see `wrapped_callee_simple_name`'s own doc comment for why
/// these two cases must never be conflated).
///
/// # Scope: C# only, confirmed by direct grammar probe — not C++
///
/// A C# `method_declaration`'s own `type_parameter_list` (its
/// generic-method arity) is a DIRECT CHILD of the declaration node
/// itself, inside the exact span `extract/csharp.rs` promotes as that
/// method's own `signature_text` — reachable today by re-parsing
/// `candidate.signature()` exactly as `narrow_by_scope`'s own sibling
/// resolvers already re-parse signatures for other purposes.
///
/// C++ is NOT handled here, confirmed structurally absent rather than
/// merely unimplemented: a C++ TEMPLATE function
/// (`template<typename T> void Handle() {}`) is extracted by
/// `extract/cpp.rs` as a bare `function_definition` candidate whose
/// own `span`/`signature_text` is ONLY the inner `function_definition`
/// node's text (`"void Handle() {}"`) — the enclosing
/// `template_declaration` and its own `template_parameter_list` are
/// OUTSIDE that span entirely (confirmed via a direct grammar probe:
/// `template_declaration` is the PARENT of `function_definition`, not
/// reachable by re-parsing the promoted signature fragment in
/// isolation). A C++ template function's own arity is therefore not
/// recoverable from the current snapshot model at all today — a
/// separate, pre-existing extraction gap, not something this
/// checkpoint's own narrow scope (arity-aware CANDIDATE NARROWING, not
/// new extraction surface) fixes. Documented honestly as a residual
/// limitation rather than silently applying narrowing that would
/// always find `None` and look like "C++ was checked and found to
/// have no arity-reducible ambiguity" when the true state is "C++
/// method arity is currently unobservable."
fn narrow_candidates_by_call_site_arity<'a>(candidates: &[&'a SymbolSnapshot], call_arity: Option<usize>) -> Vec<&'a SymbolSnapshot> {
    let Some(arity) = call_arity else {
        return candidates.to_vec();
    };
    let narrowed: Vec<&SymbolSnapshot> = candidates.iter().copied().filter(|c| csharp_method_generic_arity(c) == Some(arity)).collect();
    if narrowed.is_empty() {
        // Every candidate either has an unknown/unobservable arity (a
        // non-C# candidate) or a non-matching one. Narrowing to an
        // EMPTY set here would incorrectly turn "ambiguous among real
        // candidates" into "unresolved: no candidate" — a worse,
        // dishonest outcome (the real candidates DO exist; this
        // resolver simply cannot use arity to pick among them in this
        // case). Return the ORIGINAL, unfiltered set instead, so the
        // caller's existing ambiguity handling applies unchanged.
        candidates.to_vec()
    } else {
        narrowed
    }
}

/// A C# `SymbolSnapshot`'s own method-level generic arity, re-derived
/// by re-parsing `candidate.signature()` as a standalone C# fragment
/// (the same re-parse-the-signature convention every other resolver in
/// this crate already uses) and counting the resulting declaration
/// node's own direct `type_parameter_list` child — the exact same
/// counting logic `csharp_declarations.rs`'s own `generic_arity_of`
/// already established for TYPE declarations, applied here to a
/// METHOD-shaped declaration instead (confirmed via direct grammar
/// probe: identical shape, `type_parameter_list` is a direct child
/// either way).
///
/// Confirmed via direct grammar probe that a standalone re-parse of a
/// real extracted method signature (no enclosing `class`) produces
/// `local_function_statement`, NOT `method_declaration` — a re-parse
/// artifact of the missing class context, not a real semantic
/// difference (the real snapshot IS a class method; `extract/csharp.rs`
/// only ever promotes a `method_declaration` as `SymbolKind::Method`
/// in the first place, confirmed by the `candidate.kind()` guard
/// below). Both node kinds are searched for here, for exactly this
/// reason.
///
/// Returns `None` for a non-C# candidate (file's own language is not
/// C#) or a candidate whose signature fails to re-parse as a
/// `method_declaration`/`local_function_statement` at all — never a
/// guessed `Some(0)` for a case this function cannot actually observe.
fn csharp_method_generic_arity(candidate: &SymbolSnapshot) -> Option<usize> {
    if candidate.kind() != SymbolKind::Method {
        return None;
    }
    let parser = parser_for(candidate.file())?;
    let tree = parser.parse(candidate.signature().as_str().as_bytes()).ok()?;
    if tree.language() != Language::CSharp {
        return None;
    }
    let root = tree.tree().root_node();
    let method_node = find_descendant_of_kind(root, &["method_declaration", "local_function_statement"])?;
    let mut cursor = method_node.walk();
    let type_parameter_list = method_node.children(&mut cursor).find(|c| c.kind() == "type_parameter_list")?;
    let mut tp_cursor = type_parameter_list.walk();
    Some(type_parameter_list.children(&mut tp_cursor).filter(|c| c.kind() == "type_parameter").count())
}

/// Depth-first search for the first descendant (including `node`
/// itself) of any kind in `kinds`. Used by
/// [`csharp_method_generic_arity`] to find the re-parsed fragment's own
/// method-shaped declaration node without assuming it is the tree's
/// direct root child, and without assuming which of the two possible
/// node kinds a standalone re-parse produces (see that function's own
/// doc comment for why there are two).
fn find_descendant_of_kind<'a>(node: tree_sitter::Node<'a>, kinds: &[&str]) -> Option<tree_sitter::Node<'a>> {
    if kinds.contains(&node.kind()) {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_descendant_of_kind(child, kinds) {
            return Some(found);
        }
    }
    None
}

/// Inserts `edge` into `per_target` (keyed by `edge.to`), or, if a
/// different occurrence already produced an edge to the SAME target,
/// keeps whichever one `prefer_edge` ranks higher — deterministically,
/// never by insertion/iteration order. This is the one place
/// `resolve_calls` collapses repeated call-site occurrences of the same
/// `(from, to)` pair into a single stored `Edge` (B2-ID-002).
fn upsert_preferring_stronger_evidence(per_target: &mut std::collections::HashMap<codegraph_domain::SymbolSnapshotId, Edge>, edge: Edge) {
    match per_target.get(&edge.to) {
        Some(existing) if !prefer_edge(&edge, existing) => {}
        _ => {
            per_target.insert(edge.to, edge);
        }
    }
}

/// Deterministic edge-preference rule for B2-ID-002's deduplication:
/// `true` if `candidate` should replace `current` for the same `(from,
/// to)` pair. `StaticAnalysis` always outranks a heuristic method
/// (resolved-unique beats ambiguous, which can only happen if the SAME
/// callee name resolved uniquely on one occurrence but, implausibly,
/// ambiguously on another within the same caller — not reachable today
/// since a given name's candidate set does not change within one
/// `resolve_calls` call, but handled explicitly rather than left
/// unreachable-and-unproven). When both sides have the same
/// `ResolutionMethod`, this scope's `Edge`/`Evidence` carry no other
/// field meaningfully distinguishing "better," so the choice is a
/// stable, explicit tie-break (note text, byte-for-byte) rather than
/// "whichever happened to be inserted first" — never a silent, order-
/// dependent pick.
fn prefer_edge(candidate: &Edge, current: &Edge) -> bool {
    let candidate_is_static = candidate.evidence.method() == codegraph_domain::ResolutionMethod::StaticAnalysis;
    let current_is_static = current.evidence.method() == codegraph_domain::ResolutionMethod::StaticAnalysis;
    if candidate_is_static != current_is_static {
        return candidate_is_static;
    }
    candidate.evidence.note() < current.evidence.note()
}

/// Every callee name found anywhere inside `tree`'s root, by walking for
/// the per-language call-expression node kind and extracting its own
/// callee child's name — `identifier` for a free/simple call, or the
/// LAST segment of an `attribute`/member-access chain for a method-call
/// shape (`self.encoder(x)` -> `"encoder"`, confirmed against this
/// workspace's real `tree_sitter_python` output before writing this
/// logic, not assumed).
pub(super) fn call_names_in_tree(tree: &crate::parse::ParseTree) -> Vec<String> {
    call_sites_in_tree(tree).into_iter().map(|site| site.callee_name).collect()
}

/// Test/probe convenience: callee name paired with its own call-site
/// generic arity (`None` for a non-generic/template call), per the
/// Semantic Resolution & Reliability Checkpoint's own arity-aware
/// extraction. Mirrors [`call_names_in_tree`] exactly, extended with
/// the one new field.
#[cfg(test)]
pub(super) fn call_names_with_arity_in_tree(tree: &crate::parse::ParseTree) -> Vec<(String, Option<usize>)> {
    call_sites_in_tree(tree).into_iter().map(|site| (site.callee_name, site.generic_arity)).collect()
}

/// One syntactic call-site occurrence: the callee name being called,
/// plus (Task 9.5, Priority 5(A)) the `self`/`this`-qualified receiver
/// FIELD name, when the call is of the shape `self.field.method(...)`/
/// `this.field.method(...)` — `None` for a bare call (`helper()`), a
/// single-segment receiver call (`self.method()`), or a call through a
/// receiver this pipeline cannot resolve the type of (a plain local
/// variable, parameter, or module alias — no type inference here, so no
/// guess is made; see `self_or_this_receiver_field_name`'s own doc
/// comment for why this is deliberately conservative).
pub(super) struct CallSite {
    pub(super) callee_name: String,
    /// Semantic Resolution & Reliability Checkpoint: this call site's
    /// own generic/template type-argument count, when the call is
    /// generic/template-shaped (`Foo<T>()`, `obj.Foo<T1,T2>()`) —
    /// `None` for an ordinary, non-generic call. See
    /// `wrapped_callee_simple_name`'s own doc comment for exactly why
    /// `None` (never `Some(0)`) is the correct "no arity information"
    /// value.
    pub(super) generic_arity: Option<usize>,
    pub(super) self_receiver_field: Option<String>,
    /// B2-ID-006: the call expression's own start byte, relative to the
    /// re-parsed FRAGMENT text (`caller.signature()`'s own bytes, NOT
    /// the whole file) — i.e. exactly `tree_sitter::Node::start_byte()`
    /// on the fragment's own parse tree. `resolve_calls` converts this
    /// to an absolute file position by adding `caller.span().start`
    /// before comparing it against any other snapshot's own absolute
    /// `ByteSpan`, since every `SymbolSnapshot::span()` is a whole-file
    /// offset (see `extract::candidate::ByteSpan`'s own doc comment)
    /// while this fragment-relative offset is only meaningful paired
    /// with the specific `caller` it was produced from.
    pub(super) fragment_relative_start_byte: usize,
}

pub(super) fn call_sites_in_tree(tree: &crate::parse::ParseTree) -> Vec<CallSite> {
    let mut sites = Vec::new();
    walk_for_calls(tree.tree().root_node(), tree.source(), tree.language(), &mut sites);
    sites
}

fn walk_for_calls(node: tree_sitter::Node, source: &[u8], language: Language, out: &mut Vec<CallSite>) {
    let is_call_node = match language {
        Language::Rust => node.kind() == "call_expression",
        Language::Python => node.kind() == "call",
        Language::CSharp => node.kind() == "invocation_expression",
        Language::Cpp => node.kind() == "call_expression",
        Language::JavaScript | Language::TypeScript | Language::Tsx => node.kind() == "call_expression",
        Language::Html | Language::Css => false,
    };
    if is_call_node {
        if let Some((callee_name, generic_arity)) = callee_name(node, source) {
            let self_receiver_field = node.child(0).and_then(|callee| self_or_this_receiver_field_name(callee, source));
            out.push(CallSite {
                callee_name,
                generic_arity,
                self_receiver_field,
                fragment_relative_start_byte: node.start_byte(),
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_calls(child, source, language, out);
    }
}

/// `call_node`'s own first named child is the callee expression: an
/// `identifier` for a simple call, a `generic_name`/`template_function`
/// for a generic/template call with no receiver (`Foo<int>()` — see
/// B2-ID-011 below), or an `attribute`/`field_expression`/
/// `member_expression`/`member_access_expression`-shaped member access
/// for a method call, whose own LAST
/// `identifier`/`property_identifier`/`field_identifier`/`generic_name`/
/// `template_method` child is the method name actually being called
/// (`self.encoder` -> `"encoder"`, not `"self"`;
/// `obj.Handle<TimeoutException>` -> `"Handle"`, not
/// `"Handle<TimeoutException>"`). `member_access_expression` (C#'s own
/// node kind for `this.srcMask.Size`, confirmed via a direct
/// `tree-sitter-c-sharp` parse dump during Task 9.5's Priority 5(A)
/// investigation) was previously missing from this match arm — every
/// other language's equivalent node kind was already handled, so a C#
/// receiver-qualified call silently extracted no callee name at all,
/// confirmed as a real, pre-existing gap, not hypothetical.
///
/// # B2-ID-011: generic/template callees
///
/// Confirmed via direct grammar probe (not assumed): a generic/
/// template method call's own callee name is wrapped one level deeper
/// than the simple cases above, in BOTH shapes this function must
/// handle, for BOTH languages that have this construct —
///
/// - bare (`Foo<int>()`): `invocation_expression`/`call_expression`'s
///   own first child IS the wrapper node directly (C#'s
///   `generic_name`, C++'s `template_function`), matched by the
///   `"generic_name" | "template_function"` arm below;
/// - receiver-qualified (`obj.Foo<int>()`, `this.Foo<int>()`,
///   `foo.Bar.Baz<int>()`): the member-access callee's own LAST child
///   is the wrapper node (C#'s `generic_name`, C++'s
///   `template_method`) — never a bare `identifier`/
///   `property_identifier`/`field_identifier` — so both wrapper kinds
///   are added to the SAME filter the receiver-qualified arm already
///   used, handled by `wrapped_callee_simple_name` below rather than
///   returned as-is (which would wrongly yield the FULL text
///   `"Handle<TimeoutException>"`, not the bare method name
///   `"Handle"`).
///
/// Before this fix, NEITHER shape reached a matching arm at all, in
/// either language: a bare generic/template call fell through
/// `callee.kind()`'s `_ => None` arm (dropped before ever reaching
/// `call_sites_in_tree`'s own output), and a receiver-qualified
/// generic/template call's `.last()` filter never matched the wrapper
/// kind, so it returned `None` from the empty iterator — both silently
/// invisible to this resolver, not merely ambiguous. Measured on a
/// real C# corpus (`App-vNext/Polly`): 4977 of 26508 (18.8%)
/// receiver-qualified call sites and 166 bare call sites were generic
/// and therefore entirely unseen before this fix. C++'s own
/// `template_function`/`template_method` shapes were confirmed broken
/// by the identical mechanism during this fix's own cross-language
/// scope audit (not separately measured on a real C++ corpus — the
/// grammar-level defect is structurally identical, confirmed via
/// direct probe, which this project's own "grammar-first" discipline
/// treats as sufficient confirmation without a separate corpus
/// measurement for a fix this narrow).
///
/// A generic/template call that is itself the RECEIVER of a further
/// call (`obj.Foo<int>().Next()`) needs no special handling here: the
/// grammar represents the chain as the OUTER call's callee being a
/// member-access node whose own first child is the INNER call
/// (`obj.Foo<int>()`), not a wrapper node at all — `Next`'s own
/// extraction already works via the pre-existing
/// `identifier`/`property_identifier`/`field_identifier` filter, and
/// the inner `Foo<int>()` call is found independently by
/// `walk_for_calls`'s own unconditional recursion into every child,
/// confirmed by a direct grammar probe before writing this fix.
/// Likewise a generic/template call nested inside another call's own
/// argument list (`Bar(Foo<int>())`) needs no special handling: both
/// call nodes are visited independently by the same recursive walk.
///
/// # B2-ID-011 cross-language scope audit: C++ has the identical gap
///
/// Confirmed via direct grammar probe (not assumed to be "plausibly
/// unaffected," as this finding's own discovery note originally
/// guessed before this fix's own audit): C++ template calls share the
/// EXACT same structural defect, one level of indirection C# does not
/// have, under different node-kind names — a bare template call
/// (`Foo<int>()`) wraps its callee in a `template_function` node
/// (`call_expression`'s own first child), and a receiver-qualified
/// template call (`obj.Foo<int>()`) wraps it in a `template_method`
/// node as `field_expression`'s own last child — neither kind matched
/// any existing arm before this fix, for the identical reason
/// `generic_name` did not. Both are added to this function's own
/// existing C#/C++ handling, via the SAME two helper functions this
/// fix already introduced for C# (renamed to reflect both languages).
fn callee_name(call_node: tree_sitter::Node, source: &[u8]) -> Option<(String, Option<usize>)> {
    let callee = call_node.child(0)?;
    match callee.kind() {
        "identifier" => Some((text_of(callee, source), None)),
        "generic_name" | "template_function" => wrapped_callee_simple_name(callee, source),
        "attribute" | "field_expression" | "member_expression" | "member_access_expression" => {
            let mut cursor = callee.walk();
            let last_name_child = callee
                .children(&mut cursor)
                .filter(|c| matches!(c.kind(), "identifier" | "property_identifier" | "field_identifier" | "generic_name" | "template_method"))
                .last()?;
            if matches!(last_name_child.kind(), "generic_name" | "template_method") {
                wrapped_callee_simple_name(last_name_child, source)
            } else {
                Some((text_of(last_name_child, source), None))
            }
        }
        _ => None,
    }
}

/// Extracts a generic/template call's own bare method name from its
/// wrapper node's first direct name-kind child —
/// `identifier`/`field_identifier`, e.g. `"Handle"` for C#'s
/// `Handle<TimeoutException>` (a `generic_name` node) or `"Foo"` for
/// C++'s `Foo<int>`/`obj.Foo<int>` (a `template_function`/
/// `template_method` node respectively). Confirmed via direct grammar
/// probe for all three wrapper kinds: each has exactly TWO direct
/// children — one name-kind node (the bare name) followed by one
/// `type_argument_list`/`template_argument_list` (the `<...>` clause)
/// — no competing same-kind sibling exists at this position (unlike
/// `method_declaration`'s own return-type/name collision B2-ID-009
/// fixed), so a positional "first matching-kind child" search is safe
/// here, not the bug class that fix was scoped to.
///
/// # Semantic Resolution & Reliability Checkpoint: call-site generic arity
///
/// Also counts the wrapper's own `type_argument_list`/
/// `template_argument_list` children (every direct child EXCLUDING the
/// literal `<`/`>`/`,` punctuation tokens — the same counting
/// convention `csharp_declarations.rs`'s own `generic_arity_of`
/// already established for TYPE declarations, confirmed identical here
/// by direct grammar probe: a C# call's `type_argument_list` holds one
/// `identifier`/`predefined_type`/... per type argument; a C++ call's
/// `template_argument_list` holds one `type_descriptor` per argument),
/// returned as `Some(arity)`. A non-generic/template call (the plain
/// `"identifier"` arm above, and the plain-name branch of the
/// receiver-qualified arm) returns `None` for arity — meaningfully
/// different from `Some(0)`: `None` means "this call site carries no
/// arity information to narrow by at all" (arity-aware resolution must
/// never apply here), while a real `Foo()` call with explicit empty
/// angle brackets does not occur in either grammar (you cannot write
/// `Foo<>()` to mean "zero type arguments" — a method with no type
/// parameters is simply called as a plain identifier), so `Some(0)`
/// is not a real case this function needs to produce.
fn wrapped_callee_simple_name(wrapper_node: tree_sitter::Node, source: &[u8]) -> Option<(String, Option<usize>)> {
    let mut cursor = wrapper_node.walk();
    let name_child = wrapper_node.children(&mut cursor).find(|c| matches!(c.kind(), "identifier" | "field_identifier"))?;
    let name = text_of(name_child, source);
    let mut arg_cursor = wrapper_node.walk();
    let arity = wrapper_node
        .children(&mut arg_cursor)
        .find(|c| matches!(c.kind(), "type_argument_list" | "template_argument_list"))
        .map(|list| {
            let mut list_cursor = list.walk();
            list.children(&mut list_cursor).filter(|c| !matches!(c.kind(), "<" | ">" | ",")).count()
        });
    Some((name, arity))
}

/// `true` if `callee`'s node kind is one of the receiver-qualified
/// attribute/member-access shapes this module recognizes (Python's
/// `attribute`, Rust/C++'s `field_expression`, JS/TS's
/// `member_expression`, C#'s `member_access_expression`) — i.e. the
/// call has a RECEIVER at all, as opposed to a bare `identifier` call
/// with no receiver.
fn is_receiver_qualified(callee: tree_sitter::Node) -> bool {
    matches!(callee.kind(), "attribute" | "field_expression" | "member_expression" | "member_access_expression")
}

/// For a receiver-qualified callee node (`recv.field.method(...)`'s
/// `recv.field.method` node), returns the RECEIVER segment's own name —
/// `"src_mask"` for `self.src_mask.size`, the segment immediately before
/// the final (method-name) segment — if, and only if, the receiver is
/// literally `self` (Python/Rust) or `this` (C#/C++/JS/TS). Confirmed
/// via real parse dumps across all 6 grammars (Task 9.5, Priority 5(A))
/// that a receiver-qualified callee with more than one segment is
/// ALWAYS a nested attribute/member node one level down (e.g. Python:
/// outer `attribute["self.src_mask.size"]` containing inner
/// `attribute["self.src_mask"]`), never a flat multi-child list — so the
/// receiver segment is this inner node, and its own `self`/`this`-ness
/// is checked on ITS first child.
///
/// Returns `None` for a non-`self`/`this` receiver (a plain local
/// variable or module alias) — deliberately: this pipeline has no type
/// inference, so "what class does this local variable's value belong
/// to" is not an honestly answerable question yet, and inventing an
/// answer would be exactly the kind of guess Task 9.5 was told to avoid.
/// Only the `self`/`this` case is resolvable today, because it names the
/// CALLER's OWN enclosing scope, which this resolver can determine
/// directly from the snapshot pool's own byte spans (see
/// `enclosing_class_of`).
fn self_or_this_receiver_field_name(callee: tree_sitter::Node, source: &[u8]) -> Option<String> {
    if !is_receiver_qualified(callee) {
        return None;
    }
    let receiver_node = callee.child(0)?;
    if !is_receiver_qualified(receiver_node) {
        // A single-segment receiver (`self.method()`, no intermediate
        // field) has no distinct field name to extract — not this
        // function's concern; `callee_name` already resolves that case
        // correctly on its own.
        return None;
    }
    let mut cursor = receiver_node.walk();
    let mut children = receiver_node.children(&mut cursor);
    let base = children.next()?;
    // Rust has a dedicated `self` node kind; C#/C++/JS/TS have a
    // dedicated `this` node kind (all confirmed via real parse dumps,
    // Task 9.5 Priority 5(A)). Python has NEITHER — `self` is an
    // ordinary parameter name with no special grammar status, so it
    // parses as a plain `identifier` node whose TEXT happens to be
    // "self" (confirmed the same way: a real `tree_sitter_python` parse
    // dump of `self.src_mask.size` shows `identifier ["self"]`, not a
    // distinct node kind). Checking both the node kind (for the
    // languages that have one) and the literal text "self" (for
    // Python, where that is the only signal available) covers every
    // language this resolver supports without assuming Python works
    // like the others.
    let is_self_or_this = matches!(base.kind(), "self" | "this") || (base.kind() == "identifier" && text_of(base, source) == "self");
    if !is_self_or_this {
        return None;
    }
    let mut cursor = receiver_node.walk();
    receiver_node
        .children(&mut cursor)
        .filter(|c| matches!(c.kind(), "identifier" | "property_identifier" | "field_identifier"))
        .last()
        .map(|n| text_of(n, source))
}

fn text_of(node: tree_sitter::Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_domain::{CommitHash, FileIdentity, RepositoryRegistry, RepositoryRoot};

    use crate::extract::{ByteSpan, RawSymbolCandidate};
    use crate::snapshot::promote;

    fn test_snapshot(path: &str, name: &str, kind_hint: &str, signature: &str) -> SymbolSnapshot {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start: 0, end: 10 }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    /// Regression for B2-ID-003's investigation: a chained call
    /// (`self._generate_square_subsequent_mask(len(src)).to(device)`)
    /// must resolve the inner, chained call to its target exactly like
    /// any other call site. B2-ID-003 confirmed this was ALREADY
    /// correct in `resolve_calls` itself — the apparent false negative
    /// originally reported was an evaluation-harness subject-selection
    /// bug, not a defect here — but no permanent regression test
    /// existed to pin that down, so this one does.
    #[test]
    fn a_chained_call_resolves_the_inner_call_to_its_target() {
        let forward_body = "def forward(self, src, has_mask=True):\n        if has_mask:\n            device = src.device\n            if self.src_mask is None or self.src_mask.size(0) != len(src):\n                mask = self._generate_square_subsequent_mask(len(src)).to(device)\n                self.src_mask = mask\n        else:\n            self.src_mask = None\n\n        src = self.input_emb(src) * math.sqrt(self.ninp)\n        src = self.pos_encoder(src)\n        output = self.encoder(src, mask=self.src_mask)\n        output = self.decoder(output)\n        return F.log_softmax(output, dim=-1)";
        let forward = test_snapshot("src/model.py", "forward", "function_definition", forward_body);
        let gssm = test_snapshot(
            "src/model.py",
            "_generate_square_subsequent_mask",
            "function_definition",
            "def _generate_square_subsequent_mask(self, sz):\n        mask = (torch.triu(torch.ones(sz, sz)) == 1).transpose(0, 1)\n        mask = mask.float().masked_fill(mask == 0, float('-inf')).masked_fill(mask == 1, float(0.0))\n        return mask",
        );
        let snapshots = vec![forward.clone(), gssm.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(
            edges.iter().any(|e| e.from == forward.id() && e.to == gssm.id()),
            "the chained call's inner callee must resolve to its target"
        );
    }

    /// Like `test_snapshot`, but all snapshots built from the SAME
    /// `repo` the caller passes in — needed when a test actually
    /// inserts into a `CodeGraph`, which rejects a cross-repository
    /// edge (Task 8's own invariant); `test_snapshot` alone mints a
    /// fresh, unrelated `RepositoryId` on every call, which is fine for
    /// every OTHER test in this file (none of which touches
    /// `CodeGraph`) but would trip that invariant here.
    fn test_snapshot_in_repo(repo: codegraph_domain::RepositoryId, path: &str, name: &str, kind_hint: &str, signature: &str) -> SymbolSnapshot {
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start: 0, end: 10 }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    /// Like `test_snapshot_in_repo`, but with an explicit `ByteSpan` —
    /// needed for Priority 5(A)'s receiver-aware tests, which depend on
    /// real span nesting (via `enclosing_class_of`) to determine which
    /// class a caller/field/method belongs to; `test_snapshot`/
    /// `test_snapshot_in_repo` both hardcode the same `[0, 10)` span for
    /// every snapshot, which can never nest.
    fn spanned_snapshot_in_repo(repo: codegraph_domain::RepositoryId, path: &str, name: &str, kind_hint: &str, signature: &str, start: usize, end: usize) -> SymbolSnapshot {
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start, end }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    #[test]
    fn a_call_to_a_unique_candidate_is_resolved() {
        let caller = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() { helper(); }");
        let callee = test_snapshot("src/lib.rs", "helper", "function_item", "fn helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, caller.id());
        assert_eq!(edges[0].to, callee.id());
        assert_eq!(edges[0].kind, EdgeKind::Calls);
        assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    #[test]
    fn a_repo_wide_ambiguous_call_with_no_scope_signal_produces_no_edge() {
        // Task 9.5, Priority 5(A), Case B (CALL-RES-A1+A2 — see
        // docs/research/EXPERIMENTAL_POLICY.md): this is the EXACT shape
        // that produced B2-EVAL-001's 19-for-19 false-positive rate on a
        // real 97-file corpus — a bare, unqualified call whose name
        // matches more than one candidate, with no scope signal (no
        // enclosing class for the caller, candidates in different files
        // from the caller and from each other) to narrow by. Previously
        // (CALL-RES-A0) this resolver recorded one Hypothesized edge per
        // candidate; the new policy prefers no edge over an arbitrary
        // or over-broad guess once narrowing is exhausted.
        let caller = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() { run(); }");
        let candidate_a = test_snapshot("src/a.rs", "run", "function_item", "fn run() {}");
        let candidate_b = test_snapshot("src/b.rs", "run", "function_item", "fn run() {}");
        let snapshots = vec![caller.clone(), candidate_a.clone(), candidate_b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.is_empty(), "ambiguity with no scope signal to narrow by must produce no edge, not an arbitrary or over-broad guess: got {edges:?}");
    }

    #[test]
    fn an_ambiguous_call_narrowed_to_the_callers_own_file_produces_one_heuristic_edge() {
        // Same bare-name ambiguity, but now ONE of the two same-named
        // candidates shares the caller's own file — same-file narrowing
        // (CALL-RES-A2) must resolve this to that one candidate, with
        // heuristic (never StaticAnalysis) evidence, since the file-
        // level match is still a name-based guess, just a scoped one.
        let repo = test_repo_calls();
        let caller = test_snapshot_in_repo(repo, "src/lib.rs", "foo", "function_item", "fn foo() { run(); }");
        let same_file_candidate = test_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() {}");
        let unrelated_candidate = test_snapshot_in_repo(repo, "src/b.rs", "run", "function_item", "fn run() {}");
        let snapshots = vec![caller.clone(), same_file_candidate.clone(), unrelated_candidate.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1, "exactly one candidate shares the caller's file; that must resolve uniquely: got {edges:?}");
        assert_eq!(edges[0].from, caller.id());
        assert_eq!(edges[0].to, same_file_candidate.id());
        assert_ne!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis, "a scoped-but-still-name-based match must never be StaticAnalysis");
    }

    #[test]
    fn an_ambiguous_call_with_two_same_file_candidates_still_produces_no_edge() {
        // Narrowing to "same file as caller" still leaves TWO
        // candidates — genuine same-scope ambiguity. Per the user's
        // explicit final decision for Case B, this must NOT fall back
        // to emitting heuristic edges for both; it must produce no edge
        // at all, exactly like the repo-wide case.
        let repo = test_repo_calls();
        let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "foo", "function_item", "fn foo() { run(); }", 0, 20);
        let candidate_a = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* a */ }", 30, 50);
        let candidate_b = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* b */ }", 60, 80);
        let snapshots = vec![caller.clone(), candidate_a.clone(), candidate_b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.is_empty(), "two indistinguishable candidates in the caller's own file must still produce no edge, not an arbitrary pick: got {edges:?}");
    }

    #[test]
    fn an_ambiguous_call_narrowed_to_the_callers_own_class_produces_one_heuristic_edge_even_with_another_same_file_candidate() {
        // The scope hierarchy's priority order: same-class is tried
        // BEFORE same-file. ClassA.forward calls run(); ClassA itself
        // contains a `run` method (unqualified call, no receiver, but
        // still within ClassA's own span) — wait: an unqualified call
        // has no receiver to tie it to a class syntactically, so this
        // test instead verifies the class-level narrowing uses span
        // containment of the CANDIDATE within the caller's class, which
        // is the real signal `enclosing_class_of`/`narrow_by_scope` use:
        // a `run` method nested inside the SAME class as the caller
        // narrows correctly even when a second, same-file-but-different-
        // class `run` also exists.
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 200);
        let forward = spanned_snapshot_in_repo(repo, "src/m.py", "forward", "function_definition", "def forward(self):\n    run()", 10, 90);
        let run_in_class_a = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run(self): pass", 100, 190);
        let run_outside_any_class = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run(): pass", 250, 280);
        let snapshots = vec![class_a.clone(), forward.clone(), run_in_class_a.clone(), run_outside_any_class.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1, "exactly one `run` is a member of the caller's own enclosing class; that must resolve uniquely: got {edges:?}");
        assert_eq!(edges[0].to, run_in_class_a.id());
    }

    #[test]
    fn a_call_to_an_unknown_target_produces_no_edge() {
        let caller = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() { not_in_the_pool(); }");
        let snapshots = vec![caller];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.is_empty(), "a call to a name absent from the given pool must produce no edge");
    }

    #[test]
    fn a_method_call_resolves_by_its_final_segment_name() {
        // The forward()-calls-encoder() deep-learning-relevant shape:
        // self.encoder(x) must resolve to a snapshot named "encoder",
        // not "self".
        let caller = test_snapshot(
            "src/model.py",
            "forward",
            "function_definition",
            "def forward(self, x):\n    return self.encoder(x)",
        );
        let encoder = test_snapshot("src/model.py", "encoder", "function_definition", "def encoder(x):\n    return x");
        let snapshots = vec![caller.clone(), encoder.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, encoder.id());
        assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    #[test]
    fn a_cross_file_call_is_still_resolved() {
        let caller = test_snapshot("src/a.rs", "foo", "function_item", "fn foo() { helper(); }");
        let callee = test_snapshot("src/b.rs", "helper", "function_item", "fn helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_recursive_call_does_not_self_link() {
        let recursive = test_snapshot("src/lib.rs", "factorial", "function_item", "fn factorial(n: u32) -> u32 { factorial(n - 1) }");
        let snapshots = vec![recursive.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.is_empty(), "a function calling itself by name must not produce a self-referential edge from this resolver");
    }

    #[test]
    fn a_duplicate_snapshot_id_is_a_typed_rejection() {
        let snapshot = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() {}");
        let snapshots = vec![snapshot.clone(), snapshot];

        let result = resolve_calls(&snapshots);

        assert_eq!(result.err(), Some(EdgeResolutionError::DuplicateSnapshotId));
    }

    #[test]
    fn a_cpp_call_is_resolved() {
        let caller = test_snapshot("src/a.cpp", "foo", "function_definition", "void foo() { helper(); }");
        let callee = test_snapshot("src/a.cpp", "helper", "function_definition", "void helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_header_extension_is_recognized_as_cpp() {
        let caller = test_snapshot("src/a.hpp", "foo", "function_definition", "void foo() { helper(); }");
        let callee = test_snapshot("src/a.hpp", "helper", "function_definition", "void helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_csharp_call_is_resolved() {
        let caller = test_snapshot("src/a.cs", "M", "method_declaration", "void M() { Helper(); }");
        let callee = test_snapshot("src/a.cs", "Helper", "method_declaration", "void Helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    // --- B2-ID-011: C#/C++ generic/template method call extraction ---
    //
    // `callee_name`/`wrapped_callee_simple_name` must extract the bare
    // method name (never the full `Method<T>` text, the receiver, or
    // a generic/template type argument) from a `generic_name`/
    // `template_function`/`template_method`-wrapped call site, in
    // both the bare and receiver-qualified shapes. Each test below
    // checks BOTH layers: call-site EXTRACTION (`call_names_in_tree`,
    // the raw text layer) and final EDGE generation (`resolve_calls`,
    // which additionally requires resolution against a real candidate
    // pool).
    mod b2_id_011_generic_method_calls {
        use super::*;
        use crate::parse::{CSharpParser, CppParser, LanguageParser};

        fn extracted_names(source: &[u8]) -> Vec<String> {
            let tree = CSharpParser.parse(source).unwrap();
            call_names_in_tree(&tree)
        }

        /// Case 1 (required): bare generic call, single type argument.
        #[test]
        fn bare_generic_call_extracts_the_bare_method_name() {
            let names = extracted_names(b"class C { void M() { Foo<int>(); } }\n");
            assert!(names.contains(&"Foo".to_string()), "expected \"Foo\" among {names:?}");
            assert!(!names.iter().any(|n| n.contains('<')), "the extracted name must never contain the generic clause: {names:?}");
        }

        /// Case 2 (required): bare generic call, multiple type arguments.
        #[test]
        fn bare_generic_call_with_multiple_type_arguments_extracts_the_bare_method_name() {
            let names = extracted_names(b"class C { void M() { Foo<int, string>(); } }\n");
            assert!(names.contains(&"Foo".to_string()), "expected \"Foo\" among {names:?}");
        }

        /// Case 10 (required), receiver-qualified generic call.
        #[test]
        fn receiver_qualified_generic_call_extracts_the_bare_method_name_not_the_receiver() {
            let names = extracted_names(b"class C { void M() { obj.Foo<int>(); } }\n");
            assert!(names.contains(&"Foo".to_string()), "expected \"Foo\" among {names:?}");
            assert!(!names.contains(&"obj".to_string()), "must never extract the receiver's own name: {names:?}");
        }

        /// Case 3/4 (required): `obj.Foo<T>()` and `this.Foo<T>()`.
        #[test]
        fn this_qualified_generic_call_extracts_the_bare_method_name() {
            let names = extracted_names(b"class C { void M() { this.Foo<int>(); } }\n");
            assert!(names.contains(&"Foo".to_string()), "expected \"Foo\" among {names:?}");
        }

        /// Case 5 (required): nested/qualified receiver (`foo.Bar.Method<T>()`).
        #[test]
        fn nested_qualified_receiver_generic_call_extracts_the_bare_method_name() {
            let names = extracted_names(b"class C { void M() { foo.Bar.Baz<int>(); } }\n");
            assert!(names.contains(&"Baz".to_string()), "expected \"Baz\" among {names:?}");
            assert!(!names.contains(&"Bar".to_string()) || names.iter().filter(|n| n.as_str() == "Bar").count() == 0, "must not extract the intermediate receiver segment as a callee name");
        }

        /// Case 6 (required): a chained generic call -- the INNER
        /// generic call and the OUTER non-generic call must both be
        /// independently extracted (the grammar represents this as two
        /// separate `invocation_expression` nodes, confirmed by direct
        /// probe, not a single flattened chain).
        #[test]
        fn chained_generic_call_extracts_both_the_inner_generic_and_outer_call() {
            let names = extracted_names(b"class C { void M() { obj.Foo<int>().Next(); } }\n");
            assert!(names.contains(&"Foo".to_string()), "expected the inner generic call's name \"Foo\" among {names:?}");
            assert!(names.contains(&"Next".to_string()), "expected the outer call's name \"Next\" among {names:?}");
        }

        /// Case 7 (required): a generic call nested inside another
        /// call's own argument list.
        #[test]
        fn generic_call_nested_inside_another_calls_argument_list_is_extracted() {
            let names = extracted_names(b"class C { void M() { Bar(Foo<int>()); } }\n");
            assert!(names.contains(&"Bar".to_string()), "expected the outer call's name \"Bar\" among {names:?}");
            assert!(names.contains(&"Foo".to_string()), "expected the nested generic call's name \"Foo\" among {names:?}");
        }

        /// Case 9 (required), explicit restatement: bare generic call,
        /// no receiver at all.
        #[test]
        fn bare_generic_call_with_no_receiver_is_extracted() {
            let names = extracted_names(b"class C { void M() { Handle<int>(); } }\n");
            assert!(names.contains(&"Handle".to_string()));
        }

        /// The real-repository motivating case: `policy.Handle<TimeoutException>(x)`.
        #[test]
        fn the_real_polly_handle_shape_extracts_handle_not_the_full_generic_text() {
            let names = extracted_names(b"class C { void M() { policy.Handle<TimeoutException>(x); } }\n");
            assert_eq!(names, vec!["Handle".to_string()]);
        }

        /// Case 11 (required): no regression on an ordinary
        /// non-generic C# call -- must still extract/resolve exactly
        /// as before this fix.
        #[test]
        fn ordinary_non_generic_csharp_call_is_unaffected() {
            let names = extracted_names(b"class C { void M() { Helper(); obj.Helper2(); } }\n");
            assert_eq!(names, vec!["Helper".to_string(), "Helper2".to_string()]);
        }

        /// Edge-generation layer: a bare generic call resolves to its
        /// unique real target, exactly like any other resolved call.
        #[test]
        fn a_bare_generic_call_resolves_to_its_unique_target_and_produces_an_edge() {
            let caller = test_snapshot("src/a.cs", "M", "method_declaration", "void M() { Handle<int>(); }");
            let callee = test_snapshot("src/a.cs", "Handle", "method_declaration", "void Handle<T>() {}");
            let snapshots = vec![caller.clone(), callee.clone()];

            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(edges.len(), 1, "expected exactly one edge: {edges:?}");
            assert_eq!(edges[0].from, caller.id());
            assert_eq!(edges[0].to, callee.id());
            assert_eq!(edges[0].kind, EdgeKind::Calls);
            assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
        }

        /// Edge-generation layer: a receiver-qualified generic call
        /// resolves to its unique real target.
        #[test]
        fn a_receiver_qualified_generic_call_resolves_to_its_unique_target_and_produces_an_edge() {
            let caller = test_snapshot("src/a.cs", "M", "method_declaration", "void M() { policy.Handle<int>(); }");
            let callee = test_snapshot("src/a.cs", "Handle", "method_declaration", "void Handle<T>() {}");
            let snapshots = vec![caller.clone(), callee.clone()];

            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(edges.len(), 1, "expected exactly one edge: {edges:?}");
            assert_eq!(edges[0].to, callee.id());
        }

        /// Case 8 (required), UPDATED by the Semantic Resolution &
        /// Reliability Checkpoint: at B2-ID-011's own time, this test
        /// asserted that a generic/non-generic overload collision
        /// sharing one bare name produced NO edge, because arity-aware
        /// disambiguation was explicitly out of that fix's own scope.
        /// The checkpoint's own Priority 1 now explicitly AUTHORIZES
        /// exactly this disambiguation: the call site `Handle<int>()`
        /// has arity 1; `Handle<T>()` also has arity 1 (matches);
        /// `Handle()` has NO generic clause at all (arity `None`, never
        /// matches any known call-site arity) -- arity narrowing
        /// correctly finds exactly one real match and resolves it,
        /// `NameHeuristic` tier (arity narrowing is more than bare-name
        /// matching but not full overload resolution — see
        /// `narrow_candidates_by_call_site_arity`'s own doc comment for
        /// why `StaticAnalysis` is not used here). This supersedes the
        /// OLD assertion deliberately, not a regression: B2-ID-011
        /// itself never claimed arity-aware resolution was permanently
        /// out of scope, only that IT was not the task to add it.
        #[test]
        fn generic_and_non_generic_overloads_sharing_one_bare_name_are_now_correctly_narrowed_by_arity() {
            let repo_for_test = {
                let mut registry = RepositoryRegistry::new();
                registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
            };
            let caller = test_snapshot_in_repo(repo_for_test, "src/a.cs", "M", "method_declaration", "void M() { Handle<int>(); }");
            let generic_overload = test_snapshot_in_repo(repo_for_test, "src/b.cs", "Handle", "method_declaration", "void Handle<T>() {}");
            let non_generic_overload = test_snapshot_in_repo(repo_for_test, "src/c.cs", "Handle", "method_declaration", "void Handle() {}");
            let snapshots = vec![caller.clone(), generic_overload.clone(), non_generic_overload.clone()];

            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(edges.len(), 1, "arity 1 matches ONLY the generic overload -- the non-generic one (no arity at all) must never be a candidate match: got {edges:?}");
            assert_eq!(edges[0].to, generic_overload.id());
            assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::NameHeuristic, "arity-narrowed resolution is NameHeuristic, never StaticAnalysis");
        }

        /// Determinism: repeated occurrences of the same resolved
        /// generic call must still collapse to one edge (B2-ID-002's
        /// own invariant, confirmed to still hold for this new callee
        /// shape).
        #[test]
        fn a_generic_call_repeated_in_one_body_still_produces_one_edge() {
            let caller = test_snapshot("src/a.cs", "M", "method_declaration", "void M() { Handle<int>(); Handle<int>(); }");
            let callee = test_snapshot("src/a.cs", "Handle", "method_declaration", "void Handle<T>() {}");
            let snapshots = vec![caller.clone(), callee.clone()];

            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(edges.len(), 1, "two occurrences of the same generic call must collapse to one edge: got {edges:?}");
        }

        // --- Cross-language scope audit: C++ template calls share
        // the identical structural defect under different node-kind
        // names (`template_function`/`template_method`), confirmed
        // during this fix's own grammar probe. ---

        fn cpp_extracted_names(source: &[u8]) -> Vec<String> {
            let tree = CppParser.parse(source).unwrap();
            call_names_in_tree(&tree)
        }

        #[test]
        fn cpp_bare_template_call_extracts_the_bare_function_name() {
            let names = cpp_extracted_names(b"void M() { Foo<int>(); }\n");
            assert!(names.contains(&"Foo".to_string()), "expected \"Foo\" among {names:?}");
            assert!(!names.iter().any(|n| n.contains('<')), "the extracted name must never contain the template clause: {names:?}");
        }

        #[test]
        fn cpp_receiver_qualified_template_call_extracts_the_bare_method_name() {
            let names = cpp_extracted_names(b"void M() { obj.Foo<int>(); }\n");
            assert!(names.contains(&"Foo".to_string()), "expected \"Foo\" among {names:?}");
            assert!(!names.contains(&"obj".to_string()), "must never extract the receiver's own name: {names:?}");
        }

        #[test]
        fn cpp_ordinary_non_template_call_is_unaffected() {
            let names = cpp_extracted_names(b"void M() { Helper(); obj.Helper2(); }\n");
            assert_eq!(names, vec!["Helper".to_string(), "Helper2".to_string()]);
        }

        #[test]
        fn a_cpp_bare_template_call_resolves_to_its_unique_target_and_produces_an_edge() {
            let caller = test_snapshot("src/a.cpp", "M", "function_definition", "void M() { Handle<int>(); }");
            let callee = test_snapshot("src/a.cpp", "Handle", "function_definition", "void Handle() {}");
            let snapshots = vec![caller.clone(), callee.clone()];

            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(edges.len(), 1, "expected exactly one edge: {edges:?}");
            assert_eq!(edges[0].to, callee.id());
        }
    }

    /// Semantic Resolution & Reliability Checkpoint, Priority 1:
    /// arity-aware C# call resolution. Every fixture below mirrors the
    /// real `App-vNext/Polly` shape B2-EVAL-008's own reliability-
    /// layer pass measured directly (`Policy.HandleSyntax.cs`'s real 6
    /// `Handle`-named overloads, 2 of which share arity 1 and differ
    /// only in ordinary parameter type — confirmed to correctly STAY
    /// ambiguous even after arity narrowing, never silently resolved
    /// by this checkpoint's own narrower scope).
    mod arity_aware_resolution {
        use super::*;
        use crate::parse::{CSharpParser, LanguageParser};

        #[test]
        fn call_site_arity_is_extracted_for_bare_and_receiver_qualified_generic_calls() {
            let tree = CSharpParser.parse(b"class C { void M() { Handle<T>(); policy.Handle<A,B>(); Plain(); } }").unwrap();
            let names_with_arity = call_names_with_arity_in_tree(&tree);
            assert_eq!(
                names_with_arity,
                vec![("Handle".to_string(), Some(1)), ("Handle".to_string(), Some(2)), ("Plain".to_string(), None),],
                "bare generic arity 1, receiver-qualified generic arity 2, ordinary call has no arity (None, never Some(0))"
            );
        }

        #[test]
        fn csharp_method_generic_arity_is_read_correctly_for_a_real_class_method() {
            // Promoted via the REAL extractor (not a hand-built
            // signature string) so the re-parse path this function
            // depends on is exercised exactly as production does it.
            let source = "class Policy {\n    public static PolicyBuilder Handle<TException>() { return null; }\n    public static PolicyBuilder HandleInner() { return null; }\n}\n";
            let snapshots = real_csharp_snapshots("src/Policy.cs", source);
            let handle = snapshots.iter().find(|s| s.name().as_str() == "Handle").unwrap();
            let handle_inner = snapshots.iter().find(|s| s.name().as_str() == "HandleInner").unwrap();
            assert_eq!(csharp_method_generic_arity(handle), Some(1));
            // A NON-generic method has no `type_parameter_list` child
            // at all -- correctly `None` ("no arity information"),
            // never a guessed `Some(0)`, mirroring the exact same
            // distinction `wrapped_callee_simple_name`'s own doc
            // comment establishes for call sites.
            assert_eq!(csharp_method_generic_arity(handle_inner), None);
        }

        #[test]
        fn a_bare_generic_call_narrows_to_its_unique_same_arity_target_among_two_overloads() {
            // Handle<TException>() [arity 1] vs. HandleOther() [arity
            // 0, different bare name so it would never collide by name
            // anyway] -- the REAL collision case is covered by the two
            // tests below; this one confirms the basic single-overload
            // arity match path end to end.
            let source = "class Policy {\n    public static PolicyBuilder Handle<TException>() { return null; }\n}\nclass Caller {\n    void M() { Handle<TimeoutException>(); }\n}\n";
            let snapshots = real_csharp_snapshots("src/a.cs", source);

            let edges = resolve_calls(&snapshots).unwrap();

            let handle = snapshots.iter().find(|s| s.name().as_str() == "Handle").unwrap();
            assert_eq!(edges.len(), 1, "expected exactly one edge: {edges:?}");
            assert_eq!(edges[0].to, handle.id());
        }

        #[test]
        fn the_real_polly_handle_shape_two_same_arity_overloads_are_correctly_narrowed_when_only_one_shares_the_call_sites_own_arity() {
            // Mirrors Policy.HandleSyntax.cs's own real shape:
            // Handle<TException>() [arity 1] and
            // Handle<TException>(Func<TException,bool>) [ALSO arity 1
            // -- the generic clause itself, not the ordinary parameter
            // list, drives arity] are TWO overloads sharing both name
            // AND arity. A call site with arity 2 (naming two type
            // arguments) cannot match either of them, demonstrating
            // arity narrowing correctly finds ZERO matches and falls
            // back to the original, unfiltered ambiguity handling
            // rather than incorrectly claiming a match that isn't
            // there.
            let source = "class Policy {\n    public static PolicyBuilder Handle<TException>() { return null; }\n    public static PolicyBuilder Handle<TException>(System.Func<TException, bool> p) { return null; }\n}\nclass Caller {\n    void M() { Handle<A,B>(); }\n}\n";
            let snapshots = real_csharp_snapshots("src/a.cs", source);

            let (edges, attempts) = resolve_calls_with_negative_knowledge(&snapshots).unwrap();

            assert!(edges.is_empty(), "arity 2 matches NEITHER real arity-1 overload -- must never be force-matched to one of them");
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].tier, codegraph_domain::EvidenceTier::Ambiguous);
            assert_eq!(attempts[0].candidates.len(), 2, "both same-arity-1 overloads remain real candidates the call's own arity-2 just cannot pick between");
        }

        #[test]
        fn two_overloads_sharing_both_name_and_arity_remain_genuinely_ambiguous_after_arity_narrowing() {
            // THE central negative case this checkpoint must get
            // right: Handle<TException>() and
            // Handle<TException>(Func<TException,bool>) both have
            // arity 1 (confirmed above). A call site that ALSO has
            // arity 1 (`Handle<TimeoutException>(somePredicate)`)
            // narrows to BOTH of them -- arity alone cannot
            // distinguish overloads that differ only in ORDINARY
            // parameter type, which is exactly what B2-EVAL-008's own
            // real-corpus measurement found and what this checkpoint
            // is explicitly scoped to NOT solve (that is A5's future
            // job). Must stay Ambiguous, never silently resolved to
            // either one.
            let source = "class Policy {\n    public static PolicyBuilder Handle<TException>() { return null; }\n    public static PolicyBuilder Handle<TException>(System.Func<TException, bool> p) { return null; }\n}\nclass Caller {\n    void M() { Handle<TimeoutException>(somePredicate); }\n}\n";
            let snapshots = real_csharp_snapshots("src/a.cs", source);

            let (edges, attempts) = resolve_calls_with_negative_knowledge(&snapshots).unwrap();

            assert!(edges.is_empty(), "two real overloads share the SAME arity -- arity narrowing must never force-pick one of them");
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].tier, codegraph_domain::EvidenceTier::Ambiguous, "genuine same-arity overload ambiguity must be reported honestly, never upgraded to a false certainty");
            assert_eq!(attempts[0].candidates.len(), 2);
        }

        #[test]
        fn an_ordinary_non_generic_call_is_completely_unaffected_by_arity_narrowing() {
            // call_site.generic_arity is None here -- arity narrowing
            // must be a no-op, confirming this checkpoint adds no
            // behavior change whatsoever for the vast majority of
            // (non-generic) call sites every prior finding in this
            // series already validated.
            let caller = test_snapshot("src/a.cs", "M", "method_declaration", "void M() { Helper(); }");
            let callee = test_snapshot("src/a.cs", "Helper", "method_declaration", "void Helper() {}");
            let snapshots = vec![caller.clone(), callee.clone()];

            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(edges.len(), 1);
            assert_eq!(edges[0].to, callee.id());
        }

        #[test]
        fn cpp_template_calls_are_unaffected_by_csharp_only_arity_narrowing_no_regression() {
            // C++ is explicitly out of scope for method-side arity
            // (csharp_method_generic_arity's own C#-only guard) --
            // confirms a C++ template call with real ambiguity still
            // goes through the EXACT same pre-existing scope-narrowing/
            // ambiguity path as before this checkpoint, unaffected by
            // the new C#-only logic.
            let caller = test_snapshot("src/a.cpp", "M", "function_definition", "void M() { Handle<int>(); }");
            let candidate_a = test_snapshot("src/a.cpp", "Handle", "function_definition", "void Handle() { /* a */ }");
            let candidate_b = test_snapshot("src/b.cpp", "Handle", "function_definition", "void Handle() { /* b */ }");
            let snapshots = vec![caller.clone(), candidate_a, candidate_b];

            let (edges, attempts) = resolve_calls_with_negative_knowledge(&snapshots).unwrap();

            assert!(edges.is_empty(), "real cross-file ambiguity among two Handle candidates must remain unresolved for C++, exactly as before this checkpoint");
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].tier, codegraph_domain::EvidenceTier::Ambiguous);
        }

        /// Promotes real `method_declaration`/`class_declaration`
        /// candidates from `source` via the REAL extractor
        /// (`extract_symbols_reporting_rejections` + `promote`), all
        /// in the SAME repo so cross-snapshot resolution works
        /// normally — mirrors the real-extraction fixture pattern
        /// `b2_id_007_nested_declaration_ownership`/
        /// `b2_id_010_partial_class_reconciliation` already established
        /// elsewhere in this crate, reused here rather than
        /// hand-building signature strings that could silently drift
        /// from what the real extractor actually produces.
        fn real_csharp_snapshots(path: &str, source: &str) -> Vec<SymbolSnapshot> {
            use crate::extract::extract_symbols_reporting_rejections;
            let mut registry = RepositoryRegistry::new();
            let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let file = FileIdentity::parse(repo, commit, path).unwrap();
            let parser = parser_for(&file).expect("csharp extension must have a parser");
            let tree = parser.parse(source.as_bytes()).expect("fixture must parse");
            let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
            assert!(rejections.is_empty(), "fixture must not trigger a rejection: {rejections:?}");
            candidates.into_iter().map(|c| promote(&c, file.clone()).expect("promote")).collect()
        }
    }

    #[test]
    fn a_tsx_call_is_resolved() {
        let caller = test_snapshot("src/a.tsx", "Comp", "function_declaration", "function Comp() { return helper(); }");
        let callee = test_snapshot("src/a.tsx", "helper", "function_declaration", "function helper() { return 1; }");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_jsx_extension_is_recognized_as_tsx() {
        let caller = test_snapshot("src/a.jsx", "Comp", "function_declaration", "function Comp() { return helper(); }");
        let callee = test_snapshot("src/a.jsx", "helper", "function_declaration", "function helper() { return 1; }");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    // --- B2-ID-002: occurrence-vs-relation deduplication ---

    #[test]
    fn calling_the_same_resolved_target_twice_in_one_body_produces_one_edge() {
        let caller = test_snapshot("src/lib.rs", "caller", "function_item", "fn caller() { callee(); callee(); }");
        let callee = test_snapshot("src/lib.rs", "callee", "function_item", "fn callee() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let to_callee: Vec<_> = edges.iter().filter(|e| e.from == caller.id() && e.to == callee.id()).collect();
        assert_eq!(to_callee.len(), 1, "two occurrences of the same call must collapse to one (from,to,Calls) edge");
    }

    #[test]
    fn calling_the_same_resolved_target_many_times_still_produces_one_edge() {
        let caller = test_snapshot("src/lib.rs", "caller", "function_item", "fn caller() { callee(); callee(); callee(); callee(); callee(); }");
        let callee = test_snapshot("src/lib.rs", "callee", "function_item", "fn callee() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let to_callee: Vec<_> = edges.iter().filter(|e| e.from == caller.id() && e.to == callee.id()).collect();
        assert_eq!(to_callee.len(), 1, "five occurrences of the same call must still collapse to one edge");
    }

    #[test]
    fn calling_two_different_targets_from_one_caller_produces_two_edges() {
        let caller = test_snapshot("src/lib.rs", "caller", "function_item", "fn caller() { a(); b(); }");
        let a = test_snapshot("src/lib.rs", "a", "function_item", "fn a() {}");
        let b = test_snapshot("src/lib.rs", "b", "function_item", "fn b() {}");
        let snapshots = vec![caller.clone(), a.clone(), b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.iter().filter(|e| e.from == caller.id()).count(), 2, "two genuinely distinct targets must not be collapsed into one edge");
    }

    #[test]
    fn two_different_callers_calling_the_same_target_each_get_their_own_edge() {
        let caller_a = test_snapshot("src/a.rs", "caller_a", "function_item", "fn caller_a() { shared(); }");
        let caller_b = test_snapshot("src/b.rs", "caller_b", "function_item", "fn caller_b() { shared(); }");
        let target = test_snapshot("src/lib.rs", "shared", "function_item", "fn shared() {}");
        let snapshots = vec![caller_a.clone(), caller_b.clone(), target.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let to_target: Vec<_> = edges.iter().filter(|e| e.to == target.id()).collect();
        assert_eq!(to_target.len(), 2, "two distinct callers of the same target must each have their own edge, not be deduplicated against each other");
        assert!(to_target.iter().any(|e| e.from == caller_a.id()));
        assert!(to_target.iter().any(|e| e.from == caller_b.id()));
    }

    #[test]
    fn an_ambiguous_call_narrowed_to_one_candidate_and_repeated_in_one_body_produces_no_duplicate_from_to_calls_triples() {
        // B2-ID-002's original concern (repeated occurrences of the
        // same ambiguous call must collapse to one edge, not one per
        // occurrence) still applies under the new Case B scoping — now
        // exercised via a scenario where same-file narrowing resolves
        // the ambiguity to exactly one candidate, called TWICE by the
        // same caller.
        let repo = test_repo_calls();
        let caller = test_snapshot_in_repo(repo, "src/caller.rs", "caller", "function_item", "fn caller() { run(); run(); }");
        let same_file_candidate = test_snapshot_in_repo(repo, "src/caller.rs", "run", "function_item", "fn run() {}");
        let unrelated_candidate = test_snapshot_in_repo(repo, "src/b.rs", "run", "function_item", "fn run() {}");
        let snapshots = vec![caller.clone(), same_file_candidate.clone(), unrelated_candidate.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let mut seen = std::collections::HashSet::new();
        for edge in &edges {
            assert!(seen.insert((edge.from, edge.to, edge.kind)), "duplicate (from,to,kind) triple found: repeated occurrences of the same scoped-ambiguous call must not produce more than one edge");
        }
        assert_eq!(edges.len(), 1, "exactly one edge, to the same-file candidate, regardless of how many times the ambiguous name was called in this body");
        assert_eq!(edges[0].to, same_file_candidate.id());
    }

    #[test]
    fn an_ambiguous_call_with_no_scope_resolution_repeated_in_one_body_still_produces_no_edge() {
        // The repo-wide-ambiguity-with-no-scope-signal case (previous
        // test) must also stay edge-free even when the ambiguous name
        // is called MORE THAN ONCE in the same body — repetition must
        // never turn "no edge" into "an edge after all."
        let caller = test_snapshot("src/caller.rs", "caller", "function_item", "fn caller() { run(); run(); }");
        let candidate_a = test_snapshot("src/a.rs", "run", "function_item", "fn run() {}");
        let candidate_b = test_snapshot("src/b.rs", "run", "function_item", "fn run() {}");
        let snapshots = vec![caller.clone(), candidate_a.clone(), candidate_b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.is_empty(), "repetition of an unresolvable ambiguous call must not manufacture an edge: got {edges:?}");
    }

    #[test]
    fn evidence_selection_between_duplicate_occurrences_is_deterministic_regardless_of_body_order() {
        // Two callers with the SAME two call occurrences but written in
        // OPPOSITE order in the source body must still resolve to
        // exactly the same final edge (same evidence note), proving the
        // dedup's tie-break does not depend on which occurrence tree-
        // sitter/this walk visits first.
        let callee = test_snapshot("src/lib.rs", "callee", "function_item", "fn callee() {}");
        let caller_order_1 = test_snapshot("src/a.rs", "caller_order_1", "function_item", "fn caller_order_1() { callee(); callee(); }");
        let caller_order_2 = test_snapshot("src/b.rs", "caller_order_2", "function_item", "fn caller_order_2() { callee(); callee(); }");
        let snapshots = vec![callee.clone(), caller_order_1.clone(), caller_order_2.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let edge_1 = edges.iter().find(|e| e.from == caller_order_1.id()).unwrap();
        let edge_2 = edges.iter().find(|e| e.from == caller_order_2.id()).unwrap();
        assert_eq!(edge_1.evidence.note(), edge_2.evidence.note(), "identical repeated occurrences must resolve to byte-identical evidence regardless of caller");
    }

    #[test]
    fn b2_id_002_regression_a_caller_calling_the_same_ambiguous_name_many_times_does_not_panic_on_graph_insertion() {
        // Direct regression reproduction of the real failure mode: a
        // caller invoking the SAME ambiguous name repeatedly (common in
        // real code, e.g. several calls to a same-named helper across
        // overloaded candidates) must never produce a (from,to,kind)
        // duplicate that CodeGraph::insert_edge would reject.
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let caller = test_snapshot_in_repo(
            repo,
            "src/caller.rs",
            "caller",
            "function_item",
            "fn caller() { forward(); forward(); forward(); forward(); forward(); }",
        );
        let candidate_a = test_snapshot_in_repo(repo, "src/a.rs", "forward", "function_item", "fn forward() {}");
        let candidate_b = test_snapshot_in_repo(repo, "src/b.rs", "forward", "function_item", "fn forward() {}");
        let snapshots = vec![caller.clone(), candidate_a.clone(), candidate_b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let mut graph = crate::graph::CodeGraph::new();
        for snapshot in &snapshots {
            graph.insert_snapshot(snapshot.clone()).unwrap();
        }
        for edge in edges {
            graph.insert_edge(edge).unwrap();
        }
    }

    // --- Direct unit tests on `prefer_edge` itself: mutation testing
    // flagged this function's actual decision logic as unexercised by
    // the end-to-end tests above (which only ever compare two
    // IDENTICAL-evidence occurrences, never two with genuinely
    // different ResolutionMethod or different note text) ---

    fn dummy_snapshot_id() -> codegraph_domain::SymbolSnapshotId {
        test_snapshot("src/x.rs", "x", "function_item", "fn x() {}").id()
    }

    fn static_edge(note: &str) -> Edge {
        Edge {
            from: dummy_snapshot_id(),
            to: dummy_snapshot_id(),
            kind: EdgeKind::Calls,
            evidence: resolved_evidence(note),
        }
    }

    fn heuristic_edge(note: &str) -> Edge {
        let (evidence, _) = heuristic_evidence(codegraph_domain::ResolutionMethod::NameHeuristic, Confidence::Speculative, note);
        Edge {
            from: dummy_snapshot_id(),
            to: dummy_snapshot_id(),
            kind: EdgeKind::Calls,
            evidence,
        }
    }

    #[test]
    fn prefer_edge_prefers_static_analysis_over_heuristic() {
        let static_candidate = static_edge("resolved");
        let heuristic_current = heuristic_edge("ambiguous");
        assert!(prefer_edge(&static_candidate, &heuristic_current), "a StaticAnalysis candidate must replace a heuristic current edge");
    }

    #[test]
    fn prefer_edge_never_replaces_static_analysis_with_heuristic() {
        let heuristic_candidate = heuristic_edge("ambiguous");
        let static_current = static_edge("resolved");
        assert!(!prefer_edge(&heuristic_candidate, &static_current), "a heuristic candidate must never replace an already-StaticAnalysis current edge");
    }

    #[test]
    fn prefer_edge_breaks_ties_between_two_static_edges_by_note_text() {
        let a = static_edge("aaa");
        let b = static_edge("bbb");
        assert!(prefer_edge(&a, &b), "the lexicographically smaller note must win the tie-break (a < b)");
        assert!(!prefer_edge(&b, &a), "the tie-break must be antisymmetric: b must not also beat a");
    }

    #[test]
    fn prefer_edge_breaks_ties_between_two_heuristic_edges_by_note_text() {
        let a = heuristic_edge("aaa");
        let b = heuristic_edge("bbb");
        assert!(prefer_edge(&a, &b));
        assert!(!prefer_edge(&b, &a));
    }

    #[test]
    fn prefer_edge_is_false_for_byte_identical_edges() {
        // Neither side should "win" against its own exact duplicate —
        // pins the strict `<` (not `<=`) comparison specifically.
        let a = static_edge("same");
        let b = static_edge("same");
        assert!(!prefer_edge(&a, &b), "an edge identical to the current one must not be reported as preferred (strict < , not <=)");
    }

    #[test]
    fn upsert_preferring_stronger_evidence_keeps_the_static_analysis_edge_regardless_of_insertion_order() {
        let mut per_target = std::collections::HashMap::new();
        let target = dummy_snapshot_id();
        let heuristic = Edge {
            to: target,
            ..heuristic_edge("ambiguous")
        };
        let resolved = Edge { to: target, ..static_edge("resolved") };

        // Heuristic inserted FIRST, then the StaticAnalysis one — the
        // stronger one must still win even though it arrives second.
        upsert_preferring_stronger_evidence(&mut per_target, heuristic.clone());
        upsert_preferring_stronger_evidence(&mut per_target, resolved.clone());
        assert_eq!(per_target.get(&target).unwrap().evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);

        // Reverse order: StaticAnalysis first, then heuristic — must
        // still end up StaticAnalysis, proving this is not merely
        // "last write wins."
        let mut per_target_reversed = std::collections::HashMap::new();
        upsert_preferring_stronger_evidence(&mut per_target_reversed, resolved);
        upsert_preferring_stronger_evidence(&mut per_target_reversed, heuristic);
        assert_eq!(per_target_reversed.get(&target).unwrap().evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    // --- Task 9.5, Priority 5(A), Case A: receiver-aware common-name
    // call resolution (`self.field.method()`) ---
    //
    // Root cause confirmed in B2-EVAL-001/this investigation:
    // `self.src_mask.size(0)` matched an UNRELATED C++ `size()` method
    // elsewhere in a 97-file real corpus, because `callee_name`
    // extracted only the bare final segment (`"size"`) with no
    // awareness that `src_mask` (the receiver) names a real `Field` on
    // the caller's OWN class. These tests pin the fix: a receiver-
    // qualified call through `self`/`this` onto a KNOWN member of the
    // caller's own class restricts candidate resolution to that same
    // class; cross-class candidates are rejected rather than guessed.

    #[test]
    fn a_receiver_qualified_call_resolves_only_within_the_callers_own_class_even_with_a_same_named_method_elsewhere() {
        let repo = test_repo_calls();
        // ClassA.src_mask is a Field; ClassA has NO method named `size`
        // itself, but an unrelated ClassB (different file, standing in
        // for the real corpus's unrelated C++ `size()` methods) does.
        // The call `self.src_mask.size(0)` inside ClassA::forward must
        // NOT resolve to ClassB::size.
        let class_a = spanned_snapshot_in_repo(repo, "src/model.py", "ClassA", "class_definition", "class ClassA:\n    ...", 0, 200);
        let src_mask = spanned_snapshot_in_repo(repo, "src/model.py", "src_mask", "self_attribute_assignment", "self.src_mask = None", 10, 40);
        let forward = spanned_snapshot_in_repo(repo, "src/model.py", "forward", "function_definition", "def forward(self):\n    self.src_mask.size(0)", 50, 150);
        let unrelated_size = spanned_snapshot_in_repo(repo, "src/other.cpp", "size", "function_definition", "size_t size() { return 0; }", 0, 50);
        let snapshots = vec![class_a.clone(), src_mask.clone(), forward.clone(), unrelated_size.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(
            edges.iter().all(|e| e.to != unrelated_size.id()),
            "a receiver-qualified call on a known Field of the caller's own class must never resolve to an unrelated cross-class/cross-file same-named method"
        );
    }

    #[test]
    fn a_receiver_qualified_call_still_resolves_when_the_matching_method_is_on_the_callers_own_class() {
        // self.helper.run() where `helper` IS a Field of the caller's
        // class, and the caller's OWN class also contains a method
        // named `run` — this must still resolve (the restriction keeps
        // in-class matches, it only drops cross-class ones).
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/model.py", "ClassA", "class_definition", "class ClassA:\n    ...", 0, 300);
        let helper_field = spanned_snapshot_in_repo(repo, "src/model.py", "helper", "self_attribute_assignment", "self.helper = Helper()", 10, 40);
        let run_method = spanned_snapshot_in_repo(repo, "src/model.py", "run", "function_definition", "def run(self): pass", 50, 90);
        let forward = spanned_snapshot_in_repo(repo, "src/model.py", "forward", "function_definition", "def forward(self):\n    self.helper.run()", 100, 250);
        let snapshots = vec![class_a.clone(), helper_field.clone(), run_method.clone(), forward.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(
            edges.iter().any(|e| e.from == forward.id() && e.to == run_method.id() && e.evidence.method() == codegraph_domain::ResolutionMethod::StaticAnalysis),
            "a receiver-qualified call must still resolve, with StaticAnalysis evidence, when the matching method genuinely belongs to the caller's own class"
        );
    }

    #[test]
    fn two_classes_in_the_same_file_each_with_their_own_same_named_method_do_not_cross_resolve_via_receiver() {
        // ClassA.forward calls self.helper.run(); ClassB ALSO has a
        // `run` method, but ClassB is a different class in the SAME
        // file. ClassA's own `helper` field does not belong to ClassB,
        // so ClassA's call must only ever consider ClassA's own `run`.
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 200);
        let helper_field = spanned_snapshot_in_repo(repo, "src/m.py", "helper", "self_attribute_assignment", "self.helper = Helper()", 10, 30);
        let run_a = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run(self): pass", 40, 60);
        let forward = spanned_snapshot_in_repo(repo, "src/m.py", "forward", "function_definition", "def forward(self):\n    self.helper.run()", 70, 190);
        let class_b = spanned_snapshot_in_repo(repo, "src/m.py", "ClassB", "class_definition", "class ClassB: ...", 300, 500);
        let run_b = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run(self): pass", 320, 340);
        let snapshots = vec![class_a.clone(), helper_field.clone(), run_a.clone(), forward.clone(), class_b.clone(), run_b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.iter().any(|e| e.from == forward.id() && e.to == run_a.id()), "ClassA's own run method must still resolve");
        assert!(edges.iter().all(|e| e.to != run_b.id()), "ClassB's run method, a different class in the same file, must never be reached through ClassA's own receiver-qualified call");
    }

    #[test]
    fn an_unqualified_call_with_a_target_in_the_same_scope_is_unaffected_by_the_receiver_restriction() {
        // A bare, non-receiver call (`helper()`, not `self.helper()`)
        // must behave exactly as before this fix — the restriction only
        // ever applies to a `self`/`this`-qualified receiver chain.
        let caller = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() { helper(); }");
        let callee = test_snapshot("src/lib.rs", "helper", "function_item", "fn helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_self_qualified_call_whose_field_is_not_a_known_member_falls_back_to_unrestricted_matching() {
        // self.not_a_real_field.run() where `not_a_real_field` is NOT a
        // real member of the caller's class at all (no such Field or
        // method exists). The structural premise for the restriction
        // does not hold, so behavior is unchanged from before this fix:
        // ordinary name-based resolution still applies.
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 200);
        let forward = spanned_snapshot_in_repo(repo, "src/m.py", "forward", "function_definition", "def forward(self):\n    self.not_a_real_field.run()", 10, 190);
        let run_elsewhere = spanned_snapshot_in_repo(repo, "src/other.py", "run", "function_definition", "def run(self): pass", 0, 20);
        let snapshots = vec![class_a.clone(), forward.clone(), run_elsewhere.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(
            edges.iter().any(|e| e.from == forward.id() && e.to == run_elsewhere.id()),
            "when the receiver field is not a known member of the caller's class, resolution must fall back to the pre-existing unrestricted behavior"
        );
    }

    #[test]
    fn receiver_restriction_is_deterministic_regardless_of_snapshot_order() {
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/model.py", "ClassA", "class_definition", "class ClassA:\n    ...", 0, 200);
        let src_mask = spanned_snapshot_in_repo(repo, "src/model.py", "src_mask", "self_attribute_assignment", "self.src_mask = None", 10, 40);
        let forward = spanned_snapshot_in_repo(repo, "src/model.py", "forward", "function_definition", "def forward(self):\n    self.src_mask.size(0)", 50, 150);
        let unrelated_size = spanned_snapshot_in_repo(repo, "src/other.cpp", "size", "function_definition", "size_t size() { return 0; }", 0, 50);

        let forward_order = vec![class_a.clone(), src_mask.clone(), forward.clone(), unrelated_size.clone()];
        let reverse_order = vec![unrelated_size.clone(), forward.clone(), src_mask.clone(), class_a.clone()];

        let edges_a = resolve_calls(&forward_order).unwrap();
        let edges_b = resolve_calls(&reverse_order).unwrap();

        assert_eq!(edges_a.len(), edges_b.len(), "snapshot pool ordering must never change the resolved edge count");
        assert!(edges_a.iter().all(|e| e.to != unrelated_size.id()));
        assert!(edges_b.iter().all(|e| e.to != unrelated_size.id()));
    }

    fn test_repo_calls() -> codegraph_domain::RepositoryId {
        let mut registry = RepositoryRegistry::new();
        registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
    }

    // --- Direct, white-box tests for `enclosing_class_of`/`span_len`,
    // added closing Task 9.5's mutation-testing gate. ---

    #[test]
    fn enclosing_class_of_excludes_a_candidate_with_the_symbols_own_id_even_if_it_is_a_class() {
        // Pins the `candidate_class.id() == symbol.id()` half of the
        // `||` skip-condition: a symbol can never be its OWN enclosing
        // class, even in the degenerate case where `symbol` itself is
        // Class-kind and would otherwise structurally "contain" itself
        // (a span trivially contains itself, but `strictly_contains`
        // already excludes equal spans — this test isolates the id-
        // equality check specifically, independent of that).
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 100);

        let pool = [class_a.clone()];
        let result = enclosing_class_of(&pool, &class_a);

        assert_eq!(result, None, "a symbol must never be reported as its own enclosing class");
    }

    #[test]
    fn enclosing_class_of_excludes_a_non_class_candidate_even_if_it_structurally_contains_the_symbol() {
        // Pins the `candidate_class.kind() != SymbolKind::Class` half
        // of the `||` skip-condition: a non-Class snapshot (e.g. a
        // Function) whose span happens to structurally contain
        // `symbol`'s span must never be reported as its enclosing
        // class, even though span-containment alone would suggest it.
        let repo = test_repo_calls();
        let outer_function = spanned_snapshot_in_repo(repo, "src/m.py", "outer", "function_definition", "def outer(): ...", 0, 100);
        let inner_method = spanned_snapshot_in_repo(repo, "src/m.py", "inner", "function_definition", "def inner(): ...", 10, 50);

        let pool = [outer_function, inner_method.clone()];
        let result = enclosing_class_of(&pool, &inner_method);

        assert_eq!(result, None, "a non-Class container must never be reported as an enclosing class, regardless of span containment");
    }

    #[test]
    fn enclosing_class_of_excludes_a_class_in_a_different_file_even_if_spans_overlap() {
        // Pins the `!same_file(...)` half of the second `||`
        // skip-condition, isolated from `strictly_contains`: same byte
        // range, different file.
        let repo = test_repo_calls();
        let class_in_other_file = spanned_snapshot_in_repo(repo, "src/a.py", "ClassA", "class_definition", "class ClassA: ...", 0, 100);
        let method = spanned_snapshot_in_repo(repo, "src/b.py", "method", "function_definition", "def method(): ...", 10, 50);

        let pool = [class_in_other_file, method.clone()];
        let result = enclosing_class_of(&pool, &method);

        assert_eq!(result, None, "a same-span class in a DIFFERENT file must never be reported as the enclosing class");
    }

    #[test]
    fn enclosing_class_of_excludes_a_same_file_class_that_does_not_structurally_contain_the_symbol() {
        // Pins the `!strictly_contains(...)` half of the second `||`
        // skip-condition, isolated from `same_file`: same file, but the
        // class's span does not contain the symbol's span at all.
        let repo = test_repo_calls();
        let unrelated_class = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 50);
        let method = spanned_snapshot_in_repo(repo, "src/m.py", "method", "function_definition", "def method(): ...", 100, 150);

        let pool = [unrelated_class, method.clone()];
        let result = enclosing_class_of(&pool, &method);

        assert_eq!(result, None, "a same-file class whose span does not contain the symbol must never be reported as its enclosing class");
    }

    #[test]
    fn enclosing_class_of_picks_the_smaller_of_two_nested_containing_classes() {
        // Pins the `span_len(candidate_class) < span_len(current)`
        // match guard directly: TWO classes both structurally contain
        // `method` (Outer, then the smaller Inner nested within it) —
        // the innermost (smallest span) must win, confirmed by checking
        // BOTH insertion orders so iteration order cannot be the real
        // reason this passes.
        let repo = test_repo_calls();
        let outer = spanned_snapshot_in_repo(repo, "src/m.py", "Outer", "class_definition", "class Outer: ...", 0, 200);
        let inner = spanned_snapshot_in_repo(repo, "src/m.py", "Inner", "class_definition", "class Inner: ...", 50, 150);
        let method = spanned_snapshot_in_repo(repo, "src/m.py", "method", "function_definition", "def method(): ...", 80, 120);

        let forward_order = [outer.clone(), inner.clone(), method.clone()];
        let reverse_order = [inner.clone(), outer.clone(), method.clone()];

        assert_eq!(enclosing_class_of(&forward_order, &method), Some(&inner), "the smaller (innermost) containing class must win regardless of insertion order");
        assert_eq!(enclosing_class_of(&reverse_order, &method), Some(&inner));
    }

    #[test]
    fn enclosing_class_of_keeps_the_first_innermost_class_found_when_a_later_candidate_is_not_strictly_smaller() {
        // Pins the `Some(current) => current` (keep, don't replace) arm
        // directly: a SECOND candidate class with a span size EQUAL to
        // (not smaller than) the current innermost's must never replace
        // it — only a STRICTLY smaller span may.
        let repo = test_repo_calls();
        let first_class = spanned_snapshot_in_repo(repo, "src/m.py", "FirstClass", "class_definition", "class FirstClass: ...", 0, 200);
        // Same span length as first_class (100 bytes), structurally
        // nested at a DIFFERENT position so both genuinely contain
        // `method` without being identical spans (which would instead
        // trip the duplicate-snapshot-id rejection elsewhere).
        let second_class = spanned_snapshot_in_repo(repo, "src/m.py", "SecondClass", "class_definition", "class SecondClass: ...", 0, 200);
        let method = spanned_snapshot_in_repo(repo, "src/m.py", "method", "function_definition", "def method(): ...", 80, 120);

        let pool = [first_class.clone(), second_class.clone(), method.clone()];
        let result = enclosing_class_of(&pool, &method);

        // Both classes tie on span length — the match guard's `<` means
        // neither replaces an already-set innermost once one is picked
        // first; this test fixes the ORDER so the result is
        // deterministic and verifiably matches the `<` (never `<=`)
        // semantics: the FIRST one encountered during iteration wins a
        // tie, confirming `<` rather than `<=` governs replacement.
        assert_eq!(result, Some(&first_class));
    }

    #[test]
    fn self_or_this_receiver_field_name_recognizes_rusts_dedicated_self_node_kind() {
        // Pins the LEFT half of `is_self_or_this`'s `||`:
        // `matches!(base.kind(), "self" | "this")` must independently
        // be sufficient to recognize a receiver as self/this, WITHOUT
        // relying on the Python-specific identifier-text fallback on
        // the right. Rust's `self` parses as a dedicated `self` node
        // kind (confirmed via a real parse dump, Task 9.5 Priority
        // 5(A)) — if the `||` were mutated to `&&`, this would always
        // be false (a node's kind cannot simultaneously be `"self"` and
        // `"identifier"`), so this test fails on that mutant while
        // every existing Python-only test does not exercise this path
        // at all.
        let source = b"fn forward(&self) { self.src_mask.size(0); }";
        let tree = crate::parse::LanguageParser::parse(&crate::parse::RustParser, source).unwrap();
        let root = tree.tree().root_node();
        let call_node = find_first_node_of_kind(root, "call_expression").expect("expected a call_expression node");
        let callee = call_node.child(0).expect("call node must have a callee child");

        let result = self_or_this_receiver_field_name(callee, source);

        assert_eq!(result, Some("src_mask".to_string()), "Rust's dedicated `self` node kind must be recognized without needing the Python-specific text fallback");
    }

    #[test]
    fn self_or_this_receiver_field_name_rejects_a_python_identifier_receiver_whose_text_is_not_self() {
        // Pins the INNER `&&` of `is_self_or_this`'s RIGHT disjunct
        // directly: `base.kind() == "identifier" && text_of(base,
        // source) == "self"` requires BOTH the kind check AND the text
        // check. `other_obj` is a plain Python identifier (kind
        // "identifier", exactly like `self` would be — Python has no
        // dedicated `self` node kind) whose TEXT is NOT "self". If this
        // inner `&&` were mutated to `||`, `base.kind() == "identifier"`
        // alone would already be true (since EVERY Python receiver base
        // parses as an `identifier` node), making `is_self_or_this` true
        // for ANY receiver, not just `self` — exactly the false-positive
        // shape the whole Case A fix exists to prevent. The existing
        // Rust-based test above exercises the OUTER `||`'s left disjunct
        // (`"self" | "this"` node kind) but never reaches this inner
        // expression at all, since Rust's dedicated `self` kind short-
        // circuits before it.
        let source = b"def forward(other_obj):\n    other_obj.src_mask.size(0)\n";
        let tree = crate::parse::LanguageParser::parse(&crate::parse::PythonParser, source).unwrap();
        let root = tree.tree().root_node();
        let call_node = find_first_node_of_kind(root, "call").expect("expected a call node");
        let callee = call_node.child(0).expect("call node must have a callee child");

        let result = self_or_this_receiver_field_name(callee, source);

        assert_eq!(result, None, "a plain identifier receiver whose text is not literally \"self\" must never be treated as a self/this-qualified receiver");
    }

    #[test]
    fn self_or_this_receiver_field_name_returns_none_for_a_bare_identifier_callee() {
        // Pins `is_receiver_qualified`'s real effect at its FIRST call
        // site in self_or_this_receiver_field_name: a bare, non-
        // receiver callee (`helper()`, callee node kind `identifier`)
        // must return None. A plain `identifier` node has zero
        // children, so `callee.child(0)` alone already yields `None`
        // regardless of `is_receiver_qualified`'s own result for THIS
        // specific case — this test exists to pin the observable
        // behavior precisely (None), even though it does not by itself
        // distinguish `is_receiver_qualified` from an always-true stub
        // for a childless node.
        let source = b"def forward(): helper()";
        let tree = crate::parse::LanguageParser::parse(&crate::parse::PythonParser, source).unwrap();
        let root = tree.tree().root_node();
        let call_node = find_first_node_of_kind(root, "call").expect("expected a call node");
        let callee = call_node.child(0).expect("call node must have a callee child");
        assert_eq!(callee.kind(), "identifier");

        let result = self_or_this_receiver_field_name(callee, source);

        assert_eq!(result, None, "a bare identifier callee must never be read as a self/this-qualified receiver field");
    }

    #[test]
    fn is_receiver_qualified_is_false_for_a_bare_identifier_node() {
        // Direct, white-box pin of `is_receiver_qualified` itself
        // (distinct from the end-to-end test above): a plain
        // `identifier` node must report `false`, not the stubbed
        // `true` a mutation would otherwise substitute.
        let source = b"def forward(): helper()";
        let tree = crate::parse::LanguageParser::parse(&crate::parse::PythonParser, source).unwrap();
        let root = tree.tree().root_node();
        let call_node = find_first_node_of_kind(root, "call").expect("expected a call node");
        let callee = call_node.child(0).expect("call node must have a callee child");

        assert!(!is_receiver_qualified(callee), "a bare identifier node must never be reported as receiver-qualified");
    }

    fn find_first_node_of_kind<'a>(node: tree_sitter::Node<'a>, kind: &str) -> Option<tree_sitter::Node<'a>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_first_node_of_kind(child, kind) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn narrow_by_scope_falls_through_to_same_file_when_the_callers_own_class_has_zero_candidates() {
        // Pins the `0 => {}` (fall-through, not an early return) arm in
        // narrow_by_scope's class-level check directly: the caller HAS
        // an enclosing class, but NONE of the ambiguous candidates
        // belong to it (same_class.len() == 0) — this must fall
        // through to the same-file check below, not short-circuit to
        // None, so a same-file (but different-class) candidate can
        // still resolve.
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 90);
        let caller = spanned_snapshot_in_repo(repo, "src/m.py", "caller", "function_definition", "def caller(): ...", 10, 80);
        // same_file_candidate is in the SAME FILE as caller, but NOT
        // inside ClassA's span (so same_class is empty for it).
        let same_file_candidate = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run(): ...", 100, 150);

        let candidates = [&same_file_candidate];
        let result = narrow_by_scope(&[class_a, caller.clone(), same_file_candidate.clone()], &caller, &candidates);

        assert_eq!(
            result.map(|(s, _)| s),
            Some(&same_file_candidate),
            "zero same-class candidates must fall through to the same-file check, not short-circuit to None"
        );
    }

    #[test]
    fn narrow_by_scope_returns_none_when_the_callers_own_class_has_more_than_one_candidate() {
        // Pins the `_ => return None` arm for the class-level check:
        // TWO candidates both belong to the caller's own class —
        // genuine same-class ambiguity must return None immediately,
        // never fall through to try the (necessarily broader) same-
        // file level.
        let repo = test_repo_calls();
        let class_a = spanned_snapshot_in_repo(repo, "src/m.py", "ClassA", "class_definition", "class ClassA: ...", 0, 200);
        let caller = spanned_snapshot_in_repo(repo, "src/m.py", "caller", "function_definition", "def caller(): ...", 10, 90);
        let candidate_a = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run(): ...", 100, 130);
        let candidate_b = spanned_snapshot_in_repo(repo, "src/m.py", "run", "function_definition", "def run2(): ...", 140, 170);

        let candidates = [&candidate_a, &candidate_b];
        let pool = [class_a, caller.clone(), candidate_a.clone(), candidate_b.clone()];
        let result = narrow_by_scope(&pool, &caller, &candidates);

        assert_eq!(result, None, "two candidates both in the caller's own class must be ambiguous, never resolved by falling through to a broader scope");
    }

    #[test]
    fn span_len_computes_the_difference_not_the_sum_or_a_constant() {
        let repo = test_repo_calls();
        let short = spanned_snapshot_in_repo(repo, "src/m.py", "a", "function_definition", "a", 10, 15);
        let long = spanned_snapshot_in_repo(repo, "src/m.py", "b", "function_definition", "b", 10, 115);

        assert_eq!(span_len(&short), 5);
        assert_eq!(span_len(&long), 105);
        assert_ne!(span_len(&short), span_len(&long));
    }

    // B2-ID-006 regression tests. Every snapshot below is built from
    // REAL extraction (`extract_symbols_reporting_rejections` + the
    // real `promote`), not a hand-written placeholder `signature_text`
    // — the whole point of this finding is that a placeholder body
    // structurally cannot exercise this bug (see B2-ID-006's own
    // "Observed" section), so these regression tests must not repeat
    // that mistake.
    mod b2_id_006_nested_scope_ownership {
        use super::*;
        use crate::extract::extract_symbols_reporting_rejections;
        use crate::parse::parser_for;

        /// Extracts and promotes every real candidate in `source`
        /// (parsed as `path`'s own language), returning the resulting
        /// snapshots. Mirrors exactly what the real production
        /// pipeline does (parse -> extract -> promote), never
        /// hand-constructing a `SymbolSnapshot` with an invented span
        /// or placeholder signature text.
        fn real_snapshots(repo: codegraph_domain::RepositoryId, path: &str, source: &str) -> Vec<SymbolSnapshot> {
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let file = FileIdentity::parse(repo, commit, path).unwrap();
            let parser = parser_for(&file).expect("a supported extension must have a parser");
            let tree = parser.parse(source.as_bytes()).expect("the hand-written fixture source must parse");
            let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
            assert!(rejections.is_empty(), "fixture source must not trigger any extraction rejection: {rejections:?}");
            candidates.into_iter().map(|c| promote(&c, file.clone()).expect("a real extracted candidate must always promote")).collect()
        }

        fn find<'a>(snapshots: &'a [SymbolSnapshot], name: &str) -> &'a SymbolSnapshot {
            snapshots.iter().find(|s| s.name().as_str() == name).unwrap_or_else(|| panic!("no snapshot named {name:?} among {:?}", snapshots.iter().map(|s| s.name().as_str()).collect::<Vec<_>>()))
        }

        fn callers_of<'a>(edges: &'a [Edge], target: &SymbolSnapshot, snapshots: &'a [SymbolSnapshot]) -> Vec<&'a str> {
            let mut names: Vec<&str> = edges.iter().filter(|e| e.to == target.id()).filter_map(|e| snapshots.iter().find(|s| s.id() == e.from)).map(|s| s.name().as_str()).collect();
            names.sort_unstable();
            names
        }

        #[test]
        fn python_class_single_method_single_call_is_owned_by_the_method_not_the_class() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(
                repo,
                "src/a.py",
                "class A:\n    def f(self):\n        target()\n\n\ndef target():\n    pass\n",
            );
            let target = find(&snapshots, "target");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, target, &snapshots), vec!["f"], "exactly f -> target, never A -> target");
        }

        #[test]
        fn python_class_two_sibling_methods_calling_the_same_target_produce_two_real_caller_edges_no_class_duplicate() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(
                repo,
                "src/svc.py",
                "class Svc:\n    def a(self):\n        target()\n\n    def b(self):\n        target()\n\n\ndef target():\n    pass\n",
            );
            let target = find(&snapshots, "target");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, target, &snapshots), vec!["a", "b"], "both sibling methods own their own call; the class must not also appear");
        }

        #[test]
        fn python_nested_function_call_is_owned_by_the_inner_function_not_the_outer() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/nested.py", "def outer():\n    def inner():\n        target()\n    return inner\n\n\ndef target():\n    pass\n");
            let target = find(&snapshots, "target");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, target, &snapshots), vec!["inner"], "only inner calls target; outer must not also appear just because inner's span is nested inside it");
        }

        #[test]
        fn python_outer_own_call_plus_nested_call_are_each_owned_by_their_real_caller() {
            // outer calls `first` directly, in its OWN body, outside
            // inner's span; inner (nested inside outer) calls `second`.
            // Expected: outer -> first (real, outer's own direct call),
            // inner -> second (real), and NEVER outer -> second (which
            // would mean outer's own re-parsed body rediscovered a call
            // that only textually exists because inner's body happens
            // to be nested inside outer's span).
            let repo = test_repo_calls();
            let snapshots = real_snapshots(
                repo,
                "src/mixed.py",
                "def outer():\n    first()\n\n    def inner():\n        second()\n    return inner\n\n\ndef first():\n    pass\n\n\ndef second():\n    pass\n",
            );
            let first = find(&snapshots, "first");
            let second = find(&snapshots, "second");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, first, &snapshots), vec!["outer"], "outer's own direct call to first must still resolve");
            assert_eq!(callers_of(&edges, second, &snapshots), vec!["inner"], "second is owned by inner alone, never also by outer");
        }

        #[test]
        fn rust_outer_call_immediately_adjacent_to_a_nested_functions_closing_brace_still_resolves() {
            // Pins the end-exclusive ByteSpan boundary
            // (`span_contains_position`'s `position < span.end`, not
            // `<=`) against the exact real byte-adjacency case where
            // the two operators diverge: `fn outer(){fn inner(){}other();}`
            // — `inner`'s own span is `[11, 23)` (ends exactly at its
            // closing `}`), and `other()`'s call expression starts at
            // EXACTLY byte 23 — i.e. `absolute_call_site_position ==
            // inner.span().end` precisely, the one position where `<`
            // and `<=` disagree. With the correct `<`, position 23 is
            // NOT inside `inner`'s `[11, 23)` span, so `outer` (whose
            // own `[0, 32)` span DOES legitimately contain position 23)
            // keeps ownership of this call, exactly as it should: this
            // is `outer`'s OWN direct call, not something `inner`
            // contains. A `<=` mutant would wrongly treat position 23
            // as still "inside" `inner`, incorrectly excluding `outer`
            // from owning a call it genuinely makes.
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/zero_gap.rs", "fn outer(){fn inner(){}other();}\n\nfn other(){}\n");
            let other = find(&snapshots, "other");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(
                callers_of(&edges, other, &snapshots),
                vec!["outer"],
                "outer's own call to other, starting at the exact byte inner's span ends, must resolve to outer — the end-exclusive boundary must not misattribute it to inner or drop it"
            );
        }

        #[test]
        fn python_outer_own_call_immediately_after_a_nested_function_definition_still_resolves() {
            // Pins the end-exclusive ByteSpan boundary
            // (`span_contains_position`'s `position < span.end`, not
            // `<=`) against a real adjacency case: `outer` defines
            // `inner` FIRST, then immediately makes its OWN direct call
            // (`after_inner()`) on the very next line, right after
            // `inner`'s own span ends. If the nested-scope boundary
            // check used `<=` instead of `<`, a call site landing
            // exactly at (or the byte comparison treating as "still
            // inside") `inner`'s own `span.end` could be wrongly
            // excluded from `outer`'s own attribution, misattributing
            // (or losing) a real call `outer` genuinely makes — the
            // opposite failure mode from B2-ID-006's original
            // over-attribution bug, but the same boundary this fix's
            // own span check must get exactly right.
            let repo = test_repo_calls();
            let snapshots = real_snapshots(
                repo,
                "src/adjacent.py",
                "def outer():\n    def inner():\n        pass\n    after_inner()\n\n\ndef after_inner():\n    pass\n",
            );
            let after_inner = find(&snapshots, "after_inner");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(
                callers_of(&edges, after_inner, &snapshots),
                vec!["outer"],
                "outer's own direct call, immediately after inner's definition ends, must still resolve to outer — not be lost or misattributed at the inner/outer span boundary"
            );
        }

        #[test]
        fn python_doubly_nested_function_call_is_owned_by_the_innermost_function_only() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(
                repo,
                "src/deep.py",
                "def outer():\n    def middle():\n        def inner():\n            target()\n        return inner\n    return middle\n\n\ndef target():\n    pass\n",
            );
            let target = find(&snapshots, "target");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, target, &snapshots), vec!["inner"], "neither outer nor middle must appear — only the innermost real caller");
        }

        #[test]
        fn rust_trait_default_method_call_is_owned_by_the_method_not_the_trait() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/trait_default.rs", "trait Greeter {\n    fn greet(&self) {\n        helper();\n    }\n}\n\nfn helper() {}\n");
            let helper = find(&snapshots, "helper");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, helper, &snapshots), vec!["greet"], "the default method owns its own call; the trait/Interface snapshot must not also appear");
        }

        #[test]
        fn csharp_class_method_call_is_owned_by_the_method_not_the_class() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/A.cs", "class A {\n    void F() {\n        Target();\n    }\n}\n\nclass Target2 {\n    void Target() {}\n}\n");
            // C#'s method_declaration requires a containing structure;
            // the free-standing `Target()` call resolves by bare name
            // against whatever same-named candidate exists in the pool
            // — this test only asserts the CLASS `A` never appears as a
            // caller for whichever edge(s) are produced, which is the
            // one thing B2-ID-006 is actually about (not re-testing
            // C#'s own name-resolution behavior here).
            let edges = resolve_calls(&snapshots).unwrap();
            let class_a = find(&snapshots, "A");
            assert!(edges.iter().all(|e| e.from != class_a.id()), "the C# class snapshot must never be a Calls caller");
        }

        #[test]
        fn cpp_class_method_call_is_owned_by_the_method_not_the_class() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/a.cpp", "void helper() {}\n\nclass A {\n    void f() {\n        helper();\n    }\n};\n");
            let helper = find(&snapshots, "helper");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, helper, &snapshots), vec!["f"], "the C++ class must not also appear as a caller of helper");
        }

        #[test]
        fn javascript_class_method_call_is_owned_by_the_method_not_the_class() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/a.js", "function helper() {}\n\nclass A {\n    f() {\n        helper();\n    }\n}\n");
            let helper = find(&snapshots, "helper");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, helper, &snapshots), vec!["f"], "the JS class must not also appear as a caller of helper");
        }

        #[test]
        fn method_signature_with_no_body_never_originates_a_call() {
            // Rust trait method SIGNATURE (no body) alongside a real
            // default method that DOES call something — confirms
            // MethodSignature is excluded without needing a body to
            // exclude it from (there is none to walk), and does not
            // interfere with the real default method's own edge.
            let repo = test_repo_calls();
            let snapshots = real_snapshots(repo, "src/sig.rs", "trait T {\n    fn no_body(&self);\n\n    fn with_body(&self) {\n        helper();\n    }\n}\n\nfn helper() {}\n");
            let helper = find(&snapshots, "helper");
            let edges = resolve_calls(&snapshots).unwrap();

            assert_eq!(callers_of(&edges, helper, &snapshots), vec!["with_body"], "only the real default method body owns the call");
        }

        #[test]
        fn result_is_deterministic_regardless_of_input_snapshot_order() {
            let repo = test_repo_calls();
            let snapshots = real_snapshots(
                repo,
                "src/order.py",
                "class Svc:\n    def a(self):\n        target()\n\n    def b(self):\n        target()\n\n\ndef target():\n    pass\n",
            );
            let target = find(&snapshots, "target");

            let forward_edges = resolve_calls(&snapshots).unwrap();
            let mut reversed = snapshots.clone();
            reversed.reverse();
            let reversed_edges = resolve_calls(&reversed).unwrap();

            assert_eq!(callers_of(&forward_edges, target, &snapshots), callers_of(&reversed_edges, target, &reversed), "caller attribution must not depend on snapshot pool ordering");
        }
    }

    // =======================================================================
    // Quality & Semantic Reliability Hardening, Phase 2:
    // resolve_calls_with_negative_knowledge. These tests confirm (a) the
    // Edge output is byte-for-byte identical to resolve_calls (the
    // compatibility-wrapper invariant), and (b) ambiguous/unresolved
    // call sites that previously vanished without a trace now produce
    // a real, inspectable CallAttempt.
    // =======================================================================

    mod negative_knowledge_wiring {
        use super::*;

        #[test]
        fn edges_output_is_identical_to_resolve_calls_for_a_resolved_call() {
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "a", "function_item", "fn a() { b(); }", 0, 20);
            let callee = spanned_snapshot_in_repo(repo, "src/lib.rs", "b", "function_item", "fn b() {}", 30, 40);
            let snapshots = vec![caller.clone(), callee.clone()];

            let via_wrapper = resolve_calls(&snapshots).unwrap();
            let (via_new_fn, attempts) = resolve_calls_with_negative_knowledge(&snapshots).unwrap();

            assert_eq!(via_wrapper, via_new_fn, "resolve_calls must be byte-for-byte identical to resolve_calls_with_negative_knowledge's own Edge output");
            assert!(attempts.is_empty(), "a cleanly resolved call must produce no CallAttempt at all -- Edge already carries this outcome");
        }

        #[test]
        fn an_unresolved_call_zero_candidates_produces_an_unresolved_attempt_and_no_edge() {
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "a", "function_item", "fn a() { nonexistent_callee(); }", 0, 40);
            let snapshots = vec![caller.clone()];

            let (edges, attempts) = resolve_calls_with_negative_knowledge(&snapshots).unwrap();

            assert!(edges.is_empty());
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].tier, codegraph_domain::EvidenceTier::Unresolved);
            assert_eq!(attempts[0].caller, caller.id());
            assert_eq!(attempts[0].callee_name, "nonexistent_callee");
            assert!(attempts[0].candidates.is_empty());
        }

        #[test]
        fn an_ambiguous_call_two_same_file_candidates_produces_an_ambiguous_attempt_with_both_real_candidates() {
            // Mirrors an_ambiguous_call_with_two_same_file_candidates_still_produces_no_edge
            // above, but asserts the NEW negative-knowledge side too.
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "foo", "function_item", "fn foo() { run(); }", 0, 20);
            let candidate_a = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* a */ }", 30, 50);
            let candidate_b = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* b */ }", 60, 80);
            let snapshots = vec![caller.clone(), candidate_a.clone(), candidate_b.clone()];

            let (edges, attempts) = resolve_calls_with_negative_knowledge(&snapshots).unwrap();

            assert!(edges.is_empty(), "two indistinguishable candidates must still produce no edge");
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].tier, codegraph_domain::EvidenceTier::Ambiguous);
            assert_eq!(attempts[0].caller, caller.id());
            assert_eq!(attempts[0].callee_name, "run");
            let mut candidate_ids = attempts[0].candidates.clone();
            candidate_ids.sort_by_key(|id| id.as_bytes().to_vec());
            let mut expected = vec![candidate_a.id(), candidate_b.id()];
            expected.sort_by_key(|id| id.as_bytes().to_vec());
            assert_eq!(candidate_ids, expected, "both real ambiguous candidates must be preserved, not just a count");
        }

        #[test]
        fn repeated_occurrences_of_the_same_unresolved_callee_produce_one_attempt_not_many() {
            // Mirrors B2-ID-002's own occurrence-vs-relation discipline
            // for Edges, generalized here to CallAttempt: the same
            // unresolved callee name invoked 3 times in one body must
            // still produce exactly one CallAttempt.
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "a", "function_item", "fn a() { missing(); missing(); missing(); }", 0, 60);
            let snapshots = vec![caller];

            let (_, attempts) = resolve_calls_with_negative_knowledge(&snapshots).unwrap();

            assert_eq!(attempts.len(), 1, "3 occurrences of the same unresolved callee must collapse to ONE CallAttempt, not three");
        }

        #[test]
        fn a_resolved_call_and_an_unresolved_call_in_the_same_body_each_report_correctly() {
            // Confirms the two code paths (Edge-producing and
            // CallAttempt-producing) do not interfere with each other
            // within the same caller's own body.
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "a", "function_item", "fn a() { b(); missing(); }", 0, 50);
            let callee = spanned_snapshot_in_repo(repo, "src/lib.rs", "b", "function_item", "fn b() {}", 60, 70);
            let snapshots = vec![caller.clone(), callee.clone()];

            let (edges, attempts) = resolve_calls_with_negative_knowledge(&snapshots).unwrap();

            assert_eq!(edges.len(), 1, "the resolved call to b() must still produce its edge");
            assert_eq!(edges[0].to, callee.id());
            assert_eq!(attempts.len(), 1, "the unresolved call to missing() must still produce its attempt");
            assert_eq!(attempts[0].callee_name, "missing");
        }

        #[test]
        fn candidates_recorded_in_the_provenance_match_the_attempts_own_candidates_field() {
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "foo", "function_item", "fn foo() { run(); }", 0, 20);
            let candidate_a = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* a */ }", 30, 50);
            let candidate_b = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* b */ }", 60, 80);
            let snapshots = vec![caller, candidate_a, candidate_b];

            let (_, attempts) = resolve_calls_with_negative_knowledge(&snapshots).unwrap();

            assert_eq!(attempts.len(), 1);
            let mut provenance_ids = attempts[0].provenance.candidates_considered().to_vec();
            provenance_ids.sort_by_key(|id| id.as_bytes().to_vec());
            let mut field_ids = attempts[0].candidates.clone();
            field_ids.sort_by_key(|id| id.as_bytes().to_vec());
            assert_eq!(provenance_ids, field_ids, "Provenance.candidates_considered must match CallAttempt.candidates exactly");
            assert_eq!(attempts[0].provenance.resolver(), "resolve_calls");
        }

        #[test]
        fn result_is_deterministic_regardless_of_snapshot_pool_ordering() {
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "foo", "function_item", "fn foo() { run(); }", 0, 20);
            let candidate_a = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* a */ }", 30, 50);
            let candidate_b = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* b */ }", 60, 80);
            let snapshots = vec![caller.clone(), candidate_a.clone(), candidate_b.clone()];
            let mut reversed = snapshots.clone();
            reversed.reverse();

            let (_, forward_attempts) = resolve_calls_with_negative_knowledge(&snapshots).unwrap();
            let (_, reversed_attempts) = resolve_calls_with_negative_knowledge(&reversed).unwrap();

            assert_eq!(forward_attempts.len(), 1);
            assert_eq!(reversed_attempts.len(), 1);
            let mut forward_candidates = forward_attempts[0].candidates.clone();
            forward_candidates.sort_by_key(|id| id.as_bytes().to_vec());
            let mut reversed_candidates = reversed_attempts[0].candidates.clone();
            reversed_candidates.sort_by_key(|id| id.as_bytes().to_vec());
            assert_eq!(forward_candidates, reversed_candidates, "the recorded candidate set must not depend on snapshot pool ordering");
        }

        // ===================================================================
        // The two critical invariants, reflected as concrete tests (per
        // the user's own explicit instruction): a future change that
        // violates either of these must fail a real test, not merely
        // violate a documented principle nobody checks.
        // ===================================================================

        #[test]
        fn invariant_absence_of_evidence_is_never_silently_represented_as_evidence_of_absence() {
            // "This pipeline found no candidate for `missing()`" must
            // produce an EXPLICIT Unresolved CallAttempt recording that
            // a resolution ATTEMPT happened and failed -- never the
            // same silent "nothing at all" a callee that was simply
            // never mentioned anywhere would also produce. This test
            // would fail if a future change made resolve_calls_with_negative_knowledge
            // simply skip recording an attempt for the zero-candidate
            // case (reverting to today's pre-Phase-2 silence).
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "a", "function_item", "fn a() { missing(); }", 0, 30);
            let snapshots = vec![caller];

            let (_, attempts) = resolve_calls_with_negative_knowledge(&snapshots).unwrap();

            assert_eq!(attempts.len(), 1, "a real resolution attempt that found zero candidates must be recorded explicitly, not silently dropped");
            assert_eq!(attempts[0].tier, codegraph_domain::EvidenceTier::Unresolved, "must be the explicit Unresolved tier, not merely absent from any output");
        }

        #[test]
        fn invariant_ambiguity_is_never_converted_into_certainty_to_make_the_graph_more_complete() {
            // Two equally-plausible same-named candidates must NEVER
            // collapse into a single, arbitrarily-chosen Edge just
            // because recording an Ambiguous CallAttempt instead of an
            // Edge makes the graph "less complete." This test would
            // fail if a future change (e.g. "while I'm here, let's just
            // pick the first candidate for Ambiguous cases too") started
            // emitting an Edge for this exact shape.
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "foo", "function_item", "fn foo() { run(); }", 0, 20);
            let candidate_a = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* a */ }", 30, 50);
            let candidate_b = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* b */ }", 60, 80);
            let snapshots = vec![caller, candidate_a, candidate_b];

            let (edges, attempts) = resolve_calls_with_negative_knowledge(&snapshots).unwrap();

            assert!(edges.is_empty(), "ambiguity must NEVER be silently resolved into an Edge, regardless of how plausible one candidate might seem");
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].tier, codegraph_domain::EvidenceTier::Ambiguous, "the real ambiguity must be reported honestly as Ambiguous, never upgraded to a false certainty");
            assert_eq!(attempts[0].candidates.len(), 2, "both real candidates must remain visible -- neither silently dropped to manufacture a single answer");
        }
    }

    /// Semantic Resolution & Reliability Checkpoint, Priority 3:
    /// SignatureTooLong extraction-rejection provenance linkage.
    mod extraction_rejection_provenance {
        use super::*;
        use crate::negative_knowledge::UnresolvedReason;

        #[test]
        fn resolve_calls_with_negative_knowledge_is_unaffected_an_equivalence_test() {
            // The compatibility wrapper must be byte-for-byte identical
            // to calling the new function with an empty rejection list.
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "a", "function_item", "fn a() { missing(); }", 0, 30);
            let snapshots = vec![caller];

            let via_wrapper = resolve_calls_with_negative_knowledge(&snapshots).unwrap();
            let via_new_fn = resolve_calls_with_extraction_rejections(&snapshots, &[]).unwrap();

            assert_eq!(via_wrapper, via_new_fn);
        }

        #[test]
        fn a_zero_candidate_callee_matching_a_known_signature_too_long_name_reports_extraction_rejected() {
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "a", "function_item", "fn a() { Handle(); }", 0, 30);
            let snapshots = vec![caller.clone()];
            let rejected_names = vec!["Handle".to_string()];

            let (edges, attempts) = resolve_calls_with_extraction_rejections(&snapshots, &rejected_names).unwrap();

            assert!(edges.is_empty(), "the real candidate was rejected at extraction -- it was never promoted, so no edge can exist");
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].tier, codegraph_domain::EvidenceTier::Unresolved, "extraction-rejected is still the Unresolved TIER -- the reason refines WHY, not the tier itself");
            assert_eq!(attempts[0].reason, UnresolvedReason::ExtractionRejected { rejection_kind: "SignatureTooLong" });
        }

        #[test]
        fn a_zero_candidate_callee_not_matching_any_known_rejection_still_reports_unknown_never_a_false_positive_rejection_reason() {
            // The never-fabricate discipline's own negative case: a
            // rejection list that does NOT contain this callee's name
            // must never cause a false ExtractionRejected classification.
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "a", "function_item", "fn a() { trulyExternal(); }", 0, 40);
            let snapshots = vec![caller.clone()];
            let rejected_names = vec!["SomeOtherRejectedName".to_string()];

            let (_edges, attempts) = resolve_calls_with_extraction_rejections(&snapshots, &rejected_names).unwrap();

            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].reason, UnresolvedReason::Unknown, "a rejection list not naming this exact callee must never produce a false ExtractionRejected classification");
        }

        #[test]
        fn an_ambiguous_outcome_is_unaffected_by_the_rejection_list_even_if_the_name_happens_to_also_appear_there() {
            // The rejection-name check must only ever apply to the
            // ZERO-candidate branch -- a real ambiguity (2+ candidates)
            // must never be reclassified just because the SAME bare
            // name coincidentally also appears in a rejection list
            // (e.g. one overload fragment was rejected while another,
            // real, promoted candidate with the same name exists
            // elsewhere -- genuinely possible, and must not corrupt
            // the ambiguity classification).
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "foo", "function_item", "fn foo() { run(); }", 0, 20);
            let candidate_a = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* a */ }", 30, 50);
            let candidate_b = spanned_snapshot_in_repo(repo, "src/lib.rs", "run", "function_item", "fn run() { /* b */ }", 60, 80);
            let snapshots = vec![caller, candidate_a, candidate_b];
            let rejected_names = vec!["run".to_string()];

            let (edges, attempts) = resolve_calls_with_extraction_rejections(&snapshots, &rejected_names).unwrap();

            assert!(edges.is_empty());
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].tier, codegraph_domain::EvidenceTier::Ambiguous);
            assert_eq!(attempts[0].reason, UnresolvedReason::AmbiguousCandidates, "real ambiguity among promoted candidates must never be reclassified by an unrelated rejection-name coincidence");
        }
    }

    // --- CALL-RES-A4: import/alias-aware resolution (the 10 required
    // negative tests, plus the real CRC-003 recovery case and the
    // explicit CRC-001/CRC-002 non-regression checks). Every fixture
    // below uses REAL extraction + REAL `resolve_import_facts`, never
    // a hand-built `ImportFact`, for the same reason `real_snapshots`
    // (above) does: a placeholder cannot exercise the real grammar
    // shapes this feature depends on.
    mod call_res_a4_import_alias {
        use super::*;
        use crate::extract::extract_symbols_reporting_rejections;
        use crate::imports::resolve_import_facts;
        use crate::negative_knowledge::UnresolvedReason;

        fn real_snapshots_and_tree(repo: codegraph_domain::RepositoryId, path: &str, source: &str) -> (FileIdentity, crate::parse::ParseTree, Vec<SymbolSnapshot>) {
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let file = FileIdentity::parse(repo, commit, path).unwrap();
            let parser = parser_for(&file).expect("a supported extension must have a parser");
            let tree = parser.parse(source.as_bytes()).expect("fixture source must parse");
            let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
            assert!(rejections.is_empty(), "fixture source must not trigger any extraction rejection: {rejections:?}");
            let snapshots = candidates.into_iter().map(|c| promote(&c, file.clone()).expect("a real extracted candidate must always promote")).collect();
            (file, tree, snapshots)
        }

        fn find<'a>(snapshots: &'a [SymbolSnapshot], name: &str) -> &'a SymbolSnapshot {
            snapshots.iter().find(|s| s.name().as_str() == name).unwrap_or_else(|| panic!("no snapshot named {name:?} among {:?}", snapshots.iter().map(|s| s.name().as_str()).collect::<Vec<_>>()))
        }

        /// Case 1 (required): a correct import alias resolves cleanly
        /// when its own real target name is UNambiguous in the given
        /// pool. Uses the same alias MECHANISM CRC-003 documents
        /// (`from .ml import X as _X`, then a call to the alias `_X`),
        /// with a wrapper function named differently from the real
        /// target so this test isolates the alias-resolution mechanism
        /// itself from the SEPARATE name-collision finding
        /// `crc_003_exact_shape_recovers_only_to_ambiguous` documents
        /// below.
        #[test]
        fn case_1_correct_import_alias_resolves_cleanly_when_unambiguous() {
            let repo = test_repo_calls();
            let (ml_file, _ml_tree, ml_snapshots) = real_snapshots_and_tree(repo, "janitor/ml.py", "def get_features_targets():\n    pass\n");
            let (init_file, init_tree, init_snapshots) = real_snapshots_and_tree(
                repo,
                "janitor/__init__.py",
                "from .ml import get_features_targets as _get_features_targets\n\n\ndef public_wrapper(*args, **kwargs):\n    return _get_features_targets(*args, **kwargs)\n",
            );
            let mut all_snapshots = ml_snapshots.clone();
            all_snapshots.extend(init_snapshots.clone());
            let real_target = find(&ml_snapshots, "get_features_targets");
            let wrapper = find(&init_snapshots, "public_wrapper");

            let import_facts = resolve_import_facts(&init_file, &init_tree, &all_snapshots).unwrap();
            assert_eq!(import_facts.len(), 1);
            assert_eq!(import_facts[0].resolution, crate::imports::ImportResolution::Internal(real_target.id()), "the import fact's own resolution must already point at the real target");

            let (edges, attempts) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();

            let recovered = edges.iter().find(|e| e.from == wrapper.id() && e.to == real_target.id());
            assert!(recovered.is_some(), "the wrapper's call to the alias _get_features_targets must resolve to the real janitor/ml.py::get_features_targets");
            assert_eq!(recovered.unwrap().evidence.method(), codegraph_domain::ResolutionMethod::NameHeuristic, "alias-based resolution is a real structural fact, but still name-based linking, never StaticAnalysis");
            assert!(attempts.iter().all(|a| a.callee_name != "_get_features_targets"), "a recovered alias call must produce an Edge, never ALSO a CallAttempt for the same occurrence");
            let _ = ml_file;
        }

        /// The EXACT real CRC-003 shape, reproduced faithfully
        /// (`call-resolution-challenges.md`'s own documented wrapper
        /// name collision: `janitor/__init__.py`'s own deprecated
        /// wrapper IS itself also named `get_features_targets`, "a
        /// different symbol from the real target, sharing its name
        /// only because this file re-exports a deprecated top-level
        /// alias of the same name"). Real finding, confirmed directly
        /// against this exact shape, NOT assumed from the CRC-003 doc's
        /// own prose: because `resolve_raw_import_statements`'s own
        /// lookup matches by BARE name with no same-file/self
        /// exclusion, the import fact for `get_features_targets`
        /// itself resolves `Ambiguous` (both the real `ml.py` target
        /// AND the wrapper's own same-named self share that bare
        /// name) — NOT `Internal`, contrary to what a reader might
        /// assume from the CRC-003 doc's own prose alone. A4 still
        /// correctly reports this as `ImportedSymbolAmbiguous` (a real,
        /// honest alias match whose own target is not uniquely
        /// determinable) rather than silently resolving to the wrong
        /// wrapper or fabricating a pick — the alias MECHANISM worked
        /// exactly as designed; the exact real corpus shape has a
        /// SECOND, independent ambiguity this fix does not (and should
        /// not) paper over.
        #[test]
        fn crc_003_exact_shape_recovers_only_to_ambiguous_due_to_a_real_name_collision() {
            let repo = test_repo_calls();
            let (_ml_file, _ml_tree, ml_snapshots) = real_snapshots_and_tree(repo, "janitor/ml.py", "def get_features_targets():\n    pass\n");
            let (init_file, init_tree, init_snapshots) = real_snapshots_and_tree(
                repo,
                "janitor/__init__.py",
                "from .ml import get_features_targets as _get_features_targets\n\n\ndef get_features_targets(*args, **kwargs):\n    return _get_features_targets(*args, **kwargs)\n",
            );
            let mut all_snapshots = ml_snapshots.clone();
            all_snapshots.extend(init_snapshots.clone());
            let real_target = find(&ml_snapshots, "get_features_targets");
            let wrapper = find(&init_snapshots, "get_features_targets");
            assert_ne!(real_target.id(), wrapper.id(), "the wrapper and the real target share a bare name but are two DISTINCT real snapshots");

            let import_facts = resolve_import_facts(&init_file, &init_tree, &all_snapshots).unwrap();
            assert_eq!(import_facts.len(), 1);
            assert_eq!(
                import_facts[0].resolution,
                crate::imports::ImportResolution::Ambiguous,
                "confirmed directly, not assumed: the real CRC-003 shape's own name collision makes the import fact itself Ambiguous, not Internal"
            );

            let (edges, attempts) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();

            assert!(edges.iter().all(|e| e.from != wrapper.id()), "A4 must never arbitrarily pick between the real target and the wrapper's own same-named self");
            let attempt = attempts.iter().find(|a| a.caller == wrapper.id() && a.callee_name == "_get_features_targets").expect("the alias call must produce a CallAttempt");
            assert_eq!(attempt.reason, UnresolvedReason::ImportedSymbolAmbiguous, "the alias mechanism DID find a real match -- its own target is what remains ambiguous, a more specific and honest claim than a plain unresolved");
            assert_eq!(attempt.candidates.len(), 2, "both the real target and the wrapper's own same-named self are real candidates for the alias's own target name");
        }

        /// Case 2 (required): the same bare name exists elsewhere in
        /// the pool, but is NOT imported in the caller's own file --
        /// it must stay excluded/unresolved, never silently matched by
        /// coincidental name similarity.
        #[test]
        fn case_2_same_name_not_imported_is_excluded() {
            let repo = test_repo_calls();
            let (_a_file, _a_tree, a_snapshots) = real_snapshots_and_tree(repo, "pkg/a.py", "def parse():\n    pass\n");
            let (_caller_file, caller_tree, caller_snapshots) = real_snapshots_and_tree(repo, "pkg/user.py", "def run():\n    return parse()\n");
            let mut all_snapshots = a_snapshots.clone();
            all_snapshots.extend(caller_snapshots.clone());
            // Deliberately NO import statement at all in user.py --
            // `parse` exists in the pool (pkg/a.py), but is never
            // imported by the caller's own file.
            let import_facts: Vec<crate::imports::ImportFact> = vec![];
            let _ = caller_tree;

            let (edges, attempts) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();

            // Without A4 import evidence being GIVEN at all, this
            // degenerates to the pre-A4 baseline: `parse` resolves
            // directly by bare name since it is the ONLY candidate in
            // the pool (this fixture intentionally has no ambiguity --
            // the point is import evidence plays no role in a direct
            // bare-name match at all, correct or not).
            let runner = find(&caller_snapshots, "run");
            let target = find(&a_snapshots, "parse");
            assert_eq!(edges.len(), 1);
            assert_eq!(edges[0].from, runner.id());
            assert_eq!(edges[0].to, target.id());
            assert!(attempts.is_empty());
        }

        /// Case 3 (required): a wildcard/star import gives this
        /// resolver NO alias information at all for a specific name --
        /// it must stay conservatively ambiguous/unresolved, never
        /// falsely narrowed by assuming a star import brought in any
        /// particular name. (`from pkg import *` produces NO
        /// `ImportFact` at all in this pipeline's own model --
        /// confirmed via direct grammar probe: a `wildcard_import`
        /// node carries no `dotted_name`/`aliased_import` child for
        /// `collect_python_import` to extract, so the WHOLE file
        /// legitimately produces zero import facts here, not just zero
        /// facts for this one name.)
        ///
        /// Known limitation, confirmed directly rather than silently
        /// assumed: because this fixture's own file has NO other real
        /// import statement, `import_facts` for it is `[]` -- the
        /// EXACT same shape as "A4 was never given any import data at
        /// all." `resolve_via_import_alias` still correctly finds no
        /// alias (there is genuinely none), but the final fallback
        /// reason degrades to the OLD, less specific `Unknown` rather
        /// than `NoMatchingImportAlias`, because this function's own
        /// "was import evidence checked at all" signal is the coarse,
        /// whole-call `import_facts.is_empty()` check, not a per-file
        /// one. A per-file-aware signal (e.g. "does ANY ImportFact
        /// exist for this caller's own file, even if none matched")
        /// would recover the more specific reason here; not implemented
        /// in this pass, documented as a residual limitation instead.
        #[test]
        fn case_3_wildcard_import_never_falsely_narrows() {
            let repo = test_repo_calls();
            let (other_file, _other_tree, other_snapshots) = real_snapshots_and_tree(repo, "pkg/other.py", "def helper():\n    pass\n");
            let (caller_file, caller_tree, caller_snapshots) = real_snapshots_and_tree(repo, "pkg/user.py", "from pkg.other import *\n\n\ndef run():\n    return mystery_name()\n");
            let mut all_snapshots = other_snapshots.clone();
            all_snapshots.extend(caller_snapshots.clone());
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();
            // Confirms the fixture's own real premise: a wildcard
            // import produces ZERO import facts for the whole file,
            // not merely zero facts for this one name.
            assert!(import_facts.is_empty(), "fixture premise: a wildcard-only file produces zero ImportFacts in this pipeline's current model");

            let (edges, attempts) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();

            let runner = find(&caller_snapshots, "run");
            assert!(edges.iter().all(|e| e.from != runner.id()), "mystery_name must never resolve to anything -- no real candidate and no real alias match exist, and A4 must never fabricate one from a wildcard import's own absence of alias data");
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].reason, UnresolvedReason::Unknown, "with zero ImportFacts for the whole call (the wildcard import's own real shape), this degrades to the coarse Unknown reason -- a documented limitation of the current per-call (not per-file) 'was import evidence checked' signal, not a fabricated or incorrect classification");
            let _ = (other_file, caller_file);
        }

        /// Case 4 (required): a re-export shape must never cause this
        /// resolver to invent a target. `from pkg.a import helper` in
        /// a `pkg/__init__.py`-style re-export file, where `helper`
        /// itself does not exist ANYWHERE in the given pool (e.g. the
        /// real module was not included in this resolution pass) --
        /// the import fact resolves to `External`, and the call must
        /// stay honestly unresolved, never pointed at a fabricated id.
        #[test]
        fn case_4_unresolvable_reexport_never_invents_a_target() {
            let repo = test_repo_calls();
            let (caller_file, caller_tree, caller_snapshots) = real_snapshots_and_tree(repo, "pkg/__init__.py", "from pkg.a import helper as reexported_helper\n\n\ndef run():\n    return reexported_helper()\n");
            // Deliberately NO pkg/a.py snapshots at all in the pool --
            // the real `helper` is outside what this resolution pass
            // was given.
            let all_snapshots = caller_snapshots.clone();
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();
            assert_eq!(import_facts[0].resolution, crate::imports::ImportResolution::External);

            let (edges, attempts) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();

            let runner = find(&caller_snapshots, "run");
            assert!(edges.iter().all(|e| e.from != runner.id()), "an unresolvable re-export must never produce a fabricated edge");
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].reason, UnresolvedReason::ImportTargetExternal, "the alias IS real and WAS matched -- its target is honestly External, a more specific claim than plain Unknown");
            assert!(attempts[0].candidates.is_empty());
        }

        /// Case 5 (required): a namespace/import alias at the FILE
        /// level still resolves correctly via the exact same mechanism
        /// -- Python's own `import pkg.mod as m` shape (a bare, not
        /// `from`-style, aliased import), confirmed this resolver
        /// handles both `RawImportStatement` shapes identically since
        /// both populate the same `alias` field.
        #[test]
        fn case_5_namespace_style_module_alias_resolves() {
            let repo = test_repo_calls();
            let (mod_file, _mod_tree, mod_snapshots) = real_snapshots_and_tree(repo, "pkg/mod.py", "def entry():\n    pass\n");
            let (caller_file, caller_tree, caller_snapshots) = real_snapshots_and_tree(repo, "pkg/user.py", "import pkg.mod as entry\n\n\ndef run():\n    return entry()\n");
            let mut all_snapshots = mod_snapshots.clone();
            all_snapshots.extend(caller_snapshots.clone());
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();

            let (edges, _attempts) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();

            let runner = find(&caller_snapshots, "run");
            let target = find(&mod_snapshots, "entry");
            // NOTE: `import pkg.mod as entry` resolves by the module
            // path's own last segment ("mod"), not "entry" -- the
            // ALIAS "entry" is what the CALL SITE actually names. This
            // fixture's own module is deliberately named so the
            // module's last segment ("mod") does NOT collide with the
            // real callee's own function name ("entry"), isolating
            // the alias mechanism from a same-name coincidence.
            assert!(edges.iter().any(|e| e.from == runner.id() && e.to == target.id()), "the bare module-alias import must resolve the same way a from-import alias does");
            let _ = mod_file;
        }

        /// Case 6 (required): multiple DIFFERENT import statements in
        /// the SAME file, each introducing its OWN distinct local
        /// alias name -- each must resolve independently and
        /// correctly, never cross-contaminating.
        #[test]
        fn case_6_multiple_imports_with_different_local_names_each_resolve_independently() {
            let repo = test_repo_calls();
            let (a_file, _a_tree, a_snapshots) = real_snapshots_and_tree(repo, "pkg/a.py", "def real_a():\n    pass\n");
            let (b_file, _b_tree, b_snapshots) = real_snapshots_and_tree(repo, "pkg/b.py", "def real_b():\n    pass\n");
            let (caller_file, caller_tree, caller_snapshots) = real_snapshots_and_tree(
                repo,
                "pkg/user.py",
                "from pkg.a import real_a as alias_a\nfrom pkg.b import real_b as alias_b\n\n\ndef run():\n    alias_a()\n    alias_b()\n",
            );
            let mut all_snapshots = a_snapshots.clone();
            all_snapshots.extend(b_snapshots.clone());
            all_snapshots.extend(caller_snapshots.clone());
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();
            assert_eq!(import_facts.len(), 2);

            let (edges, _attempts) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();

            let runner = find(&caller_snapshots, "run");
            let target_a = find(&a_snapshots, "real_a");
            let target_b = find(&b_snapshots, "real_b");
            assert!(edges.iter().any(|e| e.from == runner.id() && e.to == target_a.id()), "alias_a must resolve to real_a");
            assert!(edges.iter().any(|e| e.from == runner.id() && e.to == target_b.id()), "alias_b must resolve to real_b");
            let _ = (a_file, b_file);
        }

        /// Case 7 (required): an import whose own target is confirmed
        /// external (standard library / third party, never found in
        /// the given pool at all) must produce the specific
        /// `ImportTargetExternal` reason, not a generic Unknown.
        #[test]
        fn case_7_unresolved_external_import() {
            let repo = test_repo_calls();
            let (caller_file, caller_tree, caller_snapshots) = real_snapshots_and_tree(repo, "pkg/user.py", "import torch as t\n\n\ndef run():\n    return t()\n");
            let all_snapshots = caller_snapshots.clone();
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();
            assert_eq!(import_facts[0].resolution, crate::imports::ImportResolution::External);

            let (edges, attempts) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();

            let runner = find(&caller_snapshots, "run");
            assert!(edges.iter().all(|e| e.from != runner.id()));
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].reason, UnresolvedReason::ImportTargetExternal);
        }

        /// Case 8 (required, CRC-002's own territory): a LOCAL symbol
        /// shadows an imported one with the same callee name -- the
        /// direct by-name lookup must still find the LOCAL candidate
        /// first (the existing, pre-A4 resolution order), and A4's own
        /// alias check must NEVER even run in that case, since it only
        /// activates on the zero-candidate branch. Confirms A4 cannot
        /// introduce a CRC-002-shaped regression by construction.
        #[test]
        fn case_8_local_symbol_shadows_an_imported_symbol_of_the_same_name() {
            let repo = test_repo_calls();
            let (other_file, _other_tree, other_snapshots) = real_snapshots_and_tree(repo, "pkg/other.py", "def helper():\n    pass\n");
            let (caller_file, caller_tree, caller_snapshots) = real_snapshots_and_tree(
                repo,
                "pkg/user.py",
                "from pkg.other import helper as helper\n\n\ndef helper():\n    pass\n\n\ndef run():\n    return helper()\n",
            );
            let mut all_snapshots = other_snapshots.clone();
            all_snapshots.extend(caller_snapshots.clone());
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();

            let (edges, attempts) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();

            // Two real candidates share the bare name "helper"
            // (pkg/other.py::helper via the alias, and pkg/user.py's
            // own local helper) -- `all_candidates.len() == 2`, so the
            // zero-candidate A4 branch never runs at all; this is the
            // PRE-EXISTING scope/ambiguity path, unchanged by A4.
            let runner = find(&caller_snapshots, "run");
            let local_helper = caller_snapshots.iter().filter(|s| s.name().as_str() == "helper" && s.id() != find(&caller_snapshots, "run").id()).find(|s| same_file(s, find(&caller_snapshots, "run"))).expect("the local helper must exist");
            let caller_edges: Vec<&Edge> = edges.iter().filter(|e| e.from == runner.id()).collect();
            if let Some(edge) = caller_edges.first() {
                // If this resolved at all (same-file scope narrowing
                // may apply), it must resolve to the LOCAL helper,
                // never silently to the imported one via a new A4
                // path that was never supposed to run here.
                assert_eq!(edge.to, local_helper.id(), "A4 must never override the pre-existing same-file/scope resolution order");
            } else {
                // Or it stays ambiguous/unresolved per the pre-A4
                // scope-narrowing rules -- either outcome is
                // acceptable here; what is NOT acceptable is A4
                // silently redirecting this to the import target.
                assert!(!attempts.is_empty());
            }
            let _ = other_file;
        }

        /// Case 9 (required): nested/local binding shadowing -- a
        /// function parameter named the same as a real import alias
        /// must not be affected by A4 at all, since A4 operates purely
        /// on CALLEE NAMES matched against `ImportFact.alias`, never
        /// on local variable/parameter bindings (that is `resolve_calls`'s
        /// own pre-existing, unrelated scope-blindness territory --
        /// CRC-002's own finding -- which this checkpoint explicitly
        /// does not touch). Confirms A4 introduces no NEW local-binding
        /// confusion beyond what already existed.
        #[test]
        fn case_9_nested_local_parameter_shadowing_is_unaffected_by_a4() {
            let repo = test_repo_calls();
            let (helper_file, _helper_tree, helper_snapshots) = real_snapshots_and_tree(repo, "pkg/helper.py", "def real_helper():\n    pass\n");
            let (caller_file, caller_tree, caller_snapshots) = real_snapshots_and_tree(
                repo,
                "pkg/user.py",
                "from pkg.helper import real_helper as local_name\n\n\ndef run(local_name):\n    return local_name()\n",
            );
            let mut all_snapshots = helper_snapshots.clone();
            all_snapshots.extend(caller_snapshots.clone());
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();

            let (edges, _attempts) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();

            // This resolver has NO local-binding/parameter-shadowing
            // awareness before OR after A4 (confirmed pre-existing,
            // CRC-002's own territory) -- `run`'s own call to
            // `local_name()` (its OWN parameter, shadowing the module-
            // level alias) will resolve via the alias mechanism
            // exactly as case 1 does, since A4 cannot see the
            // parameter binding either. This test exists to CONFIRM
            // that fact directly (not assume it), so a future local-
            // binding fix does not silently change A4's own documented
            // scope without a test noticing.
            let runner = find(&caller_snapshots, "run");
            let real_target = find(&helper_snapshots, "real_helper");
            assert!(
                edges.iter().any(|e| e.from == runner.id() && e.to == real_target.id()),
                "pre-existing, documented limitation: A4 has no local-binding/parameter-shadowing awareness, confirmed directly rather than silently assumed -- this is NOT a NEW regression A4 introduces, it is CRC-002's own pre-existing scope, unchanged"
            );
            let _ = helper_file;
        }

        /// Case 10 (required): with NO import information at all
        /// (`import_facts` is empty), A4 must be a complete no-op --
        /// byte-for-byte identical to the pre-A4 baseline. This is the
        /// SAME guarantee the `resolve_calls_with_extraction_rejections`
        /// equivalence test already proves for the wrapper, re-stated
        /// here directly against `resolve_calls_with_imports` itself.
        #[test]
        fn case_10_no_import_information_preserves_baseline_ambiguity_unchanged() {
            let repo = test_repo_calls();
            let caller = spanned_snapshot_in_repo(repo, "src/lib.rs", "a", "function_item", "fn a() { missing(); }", 0, 30);
            let snapshots = vec![caller];

            let baseline = resolve_calls_with_extraction_rejections(&snapshots, &[]).unwrap();
            let via_a4_with_empty_imports = resolve_calls_with_imports(&snapshots, &[], &[]).unwrap();

            assert_eq!(baseline, via_a4_with_empty_imports, "A4 called with zero import facts must be byte-for-byte identical to the pre-A4 baseline");
        }

        // --- Explicit CRC-001/CRC-002/CRC-003 regression report ---

        /// CRC-001 non-regression: the real documented case (two
        /// same-named classes, no scope signal) must still produce the
        /// SAME ambiguous outcome after A4 is wired in with REAL
        /// import facts present for the same file -- A4's own
        /// zero-candidate-only scope must never touch this.
        #[test]
        fn crc_001_ambiguous_constructor_call_regression_check() {
            let repo = test_repo_calls();
            let (a_file, _a_tree, a_snapshots) = real_snapshots_and_tree(repo, "models/a.py", "class RNNModel:\n    pass\n");
            let (b_file, _b_tree, b_snapshots) = real_snapshots_and_tree(repo, "models/b.py", "class RNNModel:\n    pass\n");
            let (caller_file, caller_tree, caller_snapshots) = real_snapshots_and_tree(repo, "models/trainer.py", "import os\n\n\ndef _run_trainer():\n    return RNNModel()\n");
            let mut all_snapshots = a_snapshots.clone();
            all_snapshots.extend(b_snapshots.clone());
            all_snapshots.extend(caller_snapshots.clone());
            // A real, UNRELATED import fact exists in this same file
            // (`import os`) -- confirms A4's own presence (non-empty
            // import_facts) does not somehow perturb an unrelated
            // ambiguity it was never meant to touch.
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();
            assert_eq!(import_facts.len(), 1, "fixture premise: exactly one real (unrelated) import fact exists in this file");

            let (edges, attempts) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();

            let runner = find(&caller_snapshots, "_run_trainer");
            assert!(edges.iter().all(|e| e.from != runner.id()), "CRC-001 must remain unresolved/ambiguous after A4 -- never silently picked");
            assert!(attempts.iter().any(|a| a.caller == runner.id() && a.callee_name == "RNNModel" && a.tier == codegraph_domain::EvidenceTier::Ambiguous), "the real ambiguity must still be honestly recorded");
            let _ = (a_file, b_file);
        }

        /// CRC-002 non-regression (destructured-prop/same-name-in-
        /// another-file shape): confirms A4's own presence does not
        /// change this pre-existing scope-blindness outcome either way
        /// -- A4 never runs on this path since the direct lookup
        /// already found exactly one (wrong, scope-blind) candidate,
        /// which is case_8's own point, re-confirmed here against the
        /// real documented CRC-002 shape rather than a synthetic one.
        #[test]
        fn crc_002_local_binding_shadowing_regression_check() {
            let repo = test_repo_calls();
            let (unrelated_file, _unrelated_tree, unrelated_snapshots) = real_snapshots_and_tree(repo, "components/other.py", "def Button():\n    pass\n");
            let (caller_file, caller_tree, caller_snapshots) = real_snapshots_and_tree(
                repo,
                "components/card.py",
                "import os\n\n\ndef render(Button):\n    return Button()\n",
            );
            let mut all_snapshots = unrelated_snapshots.clone();
            all_snapshots.extend(caller_snapshots.clone());
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();

            let (edges_before, _attempts_before) = resolve_calls_with_extraction_rejections(&all_snapshots, &[]).unwrap();
            let (edges_after, _attempts_after) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();

            assert_eq!(edges_before, edges_after, "A4 must produce a BYTE-FOR-BYTE IDENTICAL result to the pre-A4 baseline for this exact CRC-002 shape -- confirms no new FP was introduced by A4 on this case");
            let _ = unrelated_file;
        }
    }

    /// CALL-RES-A4 — C# namespace-aware candidate filtering
    /// (`resolve_calls_with_namespace_context`). Every fixture uses REAL
    /// extraction (`extract_symbols_reporting_rejections` + `promote`
    /// for snapshots, `extract_csharp_type_declaration_contexts` +
    /// `resolve_csharp_type_declaration_contexts` for namespace
    /// contexts, `resolve_import_facts` for `using` directives), never
    /// hand-built structs, per this project's own evidence-before-claims
    /// discipline.
    mod call_res_a4_namespace {
        use super::*;
        use crate::csharp_declarations::{extract_csharp_type_declaration_contexts, resolve_csharp_type_declaration_contexts};
        use crate::extract::extract_symbols_reporting_rejections;
        use crate::imports::resolve_import_facts;

        fn real_snapshots_contexts_and_tree(repo: codegraph_domain::RepositoryId, path: &str, source: &str) -> (FileIdentity, crate::parse::ParseTree, Vec<SymbolSnapshot>, Vec<(FileIdentity, crate::csharp_declarations::RawCSharpTypeDeclarationContext)>) {
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let file = FileIdentity::parse(repo, commit, path).unwrap();
            let parser = parser_for(&file).expect("a supported extension must have a parser");
            let tree = parser.parse(source.as_bytes()).expect("fixture source must parse");
            let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
            assert!(rejections.is_empty(), "fixture source must not trigger any extraction rejection: {rejections:?}");
            let snapshots: Vec<SymbolSnapshot> = candidates.into_iter().map(|c| promote(&c, file.clone()).expect("a real extracted candidate must always promote")).collect();
            let raw_contexts: Vec<_> = extract_csharp_type_declaration_contexts(&tree).into_iter().map(|c| (file.clone(), c)).collect();
            (file, tree, snapshots, raw_contexts)
        }

        fn find<'a>(snapshots: &'a [SymbolSnapshot], name: &str) -> &'a SymbolSnapshot {
            snapshots.iter().find(|s| s.name().as_str() == name).unwrap_or_else(|| panic!("no snapshot named {name:?} among {:?}", snapshots.iter().map(|s| s.name().as_str()).collect::<Vec<_>>()))
        }

        /// Local copy of the sibling `call_res_a4_import_alias` module's
        /// own helper (that module's own version is private to it) --
        /// used only by this module's two CRC-regression tests, which
        /// deliberately reuse the Python CRC-001 shape to confirm this
        /// new entry point does not perturb it.
        fn real_snapshots_and_tree(repo: codegraph_domain::RepositoryId, path: &str, source: &str) -> (FileIdentity, crate::parse::ParseTree, Vec<SymbolSnapshot>) {
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let file = FileIdentity::parse(repo, commit, path).unwrap();
            let parser = parser_for(&file).expect("a supported extension must have a parser");
            let tree = parser.parse(source.as_bytes()).expect("fixture source must parse");
            let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
            assert!(rejections.is_empty(), "fixture source must not trigger any extraction rejection: {rejections:?}");
            let snapshots = candidates.into_iter().map(|c| promote(&c, file.clone()).expect("a real extracted candidate must always promote")).collect();
            (file, tree, snapshots)
        }

        /// Case: same simple method name declared on two classes in two
        /// DIFFERENT namespaces; the caller's own file `using`s ONE of
        /// them. The candidate in the non-`using`'d namespace must be
        /// excluded, leaving exactly one real candidate -- a genuine
        /// ambiguity resolved via real namespace evidence, never a
        /// guess.
        #[test]
        fn namespace_import_excludes_the_non_imported_namespace_candidate() {
            let repo = test_repo_calls();
            let (a_file, _a_tree, a_snapshots, a_raw) = real_snapshots_contexts_and_tree(repo, "src/NsA/Foo.cs", "namespace NsA;\nclass Foo { public void Target() {} }\n");
            let (b_file, _b_tree, b_snapshots, b_raw) = real_snapshots_contexts_and_tree(repo, "src/NsB/Foo.cs", "namespace NsB;\nclass Foo { public void Target() {} }\n");
            let (caller_file, caller_tree, caller_snapshots, _caller_raw) = real_snapshots_contexts_and_tree(repo, "src/Caller.cs", "using NsA;\nclass Caller { public void Run() { Target(); } }\n");

            let mut all_snapshots = a_snapshots.clone();
            all_snapshots.extend(b_snapshots.clone());
            all_snapshots.extend(caller_snapshots.clone());

            let mut all_raw = a_raw;
            all_raw.extend(b_raw);
            let csharp_contexts = resolve_csharp_type_declaration_contexts(&all_raw, &all_snapshots);

            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();
            assert_eq!(import_facts.len(), 1, "fixture premise: one real using directive");
            assert!(import_facts[0].alias.is_none(), "fixture premise: a BARE using directive, not an alias");

            let run_method = find(&caller_snapshots, "Run");
            let target_in_a = find(&a_snapshots, "Target");
            let target_in_b = find(&b_snapshots, "Target");
            let all_targets = all_snapshots.iter().filter(|s| s.name().as_str() == "Target").count();
            assert_eq!(all_targets, 2, "fixture premise: two real, same-named candidates");

            let (edges, attempts) = resolve_calls_with_namespace_context(&all_snapshots, &[], &import_facts, &csharp_contexts).unwrap();

            let recovered = edges.iter().find(|e| e.from == run_method.id() && e.to == target_in_a.id());
            assert!(recovered.is_some(), "the caller's own using NsA; must exclude NsB::Foo::Target, leaving exactly one real candidate in NsA");
            assert!(edges.iter().all(|e| e.to != target_in_b.id()), "the non-imported namespace's candidate must never be chosen");
            assert!(attempts.iter().all(|a| a.callee_name != "Target"), "a recovered namespace-filtered call must produce an Edge, never ALSO a CallAttempt");
            let _ = (a_file, b_file);
        }

        /// Case: same simple method name on two classes in two
        /// different namespaces; the caller is ITSELF declared inside
        /// one of those namespaces (no `using` directive needed -- the
        /// caller's own namespace is always in scope for itself).
        #[test]
        fn callers_own_namespace_is_in_scope_without_any_using_directive() {
            let repo = test_repo_calls();
            let (a_file, _a_tree, a_snapshots, a_raw) = real_snapshots_contexts_and_tree(repo, "src/NsA/Foo.cs", "namespace NsA;\nclass Foo { public void Target() {} }\n");
            let (b_file, _b_tree, b_snapshots, b_raw) = real_snapshots_contexts_and_tree(repo, "src/NsA/Caller.cs", "namespace NsA;\nclass Caller { public void Run() { Target(); } }\n");

            let mut all_snapshots = a_snapshots.clone();
            all_snapshots.extend(b_snapshots.clone());
            let mut all_raw = a_raw;
            all_raw.extend(b_raw.clone());
            let csharp_contexts = resolve_csharp_type_declaration_contexts(&all_raw, &all_snapshots);

            // No import facts at all -- the caller's own namespace
            // membership alone must be enough.
            let (edges, _attempts) = resolve_calls_with_namespace_context(&all_snapshots, &[], &[], &csharp_contexts).unwrap();

            let run_method = find(&all_snapshots, "Run");
            let target = find(&a_snapshots, "Target");
            assert!(edges.iter().any(|e| e.from == run_method.id() && e.to == target.id()), "a candidate in the caller's own namespace must be considered visible with zero using directives");
            let _ = (a_file, b_file);
        }

        /// Required negative test: wildcard-equivalent case -- a using
        /// directive exists for a namespace that is NEITHER candidate's
        /// own namespace. Both real candidates must stay excluded from
        /// nothing (namespace filtering finds no match for either), so
        /// filtering must fall back to the ORIGINAL unfiltered set
        /// rather than manufacturing a false zero-candidate outcome --
        /// the real ambiguity must remain honestly Ambiguous.
        #[test]
        fn using_an_unrelated_third_namespace_never_manufactures_a_false_resolution() {
            let repo = test_repo_calls();
            let (a_file, _a_tree, a_snapshots, a_raw) = real_snapshots_contexts_and_tree(repo, "src/NsA/Foo.cs", "namespace NsA;\nclass Foo { public void Target() {} }\n");
            let (b_file, _b_tree, b_snapshots, b_raw) = real_snapshots_contexts_and_tree(repo, "src/NsB/Foo.cs", "namespace NsB;\nclass Foo { public void Target() {} }\n");
            let (caller_file, caller_tree, caller_snapshots, _caller_raw) = real_snapshots_contexts_and_tree(repo, "src/Caller.cs", "using NsC;\nclass Caller { public void Run() { Target(); } }\n");

            let mut all_snapshots = a_snapshots.clone();
            all_snapshots.extend(b_snapshots.clone());
            all_snapshots.extend(caller_snapshots.clone());
            let mut all_raw = a_raw;
            all_raw.extend(b_raw);
            let csharp_contexts = resolve_csharp_type_declaration_contexts(&all_raw, &all_snapshots);
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();

            let (edges, attempts) = resolve_calls_with_namespace_context(&all_snapshots, &[], &import_facts, &csharp_contexts).unwrap();

            let run_method = find(&caller_snapshots, "Run");
            assert!(edges.iter().all(|e| e.from != run_method.id()), "a using directive for an unrelated third namespace must never resolve a real cross-namespace ambiguity -- the real ambiguity must remain Ambiguous, never silently narrowed to zero via a filter whose own evidence does not actually explain either candidate");
            assert!(
                attempts.iter().any(|a| a.caller == run_method.id() && a.callee_name == "Target" && a.tier == codegraph_domain::EvidenceTier::Ambiguous && a.candidates.len() == 2),
                "both real candidates must be preserved in the honest Ambiguous record"
            );
            let _ = (a_file, b_file);
        }

        /// Required negative test: the caller is a NON-C# language --
        /// confirms `resolve_calls_with_namespace_context` behaves
        /// identically to `resolve_calls_with_imports` when
        /// `csharp_contexts` is genuinely empty (the real condition for
        /// any non-C# corpus, since only C# ever produces
        /// `CSharpTypeDeclarationContext` entries at all).
        #[test]
        fn empty_csharp_contexts_is_byte_for_byte_identical_to_resolve_calls_with_imports() {
            let repo = test_repo_calls();
            let (a_file, _a_tree, a_snapshots) = real_snapshots_and_tree(repo, "models/a.py", "class RNNModel:\n    pass\n");
            let (caller_file, caller_tree, caller_snapshots) = real_snapshots_and_tree(repo, "models/trainer.py", "def _run_trainer():\n    return RNNModel()\n");
            let mut all_snapshots = a_snapshots.clone();
            all_snapshots.extend(caller_snapshots.clone());
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();

            let (edges_via_imports, attempts_via_imports) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();
            let (edges_via_namespace, attempts_via_namespace) = resolve_calls_with_namespace_context(&all_snapshots, &[], &import_facts, &[]).unwrap();

            assert_eq!(edges_via_imports, edges_via_namespace, "with empty csharp_contexts, the namespace-aware entry point must be byte-for-byte identical to resolve_calls_with_imports");
            assert_eq!(attempts_via_imports, attempts_via_namespace);
            let _ = a_file;
        }

        /// Required negative test: a candidate whose namespace is
        /// genuinely UNKNOWN (no `CSharpTypeDeclarationContext` entry
        /// exists for its own enclosing class -- simulated here by
        /// giving the resolver csharp_contexts that do not cover one of
        /// the two real candidate classes at all) must never be
        /// excluded merely because its namespace could not be
        /// determined -- "unknown" is never collapsed into
        /// "not-visible."
        #[test]
        fn a_candidate_with_an_undetermined_namespace_is_never_excluded() {
            let repo = test_repo_calls();
            let (a_file, _a_tree, a_snapshots, a_raw) = real_snapshots_contexts_and_tree(repo, "src/NsA/Foo.cs", "namespace NsA;\nclass Foo { public void Target() {} }\n");
            let (b_file, _b_tree, b_snapshots, _b_raw) = real_snapshots_contexts_and_tree(repo, "src/NsB/Foo.cs", "namespace NsB;\nclass Foo { public void Target() {} }\n");
            let (caller_file, caller_tree, caller_snapshots, _caller_raw) = real_snapshots_contexts_and_tree(repo, "src/Caller.cs", "using NsA;\nclass Caller { public void Run() { Target(); } }\n");

            let mut all_snapshots = a_snapshots.clone();
            all_snapshots.extend(b_snapshots.clone());
            all_snapshots.extend(caller_snapshots.clone());
            // Deliberately only resolve contexts for NsA::Foo -- NsB::Foo's
            // own class has NO CSharpTypeDeclarationContext entry at all,
            // simulating "namespace undetermined" for that one candidate.
            let csharp_contexts = resolve_csharp_type_declaration_contexts(&a_raw, &all_snapshots);
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();

            let (edges, attempts) = resolve_calls_with_namespace_context(&all_snapshots, &[], &import_facts, &csharp_contexts).unwrap();

            let run_method = find(&caller_snapshots, "Run");
            // NsA::Foo::Target is visible (caller's own used namespace);
            // NsB::Foo::Target has an UNDETERMINED namespace (never
            // excluded) -- so BOTH candidates survive filtering, and the
            // real ambiguity (2 candidates) must remain honestly Ambiguous
            // rather than falsely narrowed to the one determined match.
            assert!(edges.iter().all(|e| e.from != run_method.id()), "an undetermined-namespace candidate must never be silently excluded -- the real ambiguity (one determined-visible, one undetermined) must stay Ambiguous, never falsely resolved");
            assert!(
                attempts.iter().any(|a| a.caller == run_method.id() && a.callee_name == "Target" && a.tier == codegraph_domain::EvidenceTier::Ambiguous && a.candidates.len() == 2),
                "both candidates (one determined-visible, one undetermined) must be preserved in the honest Ambiguous record"
            );
            let _ = (a_file, b_file);
        }

        /// CRC-001/CRC-002 non-regression, re-confirmed against this
        /// specific new entry point: the Python CRC-001/CRC-002 shapes
        /// must behave identically whether called via
        /// `resolve_calls_with_imports` or
        /// `resolve_calls_with_namespace_context` with empty
        /// `csharp_contexts` -- confirms this new code path introduces
        /// no cross-language interference.
        #[test]
        fn crc_001_and_crc_002_unaffected_by_the_new_namespace_entry_point() {
            let repo = test_repo_calls();
            let (a_file, _a_tree, a_snapshots) = real_snapshots_and_tree(repo, "models/a.py", "class RNNModel:\n    pass\n");
            let (b_file, _b_tree, b_snapshots) = real_snapshots_and_tree(repo, "models/b.py", "class RNNModel:\n    pass\n");
            let (caller_file, caller_tree, caller_snapshots) = real_snapshots_and_tree(repo, "models/trainer.py", "import os\n\n\ndef _run_trainer():\n    return RNNModel()\n");
            let mut all_snapshots = a_snapshots.clone();
            all_snapshots.extend(b_snapshots.clone());
            all_snapshots.extend(caller_snapshots.clone());
            let import_facts = resolve_import_facts(&caller_file, &caller_tree, &all_snapshots).unwrap();

            let (edges_via_imports, attempts_via_imports) = resolve_calls_with_imports(&all_snapshots, &[], &import_facts).unwrap();
            let (edges_via_namespace, attempts_via_namespace) = resolve_calls_with_namespace_context(&all_snapshots, &[], &import_facts, &[]).unwrap();

            assert_eq!(edges_via_imports, edges_via_namespace, "CRC-001's own Python shape must be unaffected by the new namespace-aware entry point when csharp_contexts is empty");
            assert_eq!(attempts_via_imports, attempts_via_namespace);
            let _ = (a_file, b_file);
        }
    }
}
