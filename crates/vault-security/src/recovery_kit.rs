//! Recovery kit: an offline, passphrase-protected, DPAPI-independent way to
//! recover the vault's live [`MasterKey`].
//!
//! [`RecoveryKit::generate`] takes the *live* `MasterKey` by reference and
//! wraps that exact instance's bytes — not a key it generates itself. That
//! is what makes a cross-machine restore (via the printed recovery kit
//! text, no DPAPI involved, e.g. after a lost/reimaged Windows profile)
//! actually recover the key protecting the real, already-written ciphertext
//! store, rather than a fresh key that doesn't open anything.
//!
//! # Argon2id parameters
//!
//! The passphrase is run through `argon2::Argon2` with the Argon2id
//! variant and:
//! - memory cost: **65536 KiB (64 MiB)**
//! - iterations (time cost): **3**
//! - parallelism: **1**
//! - output length: **32 bytes** (an AES-256-GCM key)
//!
//! These are the OWASP-recommended floor for Argon2id (64 MiB / t=3 / p=1)
//! and are deliberately chosen to be slow — on the order of 0.5-1s on the
//! target laptop-class hardware — to make offline brute-forcing of a lost
//! recovery-kit passphrase expensive, while staying fast enough that a
//! human doing an actual recovery does not notice a meaningful delay.
//! Parallelism is pinned at 1 (not left to the host's core count) so that
//! a kit generated on one machine derives the identical key on any other.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use thiserror::Error;
use zeroize::Zeroizing;

use crate::keys::{KeySource, MasterKey, MASTER_KEY_LEN};

/// Argon2id memory cost in KiB (64 MiB).
const ARGON2_MEMORY_KIB: u32 = 65536;
/// Argon2id time cost (iterations).
const ARGON2_ITERATIONS: u32 = 3;
/// Argon2id parallelism (pinned to 1 for cross-machine reproducibility).
const ARGON2_PARALLELISM: u32 = 1;
/// Length in bytes of the random Argon2id salt stored in the kit.
const SALT_LEN: usize = 16;
/// Length in bytes of the AEAD nonce stored in the kit.
const NONCE_LEN: usize = 12;

/// A generated recovery kit: the master key's bytes, AEAD-wrapped under an
/// Argon2id-derived key, plus the salt and AEAD nonce needed to redo the
/// derivation and unwrap. Does not contain the passphrase.
pub struct RecoveryKit {
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
    /// Ciphertext + AEAD tag (aes-gcm appends the tag to the ciphertext).
    wrapped: Vec<u8>,
}

/// Errors from recovery-kit generation, parsing, or unwrap.
#[derive(Debug, Error)]
pub enum RecoveryKitError {
    #[error("recovery kit text is not validly encoded: {0}")]
    Malformed(&'static str),

    #[error("recovery kit text is the wrong length to contain a valid kit")]
    InvalidLength,

    #[error("Argon2id key derivation failed: {0}")]
    Kdf(String),

    /// The passphrase was wrong, or the kit text was tampered with: AEAD
    /// authentication failed. Fails closed — never returns a different,
    /// wrong key.
    #[error("recovery kit could not be unwrapped: wrong passphrase or corrupted kit")]
    UnwrapFailed,
}

fn argon2id_params() -> Params {
    Params::new(ARGON2_MEMORY_KIB, ARGON2_ITERATIONS, ARGON2_PARALLELISM, Some(MASTER_KEY_LEN))
        .expect("fixed Argon2id params are valid")
}

/// Derives a 32-byte key from `passphrase` and `salt` using the Argon2id
/// parameters documented on this module.
fn derive_key(passphrase: &str, salt: &[u8]) -> Result<Zeroizing<[u8; MASTER_KEY_LEN]>, RecoveryKitError> {
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon2id_params());
    let mut out = Zeroizing::new([0u8; MASTER_KEY_LEN]);
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, out.as_mut())
        .map_err(|e| RecoveryKitError::Kdf(e.to_string()))?;
    Ok(out)
}

impl RecoveryKit {
    /// Generates a recovery kit that independently re-wraps `master_key`'s
    /// actual bytes under a passphrase-derived key. Takes `master_key` by
    /// reference specifically so this protects the *same live instance*
    /// (e.g. the one `MasterKey::load_or_create` just produced), not a
    /// freshly generated standalone key.
    pub fn generate(passphrase: &str, master_key: &MasterKey) -> Result<RecoveryKit, RecoveryKitError> {
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::Aes256Gcm;

        let mut salt = [0u8; SALT_LEN];
        getrandom::fill(&mut salt).expect("the OS CSPRNG must be available to generate a recovery-kit salt");
        let mut nonce_bytes = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce_bytes).expect("the OS CSPRNG must be available to generate a recovery-kit nonce");

        let derived = derive_key(passphrase, &salt)?;
        let cipher = Aes256Gcm::new_from_slice(derived.as_ref()).expect("32-byte key is valid for AES-256-GCM");

        let wrapped = cipher
            .encrypt((&nonce_bytes).into(), master_key.expose_secret().as_slice())
            .map_err(|_| RecoveryKitError::UnwrapFailed)?;

