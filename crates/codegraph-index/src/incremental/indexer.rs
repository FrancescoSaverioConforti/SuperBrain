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
use crate::graph::{CodeGraph, GraphError};
use crate::snapshot::SymbolSnapshot;

/// Internal, per-repository bookkeeping the indexer keeps alongside
/// the published graph: which commit that graph reflects, so the next
/// `observe_commit` call for the same repository can classify its own
/// relationship to it (see [`classify_commit`]).
#[derive(Debug)]
struct RepositoryState {
    commit: CommitHash,
    graph: Arc<CodeGraph>,
    generation: u64,
}

/// What [`CodeGraphIndexer::begin_observe`] (critical section 1)
/// claims and hands to the caller for the lock-free rebuild that
/// follows: the commit relationship it classified, the prior graph (if
/// any) to build an incremental update against, and the generation
/// number this call has exclusively claimed.
struct ObserveClaim {
    relationship: CommitRelationship,
    prior_graph: Option<Arc<CodeGraph>>,
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
                metrics: IndexMetrics::default(),
            });
        }

        let (new_graph, metrics) = match claim.relationship {
            CommitRelationship::NewRepository | CommitRelationship::HistoryRewriteOrUnknownAncestor => self.full_reindex(source, repository, new_commit)?,
            CommitRelationship::NormalAdvance => {
                let prior_graph = claim.prior_graph.expect("NormalAdvance implies prior state exists");
                self.incremental_reindex(source, repository, new_commit, &prior_graph, changed_files)?
            }
            CommitRelationship::AlreadyObserved => unreachable!("handled above"),
        };

        Ok(self.publish(repository, new_commit.clone(), claim.generation, new_graph, metrics))
    }

    /// Critical section 1: classify this call's relationship to the
    /// repository's current state and claim the next generation number
    /// — the ONLY work done under the lock before the (lock-free)
    /// rebuild. Returns the prior graph (a cloned `Arc`, cheap) so the
    /// caller can build the incremental update outside the lock.
    fn begin_observe(&self, repository: RepositoryId, new_commit: &CommitHash, expected_prev_commit: Option<&CommitHash>) -> ObserveClaim {
        let (relationship, prior_graph) = {
            let state = self.state.lock().unwrap();
            let prior = state.get(&repository);
            let relationship = classify_commit(prior, new_commit, expected_prev_commit);
            (relationship, prior.map(|s| s.graph.clone()))
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
        ObserveClaim { relationship, prior_graph, generation }
    }

    fn full_reindex(&self, source: &dyn RepositorySource, repository: RepositoryId, commit: &CommitHash) -> Result<(CodeGraph, IndexMetrics), ObserveError> {
        let files = source.list_files_at(repository, commit).map_err(|e| ObserveError::Pipeline(PipelineError::Source(e)))?;
        let no_known_lineage = |_: SymbolSnapshotId| None;
        let output = index_files(source, repository, commit, &files, &[], &no_known_lineage)?;
        let metrics = IndexMetrics {
            files_considered: files.len(),
            files_reparsed: files.len(),
            snapshots_removed: 0,
            snapshots_added: output.snapshots.len(),
            edges_rebuilt: output.lineage_edges.len() + output.typed_edges.len(),
        };
        let graph = build_graph(output.snapshots, output.lineage_edges, output.typed_edges)?;
        Ok((graph, metrics))
    }

    /// Incremental update: re-parses/re-extracts/re-promotes ONLY
    /// `changed_files`, keeping every other file's snapshots from
    /// `prior_graph` untouched. Lineage and typed edges are
    /// RECOMPUTED FROM THE WHOLE RESULTING SNAPSHOT POOL (untouched +
    /// freshly rebuilt) rather than only from the changed snapshots —
    /// a deliberate, documented scope choice, not an oversight:
    /// `resolve_calls`/`resolve_imports`/etc. resolve a callee NAME
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
    fn incremental_reindex(
        &self,
        source: &dyn RepositorySource,
        repository: RepositoryId,
        new_commit: &CommitHash,
        prior_graph: &CodeGraph,
        changed_files: &[String],
    ) -> Result<(CodeGraph, IndexMetrics), ObserveError> {
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

        let mut typed_edges = Vec::new();
        typed_edges.extend(crate::edges::resolve_calls(&current_snapshots).map_err(PipelineError::from)?);
        typed_edges.extend(crate::edges::resolve_imports(&current_snapshots).map_err(PipelineError::from)?);
        typed_edges.extend(crate::edges::resolve_extends_implements(&current_snapshots).map_err(PipelineError::from)?);
        typed_edges.extend(crate::edges::resolve_tests_for(&current_snapshots).map_err(PipelineError::from)?);

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
        let mut final_snapshots = current_snapshots;
        let lineage_sources: Vec<SymbolSnapshot> = lineage_edges.iter().filter_map(|edge| prior_graph.snapshot(edge.from).cloned()).collect();
        for source_snapshot in lineage_sources {
            if !final_snapshots.iter().any(|s| s.id() == source_snapshot.id()) {
                final_snapshots.push(source_snapshot);
            }
        }

        let metrics = IndexMetrics {
            files_considered: changed_files.len(),
            files_reparsed: files_to_reparse.len(),
            snapshots_removed: prior_changed_snapshots.len(),
            snapshots_added: output.snapshots.len(),
            edges_rebuilt: lineage_edges.len() + typed_edges.len(),
        };

        let graph = build_graph(final_snapshots, lineage_edges, typed_edges)?;
        Ok((graph, metrics))
    }

    /// Critical section 2: publish `candidate_graph` for `repository`
    /// as `generation`, UNLESS a later generation has already been
    /// published for this repository in the meantime (see this
    /// module's doc comment) — in which case this call's own work is
    /// discarded and the actually-current state is returned instead.
    fn publish(&self, repository: RepositoryId, commit: CommitHash, generation: u64, candidate_graph: CodeGraph, metrics: IndexMetrics) -> ObserveOutcome {
        let mut state = self.state.lock().unwrap();
        let current_generation = state.get(&repository).map(|s| s.generation).unwrap_or(0);
        if generation > current_generation {
            let graph = Arc::new(candidate_graph);
            state.insert(repository, RepositoryState { commit, graph: graph.clone(), generation });
            ObserveOutcome { generation, graph, metrics }
        } else {
            // A later generation already published — this call's
            // candidate is discarded. Return the ACTUAL current state,
            // not this call's own (stale, never-published) graph.
            let existing = state.get(&repository).expect("current_generation > 0 implies prior state exists when generation <= current_generation");
            ObserveOutcome {
                generation: existing.generation,
                graph: existing.graph.clone(),
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

        let first_outcome = indexer.publish(repo, commit.clone(), 5, CodeGraph::new(), IndexMetrics::default());
        assert_eq!(first_outcome.generation, 5);

        // Attempt to publish AGAIN at the SAME generation (5, not 4) —
        // must be rejected exactly like a strictly older one would be.
        let mut second_candidate = CodeGraph::new();
        let file = FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap();
        let candidate = RawSymbolCandidate::new("marker".to_string(), "function_item".to_string(), ByteSpan { start: 0, end: 3 }, "fn marker() {}".to_string()).unwrap();
        second_candidate.insert_snapshot(promote(&candidate, file).unwrap()).unwrap();
        let second_outcome = indexer.publish(repo, commit, 5, second_candidate, IndexMetrics::default());

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
        indexer.set_test_hook(Arc::new(move |point, generation| {
            if point == target_point && generation == 1 {
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
}
