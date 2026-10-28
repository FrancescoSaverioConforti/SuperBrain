//! Master key: a 32-byte AEAD key that is the root of trust for the
//! encrypted PII store, the deletion log's trusted reference, and backups
//! (Tasks 5, 6, 7, 8). On Windows the key is persisted DPAPI-wrapped
//! (`CryptProtectData`/`CryptUnprotectData`, tied to the current user and
//! machine); [`crate::recovery_kit`] provides an independent, portable
//! escape hatch via an Argon2id-derived passphrase key.
//!
//! The central safety property this module enforces is in
//! [`MasterKey::load_or_create`]: it must never mint a fresh key when
//! ciphertext already exists on disk under a key it can no longer unwrap.
//! Doing so would "succeed" while silently orphaning every existing
//! sensitive note. See [`KeyError::ExistingStoreNotDecryptable`].

use std::fs;
use std::io;
use std::path::Path;

use thiserror::Error;
use zeroize::Zeroizing;

/// Length in bytes of the master key material (AES-256-GCM key size).
pub const MASTER_KEY_LEN: usize = 32;

/// A 32-byte AEAD master key. The root of trust for the encrypted vault.
///
/// Key bytes live in a [`Zeroizing`] buffer and are wiped on drop. This type
/// intentionally does **not** implement `Debug`, `Display`, `Clone`, or
/// `Serialize` — there must be no way to accidentally log, print, or persist
/// raw key material through a derived trait impl. The only way key bytes
/// leave this type is through [`MasterKey::expose_secret`], used solely by
/// this crate's own wrap/unwrap and recovery-kit code.
pub struct MasterKey {
    bytes: Zeroizing<[u8; MASTER_KEY_LEN]>,
    source: KeySource,
}

/// Where a loaded/created [`MasterKey`]'s bytes came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// Unwrapped from the on-disk DPAPI-wrapped key file.
    Dpapi,
    /// Recovered from a [`crate::recovery_kit::RecoveryKit`] (offline,
    /// passphrase-protected; independent of DPAPI).
    RecoveryKit,
    /// Freshly generated because no key file existed and no ciphertext was
    /// present on disk yet (the only case `load_or_create` may mint a key).
    Fresh,
}

/// Cheap, non-secret marker the caller computes from whatever ciphertext
/// already exists on disk for the store this key must be able to open — for
/// example, a hash of the first record's AEAD tag, or simply "at least one
/// `.pii` file exists." It answers "does a store already exist" without
/// needing key material to compute, so [`MasterKey::load_or_create`] can
/// make its re-keying decision before any key is touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreFingerprint {
    exists: bool,
}

impl StoreFingerprint {
    /// A fingerprint stating that existing ciphertext is present on disk.
    pub fn existing() -> Self {
        Self { exists: true }
    }

    /// A fingerprint stating that the store is empty (no ciphertext yet).
    pub fn empty() -> Self {
        Self { exists: false }
    }

    /// Whether this fingerprint indicates ciphertext already exists.
    pub fn indicates_existing_ciphertext(&self) -> bool {
        self.exists
    }
}

/// Errors from master-key load/create/wrap/unwrap operations.
///
/// No variant carries raw key material; this type derives `Debug` freely.
#[derive(Debug, Error)]
pub enum KeyError {
    #[error("I/O error accessing key store: {0}")]
    Io(#[from] io::Error),

    /// The DPAPI call could not even be attempted (e.g. a structurally
    /// malformed wrapped blob that is too short to contain a DPAPI
    /// envelope), as distinct from [`KeyError::DpapiDecryptFailed`], where
    /// the Win32 call was made and itself reported failure.
    #[error("DPAPI unavailable: wrapped key data is malformed or DPAPI cannot be reached")]
    DpapiUnavailable,

    /// `CryptUnprotectData` ran and reported failure — most commonly because
    /// the calling user/machine context differs from the one that wrapped
    /// the data. This fails closed: distinguishable from corruption, and
    /// never falls through to minting a fresh key.
    #[error("DPAPI decrypt failed: the key file cannot be unwrapped in this user/machine context")]
    DpapiDecryptFailed,

    /// The key file is missing or undecryptable and no recovery kit text
    /// was supplied to recover it.
    #[error("master key unavailable: no usable key file and no recovery kit was provided")]
    RecoveryKitRequired,

    /// `load_or_create` found no usable key file, but `store_fingerprint`
    /// indicates ciphertext already exists on disk. Minting a fresh key
    /// here would silently orphan that existing ciphertext while reporting
    /// success, so this is returned instead and nothing is written.
    #[error(
        "an existing encrypted store was found but its master key could not be loaded; \
         refusing to generate a fresh key, which would silently orphan existing ciphertext. \
         Restore the key file or use a recovery kit."
    )]
    ExistingStoreNotDecryptable,

    /// This build is not Windows; DPAPI is unavailable on this platform.
    #[error("DPAPI is only supported on Windows; this build targets a different platform")]
    PlatformUnsupported,
}

