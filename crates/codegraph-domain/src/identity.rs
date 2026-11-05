//! B2 Task 1: repository, commit and file identity.
//!
//! # `RepositoryId` design decision
//!
//! `RepositoryId` is **not** derived from a filesystem path, a git remote
//! URL, or a git root-commit hash. Each of those was considered and
//! rejected:
//!
//! - **Absolute path hash:** the one explicitly ruled out by the task —
//!   `C:\repo\project` moved to `D:\work\project` would mint a new identity,
//!   silently severing every snapshot/lineage edge already recorded against
//!   the old one. Rejected.
//! - **Git root-commit hash:** stable across `clone`/`move` of the *same*
//!   history, and needs no extra state — attractive at first. But it is
//!   undefined for a repository with no commits yet; ambiguous for a
//!   repository with multiple root commits (`git merge
//!   --allow-unrelated-histories`); silently *different* for a shallow
//!   clone that does not fetch back to the root commit; and silently
//!   *different* again for a history rewrite (squash-to-one-commit,
//!   `git commit --orphan`) that a user would still call "the same
//!   project." Every one of these is a case where the identity would be
//!   inferred from content that can be absent or can legitimately diverge
//!   from what a human means by "the same repository" — the same
//!   allowlist-of-names-without-validated-types failure mode the P1
//!   checkpoint flagged for temporal validation (CF-02/CF-03/ST-07),
//!   generalized to identity. Rejected.
//! - **Remote/origin URL hash:** undefined for a local-only repository
//!   (no remote at all); changes if the user renames a remote or switches
//!   its URL scheme (HTTPS vs SSH) for what is otherwise the same remote;
//!   and is a poor identity for a fork, which has its own remote URL by
//!   construction even when a user wants to treat fork and upstream as
//!   related. Rejected.
//!
//! **Decision: `RepositoryId` is an opaque identifier explicitly assigned
//! by the trusted host** (the same boundary that already supplies
//! `ReadPolicy` and, per the prior decision, a separate
//! `RepositoryAllowlist`) **the first time a filesystem root is
//! registered**, via [`RepositoryRegistry`]. The current filesystem root is
//! a *location/capability* — which path `RepositorySource` is currently
//! allowed and able to read — never the identity itself. Moving, cloning or
//! forking a repository never changes its `RepositoryId` on its own,
//! because nothing about the move/clone/fork is consulted to produce it;
//! only an explicit re-registration (the host telling
//! [`RepositoryRegistry`] "this new root is the same repository as that
//! existing id") can associate a new root with an existing id.
//!
//! Trade-off accepted explicitly: if the host does NOT re-register a moved
//! root, the moved root is treated as a brand-new, unrelated repository
//! (first-registration mints a fresh id) rather than silently inferring a
//! connection. This is a correctable, explicit action, not silent
//! misclassification — consistent with every other validated type in this
//! codebase (`NoteId::parse` rejects rather than normalizes;
//! `ValidationError::SecretClassificationForbidden` rejects rather than
//! defaults). [`RepositoryRegistry`] here is **in-memory only**: Task 1
//! does not persist the `{root -> RepositoryId}` mapping to disk. Choosing
//! and implementing that persistence is explicitly deferred — to Task 10,
//! gated by S1/S2 — so Task 1 does not create a new disk write path to
//! work around those gates.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;
use uuid::Uuid;

/// Maximum accepted relative-path length within a [`FileIdentity`], in
/// bytes. Mirrors `storage::note_id::MAX_NOTE_ID_LEN`'s reasoning (bound
/// every on-disk-derived identifier, not just the ones this project
/// started with) at a larger size appropriate for a file path inside an
/// arbitrary repository (note ids are a single filename component;
/// repository-relative paths are not).
const MAX_RELATIVE_PATH_LEN: usize = 4096;

/// Accepted commit-hash lengths: SHA-1 (40 hex chars) and SHA-256 (64 hex
/// chars), the two hash algorithms git itself supports.
const SHA1_HEX_LEN: usize = 40;
const SHA256_HEX_LEN: usize = 64;

