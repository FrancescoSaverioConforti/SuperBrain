//! B2 Task 1/5/finding B2-ID-001: `SymbolSnapshotId` — the identity of a
//! symbol within one commit.
//!
//! # Encoding schema `superbrain-symbol-snapshot-v2`
//!
//! See [`docs/research/findings/B2-ID-001.md`](../../../../docs/research/findings/B2-ID-001.md)
//! for the full research write-up. Summary: the `v1` scheme (Task 5)
//! hashed `(RepositoryId, CommitHash, relative_path, SymbolKind, name,
//! signature)` with no notion of the symbol's own position in the file.
//! A real-repository smoke test against `pytorch/examples` found two
//! textually-identical methods (`init_weights`, byte-identical bodies)
//! declared in two DIFFERENT enclosing classes in the same file —
//! genuinely distinct symbols that `v1` hashed to the SAME id, which
//! `CodeGraph`'s own duplicate-id rejection (correctly) then refused to
//! accept. `v2` adds the symbol's own `ByteSpan` (start/end byte offsets
//! within the file) to the hash: two distinct syntactic occurrences in
//! the same file necessarily occupy different byte ranges, so adding
//! the span makes within-file collisions structurally impossible
//! (subject to the same non-cryptographic caveat as `v1` — this guards
//! against OUR OWN encoding producing a false collision, not against an
//! adversarial SHA-256 collision). Cross-commit continuity for a symbol
//! whose span moves is `SymbolLineageId`'s job, not this type's — see
//! the finding for why `ByteSpan` was chosen over a `QualifiedName`
//! (scope-chain) alternative.
//!
//! Task 1's ORIGINAL `derive` (superseded before `v2`, by the `v1`
//! history already in this file) hashed its fields via consecutive
//! `hasher.update(field.as_bytes())` calls with no field-boundary
//! encoding at all — `("ab", "c")` and `("a", "bc")` for two adjacent
//! variable-length fields produced the exact same byte stream. `v1`
//! fixed that with length-prefixing. `v2` keeps `v1`'s length-prefixing
//! discipline and extends the field set — not a second, parallel
//! `derive_v3`, because nothing has yet persisted a `v1` id anywhere (no
//! storage, no API, no stable format existed to migrate), so there is no
//! real backward-compatibility obligation to preserve.
//!
//! The `v2` encoding, byte-stream order:
//!
//! 1. Domain/version separator: the literal ASCII bytes
//!    `b"superbrain-symbol-snapshot-v2"`, unprefixed, first in the
//!    stream. Its purpose is to ensure this hash space can never
//!    collide with a hash of the same logical fields computed for a
//!    different purpose or a future `v3` schema, even if a future
//!    schema reused the same field values — changing ANY byte of this
//!    marker (including bumping the version) changes every derived id.
//!    Bumped from `v1` to `v2` because the canonical field set itself
//!    changed (span added), not merely because of an encoding tweak.
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
//! 8. The symbol's own source span, `span_start` then `span_end`: each a
//!    fixed-width 8-byte little-endian `u64`, no length prefix needed
//!    since their width never varies. Two different occurrences of
//!    otherwise-identical text in the same file necessarily have
//!    different `(span_start, span_end)` pairs (byte ranges in one file
//!    cannot both start AND end at the same offsets unless they are the
//!    same occurrence), which is exactly the property this field closes
//!    the collision on.
//!
//! **Length-prefixed** means: an 8-byte little-endian `u64` giving the
//! field's byte length, immediately followed by that many bytes of the
//! field itself — `field_length || field_bytes`, exactly the task's
//! specified form. This makes the byte boundary between any two
//! consecutive variable-length fields unambiguous: reconstructing where
//! one field ends and the next begins requires no scanning or
//! delimiter, only reading the fixed-width length that precedes it, so
//! no choice of delimiter character (which could itself appear inside a
//! field) is ever at stake. Repository id, symbol kind, and the span's
//! two offsets are fixed-width and therefore carry no length prefix —
//! their byte count never varies, so there is nothing a boundary
//! ambiguity could exploit for them.
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