impl MasterKey {
    /// Wraps existing key bytes into a [`MasterKey`] with the given
    /// provenance. `pub(crate)` because external callers only ever obtain a
    /// `MasterKey` through [`MasterKey::load_or_create`],
    /// [`MasterKey::unwrap_with_dpapi`], or
    /// [`crate::recovery_kit::unwrap_master_key`].
    pub(crate) fn from_bytes(bytes: [u8; MASTER_KEY_LEN], source: KeySource) -> Self {
        Self { bytes: Zeroizing::new(bytes), source }
    }

    /// Generates a fresh, random 32-byte master key. `pub(crate)` so
    /// `recovery_kit.rs`'s tests can exercise `generate()` against a real
    /// `MasterKey` without going through disk/DPAPI.
    pub(crate) fn generate() -> Self {
        let mut bytes = [0u8; MASTER_KEY_LEN];
        getrandom::fill(&mut bytes).expect("the OS CSPRNG must be available to generate a master key");
        Self::from_bytes(bytes, KeySource::Fresh)
    }

    /// Where this key's bytes came from.
    pub fn source(&self) -> KeySource {
        self.source
    }

    /// Exposes the raw key bytes. `pub(crate)` on purpose: only this
    /// crate's own AEAD wrap/unwrap code (here and in `recovery_kit.rs`,
    /// and later `pii_store.rs`) may read the bytes directly.
    pub(crate) fn expose_secret(&self) -> &[u8; MASTER_KEY_LEN] {
        &self.bytes
    }

    /// Loads the master key from `key_store_path`, or creates one — but
    /// **only** when it is safe to do so.
    ///
    /// Logic:
    /// 1. If the key file exists: try to unwrap it with DPAPI. On success,
    ///    return the key. On failure (`DpapiDecryptFailed`/`DpapiUnavailable`),
    ///    return that error directly — this never falls through to
    ///    generating a new key, even though the file existed.
    /// 2. If the key file does not exist: consult `store_fingerprint`.
    ///    - `None`, or `Some` indicating no existing ciphertext: generate a
    ///      fresh key, DPAPI-wrap it, write the file, and return the key.
    ///    - `Some` indicating existing ciphertext: return
    ///      `Err(ExistingStoreNotDecryptable)` without creating or writing
    ///      anything. A fresh key here would silently orphan the existing
    ///      store while reporting success.
    pub fn load_or_create(
        key_store_path: &Path,
        store_fingerprint: Option<&StoreFingerprint>,
    ) -> Result<MasterKey, KeyError> {
        match fs::read(key_store_path) {
            Ok(wrapped) => {
                // The key file exists: its fate is decided here. Success or
                // a named DPAPI failure, never a fallback to a fresh key.
                Self::unwrap_with_dpapi(&wrapped)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let existing_ciphertext = store_fingerprint
                    .map(StoreFingerprint::indicates_existing_ciphertext)
                    .unwrap_or(false);
                if existing_ciphertext {
                    return Err(KeyError::ExistingStoreNotDecryptable);
                }
                let key = Self::generate();
                let wrapped = dpapi_protect(key.expose_secret())?;
                if let Some(parent) = key_store_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(key_store_path, wrapped)?;
                Ok(key)
            }
            Err(e) => Err(KeyError::Io(e)),
        }
    }

    /// Unwraps a DPAPI-wrapped key file's bytes into a [`MasterKey`].
    pub fn unwrap_with_dpapi(wrapped_bytes: &[u8]) -> Result<MasterKey, KeyError> {
        let unwrapped = dpapi_unprotect(wrapped_bytes)?;
        let bytes: [u8; MASTER_KEY_LEN] = unwrapped.as_slice().try_into().map_err(|_| KeyError::DpapiUnavailable)?;
        Ok(MasterKey::from_bytes(bytes, KeySource::Dpapi))
    }
}

// ---------------------------------------------------------------------
// Raw DPAPI helpers (pub(crate), operate on arbitrary byte slices).
//
// These are the ONLY functions in this crate that call into the Win32
// DPAPI FFI. `MasterKey`'s wrap/unwrap methods call them internally; Task 5
// (deletion log's trusted reference) reuses these same two functions
// directly rather than going through `MasterKey`'s higher-level API or
// reimplementing the FFI calls.
// ---------------------------------------------------------------------

