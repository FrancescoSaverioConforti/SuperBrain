//! Plain, zero-I/O data types describing whether an encrypted store is
//! available for a given call. These carry no behavior and perform no I/O
//! or cryptography themselves — the caller (eventually `storage`, Task 6)
//! is responsible for actually trying to open the store elsewhere and then
//! constructing one of these variants to describe the outcome.
//!
//! This type exists so that `policy` can make `Gate::evaluate`'s rejection
//! of `Sensitive` notes conditional on store availability without `policy`
//! taking on any crypto or I/O dependency: `policy` depends on this plain
//! enum only, never on `vault-security`'s key-wrapping or crypto error
//! types directly.

/// Whether an encrypted store is available for the current operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptedStoreAvailability {
    /// The encrypted store was successfully opened and is ready to accept
    /// `Sensitive`-classified content.
    Available,
    /// The encrypted store could not be used for this call. The gate relies
    /// on this to keep rejecting `Sensitive` notes exactly as it always has
    /// when no store exists or none could be reached.
    Unavailable { reason: StoreUnavailableReason },
}

/// Why the encrypted store is unavailable. Intentionally its own small enum
/// rather than a re-export of `KeyError`/`PiiStoreError`: it mirrors their
/// shapes loosely but keeps `policy` from depending on `vault-security`'s
/// crypto error types directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreUnavailableReason {
    /// No store has been set up yet (the A1/A2-era default: no encrypted
    /// store exists at all).
    NotYetConfigured,
    /// A store is configured but its master key could not be reached
    /// (e.g. DPAPI unwrap failed, recovery kit not supplied).
    KeyUnreachable,
    /// A store is configured and its key is reachable, but the store's
    /// on-disk state is inconsistent (e.g. fingerprint mismatch).
    StoreInconsistent,
}
