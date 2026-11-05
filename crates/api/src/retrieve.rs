use crate::{check, response, ApiError, ApiResponse, RequestContext};
use domain::NoteStatus;
use index::search::{Provenance, ReferenceState};
use storage::{NoteId, StorageError, Vault, Version};

#[derive(Debug, Clone)]
pub enum RetrieveTarget {
    Note(NoteId),
    Chunk(String),
}
#[derive(Debug, Clone)]
pub struct RetrieveRequest {
    pub id: RetrieveTarget,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RetrieveResponse {
    pub id: NoteId,
    pub content: String,
    pub version: Version,
    pub provenance: Provenance,
    pub status: NoteStatus,
    pub references: ReferenceState,
    pub metadata_schema_version: u32,
}

pub fn retrieve(
    vault: &Vault,
    policy: &policy::ReadPolicy,
    context: RequestContext,
    request: RetrieveRequest,
) -> ApiResponse<RetrieveResponse> {
    let result = check(context, policy, policy::ReadOperation::Retrieve).and_then(|()| {
        let id = match request.id {
            RetrieveTarget::Note(id) => id,
            RetrieveTarget::Chunk(_) => return Err(ApiError::UnsupportedChunk),
        };
        let record = vault.read_note(&id).map_err(|error| match error {
            StorageError::NotFound { .. } => ApiError::NotFound,
            StorageError::Deleted { .. } | StorageError::NoteDeleted { .. } => ApiError::Deleted,
            StorageError::Inconsistent { .. } => ApiError::Inconsistent,
            _ => ApiError::ReadUnavailable,
        })?;
        let (metadata, _) =
            storage::split_record(record.content.as_bytes()).map_err(|_| ApiError::Inconsistent)?;
        if metadata.id != id.as_str() {
            return Err(ApiError::Inconsistent);
        }
        let relations = &metadata.relations;
        let references = if relations.supports.is_empty()
            && relations.contradicts.is_empty()
            && relations.derived_from.is_empty()
            && relations.supersedes.is_empty()
            && relations.applies_to.is_empty()
        {
            ReferenceState::Unlinked
        } else {
            ReferenceState::NotRevalidated
        };
        Ok(RetrieveResponse {
            id,
            content: record.content,
            version: record.version,
            provenance: Provenance {
                project: metadata.project,
                trust: metadata.trust,
                classification: metadata.classification,
                source: Some(metadata.source),
                repository_id: None,
                commit: None,
            },
            status: metadata.status,
            references,
            metadata_schema_version: metadata.schema_version,
        })
    });
    response(context, result)
}
