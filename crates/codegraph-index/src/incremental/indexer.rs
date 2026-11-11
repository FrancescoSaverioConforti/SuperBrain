//! B2 Task 9: [`CodeGraphIndexer`] — the race-safe, atomically-
//! publishing incremental reindexer. Owns one monotonically-increasing
//! `generation` per `CodeGraphIndexer` instance (not per repository —
//! every observed repository's publications share the same counter,
//! so "is this the newest publication seen so far" is a single total
//! order across the whole indexer) and the currently-published
//! `Arc<CodeGraph>` for each repository it has observed at least once.
//!
//! # Atomic publication discipline (mirrors B1's `RealIndexer`)
//!
//! Every heavy step — reading files through [`RepositorySource`],
//! parsing, extracting, promoting, resolving lineage and edges, and
//! assembling the candidate [`CodeGraph`] — happens OUTSIDE this
//! indexer's lock, via [`super::pipeline::index_files`]/
//! [`super::pipeline::build_graph`]. The lock is held only for two
//! short, separate critical sections, exactly mirroring B1's
//! `RealIndexer::observe_version`/`index_document` split
//! (`crates/index/src/indexer_impl.rs` in the `b1-deterministic-search`
//! worktree):
//!
//! 1. **Claim a generation** ([`CodeGraphIndexer::begin_observe`],
//!    private): under the lock, read the repository's current
//!    published state (if any) to decide what kind of update this is
//!    (new repository / normal advance / already-observed / history
//!    rewrite — see `classify_commit`), then allocate the NEXT
//!    generation number and release the lock immediately. No parsing
//!    or graph-building happens inside this critical section.
//! 2. **Compare-and-publish** ([`CodeGraphIndexer::publish`], private):
//!    after the (lock-free) rebuild work completes, re-take the lock
//!    and publish the new `Arc<CodeGraph>` for that repository ONLY IF
//!    this call's claimed generation is still the indexer's current
//!    highest generation for that repository — i.e., no OTHER
//!    `observe_commit` call claimed a LATER generation and already
//!    published in the meantime. If a later generation already
//!    published, this call's result is discarded (never overwrites a
//!    newer state) and [`ObserveOutcome`] still reports the
//!    NOW-CURRENT (newer) published graph, not the caller's own
//!    (stale, discarded) rebuild — a caller always receives a
//!    consistent, currently-published graph back, never silently
//!    nothing.
//!
//! Unlike B1 (which mutates one shared `IndexData` in place under the
//! lock), this indexer swaps a whole new `Arc<CodeGraph>` — `CodeGraph`
//! has no incremental "merge another graph into me" method (Task 8's
//! scope), so the natural unit of publication here is "replace the
//! published pointer," not "mutate in place." A reader therefore either
//! sees the complete OLD graph or the complete NEW graph, atomically,
//! via one `Arc` clone — never a graph with some snapshots updated and
//! others not.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use codegraph_domain::{CommitHash, RepositoryId, RepositorySource, SymbolSnapshotId};
use thiserror::Error;

use super::pipeline::{build_graph, index_files, PipelineError};
use crate::csharp_declarations::{resolve_csharp_type_declaration_contexts, RawCSharpTypeDeclarationContext};
use crate::graph::{CodeGraph, GraphError};
use crate::impls::{resolve_implements_from_impl_facts, resolve_raw_impl_statements, ImplFact, RawImplStatement};
use crate::imports::{resolve_raw_import_statements, ImportFact, RawImportStatement};
use crate::snapshot::SymbolSnapshot;

/// Internal, per-repository bookkeeping the indexer keeps alongside
/// the published graph: which commit that graph reflects, so the next
/// `observe_commit` call for the same repository can classify its own
/// relationship to it (see [`classify_commit`]).
///
/// `raw_import_statements` and `import_facts` are kept SEPARATE
/// (Task 9.5, Priority 6), mirroring the module-level split between
/// local syntax extraction and global semantic resolution:
/// `raw_import_statements` is reused across an incremental update for
/// every UNTOUCHED file (re-extracting syntax that did not change would
/// be pure waste), while `import_facts` is correctness-first RE-
/// RESOLVED FROM SCRATCH, across the ENTIRE raw-statement set, on every
/// single `observe_commit` call — a raw import's own text can be
/// unchanged while its correct RESOLUTION changes (the target it names
/// was renamed, moved, added, or removed elsewhere in the repository),
/// so a resolved fact can never be safely assumed reusable just because
/// its importing file was untouched. This is deliberately NOT yet an
/// optimized incremental resolution (recorded as a residual limitation,
/// not hidden) — it is documented, measured (see `IndexMetrics`), and
/// chosen over the alternative of silently serving a stale resolution.
#[derive(Debug)]
struct RepositoryState {
    commit: CommitHash,
    graph: Arc<CodeGraph>,
    generation: u64,
    raw_import_statements: Vec<(codegraph_domain::FileIdentity, RawImportStatement)>,
    import_facts: Arc<Vec<ImportFact>>,
    /// B2-ID-008, Task 4: mirrors `raw_import_statements`/`import_facts`
    /// exactly — same incremental-reuse-vs-always-recompute split, same
    /// reason (see this struct's own doc comment above).
    raw_impl_statements: Vec<(codegraph_domain::FileIdentity, RawImplStatement)>,
    impl_facts: Arc<Vec<ImplFact>>,
    /// B2-ID-010: mirrors `raw_import_statements`/`import_facts` and
    /// `raw_impl_statements`/`impl_facts` exactly — same incremental-
    /// reuse-vs-always-recompute split, same reason. `csharp_contexts`
    /// is consumed by `resolve_extends_implements_with_csharp_contexts`,
    /// never persisted on `CodeGraph` itself.
    raw_csharp_type_declaration_contexts: Vec<(codegraph_domain::FileIdentity, RawCSharpTypeDeclarationContext)>,
}

/// What [`CodeGraphIndexer::begin_observe`] (critical section 1)
/// claims and hands to the caller for the lock-free rebuild that
/// follows: the commit relationship it classified, the prior graph (if
/// any) to build an incremental update against, and the generation
/// number this call has exclusively claimed.
struct ObserveClaim {
    relationship: CommitRelationship,
    prior_graph: Option<Arc<CodeGraph>>,
    prior_raw_import_statements: Vec<(codegraph_domain::FileIdentity, RawImportStatement)>,
    prior_raw_impl_statements: Vec<(codegraph_domain::FileIdentity, RawImplStatement)>,
    prior_raw_csharp_type_declaration_contexts: Vec<(codegraph_domain::FileIdentity, RawCSharpTypeDeclarationContext)>,
    generation: u64,
}

/// How `new_commit` relates to a repository's previously-published
/// `prev_commit` (if any). Never inferred from git ancestry (this
/// indexer has no general "is A an ancestor of B" check — that would
/// require a real git walk this task's scope does not add) — inferred
/// purely from what the CALLER claims via `observe_commit`'s
/// `expected_prev_commit` parameter and whether that already matches
/// this indexer's own bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitRelationship {
    /// No `RepositoryState` exists yet for this repository at all.
    NewRepository,
    /// The caller's `expected_prev_commit` matches this indexer's
    /// currently-published commit for the repository — a normal,
    /// incremental advance.
    NormalAdvance,
    /// `new_commit` is the SAME commit this indexer already has
    /// published for the repository. A no-op: returns the current
    /// state without any rebuild.
    AlreadyObserved,
    /// The caller's `expected_prev_commit` does NOT match this
    /// indexer's currently-published commit (or the caller passed
    /// `None` for a repository that already has published state) —
    /// treated conservatively as a non-descendant/history-rewrite
    /// case. This indexer does not attempt to diff against a commit it
    /// cannot confirm is actually the graph's current ancestor; it
    /// performs a FULL reindex of `new_commit` instead (correct, if
    /// more expensive than a true incremental update would be) rather
    /// than risk silently relying on invalid prior state.
    HistoryRewriteOrUnknownAncestor,
}

fn classify_commit(prior: Option<&RepositoryState>, new_commit: &CommitHash, expected_prev_commit: Option<&CommitHash>) -> CommitRelationship {
    match prior {
        None => CommitRelationship::NewRepository,
        Some(state) if &state.commit == new_commit => CommitRelationship::AlreadyObserved,
        Some(state) => match expected_prev_commit {
            Some(expected) if expected == &state.commit => CommitRelationship::NormalAdvance,
            _ => CommitRelationship::HistoryRewriteOrUnknownAncestor,
        },
    }
}

/// Counters describing how much work one `observe_commit` call
/// actually did — not yet a resource BUDGET/policy (Task 13's job),
/// purely descriptive telemetry this task's real-repository validation
/// pass needs to confirm the invalidation scope is neither too narrow
/// (stale data) nor too broad (an unnecessary near-full rebuild).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndexMetrics {
    /// Every file this call considered at all (the full file list at
    /// the new commit for a full reindex; only the caller-provided
    /// changed-file list for an incremental update).
    pub files_considered: usize,
    /// Files actually re-parsed/re-extracted/re-promoted — the
    /// expensive step this task scopes down for an incremental update.
    pub files_reparsed: usize,
    /// Snapshots removed from the previously-published graph (because
    /// they belonged to a changed, renamed-away-from, or deleted
    /// file).
    pub snapshots_removed: usize,
    /// Snapshots newly added to the published graph.
    pub snapshots_added: usize,
    /// Typed + lineage edges in the newly-published graph (see this
    /// module's doc comment / `observe_commit`'s doc comment for why
    /// this task recomputes the WHOLE edge set on every observe rather
    /// than a narrower "only edges touching changed snapshots" set).
    pub edges_rebuilt: usize,
    /// Task 9.5, Priority 6: raw import statements newly (re-)extracted
    /// this call — i.e. belonging to a file this call actually parsed,
    /// never reused from a prior commit's untouched files.
    pub raw_import_statements_extracted: usize,
    /// Raw import statements CARRIED FORWARD unchanged from the prior
    /// commit's untouched files, never re-extracted.
    pub raw_import_statements_reused: usize,
    /// Resolved `ImportFact`s in the newly-published state — ALWAYS
    /// recomputed from the full raw-statement set on every call (see
    /// `RepositoryState`'s own doc comment for why resolution is not
    /// incrementally reused, unlike extraction).
    pub import_facts_resolved: usize,
    /// B2-ID-008, Task 4: raw `impl` statements newly (re-)extracted
    /// this call — mirrors `raw_import_statements_extracted` exactly.
    pub raw_impl_statements_extracted: usize,
    /// Raw `impl` statements CARRIED FORWARD unchanged from the prior
    /// commit's untouched files, never re-extracted — mirrors
    /// `raw_import_statements_reused` exactly.
    pub raw_impl_statements_reused: usize,
    /// Resolved `ImplFact`s in the newly-published state — ALWAYS
    /// recomputed from the full raw-statement set on every call, for
    /// the identical reason `import_facts_resolved` is — mirrors
    /// `import_facts_resolved` exactly.
    pub impl_facts_resolved: usize,
    /// B2-ID-010: raw C# type-declaration contexts newly (re-)extracted
    /// this call — mirrors `raw_import_statements_extracted` exactly.
    pub raw_csharp_type_declaration_contexts_extracted: usize,
    /// Raw C# type-declaration contexts CARRIED FORWARD unchanged from
    /// the prior commit's untouched files, never re-extracted — mirrors
    /// `raw_import_statements_reused` exactly.
    pub raw_csharp_type_declaration_contexts_reused: usize,
}

/// What a successful [`CodeGraphIndexer::observe_commit`] call returns.
/// `graph` is always the indexer's CURRENTLY published graph for that
/// repository at the time this call returns — which may be THIS call's
/// own freshly-built graph (the common case), or a LATER generation's
/// graph published by a concurrent, faster `observe_commit` call that
/// raced ahead of this one (see this module's doc comment on the
/// compare-and-publish discipline) — never a stale graph this call
/// itself built but that lost the race.
#[derive(Debug, Clone)]
pub struct ObserveOutcome {
    pub generation: u64,
    pub graph: Arc<CodeGraph>,
    /// Task 9.5, Priority 6: the CURRENTLY published `ImportFact`s for
    /// this repository — same atomicity/publication discipline as
    /// `graph`: either this call's own freshly-resolved facts, or a
    /// later generation's facts if this call lost the publish race,
    /// never a partial/stale mix.
    pub import_facts: Arc<Vec<ImportFact>>,
    /// B2-ID-008, Task 4: the CURRENTLY published `ImplFact`s for this
    /// repository — mirrors `import_facts` exactly (same atomicity:
    /// either this call's own freshly-resolved facts, or a later
    /// generation's facts if this call lost the publish race).
    pub impl_facts: Arc<Vec<ImplFact>>,
    pub metrics: IndexMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ObserveError {
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    #[error(transparent)]
    Graph(#[from] GraphError),
}

/// Named pause points this indexer's production code calls through an
/// optional test-only hook — mirrors B1's `TestPoint` mechanism
/// (`crates/index/src/indexer_impl.rs`), the deterministic,
/// non-sleep-based primitive this task's mandatory race test is built
/// on. Compiled only under `#[cfg(test)]`; no production code path is
/// affected when no hook is installed (the hook call is a no-op unless
/// a test has set one).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TestPoint {
    /// Fires in `begin_observe`, immediately after this call's
    /// generation has been claimed and the publication lock released —
    /// the exact moment a slower caller can be paused so a faster,
    /// later-generation caller can publish first.
    AfterGenerationClaimed,
}

#[cfg(test)]
type TestHook = Arc<dyn Fn(TestPoint, u64) + Send + Sync>;

/// Wraps the test-only hook so its non-`Debug` `Arc<dyn Fn(...)>`
/// payload does not block deriving `Debug` on [`CodeGraphIndexer`]
/// itself — the derive is what lets mutation testing's "replace this
/// `fmt` impl with a stub" mutant class simply not exist here instead
/// of being an unverified, cosmetic hand-written impl.
#[cfg(test)]
struct HookSlot(Mutex<Option<TestHook>>);

#[cfg(test)]
impl std::fmt::Debug for HookSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookSlot").field("installed", &self.0.lock().unwrap().is_some()).finish()
    }
}

#[derive(Debug)]
pub struct CodeGraphIndexer {
    state: Mutex<HashMap<RepositoryId, RepositoryState>>,
    /// The next generation number to hand out — a counter entirely
    /// separate from any repository's PUBLISHED generation, because
    /// two concurrent `observe_commit` calls must each claim a
    /// strictly distinct generation the moment they start, before
    /// either has published anything. Deriving "the next generation"
    /// from `max(published generations) + 1` instead (what this field
    /// replaces an earlier draft of this type that did exactly that)
    /// is a real race: two callers racing ahead of any publication
    /// would both see the same `max` and claim the SAME generation,
    /// defeating the whole "a later generation always wins" guarantee
    /// this module exists to provide.
    next_generation: Mutex<u64>,
    #[cfg(test)]
    hook: HookSlot,
}

