//! B2 Task 2: `RepositoryAllowlist` — the filesystem capability boundary for
//! which repository roots `codegraph-index` may read.
//!
//! Separate responsibility from `codegraph_domain::RepositoryRegistry`
//! (Task 1): the registry answers "what `RepositoryId` does this root
//! have?" (identity), while this type answers "is this root one
//! `LocalGitRepositorySource` is actually permitted to open?" (filesystem
//! capability). Per the approved decision, these stay two separate types
//! rather than being folded into one — `ReadPolicy` controls operation
//! authorization, `RepositoryAllowlist` controls filesystem capability, and
//! the same split applies here one level down: identity is not
//! authorization to read.
//!
//! Supplied by the trusted host (the same boundary that supplies
//! `ReadPolicy` and the `RepositoryRegistry`), constructed once, and never
//! mutated by anything this crate's own code does in response to
//! repository content — there is no method on this type that adds a root
//! from anything other than an explicit caller call with a path the CALLER
//! already decided to trust. In particular: no wildcard pattern, no
//! "current directory" fallback, and no root inferred from a `.git`
//! discovery walk starting anywhere this type does not already contain
//! verbatim.

use std::collections::HashSet;
use std::path::Path;

use thiserror::Error;

/// A canonical, allowlisted repository root. The only way to obtain one
/// outside this crate is [`RepositoryAllowlist::authorize`] — there is no
/// public constructor on the allowlist-membership side that skips
/// canonicalization, mirroring `codegraph_domain::RepositoryRoot`'s own
/// discipline (indeed, this wraps that same type) rather than inventing a
/// second, looser path representation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AllowlistedRoot(codegraph_domain::RepositoryRoot);