/// An opaque repository identity, stable across moving, cloning or forking
/// the same repository on disk. See the module-level doc comment for why
/// this is not derived from a path, remote URL or root-commit hash. The
/// only way to obtain one is [`RepositoryRegistry::register`] or
/// [`RepositoryRegistry::lookup`] — there is no public constructor that
/// takes a caller-supplied value, unlike [`CommitHash`]/[`FileIdentity`],
/// because nothing about a `RepositoryId`'s value is meaningful on its own;
/// it only means something relative to the registry that minted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RepositoryId(Uuid);

impl RepositoryId {
    /// The underlying UUID, for display/logging/persistence by a future
    /// caller. Never parsed back into a `RepositoryId` directly — only
    /// `RepositoryRegistry` constructs one, so a persisted mapping is
    /// reloaded through the registry's own (future, Task 10) load path,
    /// not by round-tripping this value through a public `from_uuid`.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

/// A canonical, validated filesystem root path, used only as the
/// [`RepositoryRegistry`] lookup key. Validates that the path is absolute
/// and canonicalizable; does not open or read the repository itself (that
/// is `RepositorySource`'s job, Task 2).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepositoryRoot(PathBuf);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RepositoryRootError {
    #[error("repository root must be an absolute path, got {path:?}")]
    NotAbsolute { path: String },
    #[error("repository root does not exist or is not readable: {path:?}")]
    Unreadable { path: String },
}

