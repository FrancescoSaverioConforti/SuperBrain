//! B1 Task 2: `Document` — the shared shape that sits between a parsed note
//! and the search structures (`TrigramIndex` here, `Bm25Index` in Task 3).
//!
//! `Document` deliberately holds two kinds of data, kept apart on purpose
//! (per the Task 2 brief's corrected allowlist):
//!
//! - **Structured metadata fields** — `id`, `r#type`, `status`, `trust`,
//!   `project`, `created`, `updated`, `event_period`, `schema_version`,
//!   `classification` — kept as their real typed values (not strings, where
//!   `domain::NoteMetadata` has a real enum/typed value). These are for exact
//!   filter matching, sorting and display (`search`'s `type_filter` /
//!   `status_filter` / `trust_filter` / `date_filter` / `ProjectScope`,
//!   Task 6), never for term search. Every one of the ten is present
//!   regardless of classification — none of them carries free-text
//!   confidentiality risk.
//! - **`text_searchable`** — a single `String`, separately computed, that is
//!   the ONLY thing `TrigramIndex` (this task) or `Bm25Index` (Task 3) ever
//!   tokenizes. Its content depends on classification:
//!   - `Internal` / `Public`: body + every structured field's own string
//!     form + `source` + every `relations.*` target.
//!   - `Sensitive`: ONLY the string forms of the ten structured fields above
//!     — never body, never `source` (unvetted free text, unlike the other
//!     nine closed-shape fields), never any `relations.*` target.
//!
//! # Crate-boundary decision: how `Document` receives the body
//!
//! A `storage::VaultRecord`'s `content` is the raw record text (frontmatter
//! delimiters included). Splitting it into frontmatter/body was originally
//! implemented only as `storage::propose::body_of` / `frontmatter_of`, both
//! `pub(crate)` to `storage` and therefore unreachable from `index` (a
//! different crate). Rather than have `index` (or Task 3's real indexer
//! wiring, which receives raw `record_bytes: &[u8]` straight from
//! `storage::Indexer::mark_current` — see `indexer_impl.rs`'s doc comment)
//! reimplement that delimiter-finding logic — which would have to stay
//! byte-for-byte in sync with `storage`'s own notion of where a body starts,
//! including CRLF and YAML block-scalar-dash edge cases, with no
//! compiler-enforced guarantee of doing so — `storage` now exposes a small,
//! additive public function doing the same split: `storage::split_record`.
//! It takes raw record bytes and returns `(domain::NoteMetadata,
//! Option<String>)` by calling the EXISTING `frontmatter_of` / `body_of`
//! helpers and `domain::parse_frontmatter` internally, so this crate never
//! duplicates that logic.
//!
//! `Document::new` still takes an already-split `(metadata, body)` pair —
//! the lower-level constructor, useful when a caller already has both
//! pieces (e.g. from its own parsing, or in tests). `Document::from_record_bytes`
//! is the convenience constructor that goes straight from raw bytes (calling
//! `storage::split_record` first) to a `Document`, demonstrating that `index`
//! can do this without reimplementing any frontmatter-splitting logic
//! itself. `body: None` and `body: Some("")` are treated identically by
//! `Document::new` (nothing to append) — matching `body_of`'s own behavior
//! of returning `""` for a bare-frontmatter note with no body at all, and
//! matching `split_record`'s own `None`-for-empty-body convention.

use domain::{Classification, NoteMetadata, NoteStatus, NoteType, TrustLevel};

/// The shared document shape consumed by `TrigramIndex` / `Bm25Index`.
///
/// See the module doc comment for the structured-vs-text-searchable
/// distinction and the classification-gated allowlist.
#[derive(Debug, Clone)]
pub struct Document {
    pub id: String,
    pub r#type: NoteType,
    pub status: NoteStatus,
    pub trust: TrustLevel,
    pub project: String,
    pub created: String,
    pub updated: String,
    pub event_period: String,
    pub schema_version: u32,
    pub classification: Classification,
    /// The only text ever handed to `TrigramIndex::insert` / `Bm25Index`.
    /// Built according to the classification-gated allowlist; see the
    /// module doc comment.
    pub text_searchable: String,
}

