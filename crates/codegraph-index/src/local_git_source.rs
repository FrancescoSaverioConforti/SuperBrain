//! B2 Task 2: `LocalGitRepositorySource` — the first real
//! [`codegraph_domain::RepositorySource`] implementor, backed by the local
//! filesystem and `git2`.
//!
//! # SHA-256 limitation (documented, not implemented)
//!
//! `codegraph_domain::CommitHash::parse` accepts both 40-hex-char (SHA-1)
//! and 64-hex-char (SHA-256) commit hashes — see Task 1. This
//! implementation only resolves SHA-1 commits. `git2` 0.21's SHA-256
//! object-format support exists only behind its `unstable-sha256` Cargo
//! feature, which is explicitly labeled unstable upstream and which no
//! crate in this workspace enables (`storage`, `index` and `api` all
//! depend on plain `git2 = "0.21"`, the same as this crate). Enabling it
//! here alone would not make a SHA-256 repository usable end to end — the
//! rest of this project's git integration (`storage::git`) has no
//! SHA-256 path either — and would require revisiting `git2`'s build
//! requirements (it links a `libgit2` build whose SHA-256 support has its
//! own platform/toolchain constraints) as a decision of its own, not a
//! one-crate opt-in. Per the task's explicit instruction not to simulate
//! support that does not really exist: a request to resolve a
//! 64-hex-char `CommitHash` through this source returns
//! `SourceError::Other` with a message naming this limitation, never a
//! silent SHA-1 fallback and never a panic.
//!
//! # "At commit" semantics
//!
//! [`codegraph_domain::FileIdentity`] is bound to the `CommitHash` it was
//! resolved at (Task 1's contract). `read_file` honors that literally: it
//! resolves the git **blob** for the requested path inside the commit's
//! tree, via `git2`'s object database, and never reads the working-tree
//! file from disk. For a `CommitHash` that is not the repository's current
//! `HEAD`, the working tree would hold different bytes (or none at all,
//! if the file did not exist at that commit) than the identity claims, so
//! reading through the filesystem would silently violate the identity's
//! own meaning. This also means `read_file` never touches anything
//! outside the repository's `.git` object store for content bytes — the
//! only filesystem path this module ever opens directly is the
//! already-allowlisted repository root itself, to open the `Repository`.

use std::path::Path;

use codegraph_domain::{CommitHash, FileIdentity, RepositoryId, RepositorySource, SourceError};
use git2::Repository;

use crate::allowlist::{AllowlistedRoot, RepositoryAllowlist};

/// A real, `git2`-backed [`RepositorySource`], scoped to one
/// [`RepositoryAllowlist`] and one [`codegraph_domain::RepositoryRegistry`]'s
/// worth of registered roots. Every read is checked against the allowlist
/// at the point the repository is opened — there is no cached "already
/// checked once" shortcut that could go stale if the allowlist changes
/// between calls, since each call re-derives the root from `RepositoryId`
/// via the caller-supplied lookup and re-validates it is still authorized.
pub struct LocalGitRepositorySource<'a> {
    allowlist: &'a RepositoryAllowlist,
    /// Resolves a `RepositoryId` back to the filesystem root it was
    /// registered for. Supplied by the caller (the trusted host already
    /// holds this mapping via `codegraph_domain::RepositoryRegistry`) —
    /// this type does not own or duplicate that registry; it only needs
    /// read access to answer "which root does this id mean." `Send + Sync`
    /// because `RepositorySource` itself requires both (it may be shared
    /// across threads by a future caller), so the closure a caller
    /// supplies must satisfy the same bound.
    root_for_id: &'a (dyn Fn(RepositoryId) -> Option<AllowlistedRoot> + Send + Sync),
}

impl<'a> LocalGitRepositorySource<'a> {
    pub fn new(
        allowlist: &'a RepositoryAllowlist,
        root_for_id: &'a (dyn Fn(RepositoryId) -> Option<AllowlistedRoot> + Send + Sync),
    ) -> Self {
        Self { allowlist, root_for_id }
    }

    /// Resolves `repository` to an allowlisted root, or a typed error.
    /// Fails closed on EITHER "this id's root is not known to the
    /// caller-supplied lookup" OR "this id's root is known but is not (or
    /// is no longer) in the allowlist" — both collapse to the same
    /// `SourceError::UnknownRepository`, since from this source's
    /// perspective neither case is readable, and distinguishing them
    /// further would let a caller probe which specific reason applied.
    fn resolve_root(&self, repository: RepositoryId) -> Result<AllowlistedRoot, SourceError> {
        let root = (self.root_for_id)(repository).ok_or(SourceError::UnknownRepository {
            repo: repository.as_uuid(),
        })?;
        if self.allowlist.is_authorized(root.as_path()) {
            Ok(root)
        } else {
            Err(SourceError::UnknownRepository {
                repo: repository.as_uuid(),
            })
        }
    }