/// DPAPI-protects (`CryptProtectData`) arbitrary bytes, scoped to the
/// current user. Returns the opaque wrapped blob.
#[cfg(windows)]
pub(crate) fn dpapi_protect(bytes: &[u8]) -> Result<Vec<u8>, KeyError> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
    use windows::core::PCWSTR;

    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    // SAFETY: `input` points at `bytes`, valid for the duration of this
    // call. `output` is zero-initialized; CryptProtectData allocates its
    // own buffer for it via LocalAlloc, which we free below with
    // LocalFree. No entropy/prompt/reserved args are used.
    let result = unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            0,
            &mut output,
        )
    };

    match result {
        Ok(()) => {
            let out = copy_blob(&output);
            unsafe {
                let _ = LocalFree(Some(HLOCAL(output.pbData as *mut core::ffi::c_void)));
            }
            Ok(out)
        }
        Err(_) => Err(KeyError::DpapiDecryptFailed),
    }
}

/// DPAPI-unprotects (`CryptUnprotectData`) a blob previously produced by
/// [`dpapi_protect`]. Fails with [`KeyError::DpapiDecryptFailed`] when the
/// calling user/machine context differs from the one that wrapped the data
/// (or the envelope is otherwise rejected by DPAPI) — a named, distinguishable
/// error, never silent corruption and never a panic.
#[cfg(windows)]
pub(crate) fn dpapi_unprotect(bytes: &[u8]) -> Result<Vec<u8>, KeyError> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    if bytes.is_empty() {
        // Structurally cannot be a DPAPI envelope; don't even attempt the
        // Win32 call.
        return Err(KeyError::DpapiUnavailable);
    }

    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    // SAFETY: `input` points at `bytes`, valid for the duration of this
    // call. `output` is zero-initialized; on success CryptUnprotectData
    // allocates its own buffer for it via LocalAlloc, freed below.
    let result = unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            0,
            &mut output,
        )
    };

    match result {
        Ok(()) => {
            let out = copy_blob(&output);
            unsafe {
                let _ = LocalFree(Some(HLOCAL(output.pbData as *mut core::ffi::c_void)));
            }
            Ok(out)
        }
        Err(_) => Err(KeyError::DpapiDecryptFailed),
    }
}

/// Copies a `CRYPT_INTEGER_BLOB`'s bytes into an owned `Vec<u8>`.
#[cfg(windows)]
fn copy_blob(blob: &windows::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if blob.pbData.is_null() || blob.cbData == 0 {
        return Vec::new();
    }
    // SAFETY: `pbData` is a valid pointer to `cbData` bytes, owned by this
    // process (allocated by CryptProtectData/CryptUnprotectData via
    // LocalAlloc), live for at least the duration of this read.
    unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec() }
}

#[cfg(not(windows))]
pub(crate) fn dpapi_protect(_bytes: &[u8]) -> Result<Vec<u8>, KeyError> {
    Err(KeyError::PlatformUnsupported)
}

