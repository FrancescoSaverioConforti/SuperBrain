//! B2 Task 0/1: the ingestion seam between `codegraph-index` (parsing,
//! extraction, the graph) and wherever repository bytes actually come
//! from.
//!
//! `RepositorySource` is the one interface `codegraph-index` depends on to
//! read a repository. Task 0 pinned this seam with raw `&str` identifiers,
//! mirroring `storage::Indexer::mark_current`'s own raw-`&str`-at-the-seam
//! precedent. Task 1 now REPLACES that shape: every method takes the
//! validated [`RepositoryId`]/[`CommitHash`]/[`FileIdentity`] types instead
//! of `&str`. There is no second, raw-string overload left behind —
//! `codegraph-index` cannot call this trait with an arbitrary,
//! unvalidated repository id, commit hash or file path, because the only
//! way to obtain a value of each of those types is through their own
//! validated constructors (`RepositoryRegistry::register`/`lookup`,
//! `CommitHash::parse`, `FileIdentity::parse`) — none of which accepts an
//! unvalidated passthrough.
//!
//! This crate has no dependency on `tree-sitter`, `git2`, `storage` or
//! `api`: `codegraph-domain` owns identity/validation/graph *types*, never
//! I/O, mirroring how `domain` has no dependency on `storage` or `index`.

use thiserror::Error;

use crate::identity::{CommitHash, FileIdentity, RepositoryId};

/// Everything that can go wrong resolving or reading repository content
/// through a [`RepositorySource`]. Deliberately not yet distinguishing
/// "outside the allowlist" from "not found" from "I/O error" with the
/// precision Task 2's real filesystem/git2 implementor will need — Task 0
/// only has to prove the seam carries *a* typed, non-panicking error, not
/// specify every variant a real implementation will eventually return.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourceError {
    /// The requested repository is not one this source is configured to
    /// read. Named now because Task 2's `RepositoryAllowlist` must fail
    /// closed on exactly this condition; the variant exists in Task 0 so
    /// that invariant is pinned in the seam's shape from the start, not
    /// retrofitted once Task 2 lands. Carries the `RepositoryId`'s
    /// underlying UUID (never a raw caller-supplied string, since by Task
    /// 1 a caller cannot reach this trait with anything but a validated
    /// `RepositoryId`) for diagnostics.
    #[error("repository {repo:?} is not known to this source")]
    UnknownRepository { repo: uuid::Uuid },
    /// The requested commit is not known for the requested repository.
    #[error("commit {commit} is not known for repository {repo:?}")]
    UnknownCommit { repo: uuid::Uuid, commit: String },
    /// The requested file is not known at the requested commit.
    #[error("file {file:?} is not known for repository {repo:?} at commit {commit}")]
    UnknownFile { repo: uuid::Uuid, commit: String, file: String },
    /// Any other, non-panicking failure a real implementor needs to report
    /// (I/O error, corrupt repository, etc.), carried as a sanitized
    /// message — never the raw underlying error's Display, so a future
    /// implementor cannot accidentally leak a host path or OS error text
    /// through this seam (same discipline as `ApiError`'s Display impl in
    /// `crates/api/src/lib.rs`, which never echoes an underlying error).
    #[error("repository source failed: {reason}")]
    Other { reason: String },
}

/// The seam `codegraph-index` reads a repository through. No implementor
/// exists yet in this crate or in `codegraph-index` as of Task 1: Task 2
/// adds the first real one (`LocalGitRepositorySource`, backed by the
/// filesystem and `git2`, gated by an explicit `RepositoryAllowlist`).
///
/// Every method takes validated [`RepositoryId`]/[`CommitHash`]/
/// [`FileIdentity`] values, never a raw string — there is no raw-`&str`
/// overload of either method. An implementor must still reject
/// identifiers it does not itself recognize (fail closed), since a
/// well-typed `RepositoryId`/`CommitHash` can still name a repository or
/// commit this particular source was never configured to read (e.g. one
/// `RepositoryRegistry` entry a different, unrelated source does not
/// serve); validity of the *type* is not the same as authorization to
/// read it, which remains Task 2's `RepositoryAllowlist`'s job.
pub trait RepositorySource: Send + Sync {
    /// Every file known for `repository` at `commit`, as repository-
    /// relative path strings — the raw candidates [`FileIdentity::parse`]
    /// will later validate, not yet `FileIdentity` values themselves,
    /// since a `FileIdentity` by construction already names one specific
    /// file; this method is what tells the caller which files exist in
    /// the first place. An implementor must return
    /// `SourceError::UnknownRepository` or `SourceError::UnknownCommit`
    /// rather than an empty list when the repository or commit itself is
    /// not recognized — an empty list is reserved for a real, empty
    /// commit.
    fn list_files_at(&self, repository: RepositoryId, commit: &CommitHash) -> Result<Vec<String>, SourceError>;