        Ok(RecoveryKit { salt, nonce: nonce_bytes, wrapped })
    }

    /// Encodes this kit as one copyable, base64-encoded string: this is
    /// the artifact meant for offline/printed storage (spec contract C5 —
    /// a recovery reference independent of backups). Layout before
    /// base64: `[salt (16B)][nonce (12B)][wrapped master key + AEAD tag (48B)]`.
    pub fn to_printable_string(&self) -> String {
        let mut buf = Vec::with_capacity(SALT_LEN + NONCE_LEN + self.wrapped.len());
        buf.extend_from_slice(&self.salt);
        buf.extend_from_slice(&self.nonce);
        buf.extend_from_slice(&self.wrapped);
        base64::engine::general_purpose::STANDARD.encode(buf)
    }

    /// Parses a string produced by [`RecoveryKit::to_printable_string`].
    pub fn from_printable_string(kit_text: &str) -> Result<RecoveryKit, RecoveryKitError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(kit_text.trim())
            .map_err(|_| RecoveryKitError::Malformed("not valid base64"))?;

        if bytes.len() < SALT_LEN + NONCE_LEN + 1 {
            return Err(RecoveryKitError::InvalidLength);
        }

        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&bytes[..SALT_LEN]);
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&bytes[SALT_LEN..SALT_LEN + NONCE_LEN]);
        let wrapped = bytes[SALT_LEN + NONCE_LEN..].to_vec();

        Ok(RecoveryKit { salt, nonce, wrapped })
    }

    /// Re-derives the Argon2id key from `passphrase` and this kit's salt,
    /// then AEAD-unwraps the embedded master-key bytes. Does not call any
    /// DPAPI function — this is the independent, portable recovery path.
    /// A wrong passphrase fails closed with `UnwrapFailed`, never a
    /// different, wrong key.
    fn unwrap(&self, passphrase: &str) -> Result<MasterKey, RecoveryKitError> {
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::Aes256Gcm;

        let derived = derive_key(passphrase, &self.salt)?;
        let cipher = Aes256Gcm::new_from_slice(derived.as_ref()).expect("32-byte key is valid for AES-256-GCM");

        let plaintext = cipher
            .decrypt((&self.nonce).into(), self.wrapped.as_slice())
            .map_err(|_| RecoveryKitError::UnwrapFailed)?;

        let bytes: [u8; MASTER_KEY_LEN] =
            plaintext.as_slice().try_into().map_err(|_| RecoveryKitError::UnwrapFailed)?;

        Ok(MasterKey::from_bytes(bytes, KeySource::RecoveryKit))
    }
}

/// Parses `kit_text` and unwraps it with `passphrase` in one step. Does not
/// call any DPAPI function. A wrong passphrase, or tampered `kit_text`,
/// fails closed with `RecoveryKitError::UnwrapFailed`.
pub fn unwrap_master_key(kit_text: &str, passphrase: &str) -> Result<MasterKey, RecoveryKitError> {
    let kit = RecoveryKit::from_printable_string(kit_text)?;
    kit.unwrap(passphrase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::MasterKey;

    /// generate() against a REAL MasterKey (standing in for the one
    /// load_or_create produced); then, WITHOUT calling any DPAPI function
    /// (this is the cross-machine proof: DPAPI is tied to the user/machine,
    /// so this test must prove the recovery path alone, with no DPAPI call
    /// anywhere in it, reconstructs byte-identical key material from
    /// kit_text + passphrase).
    #[test]
    fn recovery_kit_generated_from_a_live_master_key_recovers_that_exact_key_independent_of_dpapi() {
        let live_key = MasterKey::generate();
        let live_bytes = *live_key.expose_secret();

        let kit = RecoveryKit::generate("correct horse battery staple", &live_key).expect("generate should succeed");
        let kit_text = kit.to_printable_string();

        // Round-trip through the printable string, as a real offline
        // recovery would: parse text -> unwrap with passphrase. No DPAPI
        // function is called anywhere in this path.
        let recovered = unwrap_master_key(&kit_text, "correct horse battery staple")
            .expect("correct passphrase must recover the exact live key");

        assert_eq!(
            recovered.expose_secret(),
            &live_bytes,
            "recovery kit must reconstruct the SAME live MasterKey's bytes, not an independently generated key"
        );
    }

    #[test]
    fn recovery_kit_unwrap_with_the_wrong_passphrase_fails_closed() {
        let live_key = MasterKey::generate();
        let kit = RecoveryKit::generate("the right passphrase", &live_key).unwrap();
        let kit_text = kit.to_printable_string();

        let result = unwrap_master_key(&kit_text, "a very wrong passphrase");
        assert!(
            matches!(result, Err(RecoveryKitError::UnwrapFailed)),
            "wrong passphrase must fail closed with UnwrapFailed, not return Ok with different bytes"
        );
    }

    #[test]
    fn recovery_kit_two_different_master_keys_produce_different_recoverable_bytes() {
        let key_a = MasterKey::generate();
        let key_b = MasterKey::generate();
        assert_ne!(key_a.expose_secret(), key_b.expose_secret(), "sanity: two generated keys differ");

        let kit_a = RecoveryKit::generate("shared passphrase", &key_a).unwrap();
        let kit_b = RecoveryKit::generate("shared passphrase", &key_b).unwrap();

        let recovered_a = unwrap_master_key(&kit_a.to_printable_string(), "shared passphrase").unwrap();
        let recovered_b = unwrap_master_key(&kit_b.to_printable_string(), "shared passphrase").unwrap();

        assert_eq!(recovered_a.expose_secret(), key_a.expose_secret());
        assert_eq!(recovered_b.expose_secret(), key_b.expose_secret());
        assert_ne!(recovered_a.expose_secret(), recovered_b.expose_secret());
    }

    #[test]
    fn from_printable_string_rejects_garbage_input() {
        assert!(matches!(
            RecoveryKit::from_printable_string("not base64 at all!!! ***"),
            Err(RecoveryKitError::Malformed(_))
        ));
        assert!(matches!(
            RecoveryKit::from_printable_string(&base64::engine::general_purpose::STANDARD.encode(b"short")),
            Err(RecoveryKitError::InvalidLength)
        ));
    }
}
