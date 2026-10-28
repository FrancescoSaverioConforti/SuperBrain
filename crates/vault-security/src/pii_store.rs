//! Encrypted PII store for `Sensitive`-classified note content.
//!
//! `PiiStore` is the second half of the single, crash-recoverable
//! sensitive-write pipeline that `storage` (Task 6) drives: `storage` owns
//! the ONE journaled `WriteState` state machine, and `PiiStore` provides two
//! durable, individually fsynced steps — [`PiiStore::stage`] and
//! [`PiiStore::commit_staged`] — that `storage` calls from INSIDE its own
//! `write_record_unchecked`-family pipeline, checking its own `FaultPoint`s
//! between them. `PiiStore` itself has no notion of an "operation" that
//! spans a crash: it is not a second independently-recovered store. The
//! journal is the single source of truth for what the pipeline was doing;
//! `PiiStore::find_orphans` exists only to let an external auditor (Task 3's
//! audit, Task 8) cross-check the live `.pii` files against the set of
//! pointers the Markdown vault actually has, not to recover anything itself.
//!
//! Layout under `root` (a directory distinct from the Markdown vault root,
//! never git-initialized):
//!
//! - `<root>/<note_id>.pii` — the live, committed ciphertext file.
//! - `<root>/.staging/<note_id>-<operation_id>.pii` — a durably-written,
//!   fsynced staging file, not yet visible as the live ciphertext. Mirrors
//!   `storage::vault`'s `.before-images` convention: a dotfile directory
//!   sibling of the live files, keyed by `(note_id, operation_id)`.
//!
//! On-disk format of both staging and live `.pii` files, one buffer, written
//! in one go (mirroring `storage::vault::serialize_record`'s "replace
//! together or not at all" discipline):
//!
//! ```text
//! [4 bytes: wrapped-data-key length, little-endian u32]
//! [wrapped-data-key bytes: AES-256-GCM(master_key, nonce=wrap_nonce) over the fresh data key]
//! [12 bytes: wrap_nonce]
//! [12 bytes: content_nonce]
//! [remaining bytes: AES-256-GCM(data_key, nonce=content_nonce) ciphertext of the plaintext, tag included]
//! ```
//!
//! A fresh random nonce is generated for both AEAD operations on every call
//! to [`PiiStore::stage`] — a nonce is never reused across records or across
//! versions of the same record (a staged file for an update always gets a
//! brand-new data key and brand-new nonces, never the predecessor's).

use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::availability::{EncryptedStoreAvailability, StoreUnavailableReason};
use crate::keys::{KeyError, MasterKey, StoreFingerprint, MASTER_KEY_LEN};

/// Dotfile directory, inside the PII store root, that holds staged (not yet
/// committed) ciphertext. Sibling of the live `.pii` files, exactly as
/// `storage::vault::BEFORE_IMAGE_DIR` is a sibling of `.record` files.
const STAGING_DIR: &str = ".staging";

/// AES-GCM nonce length in bytes.
const NONCE_LEN: usize = 12;

/// `(wrapped_data_key, wrap_nonce, content_nonce, ciphertext)`, as parsed
/// back out of the on-disk buffer format by [`PiiStore::deserialize`].
type DeserializedRecord = (Vec<u8>, [u8; NONCE_LEN], [u8; NONCE_LEN], Vec<u8>);

/// Errors from `PiiStore` operations.
///
/// No variant carries plaintext, key, or data-key bytes — this type derives
/// `Debug` freely, and its `Display`/`Debug` output is covered by the A3
/// confidentiality-boundary audit (`a3_confidentiality_boundary.rs`).
#[derive(Debug, Error)]
pub enum PiiStoreError {
    #[error("I/O error in PII store: {0}")]
    Io(#[from] std::io::Error),
    /// The ciphertext (or its wrapped data key) for `note_id` does not
    /// verify: an AEAD tag mismatch, or the decrypted bytes do not hash to
    /// what the caller expected. Never a silent fallback to some other
    /// content — the caller must treat this exactly as
    /// `storage::StorageError::Inconsistent` treats an on-disk hash
    /// mismatch.
    #[error("PII record {note_id} is inconsistent: ciphertext or wrapped key does not verify")]
    Inconsistent { note_id: String },
    #[error("PII record {note_id} not found")]
    NotFound { note_id: String },
    #[error(transparent)]
    Key(#[from] KeyError),
}

/// The result of a successful [`PiiStore::stage`] call: the hash of the
/// PLAINTEXT that was staged (never the ciphertext), for `storage`'s journal
/// to record on the same advance that records `AfterPiiStagedBeforeCommit`
/// — mirroring how `content_hash` already works for the pointer record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedPii {
    pub content_hash: String,
}

/// A parsed `pii://<note_id>` pointer, as stored in the Markdown record body
/// in place of a `Sensitive` note's real content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiiPointer {
    pub uri: String,
}