    fn open_repository(&self, repository: RepositoryId) -> Result<(AllowlistedRoot, Repository), SourceError> {
        let root = self.resolve_root(repository)?;
        let repo = Repository::open(root.as_path()).map_err(|e| SourceError::Other {
            reason: format!("failed to open repository: {}", classify_git_error(&e)),
        })?;
        Ok((root, repo))
    }

    fn resolve_commit<'repo>(
        &self,
        repo: &'repo Repository,
        repository: RepositoryId,
        commit: &CommitHash,
    ) -> Result<git2::Commit<'repo>, SourceError> {
        if commit.as_str().len() != 40 {
            // Task 1 accepts 64-hex-char (SHA-256) CommitHash values; this
            // source cannot resolve them. See the module doc comment.
            return Err(SourceError::Other {
                reason: "SHA-256 commit resolution is not supported: git2's unstable-sha256 \
                          feature is not enabled in this workspace (see local_git_source.rs's \
                          module doc comment)"
                    .to_string(),
            });
        }
        let oid = git2::Oid::from_str(commit.as_str()).map_err(|_| SourceError::UnknownCommit {
            repo: repository.as_uuid(),
            commit: commit.as_str().to_string(),
        })?;
        repo.find_commit(oid).map_err(|e| {
            if e.code() == git2::ErrorCode::NotFound {
                SourceError::UnknownCommit {
                    repo: repository.as_uuid(),
                    commit: commit.as_str().to_string(),
                }
            } else {
                SourceError::Other {
                    reason: format!("failed to resolve commit: {}", classify_git_error(&e)),
                }
            }
        })
    }
}

impl<'a> RepositorySource for LocalGitRepositorySource<'a> {
    fn list_files_at(&self, repository: RepositoryId, commit: &CommitHash) -> Result<Vec<String>, SourceError> {
        let (_root, repo) = self.open_repository(repository)?;
        let git_commit = self.resolve_commit(&repo, repository, commit)?;
        let tree = git_commit.tree().map_err(|e| SourceError::Other {
            reason: format!("failed to read commit tree: {}", classify_git_error(&e)),
        })?;

        let mut files = Vec::new();
        let mut walk_error = None;
        tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
            if entry.kind() != Some(git2::ObjectType::Blob) {
                return git2::TreeWalkResult::Ok;
            }
            let Ok(name) = entry.name() else {
                walk_error = Some(SourceError::Other {
                    reason: "a tree entry had a non-UTF-8 name".to_string(),
                });
                return git2::TreeWalkResult::Abort;
            };
            let relative_path = if root.is_empty() {
                name.to_string()
            } else {
                format!("{}{}", root, name)
            };
            files.push(relative_path);
            git2::TreeWalkResult::Ok
        })
        .map_err(|e| SourceError::Other {
            reason: format!("failed to walk commit tree: {}", classify_git_error(&e)),
        })?;

        if let Some(err) = walk_error {
            return Err(err);
        }

        Ok(files)
    }

    fn read_file(&self, file: &FileIdentity) -> Result<Vec<u8>, SourceError> {
        let (_root, repo) = self.open_repository(file.repository())?;
        let git_commit = self.resolve_commit(&repo, file.repository(), file.commit())?;
        let tree = git_commit.tree().map_err(|e| SourceError::Other {
            reason: format!("failed to read commit tree: {}", classify_git_error(&e)),
        })?;

        let entry = tree.get_path(Path::new(file.relative_path())).map_err(|e| {
            if e.code() == git2::ErrorCode::NotFound {
                SourceError::UnknownFile {
                    repo: file.repository().as_uuid(),
                    commit: file.commit().as_str().to_string(),
                    file: file.relative_path().to_string(),
                }
            } else {
                SourceError::Other {
                    reason: format!("failed to resolve path in tree: {}", classify_git_error(&e)),
                }
            }
        })?;

        if entry.kind() != Some(git2::ObjectType::Blob) {
            return Err(SourceError::UnknownFile {
                repo: file.repository().as_uuid(),
                commit: file.commit().as_str().to_string(),
                file: file.relative_path().to_string(),
            });
        }

        let object = entry.to_object(&repo).map_err(|e| SourceError::Other {
            reason: format!("failed to load blob object: {}", classify_git_error(&e)),
        })?;
        let blob = object.as_blob().ok_or_else(|| SourceError::UnknownFile {
            repo: file.repository().as_uuid(),
            commit: file.commit().as_str().to_string(),
            file: file.relative_path().to_string(),
        })?;

        Ok(blob.content().to_vec())
    }
}

