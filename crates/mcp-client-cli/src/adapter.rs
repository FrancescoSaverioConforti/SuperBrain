//! M6.4 Phase 1's narrow adapter connecting the real
//! `storage::AuditWriterHandle` to `mcp_client_audit_worker::
//! DurableAuditWriter` -- confirmed, by directly re-reading both error
//! enums side by side, to be a lossless, 1:1 mapping that introduces no
//! new failure mode and collapses no distinct case into another:
//! `storage::AuditAppendError::{Poisoned, Io, ReferenceUpdateFailed}`
//! map exactly onto `mcp_client_audit_worker::WriterAppendError`'s own
//! three variants, with the exact same poisoning semantics preserved
//! (`Poisoned`/`Io` poison the underlying `storage::AuditWriterHandle`
//! going forward; `ReferenceUpdateFailed` does not).
//!
//! This duplicates no filesystem security logic -- `storage::
//! AuditWriterHandle::append`'s own write_all -> sync_all ->
//! reference-update ordering is called completely unchanged.

pub struct StorageAuditWriterAdapter(pub storage::AuditWriterHandle);

impl mcp_client_audit_worker::DurableAuditWriter for StorageAuditWriterAdapter {
    fn append_owned(self, bytes: Vec<u8>, sequence: u64, record_hash: String) -> (Self, Result<(), mcp_client_audit_worker::WriterAppendError>) {
        let mut inner = self.0;
        let result = inner.append(&bytes, sequence, &record_hash).map_err(|e| match e {
            storage::AuditAppendError::Poisoned => mcp_client_audit_worker::WriterAppendError::Poisoned,
            storage::AuditAppendError::Io(io_err) => mcp_client_audit_worker::WriterAppendError::Io(io_err.to_string()),
            storage::AuditAppendError::ReferenceUpdateFailed(msg) => mcp_client_audit_worker::WriterAppendError::ReferenceUpdateFailed(msg),
        });
        (StorageAuditWriterAdapter(inner), result)
    }
}
