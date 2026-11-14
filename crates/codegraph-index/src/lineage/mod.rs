//! B2 Task 6: cross-commit lineage resolution. See `resolver.rs`'s
//! module doc comment for the Confirmed/Hypothesized rule and the
//! ambiguity-handling policy. No typed non-lineage edges (Task 7), no
//! graph storage (Task 8), no persistence, no API, no MCP.

mod resolver;
mod similarity;

pub use resolver::{resolve_lineage, LineageEdge, LineageResolutionError};

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_domain::{CommitHash, FileIdentity, RepositoryRegistry, RepositoryRoot, SymbolKind};

    use crate::extract::{ByteSpan, RawSymbolCandidate};
    use crate::snapshot::promote;

    fn test_repo_and_commits() -> (codegraph_domain::RepositoryId, CommitHash, CommitHash) {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        (repo, commit_a, commit_b)
    }

    fn make_snapshot(
        repo: codegraph_domain::RepositoryId,
        commit: CommitHash,
        path: &str,
        name: &str,
        kind_hint: &str,
        signature: &str,
    ) -> crate::snapshot::SymbolSnapshot {
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start: 0, end: 10 }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    fn no_known_lineage(_id: codegraph_domain::SymbolSnapshotId) -> Option<codegraph_domain::SymbolLineageId> {
        None
    }

    // 1. Same symbol, next commit -> Confirmed.
    #[test]
    fn identical_symbol_in_the_next_commit_is_confirmed() {
        let (repo, commit_a, commit_b) = test_repo_and_commits();
        let prev = vec![make_snapshot(repo, commit_a, "src/lib.rs", "foo", "function_item", "fn foo() {}")];
        let next = vec![make_snapshot(repo, commit_b, "src/lib.rs", "foo", "function_item", "fn foo() {}")];

        let edges = resolve_lineage(&prev, &next, &no_known_lineage).unwrap();

        assert_eq!(edges.len(), 1);
        assert!(edges[0].status.is_confirmed(), "expected Confirmed for an exact structural match");
    }

    // 2. Rename -> Hypothesized.
    #[test]
    fn a_rename_in_the_same_file_is_hypothesized_not_confirmed() {
        let (repo, commit_a, commit_b) = test_repo_and_commits();
        let prev = vec![make_snapshot(repo, commit_a, "src/lib.rs", "foo", "function_item", "fn foo(x: i32) -> i32 { x + 1 }")];
        let next = vec![make_snapshot(repo, commit_b, "src/lib.rs", "bar", "function_item", "fn bar(x: i32) -> i32 { x + 1 }")];

        let edges = resolve_lineage(&prev, &next, &no_known_lineage).unwrap();

        assert_eq!(edges.len(), 1);
        assert!(!edges[0].status.is_confirmed(), "a rename, however similar the body, must never be Confirmed");
        match &edges[0].status {
            codegraph_domain::LineageStatus::Hypothesized { evidence, .. } => {
                // same_path = true, same_name = false -> the
                // `(true, false)` evidence branch specifically.
                assert_eq!(evidence.method(), codegraph_domain::ResolutionMethod::Inferred);
                assert!(evidence.note().contains("same file"), "expected the same-file rename reason, got {:?}", evidence.note());
            }
            other => panic!("expected Hypothesized, got {other:?}"),
        }
    }

    // 3. Move -> Hypothesized (no explicit strong evidence mechanism
    // exists in this resolver to ever justify Confirmed across a path
    // change, by design: "same file" is one of Confirmed's five
    // required exact-match fields).
    #[test]
    fn a_move_to_a_different_file_is_hypothesized() {
        let (repo, commit_a, commit_b) = test_repo_and_commits();
        let prev = vec![make_snapshot(repo, commit_a, "src/old.rs", "foo", "function_item", "fn foo() {}")];
        let next = vec![make_snapshot(repo, commit_b, "src/new.rs", "foo", "function_item", "fn foo() {}")];

        let edges = resolve_lineage(&prev, &next, &no_known_lineage).unwrap();

        assert_eq!(edges.len(), 1);
        assert!(!edges[0].status.is_confirmed(), "a file move must never be Confirmed, even with identical name+signature");
        match &edges[0].status {
            codegraph_domain::LineageStatus::Hypothesized { evidence, .. } => {
                // same_path = false, same_name = true -> the
                // `(false, true)` evidence branch specifically.
                assert_eq!(evidence.method(), codegraph_domain::ResolutionMethod::NameHeuristic);
                assert!(evidence.note().contains("different file, same name"), "expected the move reason, got {:?}", evidence.note());
            }
            other => panic!("expected Hypothesized, got {other:?}"),
        }
    }

    #[test]
    fn same_file_and_name_but_different_signature_is_hypothesized_with_its_own_evidence_reason() {
        // same_path = true, same_name = true, but signature differs ->
        // the `(true, true)` evidence branch — not an exact structural
        // match (signature differs), so Hypothesized, with a reason
        // distinct from both the rename and move cases above.
        let (repo, commit_a, commit_b) = test_repo_and_commits();
        let prev = vec![make_snapshot(repo, commit_a, "src/lib.rs", "foo", "function_item", "fn foo(x: i32) -> i32 { x + 1 }")];
        let next = vec![make_snapshot(repo, commit_b, "src/lib.rs", "foo", "function_item", "fn foo(x: i32) -> i32 { x + 2 }")];

        let edges = resolve_lineage(&prev, &next, &no_known_lineage).unwrap();

        assert_eq!(edges.len(), 1);
        assert!(!edges[0].status.is_confirmed());
        match &edges[0].status {
            codegraph_domain::LineageStatus::Hypothesized { evidence, .. } => {
                assert_eq!(evidence.method(), codegraph_domain::ResolutionMethod::NameHeuristic);
                assert!(evidence.note().contains("signature differs"), "expected the signature-differs reason, got {:?}", evidence.note());
            }
            other => panic!("expected Hypothesized, got {other:?}"),
        }
    }

    // 4. Rename + move -> Hypothesized.
    #[test]
    fn a_rename_combined_with_a_move_is_hypothesized() {
        let (repo, commit_a, commit_b) = test_repo_and_commits();
        let prev = vec![make_snapshot(repo, commit_a, "src/old.rs", "foo", "function_item", "fn foo(x: i32) -> i32 { x + 1 }")];
        let next = vec![make_snapshot(repo, commit_b, "src/new.rs", "bar", "function_item", "fn bar(x: i32) -> i32 { x + 1 }")];

        let edges = resolve_lineage(&prev, &next, &no_known_lineage).unwrap();

        assert_eq!(edges.len(), 1);
        assert!(!edges[0].status.is_confirmed());
    }

    // 5. Delete -> no new lineage for it (nothing in `next` references
    // the deleted symbol, so no edge mentions it at all).
    #[test]
    fn a_deleted_symbol_produces_no_edge_referencing_it() {
        let (repo, commit_a, commit_b) = test_repo_and_commits();
        let prev = vec![make_snapshot(repo, commit_a, "src/lib.rs", "foo", "function_item", "fn foo() {}")];
        let next: Vec<crate::snapshot::SymbolSnapshot> = vec![make_snapshot(repo, commit_b, "src/lib.rs", "unrelated", "function_item", "fn unrelated() { totally_different_body_xyz(); }")];

        let edges = resolve_lineage(&prev, &next, &no_known_lineage).unwrap();

        assert!(edges.is_empty(), "an unrelated new symbol must not be linked to the deleted one");
    }

    // 6. Unrelated same-name symbol (different content/signature,
    // different file, low similarity) -> no lineage.
    #[test]
    fn an_unrelated_symbol_with_the_same_name_but_different_everything_else_is_not_linked() {
        let (repo, commit_a, commit_b) = test_repo_and_commits();
        let prev = vec![make_snapshot(repo, commit_a, "src/a.rs", "run", "function_item", "fn run() { start_server(); }")];
        let next = vec![make_snapshot(repo, commit_b, "src/b.rs", "run", "method_declaration", "void run() { executeTask(); }")];
        // Different KIND entirely (function_item -> method_declaration
        // maps to Function vs Method), which already excludes it from
        // same-kind candidacy — the clearest "no lineage" case.

        let edges = resolve_lineage(&prev, &next, &no_known_lineage).unwrap();

        assert!(edges.is_empty(), "a different SymbolKind must never be linked, regardless of name");
    }

    // 7. Ambiguity -> no Confirmed (and no edge at all). This is the
    // genuine tied-score case: TWO `prev` candidates competing for the
    // SAME SINGLE `next` snapshot, both scoring identically against it
    // (same name length/content pattern relative to the target, same
    // signature length) — the resolver must not arbitrarily pick one.
    #[test]
    fn two_prev_candidates_tied_at_the_best_score_for_one_next_snapshot_produce_no_edge() {
        let (repo, commit_a, commit_b) = test_repo_and_commits();
        // "aaa" and "bbb" are symmetric with respect to "ccc": each
        // shares zero name bytes with it, and their signatures are
        // constructed to have an identical-length common substring with
        // "ccc"'s signature, so both score EXACTLY the same.
        let prev = vec![
            make_snapshot(repo, commit_a.clone(), "src/lib.rs", "aaa", "function_item", "fn x(n: i32) -> i32 { n }"),
            make_snapshot(repo, commit_a, "src/other.rs", "bbb", "function_item", "fn x(n: i32) -> i32 { n }"),
        ];
        let next = vec![make_snapshot(repo, commit_b, "src/third.rs", "ccc", "function_item", "fn x(n: i32) -> i32 { n }")];

        let edges = resolve_lineage(&prev, &next, &no_known_lineage).unwrap();

        assert!(edges.is_empty(), "two prev candidates tied at the best score for the same next snapshot must produce no edge, not an arbitrary pick");
    }

    #[test]
    fn a_duplicate_snapshot_id_in_the_candidate_pool_is_a_typed_rejection() {
        let (repo, commit_a, commit_b) = test_repo_and_commits();
        let snapshot = make_snapshot(repo, commit_a, "src/lib.rs", "foo", "function_item", "fn foo() {}");
        // The exact same snapshot included twice in `prev` — a caller
        // invariant violation this resolver must reject outright, per
        // this module's doc comment on why exact-match ambiguity cannot
        // arise from distinct, legitimate snapshots.
        let prev = vec![snapshot.clone(), snapshot];
        let next = vec![make_snapshot(repo, commit_b, "src/lib.rs", "foo", "function_item", "fn foo() {}")];

        let result = resolve_lineage(&prev, &next, &no_known_lineage);

        assert_eq!(result, Err(LineageResolutionError::DuplicateSnapshotId));
    }

    #[test]
    fn two_equally_plausible_rename_targets_produce_one_edge_not_two() {
        let (repo, commit_a, commit_b) = test_repo_and_commits();
        let prev = vec![make_snapshot(repo, commit_a, "src/lib.rs", "foo", "function_item", "fn foo(x: i32) -> i32 { x + 1 }")];
        // Two candidates in `next` that are EQUALLY similar to the one
        // `prev` snapshot (identical signatures, different but
        // equally-plausible names) — ambiguous: which one, if either, is
        // the real continuation?
        let next = vec![
            make_snapshot(repo, commit_b.clone(), "src/lib.rs", "bar", "function_item", "fn bar(x: i32) -> i32 { x + 1 }"),
            make_snapshot(repo, commit_b, "src/lib.rs", "baz", "function_item", "fn baz(x: i32) -> i32 { x + 1 }"),
        ];

        let edges = resolve_lineage(&prev, &next, &no_known_lineage).unwrap();

        // Each `next` snapshot independently considers the SAME single
        // `prev` candidate its best (and only) match, since nothing
        // distinguishes "bar" from "baz" in similarity to "foo" (same
        // score). The resolver consumes `prev` after the FIRST match,
        // so exactly one edge is produced (to whichever next snapshot is
        // processed first) and the second finds no remaining candidate
        // at all. This is the resolver's per-prev-snapshot consumption
        // rule keeping a single symbol from fanning out to multiple
        // plausible targets, which is the ambiguity this test is really
        // about: the SECOND candidate must NOT also claim the same prev
        // snapshot once it's consumed.
        assert_eq!(edges.len(), 1, "the already-consumed prev snapshot must not be claimed twice");
    }

    // 8. Round-trip does not change Hypothesized into Confirmed.
    #[test]
    fn resolving_the_same_hypothesized_pair_again_never_yields_confirmed() {
        let (repo, commit_a, commit_b) = test_repo_and_commits();
        let prev = vec![make_snapshot(repo, commit_a, "src/lib.rs", "foo", "function_item", "fn foo(x: i32) -> i32 { x + 1 }")];
        let next = vec![make_snapshot(repo, commit_b, "src/lib.rs", "bar", "function_item", "fn bar(x: i32) -> i32 { x + 1 }")];

        let first_run = resolve_lineage(&prev, &next, &no_known_lineage).unwrap();
        let second_run = resolve_lineage(&prev, &next, &no_known_lineage).unwrap();

        assert!(!first_run[0].status.is_confirmed());
        assert!(!second_run[0].status.is_confirmed(), "re-running the resolver must never upgrade a Hypothesized result to Confirmed");
    }

    // 9. Deterministic resolver: same input, same output (including the
    // same lineage id, verifying genesis derivation is also stable).
    #[test]
    fn the_resolver_is_fully_deterministic_across_independent_runs() {
        let (repo, commit_a, commit_b) = test_repo_and_commits();
        let prev = vec![make_snapshot(repo, commit_a, "src/lib.rs", "foo", "function_item", "fn foo() {}")];
        let next = vec![make_snapshot(repo, commit_b, "src/lib.rs", "foo", "function_item", "fn foo() {}")];

        let run_1 = resolve_lineage(&prev, &next, &no_known_lineage).unwrap();
        let run_2 = resolve_lineage(&prev, &next, &no_known_lineage).unwrap();

        assert_eq!(run_1, run_2);
    }

    #[test]
    fn a_known_prior_lineage_is_propagated_forward_not_rederived() {
        let (repo, commit_a, commit_b) = test_repo_and_commits();
        let prev_snapshot = make_snapshot(repo, commit_a, "src/lib.rs", "foo", "function_item", "fn foo() {}");
        let prev_id = prev_snapshot.id();
        let existing_lineage = codegraph_domain::SymbolLineageId::from_genesis(codegraph_domain::SymbolSnapshotId::derive(
            prev_snapshot.file(),
            SymbolKind::Function,
            "some_earlier_genesis",
            "fn some_earlier_genesis() {}",
            0,
            29,
        ));

        let prev = vec![prev_snapshot];
        let next = vec![make_snapshot(repo, commit_b, "src/lib.rs", "foo", "function_item", "fn foo() {}")];

        let edges = resolve_lineage(&prev, &next, &move |id| if id == prev_id { Some(existing_lineage) } else { None }).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].lineage, existing_lineage, "an already-known lineage must be propagated forward, not re-derived as a new genesis");
    }
}