impl Document {
    /// Builds a `Document` from already-parsed metadata and an
    /// already-split body (see the module doc comment for why the split
    /// happens before this call, not inside it).
    ///
    /// `body: None` and `body: Some("")` are equivalent: both contribute no
    /// body text to `text_searchable`.
    pub fn new(metadata: NoteMetadata, body: Option<&str>) -> Self {
        let text_searchable = build_text_searchable(&metadata, body);

        Self {
            id: metadata.id,
            r#type: metadata.r#type,
            status: metadata.status,
            trust: metadata.trust,
            project: metadata.project,
            created: metadata.created,
            updated: metadata.updated,
            event_period: metadata.event_period,
            schema_version: metadata.schema_version,
            classification: metadata.classification,
            text_searchable,
        }
    }

    /// Builds a `Document` straight from raw record bytes — the same shape
    /// `storage::Indexer::mark_current`'s `record_bytes: &[u8]` receives —
    /// by calling `storage::split_record` first and then [`Document::new`].
    /// This is the demonstration that `index` never has to reimplement any
    /// frontmatter/body-splitting logic itself: `storage` owns that logic in
    /// exactly one place.
    pub fn from_record_bytes(record_bytes: &[u8]) -> Result<Self, storage::SplitRecordError> {
        let (metadata, body) = storage::split_record(record_bytes)?;
        Ok(Self::new(metadata, body.as_deref()))
    }
}

/// The ten structured fields' own string forms, in a fixed order, shared by
/// both classification branches below (the `Sensitive` branch is a strict
/// subset of what the `Internal`/`Public` branch includes).
fn structured_fields_as_text(metadata: &NoteMetadata) -> String {
    let parts: Vec<String> = vec![
        metadata.id.clone(),
        metadata.schema_version.to_string(),
        type_as_str(metadata.r#type).to_string(),
        status_as_str(metadata.status).to_string(),
        trust_as_str(metadata.trust).to_string(),
        classification_as_str(metadata.classification).to_string(),
        metadata.project.clone(),
        metadata.created.clone(),
        metadata.updated.clone(),
        metadata.event_period.clone(),
    ];
    parts.join(" ")
}

/// Builds `text_searchable` per the classification-gated allowlist.
fn build_text_searchable(metadata: &NoteMetadata, body: Option<&str>) -> String {
    let structured = structured_fields_as_text(metadata);

    match metadata.classification {
        Classification::Sensitive => {
            // ONLY the ten structured fields' string forms. Never body,
            // never `source`, never any `relations.*` target.
            structured
        }
        Classification::Internal | Classification::Public => {
            let mut parts = vec![structured];
            if let Some(body) = body {
                if !body.is_empty() {
                    parts.push(body.to_string());
                }
            }
            if !metadata.source.is_empty() {
                parts.push(metadata.source.clone());
            }
            for target in relation_targets(&metadata.relations) {
                parts.push(target.clone());
            }
            parts.join(" ")
        }
    }
}

/// All `relations.*` targets, in a fixed field order, flattened into one
/// iterator (each field is itself a `Vec<String>` of targets).
fn relation_targets(relations: &domain::Relations) -> impl Iterator<Item = &String> {
    relations
        .supports
        .iter()
        .chain(relations.contradicts.iter())
        .chain(relations.derived_from.iter())
        .chain(relations.supersedes.iter())
        .chain(relations.applies_to.iter())
}

// `NoteType`/`NoteStatus`/`TrustLevel`/`Classification` are `serde`-derived
// with `#[serde(rename_all = "kebab-case")]` but do not implement
// `std::fmt::Display`. These helpers give each enum its canonical
// kebab-case string form (matching the frontmatter spelling exactly) for
// use inside `text_searchable`, without adding a `Display` impl to
// `domain` that nothing else currently needs.

fn type_as_str(value: NoteType) -> &'static str {
    match value {
        NoteType::Code => "code",
        NoteType::Bug => "bug",
        NoteType::Decision => "decision",
        NoteType::Pattern => "pattern",
        NoteType::Research => "research",
        NoteType::Session => "session",
        NoteType::Summary => "summary",
    }
}

fn status_as_str(value: NoteStatus) -> &'static str {
    match value {
        NoteStatus::Draft => "draft",
        NoteStatus::TestPassed => "test-passed",
        NoteStatus::Reviewed => "reviewed",
        NoteStatus::Verified => "verified",
        NoteStatus::Superseded => "superseded",
        NoteStatus::Contradicted => "contradicted",
    }
}

fn trust_as_str(value: TrustLevel) -> &'static str {
    match value {
        TrustLevel::Web => "web",
        TrustLevel::Internal => "internal",
        TrustLevel::Official => "official",
    }
}