const PII_SCHEME: &str = "pii://";

impl PiiPointer {
    /// Builds the pointer for `note_id`.
    pub fn for_note(note_id: &str) -> Self {
        Self { uri: format!("{PII_SCHEME}{note_id}") }
    }

    /// Recognizes a `pii://<note_id>` URI. Returns `None` for anything else
    /// (including an empty note id after the scheme, which is never a valid
    /// pointer).
    pub fn parse(s: &str) -> Option<PiiPointer> {
        let rest = s.strip_prefix(PII_SCHEME)?;
        if rest.is_empty() {
            return None;
        }
        Some(PiiPointer { uri: s.to_string() })
    }

    /// The `note_id` portion of this pointer's URI.
    pub fn note_id(&self) -> &str {
        self.uri.strip_prefix(PII_SCHEME).unwrap_or(&self.uri)
    }
}

/// Maps a [`PiiStoreError`] (as returned by [`PiiStore::open`]) to the
/// [`StoreUnavailableReason`] the gate uses to describe why a `Sensitive`
/// note cannot be accepted right now. Documented mapping:
///
/// - [`PiiStoreError::Key`]:
///   - [`KeyError::RecoveryKitRequired`] or a `KeyError::Dpapi*` variant
///     (the key file exists but cannot be unwrapped in this
///     user/machine context, or DPAPI itself cannot be reached): the store
///     IS configured but its key cannot be reached right now ->
///     [`StoreUnavailableReason::KeyUnreachable`].
///   - [`KeyError::ExistingStoreNotDecryptable`] (ciphertext exists on disk
///     but no usable key file exists for it — fingerprint/key mismatch):
///     the on-disk state is inconsistent with what can be decrypted ->
///     [`StoreUnavailableReason::StoreInconsistent`].
///   - [`KeyError::PlatformUnsupported`] or [`KeyError::Io`]: no store has
///     ever been successfully set up in this environment ->
///     [`StoreUnavailableReason::NotYetConfigured`].
/// - [`PiiStoreError::Io`]: the store directory itself could not even be
///   created/read -> [`StoreUnavailableReason::NotYetConfigured`] (nothing
///   has been configured here yet, from this call's point of view).
/// - [`PiiStoreError::Inconsistent`] / [`PiiStoreError::NotFound`]: these are
///   per-record outcomes from `stage`/`get`, never returned by `open`
///   itself; included here only so the match is exhaustive. Mapped to
///   [`StoreUnavailableReason::StoreInconsistent`] as the closest fit should
///   a future caller ever reach this function with one.
pub fn unavailable_reason_for(err: &PiiStoreError) -> StoreUnavailableReason {
    match err {
        PiiStoreError::Key(KeyError::DpapiUnavailable)
        | PiiStoreError::Key(KeyError::DpapiDecryptFailed)
        | PiiStoreError::Key(KeyError::RecoveryKitRequired) => StoreUnavailableReason::KeyUnreachable,
        PiiStoreError::Key(KeyError::ExistingStoreNotDecryptable) => StoreUnavailableReason::StoreInconsistent,
        PiiStoreError::Key(KeyError::PlatformUnsupported) | PiiStoreError::Key(KeyError::Io(_)) => {
            StoreUnavailableReason::NotYetConfigured
        }
        PiiStoreError::Io(_) => StoreUnavailableReason::NotYetConfigured,
        PiiStoreError::Inconsistent { .. } | PiiStoreError::NotFound { .. } => {
            StoreUnavailableReason::StoreInconsistent
        }
    }
}

/// Hex-encoded SHA-256 of `bytes`. Mirrors
/// `storage::vault::content_hash` exactly (same algorithm, same hex
/// encoding), kept as an independent copy here since `vault-security` has
/// no dependency on `storage`.
fn content_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn fresh_nonce() -> [u8; NONCE_LEN] {
    let mut bytes = [0u8; NONCE_LEN];
    getrandom::fill(&mut bytes).expect("the OS CSPRNG must be available to generate an AEAD nonce");
    bytes
}