impl AllowlistedRoot {
    pub fn as_path(&self) -> &Path {
        self.0.as_path()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AllowlistError {
    /// `root` failed `RepositoryRoot::parse` (not absolute, or not
    /// canonicalizable/readable) — never silently skipped or added anyway.
    #[error("cannot register {path:?} in the allowlist: {reason}")]
    InvalidRoot { path: String, reason: String },
    /// The fail-closed case this allowlist exists for: a caller asked to
    /// read a root that was never explicitly authorized. Carries the
    /// requested path (sanitized to its string form, never resolved
    /// further) for diagnostics — never silently treated as "not found"
    /// in a way that could be confused with "found but empty".
    #[error("root {path:?} is not in the allowlist")]
    NotAuthorized { path: String },
}

/// The set of filesystem roots `codegraph-index` is permitted to read, as
/// canonical, validated paths. Constructed empty; the host adds roots one
/// at a time via [`RepositoryAllowlist::authorize`], each a deliberate,
/// explicit call — there is no bulk-import-from-config convenience here
/// that could make a wildcard or glob look like a single root the caller
/// actually inspected.
#[derive(Debug, Default)]
pub struct RepositoryAllowlist {
    roots: HashSet<AllowlistedRoot>,
}

impl RepositoryAllowlist {
    pub fn new() -> Self {
        Self::default()
    }

    /// Canonicalizes and records `path` as an authorized root. Rejects
    /// (does not authorize) a path that is relative, does not exist, or
    /// cannot be canonicalized — the same `RepositoryRoot::parse` rejection
    /// behavior Task 1 already established, reused here rather than
    /// duplicated.
    pub fn authorize(&mut self, path: &Path) -> Result<AllowlistedRoot, AllowlistError> {
        let root = codegraph_domain::RepositoryRoot::parse(path).map_err(|e| AllowlistError::InvalidRoot {
            path: path.to_string_lossy().into_owned(),
            reason: e.to_string(),
        })?;
        let allowlisted = AllowlistedRoot(root);
        self.roots.insert(allowlisted.clone());
        Ok(allowlisted)
    }

    /// Checks whether `path` resolves to an already-authorized root.
    /// Canonicalizes `path` first (so a differently-spelled but equal
    /// path, e.g. with a trailing separator, still matches an authorized
    /// entry — the same equality `RepositoryRoot`'s own `Hash`/`Eq` already
    /// gives it), then requires an EXACT match against the allowlist set:
    /// a path that canonicalizes to a *descendant* of an authorized root
    /// is not itself automatically authorized by this check. Containment
    /// under an authorized root (for resolving a file within a repository
    /// whose root IS an authorized entry) is a different, narrower
    /// question `LocalGitRepositorySource` answers separately once a
    /// specific authorized root has already been selected — this method
    /// only answers "is this exact path one of the roots the host
    /// authorized," never "is this path somewhere inside one of them."
    pub fn is_authorized(&self, path: &Path) -> bool {
        match codegraph_domain::RepositoryRoot::parse(path) {
            Ok(root) => self.roots.contains(&AllowlistedRoot(root)),
            Err(_) => false,
        }
    }

    /// Returns the authorized root matching `path`, or a typed,
    /// fail-closed [`AllowlistError::NotAuthorized`] — never a silent
    /// `None`/empty-list fallback a caller could mistake for "root exists
    /// but is empty."
    pub fn require_authorized(&self, path: &Path) -> Result<AllowlistedRoot, AllowlistError> {
        let canonical_attempt = codegraph_domain::RepositoryRoot::parse(path);
        let root = match canonical_attempt {
            Ok(root) => root,
            Err(_) => {
                return Err(AllowlistError::NotAuthorized {
                    path: path.to_string_lossy().into_owned(),
                })
            }
        };
        let candidate = AllowlistedRoot(root);
        self.roots
            .get(&candidate)
            .cloned()
            .ok_or_else(|| AllowlistError::NotAuthorized {
                path: path.to_string_lossy().into_owned(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("b2-allowlist-test-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_authorized_root_is_recognized_as_authorized() {
        let dir = make_temp_dir("authorized");
        let mut allowlist = RepositoryAllowlist::new();
        allowlist.authorize(&dir).unwrap();

        assert!(allowlist.is_authorized(&dir));
    }

    #[test]
    fn a_root_never_authorized_is_rejected_fail_closed() {
        let dir = make_temp_dir("never-authorized");
        let allowlist = RepositoryAllowlist::new();

        assert!(!allowlist.is_authorized(&dir));
        assert!(matches!(
            allowlist.require_authorized(&dir),
            Err(AllowlistError::NotAuthorized { .. })
        ));
    }

    #[test]
    fn a_sibling_directory_outside_any_authorized_root_is_rejected() {
        let authorized = make_temp_dir("sibling-authorized");
        let sibling = authorized.parent().unwrap().join(format!("sibling-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&sibling).unwrap();

        let mut allowlist = RepositoryAllowlist::new();
        allowlist.authorize(&authorized).unwrap();

        assert!(!allowlist.is_authorized(&sibling));
    }

    #[test]
    fn a_descendant_of_an_authorized_root_is_not_itself_an_authorized_root_entry() {
        // is_authorized/require_authorized answer "is this EXACT path an
        // authorized root," not "is this path somewhere inside one."
        let authorized = make_temp_dir("descendant-parent");
        let nested = authorized.join("nested-subdir");
        std::fs::create_dir_all(&nested).unwrap();

        let mut allowlist = RepositoryAllowlist::new();
        allowlist.authorize(&authorized).unwrap();

        assert!(!allowlist.is_authorized(&nested));
    }

    #[test]
    fn a_relative_path_is_never_authorized() {
        let mut allowlist = RepositoryAllowlist::new();
        let result = allowlist.authorize(Path::new("relative/path"));
        assert!(matches!(result, Err(AllowlistError::InvalidRoot { .. })));
    }

    #[test]
    fn a_nonexistent_path_cannot_be_authorized() {
        let mut allowlist = RepositoryAllowlist::new();
        let nonexistent = std::env::temp_dir().join(format!("does-not-exist-{}", uuid::Uuid::new_v4()));
        let result = allowlist.authorize(&nonexistent);
        assert!(matches!(result, Err(AllowlistError::InvalidRoot { .. })));
    }

    #[test]
    fn registering_the_same_root_twice_does_not_duplicate_or_error() {
        let dir = make_temp_dir("idempotent");
        let mut allowlist = RepositoryAllowlist::new();

        assert!(allowlist.authorize(&dir).is_ok());
        assert!(allowlist.authorize(&dir).is_ok());
        assert!(allowlist.is_authorized(&dir));
    }

    #[cfg(windows)]
    #[test]
    fn windows_case_insensitive_spelling_of_an_authorized_root_is_still_recognized() {
        // RepositoryRoot::parse canonicalizes (Task 1's documented
        // departure from NoteId's never-normalize rule, specifically
        // because a RepositoryRoot is a lookup key, not an on-disk
        // filename identity). On Windows this means a different-cased
        // spelling of the SAME real directory canonicalizes to the SAME
        // path, so it must still be recognized as the already-authorized
        // root, not treated as a different, unauthorized one.
        let dir = make_temp_dir("casing");
        let mut allowlist = RepositoryAllowlist::new();
        allowlist.authorize(&dir).unwrap();

        let dir_str = dir.to_string_lossy().into_owned();
        let upper = PathBuf::from(dir_str.to_uppercase());

        // Only assert this if uppercasing the whole path still names a
        // real location on this filesystem (it does on Windows's
        // case-insensitive default filesystems); otherwise this would be
        // asserting about a path that cannot exist, which is a different
        // test entirely.
        if upper.exists() {
            assert!(allowlist.is_authorized(&upper));
        }
    }
}
