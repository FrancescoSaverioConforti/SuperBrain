//! B2 Task 6: the cross-commit lineage resolver — [`resolve_lineage`] —
//! and [`LineageEdge`], its output type.
//!
//! # Confirmed vs. Hypothesized: the one rule that governs everything
//! else in this module
//!
//! `Confirmed` requires DETERMINISTIC, STRUCTURAL evidence with no
//! similarity scoring involved at all: the same repository, the same
//! logical file (by relative path — two snapshots at different commits
//! never share a `FileIdentity` by construction, since `FileIdentity`
//! binds a specific `CommitHash`; "same file" for lineage purposes means
//! same `RepositoryId` + same `relative_path`), the same `SymbolKind`,
//! the same name, and the same signature text, byte-for-byte. Nothing
//! about this check involves a continuous score or a threshold — it is
//! either an exact structural match on every one of those five fields,
//! or it is not, and `LineageStatus::confirmed` additionally enforces at
//! the type level that only `ResolutionMethod::StaticAnalysis` evidence
//! may accompany it (this resolver constructs exactly that evidence for
//! this path and no other).
//!
//! `Hypothesized` covers every other case where SOME continuity signal
//! exists but not that exact structural match: a rename (same file/kind,
//! different name, similar signature), a move (same name/kind in a
//! different file), a rename+move, or any similarity-based match in
//! general. **No similarity score, however close to 1.0, ever
//! constructs `Confirmed`** — this is enforced structurally, not just by
//! convention: this module's only call to `LineageStatus::confirmed` is
//! in the one exact-match branch below; every similarity-based branch
//! calls `LineageStatus::hypothesized` instead, and
//! `LineageStatus::confirmed` itself independently rejects
//! non-`StaticAnalysis` evidence even if a future edit to this file
//! tried to route a similarity-derived confidence through it by mistake.
//!
//! # Ambiguity: no edge beats a false edge
//!
//! If, for a given `next` snapshot, more than one `prev` candidate ties
//! at the best similarity score, this resolver emits **no edge** for
//! that `next` snapshot rather than guessing one. Each `prev` snapshot
//! is also consumed by at most one edge: once a `prev` snapshot has been
//! matched (Confirmed or Hypothesized) to some `next` snapshot, it is
//! removed from the candidate pool for every other `next` snapshot, so
//! a single deleted-and-recreated symbol cannot be matched to two
//! unrelated new symbols that happen to tie.
//!
//! An EXACT structural match (the `Confirmed` case) cannot itself be
//! ambiguous within a single, invariant-respecting candidate pool:
//! `SymbolSnapshotId` is derived deterministically from exactly
//! `(FileIdentity, SymbolKind, name, signature)`
//! (`codegraph_domain::SymbolSnapshotId::derive`), and an exact
//! structural match requires equal repository, relative path, kind,
//! name AND signature — so two DISTINCT `prev` snapshots that both
//! exactly match the same `next` snapshot would necessarily share the
//! same `SymbolSnapshotId` too (their `FileIdentity`s differ only in
//! commit, which `derive` does not hash). Two entries in the candidate
//! pool with the SAME id is therefore not a real domain ambiguity to
//! resolve heuristically — it is an invariant violation by whatever
//! constructed the pool (e.g. a caller that accidentally included the
//! same snapshot twice), and [`resolve_lineage`] rejects that input
//! outright via [`LineageResolutionError::DuplicateSnapshotId`] rather
//! than silently deduplicating it or leaving a dead "what if there are
//! two" branch in the matching logic that no legitimate input could ever
//! reach.

use std::collections::HashSet;

use codegraph_domain::{Confidence, Evidence, FileIdentity, LineageStatus, ResolutionMethod, SymbolLineageId, SymbolSnapshotId};
use thiserror::Error;

use super::similarity::similarity_score;
use crate::snapshot::SymbolSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LineageResolutionError {
    /// `prev` or `next` contained the same `SymbolSnapshotId` more than
    /// once — a caller invariant violation (see this module's doc
    /// comment on why exact-match ambiguity cannot arise from distinct,
    /// legitimately-constructed snapshots), never a condition this
    /// resolver tries to recover from or silently paper over.
    #[error("duplicate SymbolSnapshotId in the candidate pool: a caller must never include the same snapshot twice")]
    DuplicateSnapshotId,
}

/// A resolved (or hypothesized) continuation of one symbol from an
/// earlier snapshot to a later one. `lineage` is either the earlier
/// snapshot's own already-known lineage id (if it had one — i.e. this is
/// not the first time this chain has been resolved) or a freshly derived
/// genesis id if `from` is this chain's first observed snapshot. This
/// type is never constructed with a bare `SymbolLineageId::from_genesis`
/// call outside [`resolve_lineage`]/[`LineageChainTracker`]'s own
/// bookkeeping — a caller does not mint lineage ids by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageEdge {
    pub from: SymbolSnapshotId,
    pub to: SymbolSnapshotId,
    pub lineage: SymbolLineageId,
    pub status: LineageStatus,
}

