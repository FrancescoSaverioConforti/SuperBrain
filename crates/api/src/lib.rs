//! B1 callable core: local single-user read-only functions, no transport.
//!
//! Every call checks the host-supplied read policy. This is not authentication;
//! the trusted host supplies capabilities, never note text or model output.
//! Requests and responses carry schema version and opaque correlation UUID,
//! including failures. No request content or underlying error is logged.
//!
//! Calls are synchronous: no timeout, retry scheduler, size/rate cap or remote
//! identity exists yet. k must be positive; it caps returned search results,
//! not work performed. Retries are read-only and can see newer committed data;
//! no repeatable-read/cross-note snapshot or response cache is claimed.
//!
//! Search score semantics: BM25 over freshly read matching/filter-eligible
//! candidates for this call. Exact-only substrings can score zero. Compare
//! scores within a response, not across queries. Search preserves index's
//! context/temporal annotations, NotPresent and incompleteness signal.
//!
//! Only note ids are supported; chunk ids are explicitly rejected until B2.
//! Repository/commit provenance and persisted Pending/Resolved states remain
//! unavailable as documented in B1 Task 6. No backing data is fabricated.
pub mod metrics;
pub mod retrieve;
pub mod search;

pub use retrieve::{retrieve, RetrieveRequest, RetrieveResponse, RetrieveTarget};
pub use search::{search, SearchRequest};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestContext {
    pub schema_version: u32,
    pub correlation_id: uuid::Uuid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiResponse<T> {
    pub schema_version: u32,
    pub correlation_id: uuid::Uuid,
    pub result: Result<T, ApiError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiError {
    UnsupportedSchema,
    PermissionDenied,
    UnsupportedChunk,
    NotFound,
    Deleted,
    Inconsistent,
    ReadUnavailable,
    Search(index::search::SearchError),
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnsupportedSchema => "unsupported API schema version",
            Self::PermissionDenied => "read permission denied",
            Self::UnsupportedChunk => "chunk retrieval is not implemented in B1",
            Self::NotFound => "note not found",
            Self::Deleted => "note deleted",
            Self::Inconsistent => "note integrity could not be verified",
            Self::ReadUnavailable => "vault read unavailable",
            Self::Search(_) => "search failed",
        })
    }
}
impl std::error::Error for ApiError {}

fn check(
    context: RequestContext,
    policy: &policy::ReadPolicy,
    op: policy::ReadOperation,
) -> Result<(), ApiError> {
    if context.schema_version != SCHEMA_VERSION {
        return Err(ApiError::UnsupportedSchema);
    }
    if !policy.allows(op) {
        return Err(ApiError::PermissionDenied);
    }
    Ok(())
}
fn response<T>(context: RequestContext, result: Result<T, ApiError>) -> ApiResponse<T> {
    ApiResponse {
        schema_version: SCHEMA_VERSION,
        correlation_id: context.correlation_id,
        result,
    }
}