    /// The raw bytes of `file`. `file` already binds the repository and
    /// commit it was resolved at (that is exactly what [`FileIdentity`]
    /// is), so this method takes one argument, not three — there is no way
    /// to pass a mismatched `(repository, commit, file)` triple, because
    /// there is no way to construct a `FileIdentity` without supplying
    /// all three validated fields together at the same time
    /// (`FileIdentity::parse`'s signature requires it). An implementor
    /// must not follow a request for a path outside what `list_files_at`
    /// would have returned for the same `(repository, commit)` — this seam
    /// makes no promise about containment itself; containment is Task 2's
    /// job for its real implementor, not a property every possible
    /// implementor is required to prove at the trait level.
    fn read_file(&self, file: &FileIdentity) -> Result<Vec<u8>, SourceError>;
}

#[cfg(test)]
mod tests {
    use super::{RepositorySource, SourceError};
    use crate::identity::{CommitHash, FileIdentity, RepositoryId, RepositoryRegistry, RepositoryRoot};
    use std::collections::HashMap;

    /// A test double proving `RepositorySource` is implementable and
    /// callable from outside this crate's own module, with no filesystem,
    /// tree-sitter or git2 involved — exactly what Task 0/1 needs to pin
    /// before Task 2 writes a real implementor. Keyed by `(RepositoryId,
    /// CommitHash)` -> files, and `FileIdentity` -> bytes, entirely in
    /// memory, built only from already-validated types — there is no way
    /// to construct this fixture, or drive it, with an unvalidated raw
    /// string standing in for any of the three identity types.
    struct FakeRepositorySource {
        files_by_commit: HashMap<(RepositoryId, String), Vec<String>>,
        contents: HashMap<FileIdentity, Vec<u8>>,
    }

    impl FakeRepositorySource {
        fn new() -> Self {
            Self {
                files_by_commit: HashMap::new(),
                contents: HashMap::new(),
            }
        }

        fn with_file(mut self, file: FileIdentity, bytes: &[u8]) -> Self {
            self.files_by_commit
                .entry((file.repository(), file.commit().as_str().to_string()))
                .or_default()
                .push(file.relative_path().to_string());
            self.contents.insert(file, bytes.to_vec());
            self
        }
    }

    impl RepositorySource for FakeRepositorySource {
        fn list_files_at(&self, repository: RepositoryId, commit: &CommitHash) -> Result<Vec<String>, SourceError> {
            self.files_by_commit
                .get(&(repository, commit.as_str().to_string()))
                .cloned()
                .ok_or_else(|| SourceError::UnknownCommit {
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

    fn test_repo_and_commit() -> (RepositoryId, CommitHash) {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        (repo, commit)
    }

    #[test]
    fn list_files_at_returns_every_file_registered_for_that_repo_and_commit() {
        let (repo, commit) = test_repo_and_commit();
        let file_a = FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap();
        let file_b = FileIdentity::parse(repo, commit.clone(), "src/util.rs").unwrap();
        let source = FakeRepositorySource::new()
            .with_file(file_a, b"fn main() {}")
            .with_file(file_b, b"pub fn helper() {}");

        let mut files = source.list_files_at(repo, &commit).unwrap();
        files.sort();

        assert_eq!(files, vec!["src/lib.rs".to_string(), "src/util.rs".to_string()]);
    }

    #[test]
    fn list_files_at_an_unknown_commit_is_a_typed_error_not_an_empty_list() {
        let (repo, commit) = test_repo_and_commit();
        let file = FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap();
        let source = FakeRepositorySource::new().with_file(file, b"fn main() {}");

        let other_commit = CommitHash::parse(&"b".repeat(40)).unwrap();
        let result = source.list_files_at(repo, &other_commit);

        assert_eq!(
            result,
            Err(SourceError::UnknownCommit {
                repo: repo.as_uuid(),
                commit: other_commit.as_str().to_string(),
            })
        );
    }

    #[test]
    fn read_file_returns_the_exact_bytes_registered_for_that_file() {
        let (repo, commit) = test_repo_and_commit();
        let file = FileIdentity::parse(repo, commit, "src/lib.rs").unwrap();
        let source = FakeRepositorySource::new().with_file(file.clone(), b"fn main() {}");

        let bytes = source.read_file(&file).unwrap();

        assert_eq!(bytes, b"fn main() {}".to_vec());
    }

    #[test]
    fn read_file_for_an_unknown_file_is_a_typed_error_not_a_panic() {
        let (repo, commit) = test_repo_and_commit();
        let known_file = FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap();
        let source = FakeRepositorySource::new().with_file(known_file, b"fn main() {}");

        let missing_file = FileIdentity::parse(repo, commit.clone(), "src/missing.rs").unwrap();
        let result = source.read_file(&missing_file);

        assert_eq!(
            result,
            Err(SourceError::UnknownFile {
                repo: repo.as_uuid(),
                commit: commit.as_str().to_string(),
                file: "src/missing.rs".to_string(),
            })
        );
    }

    /// Proves the seam is usable generically — the exact shape
    /// `codegraph-index` will depend on (a `&dyn RepositorySource`, not a
    /// concrete type), with zero knowledge of `FakeRepositorySource`.
    #[test]
    fn repository_source_is_usable_as_a_trait_object() {
        let (repo, commit) = test_repo_and_commit();
        let file = FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap();
        let source: Box<dyn RepositorySource> = Box::new(FakeRepositorySource::new().with_file(file, b"fn main() {}"));

        let files = source.list_files_at(repo, &commit).unwrap();

        assert_eq!(files, vec!["src/lib.rs".to_string()]);
    }
}
