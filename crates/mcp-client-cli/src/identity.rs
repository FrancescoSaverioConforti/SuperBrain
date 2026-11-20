//! M6.4 Fork A (A2, approved, narrowed scope): content-based identity
//! verification for the resolved `server.command`/`server.script` files,
//! and schema-hash verification (the fail-closed substitute for M6.5's
//! real schema-binding mechanism, narrowly scoped: one exact hash, one
//! exact match, no versioning or migration).
//!
//! **M6.4-S1 correction (supersedes the original M6.4 TOCTOU framing
//! below)**: hashing alone, followed by ordinary path-based spawning, is
//! NOT race-free, and M6.4's original disclosure understated the
//! window's real size (empirically measured at tens of milliseconds to
//! seconds, growing with audit-log size, not "narrow/millisecond-scale"
//! as first written). M6.4-S1 adds `windows_acl::verify_trusted_location`
//! (Windows only) as the real mitigation: a read-only, conservative
//! existence-scan of the resolved `command`/`script` path's own DACL and
//! every ancestor directory's DACL up to the drive root, rejecting if
//! any ALLOW-granting ACE (of an explicitly recognized type; any
//! unrecognized ACE type, NULL DACL, or malformed security descriptor
//! fails closed) grants a write/delete/re-ACL-shaped right to any
//! trustee outside a fixed allowlist (the current process's own user
//! SID, SYSTEM, Administrators). See `windows_acl`'s own module doc
//! comment for the full, corrected design -- including why this is
//! deliberately NOT a general Windows effective-access simulator (that
//! would require either live logon tokens per principal or a from-
//! scratch reimplementation of ACE-ordering/deny-precedence/group-
//! membership semantics, exactly the class of mistake that risks FALSE
//! ACCEPTANCE, not merely false rejection).
//!
//! **What this DOES establish**: if no allow-grant to a non-allowlisted
//! trustee exists anywhere in the resolved DACL chain, no OTHER Windows
//! principal holds the OS-level permission needed to modify the path --
//! this is an access-control elimination claim for principals outside
//! the allowlist, not a probabilistic "we re-checked right before
//! spawn" narrowing, and not a claim of mathematically race-free
//! execution in general.
//!
//! **What this does NOT establish (stated plainly, per M6.4-S1's own
//! review)**: ZERO protection against a compromised process running
//! under the SAME user account as this CLI -- that process already
//! holds every permission the allowlist grants to "the current user" and
//! can simply write the file directly. No SID-based ACL check can
//! distinguish legitimate operator tooling from malware running as that
//! same operator. Also: content-hash verification covers exactly the
//! named `command`/`script` files, never anything dynamically loaded by
//! the executed code (DLLs, Python's own standard library, any
//! `import`ed third-party package) -- an operator whose script imports
//! third-party packages extends their own trust boundary to that
//! package's install location, which this binary never inspects.
//! Windows-only: this mechanism adds no new Unix-specific hardening: on
//! Unix, `storage::handle_safety`'s existing symlink/hard-link checks
//! remain the sole protection, unchanged by M6.4-S1.
pub const TOCTOU_DISCLOSURE: &str = "Executable/script content verified by hash, then (on Windows) the resolved path and every ancestor directory's own DACL are scanned for any write/delete/re-ACL grant to a principal outside a fixed allowlist (current user, SYSTEM, Administrators); an unrecognized ACE type, NULL DACL, or malformed security descriptor is rejected rather than interpreted. This establishes that no OTHER Windows principal holds the permission needed to replace this path -- it is NOT a claim of race-free execution in general, and it provides ZERO protection against a compromised process running under the SAME user account, which already holds every permission this check allows. On non-Windows platforms this ACL check does not run; only the existing symlink/hard-link verification applies.";

pub const NO_SANDBOX_DISCLOSURE: &str = "This binary provides supervised, operator-reviewed authorization only. It does not sandbox the external process. A tool classified network_required: false or constrained to a filesystem_scope can still technically access anything the OS-level process itself is permitted to access; this binary trusts the external server's own good-faith compliance with what it was invoked to do, backed only by the operator's own prior review -- never by OS-enforced containment.";

