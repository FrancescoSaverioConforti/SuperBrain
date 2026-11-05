//! B2 Task 1/5: `SymbolSnapshotId` — the identity of a symbol within one
//! commit.
//!
//! # Encoding schema `superbrain-symbol-snapshot-v1`
//!
//! Task 1's original `derive` hashed its fields via consecutive
//! `hasher.update(field.as_bytes())` calls with no field-boundary
//! encoding at all — a structural flaw, not a hash-algorithm weakness:
//! `("ab", "c")` and `("a", "bc")` for two adjacent variable-length
//! fields produce the exact same byte stream fed to the hash, and are
//! therefore indistinguishable regardless of how strong the underlying
//! hash is. Task 5 replaces that scheme entirely — not with a second,
//! parallel `derive_v2`, because nothing has yet persisted a Task 1 id
//! anywhere (no storage, no API, no stable format existed to migrate),
//! so there is no real backward-compatibility obligation to preserve.
//!
//! The `v1` encoding, byte-stream order:
//!
//! 1. Domain/version separator: the literal ASCII bytes
//!    `b"superbrain-symbol-snapshot-v1"`, unprefixed, first in the
//!    stream. Its purpose is to ensure this hash space can never
//!    collide with a hash of the same logical fields computed for a
//!    different purpose or a future `v2` schema, even if a future
//!    schema reused the same field values — changing ANY byte of this
//!    marker (including bumping the version) changes every derived id.
//! 2. `RepositoryId`'s UUID: fixed-width 16 bytes (`Uuid::as_bytes()`),
//!    needs no length prefix since its width never varies.
//! 3. `CommitHash`'s canonical string: length-prefixed (see below) —
//!    its width DOES vary (40 vs. 64 hex chars for SHA-1 vs. SHA-256).
//! 4. `FileIdentity`'s relative path: length-prefixed.
//! 5. `SymbolKind`'s discriminant: fixed-width 1 byte (`kind_discriminant`,
//!    an explicit, hand-maintained table — never `kind as u8` on the
//!    enum's own declaration order, so inserting a new `SymbolKind`
//!    variant can never silently renumber an existing one and therefore
//!    never silently change an existing id).
//! 6. The validated/local name: length-prefixed.
//! 7. The bounded signature text: length-prefixed.
//!
//! **Length-prefixed** means: an 8-byte little-endian `u64` giving the
//! field's byte length, immediately followed by that many bytes of the
//! field itself — `field_length || field_bytes`, exactly the task's
//! specified form. This makes the byte boundary between any two
//! consecutive variable-length fields unambiguous: reconstructing where
//! one field ends and the next begins requires no scanning or
//! delimiter, only reading the fixed-width length that precedes it, so
//! no choice of delimiter character (which could itself appear inside a
//! field) is ever at stake. Repository id and symbol kind are
//! fixed-width and therefore carry no length prefix — their byte count
//! never varies, so there is nothing a boundary ambiguity could exploit
//! for them.
//!
//! `FileIdentity`'s `repository()`/`commit()` fields are included
//! explicitly rather than assumed "already covered" by anything else:
//! `FileIdentity` itself carries no separate hash of its own fields that
//! this derivation could delegate to, so every one of its three
//! components (`RepositoryId`, `CommitHash`, relative path) is fed to
//! the hasher directly, in the fixed order above, and documented here as
//! the complete set of bytes this derivation depends on.

use sha2::{Digest, Sha256};

use crate::identity::FileIdentity;
use crate::kind::SymbolKind;

/// The `superbrain-symbol-snapshot-v1` domain/version separator — the
/// first bytes fed to every `SymbolSnapshotId::derive` call's hasher. See
/// this module's doc comment for why it exists and what changing it
/// would mean (every derived id changes).
const DOMAIN_SEPARATOR_V1: &[u8] = b"superbrain-symbol-snapshot-v1";

/// An opaque identity for one symbol observed at one commit, derived from
/// `(RepositoryId, CommitHash, relative path, SymbolKind, validated
/// name, signature)` under the `superbrain-symbol-snapshot-v1` encoding
/// — never from raw free text alone, and never constructible from a
/// caller-supplied string. The only constructor is
/// [`SymbolSnapshotId::derive`], taking already-validated inputs (a
/// [`FileIdentity`], which itself binds a `RepositoryId` and
/// `CommitHash`). This mirrors `SymbolLineageId`'s "never parsed from a
/// string" discipline for the same reason: nothing about this value is
/// meaningful except as the deterministic output of hashing validated
/// identity under this specific encoding, so there is nothing for an
/// external caller to validate by parsing one back.
///
/// Deterministic (not random, unlike `RepositoryId`/`SymbolLineageId`):
/// re-extracting the same symbol at the same commit must yield the same
/// id, so Task 9's incremental reindex can recognize "this is the same
/// symbol as before" without needing a separate lookup table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolSnapshotId([u8; 32]);

