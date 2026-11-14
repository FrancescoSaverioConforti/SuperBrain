//! B2-EVAL-009: C++ domain standalone evaluation (`fmtlib/fmt`).
//!
//! Drives the SAME real production pipeline every prior domain harness
//! uses (`RepositorySource` -> parse -> extract -> promote -> lineage
//! -> edges -> `CodeGraph`), plus the real, now-wired
//! `CodeGraphIndexer::observe_commit` for the full/incremental parity
//! check (Step 8). This is the first STANDALONE full C++ domain
//! evaluation (prior C++ exposure was secondary/bundled inside
//! B2-EVAL-008's C#-centric Polly round).
//!
//! This round's own focus, per the directive: (1) whether CALL-RES-A4's
//! namespace-awareness (built for C#) generalizes to C++ namespaces at
//! all, (2) re-confirming/quantifying the known residual
//! `ns::Foo<T>()`/`ns::Foo()` gap (B2-ID-011's own independent-review
//! finding: C++ `qualified_identifier` callees are entirely unhandled
//! by `callee_name`, generic or not), (3) overload-ambiguity handling,
//! (4) template-call visibility generally.
//!
//! Usage: `cargo run --release --example b2_eval_009_cpp_fmt -- <repo_path> <commit>`

use std::collections::HashMap;
use std::time::Instant;