fn classification_as_str(value: Classification) -> &'static str {
    match value {
        Classification::Public => "public",
        Classification::Internal => "internal",
        Classification::Sensitive => "sensitive",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::Relations;

    fn base_metadata(classification: Classification) -> NoteMetadata {
        NoteMetadata {
            id: "bug-2026-10-30-example".to_string(),
            schema_version: 1,
            r#type: NoteType::Bug,
            status: NoteStatus::Draft,
            trust: TrustLevel::Internal,
            classification,
            project: "project-x".to_string(),
            source: "agent:coder@prompt-v3".to_string(),
            created: "2026-10-30".to_string(),
            updated: "2026-10-30".to_string(),
            event_period: "2026-10-30".to_string(),
            relations: Relations {
                supports: vec!["pattern-foo".to_string()],
                ..Relations::default()
            },
        }
    }

    #[test]
    fn internal_note_text_searchable_includes_body_source_and_relations() {
        let metadata = base_metadata(Classification::Internal);
        let doc = Document::new(metadata, Some("the body text, get_user_by_id"));

        assert!(doc.text_searchable.contains("get_user_by_id"));
        assert!(doc.text_searchable.contains("agent:coder@prompt-v3"));
        assert!(doc.text_searchable.contains("pattern-foo"));
        assert!(doc.text_searchable.contains("project-x"));
        assert!(doc.text_searchable.contains("bug"));
    }

    #[test]
    fn sensitive_note_text_searchable_excludes_body_source_and_relations() {
        let metadata = base_metadata(Classification::Sensitive);
        let doc = Document::new(metadata, Some("SECRET-MARKER-BODY"));

        assert!(!doc.text_searchable.contains("SECRET-MARKER-BODY"));
        assert!(!doc.text_searchable.contains("agent:coder@prompt-v3"));
        assert!(!doc.text_searchable.contains("pattern-foo"));
        // The ten structured fields are still present.
        assert!(doc.text_searchable.contains("project-x"));
        assert!(doc.text_searchable.contains("bug-2026-10-30-example"));
    }

    #[test]
    fn sensitive_searchable_metadata_retains_all_ten_canonical_field_values() {
        let doc = Document::new(base_metadata(Classification::Sensitive), Some("excluded-body"));
        assert_eq!(doc.text_searchable.split_whitespace().collect::<Vec<_>>(), [
            "bug-2026-10-30-example", "1", "bug", "draft", "internal", "sensitive",
            "project-x", "2026-10-30", "2026-10-30", "2026-10-30",
        ]);
    }

    #[test]
    fn structured_fields_are_kept_as_typed_values_regardless_of_classification() {
        let metadata = base_metadata(Classification::Sensitive);
        let doc = Document::new(metadata, Some("anything"));

        assert_eq!(doc.id, "bug-2026-10-30-example");
        assert_eq!(doc.r#type, NoteType::Bug);
        assert_eq!(doc.status, NoteStatus::Draft);
        assert_eq!(doc.trust, TrustLevel::Internal);
        assert_eq!(doc.project, "project-x");
        assert_eq!(doc.classification, Classification::Sensitive);
        assert_eq!(doc.schema_version, 1);
    }

    #[test]
    fn none_body_and_empty_body_are_equivalent() {
        let metadata_a = base_metadata(Classification::Internal);
        let metadata_b = base_metadata(Classification::Internal);

        let doc_a = Document::new(metadata_a, None);
        let doc_b = Document::new(metadata_b, Some(""));

        assert_eq!(doc_a.text_searchable, doc_b.text_searchable);
    }

    // --- `from_record_bytes`: raw bytes -> Document without `index` ever
    // reimplementing storage's frontmatter/body-splitting logic itself. ---

    #[test]
    fn from_record_bytes_builds_a_document_matching_the_already_split_constructor() {
        let record_bytes: &[u8] = b"---\nid: bug-2026-10-30-example\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-10-30\nupdated: 2026-10-30\nevent_period: 2026-10-30\nrelations: {}\n---\nbody text with get_user_by_id in it\n";

        let doc = Document::from_record_bytes(record_bytes).expect("valid record must parse");

        assert_eq!(doc.id, "bug-2026-10-30-example");
        assert_eq!(doc.project, "project-x");
        assert!(doc.text_searchable.contains("get_user_by_id"));
        assert!(doc.text_searchable.contains("agent:coder@prompt-v3"));
    }

    #[test]
    fn from_record_bytes_surfaces_split_record_errors() {
        let result = Document::from_record_bytes(b"not: frontmatter");

        assert!(result.is_err());
    }
}
