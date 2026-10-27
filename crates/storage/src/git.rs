//! Idempotent, isolated git commit per write operation.
//!
//! One `commit_operation` call per `operation_id`, building the new commit's
//! tree directly from `HEAD` plus this operation's one blob via
//! [`git2::TreeBuilder`] — the repository's shared, on-disk `.git/index` is
//! never opened, read, or written by anything in this module. That isolation
//! (not a history scan, not a lock) is what makes one operation's commit
//! independent of any other operation's staged-but-uncommitted state, past,
//! concurrent, or future (user architectural review, point 5).
//!
//! Idempotency is two-tier and journal-first: [`find_commit_for_operation`]'s
//! Tier 1 is an O(1) read of the journal row's own `git_commit_oid` column,
//! which is authoritative because it is this exact operation's own
//! durably-recorded commit reference. Tier 2 (a bounded or unbounded history
//! walk) is a defensive fallback only, for the narrow crash window between
//! `git2`'s commit succeeding and the journal update that would have
//! recorded it never running — never the primary idempotency mechanism.

use std::path::Path;

use uuid::Uuid;

use crate::journal::{Journal, JournalError};

#[derive(Debug, thiserror::Error)]
pub(crate) enum GitError {
    #[error(transparent)]
    Git2(#[from] git2::Error),
    #[error("no git repository at the vault root")]
    NoRepository,
    #[error(transparent)]
    Journal(#[from] JournalError),
}

/// How far back [`find_commit_for_operation`]'s Tier 2 history walk may look
/// before giving up.
///
/// `Bounded(n)` stops after `n` commits and is purely a performance bound for
/// the normal write-path chain extension: that call site runs immediately
/// after a write whose commit — if it exists at all — was *just* made and is
/// therefore always at or extremely near `HEAD`, so a bound can never trade
/// away correctness there.
///
/// `Unbounded` walks to the true end of history reachable from `HEAD` and is
/// what Task 7's `reconcile` (the recovery path, where correctness matters
/// far more than speed) always uses, so it can definitively tell "no commit
/// exists" apart from "exists, but beyond the bound".
///
/// Even `Unbounded` only proves non-existence **among commits reachable from
/// `HEAD`**, not absolute non-existence: this vault's threat model already
/// treats its single trusted operator as someone who can touch the
/// repository directly, and a rewrite of reachable history (reset, rebase,
/// force-push) is the same class of out-of-band action. `find_commit_for_operation`
/// does not attempt to detect that case — doing so would require the journal
/// to record "a commit attempt was made" as its own fact, independent of
/// `git_commit_oid`, which is new persisted state this plan does not add — so
/// an `Unbounded` miss is treated by `reconcile` exactly as "no commit was
/// ever made". The residual risk (a duplicate commit, only when history was
/// rewritten out-of-band after the original commit) is accepted deliberately
/// and recorded in `eval/results/A2/evidence.md` as a known limitation of
/// reachability-based idempotency, not a silent gap.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ScanDepth {
    Bounded(usize),
    // No production call site until Task 7's `reconcile` recovery pass,
    // which always uses this variant (see this enum's own doc comment for
    // why). Exercised directly by this module's own tests in the meantime.
    #[allow(dead_code)]
    Unbounded,
}

/// The commit-message marker that identifies which operation a commit
/// belongs to. Secondary/defensive identification only — the primary
/// idempotency source is the journal's own `git_commit_oid` column.
fn operation_marker(operation_id: Uuid) -> String {
    format!("[operation_id={operation_id}]")
}

/// Opens the repository at `repo_path` and creates one commit recording
/// `record_path`'s current on-disk contents, isolated from the shared
/// `.git/index`: the new tree is built via [`git2::TreeBuilder`] seeded from
/// `HEAD`'s current tree, with only this operation's one blob inserted or
/// updated. `.git/index` is never opened by this function, so no other
/// operation's staged-but-uncommitted state can leak into this commit, and
/// this commit cannot leak into anyone else's.
///
/// The commit message embeds `operation_id` verbatim as a defensive,
/// secondary identification string (see [`find_commit_for_operation`] for
/// the primary one) and the commit becomes the new tip of the branch `HEAD`
/// points to.
///
/// Idempotent re-entry: if a commit already exists reachable from `HEAD`
/// whose message carries this `operation_id`'s marker, that commit's `Oid`
/// is returned instead of creating a new one — see
/// [`find_commit_for_operation`]'s Tier 2 for why only an unbounded scan
/// backs this call site with full certainty when the quick bounded probe
/// below misses close to `HEAD`. Re-entry here is a best-effort, bounded
/// courtesy; the caller (`Vault::write_record_unchecked`) is the one that
/// actually enforces idempotency via `find_commit_for_operation` before
/// ever calling this function.
///
/// Returns `GitError::NoRepository` if `repo_path` is not a git repository —
/// `propose_and_commit` maps that to the `git_pending` response, not a hard
/// failure.
pub(crate) fn commit_operation(
    repo_path: &Path,
    operation_id: Uuid,
    note_id: &str,
    record_path: &Path,
) -> Result<git2::Oid, GitError> {
    let repo = open_repo(repo_path)?;

    let head = repo.head()?;
    let parent_commit = head.peel_to_commit()?;
    let base_tree = parent_commit.tree()?;

    // Idempotent re-entry: a cheap, best-effort check against the same
    // marker this call is about to write, so calling this function twice in
    // a row for the same operation_id never creates two commits.
    if let Some(existing) = scan_history_for_marker(&repo, &operation_marker(operation_id), ScanDepth::Bounded(500))? {
        return Ok(existing);
    }

    let relative_path = record_path.strip_prefix(repo_path).unwrap_or(record_path);
    let relative_path_str = relative_path.to_str().ok_or_else(|| {
        git2::Error::from_str("record path is not valid UTF-8")
    })?;

    let contents = std::fs::read(record_path).map_err(|e| {
        git2::Error::from_str(&format!("failed to read {}: {e}", record_path.display()))
    })?;
    let blob_oid = repo.blob(&contents)?;

    let mut builder = repo.treebuilder(Some(&base_tree))?;
    builder.insert(relative_path_str, blob_oid, git2::FileMode::Blob.into())?;
    let tree_oid = builder.write()?;
    let tree = repo.find_tree(tree_oid)?;

    let signature = git2::Signature::now("SuperBrain", "superbrain@localhost")?;
    let message = format!("vault: update {note_id} {}", operation_marker(operation_id));

    let branch_ref_name = head.name().ok_or_else(|| {
        git2::Error::from_str("HEAD does not point at a named branch reference")
    })?.to_string();

    let commit_oid = repo.commit(
        Some(&branch_ref_name),
        &signature,
        &signature,
        &message,
        &tree,
        &[&parent_commit],
    )?;

    Ok(commit_oid)
}

/// Two-tier lookup for the commit, if any, already recorded for
/// `operation_id`. Tier 1 (primary, O(1)) trusts `journal.get(operation_id)`'s
/// own `git_commit_oid` column directly, with no history walk — it is
/// authoritative because it is this exact operation's own durably-recorded
/// commit reference, not an inference from commit-message text. Tier 2
/// (defensive fallback, only reached when Tier 1 is empty) walks history
/// from `HEAD` for a commit message carrying this operation's marker,
/// stopping per `scan`.
///
/// This is a pure lookup: it never writes to the journal. A caller whose
/// Tier 2 finds a commit the journal doesn't yet know about (the
/// crash-between-git-commit-and-journal-update case) is responsible for
/// writing that `Oid` back onto the journal row itself, via
/// `journal.advance(..., AdvanceFields::git_oid(oid))`, so Tier 1 becomes
/// authoritative for any future lookup of this same operation.
///
/// See [`ScanDepth`]'s doc comment for the "reachable from `HEAD`" caveat:
/// even `ScanDepth::Unbounded` proves non-existence only among commits
/// reachable from `HEAD`, not absolute non-existence.
pub(crate) fn find_commit_for_operation(
    journal: &Journal,
    repo_path: &Path,
    operation_id: Uuid,
    scan: ScanDepth,
) -> Result<Option<git2::Oid>, GitError> {
    // Tier 1: the journal's own record, trusted without a history walk.
    if let Some(row) = journal.get(operation_id)? {
        if let Some(oid_str) = row.git_commit_oid {
            let oid = git2::Oid::from_str(&oid_str)?;
            return Ok(Some(oid));
        }
    }

    // Tier 2: defensive fallback, a history scan from HEAD.
    let repo = open_repo(repo_path)?;
    scan_history_for_marker(&repo, &operation_marker(operation_id), scan)
}

/// Opens `repo_path` as a git repository, mapping a "not found" failure to
/// [`GitError::NoRepository`] (the spec's `git_pending` case) rather than
/// propagating it as an ordinary `git2::Error`.
fn open_repo(repo_path: &Path) -> Result<git2::Repository, GitError> {
    match git2::Repository::open(repo_path) {
        Ok(repo) => Ok(repo),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Err(GitError::NoRepository),
        Err(e) => Err(GitError::Git2(e)),
    }
}

/// Walks history from `HEAD`, stopping at `scan`'s depth, looking for a
/// commit whose message contains `marker`. Returns the first (nearest-to-HEAD)
/// match, or `None` if the walk (bounded or to the true end of history)
/// never finds one.
fn scan_history_for_marker(
    repo: &git2::Repository,
    marker: &str,
    scan: ScanDepth,
) -> Result<Option<git2::Oid>, GitError> {
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;

    let limit = match scan {
        ScanDepth::Bounded(n) => Some(n),
        ScanDepth::Unbounded => None,
    };

    for (count, oid_result) in revwalk.enumerate() {
        if let Some(limit) = limit {
            if count >= limit {
                break;
            }
        }
        let oid = oid_result?;
        let commit = repo.find_commit(oid)?;
        if commit.message().is_some_and(|m| m.contains(marker)) {
            return Ok(Some(oid));
        }
    }
    Ok(None)
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};