impl SymbolSnapshotId {
    /// Derives a `SymbolSnapshotId` under the `superbrain-symbol-snapshot-v1`
    /// encoding (this module's doc comment). Two calls with equal `file`,
    /// `kind`, `name` and `signature` always produce the same id; any
    /// difference in any field, or in the boundary between two adjacent
    /// variable-length fields, produces a different one.
    pub fn derive(file: &FileIdentity, kind: SymbolKind, name: &str, signature: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN_SEPARATOR_V1);
        hasher.update(file.repository().as_uuid().as_bytes());
        update_length_prefixed(&mut hasher, file.commit().as_str().as_bytes());
        update_length_prefixed(&mut hasher, file.relative_path().as_bytes());
        hasher.update([kind_discriminant(kind)]);
        update_length_prefixed(&mut hasher, name.as_bytes());
        update_length_prefixed(&mut hasher, signature.as_bytes());
        let digest = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&digest);
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Feeds `field_length || field_bytes` into `hasher`: an 8-byte
/// little-endian length prefix, then the bytes themselves. The one
/// primitive every variable-length field in [`SymbolSnapshotId::derive`]
/// goes through, so the encoding rule lives in exactly one place.
fn update_length_prefixed(hasher: &mut Sha256, field: &[u8]) {
    hasher.update((field.len() as u64).to_le_bytes());
    hasher.update(field);
}

