//! Encrypted backup/restore of the Markdown vault + `PiiStore`, gated on the
//! LIVE deletion log, per spec contract C5/C7 ("Un dato cancellato non
//! riappare" must hold through a backup/restore cycle, not just through
//! ordinary reads).
//!
//! # Why two checks, not one
//!
//! A naive restore might check only the backup's OWN embedded
//! [`GenerationRef`] snapshot (taken at backup time) against... itself,
//! which is trivially always satisfied, or against the live reference,
//! which catches a STALE backup but not a note deleted strictly AFTER the
//! backup was taken but BEFORE the restore runs. [`restore`] therefore runs
//! two independent checks before writing anything to `destination_root`:
//!
//! 1. [`DeletionLog::verify_safe_to_expose_after_restore`] — the backup's
//!    own embedded reference snapshot against the LIVE trusted reference.
//!    Internally runs `DeletionLog::reconcile` FIRST, so this check only
//!    ever reports staleness (or anything else) about a live log/reference
//!    pair that has already been proven mutually consistent. Post-N1
//!    follow-up: a backup that is merely OLDER than the live state —
//!    [`DeletionLogError::BackupReferenceStale`] — no longer aborts the
//!    whole restore; every other failure here (the live log itself
//!    disagreeing with its own reference, a broken chain, a missing
//!    reference, or any other inconsistency) still does.
//! 2. A per-note cross-check: every note id the backup would restore is
//!    checked against `live_deletion_log.is_deleted(note_id)`. This catches
//!    what check 1 cannot see: which specific notes the live log reports
//!    deleted. Every such note is EXCLUDED from the restore (neither its
//!    record nor its ciphertext is written) and named in
//!    [`RestoreOutcome::excluded_deleted_note_ids`], rather than aborting
//!    the whole restore (final whole-branch review, CRITICAL C1: a deletion
//!    is a tombstone write, so every backup taken after a deletion names
//!    the deleted note, and the old "reject if any note is deleted" rule
//!    made such a vault permanently unrestorable). This cross-check runs
//!    TWICE: once before check 1 (a best-effort hint used only to choose
//!    which error to report if check 1 fails for a reason other than mere
//!    backup staleness), and once again AFTER check 1 has resolved — the
//!    second run is the ONLY one that ever decides what gets excluded, so a
//!    deletion landing in the window between the two runs (a real race an
//!    earlier draft of this fix missed) is still caught, never resurrected.
//!    If check 1 fails for a reason other than `BackupReferenceStale` while
//!    the first-pass hint names a deleted note, the refusal is reported as
//!    [`BackupError::DeletedSinceBackup`] rather than the more generic
//!    [`BackupError::DeletionLogRejected`] — a label choice only, made
//!    before anything is written either way.
//!
//! Both checks run before ANY byte is written to `destination_root`.
//! On any ambiguity — an I/O error reading either reference, a corrupt or
//! tampered reference, a missing live reference, or the live log itself
//! disagreeing with its own reference — this fails closed
//! ([`BackupError::DeletionLogRejected`]) and extracts nothing, per the same
//! discipline Task 6 arrived at only after several rounds: a destructive or
//! exposing action is gated on a POSITIVE, VERIFIED "safe," never on a
//! default taken when something is merely absent or unreadable. Only the
//! narrower, syntactically distinct "this specific backup's own snapshot is
//! older than an already-trustworthy live state" case is treated
//! differently, and only because the second, post-check-1 note-level
//! cross-check is what actually decides exposure in that case, not check 1
//! itself.
//!
//! # Key separation
//!
//! The snapshot is encrypted under an HKDF-SHA256 subkey derived from the
//! live `MasterKey`, NOT the master key directly:
//!
//! ```text
//! backup_subkey = HKDF-SHA256-Expand(
//!     HKDF-SHA256-Extract(salt = None, ikm = master_key_bytes),
//!     info = b"superbrain-backup-v1",
//!     length = 32,
//! )
//! ```
//!
//! This keeps backup ciphertext cryptographically independent from
//! `PiiStore`'s live per-record data keys (which are themselves wrapped
//! directly under the master key): a compromise of one derived key's use
//! context does not help an attacker with the other, and the fixed `info`
//! string domain-separates this one purpose from any future HKDF use of
//! the same master key.
//!
//! # Blob format
//!
//! The on-disk blob `BackupTarget::write_blob` receives is:
//!
//! ```text
//! [12 bytes: AEAD nonce]
//! [remaining bytes: AES-256-GCM(backup_subkey, nonce) ciphertext, tag
//!  included, of the serialized plaintext snapshot]
//! ```
//!
//! The plaintext snapshot (what the hash in [`BackupManifest`] is computed
//! over, and what AEAD decrypts back to) is a simple length-prefixed
//! container, chosen for the same reason `pii_store.rs` chose its own
//! buffer format: no untrusted parser needs defending against, only
//! unambiguous round-tripping:
//!
//! ```text
//! [8 bytes: generation, little-endian u64]
//! [4 bytes: chain_hash length, little-endian u32][chain_hash bytes (hex ascii)]
//! [4 bytes: vault file count, little-endian u32]
//! for each vault file, in a stable (sorted-by-name) order:
//!   [4 bytes: name length][name bytes (relative path, forward-slash separated)]
//!   [8 bytes: content length, little-endian u64][content bytes]
//! [4 bytes: pii file count, little-endian u32]
//! for each live `.pii` file, in a stable (sorted-by-name) order:
//!   [4 bytes: note_id length][note_id bytes]
//!   [8 bytes: content length, little-endian u64][content bytes (the
//!    already-AEAD-wrapped `.pii` buffer, re-encrypted again as part of
//!    this outer backup envelope — the live per-record encryption is left
//!    intact, this is defense in depth, not a substitute for it)]
//! ```

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::deletion_log::{DeletionLog, DeletionLogError, GenerationRef, TrustedReferenceStore};
use crate::keys::{MasterKey, MASTER_KEY_LEN};
use crate::pii_store::PiiStore;

/// AES-GCM nonce length in bytes.
const NONCE_LEN: usize = 12;

/// Fixed HKDF `info` string domain-separating the backup subkey from any
/// other derived use of the master key. See module docs.
const HKDF_INFO: &[u8] = b"superbrain-backup-v1";

/// Name of the single blob a backup writes/reads through [`BackupTarget`].
/// A backup target holds exactly one logical backup at a time (Task 7's
/// "second backup cycle" test replaces it in place, under this same name,
/// rather than accumulating historical blobs — the physical retention
/// policy for multiple generations of backups is out of scope here, per
/// the brief).
const BLOB_NAME: &str = "superbrain-backup.enc";

/// Errors from backup/restore operations.
#[derive(Debug, Error)]
pub enum BackupError {
    #[error("I/O error in backup/restore: {0}")]
    Io(String),

    /// [`restore`] was asked to read from a target that has never had
    /// anything written to it ([`BackupTarget::is_empty`] reported `true`,
    /// or the blob is simply absent).
    #[error("backup target is empty: nothing to restore")]
    TargetEmpty,

    /// The decrypted snapshot's recomputed hash does not match what the
    /// manifest expected it to be, or the snapshot's own serialized form is
    /// malformed. Never a silent partial restore.
    #[error("backup content hash verification failed")]
    HashMismatch,

    /// Either of [`restore`]'s two deletion-log checks failed for a reason
    /// the deletion log itself names (corrupt/tampered/missing reference,
    /// stale generation, etc.) — distinct from [`BackupError::DeletedSinceBackup`],
    /// which is this module's OWN check (not `vault-security`'s deletion-log
    /// crate code), naming the specific notes deleted after the backup.
    #[error("deletion log rejected this restore: {0}")]
    DeletionLogRejected(DeletionLogError),

    /// Check 1 (see module docs) refused this backup for a reason OTHER
    /// than mere staleness against an already-trustworthy live state (the
    /// live reference cannot be verified, the live log itself disagrees
    /// with its own reference, the chain is broken, etc. —
    /// [`DeletionLogError::BackupReferenceStale`] on its own never produces
    /// this error; see module docs) — AND at least one note the backup
    /// would have restored is deleted in the LIVE deletion log, typically
    /// because it was deleted after this backup was taken. Named
    /// explicitly and distinctly from [`BackupError::DeletionLogRejected`]
    /// so a caller can tell "the live state itself is unusable" apart from
    /// "specific notes must not be exposed." When check 1 passes (either
    /// outright, or via the `BackupReferenceStale` fall-through), deleted
    /// notes are instead excluded and reported in
    /// [`RestoreOutcome::excluded_deleted_note_ids`] on a successful
    /// restore of everything else.
    #[error("note(s) deleted since this backup was taken must not be exposed by restore: {note_ids:?}")]
    DeletedSinceBackup { note_ids: Vec<String> },

    /// [`restore`] was asked to write into a non-empty `destination_root`.
    /// Fails closed rather than silently merging or overwriting.
    #[error("destination already contains files; refusing to restore into a non-empty directory")]
    AlreadyExists,

    /// [`restore`] would have to overwrite at least one `.pii` file that
    /// already exists at `pii_root` (typically the LIVE `PiiStore`'s own
    /// root). Fails closed rather than silently clobbering live,
    /// potentially newer ciphertext with whatever the backup happened to
    /// hold — mirrors [`BackupError::AlreadyExists`]'s discipline on the
    /// Markdown vault side, applied to the PII side. Named distinctly so a
    /// caller can tell "the Markdown destination already has files" apart
    /// from "the PII root already has at least one of these specific
    /// files."
    #[error("refusing to restore: {note_ids:?} already have a live .pii file at the destination; restoring would silently overwrite live PII ciphertext")]
    PiiAlreadyExists { note_ids: Vec<String> },
}

impl From<io::Error> for BackupError {
    fn from(e: io::Error) -> Self {
        BackupError::Io(e.to_string())
    }
}

/// Where a backup is written to and read from. Implementors need not be
/// durable in any particular way beyond "what was written can be read
/// back" — [`FilesystemBackupTarget`] is the only implementation this crate
/// ships; the physical backup medium (external disk, cloud object store,
/// ...) stays an explicitly open decision per the spec's own closing
/// section, out of scope here.
pub trait BackupTarget {
    fn write_blob(&self, name: &str, bytes: &[u8]) -> Result<(), BackupError>;
    fn read_blob(&self, name: &str) -> Result<Vec<u8>, BackupError>;
    fn list_blobs(&self) -> Result<Vec<String>, BackupError>;
    fn is_empty(&self) -> Result<bool, BackupError>;
}

/// A [`BackupTarget`] backed by a plain directory on disk. Each blob is one
/// file, named `dir/<name>`, written via a sibling-temp-file-then-rename
/// (the same atomic-write discipline used throughout this crate) so a
/// crash mid-write can never leave a half-written blob visible under its
/// final name.
pub struct FilesystemBackupTarget {
    dir: PathBuf,
}