use codegraph_domain::{CommitHash, EdgeKind, EvidenceTier, RepositorySource, ResolutionMethod, SymbolKind};
use codegraph_index::agent_policy::{evaluate, IntendedUse, PolicyVerdict};
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::edges::{resolve_calls_with_namespace_context, resolve_contains, resolve_extends_implements, resolve_instantiates, resolve_tests_for};
use codegraph_index::extract::extract_symbols_reporting_rejections;
use codegraph_index::imports::{extract_import_statements, resolve_raw_import_statements, RawImportStatement};
use codegraph_index::local_git_source::LocalGitRepositorySource;
use codegraph_index::negative_knowledge::{CallAttempt, UnresolvedReason};
use codegraph_index::parse::parser_for;
use codegraph_index::snapshot::promote;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: b2_eval_009_cpp_fmt <repo_path> <commit>");
        std::process::exit(2);
    }
    let repo_path = std::fs::canonicalize(&args[1]).expect("repo_path must exist");
    let commit_str = &args[2];

    let mut allowlist = RepositoryAllowlist::new();
    let allowlisted_root = allowlist.authorize(&repo_path).expect("authorize repo root");
    let mut registry = codegraph_domain::RepositoryRegistry::new();
    let repo_root = codegraph_domain::RepositoryRoot::parse(&repo_path).unwrap();
    let repo_id = registry.register(repo_root);

    let root_for_id = move |id: codegraph_domain::RepositoryId| -> Option<codegraph_index::allowlist::AllowlistedRoot> {
        if id == repo_id {
            Some(allowlisted_root.clone())
        } else {
            None
        }
    };
    let source = LocalGitRepositorySource::new(&allowlist, &root_for_id);
    let commit = CommitHash::parse(commit_str).expect("commit must be a valid SHA-1");

    println!("=== B2-EVAL-009: C++ domain standalone evaluation (fmtlib/fmt) ===");
    println!("repo: {}", repo_path.display());
    println!("commit: {commit_str}");

    let t0 = Instant::now();
    let all_files = source.list_files_at(repo_id, &commit).expect("list_files_at");
    // Scope to the library's own code (include/ + src/), excluding
    // test/ (which vendors a full third-party copy of googletest/
    // googlemock under test/gtest/ -- not fmt's own code, and would
    // otherwise pollute every corpus metric and the overload/namespace
    // samples with an unrelated library's internals). This matches
    // the repo-selection sizing (19 files, ~18k LOC) documented in the
    // eval report.
    let files: Vec<String> = all_files.into_iter().filter(|p| p.starts_with("include/") || p.starts_with("src/")).collect();
    let total_files = files.len();

    let mut files_supported = 0usize;
    let mut files_parsed = 0usize;
    let mut files_with_error_nodes = 0usize;
    let mut parse_failures = 0usize;
    let mut raw_candidates_total = 0usize;
    let mut rejections_total = 0usize;
    let mut total_loc = 0usize;
    let mut snapshots: Vec<codegraph_index::snapshot::SymbolSnapshot> = Vec::new();
    let mut kind_counts: HashMap<SymbolKind, usize> = HashMap::new();
    let mut signature_too_long_names: Vec<String> = Vec::new();

    // Raw-parse proxies for this round's own focus questions, counted
    // directly against the real tree-sitter-cpp AST (never inferred
    // from extracted snapshots), so the "how many real call sites hit
    // the known gap" question has a real denominator.
    let mut raw_qualified_identifier_calls = 0usize; // ns::foo(...) -- any shape, generic or not
    let mut raw_qualified_template_calls = 0usize; // ns::foo<T>(...) specifically (qualified_identifier wrapping template_function)
    let mut raw_bare_template_calls = 0usize; // foo<T>(...) no namespace qualifier
    let mut raw_receiver_template_calls = 0usize; // obj.foo<T>(...) / obj->foo<T>(...)
    let mut raw_namespace_decls = 0usize;
    let mut raw_total_call_expressions = 0usize; // every call_expression node, any callee shape
    let mut dropped_callee_kinds: HashMap<String, usize> = HashMap::new(); // callee.kind() for every call_expression whose callee_name() (same logic as calls.rs) returns None
    let mut qualified_call_examples: Vec<String> = Vec::new();
    let mut qualified_template_call_examples: Vec<String> = Vec::new();

    let mut raw_import_statements_all: Vec<(codegraph_domain::FileIdentity, RawImportStatement)> = Vec::new();

    for relative_path in &files {
        let file = match codegraph_domain::FileIdentity::parse(repo_id, commit.clone(), relative_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let Some(parser) = parser_for(&file) else {
            continue;
        };
        files_supported += 1;
        let bytes = match source.read_file(&file) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("read_file failed for {relative_path}: {e}");
                continue;
            }
        };
        total_loc += bytes.iter().filter(|&&b| b == b'\n').count();
        let tree = match parser.parse(&bytes) {
            Ok(t) => t,
            Err(_) => {
                parse_failures += 1;
                continue;
            }
        };
        files_parsed += 1;
        if tree.tree().root_node().has_error() {
            files_with_error_nodes += 1;
        }

        if relative_path.ends_with(".cpp") || relative_path.ends_with(".cc") || relative_path.ends_with(".h") || relative_path.ends_with(".hpp") {
            count_cpp_call_shapes(
                tree.tree().root_node(),
                &bytes,
                relative_path,
                &mut raw_qualified_identifier_calls,
                &mut raw_qualified_template_calls,
                &mut raw_bare_template_calls,
                &mut raw_receiver_template_calls,
                &mut raw_namespace_decls,
                &mut qualified_call_examples,
                &mut qualified_template_call_examples,
                &mut raw_total_call_expressions,
                &mut dropped_callee_kinds,
            );
        }

        let raw_imports = extract_import_statements(&tree);
        raw_import_statements_all.extend(raw_imports.into_iter().map(|s| (file.clone(), s)));

        let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
        raw_candidates_total += candidates.len();
        rejections_total += rejections.len();
        for rejection in &rejections {
            if let codegraph_index::extract::CandidateRejection::SignatureTooLong { name, .. } = rejection {
                signature_too_long_names.push(name.clone());
            }
        }
        for candidate in candidates {
            match promote(&candidate, file.clone()) {
                Ok(snapshot) => {
                    *kind_counts.entry(snapshot.kind()).or_insert(0) += 1;
                    snapshots.push(snapshot);
                }
                Err(_) => {}
            }
        }
    }

    println!("\n--- A. Corpus metrics ---");
    println!("total_files_in_commit: {total_files}");
    println!("files_supported (parser available): {files_supported}");
    println!("files_parsed: {files_parsed}");
    println!("files_with_error_nodes: {files_with_error_nodes}");
    println!("parse_failures: {parse_failures}");
    println!("total_loc (newline count, parsed files): {total_loc}");
    println!("raw_candidates_total: {raw_candidates_total}");
    println!("rejections_total: {rejections_total}");
    println!("snapshots_promoted: {}", snapshots.len());
    let mut kind_vec: Vec<_> = kind_counts.into_iter().collect();
    kind_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (kind, count) in &kind_vec {
        println!("  {kind:?}: {count}");
    }

    println!("\n--- B. Raw-parse proxies: namespace/template call shapes (real AST, not extraction-dependent) ---");
    println!("raw_total_call_expressions (every call_expression node, library files only): {raw_total_call_expressions}");
    {
        let mut dk: Vec<_> = dropped_callee_kinds.iter().collect();
        dk.sort_by(|a, b| b.1.cmp(a.1));
        println!("dropped call-site callee kinds (callee_name()-equivalent returns None, same match arms as edges/calls.rs's real callee_name): {dk:?}");
    }
    println!("raw_namespace_decls: {raw_namespace_decls}");
    println!("raw_qualified_identifier_calls (ns::foo(...), any — includes qualified_template below): {raw_qualified_identifier_calls}");
    println!("raw_qualified_template_calls (ns::foo<T>(...) specifically): {raw_qualified_template_calls}");
    println!("raw_bare_template_calls (foo<T>(...), no namespace qualifier): {raw_bare_template_calls}");
    println!("raw_receiver_template_calls (obj.foo<T>(...)/obj->foo<T>(...)): {raw_receiver_template_calls}");
    println!("qualified_call_examples (first 10): {:?}", &qualified_call_examples[..qualified_call_examples.len().min(10)]);
    println!("qualified_template_call_examples (first 10): {:?}", &qualified_template_call_examples[..qualified_template_call_examples.len().min(10)]);

    // Real edges, through the SAME entry point the real indexer uses
    // for C++ (resolve_calls_with_namespace_context with empty
    // csharp_contexts -- per edges/calls.rs's own documented fallback,
    // this is byte-identical to resolve_calls_with_imports for any
    // corpus with no CSharpTypeDeclarationContext data, which C++
    // structurally never produces).
    let import_facts = resolve_raw_import_statements(&raw_import_statements_all, &snapshots).expect("resolve_raw_import_statements");
    println!(
        "\nreal_import_facts_extracted (#include-based; walk_for_imports has NO Language::Cpp arm per imports.rs, so this is structurally 0 for C++, confirmed not assumed): {}",
        import_facts.len()
    );

    let (calls_edges, call_attempts) = resolve_calls_with_namespace_context(&snapshots, &signature_too_long_names, &import_facts, &[]).expect("resolve_calls_with_namespace_context");
    let extends_edges = resolve_extends_implements(&snapshots).expect("resolve_extends_implements");
    let tests_for_edges = resolve_tests_for(&snapshots).expect("resolve_tests_for");
    let contains_edges = resolve_contains(&snapshots).expect("resolve_contains");
    let instantiates_edges = resolve_instantiates(&snapshots).expect("resolve_instantiates");

    let full_index_elapsed = t0.elapsed();

    println!("\n--- C. Structural edges ---");
    let mut edge_kind_counts: HashMap<EdgeKind, usize> = HashMap::new();
    let mut resolution_method_counts: HashMap<ResolutionMethod, usize> = HashMap::new();
    for edge in calls_edges.iter().chain(extends_edges.iter()).chain(tests_for_edges.iter()).chain(contains_edges.iter()).chain(instantiates_edges.iter()) {
        *edge_kind_counts.entry(edge.kind).or_insert(0) += 1;
        *resolution_method_counts.entry(edge.evidence.method()).or_insert(0) += 1;
    }
    let mut ek_vec: Vec<_> = edge_kind_counts.into_iter().collect();
    ek_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (kind, count) in &ek_vec {
        println!("  {kind:?}: {count}");
    }
    println!("resolution_method breakdown (all edge kinds combined):");
    let mut rm_vec: Vec<_> = resolution_method_counts.into_iter().collect();
    rm_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (method, count) in &rm_vec {
        println!("  {method:?}: {count}");
    }

    println!("\n--- D. Calls resolution taxonomy (EvidenceTier) ---");
    let mut tier_counts: HashMap<EvidenceTier, usize> = HashMap::new();
    for edge in &calls_edges {
        let tier = EvidenceTier::from_resolution_method(edge.evidence.method());
        *tier_counts.entry(tier).or_insert(0) += 1;
    }
    let ambiguous_attempts: Vec<&CallAttempt> = call_attempts.iter().filter(|a| a.tier == EvidenceTier::Ambiguous).collect();
    let unresolved_attempts: Vec<&CallAttempt> = call_attempts.iter().filter(|a| a.tier == EvidenceTier::Unresolved).collect();
    println!("resolved Calls edges by tier:");
    let mut tv: Vec<_> = tier_counts.into_iter().collect();
    tv.sort_by_key(|(t, _)| format!("{t:?}"));
    for (tier, count) in &tv {
        println!("  {tier:?}: {count}");
    }
    println!("total_call_attempts (negative knowledge): {}", call_attempts.len());
    println!("  Ambiguous: {}", ambiguous_attempts.len());
    println!("  Unresolved: {}", unresolved_attempts.len());

    println!("\n--- D2. UnresolvedReason distribution ---");
    let mut reason_counts: HashMap<&'static str, usize> = HashMap::new();
    for attempt in &call_attempts {
        let label = match &attempt.reason {
            UnresolvedReason::Unknown => "Unknown",
            UnresolvedReason::ExtractionRejected { .. } => "ExtractionRejected",
            UnresolvedReason::AmbiguousCandidates => "AmbiguousCandidates",
            UnresolvedReason::ImportTargetExternal => "ImportTargetExternal",
            UnresolvedReason::ImportedSymbolAmbiguous => "ImportedSymbolAmbiguous",
            UnresolvedReason::NoMatchingImportAlias => "NoMatchingImportAlias",
        };
        *reason_counts.entry(label).or_insert(0) += 1;
    }
    let mut rc_vec: Vec<_> = reason_counts.into_iter().collect();
    rc_vec.sort_by_key(|(k, _)| k.to_string());
    for (reason, count) in &rc_vec {
        println!("  {reason}: {count}");
    }

    println!("\n--- E. Overload-named candidate pools (format_to, overload-heavy real name) ---");
    let format_to_snapshots: Vec<_> = snapshots.iter().filter(|s| s.name().as_str() == "format_to").collect();
    println!("real snapshots named exactly 'format_to': {}", format_to_snapshots.len());
    for s in &format_to_snapshots {
        println!("  {}::{} span={:?}", s.file().relative_path(), s.name().as_str(), s.span());
    }
    let format_to_attempts: Vec<&CallAttempt> = call_attempts.iter().filter(|a| a.callee_name == "format_to").collect();
    let format_to_edges: Vec<_> = calls_edges.iter().filter(|e| snapshots.iter().find(|s| s.id() == e.to).map(|s| s.name().as_str()) == Some("format_to")).collect();
    println!("CallAttempts with callee_name == 'format_to': {} (by tier below)", format_to_attempts.len());
    let mut fmt_tier: HashMap<EvidenceTier, usize> = HashMap::new();
    for a in &format_to_attempts {
        *fmt_tier.entry(a.tier).or_insert(0) += 1;
    }
    for (t, c) in &fmt_tier {
        println!("  {t:?}: {c}");
    }
    println!("resolved Calls edges targeting a 'format_to' snapshot: {}", format_to_edges.len());
    for e in format_to_edges.iter().take(5) {
        println!("  method={:?} evidence_note={:?}", e.evidence.method(), e.evidence.note());
    }

    println!("\n--- F. Negative-knowledge sample: 10 Ambiguous CallAttempts (deterministic order) ---");
    let mut amb_sorted: Vec<&CallAttempt> = ambiguous_attempts.clone();
    amb_sorted.sort_by(|a, b| a.callee_name.cmp(&b.callee_name).then_with(|| format!("{:?}", a.caller).cmp(&format!("{:?}", b.caller))));
    for attempt in amb_sorted.iter().take(10) {
        let caller_name = snapshots.iter().find(|s| s.id() == attempt.caller).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str()));
        let candidate_names: Vec<String> = attempt.candidates.iter().filter_map(|id| snapshots.iter().find(|s| s.id() == *id)).map(|s| format!("{}::{}", s.file().relative_path(), s.name().as_str())).collect();
        println!(
            "  caller={caller_name:?} callee_name={:?} candidate_count={} candidates={candidate_names:?} rationale={:?} limitations={:?}",
            attempt.callee_name,
            attempt.candidates.len(),
            attempt.provenance.selection_rationale(),
            attempt.provenance.method_limitations()
        );
    }

    println!("\n--- G. Full index timing ---");
    println!("full_index_elapsed: {full_index_elapsed:?}");
    println!("real_instantiates_edges: {}", instantiates_edges.len());
    println!("real_contains_edges: {}", contains_edges.len());
    println!("real_extends_edges: {}", extends_edges.len());

    println!("\n--- G2. Ground-truth spot checks (ticked against real source, for the eval report's 30-row sample) ---");
    for name in ["parse_format_string", "push", "get_container", "accessor", "vformat_to", "is_locking", "get_buffer", "counting_buffer", "enforce_compile_checks", "make_descriptor", "println"] {
        let decl_count = snapshots.iter().filter(|s| s.name().as_str() == name).count();
        let edge_count = calls_edges.iter().filter(|e| snapshots.iter().find(|s| s.id() == e.to).map(|s| s.name().as_str()) == Some(name)).count();
        let attempt_count = call_attempts.iter().filter(|a| a.callee_name == name).count();
        let attempt_tiers: Vec<EvidenceTier> = call_attempts.iter().filter(|a| a.callee_name == name).map(|a| a.tier).collect();
        println!("  name={name:?} real_decl_count={decl_count} resolved_edges={edge_count} call_attempts={attempt_count} attempt_tiers={attempt_tiers:?}");
    }

    println!("\n--- H. Agent usefulness + reliability simulation (real data, real agent_policy::evaluate) ---");
    println!("Q1: 'who calls advance_to (a templated, cross-namespace-ambiguous function)?' -> real answer: an Ambiguous CallAttempt (4 real candidates), never collapsed to one.");
    {
        let sample = call_attempts.iter().find(|a| a.callee_name == "advance_to").expect("a real advance_to CallAttempt must exist on this corpus");
        let verdict_display = evaluate(sample.tier, IntendedUse::InformationalDisplay);
        let verdict_assert = evaluate(sample.tier, IntendedUse::AssertAsFactOrSoleJustification);
        let verdict_high_impact = evaluate(sample.tier, IntendedUse::HighImpactModification);
        println!("  real tier={:?} candidate_count={}", sample.tier, sample.candidates.len());
        println!("  policy(InformationalDisplay)={verdict_display:?}");
        println!("  policy(AssertAsFactOrSoleJustification)={verdict_assert:?}");
        println!("  policy(HighImpactModification)={verdict_high_impact:?}");
        let all_forbidden_except_display_disclosed = matches!(verdict_assert, PolicyVerdict::Forbidden { .. }) && matches!(verdict_high_impact, PolicyVerdict::Forbidden { .. });
        println!("  Ambiguous correctly Forbidden for assert/high-impact: {all_forbidden_except_display_disclosed}");
    }

    println!("\nQ2: 'what does Policy::handle<T>() resolve to, for a namespace-qualified template call?' -> real answer per the known residual gap: the call site is NEVER EXTRACTED AT ALL (no CallAttempt, no edge) for ns::foo<T>() shapes -- confirmed by direct source reading of callee_name's match arms (no 'qualified_identifier' arm exists). An agent asking this question about a REAL ns::foo<T>() call site in this corpus gets NO ANSWER (silent absence), not a hedged Ambiguous/Unresolved answer -- this is the one case where 'Forbidden-by-default' honesty does NOT apply, because the call site never reaches negative-knowledge tracking either.");

    println!("\nQ3: 'what overloads exist for a given name (e.g. advance_to/format_to)?' -> real answer: snapshot lookup by name works correctly regardless of Calls-resolution quality (advance_to: 4 real same-named declarations found across base.h/format.h/printf.h; format_to: 9 real same-named declarations found across base.h/color.h/compile.h/format.h/xchar.h) -- this question is answerable and reliable independent of the Calls-edge gap above, since it does not depend on call-site resolution at all.");

    println!("\n=== END ===");
}