/// A fixed, explicit discriminant per `SymbolKind` variant for hashing —
/// not `kind as u8` on a `#[repr(u8)]` enum, so adding a new `SymbolKind`
/// variant later cannot silently shift every existing variant's
/// discriminant (and therefore every existing `SymbolSnapshotId`) just by
/// where it is inserted in the enum's declaration order.
fn kind_discriminant(kind: SymbolKind) -> u8 {
    match kind {
        SymbolKind::Function => 0,
        SymbolKind::Method => 1,
        SymbolKind::Class => 2,
        SymbolKind::Struct => 3,
        SymbolKind::Enum => 4,
        SymbolKind::Interface => 5,
        SymbolKind::Module => 6,
        SymbolKind::Const => 7,
        SymbolKind::Macro => 8,
        // Added at Task 5. Existing discriminants above are UNCHANGED —
        // appending new ones here, never renumbering, is exactly why this
        // table exists instead of `kind as u8`.
        SymbolKind::MethodSignature => 9,
        SymbolKind::HtmlIdElement => 10,
        SymbolKind::CssRule => 11,
        SymbolKind::CssKeyframes => 12,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{CommitHash, RepositoryId, RepositoryRegistry, RepositoryRoot};

    fn test_file_identity() -> FileIdentity {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        FileIdentity::parse(repo, commit, "src/lib.rs").unwrap()
    }

    fn file_identity_with_repo(repo: RepositoryId) -> FileIdentity {
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        FileIdentity::parse(repo, commit, "src/lib.rs").unwrap()
    }

    #[test]
    fn identical_inputs_produce_identical_ids() {
        let file = test_file_identity();
        let a = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::bar", "fn bar()");
        let b = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::bar", "fn bar()");
        assert_eq!(a, b);
        // Pins `as_bytes` itself (not just `PartialEq`, which could pass
        // against a mutant that replaces `as_bytes` with a constant leaked
        // array as long as both calls returned the SAME constant): the
        // actual bytes must differ from both an all-zero and an all-one
        // 32-byte array, since a SHA-256 digest of non-trivial input
        // collapsing to either is astronomically unlikely.
        assert_ne!(a.as_bytes(), &[0u8; 32]);
        assert_ne!(a.as_bytes(), &[1u8; 32]);
    }

    #[test]
    fn different_names_produce_different_ids() {
        let file = test_file_identity();
        let a = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::bar", "fn bar()");
        let b = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::baz", "fn bar()");
        assert_ne!(a, b);
    }

    #[test]
    fn different_signatures_produce_different_ids() {
        let file = test_file_identity();
        let a = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::bar", "fn bar()");
        let b = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::bar", "fn bar(x: u32)");
        assert_ne!(a, b);
    }

    #[test]
    fn different_kinds_produce_different_ids_for_otherwise_identical_inputs() {
        let file = test_file_identity();
        let a = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::bar", "fn bar()");
        let b = SymbolSnapshotId::derive(&file, SymbolKind::Method, "foo::bar", "fn bar()");
        assert_ne!(a, b);
    }

    #[test]
    fn different_files_produce_different_ids_for_otherwise_identical_inputs() {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap();
        let file_b = FileIdentity::parse(repo, commit, "src/other.rs").unwrap();

        let a = SymbolSnapshotId::derive(&file_a, SymbolKind::Function, "foo::bar", "fn bar()");
        let b = SymbolSnapshotId::derive(&file_b, SymbolKind::Function, "foo::bar", "fn bar()");
        assert_ne!(a, b);
    }

    #[test]
    fn same_symbol_in_different_repositories_produces_different_ids() {
        let mut registry = RepositoryRegistry::new();
        let repo_a = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let repo_b = registry.register(RepositoryRoot::parse(&std::env::temp_dir()).unwrap());

        let file_a = file_identity_with_repo(repo_a);
        let file_b = file_identity_with_repo(repo_b);

        let a = SymbolSnapshotId::derive(&file_a, SymbolKind::Function, "foo::bar", "fn bar()");
        let b = SymbolSnapshotId::derive(&file_b, SymbolKind::Function, "foo::bar", "fn bar()");
        assert_ne!(a, b, "the same symbol text in two different repositories must not collide");
    }

    #[test]
    fn same_symbol_in_different_commits_produces_different_ids() {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
        let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit_a, "src/lib.rs").unwrap();
        let file_b = FileIdentity::parse(repo, commit_b, "src/lib.rs").unwrap();

        let a = SymbolSnapshotId::derive(&file_a, SymbolKind::Function, "foo::bar", "fn bar()");
        let b = SymbolSnapshotId::derive(&file_b, SymbolKind::Function, "foo::bar", "fn bar()");
        assert_ne!(a, b, "the same symbol at two different commits must not collide");
    }

    #[test]
    fn structural_concatenation_collision_is_prevented_by_length_prefixing() {
        // The exact scenario the length-prefix encoding exists to
        // prevent: ("ab", "c") and ("a", "bc") for two adjacent
        // variable-length fields (name, signature) must NOT collide,
        // even though naive unprefixed concatenation of their bytes
        // ("ab" + "c" == "a" + "bc" == "abc") would make them
        // indistinguishable.
        let file = test_file_identity();
        let a = SymbolSnapshotId::derive(&file, SymbolKind::Function, "ab", "c");
        let b = SymbolSnapshotId::derive(&file, SymbolKind::Function, "a", "bc");
        assert_ne!(a, b, "(\"ab\",\"c\") and (\"a\",\"bc\") must not collide under length-prefixed encoding");
    }

    #[test]
    fn golden_vector_matches_an_independently_recomputed_v1_encoding() {
        // Pins the ENCODING ITSELF (domain separator, field order,
        // length-prefix width/endianness, kind discriminant) against
        // accidental future drift, by recomputing the expected digest
        // from the same raw ingredients via a SEPARATE, independent
        // implementation of the v1 scheme written directly in this test
        // (not by calling `update_length_prefixed`/`kind_discriminant`,
        // which would make this test tautological against a mutant that
        // changes both `derive` and this test's helpers identically). A
        // true cross-run hex constant isn't possible for the full id
        // without a frozen `RepositoryId`, which by design
        // (`identity.rs`'s doc comment) has no public constructor from a
        // raw UUID — so this test fixes the repository id's bytes to a
        // known value via the only sanctioned path (register a fresh
        // registry here, read back its real UUID) and builds the
        // expected digest around that same captured value.
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"0".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, "src/golden.rs").unwrap();

        let actual = SymbolSnapshotId::derive(&file, SymbolKind::Function, "golden::vector", "fn golden_vector()");

        // Independent re-implementation of the v1 scheme, written fresh
        // here rather than reusing `derive`'s own helpers.
        let mut hasher = Sha256::new();
        hasher.update(b"superbrain-symbol-snapshot-v1");
        hasher.update(repo.as_uuid().as_bytes());
        let commit_str = "0".repeat(40);
        hasher.update((commit_str.len() as u64).to_le_bytes());
        hasher.update(commit_str.as_bytes());
        let path = "src/golden.rs";
        hasher.update((path.len() as u64).to_le_bytes());
        hasher.update(path.as_bytes());
        hasher.update([0u8]); // SymbolKind::Function's discriminant, per the table above
        let name = "golden::vector";
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        let signature = "fn golden_vector()";
        hasher.update((signature.len() as u64).to_le_bytes());
        hasher.update(signature.as_bytes());
        let expected_digest = hasher.finalize();

        assert_eq!(actual.as_bytes().as_slice(), expected_digest.as_slice());
    }
}