impl FilesystemBackupTarget {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn blob_path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl BackupTarget for FilesystemBackupTarget {
    fn write_blob(&self, name: &str, bytes: &[u8]) -> Result<(), BackupError> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.blob_path(name);
        let tmp_name = format!(
            "{name}.tmp-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
        );
        let tmp_path = self.dir.join(tmp_name);
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp_path)?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp_path, &path)?;
        Ok(())
    }

    fn read_blob(&self, name: &str) -> Result<Vec<u8>, BackupError> {
        match std::fs::read(self.blob_path(name)) {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(BackupError::TargetEmpty),
            Err(e) => Err(BackupError::Io(e.to_string())),
        }
    }

    fn list_blobs(&self) -> Result<Vec<String>, BackupError> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(BackupError::Io(e.to_string())),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry?;
            if !entry.path().is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else { continue };
            if name.contains(".tmp-") {
                continue;
            }
            names.push(name);
        }
        Ok(names)
    }

    fn is_empty(&self) -> Result<bool, BackupError> {
        Ok(self.list_blobs()?.is_empty())
    }
}

/// What [`backup`] reports on success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupManifest {
    /// Hex-encoded SHA-256 of the PLAINTEXT snapshot, computed before
    /// encryption, so it can be verified again after decryption at restore
    /// time without needing to re-derive anything secret to check it.
    pub hash: String,
    /// The deletion log's generation at the moment this backup's
    /// `GenerationRef` snapshot was taken.
    pub generation: u64,
    pub created_at: SystemTime,
}

/// What [`restore`] reports on success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    /// Recomputed hash of the restored plaintext snapshot, for the caller to
    /// compare against a previously recorded [`BackupManifest::hash`].
    pub hash: String,
    /// The generation embedded in the restored backup's own `GenerationRef`
    /// snapshot.
    pub generation: u64,
    /// Note ids (vault notes, i.e. Markdown record files minus their
    /// extension) that were actually written to `destination_root`.
    pub restored_note_ids: Vec<String>,
    /// Note ids present in the backup (as a Markdown record, a `.pii` file,
    /// or both) that the LIVE deletion log reports deleted, and that were
    /// therefore NOT written anywhere: neither their record nor their
    /// ciphertext. Reported explicitly so a deleted note never just
    /// silently vanishes from the caller's point of view.
    pub excluded_deleted_note_ids: Vec<String>,
}

/// Derives the HKDF-SHA256 backup subkey from `master_key`. See module docs
/// for the exact construction.
fn derive_backup_subkey(master_key: &MasterKey) -> [u8; MASTER_KEY_LEN] {
    let hk = Hkdf::<Sha256>::new(None, master_key.expose_secret());
    let mut subkey = [0u8; MASTER_KEY_LEN];
    hk.expand(HKDF_INFO, &mut subkey).expect("32-byte output is within HKDF-SHA256's valid expand length");
    subkey
}

fn fresh_nonce() -> [u8; NONCE_LEN] {
    let mut bytes = [0u8; NONCE_LEN];
    getrandom::fill(&mut bytes).expect("the OS CSPRNG must be available to generate an AEAD nonce");
    bytes
}

fn aead_encrypt(key_bytes: &[u8; MASTER_KEY_LEN], nonce: &[u8; NONCE_LEN], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key_bytes));
    cipher.encrypt(&Nonce::from(*nonce), plaintext).expect("AES-256-GCM encryption of a bounded in-memory buffer cannot fail")
}

fn aead_decrypt(key_bytes: &[u8; MASTER_KEY_LEN], nonce: &[u8; NONCE_LEN], ciphertext: &[u8]) -> Result<Vec<u8>, ()> {
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key_bytes));
    cipher.decrypt(&Nonce::from(*nonce), ciphertext).map_err(|_| ())
}

/// Hex-encoded SHA-256 of `bytes`. Independent copy of the same helper in
/// `pii_store.rs` (same algorithm, same encoding).
fn content_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn write_len_prefixed_u32(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(bytes);
}

fn write_len_prefixed_u64(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    buf.extend_from_slice(bytes);
}

fn read_u32(bytes: &[u8], offset: &mut usize) -> Option<u32> {
    let v = u32::from_le_bytes(bytes.get(*offset..*offset + 4)?.try_into().ok()?);
    *offset += 4;
    Some(v)
}

fn read_u64(bytes: &[u8], offset: &mut usize) -> Option<u64> {
    let v = u64::from_le_bytes(bytes.get(*offset..*offset + 8)?.try_into().ok()?);
    *offset += 8;
    Some(v)
}

fn read_bytes<'a>(bytes: &'a [u8], offset: &mut usize, len: usize) -> Option<&'a [u8]> {
    let slice = bytes.get(*offset..*offset + len)?;
    *offset += len;
    Some(slice)
}

/// Writes `contents` to `path` atomically: a sibling temp file in the same
/// directory, fsynced, then renamed over `path`. A minimal, crate-local copy
/// of the same stage-then-rename primitive `pii_store.rs` uses for its own
/// `atomic_write` (same family — create, write, fsync, rename) and
/// `storage::atomic::atomic_write` uses elsewhere in this plan: `storage` is
/// not a dependency of `vault-security`, and `pii_store.rs`'s own copy is
/// module-private, so this is a third, deliberately small copy rather than a
/// new cross-crate dependency. A crash between the temp file's rename and
/// the next loop iteration can never leave a half-written `.pii` file
/// visible under its final name — the rename is the only operation that
/// touches the final path at all.
fn atomic_write_pii(path: &Path, contents: &[u8]) -> Result<(), BackupError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp_name = format!(
        "{}.tmp-{}-{}",
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
    );
    let tmp_path = dir.join(tmp_name);
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(contents)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Every regular file under `root`, recursively, excluding `.git` and
/// dotfile directories (`.staging`, `.before-images`, ...), returned as
/// `(relative_path_forward_slash, contents)` pairs in a stable, sorted
/// order (a `BTreeMap` is used purely to get that ordering for free).
fn collect_vault_files(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, BackupError> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(BackupError::Io(e.to_string())),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if file_name == ".git" || file_name.starts_with('.') {
                // Dotfile directories/files (`.git`, `.before-images`,
                // `.staging`, `.pii-store`'s own dotfiles if nested here)
                // are never part of the plain Markdown vault snapshot. The
                // PII store's live records are collected separately, by
                // `collect_pii_files`, directly from `PiiStore`, never by
                // walking its root as a plain directory here.
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                let bytes = std::fs::read(&path)?;
                out.insert(relative, bytes);
            }
        }
    }
    Ok(out)
}

/// Every live `.pii` file in `pii_store`'s root, as `(note_id, raw ciphertext
/// bytes)` pairs in sorted order. Reads the already-AEAD-wrapped on-disk
/// buffer directly (never calls `PiiStore::get`, which would decrypt it) —
/// the backup envelope re-encrypts this already-ciphertext blob as a second,
/// independent layer; it never handles the plaintext PII content at all.
fn collect_pii_files(pii_store: &PiiStore) -> Result<BTreeMap<String, Vec<u8>>, BackupError> {
    let mut out = BTreeMap::new();
    let entries = match std::fs::read_dir(pii_store.root()) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(BackupError::Io(e.to_string())),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("pii") {
            continue;
        }
        let Some(note_id) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let bytes = std::fs::read(&path)?;
        out.insert(note_id.to_string(), bytes);
    }
    Ok(out)
}

/// Serializes the full snapshot (deletion-log reference + vault files + PII
/// files) into the plaintext buffer format documented at the top of this
/// module.
fn serialize_snapshot(reference: &GenerationRef, vault_files: &BTreeMap<String, Vec<u8>>, pii_files: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&reference.generation.to_le_bytes());
    write_len_prefixed_u32(&mut buf, reference.chain_hash.as_bytes());

    buf.extend_from_slice(&(vault_files.len() as u32).to_le_bytes());
    for (name, content) in vault_files {
        write_len_prefixed_u32(&mut buf, name.as_bytes());
        write_len_prefixed_u64(&mut buf, content);
    }

    buf.extend_from_slice(&(pii_files.len() as u32).to_le_bytes());
    for (note_id, content) in pii_files {
        write_len_prefixed_u32(&mut buf, note_id.as_bytes());
        write_len_prefixed_u64(&mut buf, content);
    }

    buf
}

/// A fully-parsed snapshot, as returned by [`deserialize_snapshot`].
struct Snapshot {
    reference: GenerationRef,
    vault_files: BTreeMap<String, Vec<u8>>,
    pii_files: BTreeMap<String, Vec<u8>>,
}

/// Inverse of [`serialize_snapshot`]. `None` on any structural malformation
/// — the caller maps that to [`BackupError::HashMismatch`] (a snapshot that
/// does not even parse can never have been the one the manifest hashed).
fn deserialize_snapshot(bytes: &[u8]) -> Option<Snapshot> {
    let mut offset = 0usize;
    let generation = u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?);
    offset += 8;
    let chain_hash_len = read_u32(bytes, &mut offset)? as usize;
    let chain_hash = std::str::from_utf8(read_bytes(bytes, &mut offset, chain_hash_len)?).ok()?.to_string();
    let reference = GenerationRef { generation, chain_hash };

    let vault_count = read_u32(bytes, &mut offset)?;
    let mut vault_files = BTreeMap::new();
    for _ in 0..vault_count {
        let name_len = read_u32(bytes, &mut offset)? as usize;
        let name = std::str::from_utf8(read_bytes(bytes, &mut offset, name_len)?).ok()?.to_string();
        let content_len = read_u64(bytes, &mut offset)? as usize;
        let content = read_bytes(bytes, &mut offset, content_len)?.to_vec();
        vault_files.insert(name, content);
    }

    let pii_count = read_u32(bytes, &mut offset)?;
    let mut pii_files = BTreeMap::new();
    for _ in 0..pii_count {
        let id_len = read_u32(bytes, &mut offset)? as usize;
        let note_id = std::str::from_utf8(read_bytes(bytes, &mut offset, id_len)?).ok()?.to_string();
        let content_len = read_u64(bytes, &mut offset)? as usize;
        let content = read_bytes(bytes, &mut offset, content_len)?.to_vec();
        pii_files.insert(note_id, content);
    }

    Some(Snapshot { reference, vault_files, pii_files })
}

/// The `GenerationRef` embedded in a backup snapshot when the deletion log
/// has never recorded a single deletion (no row has ever existed, and so
/// the trusted reference has never been written either). Mirrors
/// `DeletionLog::reconcile`'s own `(None, None) => AlreadyConsistent`
/// judgment: generation 0 with an all-zero chain hash the same width as a
/// real sha256 hex digest (same convention as
/// [`crate::deletion_log::GENESIS_PREV_CHAIN_HASH`]) is never a value a real
/// deletion can produce (the first real deletion is always generation 1),
/// so this sentinel can never collide with genuine backup content.
fn no_deletions_ever_reference() -> GenerationRef {
    GenerationRef { generation: 0, chain_hash: "0".repeat(64) }
}