use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("failed to read {1} for content verification: {0}")]
    Read(storage::StorageError, &'static str),
    #[error("{label} content hash mismatch: expected {expected}, computed {computed}")]
    HashMismatch { label: &'static str, expected: String, computed: String },
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Reads `path` through the verified `storage::read_trusted_file`
/// primitive and confirms its SHA-256 matches `expected_hex` exactly
/// (case-insensitive comparison, since config authors may paste a hash
/// in either case; the canonical form checked by `config::validate` is
/// lowercase, but this comparison itself tolerates either).
pub fn verify_file_content_hash(path: &Path, expected_hex: &str, label: &'static str) -> Result<(), IdentityError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().map(Path::new).unwrap_or_else(|| Path::new(""));
    let bytes = storage::read_trusted_file(dir, file_name, label).map_err(|e| IdentityError::Read(e, label))?;
    let computed = sha256_hex(&bytes);
    if computed.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err(IdentityError::HashMismatch { label, expected: expected_hex.to_string(), computed })
    }
}

/// Canonicalizes a discovered tool's own `input_schema` the same
/// deterministic way `serde_json::to_string` already does (key-sorted,
/// since `serde_json::Map` is `BTreeMap`-backed with the `preserve_order`
/// feature not enabled anywhere in this workspace -- confirmed by this
/// crate's own `Cargo.toml` and every other crate's, none of which
/// enables it) and returns its SHA-256 hex digest.
pub fn hash_discovered_schema(schema: &serde_json::Map<String, serde_json::Value>) -> String {
    let canonical = serde_json::to_string(schema).expect("a serde_json::Map of already-deserialized JSON values cannot fail to re-serialize");
    sha256_hex(canonical.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_is_a_64_char_lowercase_hex_string() {
        let hash = sha256_hex(b"hello");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn sha256_hex_is_deterministic_and_sensitive_to_every_byte() {
        assert_eq!(sha256_hex(b"hello"), sha256_hex(b"hello"));
        assert_ne!(sha256_hex(b"hello"), sha256_hex(b"hellp"));
    }

    #[test]
    fn verify_file_content_hash_accepts_a_matching_hash_and_rejects_a_mismatch() {
        let dir = std::env::temp_dir().join(format!("mcp-client-cli-identity-test-{}-{}", std::process::id(), uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("binary.bin");
        std::fs::write(&path, b"fake-executable-content").unwrap();
        let real_hash = sha256_hex(b"fake-executable-content");

        assert!(verify_file_content_hash(&path, &real_hash, "<test>").is_ok());

        let wrong_hash = "0".repeat(64);
        let result = verify_file_content_hash(&path, &wrong_hash, "<test>");
        assert!(matches!(result, Err(IdentityError::HashMismatch { .. })), "got {result:?}");
    }

    #[test]
    fn hash_discovered_schema_is_deterministic_for_the_same_logical_content() {
        let schema_a: serde_json::Map<String, serde_json::Value> = serde_json::from_str(r#"{"b":1,"a":2}"#).unwrap();
        let schema_b: serde_json::Map<String, serde_json::Value> = serde_json::from_str(r#"{"a":2,"b":1}"#).unwrap();
        assert_eq!(hash_discovered_schema(&schema_a), hash_discovered_schema(&schema_b), "key order in the ORIGINAL JSON text must not affect the hash, since serde_json::Map is BTreeMap-backed and re-serializes key-sorted regardless of input order");
    }

    #[test]
    fn hash_discovered_schema_differs_for_genuinely_different_schemas() {
        let schema_a: serde_json::Map<String, serde_json::Value> = serde_json::from_str(r#"{"type":"object"}"#).unwrap();
        let schema_b: serde_json::Map<String, serde_json::Value> = serde_json::from_str(r#"{"type":"string"}"#).unwrap();
        assert_ne!(hash_discovered_schema(&schema_a), hash_discovered_schema(&schema_b));
    }
}
