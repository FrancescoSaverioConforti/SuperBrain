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
//! - `<root>/.before-images/<note_id>-<operation_id>.pii` — the PRIOR live
//!   ciphertext, preserved for the duration of ONE operation, exactly
//!   mirroring `storage::vault`'s own `.before-images` mechanism for
//!   Markdown content (`persist_before_image`/`read_before_image`/
//!   `delete_before_image`). `PiiStore`'s live path is keyed by `note_id`
//!   ALONE (not by version), so [`PiiStore::commit_staged`] on an UPDATE
//!   overwrites the only copy of the prior, already-committed ciphertext —
//!   without this before-image, an operation that commits its new
//!   ciphertext and then crashes (or fails) before its pointer rename lands
//!   would permanently destroy the one copy of the content the PRIOR,
//!   already-committed pointer still legitimately describes. The caller
//!   (`storage`) is responsible for calling [`PiiStore::commit_staged`]
//!   (which persists this before-image internally, before overwriting the
//!   live file) and, on rollback, [`PiiStore::restore_before_image`]; on
//!   success past the point of no return, [`PiiStore::delete_before_image`].
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

/// Dotfile directory, inside the PII store root, that holds PII
/// before-images: the prior live ciphertext an update is about to overwrite,
/// preserved for the duration of one operation. Mirrors
/// `storage::vault::BEFORE_IMAGE_DIR` exactly, as its own sibling directory
/// (never the same directory as `STAGING_DIR`, so a crash can never confuse
/// "not yet committed" with "the content a commit is about to overwrite").
const BEFORE_IMAGE_DIR: &str = ".before-images";

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
    /// [`PiiStore::get_unless_deleted`]: the deletion log records `note_id`
    /// as deleted (in any state, `Pending` included, per C5's "non
    /// consultabile dal passo 1"), so its content is not returned, whatever
    /// ciphertext may still be on disk.
    #[error("PII record {note_id} has been deleted and is not consultable")]
    Deleted { note_id: String },
    /// [`PiiStore::get_unless_deleted`] could not consult the deletion log.
    /// Fails closed: "could not check" is never read as "not deleted".
    #[error("cannot consult the deletion log for {note_id}: {reason}")]
    DeletionLogUnavailable { note_id: String, reason: String },
}

/// File name, inside a [`PiiStore`]'s root, of the deletion log's SQLite
/// database. See [`PiiStore::deletion_log_path`].
const DELETION_LOG_FILE: &str = "deletion-log.sqlite";