/// The `superbrain-symbol-snapshot-v2` domain/version separator — the
/// first bytes fed to every `SymbolSnapshotId::derive` call's hasher. See
/// this module's doc comment for why it exists and what changing it
/// would mean (every derived id changes). Bumped from `v1` by finding
/// B2-ID-001 when the canonical field set itself changed (span added).
const DOMAIN_SEPARATOR_V2: &[u8] = b"superbrain-symbol-snapshot-v2";

/// An opaque identity for one symbol observed at one commit, derived from
/// `(RepositoryId, CommitHash, relative path, SymbolKind, validated
/// name, signature, span_start, span_end)` under the
/// `superbrain-symbol-snapshot-v2` encoding — never from raw free text
/// alone, and never constructible from a caller-supplied string. The
/// only constructor is [`SymbolSnapshotId::derive`], taking already-
/// validated inputs (a [`FileIdentity`], which itself binds a
/// `RepositoryId` and `CommitHash`). This mirrors `SymbolLineageId`'s
/// "never parsed from a string" discipline for the same reason: nothing
/// about this value is meaningful except as the deterministic output of
/// hashing validated identity under this specific encoding, so there is
/// nothing for an external caller to validate by parsing one back.
///
/// Deterministic (not random, unlike `RepositoryId`/`SymbolLineageId`):
/// re-extracting the same symbol at the same commit, at the same source
/// position, must yield the same id, so Task 9's incremental reindex can
/// recognize "this is the same symbol as before" without needing a
/// separate lookup table. `span_start`/`span_end` are part of this
/// identity (see this module's doc comment, finding B2-ID-001) — moving
/// a symbol within its own file changes its id, which is intentional:
/// cross-commit continuity through a move is `SymbolLineageId`'s job,
/// never this type's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolSnapshotId([u8; 32]);