fn aead_encrypt(key_bytes: &[u8; MASTER_KEY_LEN], nonce: &[u8; NONCE_LEN], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key_bytes));
    cipher
        .encrypt(&Nonce::from(*nonce), plaintext)
        .expect("AES-256-GCM encryption of a bounded in-memory buffer cannot fail")
}

fn aead_decrypt(
    key_bytes: &[u8; MASTER_KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
) -> Result<Vec<u8>, ()> {
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key_bytes));
    cipher.decrypt(&Nonce::from(*nonce), ciphertext).map_err(|_| ())
}

/// Writes `contents` to `path` atomically: sibling temp file in the same
/// directory, fsynced, then renamed over `path`. A minimal, crate-local copy
/// of `storage::atomic::atomic_write`'s primitive (same family — create,
/// write, fsync, rename — but `storage`'s own function is `pub(crate)` there
/// and unreachable from this crate).
fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp_name = format!(
        "{}.tmp-{}-{}",
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
    );
    let tmp_path = dir.join(tmp_name);
    {
        let mut tmp_file = File::create(&tmp_path)?;
        tmp_file.write_all(contents)?;
        tmp_file.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)?;
    #[cfg(unix)]
    {
        if let Ok(dir_file) = File::open(dir) {
            let _ = dir_file.sync_all();
        }
    }
    Ok(())
}

/// Encrypted PII store, rooted at a directory distinct from the Markdown
/// vault root. See the module docs for the on-disk layout and format.
pub struct PiiStore {
    root: PathBuf,
    master_key: MasterKey,
}

impl PiiStore {
    /// Opens (or initializes) the PII store at `root`, loading or creating
    /// its master key at `key_store_path`.
    ///
    /// The `StoreFingerprint` `MasterKey::load_or_create` needs is computed
    /// here by checking whether `root` already contains any live `.pii`
    /// file (NOT staging files — a staged-but-uncommitted file proves
    /// nothing about whether a store with a real key has ever existed, and
    /// counting it could make a crash during a FIRST write look like "an
    /// existing store" to the next open, which would then refuse to mint a
    /// key at all).
    ///
    /// This is how `storage` answers `EncryptedStoreAvailability` for a
    /// given call: `Available` iff this call to `open` succeeds for this
    /// call's configured `(root, key_store_path)`; a caller that wants
    /// `Unavailable { reason }` maps the returned error via
    /// [`unavailable_reason_for`].
    pub fn open(root: PathBuf, key_store_path: &Path) -> Result<Self, PiiStoreError> {
        std::fs::create_dir_all(&root)?;
        let fingerprint = Self::fingerprint(&root)?;
        let master_key = MasterKey::load_or_create(key_store_path, Some(&fingerprint))?;
        Ok(Self { root, master_key })
    }