/// File name, inside a [`PiiStore`]'s root, of the deletion log's
/// DPAPI-authenticated trusted reference. See
/// [`PiiStore::deletion_log_reference_path`].
const DELETION_LOG_REFERENCE_FILE: &str = "deletion-log-reference.dpapi";

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
        PiiStoreError::Inconsistent { .. }
        | PiiStoreError::NotFound { .. }
        | PiiStoreError::Deleted { .. }
        | PiiStoreError::DeletionLogUnavailable { .. } => StoreUnavailableReason::StoreInconsistent,
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

    /// `<root>/deletion-log.sqlite`: the single, documented location of the
    /// deletion log that governs this store's notes. `storage`'s write
    /// pipeline, its startup reconciliation, [`crate::backup::backup`] and
    /// [`PiiStore::get_unless_deleted`] all derive it from here, so there is
    /// one source of truth for where it lives.
    pub fn deletion_log_path(&self) -> PathBuf {
        self.root.join(DELETION_LOG_FILE)
    }

    /// `<root>/deletion-log-reference.dpapi`: the deletion log's
    /// DPAPI-authenticated trusted reference, a sibling of
    /// [`PiiStore::deletion_log_path`].
    pub fn deletion_log_reference_path(&self) -> PathBuf {
        self.root.join(DELETION_LOG_REFERENCE_FILE)
    }

    /// `<root>/<note_id>.pii`.
    fn live_path(&self, note_id: &str) -> PathBuf {
        self.root.join(format!("{note_id}.pii"))
    }

    /// `<root>/.staging/<note_id>-<operation_id>.pii`.
    fn staging_path(&self, note_id: &str, operation_id: Uuid) -> PathBuf {
        self.root.join(STAGING_DIR).join(format!("{note_id}-{operation_id}.pii"))
    }

    /// `<root>/.before-images/<note_id>-<operation_id>.pii`.
    fn before_image_path(&self, note_id: &str, operation_id: Uuid) -> PathBuf {
        self.root.join(BEFORE_IMAGE_DIR).join(format!("{note_id}-{operation_id}.pii"))
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
    ///
    /// Before overwriting, if a live file already exists (an UPDATE, not a
    /// first write), its bytes are durably copied to this operation's own
    /// before-image (`<root>/.before-images/<note_id>-<operation_id>.pii`,
    /// fsynced) FIRST — mirroring `storage::vault`'s "persist the
    /// before-image before the rename" discipline exactly. `PiiStore`'s live
    /// path is keyed by `note_id` alone, not by version, so without this,
    /// overwriting it would destroy the only copy of the PRIOR,
    /// already-committed operation's ciphertext — the content a crash or a
    /// later rollback (`restore_before_image`) might still need to recover.
    /// The caller is responsible for calling [`PiiStore::delete_before_image`]
    /// once this operation has reached a point of no return (mirroring
    /// `storage::vault`'s "only delete the before-image once `GitCommitted`
    /// is reached"), and [`PiiStore::restore_before_image`] on rollback.
    pub fn commit_staged(&self, note_id: &str, operation_id: Uuid) -> Result<(), PiiStoreError> {
        let staged = self.staging_path(note_id, operation_id);
        let live = self.live_path(note_id);

        // A3 review round 2, item #3: a missing live file (NotFound) is the
        // only case that legitimately means "nothing to preserve" (a first
        // write). Any OTHER I/O error (permission denied, a sharing
        // violation, ...) must fail closed and propagate, never be treated
        // as if the file were absent — silently proceeding to overwrite the
        // live path in that case could destroy a live file this call could
        // not even read, with no before-image ever taken of it.
        let prior_bytes = match std::fs::read(&live) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(PiiStoreError::Io(e)),
        };
        // A3 review round 3: never overwrite an existing before-image for
        // this operation. If one is already there, the live file may by now
        // be this operation's OWN ciphertext, and re-copying it would
        // replace the only copy of the prior content with a copy of the new
        // one. No legitimate caller commits the same operation twice, so
        // this is refused rather than guessed at.
        if self.before_image_exists(note_id, operation_id)? {
            return Err(PiiStoreError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "a PII before-image already exists for this operation; refusing to overwrite it",
            )));
        }
        if let Some(prior_bytes) = prior_bytes {
            let before_image_dir = self.root.join(BEFORE_IMAGE_DIR);
            std::fs::create_dir_all(&before_image_dir)?;
            let before_image_path = self.before_image_path(note_id, operation_id);
            atomic_write(&before_image_path, &prior_bytes)?;
        }

        std::fs::rename(&staged, &live)?;
        Ok(())
    }

    /// Restores the live `<note_id>.pii` file from this operation's own
    /// before-image (as persisted by [`PiiStore::commit_staged`]), verifying
    /// the restored bytes decrypt and re-hash to `expected_plaintext_hash`
    /// before accepting them. Used by `storage`'s crash-recovery and
    /// live-write rollback paths to undo a committed ciphertext when the
    /// pointer that was meant to accompany it never landed — the sensitive
    /// analogue of `storage::vault::read_before_image` plus the write-back
    /// `atomic_write` call that restores a Markdown before-image.
    ///
    /// Returns `Ok(true)` if a before-image existed and was restored and
    /// verified; `Ok(false)` if no before-image exists for this
    /// `(note_id, operation_id)` (a first write has nothing to restore —
    /// the caller should fall back to [`PiiStore::remove_live`] instead).
    /// A before-image that exists but fails to verify is
    /// [`PiiStoreError::Inconsistent`] — never silently discarded, and the
    /// live file is left untouched in that case.
    pub fn restore_before_image(
        &self,
        note_id: &str,
        operation_id: Uuid,
        expected_plaintext_hash: &str,
    ) -> Result<bool, PiiStoreError> {
        let before_image_path = self.before_image_path(note_id, operation_id);
        let before_bytes = match std::fs::read(&before_image_path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(PiiStoreError::Io(e)),
        };
        let plaintext = self.decrypt_buffer(note_id, &before_bytes)?;
        if content_hash(&plaintext) != expected_plaintext_hash {
            return Err(PiiStoreError::Inconsistent { note_id: note_id.to_string() });
        }
        let live = self.live_path(note_id);
        atomic_write(&live, &before_bytes)?;
        // Re-read and re-verify the bytes actually on disk, exactly as
        // `storage::vault`'s own rollback path re-reads after writing the
        // Markdown before-image back.
        let written = std::fs::read(&live)?;
        let written_plaintext = self.decrypt_buffer(note_id, &written)?;
        if content_hash(&written_plaintext) != expected_plaintext_hash {
            return Err(PiiStoreError::Inconsistent { note_id: note_id.to_string() });
        }
        Ok(true)
    }

    /// Removes this operation's before-image, if one exists. Idempotent.
    /// Mirrors `storage::vault::delete_before_image`. Must be called only
    /// once this operation has reached a point of no return (per
    /// `commit_staged`'s doc comment) — never while the operation's
    /// pointer-rename outcome is still unresolved.
    pub fn delete_before_image(&self, note_id: &str, operation_id: Uuid) -> Result<(), PiiStoreError> {
        let path = self.before_image_path(note_id, operation_id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PiiStoreError::Io(e)),
        }
    }

    /// Whether a before-image exists for `(note_id, operation_id)`, as a
    /// plain `bool` for tests and diagnostics ONLY: any error checking for
    /// it reads as `false`. Code that decides whether to delete, restore or
    /// overwrite anything must use [`PiiStore::before_image_exists`] or
    /// [`PiiStore::get_before_image`], which report errors instead of
    /// collapsing them into "absent" (A3 review round 3).
    pub fn has_before_image(&self, note_id: &str, operation_id: Uuid) -> bool {
        matches!(self.before_image_exists(note_id, operation_id), Ok(true))
    }

    /// Whether a before-image exists for `(note_id, operation_id)`. Only a
    /// `NotFound` means "absent"; any other I/O error, or a path that exists
    /// but is not a regular file, is an error, never `Ok(false)`.
    pub fn before_image_exists(&self, note_id: &str, operation_id: Uuid) -> Result<bool, PiiStoreError> {
        match std::fs::metadata(self.before_image_path(note_id, operation_id)) {
            Ok(meta) if meta.is_file() => Ok(true),
            Ok(_) => Err(PiiStoreError::Io(std::io::Error::other("PII before-image path exists but is not a regular file"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(PiiStoreError::Io(e)),
        }
    }

    /// Reads back and decrypts this operation's before-image, exactly as
    /// [`PiiStore::get_staged`] does for a staged file:
    /// [`PiiStoreError::NotFound`] if there is none,
    /// [`PiiStoreError::Inconsistent`] if it does not decrypt,
    /// [`PiiStoreError::Io`] on any other read failure. Used by `storage`'s
    /// recovery to POSITIVELY verify that a before-image is redundant (its
    /// content is identical to what is live) before deleting it.
    pub fn get_before_image(&self, note_id: &str, operation_id: Uuid) -> Result<Vec<u8>, PiiStoreError> {
        let bytes = match std::fs::read(self.before_image_path(note_id, operation_id)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(PiiStoreError::NotFound { note_id: note_id.to_string() })
            }
            Err(e) => return Err(PiiStoreError::Io(e)),
        };
        self.decrypt_buffer(note_id, &bytes)
    }

    /// The consumer-facing read of a sensitive note's content: [`PiiStore::get`],
    /// but only after the deletion log at [`PiiStore::deletion_log_path`]
    /// confirms `note_id` is NOT deleted (final whole-branch review, I2).
    /// A note with a deletion entry in any state is
    /// [`PiiStoreError::Deleted`], whatever ciphertext is still on disk: if
    /// `record_deletion` succeeded but a later step of the tombstone write
    /// failed or crashed, the old ciphertext can still be live, and per C5
    /// it must be non-consultable from the moment the deletion is logged.
    /// A deletion log that cannot be opened or queried is
    /// [`PiiStoreError::DeletionLogUnavailable`], never "not deleted".
    pub fn get_unless_deleted(&self, note_id: &str) -> Result<Vec<u8>, PiiStoreError> {
        let unavailable = |e: crate::deletion_log::DeletionLogError| PiiStoreError::DeletionLogUnavailable {
            note_id: note_id.to_string(),
            reason: e.to_string(),
        };
        let log = crate::deletion_log::DeletionLog::open(&self.deletion_log_path()).map_err(unavailable)?;
        if log.is_deleted(note_id).map_err(unavailable)? {
            return Err(PiiStoreError::Deleted { note_id: note_id.to_string() });
        }
        self.get(note_id)
    }

    /// Unwraps the data key, decrypts, and returns the plaintext for
    /// `note_id`'s live ciphertext. An AEAD tag mismatch on either layer is
    /// [`PiiStoreError::Inconsistent`] — never a silent fallback to some
    /// other content.
    ///
    /// Low-level primitive: it does NOT consult the deletion log. It is what
    /// crash recovery, rollback verification and the restore round-trip use,
    /// where the question is "what bytes are on disk", not "may a reader see
    /// this note". Code that serves a note's content to a reader must use
    /// [`PiiStore::get_unless_deleted`].
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
    /// UNVERIFIED: it removes whatever is there. Rollback code must use
    /// [`PiiStore::remove_live_verified`] instead, which first proves the
    /// live file is the content the caller means to remove.
    pub fn remove_live(&self, note_id: &str) -> Result<(), PiiStoreError> {
        let path = self.live_path(note_id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PiiStoreError::Io(e)),
        }
    }

    /// Removes the live `.pii` file for `note_id` ONLY after re-reading it
    /// and verifying that it decrypts to plaintext hashing to
    /// `expected_plaintext_hash` (A3 review round 3: the verification sits
    /// next to the destructive act instead of being left to every caller).
    /// `Ok(false)` if there is no live file; [`PiiStoreError::Inconsistent`]
    /// if the live file is someone else's content or does not decrypt;
    /// [`PiiStoreError::Io`] on any read failure. In every error case the
    /// live file is left untouched.
    pub fn remove_live_verified(&self, note_id: &str, expected_plaintext_hash: &str) -> Result<bool, PiiStoreError> {
        let plaintext = match self.get(note_id) {
            Ok(plaintext) => plaintext,
            Err(PiiStoreError::NotFound { .. }) => return Ok(false),
            Err(e) => return Err(e),
        };
        if content_hash(&plaintext) != expected_plaintext_hash {
            return Err(PiiStoreError::Inconsistent { note_id: note_id.to_string() });
        }
        self.remove_live(note_id)?;
        Ok(true)
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
    fn fresh_nonce_never_repeats_across_many_draws() {
        // AES-256-GCM nonce reuse under the same key is catastrophic (it
        // breaks both confidentiality and authentication). This asserts
        // fresh_nonce() genuinely draws fresh randomness each call rather
        // than e.g. returning a fixed/cached value: a mutant that replaced
        // the body with `[0; NONCE_LEN]` would pass every other test in
        // this suite (each of which only encrypts once per key) but fails
        // here. A collision among 50 draws from a 12-byte CSPRNG is
        // astronomically unlikely (~2^-96 per pair), so this is not flaky.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            let nonce = fresh_nonce();
            assert!(seen.insert(nonce), "fresh_nonce() produced a duplicate nonce: {nonce:?}");
        }
    }

    /// Final whole-branch review, I2 (read side): `record_deletion` runs
    /// BEFORE the tombstone ciphertext replaces the live one, so if a later
    /// step of the tombstone write fails or crashes, the REAL ciphertext is
    /// still live. It must already be non-consultable through the
    /// consumer-facing read, while the low-level `get` still sees the bytes.
    #[test]
    fn get_unless_deleted_refuses_a_note_whose_deletion_is_logged_even_while_its_real_ciphertext_is_still_live() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op = Uuid::new_v4();
        store.stage("note-a", b"REAL CONTENT", op).unwrap();
        store.commit_staged("note-a", op).unwrap();
        store.stage("note-b", b"untouched", op).unwrap();
        store.commit_staged("note-b", op).unwrap();
        assert_eq!(store.get_unless_deleted("note-a").unwrap(), b"REAL CONTENT", "not deleted yet: served normally");

        let log = crate::deletion_log::DeletionLog::open(&store.deletion_log_path()).unwrap();
        let reference = crate::deletion_log::FileTrustedReferenceStore::new(dir.path().join("trusted-reference.bin"));
        log.record_deletion("note-a", &reference).unwrap();

        assert!(
            matches!(store.get_unless_deleted("note-a"), Err(PiiStoreError::Deleted { note_id }) if note_id == "note-a"),
            "a logged deletion makes the note non-consultable immediately"
        );
        assert_eq!(store.get("note-a").unwrap(), b"REAL CONTENT", "precondition: the real ciphertext is still on disk");
        assert_eq!(store.get_unless_deleted("note-b").unwrap(), b"untouched", "other notes are unaffected");
    }

    /// The deletion-log locations are a contract shared by `storage`'s write
    /// pipeline and startup reconcile, `backup()`, and existing vaults on
    /// disk: they must stay exactly these sibling files of the root (the
    /// mutation re-run showed `deletion_log_reference_path -> Default`
    /// surviving, since every caller agreed with the mutated value).
    #[test]
    fn deletion_log_paths_are_the_documented_siblings_of_the_root() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        assert_eq!(store.deletion_log_path(), dir.path().join("pii-store").join("deletion-log.sqlite"));
        assert_eq!(store.deletion_log_reference_path(), dir.path().join("pii-store").join("deletion-log-reference.dpapi"));
    }

    /// I2: a deletion log that cannot be opened fails closed.
    #[test]
    fn get_unless_deleted_fails_closed_when_the_deletion_log_cannot_be_opened() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op = Uuid::new_v4();
        store.stage("note-a", b"REAL CONTENT", op).unwrap();
        store.commit_staged("note-a", op).unwrap();
        // A directory where the SQLite file should be: it cannot be opened.
        std::fs::create_dir_all(store.deletion_log_path()).unwrap();
        assert!(
            matches!(store.get_unless_deleted("note-a"), Err(PiiStoreError::DeletionLogUnavailable { .. })),
            "an unreadable deletion log must never read as 'not deleted'"
        );
    }

    /// Final whole-branch review, I4 #4: the fingerprint's `.pii` extension
    /// check is the guard against minting a fresh master key over an
    /// existing encrypted store (the plan's top Review Focus item). A root
    /// holding only non-`.pii` files (stray temp files, the deletion log,
    /// a staged file inside `.staging/`) must read as "no existing
    /// ciphertext"; adding one live `.pii` file must flip it. The `==`->`!=`
    /// mutant survived before this test because no test fingerprinted a root
    /// whose ONLY contents were non-`.pii` files.
    #[test]
    fn fingerprint_ignores_non_pii_files_and_detects_a_real_pii_file() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join(STAGING_DIR)).unwrap();
        std::fs::write(root.join("note-a.pii.tmp-1-2"), b"stray temp file").unwrap();
        std::fs::write(root.join("deletion-log.sqlite"), b"not ciphertext").unwrap();
        std::fs::write(root.join("readme.txt"), b"unrelated").unwrap();
        std::fs::write(root.join(STAGING_DIR).join("note-a-00000000-0000-0000-0000-000000000000.pii"), b"staged").unwrap();

        assert!(
            !PiiStore::fingerprint(&root).unwrap().indicates_existing_ciphertext(),
            "only non-.pii files at the root: no existing ciphertext"
        );

        std::fs::write(root.join("note-a.pii"), b"live ciphertext").unwrap();
        assert!(
            PiiStore::fingerprint(&root).unwrap().indicates_existing_ciphertext(),
            "one live .pii file: existing ciphertext"
        );
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
    fn commit_staged_on_an_update_preserves_the_prior_live_ciphertext_as_a_before_image() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op1 = Uuid::new_v4();
        store.stage("note-a", b"v1 plaintext", op1).unwrap();
        store.commit_staged("note-a", op1).unwrap();
        assert!(!store.has_before_image("note-a", op1), "a first write has no prior live file to preserve");

        let op2 = Uuid::new_v4();
        store.stage("note-a", b"v2 plaintext", op2).unwrap();
        store.commit_staged("note-a", op2).unwrap();
        assert!(store.has_before_image("note-a", op2), "an update must preserve the prior live ciphertext as op2's before-image");

        // The live file is now v2's.
        assert_eq!(store.get("note-a").unwrap(), b"v2 plaintext");
        // op2's before-image, independently decrypted, is v1's content.
        let before_hash = content_hash(b"v1 plaintext");
        let restored = store.restore_before_image("note-a", op2, &before_hash).unwrap();
        assert!(restored);
        assert_eq!(store.get("note-a").unwrap(), b"v1 plaintext", "restoring op2's before-image rolls the live file back to v1");
    }

    #[test]
    fn commit_staged_fails_closed_on_a_non_notfound_io_error_reading_the_live_file_instead_of_silently_overwriting() {
        // A3 review round 2, item #3: a live path that is a DIRECTORY must
        // not produce a before-image or a successful commit. NOTE (round
        // 3): this is NOT a regression test for the swallowed-read-error
        // fix. With the old swallowing code the read error is ignored, but
        // the following `rename(staged, <directory>)` fails too, so the
        // assertions below pass with or without the fix (verified by
        // reverting it). The genuine regression tests are the
        // `commit_staged_fails_closed_when_the_live_file_*` tests below,
        // where the live file is unreadable but still renamable-over.
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let note_id = "note-live-is-a-directory";
        // Make the live .pii path a directory before any write ever
        // touches it, so there is no legitimate prior ciphertext at all —
        // only an unreadable obstruction.
        std::fs::create_dir_all(store.live_path(note_id)).unwrap();

        let op = Uuid::new_v4();
        store.stage(note_id, b"some plaintext", op).unwrap();
        let result = store.commit_staged(note_id, op);
        assert!(
            matches!(result, Err(PiiStoreError::Io(_))),
            "a non-NotFound I/O error reading the live path must propagate as PiiStoreError::Io, not be silently swallowed, got {result:?}"
        );
        // No before-image must have been fabricated from a read that never
        // actually succeeded.
        assert!(!store.has_before_image(note_id, op), "no before-image must exist when the live-file read never succeeded");
    }

    /// Shared body of the item-3 regression tests (A3 review round 3). The
    /// directory-as-live-path test above is a FALSE GREEN: with the old,
    /// swallowing `if let Ok(..) = std::fs::read(..)`, the subsequent
    /// `rename(staged, <directory>)` ALSO fails, so `commit_staged` still
    /// returns `Err(Io)` and still leaves no before-image — the assertions
    /// pass whether or not the fix is present. A genuine regression test
    /// needs a live file that exists, holds real committed ciphertext,
    /// CANNOT be read, and CAN still be renamed over — so the old code
    /// visibly destroys it (returns `Ok`, live is now v2, no before-image),
    /// while the fixed code refuses before touching anything.
    fn assert_commit_staged_never_overwrites_a_live_file_it_could_not_read(
        obstruct: impl FnOnce(&Path) -> Box<dyn FnOnce()>,
    ) {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op1 = Uuid::new_v4();
        store.stage("note-a", b"v1 committed", op1).unwrap();
        store.commit_staged("note-a", op1).unwrap();

        let op2 = Uuid::new_v4();
        store.stage("note-a", b"v2 attempt", op2).unwrap();
        let release = obstruct(&store.live_path("note-a"));
        let result = store.commit_staged("note-a", op2);
        release();

        assert!(
            matches!(result, Err(PiiStoreError::Io(_))),
            "an unreadable live file must make commit_staged fail closed with Io, got {result:?}"
        );
        assert_eq!(
            store.get("note-a").unwrap(),
            b"v1 committed",
            "the live file must be untouched: overwriting it without a before-image would destroy the only copy of v1"
        );
        assert!(!store.has_before_image("note-a", op2), "no before-image fabricated from a read that never succeeded");
        assert!(store.get_staged("note-a", op2).is_ok(), "the staged file is left in place for the caller's rollback");
    }

    #[cfg(windows)]
    #[test]
    fn commit_staged_fails_closed_when_the_live_file_is_held_without_read_sharing() {
        // FILE_SHARE_DELETE only: every other READ open fails with a
        // sharing violation, but a replacing rename (which needs only
        // DELETE access on the target) still succeeds — exactly the
        // combination under which the old code silently overwrote v1.
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_DELETE: u32 = 0x0000_0004;
        assert_commit_staged_never_overwrites_a_live_file_it_could_not_read(|live| {
            let handle = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_DELETE).open(live).unwrap();
            assert!(std::fs::read(live).is_err(), "precondition: the live file must be unreadable while held");
            Box::new(move || drop(handle))
        });
    }

    #[cfg(unix)]
    #[test]
    fn commit_staged_fails_closed_when_the_live_file_is_not_readable() {
        use std::os::unix::fs::PermissionsExt;
        let probe_dir = tempdir().unwrap();
        let probe = probe_dir.path().join("probe");
        std::fs::write(&probe, b"x").unwrap();
        std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&probe).is_ok() {
            // Running as root: permission bits do not block reads, so this
            // environment cannot produce the error this test needs.
            return;
        }
        assert_commit_staged_never_overwrites_a_live_file_it_could_not_read(|live| {
            let live = live.to_path_buf();
            std::fs::set_permissions(&live, std::fs::Permissions::from_mode(0o000)).unwrap();
            Box::new(move || std::fs::set_permissions(&live, std::fs::Permissions::from_mode(0o600)).unwrap())
        });
    }

    #[test]
    fn commit_staged_never_overwrites_an_existing_before_image_for_the_same_operation() {
        // A3 review round 3 self-audit: committing the same operation twice
        // would re-copy the live file (by then this operation's OWN
        // ciphertext) over its before-image, replacing the only copy of the
        // prior content.
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op1 = Uuid::new_v4();
        store.stage("note-a", b"v1", op1).unwrap();
        store.commit_staged("note-a", op1).unwrap();
        let op2 = Uuid::new_v4();
        store.stage("note-a", b"v2", op2).unwrap();
        store.commit_staged("note-a", op2).unwrap();
        store.stage("note-a", b"v2 again", op2).unwrap();
        assert!(store.commit_staged("note-a", op2).is_err());
        assert_eq!(store.get_before_image("note-a", op2).unwrap(), b"v1", "op2's before-image must still hold v1");
        assert_eq!(store.get("note-a").unwrap(), b"v2", "and the live file is untouched");
    }

    #[test]
    fn before_image_exists_reports_an_unreadable_path_as_an_error_not_as_absent() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op = Uuid::new_v4();
        assert!(!store.before_image_exists("note-a", op).unwrap());
        std::fs::create_dir_all(store.before_image_path("note-a", op)).unwrap();
        assert!(store.before_image_exists("note-a", op).is_err());
    }

    #[test]
    fn remove_live_verified_never_removes_content_that_does_not_match() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op = Uuid::new_v4();
        store.stage("note-a", b"v1", op).unwrap();
        store.commit_staged("note-a", op).unwrap();
        assert!(matches!(store.remove_live_verified("note-a", &content_hash(b"other")), Err(PiiStoreError::Inconsistent { .. })));
        assert_eq!(store.get("note-a").unwrap(), b"v1");
        assert!(store.remove_live_verified("note-a", &content_hash(b"v1")).unwrap());
        assert!(!store.has_live("note-a"));
        assert!(!store.remove_live_verified("note-a", &content_hash(b"v1")).unwrap(), "absent is Ok(false)");
    }

    #[test]
    fn restore_before_image_on_a_first_write_with_no_before_image_returns_false() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op = Uuid::new_v4();
        store.stage("note-a", b"v1", op).unwrap();
        store.commit_staged("note-a", op).unwrap();
        let restored = store.restore_before_image("note-a", op, &content_hash(b"irrelevant")).unwrap();
        assert!(!restored, "a first write has nothing to restore");
    }

    #[test]
    fn restore_before_image_with_a_hash_mismatch_is_inconsistent_and_leaves_the_live_file_untouched() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op1 = Uuid::new_v4();
        store.stage("note-a", b"v1 plaintext", op1).unwrap();
        store.commit_staged("note-a", op1).unwrap();
        let op2 = Uuid::new_v4();
        store.stage("note-a", b"v2 plaintext", op2).unwrap();
        store.commit_staged("note-a", op2).unwrap();

        let wrong_hash = content_hash(b"not the actual before-image content");
        let err = store.restore_before_image("note-a", op2, &wrong_hash).unwrap_err();
        assert!(matches!(err, PiiStoreError::Inconsistent { note_id } if note_id == "note-a"));
        // The live file (v2) must be untouched by the failed restore attempt.
        assert_eq!(store.get("note-a").unwrap(), b"v2 plaintext");
    }

    #[test]
    fn delete_before_image_is_idempotent_and_removes_the_file() {
        let dir = tempdir().unwrap();
        let store = open_store(dir.path());
        let op1 = Uuid::new_v4();
        store.stage("note-a", b"v1", op1).unwrap();
        store.commit_staged("note-a", op1).unwrap();
        let op2 = Uuid::new_v4();
        store.stage("note-a", b"v2", op2).unwrap();
        store.commit_staged("note-a", op2).unwrap();
        assert!(store.has_before_image("note-a", op2));
        store.delete_before_image("note-a", op2).unwrap();
        assert!(!store.has_before_image("note-a", op2));
        store.delete_before_image("note-a", op2).unwrap(); // idempotent
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
