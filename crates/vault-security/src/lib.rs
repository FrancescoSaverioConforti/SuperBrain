//! Encrypted PII store, deletion log, and backup for `sensitive`-classified
//! data only.

pub mod availability;
pub mod deletion_log;
pub mod keys;
pub mod recovery_kit;

pub use availability::{EncryptedStoreAvailability, StoreUnavailableReason};
pub use deletion_log::{
    DeletionLog, DeletionLogError, DeletionLogState, DeletionReceipt, DpapiTrustedReferenceStore, GenerationRef,
    ReconcileOutcome, TrustedReferenceStore,
};
pub use keys::{KeyError, KeySource, MasterKey, StoreFingerprint};
pub use recovery_kit::{unwrap_master_key, RecoveryKit, RecoveryKitError};
