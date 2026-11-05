//! B2 Task 1: `SymbolSnapshotId` — the identity of a symbol within one
//! commit.

use sha2::{Digest, Sha256};

use crate::identity::FileIdentity;
use crate::kind::SymbolKind;

/// An opaque identity for one symbol observed at one commit, derived from
/// `(FileIdentity, SymbolKind, qualified_name, signature)` — never from
/// raw free text alone, and never constructible from a caller-supplied
/// string. The only constructor is [`SymbolSnapshotId::derive`], taking
/// already-validated inputs (a [`FileIdentity`], which itself binds a
/// `RepositoryId` and `CommitHash`). This mirrors `SymbolLineageId`'s
/// "never parsed from a string" discipline for the same reason: nothing
/// about this value is meaningful except as the deterministic output of
/// hashing validated identity, so there is nothing for an external caller
/// to validate by parsing one back.
///
/// Deterministic (not random, unlike `RepositoryId`/`SymbolLineageId`):
/// re-extracting the same symbol at the same commit must yield the same
/// id, so Task 9's incremental reindex can recognize "this is the same
/// symbol as before" without needing a separate lookup table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolSnapshotId([u8; 32]);

impl SymbolSnapshotId {
    /// Derives a `SymbolSnapshotId` from already-validated inputs. Two
    /// calls with equal `file`, `kind`, `qualified_name` and `signature`
    /// always produce the same id; any difference in any field produces a
    /// different one (the hash covers all four).
    pub fn derive(file: &FileIdentity, kind: SymbolKind, qualified_name: &str, signature: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(file.repository().as_uuid().as_bytes());
        hasher.update(file.commit().as_str().as_bytes());
        hasher.update(file.relative_path().as_bytes());
        hasher.update([kind_discriminant(kind)]);
        hasher.update(qualified_name.as_bytes());
        hasher.update(signature.as_bytes());
        let digest = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&digest);
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{CommitHash, RepositoryRegistry, RepositoryRoot};

    fn test_file_identity() -> FileIdentity {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
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
    fn different_qualified_names_produce_different_ids() {
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
}