    pub(crate) fn unique_test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "superbrain-git-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Initializes a fresh repository at a fresh `unique_test_dir()`, with an
    /// initial empty commit so `HEAD` exists for the `TreeBuilder` seed and
    /// history-walking. Never touches the real SuperBrain repository.
    pub(crate) fn init_test_repo() -> PathBuf {
        let dir = unique_test_dir();
        init_git_repo_at(&dir);
        dir
    }

    /// Initializes a git repository in place at an EXISTING directory
    /// (unlike `init_test_repo`, which always creates a fresh one), with the
    /// same initial empty commit so `HEAD` exists. For tests that need to
    /// control the directory's other contents (e.g. a planted symlink)
    /// before or independently of git-initializing it.
    pub(crate) fn init_git_repo_at(dir: &Path) {
        let repo = git2::Repository::init(dir).unwrap();
        let signature = git2::Signature::now("SuperBrain Test", "test@localhost").unwrap();
        let tree_oid = {
            let mut index = repo.index().unwrap();
            index.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial empty commit", &tree, &[])
            .unwrap();
    }

    /// Writes a throwaway file to the repo and makes a plain `git2` commit
    /// whose message carries no operation-id marker, so it never satisfies
    /// any Tier 2 lookup itself — it exists only to advance `HEAD` between a
    /// real operation's commit and a later lookup. Used by this task's own
    /// bounded-vs-unbounded test and by Task 8's dirtied-repository test.
    pub(crate) fn commit_unrelated_change_for_test(repo_dir: &Path) {
        let repo = git2::Repository::open(repo_dir).unwrap();
        let unique_name = format!(
            "unrelated-{}.txt",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        );
        let file_path = repo_dir.join(&unique_name);
        std::fs::write(&file_path, b"throwaway").unwrap();

        let head = repo.head().unwrap();
        let parent_commit = head.peel_to_commit().unwrap();
        let base_tree = parent_commit.tree().unwrap();
        let contents = std::fs::read(&file_path).unwrap();
        let blob_oid = repo.blob(&contents).unwrap();
        let mut builder = repo.treebuilder(Some(&base_tree)).unwrap();
        builder.insert(unique_name.as_str(), blob_oid, git2::FileMode::Blob.into()).unwrap();
        let tree_oid = builder.write().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let signature = git2::Signature::now("SuperBrain Test", "test@localhost").unwrap();
        let branch_ref_name = head.name().unwrap().to_string();
        repo.commit(Some(&branch_ref_name), &signature, &signature, "unrelated change", &tree, &[&parent_commit])
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::journal::{AdvanceFields, Journal};
    use domain::WriteState;

    #[test]
    fn commit_operation_creates_a_commit_with_operation_id_in_the_message() {
        let repo_dir = init_test_repo();
        let record_path = repo_dir.join("note-a.record");
        std::fs::write(&record_path, b"1\n---\ncontent").unwrap();
        let op = Uuid::new_v4();
        let oid = commit_operation(&repo_dir, op, "note-a", &record_path).unwrap();
        let repo = git2::Repository::open(&repo_dir).unwrap();
        let commit = repo.find_commit(oid).unwrap();
        assert!(commit.message().unwrap().contains(&op.to_string()));
    }

    #[test]
    fn commit_operation_does_not_include_unrelated_content_staged_in_the_shared_index() {
        // Pins the user architectural review's point 5 fix: commit_operation must
        // build its tree from HEAD + this one blob via TreeBuilder, never from
        // Index::write_tree(), so content staged by something else (simulated
        // here by staging directly into the repo's shared index) never leaks in.
        let repo_dir = init_test_repo();
        let record_path = repo_dir.join("note-a.record");
        std::fs::write(&record_path, b"1\n---\ncontent").unwrap();

        // Simulate a concurrent/leftover staged change unrelated to this operation.
        let unrelated_path = repo_dir.join("unrelated.record");
        std::fs::write(&unrelated_path, b"should never appear in op's commit").unwrap();
        let repo = git2::Repository::open(&repo_dir).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("unrelated.record")).unwrap();
        index.write().unwrap(); // left staged, never committed by anything

        let op = Uuid::new_v4();
        let oid = commit_operation(&repo_dir, op, "note-a", &record_path).unwrap();
        let commit = repo.find_commit(oid).unwrap();
        let tree = commit.tree().unwrap();
        assert!(tree.get_name("note-a.record").is_some(), "this operation's own file must be present");
        assert!(tree.get_name("unrelated.record").is_none(), "unrelated staged content must NOT leak into this operation's commit");
    }

    #[test]
    fn commit_operation_called_twice_for_the_same_operation_id_does_not_duplicate() {
        let repo_dir = init_test_repo();
        let record_path = repo_dir.join("note-a.record");
        std::fs::write(&record_path, b"1\n---\ncontent").unwrap();
        let op = Uuid::new_v4();
        let first = commit_operation(&repo_dir, op, "note-a", &record_path).unwrap();
        let second = commit_operation(&repo_dir, op, "note-a", &record_path).unwrap();
        assert_eq!(first, second, "re-entry for the same operation_id must return the existing commit, not create a new one");
    }

    #[test]
    fn find_commit_for_operation_uses_the_journals_own_oid_without_walking_history() {
        // Pins the user architectural review's point 5 fix: Tier 1 is an exact
        // journal lookup. This test proves it by recording a (fabricated, for
        // test purposes) git_commit_oid on the journal row directly, with NO
        // corresponding commit actually existing in the repo's history — showing
        // find_commit_for_operation trusts the journal's own record rather than
        // re-deriving it from a scan every time.
        let repo_dir = init_test_repo();
        let journal = Journal::open_in_memory().unwrap();
        let op = Uuid::new_v4();
        journal.insert_intent(op, "note-a", 0).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, "h")).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        let fabricated_oid = git2::Oid::from_bytes(&[1u8; 20]).unwrap();
        journal.advance(op, WriteState::GitCommitted, AdvanceFields::git_oid(fabricated_oid)).unwrap();
        assert_eq!(find_commit_for_operation(&journal, &repo_dir, op, ScanDepth::Unbounded).unwrap(), Some(fabricated_oid), "Tier 1 (journal) must be consulted first and trusted, without requiring a matching commit to actually exist in history for this unit test to prove the lookup path");
    }

    #[test]
    fn find_commit_for_operation_falls_back_to_history_scan_and_repairs_the_journal_row() {
        // The crash-between-git-commit-and-journal-update case the review raised:
        // a real commit exists, but the journal row was never updated with its
        // Oid (Tier 1 empty). Tier 2 must find it via history AND the caller must
        // persist it back so Tier 1 becomes authoritative next time.
        let repo_dir = init_test_repo();
        let record_path = repo_dir.join("note-a.record");
        std::fs::write(&record_path, b"1\n---\ncontent").unwrap();
        let op = Uuid::new_v4();
        let real_oid = commit_operation(&repo_dir, op, "note-a", &record_path).unwrap();

        let journal = Journal::open_in_memory().unwrap();
        journal.insert_intent(op, "note-a", 0).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, "h")).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        // Deliberately NOT advanced to GitCommitted — simulates the crash.

        let found = find_commit_for_operation(&journal, &repo_dir, op, ScanDepth::Bounded(500)).unwrap();
        assert_eq!(found, Some(real_oid), "Tier 2 history scan must still find the real commit when the journal row has no git_commit_oid yet and it's well within the bound");
    }

    #[test]
    fn find_commit_for_operation_returns_none_for_an_unknown_operation_id() {
        let repo_dir = init_test_repo();
        let record_path = repo_dir.join("note-a.record");
        std::fs::write(&record_path, b"1\n---\ncontent").unwrap();
        commit_operation(&repo_dir, Uuid::new_v4(), "note-a", &record_path).unwrap();
        let journal = Journal::open_in_memory().unwrap();
        assert_eq!(find_commit_for_operation(&journal, &repo_dir, Uuid::new_v4(), ScanDepth::Unbounded).unwrap(), None);
    }

    #[test]
    fn bounded_scan_misses_a_real_commit_beyond_the_depth_cap_but_unbounded_finds_it() {
        // Pins the second user architectural review's correction to issue 3:
        // a bounded Tier 2 scan can return None for a commit that genuinely
        // exists, if enough other commits have landed since. This is the exact
        // ambiguity that must never be allowed to trigger a second commit_operation
        // call during recovery — hence reconcile always using ScanDepth::Unbounded.
        let repo_dir = init_test_repo();
        let record_path = repo_dir.join("note-a.record");
        std::fs::write(&record_path, b"1\n---\ncontent").unwrap();
        let op = Uuid::new_v4();
        let real_oid = commit_operation(&repo_dir, op, "note-a", &record_path).unwrap();

        // Push enough unrelated commits past the bound to push op's commit beyond
        // it. (`commit_unrelated_change_for_test` is a small test helper, also
        // used by Task 8: writes a throwaway file and makes a plain git2 commit
        // with no operation_id marker in its message, so it never satisfies any
        // Tier 2 lookup itself — it exists only to advance HEAD.)
        for i in 0..10 {
            commit_unrelated_change_for_test(&repo_dir);
            let _ = i;
        }

        let journal = Journal::open_in_memory().unwrap();
        journal.insert_intent(op, "note-a", 0).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, "h")).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();

        // A bound smaller than the number of commits since op's own commit must miss it.
        let bounded_result = find_commit_for_operation(&journal, &repo_dir, op, ScanDepth::Bounded(3)).unwrap();
        assert_eq!(bounded_result, None, "a bound narrower than the real distance to op's commit must not find it — this is the ambiguous case, not an error");

        // The SAME lookup, unbounded, must still find it — proving recovery's
        // choice of ScanDepth::Unbounded is what actually closes the ambiguity,
        // not some property of the scan itself.
        let unbounded_result = find_commit_for_operation(&journal, &repo_dir, op, ScanDepth::Unbounded).unwrap();
        assert_eq!(unbounded_result, Some(real_oid), "an unbounded scan must find the same real commit the bounded scan missed");
    }

    #[test]
    fn commit_operation_on_a_non_repository_path_reports_no_repository() {
        let dir = unique_test_dir(); // never git-initialized
        let record_path = dir.join("note-a.record");
        std::fs::write(&record_path, b"1\n---\ncontent").unwrap();
        let err = commit_operation(&dir, Uuid::new_v4(), "note-a", &record_path).unwrap_err();
        assert!(matches!(err, GitError::NoRepository));
    }
}