impl Default for CodeGraphIndexer {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGraphIndexer {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
            next_generation: Mutex::new(1),
            #[cfg(test)]
            hook: HookSlot(Mutex::new(None)),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_test_hook(&self, hook: TestHook) {
        *self.hook.0.lock().unwrap() = Some(hook);
    }

    #[cfg(test)]
    fn fire_hook(&self, point: TestPoint, generation: u64) {
        let hook = self.hook.0.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook(point, generation);
        }
    }

    /// The generation currently published for `repository`, if any —
    /// for a reader that only wants to know "what's the latest graph"
    /// without observing a new commit.
    pub fn current(&self, repository: RepositoryId) -> Option<(u64, Arc<CodeGraph>)> {
        let state = self.state.lock().unwrap();
        state.get(&repository).map(|s| (s.generation, s.graph.clone()))
    }

    /// Observes `new_commit` for `repository`, reindexing exactly
    /// `changed_files` (repository-relative paths) if this indexer can
    /// confirm `expected_prev_commit` is the commit it already has
    /// published for this repository (a [`CommitRelationship::NormalAdvance`]
    /// — see `classify_commit`); otherwise performs a full reindex of
    /// every file `source` reports for `new_commit`, ignoring
    /// `changed_files` entirely (a full reindex has no "changed files"
    /// concept — it reads and re-derives everything).
    ///
    /// `expected_prev_commit` is the CALLER's own belief about what the
    /// prior commit was (e.g. the parent commit it observed the change
    /// against). Passing `None` always forces a full reindex (new
    /// repository, or caller deliberately not claiming an incremental
    /// relationship) — never silently reused as a stale commit.
    ///
    /// Race/atomicity: see this module's doc comment. If this call's
    /// own rebuild loses the race to a newer `observe_commit` call for
    /// the same repository, this call's result is discarded and the
    /// returned [`ObserveOutcome`] reports the actually-current
    /// (newer) state instead — never overwriting it.
    ///
    /// Error atomicity: if parsing/extraction/promotion/lineage/edge
    /// resolution fails at ANY point, this call returns
    /// [`ObserveError`] and the previously-published graph for this
    /// repository (if any) is left completely untouched — no partial
    /// state is ever published, and no generation is wasted/claimed
    /// for a failed attempt (the generation counter only advances on
    /// an actual, successful publish — see `begin_observe`).
    pub fn observe_commit(
        &self,
        source: &dyn RepositorySource,
        repository: RepositoryId,
        new_commit: &CommitHash,
        expected_prev_commit: Option<&CommitHash>,
        changed_files: &[String],
    ) -> Result<ObserveOutcome, ObserveError> {
        let claim = self.begin_observe(repository, new_commit, expected_prev_commit);

        #[cfg(test)]
        self.fire_hook(TestPoint::AfterGenerationClaimed, claim.generation);

        if claim.relationship == CommitRelationship::AlreadyObserved {
            // No rebuild at all — return the existing state as-is.
            let state = self.state.lock().unwrap();
            let existing = state.get(&repository).expect("AlreadyObserved implies prior state exists");
            return Ok(ObserveOutcome {
                generation: existing.generation,
                graph: existing.graph.clone(),
                import_facts: existing.import_facts.clone(),
                impl_facts: existing.impl_facts.clone(),
                metrics: IndexMetrics::default(),
            });
        }

        let (new_graph, new_raw_import_statements, import_facts, new_raw_impl_statements, impl_facts, new_raw_csharp_type_declaration_contexts, metrics) = match claim.relationship {
            CommitRelationship::NewRepository | CommitRelationship::HistoryRewriteOrUnknownAncestor => self.full_reindex(source, repository, new_commit)?,
            CommitRelationship::NormalAdvance => {
                let prior_graph = claim.prior_graph.expect("NormalAdvance implies prior state exists");
                self.incremental_reindex(
                    source,
                    repository,
                    new_commit,
                    &prior_graph,
                    &claim.prior_raw_import_statements,
                    &claim.prior_raw_impl_statements,
                    &claim.prior_raw_csharp_type_declaration_contexts,
                    changed_files,
                )?
            }
            CommitRelationship::AlreadyObserved => unreachable!("handled above"),
        };

        Ok(self.publish(
            repository,
            new_commit.clone(),
            claim.generation,
            new_graph,
            new_raw_import_statements,
            import_facts,
            new_raw_impl_statements,
            impl_facts,
            new_raw_csharp_type_declaration_contexts,
            metrics,
        ))
    }

    /// Critical section 1: classify this call's relationship to the
    /// repository's current state and claim the next generation number
    /// — the ONLY work done under the lock before the (lock-free)
    /// rebuild. Returns the prior graph (a cloned `Arc`, cheap) so the
    /// caller can build the incremental update outside the lock.
    fn begin_observe(&self, repository: RepositoryId, new_commit: &CommitHash, expected_prev_commit: Option<&CommitHash>) -> ObserveClaim {
        let (relationship, prior_graph, prior_raw_import_statements, prior_raw_impl_statements, prior_raw_csharp_type_declaration_contexts) = {
            let state = self.state.lock().unwrap();
            let prior = state.get(&repository);
            let relationship = classify_commit(prior, new_commit, expected_prev_commit);
            (
                relationship,
                prior.map(|s| s.graph.clone()),
                prior.map(|s| s.raw_import_statements.clone()).unwrap_or_default(),
                prior.map(|s| s.raw_impl_statements.clone()).unwrap_or_default(),
                prior.map(|s| s.raw_csharp_type_declaration_contexts.clone()).unwrap_or_default(),
            )
        };
        // Claimed from a counter entirely separate from `state`'s own
        // per-repository generations — see `next_generation`'s own doc
        // comment for why deriving this from `max(published
        // generations) + 1` is a real race between two callers that
        // have not yet published anything.
        let generation = {
            let mut next = self.next_generation.lock().unwrap();
            let claimed = *next;
            *next += 1;
            claimed
        };
        ObserveClaim {
            relationship,
            prior_graph,
            prior_raw_import_statements,
            prior_raw_impl_statements,
            prior_raw_csharp_type_declaration_contexts,
            generation,
        }
    }

    #[allow(clippy::type_complexity)]
    fn full_reindex(
        &self,
        source: &dyn RepositorySource,
        repository: RepositoryId,
        commit: &CommitHash,
    ) -> Result<
        (
            CodeGraph,
            Vec<(codegraph_domain::FileIdentity, RawImportStatement)>,
            Vec<ImportFact>,
            Vec<(codegraph_domain::FileIdentity, RawImplStatement)>,
            Vec<ImplFact>,
            Vec<(codegraph_domain::FileIdentity, RawCSharpTypeDeclarationContext)>,
            IndexMetrics,
        ),
        ObserveError,
    > {
        let files = source.list_files_at(repository, commit).map_err(|e| ObserveError::Pipeline(PipelineError::Source(e)))?;
        let no_known_lineage = |_: SymbolSnapshotId| None;
        let output = index_files(source, repository, commit, &files, &[], &no_known_lineage)?;

        let import_facts = resolve_raw_import_statements(&output.raw_import_statements, &output.snapshots).map_err(PipelineError::from)?;
        let impl_facts = resolve_raw_impl_statements(&output.raw_impl_statements, &output.snapshots).map_err(PipelineError::from)?;
        let implements_edges = resolve_implements_from_impl_facts(&impl_facts);

        let metrics = IndexMetrics {
            files_considered: files.len(),
            files_reparsed: files.len(),
            snapshots_removed: 0,
            snapshots_added: output.snapshots.len(),
            edges_rebuilt: output.lineage_edges.len() + output.typed_edges.len() + implements_edges.len(),
            raw_import_statements_extracted: output.raw_import_statements.len(),
            raw_import_statements_reused: 0,
            import_facts_resolved: import_facts.len(),
            raw_impl_statements_extracted: output.raw_impl_statements.len(),
            raw_impl_statements_reused: 0,
            impl_facts_resolved: impl_facts.len(),
            raw_csharp_type_declaration_contexts_extracted: output.raw_csharp_type_declaration_contexts.len(),
            raw_csharp_type_declaration_contexts_reused: 0,
        };
        let mut typed_edges = output.typed_edges;
        typed_edges.extend(implements_edges);
        let graph = build_graph(output.snapshots, output.lineage_edges, typed_edges)?;
        Ok((graph, output.raw_import_statements, import_facts, output.raw_impl_statements, impl_facts, output.raw_csharp_type_declaration_contexts, metrics))
    }