/// The minimum similarity score (see `similarity.rs`) a candidate pair
/// must reach to be considered a plausible `Hypothesized` continuation
/// at all. Below this floor, two snapshots are treated as unrelated
/// (e.g. an unrelated new symbol that merely shares a kind with some
/// deleted one) rather than a very weak hypothesis — a `Hypothesized`
/// edge with near-zero confidence would be noise, not a useful signal,
/// and the task's own "no edge beats a false edge" principle applies to
/// weak hypotheses too, not only to ambiguous ones.
const MIN_HYPOTHESIS_SCORE: f64 = 0.3;

/// Resolves lineage edges from `prev` (an earlier commit's snapshots) to
/// `next` (a later commit's snapshots). `known_lineage_of` is supplied
/// by the caller (a future incremental-indexing task) to answer "does
/// this `prev` snapshot already belong to a known lineage chain from an
/// even earlier resolution?" — if so, that existing `SymbolLineageId` is
/// propagated forward rather than this resolver deriving a new genesis
/// for every single run. A `prev` snapshot with no known prior lineage
/// is treated as its own chain's genesis.
///
/// Deterministic: calling this twice with the same `prev`, `next` and
/// `known_lineage_of` always returns the same edges in the same order
/// (iteration order follows `next`'s own slice order; within that,
/// candidate selection is total — see `best_candidate`'s tie-breaking,
/// which never depends on hash-map iteration order or any other
/// non-deterministic source).
pub fn resolve_lineage(
    prev: &[SymbolSnapshot],
    next: &[SymbolSnapshot],
    known_lineage_of: &dyn Fn(SymbolSnapshotId) -> Option<SymbolLineageId>,
) -> Result<Vec<LineageEdge>, LineageResolutionError> {
    reject_duplicate_ids(prev)?;
    reject_duplicate_ids(next)?;

    let mut available: Vec<&SymbolSnapshot> = prev.iter().collect();
    let mut edges = Vec::new();

    for next_snapshot in next {
        let Some((candidate, status)) = best_candidate(&available, next_snapshot) else {
            continue;
        };

        let lineage = known_lineage_of(candidate.id()).unwrap_or_else(|| SymbolLineageId::from_genesis(candidate.id()));

        edges.push(LineageEdge {
            from: candidate.id(),
            to: next_snapshot.id(),
            lineage,
            status,
        });

        // Consumed: this `prev` snapshot cannot also be matched to a
        // different `next` snapshot in this same resolution pass.
        let consumed_id = candidate.id();
        available.retain(|s| s.id() != consumed_id);
    }

    Ok(edges)
}

/// Fails closed on a duplicate `SymbolSnapshotId` within `snapshots` —
/// see this module's doc comment for why that can only be a caller
/// invariant violation, never a real domain ambiguity.
fn reject_duplicate_ids(snapshots: &[SymbolSnapshot]) -> Result<(), LineageResolutionError> {
    let mut seen = HashSet::with_capacity(snapshots.len());
    for snapshot in snapshots {
        if !seen.insert(snapshot.id()) {
            return Err(LineageResolutionError::DuplicateSnapshotId);
        }
    }
    Ok(())
}

