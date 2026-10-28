//! SuperBrain storage layer.
//!
//! # Public write surface
//!
//! [`propose_and_commit`] is the **only** way for code outside this crate to
//! put bytes on disk through `storage`. It validates the note id, parses and
//! link-checks the frontmatter, runs `policy::Gate::evaluate`, and only then
//! calls the crate-private write primitive under the per-note
//! optimistic-concurrency check.
//!
//! The raw primitives behind it — `atomic_write` (pure filesystem mechanics:
//! no id validation, no gate, no version check) and
//! `Vault::write_record_unchecked` (no gate) — are `pub(crate)` and are not
//! re-exported. The doctests below are the regression guard for that: each
//! `compile_fail` block must fail to compile, so if anyone ever makes one of
//! these primitives reachable from another crate again, `cargo test --doc -p
//! storage` fails loudly.
//!
//! A `compile_fail` doctest passes if compilation fails for *any* reason, and
//! stable rustdoc does not enforce the `E0xxx` codes pinned on each block
//! (only nightly does; they are kept as documentation of the intended
//! failure). What keeps these blocks honest on stable is the positive control
//! below: it uses exactly the same setup expressions (`Vault::open`,
//! `NoteId::parse`, `Version`, `uuid::Uuid::new_v4`, `dir.join`, a byte
//! slice) and must compile and run, so if the setup ever stops compiling the
//! control fails too, and the `compile_fail` blocks can only be failing
//! because of the forbidden item itself.
//!
//! Positive control (compiles and runs; the gate rejects the malformed
//! frontmatter, so nothing is written):
//!
//! ```
//! use std::collections::HashSet;
//! let dir = std::env::temp_dir().join(format!("superbrain-doctest-control-{}", uuid::Uuid::new_v4()));
//! std::fs::create_dir_all(&dir).unwrap();
//! let vault = storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap();
//! let id = storage::NoteId::parse("note-a").unwrap();
//! let _ = vault.read_note(&id);
//! let gate = policy::Gate::new(policy::SecretPolicy::new(1));
//! let known: HashSet<storage::NoteId> = HashSet::new();
//! let pending: HashSet<String> = HashSet::new();
//! let pii_store = vault_security::PiiStore::open(dir.join(".pii-store"), &dir.join(".pii-store").join("master.key")).unwrap();
//! let result = storage::propose_and_commit(
//!     &vault, "not: frontmatter", storage::Version(0), uuid::Uuid::new_v4(), &known, &pending, &gate, &pii_store,
//! );
//! assert!(matches!(result, Err(storage::ProposeError::Rejected(_))));
//! let _unused: (&std::path::Path, &[u8]) = (&dir.join("x"), b"bypass");
//! ```
//!
//! `atomic_write` is not reachable at the crate root:
//!
//! ```compile_fail,E0425
//! let dir = std::env::temp_dir();
//! let _ = storage::atomic_write(&dir.join("x"), b"bypass");
//! ```
//!
//! ...nor through its (private) defining module:
//!
//! ```compile_fail,E0603
//! let dir = std::env::temp_dir();
//! let _ = storage::atomic::atomic_write(&dir.join("x"), b"bypass");
//! ```
//!
//! `AtomicWriteError` is not part of the public surface either:
//!
//! ```compile_fail,E0425
//! let _unused: Option<storage::AtomicWriteError> = None;
//! ```
//!
//! `Vault::write_record_unchecked` (the ungated primitive) is private to
//! this crate:
//!
//! ```compile_fail,E0624
//! let dir = std::env::temp_dir().join(format!("superbrain-doctest-bypass-{}", uuid::Uuid::new_v4()));
//! std::fs::create_dir_all(&dir).unwrap();
//! let vault = storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap();
//! let id = storage::NoteId::parse("note-a").unwrap();
//! let _ = vault.write_record_unchecked(&id, "bypass", storage::Version(0), uuid::Uuid::new_v4());
//! ```

mod atomic;
mod fault;
mod git;
mod journal;
mod note_id;
mod propose;
mod recovery;
mod vault;

// `atomic` is deliberately NOT re-exported: `atomic_write` is a raw,
// ungated, unvalidated filesystem primitive and must stay crate-private (see
// the crate docs above and Finding C1 of the A1 final review).
pub use note_id::{NoteId, NoteIdError};
pub use propose::{propose_and_commit, CommitOutcome, CommitResult, ProposeError, SENSITIVE_TOMBSTONE_BODY};
pub use vault::{StorageError, Vault, VaultRecord, Version};

#[test]
fn crate_builds() { assert!(true); }
