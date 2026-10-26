mod atomic;
mod note_id;
mod propose;
mod vault;

pub use atomic::{atomic_write, AtomicWriteError};
pub use note_id::{NoteId, NoteIdError};
pub use propose::{propose_and_commit, ProposeError};
pub use vault::{StorageError, Vault, VaultRecord, Version};

#[test]
fn crate_builds() { assert!(true); }