/// Maps a `git2::Error` to a short, sanitized classification string —
/// the error's `code()`/`class()` (closed enums `git2` itself defines),
/// never `e.message()`'s free-text Display, so this can never leak a raw
/// host path or other underlying detail through `SourceError::Other`'s
/// `reason` field. Mirrors `ApiError`'s sanitized-`Display` discipline
/// (`crates/api/src/lib.rs`) one layer down, for the one error type in
/// this module that legitimately needs to wrap an arbitrary `git2`
/// failure.
fn classify_git_error(e: &git2::Error) -> String {
    format!("{:?}/{:?}", e.class(), e.code())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_domain::{RepositoryRegistry, RepositoryRoot};
    use std::process::Command;

    /// Initializes a real git repository at a fresh temp directory with
    /// one commit containing the given `(relative_path, content)` files,
    /// returning the directory and the commit's SHA-1 hex string.
    /// Deliberately shells out to the real `git` binary rather than using
    /// `git2` to build the fixture, so the fixture's correctness does not
    /// depend on the same library this module's own logic is being tested
    /// against.
    fn init_repo_with_commit(files: &[(&str, &[u8])]) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("b2-local-git-source-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        run_git(&dir, &["init", "--quiet"]);
        run_git(&dir, &["config", "user.email", "test@example.com"]);
        run_git(&dir, &["config", "user.name", "Test"]);

        for (relative_path, content) in files {
            let full_path = dir.join(relative_path);
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full_path, content).unwrap();
            run_git(&dir, &["add", relative_path]);
        }
        run_git(&dir, &["commit", "--quiet", "-m", "test commit"]);

        let commit_hash = run_git_capture(&dir, &["rev-parse", "HEAD"]).trim().to_string();
        (dir, commit_hash)
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git").args(args).current_dir(dir).status().unwrap();
        assert!(status.success(), "git {args:?} failed in {dir:?}");
    }

    fn run_git_capture(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(output.status.success(), "git {args:?} failed in {dir:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    /// Wires an allowlist + registry + `LocalGitRepositorySource` around a
    /// fixture repository in one call, returning everything a test needs
    /// (kept alive by the caller's own bindings — `root_map`'s closure
    /// borrows it).
    struct Fixture {
        allowlist: RepositoryAllowlist,
        registry: RepositoryRegistry,
        repo_id: RepositoryId,
        commit: CommitHash,
        _dir: std::path::PathBuf,
    }

    fn build_fixture(files: &[(&str, &[u8])]) -> Fixture {
        let (dir, commit_hex) = init_repo_with_commit(files);
        let mut allowlist = RepositoryAllowlist::new();
        let allowlisted = allowlist.authorize(&dir).unwrap();
        let mut registry = RepositoryRegistry::new();
        let repo_id = registry.register(RepositoryRoot::parse(allowlisted.as_path()).unwrap());
        let commit = CommitHash::parse(&commit_hex).unwrap();
        Fixture {
            allowlist,
            registry,
            repo_id,
            commit,
            _dir: dir,
        }
    }

    fn root_lookup<'a>(registry: &'a RepositoryRegistry, root_path: &'a Path) -> impl Fn(RepositoryId) -> Option<AllowlistedRoot> + 'a {
        let root_path = root_path.to_path_buf();
        move |_id: RepositoryId| {
            let root = RepositoryRoot::parse(&root_path).ok()?;
            // Re-derive an AllowlistedRoot the same way `authorize` would
            // have: this closure stands in for "the host's own lookup
            // from RepositoryId to its registered root," which in this
            // test fixture is always the one fixture root, regardless of
            // which id is asked (a real host's lookup would consult its
            // own RepositoryId -> path table).
            let _ = registry;
            Some(allowlisted_root_for_test(root))
        }
    }

    /// A test-only way to obtain an `AllowlistedRoot` for a path this
    /// test already trusts, since `AllowlistedRoot` has no public
    /// constructor of its own — this routes through
    /// `RepositoryAllowlist::authorize` against the same path, which is
    /// itself a real, validated construction, not a bypass of it.
    fn allowlisted_root_for_test(root: RepositoryRoot) -> AllowlistedRoot {
        let mut allowlist = RepositoryAllowlist::new();
        allowlist.authorize(root.as_path()).unwrap()
    }

    #[test]
    fn list_files_at_returns_every_blob_in_the_commit_tree() {
        let fixture = build_fixture(&[("src/lib.rs", b"fn main() {}"), ("README.md", b"hello")]);
        let lookup = root_lookup(&fixture.registry, &fixture._dir);
        let source = LocalGitRepositorySource::new(&fixture.allowlist, &lookup);

        let mut files = source.list_files_at(fixture.repo_id, &fixture.commit).unwrap();
        files.sort();

        assert_eq!(files, vec!["README.md".to_string(), "src/lib.rs".to_string()]);
    }

    #[test]
    fn read_file_returns_the_exact_blob_content_at_that_commit() {
        let fixture = build_fixture(&[("src/lib.rs", b"fn main() {}")]);
        let lookup = root_lookup(&fixture.registry, &fixture._dir);
        let source = LocalGitRepositorySource::new(&fixture.allowlist, &lookup);

        let file = FileIdentity::parse(fixture.repo_id, fixture.commit.clone(), "src/lib.rs").unwrap();
        let bytes = source.read_file(&file).unwrap();

        assert_eq!(bytes, b"fn main() {}".to_vec());
    }

    #[test]
    fn reading_a_file_that_does_not_exist_at_that_commit_is_a_typed_error() {
        let fixture = build_fixture(&[("src/lib.rs", b"fn main() {}")]);
        let lookup = root_lookup(&fixture.registry, &fixture._dir);
        let source = LocalGitRepositorySource::new(&fixture.allowlist, &lookup);

        let missing = FileIdentity::parse(fixture.repo_id, fixture.commit.clone(), "does/not/exist.rs").unwrap();
        let result = source.read_file(&missing);

        assert!(matches!(result, Err(SourceError::UnknownFile { .. })));
    }

    #[test]
    fn a_syntactically_valid_but_nonexistent_commit_is_a_typed_error_not_a_panic() {
        let fixture = build_fixture(&[("src/lib.rs", b"fn main() {}")]);
        let lookup = root_lookup(&fixture.registry, &fixture._dir);
        let source = LocalGitRepositorySource::new(&fixture.allowlist, &lookup);

        let fake_commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let result = source.list_files_at(fixture.repo_id, &fake_commit);

        assert!(matches!(result, Err(SourceError::UnknownCommit { .. })));
    }

    #[test]
    fn a_sha256_commit_hash_is_a_documented_unsupported_error_not_a_silent_fallback() {
        let fixture = build_fixture(&[("src/lib.rs", b"fn main() {}")]);
        let lookup = root_lookup(&fixture.registry, &fixture._dir);
        let source = LocalGitRepositorySource::new(&fixture.allowlist, &lookup);

        let sha256_shaped = CommitHash::parse(&"a".repeat(64)).unwrap();
        let result = source.list_files_at(fixture.repo_id, &sha256_shaped);

        match result {
            Err(SourceError::Other { reason }) => assert!(reason.contains("SHA-256")),
            other => panic!("expected a documented SHA-256-unsupported error, got {other:?}"),
        }
    }

    #[test]
    fn a_root_not_in_the_allowlist_is_rejected_even_with_a_valid_repository_id() {
        let fixture = build_fixture(&[("src/lib.rs", b"fn main() {}")]);
        // A fresh, EMPTY allowlist — the fixture's root was never
        // authorized against this one, simulating a host lookup that
        // resolves the id to a real root the CALLER never allowlisted.
        let empty_allowlist = RepositoryAllowlist::new();
        let dir = fixture._dir.clone();
        let lookup = move |_id: RepositoryId| {
            let root = RepositoryRoot::parse(&dir).ok()?;
            Some(allowlisted_root_for_test(root))
        };
        let source = LocalGitRepositorySource::new(&empty_allowlist, &lookup);

        let result = source.list_files_at(fixture.repo_id, &fixture.commit);

        assert!(matches!(result, Err(SourceError::UnknownRepository { .. })));
    }

    #[test]
    fn an_unregistered_repository_id_is_rejected() {
        let fixture = build_fixture(&[("src/lib.rs", b"fn main() {}")]);
        let lookup: fn(RepositoryId) -> Option<AllowlistedRoot> = |_id| None;
        let source = LocalGitRepositorySource::new(&fixture.allowlist, &lookup);

        let result = source.list_files_at(fixture.repo_id, &fixture.commit);

        assert!(matches!(result, Err(SourceError::UnknownRepository { .. })));
    }

    #[test]
    fn opening_a_non_git_directory_is_a_typed_error_not_a_panic() {
        let dir = std::env::temp_dir().join(format!("b2-not-a-repo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut allowlist = RepositoryAllowlist::new();
        allowlist.authorize(&dir).unwrap();
        let mut registry = RepositoryRegistry::new();
        let repo_id = registry.register(RepositoryRoot::parse(&dir).unwrap());
        let lookup = root_lookup(&registry, &dir);
        let source = LocalGitRepositorySource::new(&allowlist, &lookup);

        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let result = source.list_files_at(repo_id, &commit);

        match result {
            Err(SourceError::Other { reason }) => {
                // Pins `classify_git_error`'s actual output format
                // (`"{class:?}/{code:?}"`) against a mutant that replaces
                // it with an empty or constant string: a non-repository
                // path reported by `git2` as `ErrorClass::Repository` with
                // `ErrorCode::NotFound` must show up verbatim.
                assert!(
                    reason.contains("Repository") && reason.contains("NotFound"),
                    "expected the real git2 error class/code in the reason, got {reason:?}"
                );
            }
            other => panic!("expected Err(SourceError::Other {{ .. }}), got {other:?}"),
        }
    }

    #[test]
    fn a_repository_with_zero_commits_reports_unknown_commit_not_a_panic() {
        let dir = std::env::temp_dir().join(format!("b2-empty-repo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        run_git(&dir, &["init", "--quiet"]);

        let mut allowlist = RepositoryAllowlist::new();
        allowlist.authorize(&dir).unwrap();
        let mut registry = RepositoryRegistry::new();
        let repo_id = registry.register(RepositoryRoot::parse(&dir).unwrap());
        let lookup = root_lookup(&registry, &dir);
        let source = LocalGitRepositorySource::new(&allowlist, &lookup);

        // There is no real commit to ask for in a zero-commit repository;
        // any syntactically valid SHA-1 must report UnknownCommit, not
        // panic trying to resolve HEAD.
        let fake_commit = CommitHash::parse(&"0".repeat(40)).unwrap();
        let result = source.list_files_at(repo_id, &fake_commit);

        assert!(matches!(result, Err(SourceError::UnknownCommit { .. })));
    }

    #[test]
    fn detached_head_repository_still_resolves_a_specific_commit_by_hash() {
        // "Detached HEAD" is a property of the repository's HEAD
        // reference, not of a specific commit object. Since this source
        // resolves commits by their explicit hash (never by reading
        // `HEAD`), a detached-HEAD checkout changes nothing about whether
        // a given commit hash resolves. Proven by explicitly detaching
        // HEAD after the fixture commit and resolving that same commit by
        // hash again.
        let fixture = build_fixture(&[("src/lib.rs", b"fn main() {}")]);
        run_git(&fixture._dir, &["checkout", "--quiet", "--detach", fixture.commit.as_str()]);

        let lookup = root_lookup(&fixture.registry, &fixture._dir);
        let source = LocalGitRepositorySource::new(&fixture.allowlist, &lookup);

        let bytes = source
            .read_file(&FileIdentity::parse(fixture.repo_id, fixture.commit.clone(), "src/lib.rs").unwrap())
            .unwrap();

        assert_eq!(bytes, b"fn main() {}".to_vec());
    }

    #[test]
    fn reading_at_an_old_commit_returns_old_content_not_the_current_working_tree() {
        // Pins the "at commit" semantics directly: write a SECOND commit
        // that changes the file's content, then read the file at the
        // FIRST commit's hash and confirm it returns the OLD content, not
        // whatever is currently checked out in the working tree.
        let fixture = build_fixture(&[("src/lib.rs", b"fn main() {}")]);
        std::fs::write(fixture._dir.join("src/lib.rs"), b"fn main() { changed(); }").unwrap();
        run_git(&fixture._dir, &["add", "src/lib.rs"]);
        run_git(&fixture._dir, &["commit", "--quiet", "-m", "second commit"]);

        let lookup = root_lookup(&fixture.registry, &fixture._dir);
        let source = LocalGitRepositorySource::new(&fixture.allowlist, &lookup);

        // fixture.commit is still the FIRST commit's hash; the working
        // tree now holds the SECOND commit's content.
        let bytes = source
            .read_file(&FileIdentity::parse(fixture.repo_id, fixture.commit.clone(), "src/lib.rs").unwrap())
            .unwrap();

        assert_eq!(bytes, b"fn main() {}".to_vec());
    }
}
