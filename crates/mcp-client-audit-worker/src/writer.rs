//! M6.3 Phase 2: the narrow injected writer interface this crate's
//! worker depends on, instead of depending on `storage` directly.
//!
//! **Why an injected interface, not a direct `storage::Vault` dependency
//! (unlike `brain-api-audit-worker`, which DOES depend on `storage`
//! directly)**: this crate's own Phase 2 authorization is explicit —
//! `mcp-client-audit-worker` must not depend on `storage` at all. The
//! durable writer itself remains owned by a FUTURE composition-root
//! binary (not part of this task), which constructs a real
//! `storage::Vault::open_audit_writer` handle (Option C, already
//! approved at the M6.1 security gate) and adapts it to
//! [`DurableAuditWriter`] below. This module defines ONLY the contract;
//! it contains no filesystem code of its own and introduces no
//! independent filesystem writer (an explicit scope prohibition).
//!
//! **Why this shape, and not a simpler `&mut self` trait method**: the
//! fork research into `storage::AuditWriterHandle::append`'s own real
//! shape confirmed the exact mechanism `brain-api-audit-worker` depends
//! on for cancellation safety: `append` is a plain, synchronous,
//! `&mut self` function (never `.await`s internally), and the ONLY way
//! to call it from an async worker loop without blocking the runtime
//! thread is to MOVE the owning handle into `spawn_blocking`'s `'static`
//! closure and move it back out via the closure's return value — never
//! borrowed across an await point, never behind a shared mutex. A prior
//! per-call-mutex-across-`spawn_blocking` design was proven unsafe by
//! direct reproduction elsewhere in this codebase (duplicate sequence
//! numbers under cancellation) and must not be reintroduced here.
//!
//! [`DurableAuditWriter::append_owned`] encodes exactly that ownership-
//! move contract at the trait boundary: it consumes `self` by value and
//! returns `self` back by value alongside the result, so ANY
//! implementation (a real `storage::AuditWriterHandle` adapter, or a
//! test fake) is forced into the same move-in/move-out shape this
//! worker's own `spawn_blocking` call requires — the trait cannot be
//! implemented in a way that smuggles interior mutability or shared
//! state back in, because there is no `&mut self` method to ever call
//! from outside an owning move.
//!
//! If a future implementation genuinely cannot preserve correct
//! append/fsync/reference-ordering through this exact shape without
//! duplicating `storage`'s own internals, that is this task's own
//! explicit STOP condition — report it rather than silently relaxing
//! this contract.

/// Why one `append_owned` call did not complete the full
/// append+fsync+reference-update contract. Mirrors
/// `brain_api_audit_worker::DurableRecordError`'s own real-failure
/// subset (the queue/worker-liveness variants live in this crate's own
/// [`crate::error::DurableRecordError`] instead, one layer up, since
/// those are facts about THIS crate's worker, not about the injected
/// writer itself).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriterAppendError {
    /// The underlying writer is poisoned by an earlier ambiguous
    /// append/fsync failure and refuses every further write. Mirrors
    /// `storage::AuditAppendError::Poisoned`'s own meaning exactly (an
    /// adapter over a real `storage::AuditWriterHandle` is expected to
    /// surface that variant as this one, unchanged).
    #[error("audit writer is poisoned after an earlier ambiguous I/O failure")]
    Poisoned,
    /// The log bytes were durably appended and fsync'd, but the
    /// subsequent head-reference update failed — carried through
    /// unchanged from `storage::AuditAppendError::ReferenceUpdateFailed`.
    /// NOT poisoning: the log itself is self-consistent and a future
    /// reopen's own roll-forward recovery can resume past this.
    #[error("audit log record was durably appended, but the head reference update failed: {0}")]
    ReferenceUpdateFailed(String),
    /// A plain I/O failure during append/fsync — the ambiguous case
    /// that DOES poison the underlying writer going forward (mirrors
    /// `storage::AuditAppendError::Io`'s own documented poisoning
    /// behavior).
    #[error("audit log append I/O error: {0}")]
    Io(String),
}

/// The narrow contract a future composition-root binary's real
/// `storage::Vault::open_audit_writer`-backed adapter must satisfy —
/// and the ONLY contract this crate's worker depends on. See this
/// module's own top doc comment for why `append_owned` consumes and
/// returns `Self` by value rather than taking `&mut self`.
///
/// `Send + 'static` is required so a value of this type can be moved
/// into `tokio::task::spawn_blocking`'s own `'static` closure bound —
/// the exact mechanism this worker's cancellation-safety guarantee
/// depends on (see [`crate::worker`]'s own doc comment).
pub trait DurableAuditWriter: Send + 'static {
    /// Appends one already-fully-serialized record's `bytes` (including
    /// any trailing newline the caller's own record format requires),
    /// at `sequence`, whose hash is `record_hash` — this trait is
    /// opaque to what `bytes` means or how `record_hash` was computed
    /// (the same "caller supplies fully-serialized bytes; this contract
    /// computes nothing itself" discipline `storage::AuditWriterHandle::
    /// append` already establishes). Returns `self` back, moved,
    /// alongside the outcome — never borrowed, so a caller can move the
    /// returned `self` into the next `spawn_blocking` call in turn.
    ///
    /// Must preserve the exact write_all → sync_all → reference-update
    /// ordering this crate's own durability guarantee depends on: `Ok`
    /// must never be returned before the record is both written AND
    /// fsync'd (an implementation adapting a real `storage::
    /// AuditWriterHandle` satisfies this automatically, since that
    /// type's own `append` already enforces this ordering internally).
    fn append_owned(self, bytes: Vec<u8>, sequence: u64, record_hash: String) -> (Self, Result<(), WriterAppendError>)
    where
        Self: Sized;
}