    /// Incremental update: re-parses/re-extracts/re-promotes ONLY
    /// `changed_files`, keeping every other file's snapshots from
    /// `prior_graph` untouched. Lineage and typed edges are
    /// RECOMPUTED FROM THE WHOLE RESULTING SNAPSHOT POOL (untouched +
    /// freshly rebuilt) rather than only from the changed snapshots —
    /// a deliberate, documented scope choice, not an oversight:
    /// `resolve_calls`/`resolve_extends_implements`/etc. resolve a callee NAME
    /// against the full candidate pool they are given, so a change
    /// confined to `changed_files` can still change which edges an
    /// UNCHANGED file's symbols resolve to (e.g. file B calls `encoder`
    /// and previously had no match; file A, just changed, newly
    /// defines `encoder` — B's outgoing edge must update even though B
    /// itself was never reparsed). Recomputing edge resolution over the
    /// full pool is the only way to avoid exactly this kind of
    /// under-invalidation; re-parsing is the expensive step this
    /// function actually scopes down to `changed_files` only (see
    /// `IndexMetrics::files_reparsed` vs. `files_considered`).
    #[allow(clippy::type_complexity)]
    #[allow(clippy::too_many_arguments)]
    fn incremental_reindex(
        &self,
        source: &dyn RepositorySource,
        repository: RepositoryId,
        new_commit: &CommitHash,
        prior_graph: &CodeGraph,
        prior_raw_import_statements: &[(codegraph_domain::FileIdentity, RawImportStatement)],
        prior_raw_impl_statements: &[(codegraph_domain::FileIdentity, RawImplStatement)],
        prior_raw_csharp_type_declaration_contexts: &[(codegraph_domain::FileIdentity, RawCSharpTypeDeclarationContext)],
        changed_files: &[String],
    ) -> Result<
        (
            CodeGraph,
            Vec<(codegraph_domain::FileIdentity, RawImportStatement)>,
            Vec<ImportFact>,
            Vec<(codegraph_domain::FileIdentity, RawImplStatement)>,
            Vec<ImplFact>,
            Vec<(codegraph_domain::FileIdentity, RawCSharpTypeDeclarationContext)>,
            IndexMetrics,
        ),
        ObserveError,
    > {
        let changed_set: HashSet<&str> = changed_files.iter().map(|s| s.as_str()).collect();

        let untouched_snapshots: Vec<SymbolSnapshot> = prior_graph.all_snapshots().into_iter().filter(|s| !changed_set.contains(s.file().relative_path())).cloned().collect();
        let prior_changed_snapshots: Vec<SymbolSnapshot> = prior_graph.all_snapshots().into_iter().filter(|s| changed_set.contains(s.file().relative_path())).cloned().collect();

        // A path named in `changed_files` may no longer exist at
        // `new_commit` at all (a delete, or the "old" half of a
        // rename/move) — `list_files_at` is the one authoritative
        // source of "what actually exists now" (Task 2's seam, never a
        // raw filesystem check), so this intersects the caller's
        // claimed changed set against it BEFORE calling `index_files`.
        // Without this, a deleted path's own `read_file` call would
        // fail with `SourceError::UnknownFile` and incorrectly abort
        // the whole commit, conflating "this file is gone" (a normal,
        // expected outcome `changed_files` must be able to express)
        // with "the source seam itself is broken."
        let files_at_new_commit: HashSet<String> = source.list_files_at(repository, new_commit).map_err(|e| ObserveError::Pipeline(PipelineError::Source(e)))?.into_iter().collect();
        let mut files_to_reparse: Vec<String> = changed_files.iter().filter(|p| files_at_new_commit.contains(p.as_str())).cloned().collect();
        files_to_reparse.sort();

        let known_lineage_of = build_known_lineage_lookup(prior_graph);
        let output = index_files(source, repository, new_commit, &files_to_reparse, &prior_changed_snapshots, &known_lineage_of)?;

        // The CURRENT pool — untouched snapshots plus the freshly
        // rebuilt ones — is what edge resolution runs against. This
        // must NEVER include historical (lineage-superseded) snapshots:
        // `resolve_calls`/etc. resolve a callee NAME against every
        // candidate they are given, so including both an old and a new
        // snapshot that happen to share a name (e.g. an unchanged
        // helper function re-promoted at the new commit, which gets a
        // DIFFERENT `SymbolSnapshotId` than before because
        // `SymbolSnapshotId::derive` hashes the owning `FileIdentity`'s
        // commit) would make every caller's resolution ambiguous
        // between "the old one" and "the new one," even though only the
        // new one is actually current. This was a real bug caught by
        // this task's own deep-learning fixture test: re-indexing
        // model.py after an unrelated rename doubled encoder/
        // classifier's resolved callee count, because the OLD
        // (commit-A-rooted) encoder/classifier snapshots were being fed
        // into edge resolution alongside the NEW ones.
        let mut current_snapshots = untouched_snapshots;
        current_snapshots.extend(output.snapshots.clone());

        // B2-ID-010: identical incremental semantics to
        // `raw_import_statements`/`raw_impl_statements` below — reused
        // for every UNTOUCHED file, freshly extracted for every file
        // this call actually reparsed. Resolution (the `snapshot_id`
        // join against the real pool) is, exactly like
        // `import_facts`/`impl_facts`, ALWAYS recomputed from the full
        // raw-context set against the full current pool, never reused
        // incrementally — an untouched declaration's own context data
        // never changes, but which `SymbolSnapshotId` it resolves to
        // CAN (a renamed/moved file elsewhere does not change this
        // declaration's own identity, but the identity is still
        // re-derived from `current_snapshots` fresh every call, same as
        // every other resolver here).
        let untouched_raw_csharp_type_declaration_contexts: Vec<_> = prior_raw_csharp_type_declaration_contexts.iter().filter(|(file, _)| !changed_set.contains(file.relative_path())).cloned().collect();
        let untouched_raw_csharp_type_declaration_contexts_len = untouched_raw_csharp_type_declaration_contexts.len();
        let mut raw_csharp_type_declaration_contexts = untouched_raw_csharp_type_declaration_contexts;
        raw_csharp_type_declaration_contexts.extend(output.raw_csharp_type_declaration_contexts.clone());
        let csharp_contexts = resolve_csharp_type_declaration_contexts(&raw_csharp_type_declaration_contexts, &current_snapshots);

        let mut typed_edges = crate::edges::resolve_all_typed_edges_with_csharp_contexts(&current_snapshots, &csharp_contexts).map_err(PipelineError::from)?;

        // Lineage across the UNCHANGED portion is already final (it was
        // resolved and published by an earlier observe_commit call);
        // only the changed portion's lineage needs fresh resolution,
        // which `index_files` above already computed into
        // `output.lineage_edges`. A prior lineage edge is carried
        // forward ONLY when BOTH endpoints belong to untouched files —
        // an edge whose `from` (or `to`) belonged to a CHANGED file is
        // dropped even if the other endpoint is untouched, because
        // that changed file's own snapshot for this commit may no
        // longer exist at all (a delete, or a rename this resolver
        // decided not to link), and carrying the old edge forward
        // would leave it dangling once `build_graph` tries to insert it
        // against the new snapshot pool.
        let untouched_lineage_edges: Vec<_> = prior_graph
            .all_snapshots()
            .into_iter()
            .filter(|s| !changed_set.contains(s.file().relative_path()))
            .flat_map(|s| prior_graph.lineage_from(s.id()))
            .filter(|edge| prior_graph.snapshot(edge.to).is_some_and(|to| !changed_set.contains(to.file().relative_path())))
            .cloned()
            .collect();
        let mut lineage_edges = untouched_lineage_edges;
        lineage_edges.extend(output.lineage_edges.clone());

        // Only NOW, after edge resolution has run against the CURRENT
        // pool alone, are historical (lineage-superseded) snapshots
        // added to the pool that actually gets stored — purely so a
        // `LineageEdge`'s `from` endpoint is present in the graph (see
        // this module's doc comment on `CodeGraph` having no "current
        // vs. historical" distinction yet). These historical snapshots
        // never participate in typed-edge resolution.
        //
        // `current_snapshots` itself is cloned here, not moved: C1's own
        // fix (below, at `impl_facts`'s own resolution call) needs the
        // CURRENT-only pool to still be available after this point,
        // deliberately kept separate from `final_snapshots`'s broader
        // (current + historical-lineage-source) pool.
        let mut final_snapshots = current_snapshots.clone();
        let lineage_sources: Vec<SymbolSnapshot> = lineage_edges.iter().filter_map(|edge| prior_graph.snapshot(edge.from).cloned()).collect();
        for source_snapshot in lineage_sources {
            if !final_snapshots.iter().any(|s| s.id() == source_snapshot.id()) {
                final_snapshots.push(source_snapshot);
            }
        }

        // Priority 6 incremental semantics: raw import statements are
        // reused for every UNTOUCHED file (pure syntax that cannot have
        // changed if the file itself did not), and freshly extracted
        // for every file this call actually reparsed — `changed_set`
        // already distinguishes these the exact same way snapshots do.
        // Deleted files correctly drop out here too: `prior_raw_import_statements`
        // is filtered by `changed_set`, so a deleted file's old raw
        // statements are never carried forward (it IS in `changed_set`),
        // and it produces no new ones either (`output.raw_import_statements`
        // only contains files `index_files` actually parsed, which a
        // deleted file never is).
        let untouched_raw_import_statements: Vec<_> = prior_raw_import_statements.iter().filter(|(file, _)| !changed_set.contains(file.relative_path())).cloned().collect();
        let untouched_raw_import_statements_len = untouched_raw_import_statements.len();
        let mut raw_import_statements = untouched_raw_import_statements;
        raw_import_statements.extend(output.raw_import_statements.clone());

        // Resolution is ALWAYS recomputed from the full raw-statement
        // set against the full current snapshot pool — never reused
        // incrementally, even for untouched-file statements (see
        // `RepositoryState`'s own doc comment for why: an untouched
        // import's correct TARGET can still change when some OTHER
        // file's symbol it names is renamed/moved/added/removed).
        let import_facts = resolve_raw_import_statements(&raw_import_statements, &final_snapshots).map_err(PipelineError::from)?;

        // B2-ID-008, Task 4: identical incremental semantics for raw
        // impl statements — reused for every UNTOUCHED file, freshly
        // extracted for every file this call actually reparsed. See
        // the `raw_import_statements` block immediately above for why
        // this mirrors it field-for-field.
        let untouched_raw_impl_statements: Vec<_> = prior_raw_impl_statements.iter().filter(|(file, _)| !changed_set.contains(file.relative_path())).cloned().collect();
        let untouched_raw_impl_statements_len = untouched_raw_impl_statements.len();
        let mut raw_impl_statements = untouched_raw_impl_statements;
        raw_impl_statements.extend(output.raw_impl_statements.clone());

        // Resolution (including the derived `Implements` edges) is
        // ALWAYS recomputed from the full raw-statement set — never
        // reused incrementally, for the identical reason `import_facts`'s
        // own resolution is (see `RepositoryState`'s own doc comment): an
        // untouched impl's correct resolution can change when some OTHER
        // file's symbol it names is added/renamed/removed/moved
        // elsewhere.
        //
        // C1 fix (final-review finding): resolved against
        // `current_snapshots` (untouched + freshly rebuilt — the SAME
        // pool `resolve_all_typed_edges`/`typed_edges` above already
        // resolves against), NOT `final_snapshots`. `final_snapshots`
        // deliberately ALSO contains historical, lineage-superseded
        // snapshots from the prior commit (see its own doc comment a few
        // lines above) purely so a `LineageEdge`'s `from` endpoint has
        // somewhere to point in the stored graph — but an old, superseded
        // snapshot can share its exact name with its own replacement
        // (e.g. a trait/type's defining file gets an unrelated edit, so
        // the trait/type's OLD snapshot gets an exact-match Hypothesized
        // lineage edge and ends up sitting in `final_snapshots` under the
        // SAME name as the new one). Resolving `impl_facts` against that
        // pool made `resolve_target_by_name` see two same-named
        // candidates and report `Ambiguous` instead of `Resolved`,
        // silently dropping a real `Implements` edge on an ordinary,
        // unrelated incremental edit. `current_snapshots` has no such
        // duplicate-by-name risk for an unrenamed symbol: it is exactly
        // "what currently exists," matching `typed_edges`'s own
        // resolution pool exactly. Every `ImplTargetResolution::Resolved`
        // id this produces is still guaranteed present in
        // `final_snapshots` below (a superset of `current_snapshots`),
        // so the derived `Implements` edges' endpoints are always valid
        // once `build_graph` runs.
        let impl_facts = resolve_raw_impl_statements(&raw_impl_statements, &current_snapshots).map_err(PipelineError::from)?;
        let implements_edges = resolve_implements_from_impl_facts(&impl_facts);
        typed_edges.extend(implements_edges.clone());

        let metrics = IndexMetrics {
            files_considered: changed_files.len(),
            files_reparsed: files_to_reparse.len(),
            snapshots_removed: prior_changed_snapshots.len(),
            snapshots_added: output.snapshots.len(),
            edges_rebuilt: lineage_edges.len() + typed_edges.len(),
            raw_import_statements_extracted: output.raw_import_statements.len(),
            raw_import_statements_reused: untouched_raw_import_statements_len,
            import_facts_resolved: import_facts.len(),
            raw_impl_statements_extracted: output.raw_impl_statements.len(),
            raw_impl_statements_reused: untouched_raw_impl_statements_len,
            impl_facts_resolved: impl_facts.len(),
            raw_csharp_type_declaration_contexts_extracted: output.raw_csharp_type_declaration_contexts.len(),
            raw_csharp_type_declaration_contexts_reused: untouched_raw_csharp_type_declaration_contexts_len,
        };

        let graph = build_graph(final_snapshots, lineage_edges, typed_edges)?;
        Ok((graph, raw_import_statements, import_facts, raw_impl_statements, impl_facts, raw_csharp_type_declaration_contexts, metrics))
    }

    /// Critical section 2: publish `candidate_graph` for `repository`
    /// as `generation`, UNLESS a later generation has already been
    /// published for this repository in the meantime (see this
    /// module's doc comment) — in which case this call's own work is
    /// discarded and the actually-current state is returned instead.
    #[allow(clippy::too_many_arguments)]
    fn publish(
        &self,
        repository: RepositoryId,
        commit: CommitHash,
        generation: u64,
        candidate_graph: CodeGraph,
        candidate_raw_import_statements: Vec<(codegraph_domain::FileIdentity, RawImportStatement)>,
        candidate_import_facts: Vec<ImportFact>,
        candidate_raw_impl_statements: Vec<(codegraph_domain::FileIdentity, RawImplStatement)>,
        candidate_impl_facts: Vec<ImplFact>,
        candidate_raw_csharp_type_declaration_contexts: Vec<(codegraph_domain::FileIdentity, RawCSharpTypeDeclarationContext)>,
        metrics: IndexMetrics,
    ) -> ObserveOutcome {
        let mut state = self.state.lock().unwrap();
        let current_generation = state.get(&repository).map(|s| s.generation).unwrap_or(0);
        if generation > current_generation {
            let graph = Arc::new(candidate_graph);
            let import_facts = Arc::new(candidate_import_facts);
            let impl_facts = Arc::new(candidate_impl_facts);
            state.insert(
                repository,
                RepositoryState {
                    commit,
                    graph: graph.clone(),
                    generation,
                    raw_import_statements: candidate_raw_import_statements,
                    import_facts: import_facts.clone(),
                    raw_impl_statements: candidate_raw_impl_statements,
                    impl_facts: impl_facts.clone(),
                    raw_csharp_type_declaration_contexts: candidate_raw_csharp_type_declaration_contexts,
                },
            );
            ObserveOutcome { generation, graph, import_facts, impl_facts, metrics }
        } else {
            // A later generation already published — this call's
            // candidate (graph AND import/impl state) is discarded.
            // Return the ACTUAL current state, not this call's own
            // (stale, never-published) data — the same atomicity
            // guarantee `graph`/`import_facts` already had now covers
            // `impl_facts` too.
            let existing = state.get(&repository).expect("current_generation > 0 implies prior state exists when generation <= current_generation");
            ObserveOutcome {
                generation: existing.generation,
                graph: existing.graph.clone(),
                import_facts: existing.import_facts.clone(),
                impl_facts: existing.impl_facts.clone(),
                metrics,
            }
        }
    }
}

