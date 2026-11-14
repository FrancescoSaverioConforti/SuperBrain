//! B2 Task 5: the Raw -> Validated promotion boundary. The ONLY path
//! from a Task 4 [`crate::extract::RawSymbolCandidate`] plus validated
//! identity (`FileIdentity`, which already binds a `RepositoryId` and
//! `CommitHash`) to a validated [`SymbolSnapshot`] is [`promote`]. No
//! lineage, no typed edges, no graph storage, no persistence, no API —
//! those are later tasks.

mod kind_mapping;
mod name;
mod signature;

pub use kind_mapping::{map_kind_hint, UnrecognizedKindHint};
pub use name::{ValidatedName, ValidatedNameError};
pub use signature::{BoundedSignature, BoundedSignatureError};

use codegraph_domain::{FileIdentity, SymbolKind, SymbolSnapshotId};
use thiserror::Error;

use crate::extract::{ByteSpan, RawSymbolCandidate};

/// A validated symbol, bound to the exact commit/file it was observed
/// at. The only way to construct one is [`promote`] — there is no public
/// field, no `pub` struct literal construction available outside this
/// module, and no constructor that accepts a raw, unvalidated `&str` for
/// `name`/`signature`/`kind_hint`. External callers (including a future
/// Task 6 lineage resolver) can only ever hold a `SymbolSnapshot`
/// produced by a successful `promote` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolSnapshot {
    id: SymbolSnapshotId,
    kind: SymbolKind,
    name: ValidatedName,
    signature: BoundedSignature,
    file: FileIdentity,
    span: ByteSpan,
}

impl SymbolSnapshot {
    pub fn id(&self) -> SymbolSnapshotId {
        self.id
    }

    pub fn kind(&self) -> SymbolKind {
        self.kind
    }

    pub fn name(&self) -> &ValidatedName {
        &self.name
    }

    pub fn signature(&self) -> &BoundedSignature {
        &self.signature
    }

    pub fn file(&self) -> &FileIdentity {
        &self.file
    }