impl RepositoryRoot {
    /// Validates `raw` is an absolute, canonicalizable path. Canonicalizes
    /// (resolves `.`/`..` and symlinks) so two spellings of the same root
    /// (e.g. a trailing slash, or a symlinked ancestor) are treated as the
    /// same registry key — unlike `NoteId`, which deliberately never
    /// normalizes, because a `NoteId` IS the on-disk filename (normalizing
    /// would change what file it refers to), whereas a `RepositoryRoot` is
    /// only a lookup key for a UUID the registry already owns; two
    /// spellings of the same real directory should hit the same entry.
    pub fn parse(raw: &Path) -> Result<Self, RepositoryRootError> {
        if !raw.is_absolute() {
            return Err(RepositoryRootError::NotAbsolute {
                path: raw.to_string_lossy().into_owned(),
            });
        }
        let canonical = raw.canonicalize().map_err(|_| RepositoryRootError::Unreadable {
            path: raw.to_string_lossy().into_owned(),
        })?;
        Ok(Self(canonical))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// In-memory `{RepositoryRoot -> RepositoryId}` mapping. The only source of
/// `RepositoryId` values. Deliberately has no disk persistence in Task 1
/// (see the module doc comment) — a future caller (Task 10, gated by
/// S1/S2) is responsible for saving and reloading this mapping; this type
/// only owns the in-memory bookkeeping and the invariant that the same
/// root always yields the same id for the lifetime of one registry
/// instance, and that a fresh id is never reused.
#[derive(Debug, Default)]
pub struct RepositoryRegistry {
    by_root: HashMap<RepositoryRoot, RepositoryId>,
}

impl RepositoryRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the existing `RepositoryId` for `root` if one was already
    /// registered (by this call or an earlier `register`/`associate`
    /// call), without minting a new one.
    pub fn lookup(&self, root: &RepositoryRoot) -> Option<RepositoryId> {
        self.by_root.get(root).copied()
    }

    /// Registers `root` as a repository, minting a fresh `RepositoryId` if
    /// this exact root has never been registered, or returning the
    /// existing id if it has. Never mints two different ids for the same
    /// `RepositoryRoot` within one registry instance.
    pub fn register(&mut self, root: RepositoryRoot) -> RepositoryId {
        if let Some(existing) = self.by_root.get(&root) {
            return *existing;
        }
        let id = RepositoryId(Uuid::new_v4());
        self.by_root.insert(root, id);
        id
    }

    /// Explicitly associates `new_root` with an `existing` id — the
    /// re-registration path for a moved/cloned repository the host
    /// recognizes as the same project. Never inferred automatically; the
    /// caller must already hold `existing` (from an earlier `register`)
    /// to assert this. If `new_root` is already associated with a
    /// DIFFERENT id, that association is overwritten — the caller is
    /// explicitly asserting identity, so the registry trusts it rather
    /// than silently keeping the old mapping and ignoring the request.
    pub fn associate(&mut self, new_root: RepositoryRoot, existing: RepositoryId) {
        self.by_root.insert(new_root, existing);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommitHash(String);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CommitHashError {
    #[error("commit hash must not be empty")]
    Empty,
    #[error("commit hash must be lowercase hex: found {found:?}")]
    NotLowercaseHex { found: char },
    #[error("commit hash must be {sha1} (SHA-1) or {sha256} (SHA-256) hex characters, got {actual}", sha1 = SHA1_HEX_LEN, sha256 = SHA256_HEX_LEN)]
    WrongLength { actual: usize },
}

impl CommitHash {
    /// Accepts exactly 40 (SHA-1) or 64 (SHA-256) lowercase hex characters.
    /// Never normalizes: an uppercase or mixed-case hash (valid git output
    /// on some tools/platforms) is rejected, not lowercased, mirroring
    /// `NoteId::parse`'s refusal to fold case silently — two differently-
    /// cased spellings must not be treated as interchangeable identifiers
    /// if one of them was ever accepted without normalization elsewhere.
    pub fn parse(raw: &str) -> Result<Self, CommitHashError> {
        if raw.is_empty() {
            return Err(CommitHashError::Empty);
        }
        if raw.len() != SHA1_HEX_LEN && raw.len() != SHA256_HEX_LEN {
            return Err(CommitHashError::WrongLength { actual: raw.len() });
        }
        for c in raw.chars() {
            if !(c.is_ascii_digit() || ('a'..='f').contains(&c)) {
                return Err(CommitHashError::NotLowercaseHex { found: c });
            }
        }
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated, repository-relative file path, bound to the
/// `RepositoryId`/`CommitHash` it was resolved at. Rejects the same shapes
/// `NoteId::parse` rejects (traversal, absolute paths, backslash-as-
/// separator on any platform so the identity is platform-independent),
/// applied to a full relative path rather than a single filename
/// component.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    repository: RepositoryId,
    commit: CommitHash,
    relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FileIdentityError {
    #[error("file path must not be empty")]
    Empty,
    #[error("file path is {actual} bytes long, maximum is {max}")]
    TooLong { max: usize, actual: usize },
    #[error("file path must not contain a parent-directory reference ('..')")]
    ContainsParentRef,
    #[error("file path must be relative")]
    NotRelative,
    #[error("file path must use '/' as separator, not '\\'")]
    ContainsBackslash,
}

impl FileIdentity {
    /// Validates `relative_path` and binds it to `repository`/`commit`.
    /// Never normalizes: a backslash-separated path is rejected (not
    /// converted to forward slashes), because accepting both spellings
    /// would let two different raw strings alias the same `FileIdentity`,
    /// the same aliasing risk `NoteId::parse` refuses for uppercase note
    /// ids.
    pub fn parse(repository: RepositoryId, commit: CommitHash, relative_path: &str) -> Result<Self, FileIdentityError> {
        if relative_path.is_empty() {
            return Err(FileIdentityError::Empty);
        }
        if relative_path.len() > MAX_RELATIVE_PATH_LEN {
            return Err(FileIdentityError::TooLong {
                max: MAX_RELATIVE_PATH_LEN,
                actual: relative_path.len(),
            });
        }
        if relative_path.contains('\\') {
            return Err(FileIdentityError::ContainsBackslash);
        }
        for component in Path::new(relative_path).components() {
            match component {
                Component::ParentDir => return Err(FileIdentityError::ContainsParentRef),
                Component::RootDir | Component::Prefix(_) => return Err(FileIdentityError::NotRelative),
                Component::CurDir | Component::Normal(_) => {}
            }
        }
        Ok(Self {
            repository,
            commit,
            relative_path: relative_path.to_string(),
        })
    }

    pub fn repository(&self) -> RepositoryId {
        self.repository
    }

    pub fn commit(&self) -> &CommitHash {
        &self.commit
    }

    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- RepositoryRegistry ---

    #[test]
    fn registering_the_same_root_twice_returns_the_same_id() {
        let mut registry = RepositoryRegistry::new();
        let root = RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap();

        let first = registry.register(root.clone());
        let second = registry.register(root);

        assert_eq!(first, second);
    }

    #[test]
    fn registering_two_different_roots_yields_two_different_ids() {
        let mut registry = RepositoryRegistry::new();
        let root_a = RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap();
        let root_b = RepositoryRoot::parse(&std::env::temp_dir()).unwrap();

        let id_a = registry.register(root_a);
        let id_b = registry.register(root_b);

        assert_ne!(id_a, id_b);
    }

    #[test]
    fn as_uuid_returns_the_actual_wrapped_value_not_a_default() {
        // Pins RepositoryId::as_uuid against a mutant that replaces it
        // with Default::default() (the nil UUID): two independently
        // registered ids must report two different UUIDs via as_uuid,
        // which a constant-default stub could never do.
        let mut registry = RepositoryRegistry::new();
        let id_a = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let id_b = registry.register(RepositoryRoot::parse(&std::env::temp_dir()).unwrap());

        assert_ne!(id_a.as_uuid(), id_b.as_uuid());
        assert_ne!(id_a.as_uuid(), Uuid::default());
    }

    #[test]
    fn lookup_before_registration_is_none() {
        let registry = RepositoryRegistry::new();
        let root = RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap();

        assert_eq!(registry.lookup(&root), None);
    }

    #[test]
    fn moving_a_repository_does_not_change_its_id_when_the_host_re_associates_it() {
        // The exact scenario the task called out: the SAME repository
        // appears to live at a new root after a move. Nothing here infers
        // that from path similarity; the host must say so explicitly via
        // `associate`, and once it does, the new root resolves to the
        // SAME RepositoryId the old root had.
        let mut registry = RepositoryRegistry::new();
        let old_root = RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap();
        let new_root = RepositoryRoot::parse(&std::env::temp_dir()).unwrap();

        let original_id = registry.register(old_root);
        registry.associate(new_root.clone(), original_id);

        assert_eq!(registry.lookup(&new_root), Some(original_id));
    }

    #[test]
    fn an_unregistered_moved_root_is_treated_as_a_new_repository_not_silently_linked() {
        // The accepted trade-off from the module doc comment: without an
        // explicit `associate` call, a different root is just a different
        // (new) repository, never silently inferred as related.
        let mut registry = RepositoryRegistry::new();
        let root_a = RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap();
        let root_b = RepositoryRoot::parse(&std::env::temp_dir()).unwrap();

        let id_a = registry.register(root_a);
        let id_b = registry.register(root_b);

        assert_ne!(id_a, id_b);
    }

    #[test]
    fn repository_root_rejects_a_relative_path() {
        assert!(matches!(
            RepositoryRoot::parse(Path::new("relative/path")),
            Err(RepositoryRootError::NotAbsolute { .. })
        ));
    }

    // --- CommitHash ---

    #[test]
    fn valid_sha1_commit_hash_is_accepted() {
        let hash = "a".repeat(40);
        assert!(CommitHash::parse(&hash).is_ok());
    }

    #[test]
    fn valid_sha256_commit_hash_is_accepted() {
        let hash = "a".repeat(64);
        assert!(CommitHash::parse(&hash).is_ok());
    }

    #[test]
    fn empty_commit_hash_is_rejected() {
        assert_eq!(CommitHash::parse(""), Err(CommitHashError::Empty));
    }

    #[test]
    fn wrong_length_commit_hash_is_rejected_not_truncated_or_padded() {
        let hash = "a".repeat(39);
        assert_eq!(CommitHash::parse(&hash), Err(CommitHashError::WrongLength { actual: 39 }));

        let too_long = "a".repeat(41);
        assert_eq!(CommitHash::parse(&too_long), Err(CommitHashError::WrongLength { actual: 41 }));
    }

    #[test]
    fn uppercase_commit_hash_is_rejected_not_lowercased() {
        let hash = "A".repeat(40);
        assert_eq!(CommitHash::parse(&hash), Err(CommitHashError::NotLowercaseHex { found: 'A' }));
    }

    #[test]
    fn non_hex_commit_hash_is_rejected() {
        let mut hash = "a".repeat(39);
        hash.push('g');
        assert_eq!(CommitHash::parse(&hash), Err(CommitHashError::NotLowercaseHex { found: 'g' }));
    }

    #[test]
    fn accepted_commit_hash_is_returned_byte_for_byte() {
        let hash = "f".repeat(40);
        assert_eq!(CommitHash::parse(&hash).unwrap().as_str(), hash);
    }

    // --- FileIdentity ---

    fn test_repo_id() -> RepositoryId {
        let mut registry = RepositoryRegistry::new();
        registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
    }

    fn test_commit() -> CommitHash {
        CommitHash::parse(&"a".repeat(40)).unwrap()
    }

    #[test]
    fn valid_relative_file_path_is_accepted() {
        assert!(FileIdentity::parse(test_repo_id(), test_commit(), "src/lib.rs").is_ok());
    }

    #[test]
    fn empty_file_path_is_rejected() {
        assert_eq!(
            FileIdentity::parse(test_repo_id(), test_commit(), ""),
            Err(FileIdentityError::Empty)
        );
    }

    #[test]
    fn file_path_with_parent_dir_traversal_is_rejected() {
        assert_eq!(
            FileIdentity::parse(test_repo_id(), test_commit(), "../../etc/passwd"),
            Err(FileIdentityError::ContainsParentRef)
        );
    }

    #[test]
    fn absolute_file_path_is_rejected() {
        assert_eq!(
            FileIdentity::parse(test_repo_id(), test_commit(), "/etc/passwd"),
            Err(FileIdentityError::NotRelative)
        );
    }

    #[test]
    fn backslash_separated_file_path_is_rejected_not_normalized() {
        assert_eq!(
            FileIdentity::parse(test_repo_id(), test_commit(), "src\\lib.rs"),
            Err(FileIdentityError::ContainsBackslash)
        );
    }

    #[test]
    fn oversized_file_path_is_rejected_not_truncated() {
        let huge = "a/".repeat(MAX_RELATIVE_PATH_LEN);
        let result = FileIdentity::parse(test_repo_id(), test_commit(), &huge);
        assert!(matches!(result, Err(FileIdentityError::TooLong { .. })));
    }

    #[test]
    fn file_path_exactly_at_the_length_limit_is_accepted_not_rejected() {
        // Pins the `>` (strictly greater than), not `>=`, boundary: a path
        // of EXACTLY MAX_RELATIVE_PATH_LEN bytes must be accepted. A
        // single `a` repeated has no path-separator/traversal issue, so
        // length is the only thing this exercises.
        let exactly_at_limit = "a".repeat(MAX_RELATIVE_PATH_LEN);
        assert_eq!(exactly_at_limit.len(), MAX_RELATIVE_PATH_LEN);
        let result = FileIdentity::parse(test_repo_id(), test_commit(), &exactly_at_limit);
        assert!(result.is_ok());
    }

    #[test]
    fn file_path_one_byte_over_the_length_limit_is_rejected() {
        let one_over = "a".repeat(MAX_RELATIVE_PATH_LEN + 1);
        let result = FileIdentity::parse(test_repo_id(), test_commit(), &one_over);
        assert_eq!(
            result,
            Err(FileIdentityError::TooLong {
                max: MAX_RELATIVE_PATH_LEN,
                actual: MAX_RELATIVE_PATH_LEN + 1,
            })
        );
    }

    #[test]
    fn accepted_file_path_is_returned_byte_for_byte_with_no_normalization() {
        let id = FileIdentity::parse(test_repo_id(), test_commit(), "src/lib.rs").unwrap();
        assert_eq!(id.relative_path(), "src/lib.rs");
    }
}