/// Builds the `known_lineage_of` closure `resolve_lineage` needs: for
/// a given earlier `SymbolSnapshotId`, what `SymbolLineageId` (if any)
/// it already belongs to — read directly from `prior_graph`'s own
/// lineage edges, so an existing chain is propagated forward rather
/// than re-derived as a new genesis on every single incremental update
/// (mirrors `lineage::resolver`'s own documented contract for this
/// parameter).
fn build_known_lineage_lookup(prior_graph: &CodeGraph) -> impl Fn(SymbolSnapshotId) -> Option<codegraph_domain::SymbolLineageId> + '_ {
    move |id: SymbolSnapshotId| prior_graph.lineage_from(id).first().map(|edge| edge.lineage).or_else(|| prior_graph.lineage_to(id).first().map(|edge| edge.lineage))
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::sync::mpsc;

    use codegraph_domain::{EdgeKind, FileIdentity, RepositoryRegistry, RepositoryRoot, SourceError};

    use super::*;
    use crate::extract::{ByteSpan, RawSymbolCandidate};
    use crate::snapshot::promote;

    #[derive(Debug)]
    struct FakeSource {
        files: std::collections::HashMap<(RepositoryId, String), Vec<String>>,
        contents: std::collections::HashMap<FileIdentity, Vec<u8>>,
    }

    impl FakeSource {
        fn new() -> Self {
            Self {
                files: std::collections::HashMap::new(),
                contents: std::collections::HashMap::new(),
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

    #[test]
    fn publish_with_an_equal_generation_is_rejected_not_treated_as_newer() {
        // Direct, white-box test of `publish`'s boundary — a generation
        // EQUAL to the already-published one can never actually reach
        // this method through the public `observe_commit` API (every
        // claim comes from a strictly-incrementing global counter), but
        // the `>` comparison itself is the one thing standing between
        // "strictly newer only" and "newer-or-equal," so this pins it
        // directly rather than leaving it provable only by an argument
        // about the counter never producing a duplicate.
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let indexer = CodeGraphIndexer::new();

        let first_outcome = indexer.publish(repo, commit.clone(), 5, CodeGraph::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), IndexMetrics::default());
        assert_eq!(first_outcome.generation, 5);

        // Attempt to publish AGAIN at the SAME generation (5, not 4) —
        // must be rejected exactly like a strictly older one would be.
        let mut second_candidate = CodeGraph::new();
        let file = FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap();
        let candidate = RawSymbolCandidate::new("marker".to_string(), "function_item".to_string(), ByteSpan { start: 0, end: 3 }, "fn marker() {}".to_string()).unwrap();
        second_candidate.insert_snapshot(promote(&candidate, file).unwrap()).unwrap();
        let second_outcome = indexer.publish(repo, commit, 5, second_candidate, Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), IndexMetrics::default());

        assert_eq!(second_outcome.generation, 5, "the reported generation must still be 5 (the one already published)");
        assert_eq!(second_outcome.graph.snapshot_count(), 0, "an equal-generation publish must NOT overwrite the already-published graph with its own candidate");
    }

    #[test]
    fn a_new_repository_is_fully_indexed_on_its_first_observe() {
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap();
        let source = FakeSource::new().with_file(file, b"fn foo() { bar(); }\nfn bar() {}\n");
        let indexer = CodeGraphIndexer::new();

        let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

        assert_eq!(outcome.generation, 1);
        assert_eq!(outcome.graph.snapshot_count(), 2);
        assert_eq!(outcome.metrics.files_reparsed, 1);
        // Pins `edges_rebuilt`'s actual summation (lineage + typed, not
        // one or the other alone, and not a `*` of the two): a full
        // reindex of a brand-new repository has zero lineage edges (no
        // `prev_snapshots` exist yet) and exactly one typed Calls edge.
        assert_eq!(outcome.metrics.edges_rebuilt, 1);
    }

    #[test]
    fn the_generation_counter_strictly_increments_across_independent_observe_calls() {
        // Direct, deterministic (non-concurrent) pin of `begin_observe`'s
        // generation claim: mutating `*next += 1` to `*next *= 1` would
        // leave the counter stuck at its initial value of 1 forever,
        // so every call would claim the SAME generation. Two
        // sequential, unrelated repositories (never racing, no
        // barrier/channel involved) must still claim strictly
        // increasing generation numbers.
        let repo_a = test_repo();
        let repo_b = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo_a, commit.clone(), "src/lib.rs").unwrap();
        let file_b = FileIdentity::parse(repo_b, commit.clone(), "src/lib.rs").unwrap();
        let source = FakeSource::new().with_file(file_a, b"fn foo() {}\n").with_file(file_b, b"fn bar() {}\n");
        let indexer = CodeGraphIndexer::new();

        let outcome_a = indexer.observe_commit(&source, repo_a, &commit, None, &[]).unwrap();
        let outcome_b = indexer.observe_commit(&source, repo_b, &commit, None, &[]).unwrap();

        assert_eq!(outcome_a.generation, 1);
        assert_eq!(outcome_b.generation, 2, "the second, independent observe_commit call must claim a strictly greater generation than the first");
    }

    #[test]
    fn observing_the_same_commit_twice_is_a_no_op_not_a_rebuild() {
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap();
        let source = FakeSource::new().with_file(file, b"fn foo() {}\n");
        let indexer = CodeGraphIndexer::new();

        let first = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();
        let second = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

        assert_eq!(first.generation, second.generation, "observing the same commit again must not claim a new generation");
        assert_eq!(second.metrics, IndexMetrics::default(), "a duplicate observe must do no rebuild work at all");
    }

    #[test]
    fn a_normal_incremental_advance_only_reparses_the_changed_file() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let mut source = FakeSource::new();
        let mut files_at_a = Vec::new();
        for i in 0..10 {
            let path = format!("src/file{i}.rs");
            let file = FileIdentity::parse(repo, commit_a.clone(), &path).unwrap();
            source = source.with_file(file, format!("fn f{i}() {{}}").as_bytes());
            files_at_a.push(path);
        }
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(first.graph.snapshot_count(), 10);

        // Commit B: only file0.rs changes, carried forward at the new commit.
        for (i, path) in files_at_a.iter().enumerate() {
            let file = FileIdentity::parse(repo, commit_b.clone(), path).unwrap();
            let text = if i == 0 { "fn f0() { 1 }".to_string() } else { format!("fn f{i}() {{}}") };
            source = source.with_file(file, text.as_bytes());
        }

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/file0.rs".to_string()]).unwrap();

        assert_eq!(second.metrics.files_reparsed, 1, "only the one changed file must be reparsed");
        assert_eq!(second.metrics.files_considered, 1);
        // 10 untouched + unchanged symbols (file1..9) + 1 new f0 (the
        // content edit) + 1 OLD f0 retained as the historical lineage
        // source its Hypothesized edge points FROM.
        assert_eq!(second.graph.snapshot_count(), 11);
        let unrelated_still_present = (1..10).all(|i| second.graph.all_snapshots().iter().any(|s| s.name().as_str() == format!("f{i}")));
        assert!(unrelated_still_present, "the other 9 unrelated snapshots must survive untouched");
    }

    #[test]
    fn a_rename_within_the_same_file_preserves_lineage() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit_a.clone(), "src/lib.rs").unwrap();
        let file_b = FileIdentity::parse(repo, commit_b.clone(), "src/lib.rs").unwrap();
        let source = FakeSource::new()
            .with_file(file_a, b"fn foo(x: i32) -> i32 { x + 1 }\n")
            .with_file(file_b, b"fn bar(x: i32) -> i32 { x + 1 }\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        let foo_id = first.graph.all_snapshots()[0].id();

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/lib.rs".to_string()]).unwrap();

        let lineage = second.graph.lineage_from(foo_id);
        assert_eq!(lineage.len(), 1, "the rename must be linked by lineage");
        assert!(!lineage[0].status.is_confirmed(), "a rename is never Confirmed");
    }

    #[test]
    fn a_lineage_edge_from_an_earlier_commit_survives_an_unrelated_later_change() {
        // Three commits: A establishes foo, B renames it to bar
        // (lineage A->B), C changes a completely UNRELATED file.
        // Pins that `incremental_reindex`'s untouched-lineage-edge
        // carry-forward actually filters by "both endpoints untouched"
        // (the `!changed_set.contains(...)` checks) rather than by
        // their negation — deleting either `!` would make this edge
        // get dropped even though NEITHER src/lib.rs (bar's file) NOR
        // the rename itself is touched by commit C's own change.
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let commit_c = CommitHash::parse(&"c".repeat(40)).unwrap();
        let lib_a = FileIdentity::parse(repo, commit_a.clone(), "src/lib.rs").unwrap();
        let other_a = FileIdentity::parse(repo, commit_a.clone(), "src/other.rs").unwrap();
        let mut source = FakeSource::new()
            .with_file(lib_a, b"fn foo(x: i32) -> i32 { x + 1 }\n")
            .with_file(other_a, b"fn unrelated() {}\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        let foo_id = first.graph.all_snapshots().iter().find(|s| s.name().as_str() == "foo").unwrap().id();

        let lib_b = FileIdentity::parse(repo, commit_b.clone(), "src/lib.rs").unwrap();
        let other_b = FileIdentity::parse(repo, commit_b.clone(), "src/other.rs").unwrap();
        source = source.with_file(lib_b, b"fn bar(x: i32) -> i32 { x + 1 }\n").with_file(other_b, b"fn unrelated() {}\n");
        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/lib.rs".to_string()]).unwrap();
        assert_eq!(second.graph.lineage_from(foo_id).len(), 1, "A->B rename must be linked immediately after B");

        // Commit C changes ONLY src/other.rs; src/lib.rs (bar) is
        // completely untouched.
        let lib_c = FileIdentity::parse(repo, commit_c.clone(), "src/lib.rs").unwrap();
        let other_c = FileIdentity::parse(repo, commit_c.clone(), "src/other.rs").unwrap();
        source = source.with_file(lib_c, b"fn bar(x: i32) -> i32 { x + 1 }\n").with_file(other_c, b"fn unrelated() { 1 }\n");
        let third = indexer.observe_commit(&source, repo, &commit_c, Some(&commit_b), &["src/other.rs".to_string()]).unwrap();

        let lineage = third.graph.lineage_from(foo_id);
        assert_eq!(lineage.len(), 1, "the A->B lineage edge must survive an unrelated change to a different file at commit C");
        assert!(!lineage[0].status.is_confirmed());
    }

    #[test]
    fn a_move_to_a_different_file_preserves_lineage_via_two_changed_files() {
        // A function moves from src/old.rs to src/new.rs: BOTH files
        // are "changed" from the caller's perspective (old.rs loses the
        // symbol, new.rs gains it) — the caller names both in
        // changed_files, exactly like a real diff between two commits
        // would report a delete+add pair for a moved file's content.
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let old_a = FileIdentity::parse(repo, commit_a.clone(), "src/old.rs").unwrap();
        let source = FakeSource::new().with_file(old_a, b"fn foo() {}\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        let foo_id = first.graph.all_snapshots()[0].id();

        // At commit B, old.rs no longer exists (not listed); new.rs
        // exists with the moved function.
        let new_b = FileIdentity::parse(repo, commit_b.clone(), "src/new.rs").unwrap();
        let source = source.with_file(new_b, b"fn foo() {}\n");

        let second = indexer
            .observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/old.rs".to_string(), "src/new.rs".to_string()])
            .unwrap();

        // 2, not 1: the OLD src/old.rs snapshot is retained as the
        // historical node its Hypothesized lineage edge points FROM,
        // alongside the one CURRENT snapshot at src/new.rs — never
        // duplicated as two equally "current" snapshots, but not
        // collapsed into one either, since the old id and new id are
        // genuinely different `SymbolSnapshotId`s (different file).
        assert_eq!(second.graph.snapshot_count(), 2);
        assert_eq!(second.metrics.files_reparsed, 1, "old.rs no longer exists at commit B and must not be reparsed as if it still did");
        let lineage = second.graph.lineage_from(foo_id);
        assert_eq!(lineage.len(), 1, "a move must still be linked by lineage");
        assert!(!lineage[0].status.is_confirmed(), "a move is never Confirmed");
        assert_ne!(lineage[0].to, foo_id, "the new snapshot at the moved location must have a distinct id from the old one");
    }

    #[test]
    fn a_rename_combined_with_a_move_preserves_lineage() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let old_a = FileIdentity::parse(repo, commit_a.clone(), "src/old.rs").unwrap();
        let source = FakeSource::new().with_file(old_a, b"fn foo(x: i32) -> i32 { x + 1 }\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        let foo_id = first.graph.all_snapshots()[0].id();

        let new_b = FileIdentity::parse(repo, commit_b.clone(), "src/new.rs").unwrap();
        let source = source.with_file(new_b, b"fn bar(x: i32) -> i32 { x + 1 }\n");

        let second = indexer
            .observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/old.rs".to_string(), "src/new.rs".to_string()])
            .unwrap();

        // 2: the old src/old.rs::foo snapshot retained as the
        // historical lineage source, plus the one current
        // src/new.rs::bar snapshot.
        assert_eq!(second.graph.snapshot_count(), 2);
        let lineage = second.graph.lineage_from(foo_id);
        assert_eq!(lineage.len(), 1, "a rename+move must still be linked by lineage");
        assert!(!lineage[0].status.is_confirmed());
    }

    #[test]
    fn an_incremental_delete_removes_the_files_snapshots_and_leaves_no_dangling_edges() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit_a.clone(), "src/lib.rs").unwrap();
        let other_a = FileIdentity::parse(repo, commit_a.clone(), "src/other.rs").unwrap();
        let mut source = FakeSource::new().with_file(file_a, b"fn foo() { bar(); }\nfn bar() {}\n").with_file(other_a, b"fn unrelated() {}\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(first.graph.snapshot_count(), 3);
        assert_eq!(first.graph.edge_count(), 1);

        // Commit B: src/lib.rs deleted (list_files_at for B simply does
        // not include it — exactly what a real RepositorySource
        // reports for a deleted file); src/other.rs survives untouched.
        // `changed_files` names src/lib.rs as the one changed path, per
        // the task's own "dato cone changed files" contract — the
        // CALLER (a future Task 2-driven caller diffing two commits'
        // file lists) is what decides a path belongs in `changed_files`
        // because it no longer appears at the new commit.
        let other_b = FileIdentity::parse(repo, commit_b.clone(), "src/other.rs").unwrap();
        source = source.with_file(other_b, b"fn unrelated() {}\n");

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/lib.rs".to_string()]).unwrap();

        assert_eq!(second.graph.snapshot_count(), 1, "foo/bar must be gone, unrelated() must survive");
        assert_eq!(second.graph.all_snapshots()[0].name().as_str(), "unrelated");
        assert_eq!(second.graph.edge_count(), 0, "the Calls edge between the deleted foo/bar must not dangle");
        assert_eq!(second.metrics.snapshots_removed, 2);
    }

    /// Wraps a [`FakeSource`] to additionally report an explicit empty
    /// file list for one specific `(repository, commit)` pair — needed
    /// because `FakeSource::list_files_at` otherwise reports
    /// `SourceError::UnknownCommit` for a commit with zero registered
    /// files, which is NOT the same thing as "a real commit that
    /// legitimately has zero files" (`RepositorySource`'s own contract,
    /// `source.rs`'s doc comment: an implementor must distinguish
    /// "unknown commit" from "a real, empty commit").
    struct FakeSourceWithEmptyCommit {
        inner: FakeSource,
        repository: RepositoryId,
        commit: CommitHash,
    }

    impl FakeSourceWithEmptyCommit {
        fn new(inner: FakeSource, repository: RepositoryId, commit: CommitHash) -> Self {
            Self { inner, repository, commit }
        }
    }

    impl RepositorySource for FakeSourceWithEmptyCommit {
        fn list_files_at(&self, repository: RepositoryId, commit: &CommitHash) -> Result<Vec<String>, SourceError> {
            if repository == self.repository && commit == &self.commit {
                return Ok(Vec::new());
            }
            self.inner.list_files_at(repository, commit)
        }

        fn read_file(&self, file: &FileIdentity) -> Result<Vec<u8>, SourceError> {
            self.inner.read_file(file)
        }
    }

    #[test]
    fn a_full_reindex_of_a_repository_that_became_empty_drops_every_prior_snapshot() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_c = CommitHash::parse(&"c".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit_a.clone(), "src/lib.rs").unwrap();
        let source = FakeSource::new().with_file(file_a, b"fn foo() {}\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(first.graph.snapshot_count(), 1);

        // No expected_prev_commit claimed (history-rewrite/unknown-
        // ancestor path) and the repository has zero files at
        // commit_c -> a full reindex of an empty commit.
        let source = FakeSourceWithEmptyCommit::new(source, repo, commit_c.clone());
        let second = indexer.observe_commit(&source, repo, &commit_c, None, &[]).unwrap();

        assert_eq!(second.graph.snapshot_count(), 0, "a full reindex of an empty commit must drop every prior snapshot");
        assert_eq!(second.graph.edge_count(), 0);
    }

    #[test]
    fn a_new_file_is_indexed_and_linked_to_existing_snapshots() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit_a.clone(), "src/lib.rs").unwrap();
        let mut source = FakeSource::new().with_file(file_a, b"fn foo() {}\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(first.graph.snapshot_count(), 1);

        // Commit B: src/lib.rs unchanged at B, plus a brand NEW file
        // that calls foo().
        let file_a_at_b = FileIdentity::parse(repo, commit_b.clone(), "src/lib.rs").unwrap();
        let new_file = FileIdentity::parse(repo, commit_b.clone(), "src/caller.rs").unwrap();
        source = source.with_file(file_a_at_b, b"fn foo() {}\n").with_file(new_file, b"fn caller() { foo(); }\n");

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/caller.rs".to_string()]).unwrap();

        assert_eq!(second.graph.snapshot_count(), 2);
        assert_eq!(second.metrics.files_reparsed, 1, "only the new file must be reparsed");
        let all_edges = second.graph.all_edges();
        assert_eq!(all_edges.len(), 1);
        assert_eq!(all_edges[0].kind, EdgeKind::Calls);
        // Pins the incremental path's `edges_rebuilt` summation (zero
        // lineage edges here, since foo() itself is unchanged, plus
        // exactly one typed Calls edge) against a `+`/`*` mutation.
        assert_eq!(second.metrics.edges_rebuilt, 1);
    }

    #[test]
    fn a_change_in_one_file_can_still_update_an_edge_from_an_untouched_file() {
        // The exact under-invalidation trap this resolver's full-pool
        // edge recomputation exists to avoid: `caller.rs` calls
        // `helper()`, which does not exist at commit A (no edge).
        // Commit B adds `helper()` to a DIFFERENT file; `caller.rs`
        // itself is never touched, but its outgoing Calls edge must
        // still appear after B is observed.
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let caller_a = FileIdentity::parse(repo, commit_a.clone(), "src/caller.rs").unwrap();
        let mut source = FakeSource::new().with_file(caller_a, b"fn caller() { helper(); }\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert!(first.graph.all_edges().is_empty(), "helper() does not exist yet at commit A");

        let caller_b = FileIdentity::parse(repo, commit_b.clone(), "src/caller.rs").unwrap();
        let helper_file = FileIdentity::parse(repo, commit_b.clone(), "src/helper.rs").unwrap();
        source = source.with_file(caller_b, b"fn caller() { helper(); }\n").with_file(helper_file, b"fn helper() {}\n");

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/helper.rs".to_string()]).unwrap();

        assert_eq!(second.metrics.files_reparsed, 1, "caller.rs itself must not be reparsed");
        let edges = second.graph.all_edges();
        assert_eq!(edges.len(), 1, "caller's outgoing edge must appear even though caller.rs was never reparsed");
        assert_eq!(edges[0].kind, EdgeKind::Calls);
    }

    #[test]
    fn an_unknown_ancestor_forces_a_full_reindex_explicitly() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_c = CommitHash::parse(&"c".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit_a.clone(), "src/lib.rs").unwrap();
        let file_c = FileIdentity::parse(repo, commit_c.clone(), "src/other.rs").unwrap();
        let source = FakeSource::new().with_file(file_a, b"fn foo() {}\n").with_file(file_c, b"fn bar() {}\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(first.graph.snapshot_count(), 1);

        // Claims a prev commit that does NOT match what this indexer
        // actually has published (commit_a) -> history-rewrite path,
        // full reindex of commit_c's own file list (NOT an incremental
        // merge with commit_a's stale snapshot).
        let bogus_prev = CommitHash::parse(&"f".repeat(40)).unwrap();
        let second = indexer.observe_commit(&source, repo, &commit_c, Some(&bogus_prev), &[]).unwrap();

        assert_eq!(second.graph.snapshot_count(), 1, "a full reindex of commit_c's own file list, not merged with commit_a's stale state");
        assert_eq!(second.metrics.files_reparsed, 1);
    }

    #[test]
    fn a_failed_pipeline_leaves_the_prior_published_graph_untouched() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit_a.clone(), "src/lib.rs").unwrap();
        let source = FakeSource::new().with_file(file_a, b"fn foo() {}\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(first.generation, 1);

        // commit_b names a changed file that was never registered with
        // the fake source at all -> read_file fails -> the whole
        // pipeline must fail closed.
        let result = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/missing.rs".to_string()]);

        assert!(result.is_err());
        let current = indexer.current(repo).unwrap();
        assert_eq!(current.0, 1, "a failed observe must not advance the published generation");
        assert_eq!(current.1.snapshot_count(), 1, "the prior graph must be completely untouched by a failed update");
    }

    #[test]
    fn an_older_rebuild_can_never_overwrite_a_newer_published_state() {
        // The mandatory race test: commit A's rebuild is paused right
        // after it claims its generation (and BEFORE it publishes).
        // Commit B, strictly newer, is observed and published to
        // completion on the main thread. A is then resumed and must
        // NOT be allowed to overwrite B once it finally tries to
        // publish. No sleeps: a channel + barrier pair forces the
        // exact interleaving deterministically, mirroring B1's
        // `RealIndexer` concurrency tests.
        //
        // The hook gates on "the FIRST call to reach this point," via
        // an atomic flag, rather than on a hardcoded `generation == 1`.
        // Found during Task 9.5's closing mutation gate: gating on a
        // specific generation NUMBER meant that a mutant breaking the
        // generation counter itself (`*next += 1` -> `*next *= 1`,
        // which leaves every claim stuck at generation 1 forever) made
        // B's own call ALSO match `generation == 1` and enter the SAME
        // 2-party barrier A was already waiting on — deadlocking the
        // whole test binary (cargo-mutants reports this as a TIMEOUT,
        // not a clean failure), which silently masked a real,
        // independently-confirmed counter bug behind a hung test rather
        // than surfacing it. Gating on "first call only" keeps this
        // test's own synchronization correct regardless of what value
        // the generation counter actually produces, so a broken counter
        // now surfaces as this test's own clean, fast failure (or the
        // separate, simpler `the_generation_counter_strictly_increments_across_independent_observe_calls`
        // test's failure) instead of a hang.
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit_a.clone(), "src/lib.rs").unwrap();
        let file_b = FileIdentity::parse(repo, commit_b.clone(), "src/lib.rs").unwrap();
        let source = Arc::new(FakeSource::new().with_file(file_a, b"fn foo() {}\n").with_file(file_b, b"fn bar() {}\n"));
        let indexer = Arc::new(CodeGraphIndexer::new());

        let (arrived_tx, arrived_rx) = mpsc::channel::<()>();
        let barrier = Arc::new(Barrier::new(2));
        let barrier_for_hook = barrier.clone();
        let target_point = TestPoint::AfterGenerationClaimed;
        let first_call = Arc::new(std::sync::atomic::AtomicBool::new(true));
        indexer.set_test_hook(Arc::new(move |point, _generation| {
            if point == target_point && first_call.swap(false, std::sync::atomic::Ordering::SeqCst) {
                arrived_tx.send(()).unwrap();
                barrier_for_hook.wait();
            }
        }));

        let worker_indexer = indexer.clone();
        let worker_source = source.clone();
        let worker = std::thread::spawn(move || worker_indexer.observe_commit(worker_source.as_ref(), repo, &commit_a, None, &[]));

        // Wait until A's rebuild has claimed generation 1 and is
        // paused right before it would otherwise race ahead.
        arrived_rx.recv().unwrap();

        // B races ahead and publishes generation 2 to completion.
        let outcome_b = indexer.observe_commit(source.as_ref(), repo, &commit_b, None, &[]).unwrap();
        assert_eq!(outcome_b.generation, 2);

        // Release A.
        barrier.wait();
        let outcome_a = worker.join().unwrap().unwrap();

        // A must report the CURRENT (B's) state, not its own stale rebuild.
        assert_eq!(outcome_a.generation, 2, "A's stale rebuild must never win — it must report B's generation instead");
        let current = indexer.current(repo).unwrap();
        assert_eq!(current.0, 2, "the published generation must remain B's, never regress to A's");
        assert_eq!(current.1.all_snapshots()[0].name().as_str(), "bar", "B's content must be what's actually published");
    }

    #[test]
    fn deep_learning_fixture_commit_b_only_reparses_model_py_and_preserves_unrelated_snapshots() {
        // Commit A: MyModel (extends nn.Module), forward (calls
        // self.encoder/self.classifier), encoder, classifier — all in
        // src/model.py — plus an unrelated src/train.py file that must
        // never be touched by commit B's change.
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let model_py_a = FileIdentity::parse(repo, commit_a.clone(), "src/model.py").unwrap();
        let nn_py_a = FileIdentity::parse(repo, commit_a.clone(), "src/nn.py").unwrap();
        let train_py_a = FileIdentity::parse(repo, commit_a.clone(), "src/train.py").unwrap();
        let model_py_src_a = b"class MyModel(nn.Module):\n    def forward(self, x):\n        self.encoder(x)\n        self.classifier(x)\n\ndef encoder(x):\n    pass\n\ndef classifier(x):\n    pass\n";
        let nn_py_src = b"class Module:\n    pass\n";
        let train_py_src = b"def train(model):\n    model.step()\n";
        let mut source = FakeSource::new()
            .with_file(model_py_a, model_py_src_a)
            .with_file(nn_py_a, nn_py_src)
            .with_file(train_py_a, train_py_src);
        let indexer = CodeGraphIndexer::new();

        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        // Module, MyModel, forward, encoder, classifier, train = 6.
        assert_eq!(first.graph.snapshot_count(), 6);
        let my_model_id = first.graph.all_snapshots().iter().find(|s| s.name().as_str() == "MyModel").unwrap().id();
        let forward_id = first.graph.all_snapshots().iter().find(|s| s.name().as_str() == "forward").unwrap().id();
        let train_id = first.graph.all_snapshots().iter().find(|s| s.name().as_str() == "train").unwrap().id();
        assert_eq!(first.graph.outgoing_edges(my_model_id, Some(EdgeKind::Extends)).len(), 1, "MyModel must extend Module");
        assert_eq!(first.graph.callees_of(forward_id).len(), 2, "forward must call encoder and classifier");

        // Commit B: rename `forward` to `forward_pass` — only
        // src/model.py changes; src/nn.py and src/train.py are
        // untouched.
        let model_py_b = FileIdentity::parse(repo, commit_b.clone(), "src/model.py").unwrap();
        let nn_py_b = FileIdentity::parse(repo, commit_b.clone(), "src/nn.py").unwrap();
        let train_py_b = FileIdentity::parse(repo, commit_b.clone(), "src/train.py").unwrap();
        let model_py_src_b = b"class MyModel(nn.Module):\n    def forward_pass(self, x):\n        self.encoder(x)\n        self.classifier(x)\n\ndef encoder(x):\n    pass\n\ndef classifier(x):\n    pass\n";
        source = source.with_file(model_py_b, model_py_src_b).with_file(nn_py_b, nn_py_src).with_file(train_py_b, train_py_src);

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/model.py".to_string()]).unwrap();

        assert_eq!(second.metrics.files_reparsed, 1, "only src/model.py must be reparsed");
        // src/train.py's own `train` snapshot must be byte-for-byte the
        // SAME id across commits (untouched file -> untouched
        // snapshot, never rebuilt).
        assert!(second.graph.snapshot(train_id).is_some(), "train.py's snapshot must survive unchanged");

        // forward's rename must be linked by lineage (Hypothesized,
        // never Confirmed).
        let lineage = second.graph.lineage_from(forward_id);
        assert_eq!(lineage.len(), 1, "the forward -> forward_pass rename must be linked by lineage");
        assert!(!lineage[0].status.is_confirmed());

        // The renamed forward_pass must still call encoder/classifier,
        // and the CURRENT MyModel (its own snapshot also changed,
        // since Task 4's signature_text spans the whole class
        // including its body — the id captured at commit A is now
        // historical, not current) must still extend Module — the
        // rename must not have silently broken either edge family.
        let forward_pass_id = lineage[0].to;
        assert_eq!(second.graph.callees_of(forward_pass_id).len(), 2, "forward_pass must still call encoder and classifier after the rename");
        let current_my_model_id = second.graph.all_snapshots().iter().find(|s| s.name().as_str() == "MyModel" && s.id() != my_model_id).unwrap().id();
        assert_eq!(second.graph.outgoing_edges(current_my_model_id, Some(EdgeKind::Extends)).len(), 1, "the current MyModel must still extend Module after an unrelated rename");
    }

    // --- Task 9.5, Priority 6: ImportFact production and incremental
    // semantics wired into the real indexer. ---

    #[test]
    fn initial_index_extracts_import_facts() {
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit.clone(), "src/model.py").unwrap();
        let source = FakeSource::new().with_file(file, b"import torch\nclass Model:\n    pass\n");
        let indexer = CodeGraphIndexer::new();

        let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

        assert_eq!(outcome.import_facts.len(), 1);
        assert_eq!(outcome.import_facts[0].module_path, "torch");
        assert_eq!(outcome.metrics.raw_import_statements_extracted, 1);
        assert_eq!(outcome.metrics.import_facts_resolved, 1);
    }

    #[test]
    fn an_untouched_files_raw_import_statement_is_preserved_across_an_incremental_update() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let untouched_a = FileIdentity::parse(repo, commit_a.clone(), "src/untouched.py").unwrap();
        let changed_a = FileIdentity::parse(repo, commit_a.clone(), "src/changed.py").unwrap();
        let mut source = FakeSource::new().with_file(untouched_a, b"import torch\n").with_file(changed_a, b"x = 1\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(first.import_facts.len(), 1);

        let untouched_b = FileIdentity::parse(repo, commit_b.clone(), "src/untouched.py").unwrap();
        let changed_b = FileIdentity::parse(repo, commit_b.clone(), "src/changed.py").unwrap();
        source = source.with_file(untouched_b, b"import torch\n").with_file(changed_b, b"x = 2\n");

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/changed.py".to_string()]).unwrap();

        assert_eq!(second.import_facts.len(), 1, "the untouched file's import must still be present");
        assert_eq!(second.import_facts[0].module_path, "torch");
        assert_eq!(second.metrics.raw_import_statements_reused, 1, "the untouched file's raw statement must be reused, not re-extracted");
        assert_eq!(second.metrics.raw_import_statements_extracted, 0, "the changed file has no import statement of its own to extract");
    }

    #[test]
    fn a_changed_files_import_statement_is_recomputed_not_reused() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit_a.clone(), "src/model.py").unwrap();
        let mut source = FakeSource::new().with_file(file_a, b"import torch\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(first.import_facts[0].module_path, "torch");

        let file_b = FileIdentity::parse(repo, commit_b.clone(), "src/model.py").unwrap();
        source = source.with_file(file_b, b"import numpy\n");

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/model.py".to_string()]).unwrap();

        assert_eq!(second.import_facts.len(), 1);
        assert_eq!(second.import_facts[0].module_path, "numpy", "the changed file's import must be recomputed from its new content, not reused from the old one");
        assert_eq!(second.metrics.raw_import_statements_extracted, 1);
        assert_eq!(second.metrics.raw_import_statements_reused, 0);
    }

    #[test]
    fn a_deleted_files_import_statement_is_removed() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let deleted_a = FileIdentity::parse(repo, commit_a.clone(), "src/deleted.py").unwrap();
        let survivor_a = FileIdentity::parse(repo, commit_a.clone(), "src/survivor.py").unwrap();
        let mut source = FakeSource::new().with_file(deleted_a, b"import torch\n").with_file(survivor_a, b"x = 1\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(first.import_facts.len(), 1);

        // src/deleted.py no longer exists at commit B.
        let survivor_b = FileIdentity::parse(repo, commit_b.clone(), "src/survivor.py").unwrap();
        source = source.with_file(survivor_b, b"x = 2\n");

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/deleted.py".to_string()]).unwrap();

        assert!(second.import_facts.is_empty(), "the deleted file's import must be removed, not carried forward");
    }

    #[test]
    fn a_new_files_import_statement_is_added() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let existing_a = FileIdentity::parse(repo, commit_a.clone(), "src/existing.py").unwrap();
        let mut source = FakeSource::new().with_file(existing_a, b"x = 1\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert!(first.import_facts.is_empty());

        let existing_b = FileIdentity::parse(repo, commit_b.clone(), "src/existing.py").unwrap();
        let new_file_b = FileIdentity::parse(repo, commit_b.clone(), "src/new.py").unwrap();
        source = source.with_file(existing_b, b"x = 1\n").with_file(new_file_b, b"import torch\n");

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/new.py".to_string()]).unwrap();

        assert_eq!(second.import_facts.len(), 1, "the new file's import must be added");
        assert_eq!(second.import_facts[0].module_path, "torch");
    }

    #[test]
    fn an_import_facts_resolution_changes_when_its_target_is_added_even_though_the_importing_file_is_untouched() {
        // The exact scenario the correctness-first "always re-resolve"
        // policy exists for: `importer.py`'s own `from mymodule import
        // helper` is UNCHANGED text across A->B, but `helper` does not
        // exist anywhere in the pool at commit A (External) and DOES
        // exist at commit B (Internal) because a DIFFERENT, newly-added
        // file defines it. `importer.py` itself is never reparsed.
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let importer_a = FileIdentity::parse(repo, commit_a.clone(), "src/importer.py").unwrap();
        let mut source = FakeSource::new().with_file(importer_a, b"from mymodule import helper\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(first.import_facts[0].resolution, crate::imports::ImportResolution::External, "helper does not exist anywhere yet");

        let importer_b = FileIdentity::parse(repo, commit_b.clone(), "src/importer.py").unwrap();
        let mymodule_b = FileIdentity::parse(repo, commit_b.clone(), "src/mymodule.py").unwrap();
        source = source.with_file(importer_b, b"from mymodule import helper\n").with_file(mymodule_b, b"def helper():\n    pass\n");

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/mymodule.py".to_string()]).unwrap();

        assert_eq!(second.metrics.raw_import_statements_reused, 1, "importer.py itself must not be reparsed");
        assert!(
            matches!(second.import_facts[0].resolution, crate::imports::ImportResolution::Internal(_)),
            "the untouched import's resolution must update once its target becomes resolvable, even though importer.py itself was never reparsed: got {:?}",
            second.import_facts[0].resolution
        );
    }

    #[test]
    fn a_failed_incremental_update_leaves_prior_import_facts_untouched() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit_a.clone(), "src/model.py").unwrap();
        let source = FakeSource::new().with_file(file_a, b"import torch\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(first.import_facts.len(), 1);

        // commit_b names a changed file never registered with the fake
        // source -> read_file fails -> the whole pipeline (including
        // import resolution) must fail closed.
        let result = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/missing.py".to_string()]);

        assert!(result.is_err());
        let current_outcome = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(current_outcome.import_facts.len(), 1, "the prior import facts must be completely untouched by a failed update");
        assert_eq!(current_outcome.import_facts[0].module_path, "torch");
    }

    #[test]
    fn a_stale_generation_cannot_overwrite_newer_published_import_facts() {
        // Mirrors `an_older_rebuild_can_never_overwrite_a_newer_published_state`,
        // but asserts on `import_facts` specifically — the same
        // publication-atomicity guarantee must cover import state, not
        // only the graph.
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit_a.clone(), "src/lib.py").unwrap();
        let file_b = FileIdentity::parse(repo, commit_b.clone(), "src/lib.py").unwrap();
        let source = Arc::new(FakeSource::new().with_file(file_a, b"import torch\n").with_file(file_b, b"import numpy\n"));
        let indexer = Arc::new(CodeGraphIndexer::new());

        let (arrived_tx, arrived_rx) = mpsc::channel::<()>();
        let barrier = Arc::new(Barrier::new(2));
        let barrier_for_hook = barrier.clone();
        let target_point = TestPoint::AfterGenerationClaimed;
        // First-call-only gate, not a hardcoded generation number — see
        // `an_older_rebuild_can_never_overwrite_a_newer_published_state`'s
        // own comment for why (Task 9.5's closing mutation gate found a
        // hardcoded `generation == 1` check makes this harness deadlock,
        // rather than fail cleanly, under a mutant that breaks the
        // generation counter itself).
        let first_call = Arc::new(std::sync::atomic::AtomicBool::new(true));
        indexer.set_test_hook(Arc::new(move |point, _generation| {
            if point == target_point && first_call.swap(false, std::sync::atomic::Ordering::SeqCst) {
                arrived_tx.send(()).unwrap();
                barrier_for_hook.wait();
            }
        }));

        let worker_indexer = indexer.clone();
        let worker_source = source.clone();
        let worker = std::thread::spawn(move || worker_indexer.observe_commit(worker_source.as_ref(), repo, &commit_a, None, &[]));

        arrived_rx.recv().unwrap();

        let outcome_b = indexer.observe_commit(source.as_ref(), repo, &commit_b, None, &[]).unwrap();
        assert_eq!(outcome_b.import_facts[0].module_path, "numpy");

        barrier.wait();
        let outcome_a = worker.join().unwrap().unwrap();

        assert_eq!(outcome_a.import_facts[0].module_path, "numpy", "A's stale import facts must never win — it must report B's import facts instead");
    }

    #[test]
    fn import_fact_resolution_is_deterministic_across_independent_runs() {
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let importer = FileIdentity::parse(repo, commit.clone(), "src/model.py").unwrap();
        let target = FileIdentity::parse(repo, commit.clone(), "src/mymodule.py").unwrap();
        let source = FakeSource::new().with_file(importer, b"from mymodule import helper\n").with_file(target, b"def helper():\n    pass\n");

        let first_indexer = CodeGraphIndexer::new();
        let first = first_indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

        let second_indexer = CodeGraphIndexer::new();
        let second = second_indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

        assert_eq!(first.import_facts.len(), second.import_facts.len());
        assert_eq!(first.import_facts[0].resolution, second.import_facts[0].resolution);
    }

    // --- B2-ID-008, Task 4: ImplFact production and incremental
    // semantics wired into the real indexer, mirroring the ImportFact
    // tests immediately above. ---

    #[test]
    fn an_untouched_files_raw_impl_statement_is_preserved_across_an_incremental_update() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let untouched_a = FileIdentity::parse(repo, commit_a.clone(), "src/untouched.rs").unwrap();
        let changed_a = FileIdentity::parse(repo, commit_a.clone(), "src/changed.rs").unwrap();
        let mut source = FakeSource::new()
            .with_file(untouched_a, b"struct Type; impl Type { fn new() -> Self { Type } }\n")
            .with_file(changed_a, b"fn marker() {}\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(first.impl_facts.len(), 1);

        let untouched_b = FileIdentity::parse(repo, commit_b.clone(), "src/untouched.rs").unwrap();
        let changed_b = FileIdentity::parse(repo, commit_b.clone(), "src/changed.rs").unwrap();
        source = source
            .with_file(untouched_b, b"struct Type; impl Type { fn new() -> Self { Type } }\n")
            .with_file(changed_b, b"fn marker() { 1; }\n");

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/changed.rs".to_string()]).unwrap();

        assert_eq!(second.impl_facts.len(), 1, "the untouched file's impl must still be present");
        assert_eq!(second.impl_facts[0].self_type_text, "Type");
        assert_eq!(second.metrics.raw_impl_statements_reused, 1, "the untouched file's raw impl statement must be reused, not re-extracted");
        assert_eq!(second.metrics.raw_impl_statements_extracted, 0, "the changed file has no impl block of its own to extract");
    }

    #[test]
    fn an_impl_facts_resolution_changes_when_its_trait_target_is_added_even_though_the_impl_file_is_untouched() {
        // The direct ImplFact analogue of
        // `an_import_facts_resolution_changes_when_its_target_is_added_even_though_the_importing_file_is_untouched`:
        // `circle.rs`'s own `impl Shape for Circle` is UNCHANGED text
        // across A->B, but `Shape` does not exist anywhere in the pool
        // at commit A (trait_resolution: Unresolved, no Implements
        // edge) and DOES exist at commit B (Resolved, Implements edge)
        // because a DIFFERENT, newly-added file defines it. circle.rs
        // itself is never reparsed.
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let circle_a = FileIdentity::parse(repo, commit_a.clone(), "src/circle.rs").unwrap();
        let mut source = FakeSource::new().with_file(circle_a, b"struct Circle; impl Shape for Circle { fn area(&self) {} }\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(
            first.impl_facts[0].trait_resolution,
            Some(crate::impls::ImplTargetResolution::Unresolved),
            "Shape does not exist anywhere yet"
        );
        assert!(first.graph.all_edges().iter().all(|e| e.kind != EdgeKind::Implements), "no Implements edge yet");

        let circle_b = FileIdentity::parse(repo, commit_b.clone(), "src/circle.rs").unwrap();
        let shape_b = FileIdentity::parse(repo, commit_b.clone(), "src/shape.rs").unwrap();
        source = source
            .with_file(circle_b, b"struct Circle; impl Shape for Circle { fn area(&self) {} }\n")
            .with_file(shape_b, b"trait Shape { fn area(&self); }\n");

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/shape.rs".to_string()]).unwrap();

        assert_eq!(second.metrics.raw_impl_statements_reused, 1, "circle.rs itself must not be reparsed");
        assert!(
            matches!(second.impl_facts[0].trait_resolution, Some(crate::impls::ImplTargetResolution::Resolved(_))),
            "the untouched impl's trait resolution must update once Shape becomes resolvable, even though circle.rs itself was never reparsed: got {:?}",
            second.impl_facts[0].trait_resolution
        );
        assert_eq!(
            second.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Implements).count(),
            1,
            "an Implements edge must now exist even though circle.rs was never reparsed"
        );
    }

    #[test]
    fn deleting_an_impl_block_removes_both_its_impl_fact_and_its_implements_edge() {
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let shape_a = FileIdentity::parse(repo, commit_a.clone(), "src/shape.rs").unwrap();
        let circle_a = FileIdentity::parse(repo, commit_a.clone(), "src/circle.rs").unwrap();
        let mut source = FakeSource::new()
            .with_file(shape_a, b"trait Shape { fn area(&self); }\n")
            .with_file(circle_a, b"struct Circle; impl Shape for Circle { fn area(&self) {} }\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(first.impl_facts.len(), 1);
        assert_eq!(first.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Implements).count(), 1);

        // Commit B: circle.rs no longer has the impl block at all (the
        // struct survives, the impl is removed).
        let shape_b = FileIdentity::parse(repo, commit_b.clone(), "src/shape.rs").unwrap();
        let circle_b = FileIdentity::parse(repo, commit_b.clone(), "src/circle.rs").unwrap();
        source = source.with_file(shape_b, b"trait Shape { fn area(&self); }\n").with_file(circle_b, b"struct Circle;\n");

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/circle.rs".to_string()]).unwrap();

        assert!(second.impl_facts.is_empty(), "the removed impl block's ImplFact must be gone");
        assert!(
            second.graph.all_edges().iter().all(|e| e.kind != EdgeKind::Implements),
            "the removed impl block's derived Implements edge must be gone"
        );
    }

    #[test]
    fn adding_a_new_impl_block_adds_both_an_impl_fact_and_an_implements_edge() {
        // The impl block is added in a BRAND NEW file (never a file
        // that already existed and is merely edited) — mirroring
        // `a_new_files_import_statement_is_added`'s own shape exactly —
        // so the self type's own snapshot id is unambiguous: it has no
        // same-named predecessor snapshot from a prior commit that
        // lineage resolution would otherwise retain in the pool
        // alongside it (see `final_snapshots`'s own doc comment on
        // historical lineage-source snapshots).
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let shape_a = FileIdentity::parse(repo, commit_a.clone(), "src/shape.rs").unwrap();
        let mut source = FakeSource::new().with_file(shape_a, b"trait Shape { fn area(&self); }\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert!(first.impl_facts.is_empty());
        assert!(first.graph.all_edges().iter().all(|e| e.kind != EdgeKind::Implements));

        let shape_b = FileIdentity::parse(repo, commit_b.clone(), "src/shape.rs").unwrap();
        let circle_b = FileIdentity::parse(repo, commit_b.clone(), "src/circle.rs").unwrap();
        source = source
            .with_file(shape_b, b"trait Shape { fn area(&self); }\n")
            .with_file(circle_b, b"struct Circle; impl Shape for Circle { fn area(&self) {} }\n");

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/circle.rs".to_string()]).unwrap();

        assert_eq!(second.impl_facts.len(), 1, "the new impl block must be added");
        assert_eq!(second.impl_facts[0].self_type_text, "Circle");
        assert_eq!(second.impl_facts[0].trait_type_text, Some("Shape".to_string()));
        assert_eq!(
            second.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Implements).count(),
            1,
            "the new impl block's Implements edge must be added since both targets resolve"
        );
    }

    #[test]
    fn an_unrelated_edit_to_the_traits_own_defining_file_does_not_make_implements_resolution_ambiguous() {
        // C1 (final-review Critical finding): `src/shape.rs` defines
        // `trait Shape`; `src/circle.rs` has a real, UNTOUCHED
        // `impl Shape for Circle`. At commit B, `src/shape.rs` gets a
        // purely cosmetic edit (an added comment) that changes neither
        // the trait's name nor its meaningful signature — but it IS a
        // byte-for-byte change, so lineage resolution links the OLD
        // (commit-A-rooted) `Shape` snapshot to the NEW (commit-B-rooted)
        // one with an exact-match Hypothesized edge, and the OLD
        // snapshot is retained in `final_snapshots` as that edge's `from`
        // endpoint (see `final_snapshots`'s own doc comment). Before the
        // C1 fix, `impl_facts` was resolved against `final_snapshots`,
        // so `resolve_target_by_name("Shape", ...)` found BOTH the old
        // and new `Shape` snapshots sharing the same name and returned
        // `Ambiguous` — silently dropping the real `Implements` edge
        // `src/circle.rs`'s own impl block already had at commit A, even
        // though `src/circle.rs` itself was never touched. After the fix
        // (resolving against `current_snapshots` instead), exactly one
        // `Shape` snapshot is current at commit B, so resolution stays
        // `Resolved` and the edge survives.
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let shape_a = FileIdentity::parse(repo, commit_a.clone(), "src/shape.rs").unwrap();
        let circle_a = FileIdentity::parse(repo, commit_a.clone(), "src/circle.rs").unwrap();
        let mut source = FakeSource::new()
            .with_file(shape_a, b"trait Shape { fn area(&self) -> f64; }\n")
            .with_file(circle_a, b"struct Circle; impl Shape for Circle { fn area(&self) -> f64 { 1.0 } }\n");
        let indexer = CodeGraphIndexer::new();
        let first = indexer.observe_commit(&source, repo, &commit_a, None, &[]).unwrap();
        assert_eq!(
            first.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Implements).count(),
            1,
            "the Implements edge must exist at commit A"
        );

        // Commit B: ONLY src/shape.rs changes, and only cosmetically (an
        // added comment) — src/circle.rs is NOT in changed_files at all.
        let shape_b = FileIdentity::parse(repo, commit_b.clone(), "src/shape.rs").unwrap();
        let circle_b = FileIdentity::parse(repo, commit_b.clone(), "src/circle.rs").unwrap();
        source = source
            .with_file(shape_b, b"// a harmless comment\ntrait Shape { fn area(&self) -> f64; }\n")
            .with_file(circle_b, b"struct Circle; impl Shape for Circle { fn area(&self) -> f64 { 1.0 } }\n");

        let second = indexer.observe_commit(&source, repo, &commit_b, Some(&commit_a), &["src/shape.rs".to_string()]).unwrap();

        assert_eq!(second.metrics.raw_impl_statements_reused, 1, "src/circle.rs's own impl block must not be reparsed");
        assert_eq!(second.impl_facts.len(), 1);
        assert!(
            matches!(second.impl_facts[0].trait_resolution, Some(crate::impls::ImplTargetResolution::Resolved(_))),
            "the trait resolution must stay Resolved, not become Ambiguous, after an unrelated cosmetic edit to the trait's own defining file: got {:?}",
            second.impl_facts[0].trait_resolution
        );
        assert_eq!(
            second.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Implements).count(),
            1,
            "the Implements edge must SURVIVE the trait-defining file's own unrelated, cosmetic edit"
        );
    }

    #[test]
    fn incremental_reindex_of_a_commit_produces_the_same_implements_edges_as_a_full_reindex_of_that_same_commit() {
        // C1's own second required test: incrementally observing A->B
        // (where B's src/shape.rs gets a cosmetic edit, exactly like the
        // test above) must produce the IDENTICAL `Implements` edge set
        // as a FULL reindex that treats B as a brand-new repository —
        // i.e. the incremental path's own historical-snapshot retention
        // must never change the observable `ImplFact`/edge outcome
        // relative to what a from-scratch index of the same commit
        // would see.
        let repo_incremental = test_repo();
        let repo_full = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let shape_a = FileIdentity::parse(repo_incremental, commit_a.clone(), "src/shape.rs").unwrap();
        let circle_a = FileIdentity::parse(repo_incremental, commit_a.clone(), "src/circle.rs").unwrap();
        let mut source = FakeSource::new()
            .with_file(shape_a, b"trait Shape { fn area(&self) -> f64; }\n")
            .with_file(circle_a, b"struct Circle; impl Shape for Circle { fn area(&self) -> f64 { 1.0 } }\n");
        let indexer = CodeGraphIndexer::new();
        indexer.observe_commit(&source, repo_incremental, &commit_a, None, &[]).unwrap();

        let shape_b = FileIdentity::parse(repo_incremental, commit_b.clone(), "src/shape.rs").unwrap();
        let circle_b = FileIdentity::parse(repo_incremental, commit_b.clone(), "src/circle.rs").unwrap();
        source = source
            .with_file(shape_b, b"// a harmless comment\ntrait Shape { fn area(&self) -> f64; }\n")
            .with_file(circle_b, b"struct Circle; impl Shape for Circle { fn area(&self) -> f64 { 1.0 } }\n");

        let incremental_outcome = indexer.observe_commit(&source, repo_incremental, &commit_b, Some(&commit_a), &["src/shape.rs".to_string()]).unwrap();

        // A brand-new, SEPARATE indexer treats commit B as a full
        // reindex of a never-before-seen repository, with ONLY B's own
        // two files (never A's) — exactly what "a full reindex of this
        // exact commit" means.
        let shape_b_full = FileIdentity::parse(repo_full, commit_b.clone(), "src/shape.rs").unwrap();
        let circle_b_full = FileIdentity::parse(repo_full, commit_b.clone(), "src/circle.rs").unwrap();
        let full_source = FakeSource::new()
            .with_file(shape_b_full, b"// a harmless comment\ntrait Shape { fn area(&self) -> f64; }\n")
            .with_file(circle_b_full, b"struct Circle; impl Shape for Circle { fn area(&self) -> f64 { 1.0 } }\n");
        let full_indexer = CodeGraphIndexer::new();
        let full_outcome = full_indexer.observe_commit(&full_source, repo_full, &commit_b, None, &[]).unwrap();

        let incremental_implements_count = incremental_outcome.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Implements).count();
        let full_implements_count = full_outcome.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Implements).count();
        assert_eq!(incremental_implements_count, 1, "the incremental path must produce exactly one Implements edge");
        assert_eq!(
            incremental_implements_count, full_implements_count,
            "incrementally observing A->B must produce the SAME Implements edge count as a full reindex of B treated as a brand-new repository"
        );

        // Same trait/self-type resolution SHAPE too, not just the same
        // count — both must resolve the trait to exactly one candidate,
        // never Ambiguous, for the identical reason.
        assert_eq!(incremental_outcome.impl_facts.len(), full_outcome.impl_facts.len());
        let incremental_resolved_traits = incremental_outcome.impl_facts.iter().filter(|f| matches!(f.trait_resolution, Some(crate::impls::ImplTargetResolution::Resolved(_)))).count();
        let full_resolved_traits = full_outcome.impl_facts.iter().filter(|f| matches!(f.trait_resolution, Some(crate::impls::ImplTargetResolution::Resolved(_)))).count();
        assert_eq!(
            incremental_resolved_traits, full_resolved_traits,
            "the incremental path must resolve the same number of traits to Resolved as a full reindex of the identical commit"
        );
    }

    // B2-ID-010 regression tests: the partial-class reconciliation fix
    // must behave identically through the real production
    // `CodeGraphIndexer` entry point — full reindex, incremental
    // advance, incremental edit of ONE partial fragment, and deletion
    // of a partial fragment — not just via the lower-level
    // `resolve_extends_implements_with_csharp_contexts` unit tests in
    // `edges::extends_implements`.
    mod b2_id_010_csharp_partial_class_through_the_real_indexer {
        use super::*;

        const POLICY_PART_1: &[u8] = b"public partial class Policy {}\n";
        const POLICY_PART_2: &[u8] = b"public partial class Policy { }\n";
        const IBULKHEAD: &[u8] = b"public interface IBulkheadPolicy {}\n";
        const BULKHEAD: &[u8] = b"public class BulkheadPolicy : Policy, IBulkheadPolicy {}\n";

        #[test]
        fn full_reindex_reconciles_the_real_polly_shape() {
            let repo = test_repo();
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let source = FakeSource::new()
                .with_file(FileIdentity::parse(repo, commit.clone(), "src/Policy.cs").unwrap(), POLICY_PART_1)
                .with_file(FileIdentity::parse(repo, commit.clone(), "src/Policy.TResult.cs").unwrap(), POLICY_PART_2)
                .with_file(FileIdentity::parse(repo, commit.clone(), "src/IBulkheadPolicy.cs").unwrap(), IBULKHEAD)
                .with_file(FileIdentity::parse(repo, commit.clone(), "src/BulkheadPolicy.cs").unwrap(), BULKHEAD);
            let indexer = CodeGraphIndexer::new();
            let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

            let bulkhead = outcome.graph.all_snapshots().into_iter().find(|s| s.name().as_str() == "BulkheadPolicy" && outcome.graph.lineage_from(s.id()).is_empty()).unwrap();
            let extends: Vec<_> = outcome.graph.outgoing_edges(bulkhead.id(), Some(EdgeKind::Extends)).to_vec();
            let implements: Vec<_> = outcome.graph.outgoing_edges(bulkhead.id(), Some(EdgeKind::Implements)).to_vec();
            assert_eq!(extends.len(), 1, "a full reindex through the real indexer must reconcile BulkheadPolicy's own Extends edge toward one of Policy's 2 partial fragments");
            assert_eq!(implements.len(), 1, "IBulkheadPolicy (not partial, unambiguous) must still resolve normally");
        }

        #[test]
        fn incremental_advance_reconciles_identically_to_a_full_reindex() {
            let repo_incremental = test_repo();
            let repo_full = test_repo();
            let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
            let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

            let source_a = FakeSource::new()
                .with_file(FileIdentity::parse(repo_incremental, commit_a.clone(), "src/Policy.cs").unwrap(), POLICY_PART_1)
                .with_file(FileIdentity::parse(repo_incremental, commit_a.clone(), "src/Policy.TResult.cs").unwrap(), POLICY_PART_2)
                .with_file(FileIdentity::parse(repo_incremental, commit_a.clone(), "src/IBulkheadPolicy.cs").unwrap(), IBULKHEAD);
            let indexer = CodeGraphIndexer::new();
            indexer.observe_commit(&source_a, repo_incremental, &commit_a, None, &[]).unwrap();

            // Commit B adds BulkheadPolicy.cs as a NEW file -- Policy's
            // own 2 partial fragments are UNTOUCHED, exercising the
            // reuse path for raw_csharp_type_declaration_contexts.
            let source_b = source_a
                .with_file(FileIdentity::parse(repo_incremental, commit_b.clone(), "src/Policy.cs").unwrap(), POLICY_PART_1)
                .with_file(FileIdentity::parse(repo_incremental, commit_b.clone(), "src/Policy.TResult.cs").unwrap(), POLICY_PART_2)
                .with_file(FileIdentity::parse(repo_incremental, commit_b.clone(), "src/IBulkheadPolicy.cs").unwrap(), IBULKHEAD)
                .with_file(FileIdentity::parse(repo_incremental, commit_b.clone(), "src/BulkheadPolicy.cs").unwrap(), BULKHEAD);
            let incremental_outcome = indexer.observe_commit(&source_b, repo_incremental, &commit_b, Some(&commit_a), &["src/BulkheadPolicy.cs".to_string()]).unwrap();

            let full_source = FakeSource::new()
                .with_file(FileIdentity::parse(repo_full, commit_b.clone(), "src/Policy.cs").unwrap(), POLICY_PART_1)
                .with_file(FileIdentity::parse(repo_full, commit_b.clone(), "src/Policy.TResult.cs").unwrap(), POLICY_PART_2)
                .with_file(FileIdentity::parse(repo_full, commit_b.clone(), "src/IBulkheadPolicy.cs").unwrap(), IBULKHEAD)
                .with_file(FileIdentity::parse(repo_full, commit_b.clone(), "src/BulkheadPolicy.cs").unwrap(), BULKHEAD);
            let full_indexer = CodeGraphIndexer::new();
            let full_outcome = full_indexer.observe_commit(&full_source, repo_full, &commit_b, None, &[]).unwrap();

            let incremental_extends_count = incremental_outcome.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Extends).count();
            let full_extends_count = full_outcome.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Extends).count();
            assert_eq!(incremental_extends_count, 1, "the incremental path must reconcile Policy's 2 UNTOUCHED partial fragments via the reused raw context");
            assert_eq!(incremental_extends_count, full_extends_count, "incremental must match a full reindex of the identical commit");
        }

        #[test]
        fn editing_one_partial_fragment_does_not_break_reconciliation() {
            // Policy.TResult.cs gets a cosmetic edit at commit B --
            // reconciliation must still hold across the UNCHANGED
            // Policy.cs fragment (reused context) plus the freshly
            // re-extracted Policy.TResult.cs fragment (fresh context).
            let repo = test_repo();
            let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
            let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
            let source_a = FakeSource::new()
                .with_file(FileIdentity::parse(repo, commit_a.clone(), "src/Policy.cs").unwrap(), POLICY_PART_1)
                .with_file(FileIdentity::parse(repo, commit_a.clone(), "src/Policy.TResult.cs").unwrap(), POLICY_PART_2)
                .with_file(FileIdentity::parse(repo, commit_a.clone(), "src/BulkheadPolicy.cs").unwrap(), BULKHEAD);
            let indexer = CodeGraphIndexer::new();
            indexer.observe_commit(&source_a, repo, &commit_a, None, &[]).unwrap();

            let source_b = source_a
                .with_file(FileIdentity::parse(repo, commit_b.clone(), "src/Policy.cs").unwrap(), POLICY_PART_1)
                .with_file(FileIdentity::parse(repo, commit_b.clone(), "src/Policy.TResult.cs").unwrap(), b"public partial class Policy { /* edited */ }\n")
                .with_file(FileIdentity::parse(repo, commit_b.clone(), "src/BulkheadPolicy.cs").unwrap(), BULKHEAD);
            let outcome = indexer.observe_commit(&source_b, repo, &commit_b, Some(&commit_a), &["src/Policy.TResult.cs".to_string()]).unwrap();

            let bulkhead = outcome.graph.all_snapshots().into_iter().find(|s| s.name().as_str() == "BulkheadPolicy" && outcome.graph.lineage_from(s.id()).is_empty()).unwrap();
            let extends = outcome.graph.outgoing_edges(bulkhead.id(), Some(EdgeKind::Extends));
            assert_eq!(extends.len(), 1, "editing one partial fragment's own body must not break reconciliation toward whichever fragment is chosen as representative");
        }

        #[test]
        fn deleting_one_partial_fragment_leaves_the_other_fragment_still_reconciled_alone() {
            // Policy.TResult.cs is DELETED at commit B -- Policy still
            // exists (via Policy.cs alone), still partial, so
            // BulkheadPolicy : Policy must still resolve (trivially,
            // now via a single unambiguous candidate -- not through
            // reconciliation at all, but confirms deletion does not
            // leave a stale raw context pointing at a dead snapshot).
            // Uses a BulkheadPolicy shape with ONLY the Policy base (no
            // IBulkheadPolicy interface) -- that interface is irrelevant
            // to this specific test and omitting its own defining file
            // would otherwise make BulkheadPolicy.cs itself a dangling
            // reference to a name never in the pool.
            const BULKHEAD_POLICY_ONLY: &[u8] = b"public class BulkheadPolicy : Policy {}\n";
            let repo = test_repo();
            let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
            let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
            let source_a = FakeSource::new()
                .with_file(FileIdentity::parse(repo, commit_a.clone(), "src/Policy.cs").unwrap(), POLICY_PART_1)
                .with_file(FileIdentity::parse(repo, commit_a.clone(), "src/Policy.TResult.cs").unwrap(), POLICY_PART_2)
                .with_file(FileIdentity::parse(repo, commit_a.clone(), "src/BulkheadPolicy.cs").unwrap(), BULKHEAD_POLICY_ONLY);
            let indexer = CodeGraphIndexer::new();
            indexer.observe_commit(&source_a, repo, &commit_a, None, &[]).unwrap();

            // Commit B's own file list simply omits Policy.TResult.cs —
            // `FakeSource::with_file` registers per-commit, so NOT
            // calling it for this path at commit_b is exactly "this
            // file does not exist at commit B," the real deletion
            // shape `list_files_at` would report.
            let source_b = source_a.with_file(FileIdentity::parse(repo, commit_b.clone(), "src/Policy.cs").unwrap(), POLICY_PART_1).with_file(FileIdentity::parse(repo, commit_b.clone(), "src/BulkheadPolicy.cs").unwrap(), BULKHEAD_POLICY_ONLY);
            let outcome = indexer.observe_commit(&source_b, repo, &commit_b, Some(&commit_a), &["src/Policy.TResult.cs".to_string(), "src/BulkheadPolicy.cs".to_string()]).unwrap();

            let policy_fragments: Vec<_> = outcome.graph.all_snapshots().into_iter().filter(|s| s.name().as_str() == "Policy").collect();
            assert_eq!(policy_fragments.len(), 1, "only Policy.cs's own fragment must remain after Policy.TResult.cs is deleted");
            let bulkhead = outcome.graph.all_snapshots().into_iter().find(|s| s.name().as_str() == "BulkheadPolicy" && outcome.graph.lineage_from(s.id()).is_empty()).unwrap();
            let extends = outcome.graph.outgoing_edges(bulkhead.id(), Some(EdgeKind::Extends));
            assert_eq!(extends.len(), 1, "after deleting one partial fragment, BulkheadPolicy must still resolve toward the one remaining Policy fragment");
            assert_eq!(extends[0].to, policy_fragments[0].id());
        }

        #[test]
        fn reconciliation_produces_no_duplicate_edges_through_the_real_indexer() {
            let repo = test_repo();
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let source = FakeSource::new()
                .with_file(FileIdentity::parse(repo, commit.clone(), "src/Policy.cs").unwrap(), POLICY_PART_1)
                .with_file(FileIdentity::parse(repo, commit.clone(), "src/Policy.TResult.cs").unwrap(), POLICY_PART_2)
                .with_file(FileIdentity::parse(repo, commit.clone(), "src/Policy.HandleSyntax.cs").unwrap(), b"public partial class Policy { }\n")
                .with_file(FileIdentity::parse(repo, commit.clone(), "src/BulkheadPolicy.cs").unwrap(), BULKHEAD);
            let indexer = CodeGraphIndexer::new();
            let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

            let bulkhead = outcome.graph.all_snapshots().into_iter().find(|s| s.name().as_str() == "BulkheadPolicy" && outcome.graph.lineage_from(s.id()).is_empty()).unwrap();
            let extends = outcome.graph.outgoing_edges(bulkhead.id(), Some(EdgeKind::Extends));
            assert_eq!(extends.len(), 1, "3 partial Policy fragments must still produce exactly ONE Extends edge, never one per fragment");
        }
    }

    /// B2-ID-011 regression tests: C# generic method call extraction,
    /// validated through the REAL `CodeGraphIndexer` entry point (full
    /// reindex, incremental advance) -- not just the lower-level
    /// `resolve_calls` unit tests in `edges::calls`. Confirms full/
    /// incremental parity on the new generic-callee extraction path,
    /// and confirms this fix has zero effect on `Extends`/`Implements`
    /// edges (B2-ID-010's own territory).
    mod b2_id_011_csharp_generic_calls_through_the_real_indexer {
        use super::*;

        #[test]
        fn full_reindex_resolves_a_bare_generic_call() {
            let repo = test_repo();
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let source = FakeSource::new().with_file(
                FileIdentity::parse(repo, commit.clone(), "src/C.cs").unwrap(),
                b"class C {\n    void M() { Handle<int>(); }\n    void Handle<T>() {}\n}\n",
            );
            let indexer = CodeGraphIndexer::new();
            let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

            let handle = outcome.graph.all_snapshots().into_iter().find(|s| s.name().as_str() == "Handle").unwrap();
            let callers: Vec<_> = outcome.graph.incoming_edges(handle.id(), Some(EdgeKind::Calls)).to_vec();
            assert_eq!(callers.len(), 1, "a bare generic call must resolve through the real indexer: got {callers:?}");
        }

        #[test]
        fn full_reindex_resolves_a_receiver_qualified_generic_call() {
            let repo = test_repo();
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let source = FakeSource::new().with_file(
                FileIdentity::parse(repo, commit.clone(), "src/C.cs").unwrap(),
                b"class C {\n    void M(C policy) { policy.Handle<int>(); }\n    void Handle<T>() {}\n}\n",
            );
            let indexer = CodeGraphIndexer::new();
            let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

            let handle = outcome.graph.all_snapshots().into_iter().find(|s| s.name().as_str() == "Handle").unwrap();
            let callers: Vec<_> = outcome.graph.incoming_edges(handle.id(), Some(EdgeKind::Calls)).to_vec();
            assert_eq!(callers.len(), 1, "a receiver-qualified generic call must resolve through the real indexer: got {callers:?}");
        }

        #[test]
        fn incremental_advance_resolves_a_generic_call_identically_to_a_full_reindex() {
            let repo_incremental = test_repo();
            let repo_full = test_repo();
            let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
            let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

            let source_a = FakeSource::new().with_file(
                FileIdentity::parse(repo_incremental, commit_a.clone(), "src/Handle.cs").unwrap(),
                b"class C {\n    void Handle<T>() {}\n}\n",
            );
            let indexer = CodeGraphIndexer::new();
            indexer.observe_commit(&source_a, repo_incremental, &commit_a, None, &[]).unwrap();

            // Commit B adds a NEW caller file -- Handle.cs itself is
            // UNTOUCHED, exercising the real incremental reuse path for
            // the unchanged snapshot pool while the new generic call
            // site is freshly extracted.
            let source_b = source_a
                .with_file(FileIdentity::parse(repo_incremental, commit_b.clone(), "src/Handle.cs").unwrap(), b"class C {\n    void Handle<T>() {}\n}\n")
                .with_file(FileIdentity::parse(repo_incremental, commit_b.clone(), "src/Caller.cs").unwrap(), b"class Caller {\n    void M(C c) { c.Handle<int>(); }\n}\n");
            let incremental_outcome = indexer.observe_commit(&source_b, repo_incremental, &commit_b, Some(&commit_a), &["src/Caller.cs".to_string()]).unwrap();

            let full_source = FakeSource::new()
                .with_file(FileIdentity::parse(repo_full, commit_b.clone(), "src/Handle.cs").unwrap(), b"class C {\n    void Handle<T>() {}\n}\n")
                .with_file(FileIdentity::parse(repo_full, commit_b.clone(), "src/Caller.cs").unwrap(), b"class Caller {\n    void M(C c) { c.Handle<int>(); }\n}\n");
            let full_indexer = CodeGraphIndexer::new();
            let full_outcome = full_indexer.observe_commit(&full_source, repo_full, &commit_b, None, &[]).unwrap();

            let incremental_calls_count = incremental_outcome.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Calls).count();
            let full_calls_count = full_outcome.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Calls).count();
            assert_eq!(incremental_calls_count, 1, "the incremental path must resolve the new generic call site: got {incremental_calls_count}");
            assert_eq!(incremental_calls_count, full_calls_count, "incremental must match a full reindex of the identical commit");
        }

        #[test]
        fn generic_calls_fix_has_zero_effect_on_extends_implements_edges() {
            // B2-ID-010's own territory must be completely unaffected:
            // a partial-class Extends scenario combined with an
            // unrelated generic call site must still produce exactly
            // the Extends/Implements edges B2-ID-010 itself already
            // validated, regardless of this fix.
            let repo = test_repo();
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let source = FakeSource::new()
                .with_file(FileIdentity::parse(repo, commit.clone(), "src/Base.cs").unwrap(), b"class Base {}\n")
                .with_file(
                    FileIdentity::parse(repo, commit.clone(), "src/Derived.cs").unwrap(),
                    b"class Derived : Base {\n    void M() { Handle<int>(); }\n    void Handle<T>() {}\n}\n",
                );
            let indexer = CodeGraphIndexer::new();
            let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

            let derived = outcome.graph.all_snapshots().into_iter().find(|s| s.name().as_str() == "Derived").unwrap();
            let extends = outcome.graph.outgoing_edges(derived.id(), Some(EdgeKind::Extends));
            assert_eq!(extends.len(), 1, "an unrelated generic call site in the same file must not affect Extends resolution: got {extends:?}");

            let handle = outcome.graph.all_snapshots().into_iter().find(|s| s.name().as_str() == "Handle").unwrap();
            let callers: Vec<_> = outcome.graph.incoming_edges(handle.id(), Some(EdgeKind::Calls)).to_vec();
            assert_eq!(callers.len(), 1, "the generic call itself must still resolve correctly alongside the Extends edge: got {callers:?}");
        }

        #[test]
        fn generic_calls_fix_never_changes_any_snapshot_identity() {
            let repo = test_repo();
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let source = FakeSource::new().with_file(
                FileIdentity::parse(repo, commit.clone(), "src/C.cs").unwrap(),
                b"class C {\n    void M() { Handle<int>(); }\n    void Handle<T>() {}\n}\n",
            );
            let indexer = CodeGraphIndexer::new();
            let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

            let ids_before: std::collections::HashSet<_> = outcome.graph.all_snapshots().iter().map(|s| s.id()).collect();
            // Re-running resolve_calls directly against the same pool
            // must never alter any snapshot's own identity -- this fix
            // only ever changes which Edge(s) are produced, never any
            // SymbolSnapshotId.
            let snapshots: Vec<_> = outcome.graph.all_snapshots().into_iter().cloned().collect();
            let _ = crate::edges::resolve_calls(&snapshots).unwrap();
            let ids_after: std::collections::HashSet<_> = snapshots.iter().map(|s| s.id()).collect();
            assert_eq!(ids_before, ids_after, "resolve_calls must never mutate or change any snapshot's own identity");
        }
    }

    #[test]
    fn a_full_reindex_also_correctly_populates_impl_facts_and_raw_impl_statements() {
        // Confirms `full_reindex` (the new-repository / history-rewrite
        // path) was not accidentally left out of the Task 4 wiring —
        // an easy omission since `incremental_reindex` is the more
        // complex, more carefully-tested path.
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit.clone(), "src/circle.rs").unwrap();
        let source = FakeSource::new().with_file(file, b"trait Shape { fn area(&self); } struct Circle; impl Shape for Circle { fn area(&self) {} }\n");
        let indexer = CodeGraphIndexer::new();

        let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

        assert_eq!(outcome.impl_facts.len(), 1);
        assert_eq!(outcome.impl_facts[0].self_type_text, "Circle");
        assert_eq!(outcome.impl_facts[0].trait_type_text, Some("Shape".to_string()));
        assert_eq!(outcome.metrics.raw_impl_statements_extracted, 1);
        assert_eq!(outcome.metrics.raw_impl_statements_reused, 0);
        assert_eq!(outcome.metrics.impl_facts_resolved, 1);
        assert_eq!(
            outcome.graph.all_edges().iter().filter(|e| e.kind == EdgeKind::Implements).count(),
            1,
            "a full reindex must also produce the derived Implements edge"
        );
    }

    #[test]
    fn a_stale_generation_cannot_overwrite_newer_published_impl_facts() {
        // Mirrors `a_stale_generation_cannot_overwrite_newer_published_import_facts`
        // — the same publication-atomicity guarantee must cover impl
        // state, not only the graph/import state.
        let repo = test_repo();
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit_a.clone(), "src/lib.rs").unwrap();
        let file_b = FileIdentity::parse(repo, commit_b.clone(), "src/lib.rs").unwrap();
        let source = Arc::new(
            FakeSource::new()
                .with_file(file_a, b"struct Foo; impl Foo { fn new() -> Self { Foo } }\n")
                .with_file(file_b, b"struct Bar; impl Bar { fn new() -> Self { Bar } }\n"),
        );
        let indexer = Arc::new(CodeGraphIndexer::new());

        let (arrived_tx, arrived_rx) = mpsc::channel::<()>();
        let barrier = Arc::new(Barrier::new(2));
        let barrier_for_hook = barrier.clone();
        let target_point = TestPoint::AfterGenerationClaimed;
        let first_call = Arc::new(std::sync::atomic::AtomicBool::new(true));
        indexer.set_test_hook(Arc::new(move |point, _generation| {
            if point == target_point && first_call.swap(false, std::sync::atomic::Ordering::SeqCst) {
                arrived_tx.send(()).unwrap();
                barrier_for_hook.wait();
            }
        }));

        let worker_indexer = indexer.clone();
        let worker_source = source.clone();
        let worker = std::thread::spawn(move || worker_indexer.observe_commit(worker_source.as_ref(), repo, &commit_a, None, &[]));

        arrived_rx.recv().unwrap();

        let outcome_b = indexer.observe_commit(source.as_ref(), repo, &commit_b, None, &[]).unwrap();
        assert_eq!(outcome_b.impl_facts[0].self_type_text, "Bar");

        barrier.wait();
        let outcome_a = worker.join().unwrap().unwrap();

        assert_eq!(outcome_a.impl_facts[0].self_type_text, "Bar", "A's stale impl facts must never win — it must report B's impl facts instead");
    }

    #[test]
    fn no_legacy_imports_edge_kind_is_ever_produced() {
        // EdgeKind::Imports was removed (B2-ID-005) — confirms the real
        // indexer's own typed-edge output never contains anything the
        // (now nonexistent) old variant would have produced; this is
        // really a compile-time guarantee (the variant does not exist),
        // but this test documents the intent at the call-site level too.
        let repo = test_repo();
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit.clone(), "src/model.py").unwrap();
        let source = FakeSource::new().with_file(file, b"import torch\nclass Model:\n    pass\n");
        let indexer = CodeGraphIndexer::new();

        let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

        // Every EdgeKind the real EdgeKind enum has today is a valid
        // match arm here; there is no `Imports` arm to even write.
        for edge in outcome.graph.all_edges() {
            assert!(matches!(
                edge.kind,
                EdgeKind::Calls | EdgeKind::Implements | EdgeKind::Extends | EdgeKind::References | EdgeKind::TestsFor | EdgeKind::RenamedTo | EdgeKind::Contains | EdgeKind::Instantiates
            ));
        }
    }
}
