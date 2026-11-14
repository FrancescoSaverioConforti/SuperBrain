//! B2 Task 9: the one-shot "parse -> extract -> promote -> lineage ->
//! edges -> `CodeGraph`" pipeline, pure and side-effect-free (no shared
//! state, no publication) — [`index_files`] is called by
//! [`super::indexer::CodeGraphIndexer`] OUTSIDE its publication lock, so
//! a slow/large reindex never blocks a concurrent reader of the
//! currently-published graph. Nothing here composed before this task:
//! every earlier task's own module only unit-tested its one stage in
//! isolation against hand-built fixtures (see this module's own tests,
//! which exercise the whole chain against `RepositorySource`-backed
//! fixtures instead).

use codegraph_domain::{CommitHash, FileIdentity, RepositoryId, RepositorySource, SourceError, SymbolSnapshotId};
use thiserror::Error;

use crate::csharp_declarations::{extract_csharp_type_declaration_contexts, resolve_csharp_type_declaration_contexts, RawCSharpTypeDeclarationContext};
use crate::edges::{resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge, Edge, EdgeResolutionError};
use crate::extract::{extract_symbols_reporting_rejections, CandidateRejection};
use crate::graph::{CodeGraph, GraphError};
use crate::negative_knowledge::CallAttempt;
use crate::impls::{extract_impl_statements, RawImplStatement};
use crate::imports::{extract_import_statements, resolve_raw_import_statements, ImportFactError, RawImportStatement};
use crate::lineage::{resolve_lineage, LineageEdge, LineageResolutionError};
use crate::parse::parser_for;
use crate::snapshot::{promote, PromotionError, SymbolSnapshot};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PipelineError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Promotion(#[from] PromotionError),
    #[error(transparent)]
    Lineage(#[from] LineageResolutionError),
    #[error(transparent)]
    EdgeResolution(#[from] EdgeResolutionError),
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    ImportResolution(#[from] ImportFactError),
    #[error(transparent)]
    ImplResolution(#[from] crate::impls::ImplFactError),
}

/// Everything [`index_files`] produced for one commit, before any
/// publication decision is made. Deliberately NOT a `CodeGraph` by
/// itself — the caller ([`super::indexer::CodeGraphIndexer`]) decides
/// whether/how to fold this into the currently-published graph (for a
/// full reindex, this *is* the whole graph; for an incremental update,
/// the caller merges it with the untouched portion of the previous
/// graph — see `indexer.rs`).
#[derive(Debug, Clone)]
pub struct PipelineOutput {
    pub snapshots: Vec<SymbolSnapshot>,
    pub lineage_edges: Vec<LineageEdge>,
    pub typed_edges: Vec<Edge>,
    /// Task 9.5, Priority 6: raw (UNRESOLVED) import statements
    /// extracted from exactly the files this call parsed, paired with
    /// the `FileIdentity` that imports them. Deliberately raw, not
    /// `ImportFact` — resolving an import against the full repository-
    /// wide snapshot pool requires the pool `indexer.rs` assembles
    /// AFTER this function returns (the same reason `typed_edges`
    /// itself is resolved from the full current pool, not from
    /// `relative_paths` alone); this function only has the files it
    /// was actually asked to parse. See `imports` module's own doc
    /// comment for why this split (local syntax extraction vs. global
    /// semantic resolution) exists.
    pub raw_import_statements: Vec<(FileIdentity, RawImportStatement)>,
    /// B2-ID-008, Task 4: raw (UNRESOLVED) `impl` blocks extracted from
    /// exactly the files this call parsed, paired with the
    /// `FileIdentity` that defines them. Mirrors
    /// `raw_import_statements` exactly, for the identical reason: a raw
    /// `impl` block's `self_type_text`/`trait_type_text` resolution
    /// against the full repository-wide snapshot pool requires the pool
    /// `indexer.rs` assembles AFTER this function returns — see
    /// `impls` module's own doc comment.
    pub raw_impl_statements: Vec<(FileIdentity, RawImplStatement)>,
    /// B2-ID-010: raw (UNRESOLVED) C# `class_declaration`/
    /// `interface_declaration` contexts extracted from exactly the
    /// files this call parsed, paired with the `FileIdentity` that
    /// defines them. Mirrors `raw_import_statements`/`raw_impl_statements`
    /// exactly, for the identical reason: resolving a context's own
    /// `snapshot_id` requires the pool `indexer.rs` assembles AFTER
    /// this function returns — see `csharp_declarations` module's own
    /// doc comment.
    pub raw_csharp_type_declaration_contexts: Vec<(FileIdentity, RawCSharpTypeDeclarationContext)>,
    /// Semantic Resolution & Reliability Checkpoint, Priority 1: the
    /// negative-knowledge `CallAttempt` record for every call site
    /// `resolve_calls_with_extraction_rejections` examined in THIS
    /// call's own typed-edge resolution pass — mirrors `typed_edges`
    /// exactly (same resolution call, same pool, same scope), never
    /// incrementally reused on its own (see `indexer.rs`'s own
    /// re-resolution convention for why).
    pub call_attempts: Vec<CallAttempt>,
    /// Semantic Resolution & Reliability Checkpoint, Priority 3: every
    /// real `SignatureTooLong` rejection's own candidate name, paired
    /// with the `FileIdentity` it was rejected from — collected from
    /// exactly the files this call parsed. Kept per-file (mirroring
    /// `raw_import_statements`/`raw_impl_statements` exactly) so
    /// `indexer.rs`'s incremental path can reuse an untouched file's
    /// own rejection names rather than re-deriving them, while still
    /// dropping a changed/deleted file's stale names correctly. This
    /// is the independently-verified evidence
    /// `resolve_calls_with_extraction_rejections` requires before it
    /// may ever link an `Unresolved` `CallAttempt` to
    /// `UnresolvedReason::ExtractionRejected` (see that function's own
    /// doc comment: it performs no rejection detection of its own,
    /// only consumes what its caller already verified).
    pub signature_too_long_names: Vec<(FileIdentity, String)>,
}

/// Parses, extracts and promotes every file in `relative_paths` (all
/// assumed to belong to `repository`/`commit`), via `source` — the
/// ONLY repository-reading seam this function uses (Task 2's
/// `RepositorySource`; no raw filesystem walk). A file whose extension
/// names no known [`crate::parse::Language`] (per `parser_for`) is
/// silently skipped — this mirrors every edge resolver's own "no
/// language, no edge" policy (Task 7) rather than failing the whole
/// pass over one unsupported file (e.g. a `.md`/`.json` file sitting
/// alongside source code in a real repository must not abort indexing
/// of everything else).
///
/// `prev_snapshots` and `known_lineage_of` are threaded straight into
/// [`resolve_lineage`] unchanged — this function adds no lineage
/// policy of its own, it only wires the already-existing resolver into
/// a real `RepositorySource`-driven pipeline for the first time.
///
/// Fails CLOSED on the first error from any stage (source read,
/// promotion, lineage, edge resolution) — returns immediately, with NO
/// partial `PipelineOutput`. The caller is responsible for ensuring a
/// failure here never touches the currently-published graph (see
/// `indexer.rs`'s error-rollback semantics).
pub fn index_files(
    source: &dyn RepositorySource,
    repository: RepositoryId,
    commit: &CommitHash,
    relative_paths: &[String],
    prev_snapshots: &[SymbolSnapshot],
    known_lineage_of: &dyn Fn(SymbolSnapshotId) -> Option<codegraph_domain::SymbolLineageId>,
) -> Result<PipelineOutput, PipelineError> {
    let mut snapshots = Vec::new();
    let mut raw_import_statements = Vec::new();
    let mut raw_impl_statements = Vec::new();
    let mut raw_csharp_type_declaration_contexts = Vec::new();
    let mut signature_too_long_names = Vec::new();

    for relative_path in relative_paths {
        let file = FileIdentity::parse(repository, commit.clone(), relative_path).map_err(|e| PipelineError::Source(SourceError::Other { reason: e.to_string() }))?;
        let Some(parser) = parser_for(&file) else {
            continue;
        };
        let bytes = source.read_file(&file)?;
        let Ok(tree) = parser.parse(&bytes) else {
            // A ParseError here is this module's own resource-budget
            // guard (Task 13's own domain, not this task's) — e.g. a
            // source file larger than `MAX_SOURCE_BYTES`. Skipped, not
            // fatal to the rest of the commit, mirroring the
            // unsupported-extension skip above: one oversized/corrupt
            // file must not abort indexing everything else.
            continue;
        };
        // Semantic Resolution & Reliability Checkpoint, Priority 3:
        // `extract_symbols_reporting_rejections` (not the rejection-
        // discarding `extract_symbols`) so a real `SignatureTooLong`
        // rejection's own candidate name is observable here, scoped to
        // exactly the files this call parsed — mirroring every other
        // raw-fact extraction's own scope in this function.
        let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
        for rejection in rejections {
            if let CandidateRejection::SignatureTooLong { name, .. } = rejection {
                signature_too_long_names.push((file.clone(), name));
            }
        }
        for candidate in candidates {
            // promote's own errors (oversized name/signature, invalid
            // span, unrecognized kind hint) ARE fatal to the whole
            // commit, per this function's fail-closed contract — a
            // promotion failure signals something more fundamentally
            // wrong (e.g. a kind-mapping gap) than one skippable file.
            snapshots.push(promote(&candidate, file.clone())?);
        }
        // Priority 6: raw import extraction is pure syntax (no
        // snapshot-pool dependency), so it runs here, scoped to exactly
        // the files this call parsed — mirroring symbol extraction's
        // own scope, never the full repository.
        for statement in extract_import_statements(&tree) {
            raw_import_statements.push((file.clone(), statement));
        }
        // B2-ID-008, Task 4: raw impl extraction is likewise pure
        // syntax (no snapshot-pool dependency), scoped to exactly the
        // files this call parsed — mirroring import extraction's own
        // scope immediately above.
        for statement in extract_impl_statements(&tree) {
            raw_impl_statements.push((file.clone(), statement));
        }
        // B2-ID-010: raw C# type-declaration context extraction is
        // likewise pure syntax (no snapshot-pool dependency), scoped to
        // exactly the files this call parsed — mirroring import/impl
        // extraction's own scope immediately above. Yields an empty
        // Vec unconditionally for any non-C# file (checked internally
        // by `extract_csharp_type_declaration_contexts` itself).
        for context in extract_csharp_type_declaration_contexts(&tree) {
            raw_csharp_type_declaration_contexts.push((file.clone(), context));
        }
    }

    let lineage_edges = resolve_lineage(prev_snapshots, &snapshots, known_lineage_of)?;

    // B2-ID-010: this call's own typed-edge resolution uses ONLY the
    // contexts extracted from THIS call's own files — correct for a
    // full reindex (every file, hence every context, is present), but
    // `indexer.rs`'s incremental path (NormalAdvance) re-resolves
    // typed_edges separately against the full current pool plus the
    // full reused+fresh context set, exactly as it already does for
    // import_facts/impl_facts; see that module's own doc comment.
    let csharp_contexts = resolve_csharp_type_declaration_contexts(&raw_csharp_type_declaration_contexts, &snapshots);
    // CALL-RES-A4 wiring: mirrors `csharp_contexts` immediately above
    // exactly, for the identical reason — this call's own
    // `raw_import_statements` only covers THIS call's own files,
    // correct for a full reindex (every file's own import statement is
    // present), but `indexer.rs`'s incremental path re-resolves
    // `import_facts` separately against the full current pool, same as
    // `csharp_contexts`.
    let import_facts = resolve_raw_import_statements(&raw_import_statements, &snapshots)?;
    // Semantic Resolution & Reliability Checkpoint, Priority 1 + 3:
    // this call's own `signature_too_long_names` (collected above,
    // scoped to exactly the files this call parsed) is the real,
    // independently-verified evidence `resolve_calls_with_extraction_rejections`
    // requires before linking an Unresolved CallAttempt to
    // UnresolvedReason::ExtractionRejected — never fabricated, per
    // that function's own "performs no rejection detection of its own"
    // discipline.
    let signature_too_long_names_only: Vec<String> = signature_too_long_names.iter().map(|(_, name)| name.clone()).collect();
    let (typed_edges, call_attempts) = resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge(&snapshots, &csharp_contexts, &signature_too_long_names_only, &import_facts)?;

    Ok(PipelineOutput {
        snapshots,
        lineage_edges,
        typed_edges,
        raw_import_statements,
        raw_impl_statements,
        raw_csharp_type_declaration_contexts,
        call_attempts,
        signature_too_long_names,
    })
}

/// Builds a fresh [`CodeGraph`] from exactly the given snapshots/edges —
/// used both for a full reindex (the WHOLE repository's current state)
/// and, by `indexer.rs`, to assemble the post-incremental-update graph
/// from the union of untouched-prior-state and freshly-recomputed
/// state. Fails closed via [`GraphError`] on any invariant violation
/// (duplicate id, dangling edge, etc.) — a caller assembling this from
/// already-validated upstream data should never see this fail in
/// practice, but this function does not special-case that assumption
/// away.
pub fn build_graph(snapshots: Vec<SymbolSnapshot>, lineage_edges: Vec<LineageEdge>, typed_edges: Vec<Edge>) -> Result<CodeGraph, GraphError> {
    let mut graph = CodeGraph::new();
    for snapshot in snapshots {
        graph.insert_snapshot(snapshot)?;
    }
    for edge in lineage_edges {
        graph.insert_lineage_edge(edge)?;
    }
    for edge in typed_edges {
        graph.insert_edge(edge)?;
    }
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use codegraph_domain::{CommitHash, FileIdentity, RepositoryId, RepositoryRegistry, RepositoryRoot};

    use super::*;

    struct FakeSource {
        files: HashMap<(RepositoryId, String), Vec<String>>,
        contents: HashMap<FileIdentity, Vec<u8>>,
    }

    impl FakeSource {
        fn new() -> Self {
            Self {
                files: HashMap::new(),
                contents: HashMap::new(),
            }
        }

        fn with_file(mut self, file: FileIdentity, bytes: &[u8]) -> Self {
            self.files.entry((file.repository(), file.commit().as_str().to_string())).or_default().push(file.relative_path().to_string());
            self.contents.insert(file, bytes.to_vec());
            self
        }
    }

    impl RepositorySource for FakeSource {
        fn list_files_at(&self, repository: RepositoryId, commit: &CommitHash) -> Result<Vec<String>, SourceError> {
            self.files.get(&(repository, commit.as_str().to_string())).cloned().ok_or(SourceError::UnknownCommit {
                repo: repository.as_uuid(),
                commit: commit.as_str().to_string(),
            })
        }

        fn read_file(&self, file: &FileIdentity) -> Result<Vec<u8>, SourceError> {
            self.contents.get(file).cloned().ok_or_else(|| SourceError::UnknownFile {
                repo: file.repository().as_uuid(),
                commit: file.commit().as_str().to_string(),
                file: file.relative_path().to_string(),
            })
        }
    }

    fn test_repo() -> RepositoryId {
        let mut registry = RepositoryRegistry::new();
        registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
    }

    fn no_known_lineage(_id: SymbolSnapshotId) -> Option<codegraph_domain::SymbolLineageId> {
        None
    }

    #[test]
    fn indexing_a_real_repository_source_produces_snapshots_lineage_and_edges() {
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let model_file = FileIdentity::parse(repo, commit.clone(), "src/model.py").unwrap();
        let source = FakeSource::new().with_file(
            model_file,
            b"class Module:\n    pass\n\nclass MyModel(Module):\n    def forward(self, x):\n        return x\n",
        );

        let output = index_files(&source, repo, &commit, &["src/model.py".to_string()], &[], &no_known_lineage).unwrap();

        assert_eq!(output.snapshots.len(), 3, "expected Module, MyModel, forward");
        // 2 typed edges, not 1: an Extends edge (MyModel extends
        // Module) PLUS a Contains edge (MyModel contains forward,
        // Task 9.5) — this is the whole point of resolve_all_typed_edges
        // now running every resolver, not a regression.
        assert_eq!(output.typed_edges.len(), 2, "expected one Extends edge and one Contains edge");
        assert_eq!(output.typed_edges.iter().filter(|e| e.kind == codegraph_domain::EdgeKind::Extends).count(), 1);
        assert_eq!(output.typed_edges.iter().filter(|e| e.kind == codegraph_domain::EdgeKind::Contains).count(), 1);
    }

    #[test]
    fn a_file_with_an_unsupported_extension_is_skipped_not_fatal() {
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let readme = FileIdentity::parse(repo, commit.clone(), "README.md").unwrap();
        let lib = FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap();
        let source = FakeSource::new().with_file(readme, b"# hello").with_file(lib, b"fn foo() {}");

        let output = index_files(
            &source,
            repo,
            &commit,
            &["README.md".to_string(), "src/lib.rs".to_string()],
            &[],
            &no_known_lineage,
        )
        .unwrap();

        assert_eq!(output.snapshots.len(), 1, "the unsupported README.md must be skipped, not abort the whole commit");
    }

    #[test]
    fn an_unknown_file_read_failure_fails_the_whole_pass_closed() {
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        // Never actually registered with the fake source -> read_file
        // will fail with SourceError::UnknownFile.
        let source = FakeSource::new();

        let result = index_files(&source, repo, &commit, &["src/missing.rs".to_string()], &[], &no_known_lineage);

        assert!(matches!(result, Err(PipelineError::Source(_))));
    }

    #[test]
    fn lineage_is_propagated_forward_across_two_real_pipeline_runs() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit_a.clone(), "src/lib.rs").unwrap();
        let file_b = FileIdentity::parse(repo, commit_b.clone(), "src/lib.rs").unwrap();
        let source = FakeSource::new().with_file(file_a, b"fn foo() {}\n").with_file(file_b, b"fn foo() {}\n");

        let first = index_files(&source, repo, &commit_a, &["src/lib.rs".to_string()], &[], &no_known_lineage).unwrap();
        assert_eq!(first.snapshots.len(), 1);

        let second = index_files(&source, repo, &commit_b, &["src/lib.rs".to_string()], &first.snapshots, &no_known_lineage).unwrap();

        assert_eq!(second.lineage_edges.len(), 1);
        assert!(second.lineage_edges[0].status.is_confirmed(), "identical symbol in the next commit must be Confirmed");
    }

    #[test]
    fn build_graph_assembles_a_consistent_graph_from_pipeline_output() {
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap();
        let source = FakeSource::new().with_file(file, b"fn foo() { bar(); }\nfn bar() {}\n");

        let output = index_files(&source, repo, &commit, &["src/lib.rs".to_string()], &[], &no_known_lineage).unwrap();
        let graph = build_graph(output.snapshots, output.lineage_edges, output.typed_edges).unwrap();

        assert_eq!(graph.snapshot_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }
}