/// Finds the single best, unambiguous `prev` candidate for `next_snapshot`,
/// or `None` if there is no plausible candidate at all, or more than one
/// candidate ties for best. Given `resolve_lineage`'s upfront
/// `reject_duplicate_ids` check, `available` never contains two entries
/// with the same `SymbolSnapshotId` — so at most one exact structural
/// match can ever exist for a given `next_snapshot` (see this module's
/// doc comment: two distinct exact matches would require two distinct
/// `prev` entries sharing a `SymbolSnapshotId`, already excluded). There
/// is deliberately no "more than one exact match" branch here: that
/// state is unreachable given the precondition this function is always
/// called under, not a case silently ignored.
fn best_candidate<'a>(available: &[&'a SymbolSnapshot], next_snapshot: &SymbolSnapshot) -> Option<(&'a SymbolSnapshot, LineageStatus)> {
    // Step 1: an exact structural match (the Confirmed candidate), if any.
    let exact_match = available.iter().find(|prev_snapshot| is_exact_structural_match(prev_snapshot, next_snapshot));

    if let Some(prev_snapshot) = exact_match {
        let prev_snapshot = *prev_snapshot;
        let evidence = exact_match_evidence(prev_snapshot, next_snapshot);
        let status = LineageStatus::confirmed(evidence).expect("StaticAnalysis evidence always satisfies LineageStatus::confirmed");
        return Some((prev_snapshot, status));
    }

    // Step 2: similarity-based candidates (Hypothesized only), among
    // same-kind snapshots not already claimed by an exact match (there
    // was none at this point, since exact_match was None).
    let same_kind: Vec<&&SymbolSnapshot> = available.iter().filter(|prev_snapshot| prev_snapshot.kind() == next_snapshot.kind()).collect();

    let mut scored: Vec<(&&SymbolSnapshot, f64)> = same_kind
        .into_iter()
        .map(|prev_snapshot| {
            let score = similarity_score(
                prev_snapshot.name().as_str(),
                next_snapshot.name().as_str(),
                prev_snapshot.signature().as_str(),
                next_snapshot.signature().as_str(),
            );
            (prev_snapshot, score)
        })
        .filter(|(_, score)| *score >= MIN_HYPOTHESIS_SCORE)
        .collect();

    if scored.is_empty() {
        return None;
    }

    // Deterministic ordering: sort by score descending, then by the
    // candidate's own SymbolSnapshotId bytes as a total, stable
    // tie-breaker so this function's result never depends on `available`'s
    // incoming order (which Vec::retain preserves, but this still makes
    // the determinism explicit and independently verifiable rather than
    // incidental).
    scored.sort_by(|(snapshot_a, score_a), (snapshot_b, score_b)| {
        score_b.partial_cmp(score_a).unwrap().then_with(|| snapshot_a.id().as_bytes().cmp(snapshot_b.id().as_bytes()))
    });

    let best_score = scored[0].1;
    let tied_at_best = scored.iter().filter(|(_, score)| *score == best_score).count();
    if tied_at_best > 1 {
        // Two or more equally-plausible candidates: ambiguous rename/move
        // target. No edge.
        return None;
    }

    let (prev_snapshot, score) = scored[0];
    let confidence = Confidence::from_score(score).expect("similarity_score always returns a finite value in [0.0, 1.0]");
    let evidence = similarity_evidence(prev_snapshot, next_snapshot, score);
    let status = LineageStatus::hypothesized(evidence, confidence);
    Some((*prev_snapshot, status))
}

fn is_exact_structural_match(prev_snapshot: &SymbolSnapshot, next_snapshot: &SymbolSnapshot) -> bool {
    same_repository_and_path(prev_snapshot.file(), next_snapshot.file())
        && prev_snapshot.kind() == next_snapshot.kind()
        && prev_snapshot.name().as_str() == next_snapshot.name().as_str()
        && prev_snapshot.signature().as_str() == next_snapshot.signature().as_str()
}

fn same_repository_and_path(a: &FileIdentity, b: &FileIdentity) -> bool {
    a.repository() == b.repository() && a.relative_path() == b.relative_path()
}

fn exact_match_evidence(prev_snapshot: &SymbolSnapshot, _next_snapshot: &SymbolSnapshot) -> Evidence {
    Evidence::new(
        ResolutionMethod::StaticAnalysis,
        None,
        &format!(
            "exact structural match: same repository, same file path ({}), same kind, same name, same signature",
            prev_snapshot.file().relative_path()
        ),
    )
    .expect("this note is well within MAX_NOTE_LEN")
}

fn similarity_evidence(prev_snapshot: &SymbolSnapshot, next_snapshot: &SymbolSnapshot, score: f64) -> Evidence {
    let same_path = prev_snapshot.file().relative_path() == next_snapshot.file().relative_path();
    let same_name = prev_snapshot.name().as_str() == next_snapshot.name().as_str();
    // `NameHeuristic` when the name itself is part of what matched
    // (same name somewhere in the comparison, even across a file move);
    // `Inferred` when the name differed too and the match rests
    // entirely on signature-text similarity, a weaker, more indirect
    // signal than an actual name correlation.
    let (method, reason) = match (same_path, same_name) {
        (true, false) => (ResolutionMethod::Inferred, "same file, different name, matched by signature similarity (possible rename)"),
        (false, true) => (ResolutionMethod::NameHeuristic, "different file, same name (possible move)"),
        (false, false) => (
            ResolutionMethod::Inferred,
            "different file and name, matched by signature similarity alone (possible rename+move)",
        ),
        (true, true) => (
            ResolutionMethod::NameHeuristic,
            "same file and name, but signature differs (not an exact structural match)",
        ),
    };
    Evidence::new(method, None, &format!("{reason}; similarity score {score:.3}")).expect("this note is well within MAX_NOTE_LEN")
}