    /// The symbol's source span: zero-based, end-exclusive byte offsets
    /// into `self.file`'s content at `self.file.commit()` — Task 4's own
    /// [`ByteSpan`] convention, preserved as-is and never implicitly
    /// converted to a line/column representation at this layer (see
    /// `extract::candidate::ByteSpan`'s doc comment for the full
    /// rationale; a future presentation layer that needs line/column may
    /// derive it explicitly from this span plus the same source bytes,
    /// but that conversion does not belong here).
    pub fn span(&self) -> ByteSpan {
        self.span
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PromotionError {
    #[error(transparent)]
    UnrecognizedKind(#[from] UnrecognizedKindHint),
    #[error(transparent)]
    InvalidName(#[from] ValidatedNameError),
    #[error(transparent)]
    InvalidSignature(#[from] BoundedSignatureError),
    #[error("symbol span is invalid: start ({start}) must be less than end ({end})")]
    InvalidSpan { start: usize, end: usize },
}

/// The one authoritative Raw -> Validated choke point. Takes an already-
/// extracted `candidate` (Task 4's raw DTO) plus the validated `file`
/// identity it was found in, and either returns a fully validated
/// `SymbolSnapshot` or a specific, typed rejection reason — reject,
/// never truncate or default, on every validation failure:
///
/// - an unrecognized `kind_hint` ([`map_kind_hint`]'s own rejection);
/// - an empty or oversized `name` ([`ValidatedName::parse`]'s own
///   rejection — this re-validates independently of whatever bound
///   `RawSymbolCandidate::new` already applied, since `promote` must not
///   assume every `RawSymbolCandidate` it ever receives necessarily came
///   through that constructor);
/// - an oversized `signature_text` ([`BoundedSignature::parse`]'s own
///   rejection, same reasoning);
/// - an invalid span (`start >= end`, which [`ByteSpan`] itself does not
///   prevent at construction — `promote` is this pipeline's chosen place
///   to reject that shape before it reaches a `SymbolSnapshot`).
pub fn promote(candidate: &RawSymbolCandidate, file: FileIdentity) -> Result<SymbolSnapshot, PromotionError> {
    let kind = map_kind_hint(&candidate.kind_hint)?;
    let name = ValidatedName::parse(&candidate.name)?;
    let signature = BoundedSignature::parse(&candidate.signature_text)?;

    if candidate.span.start >= candidate.span.end {
        return Err(PromotionError::InvalidSpan {
            start: candidate.span.start,
            end: candidate.span.end,
        });
    }

    let id = SymbolSnapshotId::derive(&file, kind, name.as_str(), signature.as_str(), candidate.span.start as u64, candidate.span.end as u64);

    Ok(SymbolSnapshot {
        id,
        kind,
        name,
        signature,
        file,
        span: candidate.span,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_domain::{CommitHash, RepositoryRegistry, RepositoryRoot};

    fn test_file() -> FileIdentity {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        FileIdentity::parse(repo, commit, "src/lib.rs").unwrap()
    }

    fn valid_candidate() -> RawSymbolCandidate {
        RawSymbolCandidate::new(
            "foo".to_string(),
            "function_item".to_string(),
            ByteSpan { start: 0, end: 10 },
            "fn foo()".to_string(),
        )
        .unwrap()
    }

    #[test]
    fn a_valid_candidate_promotes_successfully() {
        let snapshot = promote(&valid_candidate(), test_file()).unwrap();
        assert_eq!(snapshot.kind(), SymbolKind::Function);
        assert_eq!(snapshot.name().as_str(), "foo");
        assert_eq!(snapshot.signature().as_str(), "fn foo()");
        assert_eq!(snapshot.span(), ByteSpan { start: 0, end: 10 });
    }

    #[test]
    fn an_unrecognized_kind_hint_is_rejected() {
        let candidate = RawSymbolCandidate::new(
            "foo".to_string(),
            "totally_unknown_node_kind".to_string(),
            ByteSpan { start: 0, end: 10 },
            "fn foo()".to_string(),
        )
        .unwrap();
        let result = promote(&candidate, test_file());
        assert!(matches!(result, Err(PromotionError::UnrecognizedKind(_))));
    }

    #[test]
    fn an_invalid_span_where_start_equals_end_is_rejected() {
        let candidate = RawSymbolCandidate::new(
            "foo".to_string(),
            "function_item".to_string(),
            ByteSpan { start: 5, end: 5 },
            "fn foo()".to_string(),
        )
        .unwrap();
        let result = promote(&candidate, test_file());
        assert_eq!(result, Err(PromotionError::InvalidSpan { start: 5, end: 5 }));
    }

    #[test]
    fn an_invalid_span_where_start_exceeds_end_is_rejected() {
        let candidate = RawSymbolCandidate::new(
            "foo".to_string(),
            "function_item".to_string(),
            ByteSpan { start: 10, end: 5 },
            "fn foo()".to_string(),
        )
        .unwrap();
        let result = promote(&candidate, test_file());
        assert_eq!(result, Err(PromotionError::InvalidSpan { start: 10, end: 5 }));
    }

    #[test]
    fn promoting_the_same_candidate_twice_yields_the_same_snapshot_id() {
        // Reuses the SAME FileIdentity for both calls: `test_file()`
        // itself mints a fresh RepositoryId per call (by design — see
        // RepositoryRegistry's own doc comment), so calling it twice
        // would correctly produce two DIFFERENT ids here, which is not
        // what this test is checking.
        let file = test_file();
        let a = promote(&valid_candidate(), file.clone()).unwrap();
        let b = promote(&valid_candidate(), file).unwrap();
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn promoting_candidates_with_different_names_yields_different_snapshot_ids() {
        let file = test_file();
        let a = promote(&valid_candidate(), file.clone()).unwrap();
        let other = RawSymbolCandidate::new(
            "bar".to_string(),
            "function_item".to_string(),
            ByteSpan { start: 0, end: 10 },
            "fn foo()".to_string(),
        )
        .unwrap();
        let b = promote(&other, file).unwrap();
        assert_ne!(a.id(), b.id());
    }
}