/// Encrypts a snapshot of the Markdown vault at `vault_root`, `pii_store`'s
/// live ciphertext files, and a [`GenerationRef`] snapshot of
/// `deletion_log`'s current trusted reference, and writes the result via
/// `target.write_blob`. See module docs for the exact key derivation and
/// blob format.
///
/// The `GenerationRef` snapshot is read from the LIVE trusted reference at
/// the moment this call runs — the real, DPAPI-authenticated
/// [`crate::deletion_log::DpapiTrustedReferenceStore`], rooted at
/// `<pii_store.root()>/deletion-log-reference.dpapi`, the exact same fixed
/// sibling path `storage`'s sensitive-write pipeline uses — never
/// reconstructed from the log's own latest row independently of the
/// reference (the reference, not the raw log, is the authenticated value
/// `restore`'s checks compare against). If no deletion has ever been
/// recorded, the reference file does not exist yet; this is captured as
/// [`no_deletions_ever_reference`], not treated as an I/O error.
pub fn backup(
    vault_root: &Path,
    pii_store: &PiiStore,
    deletion_log: &DeletionLog,
    master_key: &MasterKey,
    target: &dyn BackupTarget,
) -> Result<BackupManifest, BackupError> {
    let trusted_ref = crate::deletion_log::DpapiTrustedReferenceStore::new(pii_store.deletion_log_reference_path());
    backup_with_reference(vault_root, pii_store, deletion_log, master_key, target, &trusted_ref)
}

/// [`backup`], but lets a caller/test supply the exact [`TrustedReferenceStore`]
/// to read the snapshot's `GenerationRef` from. Production code always uses
/// [`backup`], which always pairs `pii_store`'s fixed, documented
/// `deletion-log-reference.dpapi` sibling path with the real
/// `DpapiTrustedReferenceStore`; this function exists so Task 7's own tests
/// can use the same plain, unauthenticated `FileTrustedReferenceStore`
/// `deletion_log.rs`'s own tests use, for the same reason that module gives
/// (DPAPI is awkward to drive in a tight unit-test loop) — the SAME
/// instance (or another instance pointed at the same on-disk file) the
/// test's own `DeletionLog::record_deletion` calls go through.
pub(crate) fn backup_with_reference(
    vault_root: &Path,
    pii_store: &PiiStore,
    _deletion_log: &DeletionLog,
    master_key: &MasterKey,
    target: &dyn BackupTarget,
    trusted_ref: &dyn TrustedReferenceStore,
) -> Result<BackupManifest, BackupError> {
    // A missing reference means "no deletion has ever been recorded," not
    // an error to fail closed on — `DeletionLog::reconcile` makes the exact
    // same judgment for `(None, None)`. Any OTHER error reading the
    // reference (corrupt, tampered) DOES propagate: backup must not
    // silently embed a fabricated reference over a reference that exists
    // but cannot be trusted.
    let reference = trusted_ref.read().map_err(BackupError::DeletionLogRejected)?.unwrap_or_else(no_deletions_ever_reference);

    let vault_files = collect_vault_files(vault_root)?;
    let pii_files = collect_pii_files(pii_store)?;

    let plaintext_snapshot = serialize_snapshot(&reference, &vault_files, &pii_files);
    let hash = content_hash(&plaintext_snapshot);

    let subkey = derive_backup_subkey(master_key);
    let nonce = fresh_nonce();
    let ciphertext = aead_encrypt(&subkey, &nonce, &plaintext_snapshot);

    let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);

    target.write_blob(BLOB_NAME, &blob)?;

    Ok(BackupManifest { hash, generation: reference.generation, created_at: SystemTime::now() })
}

/// Reads the single blob from `target`, decrypts it under `master_key`'s
/// HKDF subkey, verifies its hash, then runs BOTH deletion-log checks
/// described in the module docs before writing anything to
/// `destination_root` or to `pii_store`'s live ciphertext location. Only if
/// both checks pass does extraction proceed.
///
/// `destination_root` must not already exist, or must exist and be empty —
/// restoring into a non-empty directory is refused
/// ([`BackupError::AlreadyExists`]) rather than silently merging or
/// overwriting whatever is already there.
///
/// Every live `.pii` file the backup held is written back to
/// `pii_store.root()`, under the exact same `<note_id>.pii` convention
/// [`PiiStore`]'s own live-file storage uses (see `pii_store.rs`'s
/// `live_path`) — never only the Markdown pointer records. A restore that
/// wrote the pointer but not the ciphertext behind it would leave a
/// sensitive note as a dangling `pii://` pointer with nothing to resolve
/// it, while still reporting success: exactly the durability failure this
/// phase exists to prevent.
pub fn restore(
    target: &dyn BackupTarget,
    master_key: &MasterKey,
    destination_root: &Path,
    live_deletion_log: &DeletionLog,
    live_trusted_ref: &dyn TrustedReferenceStore,
    pii_store: &PiiStore,
) -> Result<RestoreOutcome, BackupError> {
    restore_with_pii_root(target, master_key, destination_root, live_deletion_log, live_trusted_ref, pii_store.root())
}

