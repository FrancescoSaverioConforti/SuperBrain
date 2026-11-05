//! B2 Task 0: the ingestion seam between `codegraph-index` (parsing,
//! extraction, the graph) and wherever repository bytes actually come from.
//!
//! `RepositorySource` is the one interface `codegraph-index` depends on to
//! read a repository. It takes raw, not-yet-validated identifiers
//! (`&str`), the same way `storage::Indexer::mark_current` takes a raw
//! `note_id: &str` rather than a validated `storage::NoteId` — validation
//! lives one layer up, in whichever caller constructs the validated
//! identity types Task 1 defines (`RepositoryId`, `CommitHash`,
//! `FileIdentity`), not inside this seam. Task 0 intentionally defines
//! neither those validated types nor any real implementor: this module
//! exists only to prove the seam is implementable and callable from
//! outside this crate before anything concrete (tree-sitter, git2, a real
//! filesystem walk) is built behind it.
//!
//! This crate has no dependency on `tree-sitter`, `git2`, `storage` or
//! `api`: `codegraph-domain` owns identity/validation/graph *types*, never
//! I/O, mirroring how `domain` has no dependency on `storage` or `index`.

use thiserror::Error;

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
    /// retrofitted once Task 2 lands.
    #[error("repository {repo:?} is not known to this source")]
    UnknownRepository { repo: String },
    /// The requested commit is not known for the requested repository.
    #[error("commit {commit:?} is not known for repository {repo:?}")]
    UnknownCommit { repo: String, commit: String },
    /// The requested file is not known at the requested commit.
    #[error("file {file:?} is not known for repository {repo:?} at commit {commit:?}")]
    UnknownFile { repo: String, commit: String, file: String },
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
/// exists yet in this crate or in `codegraph-index` as of Task 0: Task 2
/// adds the first real one (`LocalGitRepositorySource`, backed by the
/// filesystem and `git2`, gated by an explicit `RepositoryAllowlist`).
///
/// Every method takes raw string identifiers, not validated types: Task 1
/// defines `RepositoryId`/`CommitHash`/`FileIdentity`, and a caller
/// constructs those from validated input before ever reaching this trait.
/// `RepositorySource` itself validates nothing and assumes nothing about
/// its caller's validation — implementors must still reject unknown/
/// unauthorized identifiers themselves (fail closed), since an invalid-but-
/// well-typed-elsewhere string can still reach here.
pub trait RepositorySource: Send + Sync {
    /// Every file known for `repo` at `commit`, as repository-relative
    /// path strings. An implementor must return `SourceError::UnknownRepository`
    /// or `SourceError::UnknownCommit` rather than an empty list when the
    /// repository or commit itself is not recognized — an empty list is
    /// reserved for a real, empty commit.
    fn list_files_at(&self, repo: &str, commit: &str) -> Result<Vec<String>, SourceError>;

    /// The raw bytes of `file` within `repo` at `commit`. An implementor
    /// must not follow a request for a path outside what `list_files_at`
    /// would have returned for the same `(repo, commit)` — this seam makes
    /// no promise about containment itself; containment is Task 2's job
    /// for its real implementor, not a property every possible implementor
    /// is required to prove at the trait level.
    fn read_file(&self, repo: &str, commit: &str, file: &str) -> Result<Vec<u8>, SourceError>;
}

#[cfg(test)]
mod tests {
    use super::{RepositorySource, SourceError};
    use std::collections::HashMap;

    /// A test double proving `RepositorySource` is implementable and
    /// callable from outside this crate's own module, with no filesystem,
    /// tree-sitter or git2 involved — exactly what Task 0 needs to pin
    /// before Task 2 writes a real implementor. Keyed by `(repo, commit)`
    /// -> files, and `(repo, commit, file)` -> bytes, entirely in memory.
    struct FakeRepositorySource {
        files_by_commit: HashMap<(String, String), Vec<String>>,
        contents: HashMap<(String, String, String), Vec<u8>>,
    }