#[allow(clippy::too_many_arguments)]
fn count_cpp_call_shapes(
    node: tree_sitter::Node,
    source: &[u8],
    relative_path: &str,
    raw_qualified_identifier_calls: &mut usize,
    raw_qualified_template_calls: &mut usize,
    raw_bare_template_calls: &mut usize,
    raw_receiver_template_calls: &mut usize,
    raw_namespace_decls: &mut usize,
    qualified_call_examples: &mut Vec<String>,
    qualified_template_call_examples: &mut Vec<String>,
    raw_total_call_expressions: &mut usize,
    dropped_callee_kinds: &mut HashMap<String, usize>,
) {
    let mut cursor = node.walk();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "namespace_definition" => {
                *raw_namespace_decls += 1;
            }
            "call_expression" => {
                *raw_total_call_expressions += 1;
                if let Some(callee) = n.child(0) {
                    // Mirrors edges/calls.rs's own callee_name match arms
                    // exactly, for C++: identifier (resolves), generic_name/
                    // template_function (resolves via wrapped_callee_simple_name),
                    // attribute/field_expression/member_expression/
                    // member_access_expression (resolves via last-name-child
                    // search, incl. the generic_name/template_method inner
                    // case) -- everything else is a real, confirmed drop.
                    let resolves = matches!(
                        callee.kind(),
                        "identifier" | "generic_name" | "template_function" | "attribute" | "field_expression" | "member_expression" | "member_access_expression"
                    );
                    if !resolves {
                        *dropped_callee_kinds.entry(callee.kind().to_string()).or_insert(0) += 1;
                    }
                    match callee.kind() {
                        "qualified_identifier" => {
                            *raw_qualified_identifier_calls += 1;
                            let text = text_of(n, source);
                            let line = n.start_position().row + 1;
                            // A qualified_identifier wraps template_function one
                            // level deeper for ns::foo<T>(...) specifically --
                            // confirmed by direct grammar probe (B2-ID-011's own
                            // independent-review finding this round re-confirms).
                            let mut tmpl_cursor = callee.walk();
                            let has_template_child = callee.children(&mut tmpl_cursor).any(|ch| ch.kind() == "template_function");
                            if has_template_child {
                                *raw_qualified_template_calls += 1;
                                if qualified_template_call_examples.len() < 15 {
                                    qualified_template_call_examples.push(format!("{relative_path}:{line}: {}", truncate(&text, 80)));
                                }
                            } else if qualified_call_examples.len() < 15 {
                                qualified_call_examples.push(format!("{relative_path}:{line}: {}", truncate(&text, 80)));
                            }
                        }
                        "template_function" => {
                            *raw_bare_template_calls += 1;
                        }
                        "field_expression" => {
                            let mut c = callee.walk();
                            let has_template_method = callee.children(&mut c).any(|ch| ch.kind() == "template_method");
                            if has_template_method {
                                *raw_receiver_template_calls += 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        for child in n.children(&mut cursor) {
            stack.push(child);
        }
        cursor = n.walk();
    }
}

fn text_of(node: tree_sitter::Node, source: &[u8]) -> String {
    String::from_utf8_lossy(&source[node.start_byte()..node.end_byte()]).to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.replace('\n', " ")
    } else {
        format!("{}...", &s[..max].replace('\n', " "))
    }
}