impl SymbolSnapshotId {
    /// Derives a `SymbolSnapshotId` under the `superbrain-symbol-snapshot-v2`
    /// encoding (this module's doc comment). Two calls with equal `file`,
    /// `kind`, `name`, `signature`, `span_start` and `span_end` always
    /// produce the same id; any difference in any field, or in the
    /// boundary between two adjacent variable-length fields, produces a
    /// different one. `span_start`/`span_end` are the symbol's own
    /// zero-based, end-exclusive byte offsets within the file at
    /// `file.commit()` (`extract::ByteSpan`'s convention) — the caller
    /// (`promote`) has already rejected `span_start >= span_end` before
    /// calling this, but `derive` itself does not re-validate that
    /// ordering: it is a pure hash over whatever `u64` values it is
    /// given, not a span-validity check.
    pub fn derive(file: &FileIdentity, kind: SymbolKind, name: &str, signature: &str, span_start: u64, span_end: u64) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN_SEPARATOR_V2);
        hasher.update(file.repository().as_uuid().as_bytes());
        update_length_prefixed(&mut hasher, file.commit().as_str().as_bytes());
        update_length_prefixed(&mut hasher, file.relative_path().as_bytes());
        hasher.update([kind_discriminant(kind)]);
        update_length_prefixed(&mut hasher, name.as_bytes());
        update_length_prefixed(&mut hasher, signature.as_bytes());
        hasher.update(span_start.to_le_bytes());
        hasher.update(span_end.to_le_bytes());
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
        let a = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::bar", "fn bar()", 0, 10);
        let b = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::bar", "fn bar()", 0, 10);
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
        let a = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::bar", "fn bar()", 0, 10);
        let b = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::baz", "fn bar()", 0, 10);
        assert_ne!(a, b);
    }

    #[test]
    fn different_signatures_produce_different_ids() {
        let file = test_file_identity();
        let a = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::bar", "fn bar()", 0, 10);
        let b = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::bar", "fn bar(x: u32)", 0, 14);
        assert_ne!(a, b);
    }

    #[test]
    fn different_kinds_produce_different_ids_for_otherwise_identical_inputs() {
        let file = test_file_identity();
        let a = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo::bar", "fn bar()", 0, 10);
        let b = SymbolSnapshotId::derive(&file, SymbolKind::Method, "foo::bar", "fn bar()", 0, 10);
        assert_ne!(a, b);
    }

    #[test]
    fn different_files_produce_different_ids_for_otherwise_identical_inputs() {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file_a = FileIdentity::parse(repo, commit.clone(), "src/lib.rs").unwrap();
        let file_b = FileIdentity::parse(repo, commit, "src/other.rs").unwrap();

        let a = SymbolSnapshotId::derive(&file_a, SymbolKind::Function, "foo::bar", "fn bar()", 0, 10);
        let b = SymbolSnapshotId::derive(&file_b, SymbolKind::Function, "foo::bar", "fn bar()", 0, 10);
        assert_ne!(a, b);
    }

    #[test]
    fn same_symbol_in_different_repositories_produces_different_ids() {
        let mut registry = RepositoryRegistry::new();
        let repo_a = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let repo_b = registry.register(RepositoryRoot::parse(&std::env::temp_dir()).unwrap());

        let file_a = file_identity_with_repo(repo_a);
        let file_b = file_identity_with_repo(repo_b);

        let a = SymbolSnapshotId::derive(&file_a, SymbolKind::Function, "foo::bar", "fn bar()", 0, 10);
        let b = SymbolSnapshotId::derive(&file_b, SymbolKind::Function, "foo::bar", "fn bar()", 0, 10);
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

        let a = SymbolSnapshotId::derive(&file_a, SymbolKind::Function, "foo::bar", "fn bar()", 0, 10);
        let b = SymbolSnapshotId::derive(&file_b, SymbolKind::Function, "foo::bar", "fn bar()", 0, 10);
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
        let a = SymbolSnapshotId::derive(&file, SymbolKind::Function, "ab", "c", 0, 3);
        let b = SymbolSnapshotId::derive(&file, SymbolKind::Function, "a", "bc", 0, 3);
        assert_ne!(a, b, "(\"ab\",\"c\") and (\"a\",\"bc\") must not collide under length-prefixed encoding");
    }

    // --- B2-ID-001 regression coverage: span is now part of identity ---

    #[test]
    fn identical_everything_but_different_span_start_produces_different_ids() {
        let file = test_file_identity();
        let a = SymbolSnapshotId::derive(&file, SymbolKind::Function, "init_weights", "def init_weights(self): pass", 100, 200);
        let b = SymbolSnapshotId::derive(&file, SymbolKind::Function, "init_weights", "def init_weights(self): pass", 300, 200);
        assert_ne!(a, b, "a different span_start alone must change the id");
    }

    #[test]
    fn identical_everything_but_different_span_end_produces_different_ids() {
        let file = test_file_identity();
        let a = SymbolSnapshotId::derive(&file, SymbolKind::Function, "init_weights", "def init_weights(self): pass", 100, 200);
        let b = SymbolSnapshotId::derive(&file, SymbolKind::Function, "init_weights", "def init_weights(self): pass", 100, 250);
        assert_ne!(a, b, "a different span_end alone must change the id");
    }

    #[test]
    fn identical_span_and_every_other_field_produces_the_same_id() {
        let file = test_file_identity();
        let a = SymbolSnapshotId::derive(&file, SymbolKind::Function, "init_weights", "def init_weights(self): pass", 100, 200);
        let b = SymbolSnapshotId::derive(&file, SymbolKind::Function, "init_weights", "def init_weights(self): pass", 100, 200);
        assert_eq!(a, b, "fully identical input, including span, must still be deterministic");
    }

    #[test]
    fn b2_id_001_regression_two_methods_with_identical_name_and_body_in_different_classes_get_distinct_ids() {
        // The exact real-repository scenario (pytorch/examples,
        // word_language_model/model.py): RNNModel.init_weights and
        // TransformerModel.init_weights, byte-identical bodies, same
        // file, same SymbolKind, same local name — but at two genuinely
        // different byte offsets in the file, since they are two
        // distinct syntactic occurrences. Before B2-ID-001's fix (v1,
        // no span in the hash), these collided. After the fix, the
        // differing span alone must be enough to separate them.
        let file = test_file_identity();
        let body = "def init_weights(self):\n    initrange = 0.1\n    nn.init.uniform_(self.encoder.weight, -initrange, initrange)\n";
        let rnn_model_init_weights = SymbolSnapshotId::derive(&file, SymbolKind::Method, "init_weights", body, 850, 975);
        let transformer_model_init_weights = SymbolSnapshotId::derive(&file, SymbolKind::Method, "init_weights", body, 3100, 3225);
        assert_ne!(
            rnn_model_init_weights, transformer_model_init_weights,
            "two distinct methods with identical name+body in different classes must not collide (B2-ID-001)"
        );
    }

    #[test]
    fn golden_vector_matches_an_independently_recomputed_v2_encoding() {
        // Pins the ENCODING ITSELF (domain separator, field order,
        // length-prefix width/endianness, kind discriminant, span
        // field width/endianness) against accidental future drift, by
        // recomputing the expected digest from the same raw ingredients
        // via a SEPARATE, independent implementation of the v2 scheme
        // written directly in this test (not by calling
        // `update_length_prefixed`/`kind_discriminant`, which would
        // make this test tautological against a mutant that changes
        // both `derive` and this test's helpers identically). A true
        // cross-run hex constant isn't possible for the full id without
        // a frozen `RepositoryId`, which by design (`identity.rs`'s doc
        // comment) has no public constructor from a raw UUID — so this
        // test fixes the repository id's bytes to a known value via the
        // only sanctioned path (register a fresh registry here, read
        // back its real UUID) and builds the expected digest around
        // that same captured value.
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"0".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, "src/golden.rs").unwrap();

        let actual = SymbolSnapshotId::derive(&file, SymbolKind::Function, "golden::vector", "fn golden_vector()", 42, 61);

        // Independent re-implementation of the v2 scheme, written fresh
        // here rather than reusing `derive`'s own helpers.
        let mut hasher = Sha256::new();
        hasher.update(b"superbrain-symbol-snapshot-v2");
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
        hasher.update(42u64.to_le_bytes());
        hasher.update(61u64.to_le_bytes());
        let expected_digest = hasher.finalize();

        assert_eq!(actual.as_bytes().as_slice(), expected_digest.as_slice());
    }

    #[test]
    fn domain_separator_is_explicitly_v2_not_v1() {
        // Pins that the version marker ACTUALLY changed (not just the
        // doc comment claiming it did) — a mutant that reverted the
        // byte string back to "...v1" while keeping every other v2
        // behavior would otherwise silently reintroduce cross-scheme
        // ambiguity.
        let file = test_file_identity();
        let v2_id = SymbolSnapshotId::derive(&file, SymbolKind::Function, "foo", "fn foo()", 0, 8);

        let mut v1_style_hasher = Sha256::new();
        v1_style_hasher.update(b"superbrain-symbol-snapshot-v1");
        v1_style_hasher.update(file.repository().as_uuid().as_bytes());
        update_length_prefixed(&mut v1_style_hasher, file.commit().as_str().as_bytes());
        update_length_prefixed(&mut v1_style_hasher, file.relative_path().as_bytes());
        v1_style_hasher.update([0u8]);
        update_length_prefixed(&mut v1_style_hasher, b"foo");
        update_length_prefixed(&mut v1_style_hasher, b"fn foo()");
        let v1_style_digest = v1_style_hasher.finalize();

        assert_ne!(v2_id.as_bytes().as_slice(), v1_style_digest.as_slice(), "the v2 domain separator must change every derived id relative to the old v1 scheme");
    }
}