/// [`restore`], but lets a caller/test supply the exact PII live-file root
/// directory directly, rather than requiring a full [`PiiStore`] (whose
/// `root()` this function's `pii_root` parameter mirrors exactly — see
/// [`PiiStore::root`]). Exists for the same reason [`backup_with_reference`]
/// exists alongside [`backup`]: production code always goes through
/// [`restore`], which always passes `pii_store.root()`; this lets tests
/// restore into an ISOLATED pii root distinct from the live store being
/// backed up, so a round-trip test can assert on the restored bytes without
/// clobbering the original live file.
pub(crate) fn restore_with_pii_root(
    target: &dyn BackupTarget,
    master_key: &MasterKey,
    destination_root: &Path,
    live_deletion_log: &DeletionLog,
    live_trusted_ref: &dyn TrustedReferenceStore,
    pii_root: &Path,
) -> Result<RestoreOutcome, BackupError> {
    if target.is_empty()? {
        return Err(BackupError::TargetEmpty);
    }

    // Refuse to write into a non-empty destination BEFORE touching the
    // blob at all: an empty destination is the only state this function
    // ever writes into.
    match std::fs::read_dir(destination_root) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(BackupError::AlreadyExists);
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(BackupError::Io(e.to_string())),
    }

    let blob = target.read_blob(BLOB_NAME)?;
    if blob.len() < NONCE_LEN {
        return Err(BackupError::HashMismatch);
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce_bytes.try_into().expect("exactly NONCE_LEN bytes by construction above");

    let subkey = derive_backup_subkey(master_key);
    let plaintext_snapshot = aead_decrypt(&subkey, &nonce, ciphertext).map_err(|_| BackupError::HashMismatch)?;

    let recomputed_hash = content_hash(&plaintext_snapshot);

    let snapshot = deserialize_snapshot(&plaintext_snapshot).ok_or(BackupError::HashMismatch)?;

    // Both checks (module docs) run before anything is written.
    //
    // Check 2 (the per-note cross-check against the LIVE log) no longer
    // rejects the whole restore on its own (final whole-branch review,
    // CRITICAL C1). It used to: any note id in the snapshot that the live
    // log reports deleted aborted everything. But a deletion is a tombstone
    // WRITE, so every backup taken AFTER a deletion still names the deleted
    // note (its tombstone record and tombstone ciphertext), and every such
    // backup was refused. A vault that had ever deleted one sensitive note
    // could never be restored again, not even its other notes.
    //
    // Now a deleted note is EXCLUDED: neither its record nor its ciphertext
    // is written, and its id is reported in
    // `RestoreOutcome::excluded_deleted_note_ids` ("un restore applica prima
    // le cancellazioni più recenti e solo dopo diventa consultabile", spec
    // Fase A item 5). Exclusion is unconditional for a deleted id: it does
    // NOT try to tell a backed-up tombstone apart from backed-up real
    // content. That distinction is not safely decidable here. The tombstone
    // record is the same `pii://<id>` pointer shape as a live one; only its
    // decrypted plaintext (empty) differs, and this function never decrypts
    // `.pii` contents. And `record_deletion` runs BEFORE the tombstone
    // ciphertext replaces the real one, so a backup taken in between has a
    // current reference but still holds the real content. Dropping every
    // deleted id is correct in both cases: a deleted note has nothing left
    // that a restore should expose.
    //
    // Check 1 (the backup's embedded reference vs the live reference,
    // below) still fails closed on a corrupt, tampered, or missing
    // reference, or on the LIVE log disagreeing with its OWN reference —
    // but, post-N1 follow-up, no longer on mere staleness against an
    // already-trustworthy live state (`BackupReferenceStale`). When check 1
    // fails for any OTHER reason AND this early, best-effort hint named a
    // deleted note, the more specific `DeletedSinceBackup { note_ids }` is
    // returned instead of the generic `DeletionLogRejected`; exclusion
    // itself only ever applies once check 1 has passed (outright, or via
    // the `BackupReferenceStale` fall-through), and is decided by a SECOND,
    // later recomputation of this same lookup — see that recomputation's
    // own comment for why.

    // Check 2 (part A): the candidate note ids in the snapshot — pure
    // computation, no I/O, safe to do before check 1. A note is named here
    // if EITHER its Markdown record (vault_files, keyed by relative path
    // ending in ".record") OR its PII ciphertext (pii_files, keyed by bare
    // note id) is present in the snapshot.
    let mut candidate_note_ids: Vec<String> = Vec::new();
    for name in snapshot.vault_files.keys() {
        if let Some(stem) = name.strip_suffix(".record") {
            candidate_note_ids.push(stem.to_string());
        }
    }
    for note_id in snapshot.pii_files.keys() {
        candidate_note_ids.push(note_id.clone());
    }
    candidate_note_ids.sort();
    candidate_note_ids.dedup();

    // Reviewer-found race (post-N1 follow-up to the post-N1 follow-up):
    // Check 2's actual `is_deleted` LOOKUPS must run AFTER check 1 has
    // resolved, never before. Before this fix they ran first; a deletion
    // landing in the gap between this lookup and check 1's own read of the
    // live reference was invisible to this check, and — once staleness
    // alone stopped unconditionally blocking the whole restore — nothing
    // else caught it either, so that note could be written out already
    // deleted. This is now split in two: `pre_check_1_deleted_hint`, a
    // best-effort snapshot used ONLY to pick which error variant to report
    // if check 1 fails for a reason OTHER than `BackupReferenceStale` (an
    // error-labeling convenience, not a safety decision); and
    // `deleted_since_backup` (below, after check 1), which is the ONLY list
    // ever used to decide what gets excluded from the write.
    let pre_check_1_deleted_hint: Vec<String> = candidate_note_ids
        .iter()
        .filter(|note_id| live_deletion_log.is_deleted(note_id).unwrap_or(false))
        .cloned()
        .collect();

    // Check 1: the backup's OWN embedded reference snapshot against the
    // LIVE trusted reference. Any error here (corrupt, tampered, missing
    // reference, or a genuinely stale generation) fails closed as
    // DeletionLogRejected — never proceeds as if "unreadable" meant "safe."
    // The one exception, mirroring `backup`'s own and
    // `DeletionLog::reconcile`'s `(None, None) => AlreadyConsistent`
    // judgment: if the backup's own snapshot says no deletion had EVER
    // happened by backup time ([`no_deletions_ever_reference`]) AND the
    // live reference genuinely does not exist either (not an error reading
    // it — a clean, positive "nothing has ever been written here") AND the
    // live log itself has never recorded a single row, there is nothing to
    // verify: no deletion the backup could possibly be stale about has ever
    // occurred on either side. This is the common case for any vault that
    // has never had a PII deletion, and without it `restore` would be
    // permanently impossible for such a vault.
    //
    // The log's own row history (not just the reference) must be checked
    // here: a reference can be ABSENT for two different reasons — "nothing
    // has ever been written" (safe) or "the reference was lost/corrupted
    // while the log already has completed deletion history" (exactly the
    // state `DeletionLog::reconcile`'s `(Some(row), None)` branch fails
    // closed on via `ReferenceMissing`, unless that row is generation 1 and
    // still Pending). A sentinel-embedding backup must not be allowed to
    // bypass that same fail-closed judgment merely because the reference
    // happens to be unreadable; only a genuinely empty log justifies the
    // exception.
    // Post-N1 follow-up (user requirement): a backup whose OWN embedded
    // reference is merely BEHIND an already-trustworthy live state —
    // `BackupReferenceStale`, which `verify_safe_to_expose_after_restore`
    // can only return AFTER its own internal `reconcile` call has already
    // proved the live log/reference pair agrees with itself — no longer
    // rejects the whole restore. Exclusion (already computed above, against
    // the SAME live, trustworthy log, independent of how stale the
    // backup's own snapshot is) is the real protection against
    // resurrection; staleness alone is not a reason to refuse notes that
    // were never touched by whatever happened after this backup was taken.
    // Spec Fase A item 5: "un restore applica prima le cancellazioni più
    // recenti e solo dopo diventa consultabile" — exclusion already applies
    // the most recent deletions; a stale backup reference no longer needs
    // to block the "diventa consultabile" that follows.
    //
    // Every OTHER check-1 outcome (`ChainBroken`, `ReferenceMissing`,
    // `ReferenceStale` — the LIVE log disagreeing with its OWN reference,
    // a genuinely different situation from `BackupReferenceStale`, see that
    // variant's doc comment — or any other `Inconsistent` disagreement)
    // still rejects the whole restore unconditionally: those mean the live
    // log itself could not be positively verified, so nothing it reports —
    // including which notes are deleted — can be trusted enough to act on.
    // C5's "se il registro è assente, corrotto o non verificabile, i dati
    // potenzialmente cancellati non vengono esposti" stays enforced exactly
    // as before; only the narrower, syntactically-distinct "valido ma
    // obsoleto" case — and only at the BACKUP's own snapshot level, never
    // at the live log's own level — is now handled by exclusion instead of
    // outright refusal.
    let check_1 = (|| -> Result<(), DeletionLogError> {
        let live_reference_for_check1 = live_trusted_ref.read()?;
        let live_log_is_empty = live_deletion_log.is_empty()?;
        let both_sides_have_never_seen_a_deletion =
            snapshot.reference == no_deletions_ever_reference() && live_reference_for_check1.is_none() && live_log_is_empty;
        if both_sides_have_never_seen_a_deletion {
            return Ok(());
        }
        live_deletion_log.verify_safe_to_expose_after_restore(&snapshot.reference, live_trusted_ref)
    })();
    if let Err(e) = check_1 {
        if matches!(e, DeletionLogError::BackupReferenceStale { .. }) {
            // The live log/reference pair is trustworthy (reconcile
            // already proved it); only this backup's own snapshot is
            // older. Fall through to exclusion.
        } else if !pre_check_1_deleted_hint.is_empty() {
            return Err(BackupError::DeletedSinceBackup { note_ids: pre_check_1_deleted_hint });
        } else {
            return Err(BackupError::DeletionLogRejected(e));
        }
    }

    // Check 2 (part B): NOW, after check 1 has resolved (reconcile has run
    // and the live log/reference pair is known trustworthy, or the
    // sentinel exception applied), compute the REAL exclusion list. This
    // closes the race above: a deletion that landed anywhere up to and
    // including check 1 itself is reflected here, because this is the
    // very last read of the live log before anything is written.
    let mut deleted_since_backup = Vec::new();
    for note_id in &candidate_note_ids {
        // Any error here (I/O failure reading the log) must fail closed:
        // never treat "couldn't check" as "not deleted."
        if live_deletion_log.is_deleted(note_id).map_err(BackupError::DeletionLogRejected)? {
            deleted_since_backup.push(note_id.clone());
        }
    }

    // Both checks passed: drop every deleted note from what gets written.
    let excluded: std::collections::BTreeSet<&str> = deleted_since_backup.iter().map(String::as_str).collect();
    let is_excluded_record = |name: &str| name.strip_suffix(".record").is_some_and(|stem| excluded.contains(stem));
    let vault_files_to_write: Vec<(&String, &Vec<u8>)> =
        snapshot.vault_files.iter().filter(|(name, _)| !is_excluded_record(name)).collect();
    let pii_files_to_write: Vec<(&String, &Vec<u8>)> =
        snapshot.pii_files.iter().filter(|(note_id, _)| !excluded.contains(note_id.as_str())).collect();

    // Guard (Task 7 round 2 review finding), checked BEFORE any byte of
    // EITHER the vault files or the `.pii` files is written: the earlier
    // "`destination_root` must be empty" check says nothing about
    // `pii_root`, which in production IS the LIVE `PiiStore`'s root — a
    // directory that can perfectly well already hold `.pii` files newer
    // than this backup (the reviewer's repro: back up a note, commit newer
    // content to the SAME live store, then restore into a brand-new empty
    // Markdown destination — `destination_root` being empty says nothing
    // about whether `pii_root` already has files). Mirror `AlreadyExists`'s
    // discipline on the vault side exactly: check the WHOLE set of target
    // `.pii` paths for existing files before writing any of them (of either
    // kind), and refuse the entire restore if even one already exists,
    // naming every offending note id, so a silent rollback of live,
    // possibly-newer PII content can never happen.
    std::fs::create_dir_all(pii_root)?;
    let mut already_live: Vec<String> = Vec::new();
    for (note_id, _) in &pii_files_to_write {
        let dest_path = pii_root.join(format!("{note_id}.pii"));
        match std::fs::metadata(&dest_path) {
            Ok(_) => already_live.push((*note_id).clone()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(BackupError::Io(e.to_string())),
        }
    }
    if !already_live.is_empty() {
        already_live.sort();
        return Err(BackupError::PiiAlreadyExists { note_ids: already_live });
    }

    // Both deletion-log checks AND the PII existence guard passed: NOW, and
    // only now, extract.
    std::fs::create_dir_all(destination_root)?;
    for (name, content) in &vault_files_to_write {
        let dest_path = destination_root.join(name);
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest_path, content)?;
    }

    // Write every live `.pii` ciphertext file back to its PiiStore live
    // location (`<pii_root>/<note_id>.pii`, the exact convention
    // `PiiStore::live_path` uses) — never only the Markdown pointer. See
    // this function's doc comment: a restore that wrote the pointer but not
    // the ciphertext behind it would leave a sensitive note as a dangling
    // `pii://` reference while still reporting success. Each write is
    // staged-then-renamed ([`atomic_write_pii`]) rather than a plain
    // `std::fs::write`, so a crash mid-loop can never leave a truncated
    // `.pii` file over previously-valid ciphertext: the rename is the only
    // operation that ever touches the final path.
    for (note_id, content) in &pii_files_to_write {
        let dest_path = pii_root.join(format!("{note_id}.pii"));
        atomic_write_pii(&dest_path, content)?;
    }

    let restored_note_ids: Vec<String> = vault_files_to_write
        .iter()
        .filter_map(|(name, _)| name.strip_suffix(".record").map(|s| s.to_string()))
        .collect();

    Ok(RestoreOutcome {
        hash: recomputed_hash,
        generation: snapshot.reference.generation,
        restored_note_ids,
        excluded_deleted_note_ids: deleted_since_backup,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deletion_log::{FaultInjector, FaultPoint, FileTrustedReferenceStore};
    use crate::keys::MasterKey;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn write_file(path: &Path, content: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn fresh_master_key(dir: &Path) -> MasterKey {
        MasterKey::load_or_create(&dir.join("backup-master.key"), None).unwrap()
    }

    fn open_pii_store(dir: &Path) -> PiiStore {
        PiiStore::open(dir.join("pii-store"), &dir.join("pii-master.key")).unwrap()
    }

    fn open_deletion_log_and_ref(dir: &Path) -> (DeletionLog, FileTrustedReferenceStore) {
        let log = DeletionLog::open(&dir.join("deletion-log.sqlite")).unwrap();
        let reference = FileTrustedReferenceStore::new(dir.join("trusted-reference.bin"));
        (log, reference)
    }

    fn all_files_under(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(path);
                }
            }
        }
        out
    }

    #[test]
    fn full_backup_then_restore_to_an_empty_directory_yields_byte_identical_content_with_verified_hashes() {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\nhello world");
        write_file(&vault_root.join("note-b.record"), b"0\n---\nanother note");

        let pii_store = open_pii_store(dir.path());
        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        let manifest = backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();
        assert_eq!(manifest.generation, 0, "no deletions have happened yet");

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let outcome = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root).unwrap();
        assert_eq!(outcome.hash, manifest.hash, "restore must verify the same plaintext hash backup computed");

        assert_eq!(std::fs::read(destination.join("note-a.record")).unwrap(), b"0\n---\nhello world");
        assert_eq!(std::fs::read(destination.join("note-b.record")).unwrap(), b"0\n---\nanother note");
        assert_eq!(outcome.restored_note_ids, vec!["note-a".to_string(), "note-b".to_string()]);
    }

    /// Post-N1 follow-up: renamed and re-asserted. Before this fix, a
    /// backup whose own embedded reference snapshot was behind the live one
    /// refused the WHOLE restore with `ReferenceStale`, even though
    /// `note-a` here was never deleted — only an UNRELATED note was. That
    /// blanket refusal is exactly the behavior the user's requirements
    /// asked to relax: a stale backup must now restore the notes it holds
    /// that were never deleted, excluding nothing, since nothing in this
    /// backup was ever deleted.
    #[test]
    fn restore_from_a_stale_backup_still_restores_notes_untouched_by_the_unrelated_later_deletion() {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\nhello world");

        let pii_store = open_pii_store(dir.path());
        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        // Take the backup while the reference is still at generation 0.
        backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();

        // Now advance the LIVE log/reference past what the backup captured
        // — but note-a itself, the only note in this backup, was never
        // deleted; only an unrelated note was.
        log.record_deletion("note-unrelated", &reference).unwrap();
        let live = reference.read().unwrap().unwrap();
        assert_eq!(live.generation, 1, "the live reference must now be ahead of the backup's embedded snapshot");

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let outcome = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root)
            .expect("a stale backup must restore notes the later, unrelated deletion never touched");
        assert_eq!(outcome.restored_note_ids, vec!["note-a".to_string()]);
        assert!(outcome.excluded_deleted_note_ids.is_empty(), "note-a was never deleted");
        assert_eq!(std::fs::read(destination.join("note-a.record")).unwrap(), b"0\n---\nhello world");
    }

    /// Post-N1 follow-up: re-asserted. The note under test still must never
    /// reappear — that goal is unchanged — but the user's new requirement 1
    /// means the OUTCOME is now `Ok`, with the note excluded, rather than a
    /// blanket `Err(DeletedSinceBackup)` refusing the whole restore. This
    /// backup held only `note-a`, so there is nothing else to restore here;
    /// `an_old_backup_holding_a_later_deleted_note_restores_the_rest_and_excludes_only_that_note`
    /// covers the case where OTHER notes in the same stale backup survive.
    #[test]
    fn a_note_deleted_after_the_backup_was_taken_does_not_reappear_on_restore_even_though_the_backups_own_snapshot_predates_the_deletion(
    ) {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\npii://note-a");

        let pii_store = open_pii_store(dir.path());
        let op = Uuid::new_v4();
        pii_store.stage("note-a", b"sensitive content", op).unwrap();
        pii_store.commit_staged("note-a", op).unwrap();

        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        // Backup taken while note-a is still live.
        backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();

        // THEN delete note-a through the same deletion-log mechanism
        // `storage`'s Task 6 sensitive-write pipeline drives
        // (`DeletionLog::record_deletion` against the live trusted
        // reference), recording it in the LIVE log.
        log.record_deletion("note-a", &reference).unwrap();
        assert!(log.is_deleted("note-a").unwrap());

        // Restore from the EARLIER backup against that now-updated live log.
        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let outcome = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root)
            .expect("the restore itself must succeed; the deleted note is excluded, not a reason to refuse everything");
        assert_eq!(outcome.excluded_deleted_note_ids, vec!["note-a".to_string()]);
        assert!(outcome.restored_note_ids.is_empty(), "this backup held only note-a, which is excluded");
        assert!(!destination.join("note-a.record").exists(), "the deleted note must never reappear");
        assert!(!restored_pii_root.join("note-a.pii").exists(), "the deleted note's ciphertext must never reappear");
    }

    #[test]
    fn a_backup_taken_while_a_deletion_is_still_pending_and_then_restored_does_not_expose_the_note_that_deletion_targets(
    ) {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\npii://note-a");

        let pii_store = open_pii_store(dir.path());
        let op = Uuid::new_v4();
        pii_store.stage("note-a", b"sensitive content", op).unwrap();
        pii_store.commit_staged("note-a", op).unwrap();

        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        // Pause record_deletion exactly at AfterGenerationWrittenPending:
        // the log row exists and is Pending, the trusted reference has NOT
        // been updated yet.
        let (paused_tx, paused_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let injector = FaultInjector::pause_at(FaultPoint::AfterGenerationWrittenPending, paused_tx, resume_rx);

        let deleter_log = &log;
        let deleter_reference_path = dir.path().join("trusted-reference.bin");
        std::thread::scope(|scope| {
            let handle = scope.spawn(move || {
                let reference_for_thread = FileTrustedReferenceStore::new(deleter_reference_path.clone());
                deleter_log.record_deletion_with_injector("note-a", &reference_for_thread, &injector).unwrap()
            });

            // Wait until record_deletion is paused mid-protocol.
            paused_rx.recv().unwrap();

            // While paused: the log row is Pending (note-a already reads as
            // deleted), but the trusted reference has NOT been updated. The
            // deleter thread is parked INSIDE record_deletion_with_injector
            // holding `log`'s own in-process connection mutex for the whole
            // pause (by design, mirroring storage::fault's pause_at), so
            // this check opens a SEPARATE DeletionLog connection onto the
            // same SQLite file rather than calling `log.is_deleted` here,
            // which would deadlock against that held mutex.
            let concurrent_reader = DeletionLog::open(log.db_path()).unwrap();
            assert!(concurrent_reader.is_deleted("note-a").unwrap(), "is_deleted must already be true inside the pending window");
            assert!(reference.read().unwrap().is_none(), "the trusted reference must not be updated yet, mid-pause");

            // Run backup() against this same vault WHILE paused.
            let manifest = backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();
            assert_eq!(manifest.generation, 0, "the backup's own snapshot predates the pending deletion's reference update");

            // Resume and let record_deletion complete normally.
            resume_tx.send(()).unwrap();
            let receipt = handle.join().unwrap();
            assert_eq!(receipt.generation, 1);
        });

        // Now the live trusted reference is fully updated (generation 1).
        let live = reference.read().unwrap().unwrap();
        assert_eq!(live.generation, 1);

        // Restore from the backup taken mid-pending-window against the NOW
        // fully-updated live trusted ref.
        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let result = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root);
        // Post-N1 follow-up: check 2 (the specific per-note cross-check)
        // still runs before check 1 and still names note-a explicitly, via
        // `RestoreOutcome::excluded_deleted_note_ids` on a NOW-SUCCESSFUL
        // restore rather than `Err(DeletedSinceBackup)` refusing the whole
        // operation. `is_deleted`'s "pending window is already
        // non-consultable" rule (C5 line 409) still holds exactly as it did
        // before this fix: the live log already reports note-a deleted at
        // the moment restore's check 2 runs, regardless of whether
        // record_deletion's step 2/3 had completed yet when the backup
        // itself was taken — only the OUTCOME for the backup's OTHER,
        // unaffected notes (there are none in this backup) has changed.
        let outcome = result.expect("the restore must succeed; note-a is excluded, not a reason to refuse everything");
        assert_eq!(outcome.excluded_deleted_note_ids, vec!["note-a".to_string()]);
        assert!(outcome.restored_note_ids.is_empty(), "this backup held only note-a");
        assert!(!destination.join("note-a.record").exists(), "note-a must never be exposed");
        assert!(!restored_pii_root.join("note-a.pii").exists(), "note-a's ciphertext must never be exposed");
        // note-a must not be readable/present in any form.
        if destination.exists() {
            for path in all_files_under(&destination) {
                assert!(!path.to_string_lossy().contains("note-a"), "note-a must not appear in the restored output at all");
            }
        }
    }

    #[test]
    fn a_second_backup_to_an_already_used_target_replaces_rather_than_corrupts_the_prior_one_and_restore_still_round_trips(
    ) {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\nfirst version");

        let pii_store = open_pii_store(dir.path());
        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        let first_manifest = backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();

        // Mutate the vault and record a deletion so generation advances.
        write_file(&vault_root.join("note-a.record"), b"1\n---\nsecond version");
        write_file(&vault_root.join("note-c.record"), b"0\n---\nbrand new note");
        log.record_deletion("note-unrelated-for-generation-bump", &reference).unwrap();

        let second_manifest = backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();
        assert_ne!(second_manifest.hash, first_manifest.hash);
        assert_eq!(second_manifest.generation, 1, "the embedded reference snapshot must be the SECOND backup's own generation, not stale from the first");

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let outcome = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root).unwrap();
        assert_eq!(outcome.hash, second_manifest.hash, "restore must reflect the SECOND backup, not a corrupted merge");
        assert_eq!(outcome.generation, 1);

        assert_eq!(std::fs::read(destination.join("note-a.record")).unwrap(), b"1\n---\nsecond version");
        assert_eq!(std::fs::read(destination.join("note-c.record")).unwrap(), b"0\n---\nbrand new note");
        assert_eq!(outcome.restored_note_ids.len(), 2, "exactly the second backup's two notes, not a merge of both backups' files");
    }

    #[test]
    fn restore_on_an_empty_target_is_target_empty_not_a_panic_or_silent_success() {
        let dir = tempdir().unwrap();
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));
        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");

        let result = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root);
        assert!(matches!(result, Err(BackupError::TargetEmpty)), "got {result:?}");
        assert!(!destination.exists());
    }

    #[test]
    fn restore_refuses_to_write_into_a_non_empty_destination() {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\nhello world");

        let pii_store = open_pii_store(dir.path());
        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));
        backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();

        let destination = dir.path().join("restored");
        write_file(&destination.join("pre-existing.txt"), b"already here");
        let restored_pii_root = dir.path().join("restored-pii-store");

        let result = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root);
        assert!(matches!(result, Err(BackupError::AlreadyExists)), "got {result:?}");
        // Pre-existing content must be untouched.
        assert_eq!(std::fs::read(destination.join("pre-existing.txt")).unwrap(), b"already here");
    }

    #[test]
    fn restore_with_a_tampered_blob_fails_as_hash_mismatch_never_as_a_silent_wrong_result() {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\nhello world");

        let pii_store = open_pii_store(dir.path());
        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target_dir = dir.path().join("backup-target");
        let target = FilesystemBackupTarget::new(target_dir.clone());
        backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();

        let blob_path = target_dir.join(BLOB_NAME);
        let mut bytes = std::fs::read(&blob_path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        std::fs::write(&blob_path, &bytes).unwrap();

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let result = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root);
        assert!(matches!(result, Err(BackupError::HashMismatch)), "got {result:?}");
        assert!(!destination.exists() || all_files_under(&destination).is_empty());
    }

    #[test]
    fn restore_writes_the_pii_ciphertext_back_to_the_live_pii_store_byte_identical_to_what_was_backed_up() {
        // CRITICAL regression test: restore()'s extraction loop must write
        // snapshot.pii_files to the PiiStore's live location, not just the
        // vault_files. Before the fix, a restored sensitive note is left as
        // a dangling pii:// pointer with no ciphertext behind it at all,
        // while restore() still returns Ok.
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\npii://note-a");

        let pii_store = open_pii_store(dir.path());
        let op = Uuid::new_v4();
        pii_store.stage("note-a", b"sensitive content for round trip", op).unwrap();
        pii_store.commit_staged("note-a", op).unwrap();

        // The exact live ciphertext bytes backed up, captured independently
        // of restore(), so this test does not rely on restore() itself to
        // tell us what "correct" looks like.
        let original_pii_bytes = std::fs::read(pii_store.root().join("note-a.pii")).unwrap();

        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let outcome = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root).unwrap();
        assert_eq!(outcome.restored_note_ids, vec!["note-a".to_string()]);

        let restored_pii_path = restored_pii_root.join("note-a.pii");
        assert!(restored_pii_path.exists(), "restore() must write the .pii ciphertext back to the live PiiStore location, not just the Markdown pointer");
        let restored_pii_bytes = std::fs::read(&restored_pii_path).unwrap();
        assert_eq!(restored_pii_bytes, original_pii_bytes, "the restored .pii bytes must be byte-identical to what was backed up");

        // And it must actually decrypt back to the original plaintext via a
        // PiiStore opened on the restored root (end-to-end proof, not just a
        // byte comparison).
        let restored_pii_store = PiiStore::open(restored_pii_root.clone(), &dir.path().join("pii-master.key")).unwrap();
        assert_eq!(restored_pii_store.get("note-a").unwrap(), b"sensitive content for round trip");
    }

    #[test]
    fn sentinel_exception_does_not_fire_when_the_live_log_has_completed_history_but_the_reference_is_missing() {
        // IMPORTANT item 1: two completed deletions, then the reference file
        // is lost/corrupted (deleted, simulating loss). DeletionLog::reconcile
        // would correctly refuse this state (Err(ReferenceMissing)); restore()
        // must do the same, NOT silently proceed via the "no deletions ever"
        // sentinel exception just because the reference happens to be absent.
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\nhello world");

        let pii_store = open_pii_store(dir.path());
        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        // Take a backup BEFORE any deletion has ever happened: its embedded
        // reference snapshot is the sentinel (no_deletions_ever_reference).
        backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();

        // Now record two completed deletions on the live log.
        log.record_deletion("note-unrelated-1", &reference).unwrap();
        log.record_deletion("note-unrelated-2", &reference).unwrap();
        assert!(!log.is_empty().unwrap(), "the live log now has real history");

        // Simulate the reference being lost/corrupted: delete the reference
        // file entirely, so live_trusted_ref.read() returns Ok(None) even
        // though the log itself has two completed rows.
        let reference_path = dir.path().join("trusted-reference.bin");
        std::fs::remove_file(&reference_path).unwrap();
        assert!(reference.read().unwrap().is_none(), "the live reference must now read as absent");

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let result = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root);
        assert!(
            matches!(result, Err(BackupError::DeletionLogRejected(DeletionLogError::ReferenceMissing))),
            "a live log with real history but a missing reference must fail closed via the normal path (ReferenceMissing), not silently proceed via the sentinel exception, got {result:?}"
        );
        assert!(!destination.exists() || all_files_under(&destination).is_empty(), "destination must remain completely empty");
    }

    #[test]
    fn sentinel_exception_still_fires_when_the_live_log_is_genuinely_empty_and_the_reference_is_absent() {
        // IMPORTANT item 1 (part b): confirm the "genuinely never deleted"
        // case still works after the fix — reference absent AND log
        // genuinely empty must still take the sentinel exception and succeed.
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\nhello world");

        let pii_store = open_pii_store(dir.path());
        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();
        assert!(log.is_empty().unwrap(), "no deletion has ever been recorded");
        assert!(reference.read().unwrap().is_none());

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let outcome = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root).unwrap();
        assert_eq!(outcome.restored_note_ids, vec!["note-a".to_string()]);
    }

    #[test]
    fn restoring_from_a_sentinel_backup_while_the_deletion_is_still_pending_is_rejected_not_silently_succeeded(
    ) {
        // IMPORTANT item 3: restore from a sentinel-embedding backup taken
        // WHILE a deletion is still mid-pending (live reference still None,
        // log has a Pending row) — WITHOUT resuming the deletion first. This
        // is distinct from the existing pending-then-resumed-then-restore
        // test: here restore() runs against the STILL-PAUSED state.
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\npii://note-a");

        let pii_store = open_pii_store(dir.path());
        let op = Uuid::new_v4();
        pii_store.stage("note-a", b"sensitive content", op).unwrap();
        pii_store.commit_staged("note-a", op).unwrap();

        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        let (paused_tx, paused_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let injector = FaultInjector::pause_at(FaultPoint::AfterGenerationWrittenPending, paused_tx, resume_rx);

        let deleter_log = &log;
        let deleter_reference_path = dir.path().join("trusted-reference.bin");
        std::thread::scope(|scope| {
            let handle = scope.spawn(move || {
                let reference_for_thread = FileTrustedReferenceStore::new(deleter_reference_path.clone());
                deleter_log.record_deletion_with_injector("note-a", &reference_for_thread, &injector).unwrap()
            });

            paused_rx.recv().unwrap();

            let concurrent_reader = DeletionLog::open(log.db_path()).unwrap();
            assert!(concurrent_reader.is_deleted("note-a").unwrap(), "is_deleted must already be true inside the pending window");
            assert!(reference.read().unwrap().is_none(), "the trusted reference must not be updated yet, mid-pause");
            assert!(!concurrent_reader.is_empty().unwrap(), "the log has a Pending row, so it is NOT empty, even mid-pause");

            let manifest = backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();
            assert_eq!(manifest.generation, 0, "the backup's own snapshot predates the pending deletion's reference update, so it embeds the sentinel");

            // Restore WHILE STILL PAUSED, against the still-pending state —
            // the live reference is still None and the log still has the
            // Pending row for note-a. Post-N1 follow-up: `check_1`'s
            // `reconcile` call safely rolls this forward (generation 1,
            // still Pending, no reference yet, is exactly `reconcile`'s own
            // "first-ever deletion crashed mid-protocol" roll-forward case
            // — the same completion a live `Vault::open` would perform),
            // after which the backup's sentinel-embedding snapshot is
            // `BackupReferenceStale` against the now-rolled-forward live
            // reference. The note is still never exposed — caught by check
            // 2's `is_deleted` (true for any row state) and excluded — but
            // the restore as a whole now succeeds, reporting the exclusion
            // explicitly rather than refusing everything.
            //
            // Uses a SEPARATE DeletionLog connection onto the same SQLite
            // file, exactly like `concurrent_reader` above: `log` itself is
            // held by the deleter thread's blocked call for the whole pause
            // (by design), so calling `is_empty`/`is_deleted` through `log`
            // directly here would deadlock against that held mutex.
            let restore_time_reader = DeletionLog::open(log.db_path()).unwrap();
            let destination = dir.path().join("restored");
            let restored_pii_root = dir.path().join("restored-pii-store");
            let result = restore_with_pii_root(&target, &master_key, &destination, &restore_time_reader, &reference, &restored_pii_root);
            let destination_untouched = !destination.exists() || all_files_under(&destination).is_empty();

            // Resume BEFORE asserting: a failed assertion here must fail the
            // test, not leave the deleter thread parked forever and hang the
            // scope (which `cargo mutants` reports as a timeout, not a kill).
            resume_tx.send(()).unwrap();
            let receipt = handle.join().unwrap();
            assert_eq!(receipt.generation, 1);

            let outcome = result.expect("the restore must succeed; the still-pending note is excluded, not a reason to refuse everything");
            assert_eq!(outcome.excluded_deleted_note_ids, vec!["note-a".to_string()]);
            assert!(outcome.restored_note_ids.is_empty(), "this backup held only the still-pending note-a");
            assert!(destination_untouched || !destination.join("note-a.record").exists(), "note-a must never be exposed, pending or not");
            assert!(!restored_pii_root.join("note-a.pii").exists(), "note-a's ciphertext must never be exposed, pending or not");
        });
    }

    /// Final-review re-review, N1: `verify_safe_to_expose_after_restore`
    /// used to check only that the backup's own embedded reference was not
    /// behind whatever `live_trusted_ref.read()` returned, plus that the
    /// LIVE LOG's chain was internally self-consistent (`verify_chain`) — it
    /// never confirmed the live log's own head actually agrees with that
    /// live reference. A deletion whose `record_deletion` call logs the
    /// generation but whose tombstone write never lands (a crash, or any
    /// later pipeline failure) leaves the real ciphertext live; if the live
    /// deletion-log FILE is then lost (deleted, rolled back to an earlier
    /// backup of just that file) while the separate, DPAPI-protected
    /// trusted-reference file survives untouched, the old check still
    /// passed — a fresh, empty log is a trivially "valid" chain — and
    /// `restore()` exposed the deleted note's real content. Reusing
    /// `reconcile` (which already proves the live log's head agrees with
    /// the live reference, not merely that the log is self-consistent)
    /// closes this: an empty log against a reference claiming generation 1
    /// is exactly the `(None, Some(r))` case `reconcile` already fails
    /// closed on as `ReferenceMissing`.
    #[test]
    fn restore_fails_closed_when_the_live_deletion_log_itself_is_lost_even_though_its_chain_is_trivially_valid() {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\npii://note-a");
        write_file(&vault_root.join("note-b.record"), b"0\n---\nordinary note");

        let pii_store = open_pii_store(dir.path());
        let op = Uuid::new_v4();
        pii_store.stage("note-a", b"REAL SECRET", op).unwrap();
        pii_store.commit_staged("note-a", op).unwrap();

        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        // record_deletion runs to completion (generation 1, reference
        // updated) — but note-a's tombstone write is never performed here
        // (this unit test operates below storage's write pipeline, so
        // there is no tombstone to send; this directly models the
        // crashed/failed-tombstone state the review's repro constructs via
        // storage's own pipeline). The real ciphertext stays live under
        // note-a's pointer, exactly as it would after a crash between
        // DeletionLog::record_deletion completing and the tombstone commit.
        log.record_deletion("note-a", &reference).unwrap();
        assert!(reference.read().unwrap().is_some(), "the reference survives — this is the part that must NOT be lost for the bug to be absent");
        assert_eq!(pii_store.get("note-a").unwrap(), b"REAL SECRET", "the real ciphertext is still live — the tombstone never landed");

        let manifest = backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();
        assert_eq!(manifest.generation, 1, "the backup's own snapshot correctly reflects the completed deletion");

        // Lose the live deletion-log FILE (not the reference) — e.g. the
        // SQLite file is deleted, or a stale copy from before the deletion
        // was ever recorded is restored in its place. A fresh DeletionLog
        // opened at the same path is an empty, trivially self-consistent
        // chain. `log`'s own connection must be dropped first: on Windows,
        // SQLite's open file handle would otherwise block the removal.
        let db_path = log.db_path().to_path_buf();
        drop(log);
        std::fs::remove_file(&db_path).unwrap();
        let log_after_loss = DeletionLog::open(&db_path).unwrap();
        assert!(log_after_loss.is_empty().unwrap(), "the live log is now empty — lost, not merely stale");
        assert!(reference.read().unwrap().is_some(), "the reference itself was never touched by this loss");

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let result = restore_with_pii_root(&target, &master_key, &destination, &log_after_loss, &reference, &restored_pii_root);

        assert!(
            result.is_err(),
            "restore must fail closed when the live log cannot vouch for the reference it is being compared against, got {result:?}"
        );
        let destination_untouched = !destination.exists() || all_files_under(&destination).is_empty();
        assert!(destination_untouched, "nothing must be written, including note-a's real content, when the live log itself cannot be trusted");
    }

    /// Post-N1 follow-up, requirement 1: a backup taken BEFORE the latest
    /// deletion must still restore its still-valid notes, provided the live
    /// deletion log is complete, authenticated, and consistent with its own
    /// trusted reference (exactly what `reconcile`, reused by
    /// `verify_safe_to_expose_after_restore` since N1, already proves).
    /// Before this fix, check 1 rejected ANY backup whose own embedded
    /// reference was behind the live one — even when every note the backup
    /// held was still perfectly valid and the only thing that happened since
    /// was an unrelated deletion. The user's note-a and note-c here were
    /// never touched by the later deletion of note-b; their restore must not
    /// depend on it.
    #[test]
    fn a_backup_older_than_the_latest_deletion_still_restores_notes_the_deletion_never_touched() {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"1\n---\nstill valid note a");
        write_file(&vault_root.join("note-c.record"), b"1\n---\nstill valid note c");

        let pii_store = open_pii_store(dir.path());
        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        // Backup taken BEFORE any deletion has ever happened.
        let old_manifest = backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();
        assert_eq!(old_manifest.generation, 0);

        // A later, unrelated deletion (note-b never appeared in the vault
        // above at all — this models "something elsewhere was deleted").
        log.record_deletion("note-b", &reference).unwrap();
        assert_eq!(reference.read().unwrap().unwrap().generation, 1, "the live reference has moved on");

        // Restoring the OLD backup (generation 0, strictly behind the live
        // reference's generation 1) must still succeed: the live log is
        // complete and consistent with its reference (reconcile would
        // accept it), and neither note-a nor note-c was ever deleted.
        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let outcome = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root)
            .expect("a stale-but-otherwise-valid backup must restore the notes it holds that were never deleted");
        assert_eq!(outcome.restored_note_ids, vec!["note-a".to_string(), "note-c".to_string()]);
        assert!(outcome.excluded_deleted_note_ids.is_empty(), "neither restored note was ever deleted");
        assert_eq!(std::fs::read(destination.join("note-a.record")).unwrap(), b"1\n---\nstill valid note a");
        assert_eq!(std::fs::read(destination.join("note-c.record")).unwrap(), b"1\n---\nstill valid note c");
    }

    /// Post-N1 follow-up, requirement 2: an OLD backup that also happens to
    /// hold a note deleted AFTER it was taken must still exclude that note,
    /// combining staleness-tolerance with per-note exclusion in one restore.
    #[test]
    fn an_old_backup_holding_a_later_deleted_note_restores_the_rest_and_excludes_only_that_note() {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"1\n---\npii://note-a");
        write_file(&vault_root.join("note-b.record"), b"1\n---\nstays valid");

        let pii_store = open_pii_store(dir.path());
        let op = Uuid::new_v4();
        pii_store.stage("note-a", b"SENSITIVE, SOON DELETED", op).unwrap();
        pii_store.commit_staged("note-a", op).unwrap();

        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        // Backup taken while BOTH notes are still fully valid.
        let old_manifest = backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();
        assert_eq!(old_manifest.generation, 0);

        // note-a is deleted AFTER this backup was taken.
        log.record_deletion("note-a", &reference).unwrap();

        // A SECOND, unrelated deletion happens too, so the live reference is
        // now TWO generations ahead of this backup's own snapshot —
        // confirming exclusion does not depend on being exactly one
        // generation stale.
        log.record_deletion("note-does-not-exist-in-this-backup", &reference).unwrap();
        assert_eq!(reference.read().unwrap().unwrap().generation, 2);

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let outcome = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root)
            .expect("the old backup must restore note-b and exclude only note-a, not be rejected wholesale");
        assert_eq!(outcome.restored_note_ids, vec!["note-b".to_string()]);
        assert_eq!(outcome.excluded_deleted_note_ids, vec!["note-a".to_string()]);
        assert!(!destination.join("note-a.record").exists());
        assert!(!restored_pii_root.join("note-a.pii").exists());
        assert_eq!(std::fs::read(destination.join("note-b.record")).unwrap(), b"1\n---\nstays valid");
    }

    /// Post-N1 follow-up, requirement 3: an old backup must still fail
    /// closed — not be waved through by the new staleness tolerance — when
    /// the live log itself cannot be trusted (here: the log file is lost,
    /// exactly N1's scenario, but now checked specifically against a STALE
    /// backup rather than N1's same-generation one). Staleness tolerance
    /// must never become a loophole for an unverifiable live log.
    #[test]
    fn a_stale_backup_still_fails_closed_when_the_live_log_cannot_be_trusted() {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"1\n---\nnever deleted");

        let pii_store = open_pii_store(dir.path());
        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();

        // Advance the live reference so the backup above is now stale...
        log.record_deletion("note-elsewhere", &reference).unwrap();
        // ...then lose the live log file itself (reconcile can no longer
        // vouch that the reference it just advanced actually matches what
        // the log holds, because there is no log left to check).
        let db_path = log.db_path().to_path_buf();
        drop(log);
        std::fs::remove_file(&db_path).unwrap();
        let log_after_loss = DeletionLog::open(&db_path).unwrap();
        assert!(log_after_loss.is_empty().unwrap());

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let result = restore_with_pii_root(&target, &master_key, &destination, &log_after_loss, &reference, &restored_pii_root);
        assert!(
            result.is_err(),
            "a stale backup must still fail closed when the live log cannot be trusted, got {result:?}"
        );
        let destination_untouched = !destination.exists() || all_files_under(&destination).is_empty();
        assert!(destination_untouched, "nothing must be written when the live log is untrustworthy, stale backup or not");
    }

    /// Reviewer-found race (post-N1 follow-up to the post-N1 follow-up):
    /// `TrustedReferenceStore` wrapper that records a deletion on its FIRST
    /// `read()` call — which is the very first thing check 1 does inside
    /// `verify_safe_to_expose_after_restore`/`reconcile`. This simulates a
    /// deletion landing in the gap between check 2's note-id lookups and
    /// check 1's own read of the live reference.
    struct DeleteOnFirstRead<'a> {
        inner: &'a dyn TrustedReferenceStore,
        log: &'a DeletionLog,
        note_id_to_delete: &'a str,
        fired: std::cell::Cell<bool>,
    }

    impl<'a> TrustedReferenceStore for DeleteOnFirstRead<'a> {
        fn read(&self) -> Result<Option<GenerationRef>, DeletionLogError> {
            if !self.fired.replace(true) {
                self.log.record_deletion(self.note_id_to_delete, self.inner).unwrap();
            }
            self.inner.read()
        }
        fn write(&self, r: &GenerationRef) -> Result<(), DeletionLogError> {
            self.inner.write(r)
        }
    }

    /// Reviewer-found race, now fixed: a deletion landing in the window
    /// between check 2's ORIGINAL note-id lookups and check 1's own read of
    /// the live reference must still be caught. Before this fix,
    /// `deleted_since_backup` was computed once, before check 1 ran; a
    /// deletion racing in after that computation but before (or during)
    /// check 1 was invisible to it, and — once staleness alone stopped
    /// unconditionally blocking the whole restore — nothing else caught it,
    /// so the raced note was written out already deleted. Fixed by
    /// recomputing the exclusion list AFTER check 1 has resolved, as the
    /// very last read of the live log before anything is written.
    #[test]
    fn a_deletion_racing_in_between_check_2_and_check_1_is_still_caught_not_resurrected() {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\npii://note-a");
        write_file(&vault_root.join("note-b.record"), b"0\n---\nstays valid");

        let pii_store = open_pii_store(dir.path());
        let op = Uuid::new_v4();
        pii_store.stage("note-a", b"RACED SECRET MUST NOT RESURFACE", op).unwrap();
        pii_store.commit_staged("note-a", op).unwrap();

        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        // Backup taken while BOTH notes are still fully valid — no
        // deletion has happened yet, so this is a sentinel backup, which is
        // exactly the case where the old code's blanket staleness rejection
        // used to (incidentally) close this same race window.
        backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();

        // An UNRELATED deletion happens first, so the backup is already
        // provably stale (not the never-deleted sentinel case) by the time
        // restore runs — forcing execution through the NEW BackupReferenceStale
        // fall-through path, not the sentinel exception.
        log.record_deletion("note-unrelated", &reference).unwrap();

        // note-a's deletion races in on check 1's FIRST read of the live
        // reference — after check 2's original lookups would have already
        // run, in the pre-fix code.
        let racer = DeleteOnFirstRead {
            inner: &reference,
            log: &log,
            note_id_to_delete: "note-a",
            fired: std::cell::Cell::new(false),
        };

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let outcome = restore_with_pii_root(&target, &master_key, &destination, &log, &racer, &restored_pii_root)
            .expect("the restore itself must still succeed for note-b");
        assert_eq!(
            outcome.excluded_deleted_note_ids,
            vec!["note-a".to_string()],
            "note-a's race-landed deletion must still be excluded, not resurrected"
        );
        assert_eq!(outcome.restored_note_ids, vec!["note-b".to_string()]);
        assert!(!destination.join("note-a.record").exists(), "the raced note must never be written");
        assert!(!restored_pii_root.join("note-a.pii").exists(), "the raced note's ciphertext must never be written");
    }

    /// Post-N1 follow-up, requirement 5 ("ripristino effettivo tramite API
    /// storage"): confirms the stale-but-valid restore is not just correct
    /// at the `vault-security` level but produces a vault the `storage`
    /// crate's own API can actually read — mirrors I1's own test shape.
    /// Exercised fully in `crates/storage/tests/a3_backup_restore.rs`
    /// (`a_stale_backup_restores_through_storages_own_api_excluding_only_the_later_deleted_note`);
    /// this comment anchors the cross-crate link for a reader of this file.
    ///
    /// Final whole-branch review, CRITICAL C1: a backup taken AFTER a
    /// deletion (its own embedded reference already reflects it) must be
    /// restorable. Before the fix, check 2 rejected the WHOLE restore as
    /// `DeletedSinceBackup` because the deleted note's id appears anywhere in
    /// the snapshot, so a vault that had ever deleted one sensitive note
    /// could never be restored again, not even its other notes.
    #[test]
    fn a_backup_taken_after_a_deletion_restores_its_other_notes_and_excludes_the_deleted_one() {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"2\n---\npii://note-a");
        write_file(&vault_root.join("note-b.record"), b"1\n---\nplain note b");

        let pii_store = open_pii_store(dir.path());
        let op1 = Uuid::new_v4();
        pii_store.stage("note-a", b"REAL SENSITIVE CONTENT", op1).unwrap();
        pii_store.commit_staged("note-a", op1).unwrap();

        let (log, reference) = open_deletion_log_and_ref(dir.path());
        // Delete note-a exactly as the storage pipeline does: log first,
        // then the tombstone (empty plaintext) replaces the live ciphertext.
        log.record_deletion("note-a", &reference).unwrap();
        let op2 = Uuid::new_v4();
        pii_store.stage("note-a", b"", op2).unwrap();
        pii_store.commit_staged("note-a", op2).unwrap();
        pii_store.delete_before_image("note-a", op2).unwrap();

        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));
        let manifest = backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();
        assert_eq!(manifest.generation, 1, "the backup postdates the deletion");

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let outcome = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root)
            .expect("a backup taken after the deletion must restore, not be rejected wholesale");
        assert_eq!(outcome.restored_note_ids, vec!["note-b".to_string()]);
        assert_eq!(outcome.excluded_deleted_note_ids, vec!["note-a".to_string()], "the excluded note must be reported, never silently dropped");
        assert_eq!(std::fs::read(destination.join("note-b.record")).unwrap(), b"1\n---\nplain note b");
        assert!(!destination.join("note-a.record").exists(), "the deleted note's record must not be restored");
        assert!(!restored_pii_root.join("note-a.pii").exists(), "the deleted note's ciphertext must not be restored");
    }

    /// C1, the reason exclusion is unconditional rather than "only if the
    /// backed-up copy is a tombstone": `record_deletion` runs BEFORE the
    /// tombstone ciphertext replaces the live one, so a backup taken in that
    /// window has a current reference but still holds the REAL content.
    /// Restore must still drop it.
    #[test]
    fn a_deleted_note_whose_backed_up_ciphertext_is_still_the_real_content_is_excluded_too() {
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"1\n---\npii://note-a");

        let pii_store = open_pii_store(dir.path());
        let op = Uuid::new_v4();
        pii_store.stage("note-a", b"REAL SENSITIVE CONTENT", op).unwrap();
        pii_store.commit_staged("note-a", op).unwrap();

        let (log, reference) = open_deletion_log_and_ref(dir.path());
        log.record_deletion("note-a", &reference).unwrap();

        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));
        backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        let outcome = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root).unwrap();
        assert!(outcome.restored_note_ids.is_empty());
        assert_eq!(outcome.excluded_deleted_note_ids, vec!["note-a".to_string()]);
        assert!(all_files_under(&destination).is_empty() || !destination.exists());
        assert!(!restored_pii_root.join("note-a.pii").exists());
    }

    #[test]
    fn the_backup_blob_is_encrypted_under_a_subkey_distinct_from_the_master_key_itself() {
        // The HKDF subkey must not simply equal the master key bytes.
        let dir = tempdir().unwrap();
        let master_key = fresh_master_key(dir.path());
        let subkey = derive_backup_subkey(&master_key);
        assert_ne!(&subkey, master_key.expose_secret(), "the backup subkey must be cryptographically distinct from the raw master key");
    }

    /// Final whole-branch review, I4 #1: the only prior subkey test asserted
    /// "subkey != master key", which a constant `[0; 32]` also satisfies, so
    /// that mutant survived. Pins a known-answer vector (HKDF-SHA256 per RFC
    /// 5869, salt absent = 32 zero bytes, ikm = bytes 0..32, info =
    /// "superbrain-backup-v1", computed independently with Python's `hmac`),
    /// determinism, and that different master keys give different subkeys.
    #[test]
    fn derive_backup_subkey_matches_a_known_answer_and_depends_on_the_master_key() {
        let ikm: [u8; MASTER_KEY_LEN] = std::array::from_fn(|i| i as u8);
        let key = MasterKey::from_bytes(ikm, crate::keys::KeySource::Fresh);
        let subkey = derive_backup_subkey(&key);
        let hex: String = subkey.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "17d5fd5e35ab48a7eb46511f0635ffe33a96b75214607f2f11db1ae36955c4e7");

        assert_eq!(derive_backup_subkey(&key), subkey, "deterministic for the same master key");
        let mut other_bytes = ikm;
        other_bytes[0] ^= 0x01;
        let other = MasterKey::from_bytes(other_bytes, crate::keys::KeySource::Fresh);
        assert_ne!(derive_backup_subkey(&other), subkey, "a different master key must give a different subkey");
    }

    /// I4 #2: `backup.rs`'s own `fresh_nonce` (a separate function from
    /// `pii_store.rs`'s) collapsed to a constant survived, i.e. nonce reuse
    /// across backups under one subkey went undetected. Same check Task 8
    /// added for `pii_store::fresh_nonce`, plus the observable consequence:
    /// two backups of identical content produce different blobs.
    #[test]
    fn backup_fresh_nonce_never_repeats_and_two_identical_backups_use_different_nonces() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            let nonce = fresh_nonce();
            assert!(seen.insert(nonce), "backup fresh_nonce() produced a duplicate nonce: {nonce:?}");
        }

        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        write_file(&vault_root.join("note-a.record"), b"1\n---\nsame content");
        let pii_store = open_pii_store(dir.path());
        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target_1 = FilesystemBackupTarget::new(dir.path().join("t1"));
        let target_2 = FilesystemBackupTarget::new(dir.path().join("t2"));
        let m1 = backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target_1, &reference).unwrap();
        let m2 = backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target_2, &reference).unwrap();
        assert_eq!(m1.hash, m2.hash, "precondition: identical plaintext snapshots");
        let blob_1 = target_1.read_blob(BLOB_NAME).unwrap();
        let blob_2 = target_2.read_blob(BLOB_NAME).unwrap();
        assert_ne!(blob_1[..NONCE_LEN], blob_2[..NONCE_LEN], "two backups under one subkey must never share a nonce");
    }

    #[test]
    fn restore_must_not_silently_roll_back_live_pii_ciphertext_that_was_committed_after_the_backup_was_taken() {
        // Reviewer's exact repro (Task 7 round 2 finding): back up note-a
        // with content "OLD". Commit NEWER content to the SAME live store
        // afterward (a completely ordinary, independent write — no
        // deletion involved, so neither deletion-log check fires). Restore
        // to a brand-new, EMPTY vault destination (not even restoring the
        // vault's Markdown records matters here — the point is that
        // `destination_root` being empty says nothing about whether the
        // LIVE `pii_root` already has files). Before the fix, restore
        // silently overwrote the LIVE note-a.pii back to "OLD" ciphertext,
        // with no before-image kept: a silent rollback of current, valid
        // sensitive content. After the fix, restore must FAIL and the live
        // content must remain "NEWER", untouched, byte for byte.
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\npii://note-a");

        // The LIVE pii store — restore in this test targets THIS SAME
        // store's root, exactly like production's `restore` (not
        // `restore_with_pii_root` pointed at an isolated root), because the
        // whole point of the finding is restoring into a live, in-use
        // PiiStore.
        let pii_store = open_pii_store(dir.path());
        let op1 = Uuid::new_v4();
        pii_store.stage("note-a", b"OLD", op1).unwrap();
        pii_store.commit_staged("note-a", op1).unwrap();

        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));

        // Back up note-a while it still holds "OLD".
        backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();

        // Commit NEWER content to the SAME live store, after the backup was
        // taken. No deletion is involved anywhere in this sequence.
        let op2 = Uuid::new_v4();
        pii_store.stage("note-a", b"NEWER", op2).unwrap();
        pii_store.commit_staged("note-a", op2).unwrap();
        let live_pii_path = pii_store.root().join("note-a.pii");
        let live_bytes_before_restore = std::fs::read(&live_pii_path).unwrap();
        assert_eq!(pii_store.get("note-a").unwrap(), b"NEWER", "precondition: the live store now holds NEWER");

        // Attempt to restore into a brand-new, EMPTY vault destination,
        // targeting the SAME live `pii_store` the backup read from.
        let destination = dir.path().join("restored");
        assert!(!destination.exists(), "precondition: destination does not exist yet");
        let result = restore(&target, &master_key, &destination, &log, &reference, &pii_store);

        assert!(
            matches!(&result, Err(BackupError::PiiAlreadyExists { note_ids }) if note_ids == &["note-a".to_string()]),
            "restore must fail closed naming note-a, not silently succeed, got {result:?}"
        );

        // The live content must remain NEWER, untouched, byte for byte —
        // no silent rollback to OLD, and no before-image side effect either
        // (this path never touches PiiStore's own stage/commit machinery at
        // all).
        assert_eq!(std::fs::read(&live_pii_path).unwrap(), live_bytes_before_restore, "the live .pii file on disk must be byte-identical to before the restore attempt");
        assert_eq!(pii_store.get("note-a").unwrap(), b"NEWER", "the live content must remain NEWER, not rolled back to OLD");
    }

    #[test]
    fn restore_into_a_pii_root_with_no_existing_pii_files_still_succeeds_normally() {
        // Happy-path regression guard: the new existence guard must not
        // block the ordinary case where the target pii_root has nothing
        // live yet (a fresh isolated root, or a live store that has never
        // held this note).
        let dir = tempdir().unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        write_file(&vault_root.join("note-a.record"), b"0\n---\npii://note-a");

        let pii_store = open_pii_store(dir.path());
        let op = Uuid::new_v4();
        pii_store.stage("note-a", b"sensitive content", op).unwrap();
        pii_store.commit_staged("note-a", op).unwrap();

        let (log, reference) = open_deletion_log_and_ref(dir.path());
        let master_key = fresh_master_key(dir.path());
        let target = FilesystemBackupTarget::new(dir.path().join("backup-target"));
        backup_with_reference(&vault_root, &pii_store, &log, &master_key, &target, &reference).unwrap();

        let destination = dir.path().join("restored");
        let restored_pii_root = dir.path().join("restored-pii-store");
        assert!(!restored_pii_root.exists(), "precondition: no pii root exists yet at the restore target");

        let outcome = restore_with_pii_root(&target, &master_key, &destination, &log, &reference, &restored_pii_root).unwrap();
        assert_eq!(outcome.restored_note_ids, vec!["note-a".to_string()]);
        assert!(restored_pii_root.join("note-a.pii").exists());
        let restored_store = PiiStore::open(restored_pii_root.clone(), &dir.path().join("pii-master.key")).unwrap();
        assert_eq!(restored_store.get("note-a").unwrap(), b"sensitive content");
    }

    #[test]
    fn pii_writes_during_restore_go_through_a_sibling_temp_file_and_rename_never_a_direct_write() {
        // Required test 3 (atomicity, by code inspection + an observable
        // proxy): atomic_write_pii must never leave a stray temp file
        // behind on success, and the final file must be reachable only via
        // rename, never via a partially-written direct `std::fs::write`.
        // This exercises atomic_write_pii directly (the same function
        // restore()'s loop calls for every `.pii` file) and confirms: (a)
        // the final file's content is exactly what was requested, (b) no
        // `.tmp-` sibling is left behind, and (c) by inspection of
        // `atomic_write_pii` itself (see the function above in this same
        // module) the only call that ever touches the FINAL path is
        // `std::fs::rename` — there is no code path in which the final
        // path receives partial bytes directly.
        let dir = tempdir().unwrap();
        let pii_root = dir.path().join("pii-root");
        std::fs::create_dir_all(&pii_root).unwrap();
        let final_path = pii_root.join("note-a.pii");

        atomic_write_pii(&final_path, b"full content, never partial").unwrap();

        assert_eq!(std::fs::read(&final_path).unwrap(), b"full content, never partial");
        let leftover_tmp: Vec<_> = std::fs::read_dir(&pii_root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftover_tmp.is_empty(), "atomic_write_pii must leave no temp file behind on success: {leftover_tmp:?}");

        // Overwriting again (simulating a second restore of the same note
        // into an otherwise-cleared root) must also go cleanly through the
        // same rename, never corrupting the destination with a partial
        // write even though content differs in length.
        atomic_write_pii(&final_path, b"short").unwrap();
        assert_eq!(std::fs::read(&final_path).unwrap(), b"short", "rename must fully replace prior content, never append or truncate in place");
    }
}