    /// Whether `root` already contains at least one live (committed) `.pii`
    /// file. Staging files are deliberately excluded — see [`PiiStore::open`].
    fn fingerprint(root: &Path) -> Result<StoreFingerprint, PiiStoreError> {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(StoreFingerprint::empty()),
            Err(e) => return Err(PiiStoreError::Io(e)),
        };
        for entry in entries {
            let entry = entry?;
            if entry.path().extension().and_then(|e| e.to_str()) == Some("pii") {
                return Ok(StoreFingerprint::existing());
            }
        }
        Ok(StoreFingerprint::empty())
    }

    /// This store's root directory, so a caller (`storage`, Task 6) can
    /// derive sibling paths — for example the deletion log's own SQLite
    /// file and trusted-reference file — without needing a second,
    /// independently-configured root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<root>/<note_id>.pii`.
    fn live_path(&self, note_id: &str) -> PathBuf {
        self.root.join(format!("{note_id}.pii"))
    }

    /// `<root>/.staging/<note_id>-<operation_id>.pii`.
    fn staging_path(&self, note_id: &str, operation_id: Uuid) -> PathBuf {
        self.root.join(STAGING_DIR).join(format!("{note_id}-{operation_id}.pii"))
    }

    /// Serializes `wrapped_key`, `wrap_nonce`, `content_nonce` and
    /// `ciphertext` into the single on-disk buffer format (module docs).
    fn serialize(wrapped_key: &[u8], wrap_nonce: &[u8; NONCE_LEN], content_nonce: &[u8; NONCE_LEN], ciphertext: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + wrapped_key.len() + NONCE_LEN + NONCE_LEN + ciphertext.len());
        buf.extend_from_slice(&(wrapped_key.len() as u32).to_le_bytes());
        buf.extend_from_slice(wrapped_key);
        buf.extend_from_slice(wrap_nonce);
        buf.extend_from_slice(content_nonce);
        buf.extend_from_slice(ciphertext);
        buf
    }

    /// Inverse of [`PiiStore::serialize`].
    fn deserialize(bytes: &[u8]) -> Option<DeserializedRecord> {
        if bytes.len() < 4 {
            return None;
        }
        let key_len = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let mut offset = 4;
        if bytes.len() < offset + key_len + NONCE_LEN + NONCE_LEN {
            return None;
        }
        let wrapped_key = bytes[offset..offset + key_len].to_vec();
        offset += key_len;
        let wrap_nonce: [u8; NONCE_LEN] = bytes[offset..offset + NONCE_LEN].try_into().ok()?;
        offset += NONCE_LEN;
        let content_nonce: [u8; NONCE_LEN] = bytes[offset..offset + NONCE_LEN].try_into().ok()?;
        offset += NONCE_LEN;
        let ciphertext = bytes[offset..].to_vec();
        Some((wrapped_key, wrap_nonce, content_nonce, ciphertext))
    }

    /// AEAD-encrypts `plaintext` under a FRESH per-record data key (itself
    /// wrapped under `self.master_key`, with its own fresh nonce), and
    /// writes the result to the durable, fsynced staging file at
    /// `<root>/.staging/<note_id>-<operation_id>.pii` — NOT yet the live
    /// `<note_id>.pii`.
    ///
    /// Returns [`StagedPii`] carrying the hash of the PLAINTEXT (never the
    /// ciphertext), for `storage`'s journal to record on the same advance
    /// that records reaching `AfterPiiStagedBeforeCommit`.
    pub fn stage(&self, note_id: &str, plaintext: &[u8], operation_id: Uuid) -> Result<StagedPii, PiiStoreError> {
        let mut data_key = [0u8; MASTER_KEY_LEN];
        getrandom::fill(&mut data_key).expect("the OS CSPRNG must be available to generate a data key");

        let wrap_nonce = fresh_nonce();
        let wrapped_key = aead_encrypt(self.master_key.expose_secret(), &wrap_nonce, &data_key);

        let content_nonce = fresh_nonce();
        let ciphertext = aead_encrypt(&data_key, &content_nonce, plaintext);

        let buffer = Self::serialize(&wrapped_key, &wrap_nonce, &content_nonce, &ciphertext);

        let staging_dir = self.root.join(STAGING_DIR);
        std::fs::create_dir_all(&staging_dir)?;
        let path = self.staging_path(note_id, operation_id);
        atomic_write(&path, &buffer)?;

        Ok(StagedPii { content_hash: content_hash(plaintext) })
    }

    /// Atomically renames the staged file for `(note_id, operation_id)` onto
    /// the live `<root>/<note_id>.pii` path, then removes the staging entry
    /// (idempotent: a missing staging file at that point is not an error,
    /// since the rename already consumed it).
    pub fn commit_staged(&self, note_id: &str, operation_id: Uuid) -> Result<(), PiiStoreError> {
        let staged = self.staging_path(note_id, operation_id);
        let live = self.live_path(note_id);
        std::fs::rename(&staged, &live)?;
        Ok(())
    }

    /// Unwraps the data key, decrypts, and returns the plaintext for
    /// `note_id`'s live ciphertext. An AEAD tag mismatch on either layer is
    /// [`PiiStoreError::Inconsistent`] — never a silent fallback to some
    /// other content.
    pub fn get(&self, note_id: &str) -> Result<Vec<u8>, PiiStoreError> {
        let path = self.live_path(note_id);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(PiiStoreError::NotFound { note_id: note_id.to_string() })
            }
            Err(e) => return Err(PiiStoreError::Io(e)),
        };
        self.decrypt_buffer(note_id, &bytes)
    }

    /// Reads back and decrypts a STAGED (not yet committed) record for
    /// `(note_id, operation_id)`, exactly as `get` does for the live file.
    /// Used by `storage`'s crash-recovery path to verify a staged
    /// ciphertext's hash before rolling the pipeline forward.
    pub fn get_staged(&self, note_id: &str, operation_id: Uuid) -> Result<Vec<u8>, PiiStoreError> {
        let path = self.staging_path(note_id, operation_id);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(PiiStoreError::NotFound { note_id: note_id.to_string() })
            }
            Err(e) => return Err(PiiStoreError::Io(e)),
        };
        self.decrypt_buffer(note_id, &bytes)
    }

    fn decrypt_buffer(&self, note_id: &str, bytes: &[u8]) -> Result<Vec<u8>, PiiStoreError> {
        let Some((wrapped_key, wrap_nonce, content_nonce, ciphertext)) = Self::deserialize(bytes) else {
            return Err(PiiStoreError::Inconsistent { note_id: note_id.to_string() });
        };
        let data_key_bytes = aead_decrypt(self.master_key.expose_secret(), &wrap_nonce, &wrapped_key)
            .map_err(|_| PiiStoreError::Inconsistent { note_id: note_id.to_string() })?;
        let data_key: [u8; MASTER_KEY_LEN] = data_key_bytes
            .as_slice()
            .try_into()
            .map_err(|_| PiiStoreError::Inconsistent { note_id: note_id.to_string() })?;
        let plaintext = aead_decrypt(&data_key, &content_nonce, &ciphertext)
            .map_err(|_| PiiStoreError::Inconsistent { note_id: note_id.to_string() })?;
        Ok(plaintext)
    }

    /// Removes the staged file for `(note_id, operation_id)` if one exists.
    /// Idempotent. Used by `storage`'s crash-recovery rollback path to
    /// discard an orphaned stage when rolling back a failed sensitive write.
    pub fn discard_staged(&self, note_id: &str, operation_id: Uuid) -> Result<(), PiiStoreError> {
        let path = self.staging_path(note_id, operation_id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PiiStoreError::Io(e)),
        }
    }

    /// Removes the live `.pii` file for `note_id` if one exists. Idempotent.
    /// Used by `storage`'s crash-recovery rollback path to undo a committed
    /// ciphertext when the pointer rename never happened (the sensitive
    /// analogue of restoring a before-image).
    pub fn remove_live(&self, note_id: &str) -> Result<(), PiiStoreError> {
        let path = self.live_path(note_id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PiiStoreError::Io(e)),
        }
    }

    /// Whether a live `.pii` file exists for `note_id`.
    pub fn has_live(&self, note_id: &str) -> bool {
        self.live_path(note_id).is_file()
    }

    /// Opens a `PiiStore` and reports the result as
    /// [`EncryptedStoreAvailability`], mapping any [`PiiStoreError`]/
    /// [`KeyError`] into a [`StoreUnavailableReason`] per
    /// [`unavailable_reason_for`]'s mapping. This is the one function
    /// `storage` calls to answer the gate's `store` parameter — it is a thin
    /// wrapper over `open` so `storage` never has to duplicate the mapping
    /// logic itself.
    pub fn open_for_availability(
        root: PathBuf,
        key_store_path: &Path,
    ) -> (Option<Self>, EncryptedStoreAvailability) {
        match Self::open(root, key_store_path) {
            Ok(store) => (Some(store), EncryptedStoreAvailability::Available),
            Err(e) => {
                let reason = unavailable_reason_for(&e);
                (None, EncryptedStoreAvailability::Unavailable { reason })
            }
        }
    }

    /// Enumerates live `.pii` files (never staging ones — a staged-but-not-
    /// yet-committed file is mid-write, not an orphan) with no corresponding
    /// entry in `known_pointers`. Returns the bare note ids (the `.pii`
    /// file's stem), not full paths.
    pub fn find_orphans(&self, known_pointers: &HashSet<String>) -> Vec<String> {
        let mut orphans = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return orphans;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("pii") {
                continue;
            }
            // `.staging` is a directory, not a `.pii`-suffixed file, so it is
            // already excluded by the extension check above; this is an
            // extra, explicit guard in case that ever changes.
            if path.file_name().and_then(|n| n.to_str()) == Some(STAGING_DIR) {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !known_pointers.contains(stem) {
                orphans.push(stem.to_string());
            }
        }
        orphans
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn open_store(dir: &Path) -> PiiStore {
        PiiStore::open(dir.join("pii-store"), &dir.join("master.key")).unwrap()
    }

    #[test]
    fn stage_then_commit_then_get_round_trips_plaintext() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op = Uuid::new_v4();
        let staged = store.stage("note-a", b"super secret plaintext", op).unwrap();
        assert_eq!(staged.content_hash, content_hash(b"super secret plaintext"));

        assert!(!store.has_live("note-a"), "not yet committed");
        store.commit_staged("note-a", op).unwrap();
        assert!(store.has_live("note-a"));

        let plaintext = store.get("note-a").unwrap();
        assert_eq!(plaintext, b"super secret plaintext");
    }

    #[test]
    fn staged_file_never_contains_the_plaintext_as_a_substring() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op = Uuid::new_v4();
        store.stage("note-a", b"UNIQUE_SECRET_MARKER_XYZ", op).unwrap();
        let path = store.staging_path("note-a", op);
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.windows(b"UNIQUE_SECRET_MARKER_XYZ".len()).any(|w| w == b"UNIQUE_SECRET_MARKER_XYZ"));
    }

    #[test]
    fn get_on_tampered_ciphertext_is_inconsistent_never_silently_wrong() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op = Uuid::new_v4();
        store.stage("note-a", b"plaintext", op).unwrap();
        store.commit_staged("note-a", op).unwrap();

        let path = store.live_path("note-a");
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();

        let err = store.get("note-a").unwrap_err();
        assert!(matches!(err, PiiStoreError::Inconsistent { note_id } if note_id == "note-a"));
    }

    #[test]
    fn get_on_missing_note_is_not_found() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let err = store.get("missing").unwrap_err();
        assert!(matches!(err, PiiStoreError::NotFound { note_id } if note_id == "missing"));
    }

    #[test]
    fn find_orphans_excludes_known_pointers_and_staged_only_files() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());

        let committed_op = Uuid::new_v4();
        store.stage("note-known", b"a", committed_op).unwrap();
        store.commit_staged("note-known", committed_op).unwrap();

        let orphan_op = Uuid::new_v4();
        store.stage("note-orphan", b"b", orphan_op).unwrap();
        store.commit_staged("note-orphan", orphan_op).unwrap();

        // Still-staged, never committed: must NOT be reported as an orphan.
        let staged_only_op = Uuid::new_v4();
        store.stage("note-staged-only", b"c", staged_only_op).unwrap();

        let mut known = HashSet::new();
        known.insert("note-known".to_string());

        let orphans = store.find_orphans(&known);
        assert_eq!(orphans, vec!["note-orphan".to_string()]);
    }

    #[test]
    fn two_stages_of_the_same_note_use_independent_data_keys_and_nonces() {
        // Guards against nonce/key reuse across versions of the same note.
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op1 = Uuid::new_v4();
        store.stage("note-a", b"v1", op1).unwrap();
        let bytes1 = std::fs::read(store.staging_path("note-a", op1)).unwrap();

        let op2 = Uuid::new_v4();
        store.stage("note-a", b"v2", op2).unwrap();
        let bytes2 = std::fs::read(store.staging_path("note-a", op2)).unwrap();

        // Different operation ids -> different staging files, and their
        // serialized buffers must differ (different key, different nonces).
        assert_ne!(bytes1, bytes2);
    }

    #[test]
    fn pii_pointer_parses_and_round_trips_note_id() {
        let pointer = PiiPointer::for_note("note-a");
        assert_eq!(pointer.uri, "pii://note-a");
        let parsed = PiiPointer::parse("pii://note-a").unwrap();
        assert_eq!(parsed.note_id(), "note-a");
        assert!(PiiPointer::parse("not-a-pointer").is_none());
        assert!(PiiPointer::parse("pii://").is_none());
    }

    #[test]
    fn open_refuses_to_mint_a_fresh_key_over_an_existing_store_with_a_lost_key_file() {
        let dir = tempdir().unwrap();
        let store_root = dir.path().join("pii-store");
        let key_path = dir.path().join("master.key");
        let store = PiiStore::open(store_root.clone(), &key_path).unwrap();
        let op = Uuid::new_v4();
        store.stage("note-a", b"secret", op).unwrap();
        store.commit_staged("note-a", op).unwrap();
        drop(store);

        // Simulate the key file being lost/corrupted beyond use while
        // ciphertext remains on disk.
        std::fs::remove_file(&key_path).unwrap();

        let reopened = PiiStore::open(store_root, &key_path);
        let described = reopened.as_ref().map(|_| "Ok(PiiStore)").map_err(|e| format!("{e}"));
        assert!(
            matches!(reopened, Err(PiiStoreError::Key(KeyError::ExistingStoreNotDecryptable))),
            "must refuse to silently orphan the existing store, got {described:?}"
        );
    }
}