    impl FakeRepositorySource {
        fn new() -> Self {
            Self {
                files_by_commit: HashMap::new(),
                contents: HashMap::new(),
            }
        }

        fn with_file(mut self, repo: &str, commit: &str, file: &str, bytes: &[u8]) -> Self {
            self.files_by_commit
                .entry((repo.to_string(), commit.to_string()))
                .or_default()
                .push(file.to_string());
            self.contents
                .insert((repo.to_string(), commit.to_string(), file.to_string()), bytes.to_vec());
            self
        }
    }

    impl RepositorySource for FakeRepositorySource {
        fn list_files_at(&self, repo: &str, commit: &str) -> Result<Vec<String>, SourceError> {
            self.files_by_commit
                .get(&(repo.to_string(), commit.to_string()))
                .cloned()
                .ok_or_else(|| SourceError::UnknownCommit {
                    repo: repo.to_string(),
                    commit: commit.to_string(),
                })
        }

        fn read_file(&self, repo: &str, commit: &str, file: &str) -> Result<Vec<u8>, SourceError> {
            self.contents
                .get(&(repo.to_string(), commit.to_string(), file.to_string()))
                .cloned()
                .ok_or_else(|| SourceError::UnknownFile {
                    repo: repo.to_string(),
                    commit: commit.to_string(),
                    file: file.to_string(),
                })
        }
    }

    #[test]
    fn list_files_at_returns_every_file_registered_for_that_repo_and_commit() {
        let source = FakeRepositorySource::new()
            .with_file("repo-a", "deadbeef", "src/lib.rs", b"fn main() {}")
            .with_file("repo-a", "deadbeef", "src/util.rs", b"pub fn helper() {}");

        let mut files = source.list_files_at("repo-a", "deadbeef").unwrap();
        files.sort();

        assert_eq!(files, vec!["src/lib.rs".to_string(), "src/util.rs".to_string()]);
    }

    #[test]
    fn list_files_at_an_unknown_commit_is_a_typed_error_not_an_empty_list() {
        let source = FakeRepositorySource::new().with_file("repo-a", "deadbeef", "src/lib.rs", b"fn main() {}");

        let result = source.list_files_at("repo-a", "not-a-real-commit");

        assert_eq!(
            result,
            Err(SourceError::UnknownCommit {
                repo: "repo-a".to_string(),
                commit: "not-a-real-commit".to_string(),
            })
        );
    }

    #[test]
    fn read_file_returns_the_exact_bytes_registered_for_that_file() {
        let source = FakeRepositorySource::new().with_file("repo-a", "deadbeef", "src/lib.rs", b"fn main() {}");

        let bytes = source.read_file("repo-a", "deadbeef", "src/lib.rs").unwrap();

        assert_eq!(bytes, b"fn main() {}".to_vec());
    }

    #[test]
    fn read_file_for_an_unknown_file_is_a_typed_error_not_a_panic() {
        let source = FakeRepositorySource::new().with_file("repo-a", "deadbeef", "src/lib.rs", b"fn main() {}");

        let result = source.read_file("repo-a", "deadbeef", "src/missing.rs");

        assert_eq!(
            result,
            Err(SourceError::UnknownFile {
                repo: "repo-a".to_string(),
                commit: "deadbeef".to_string(),
                file: "src/missing.rs".to_string(),
            })
        );
    }

    /// Proves the seam is usable generically — the exact shape
    /// `codegraph-index` will depend on (a `&dyn RepositorySource`, not a
    /// concrete type), with zero knowledge of `FakeRepositorySource`.
    #[test]
    fn repository_source_is_usable_as_a_trait_object() {
        let source: Box<dyn RepositorySource> =
            Box::new(FakeRepositorySource::new().with_file("repo-a", "deadbeef", "src/lib.rs", b"fn main() {}"));

        let files = source.list_files_at("repo-a", "deadbeef").unwrap();

        assert_eq!(files, vec!["src/lib.rs".to_string()]);
    }
}