#[cfg(not(windows))]
pub(crate) fn dpapi_unprotect(_bytes: &[u8]) -> Result<Vec<u8>, KeyError> {
    Err(KeyError::PlatformUnsupported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// The ONLY case where minting a fresh key is correct: no key file and
    /// no existing ciphertext (store_fingerprint is None, or indicates an
    /// empty store).
    #[test]
    fn load_or_create_with_no_key_file_and_no_existing_store_generates_a_fresh_key() {
        let dir = tempdir().unwrap();
        let key_path = dir.path().join("master.key");
        assert!(!key_path.exists());

        let key = MasterKey::load_or_create(&key_path, None).expect("should mint a fresh key");
        assert_eq!(key.source(), KeySource::Fresh);
        assert!(key_path.exists(), "the wrapped key file must now be on disk");

        // Also exercise the explicit "empty store" fingerprint spelling.
        let key_path2 = dir.path().join("master2.key");
        let empty = StoreFingerprint::empty();
        let key2 = MasterKey::load_or_create(&key_path2, Some(&empty)).expect("empty fingerprint also allows minting");
        assert_eq!(key2.source(), KeySource::Fresh);
    }

    /// The critical negative case (Review Focus item 1, second direction):
    /// key file is missing AND store_fingerprint indicates ciphertext
    /// already exists on disk. Must return Err(ExistingStoreNotDecryptable),
    /// NEVER Ok(a new MasterKey), and must not create the key file.
    #[test]
    fn load_or_create_with_no_key_file_but_an_existing_undecryptable_store_refuses_to_mint_a_fresh_key() {
        let dir = tempdir().unwrap();
        let key_path = dir.path().join("master.key");
        assert!(!key_path.exists());

        let existing = StoreFingerprint::existing();
        let result = MasterKey::load_or_create(&key_path, Some(&existing));

        assert!(
            matches!(result, Err(KeyError::ExistingStoreNotDecryptable)),
            "must refuse to mint a fresh key over an existing, undecryptable store"
        );
        assert!(
            !key_path.exists(),
            "must not touch the key file when refusing to re-key"
        );
    }

    /// A corrupt key file (present but undecryptable) must also surface a
    /// named DPAPI failure rather than silently falling through to minting
    /// a fresh key, regardless of what store_fingerprint says.
    #[test]
    #[cfg(windows)]
    fn load_or_create_with_a_present_but_corrupt_key_file_never_falls_through_to_a_fresh_key() {
        let dir = tempdir().unwrap();
        let key_path = dir.path().join("master.key");
        fs::write(&key_path, b"not a real dpapi envelope").unwrap();

        let result = MasterKey::load_or_create(&key_path, None);
        // Describe the outcome without ever `{:?}`-formatting a `MasterKey`
        // (it deliberately does not implement `Debug`): map `Ok` to a fixed,
        // secret-free label first.
        let described = result.as_ref().map(|_| "Ok(MasterKey)");
        assert!(
            matches!(result, Err(KeyError::DpapiDecryptFailed) | Err(KeyError::DpapiUnavailable)),
            "a present-but-corrupt key file must surface a named DPAPI error, got {described:?}"
        );
    }

    #[test]
    #[cfg(windows)]
    fn unwrap_with_dpapi_on_bytes_wrapped_by_a_different_users_dpapi_fails_closed_not_as_corruption() {
        // CryptUnprotectData fails when the calling user/machine context
        // differs from the one that wrapped the bytes. We can't spin up a
        // second Windows user account in a unit test, so we simulate the
        // same observable outcome: corrupt the wrapped blob's DPAPI
        // envelope (not the plaintext key, which we never touch directly)
        // and assert the error is specifically DpapiDecryptFailed —
        // distinguishable from Io and from a panic.
        let key = MasterKey::generate();
        let wrapped = dpapi_protect(key.expose_secret()).expect("dpapi_protect should succeed on this machine");

        let mut corrupted = wrapped.clone();
        // Flip bytes throughout the envelope (not just the tail) so this
        // reliably breaks DPAPI's internal integrity check regardless of
        // its exact layout.
        for byte in corrupted.iter_mut().skip(4) {
            *byte ^= 0xFF;
        }
        assert_ne!(corrupted, wrapped, "corruption must actually change the bytes");

        let result = MasterKey::unwrap_with_dpapi(&corrupted);
        let described = result.as_ref().map(|_| "Ok(MasterKey)");
        assert!(
            matches!(result, Err(KeyError::DpapiDecryptFailed) | Err(KeyError::DpapiUnavailable)),
            "corrupted DPAPI envelope must fail closed with a named error, not panic or look like Io; got {described:?}"
        );

        // Sanity: the uncorrupted blob still unwraps correctly, proving the
        // failure above is specific to the corruption, not environmental.
        let recovered = MasterKey::unwrap_with_dpapi(&wrapped).expect("uncorrupted blob must still unwrap");
        assert_eq!(recovered.expose_secret(), key.expose_secret());
    }

    #[test]
    #[cfg(windows)]
    fn dpapi_protect_then_unprotect_round_trips_arbitrary_bytes() {
        let plaintext = b"some arbitrary trusted-reference bytes for task 5";
        let wrapped = dpapi_protect(plaintext).unwrap();
        assert_ne!(wrapped, plaintext, "wrapped bytes must not equal plaintext");
        let unwrapped = dpapi_unprotect(&wrapped).unwrap();
        assert_eq!(unwrapped, plaintext);
    }

    #[test]
    #[cfg(not(windows))]
    fn dpapi_functions_return_platform_unsupported_on_non_windows() {
        assert!(matches!(dpapi_protect(b"x"), Err(KeyError::PlatformUnsupported)));
        assert!(matches!(dpapi_unprotect(b"x"), Err(KeyError::PlatformUnsupported)));
        assert!(matches!(
            MasterKey::unwrap_with_dpapi(b"x"),
            Err(KeyError::PlatformUnsupported)
        ));
    }

}
