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
//!   regardless of classification. Temporal values are validated separately;
//!   malformed temporal strings are discarded from this derived Document.
//! - **`text_searchable`** — a single `String`, separately computed, that is
//!   the ONLY thing `TrigramIndex` (this task) or `Bm25Index` (Task 3) ever
//!   tokenizes. Its content depends on classification:
//!   - `Internal` / `Public`: body + every structured field's own string
//!     form + `source` + every `relations.*` target.
//!   - `Sensitive`: the seven non-temporal allowlisted fields only. Dates and
//!     event periods are filter metadata, never postings text, even when valid.
//!     Body, `source` and every `relations.*` target remain excluded.
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

use chrono::{DateTime, NaiveDate, TimeZone};
use chrono_tz::{Europe::Rome, Tz};
use domain::{Classification, NoteMetadata, NoteStatus, NoteType, TrustLevel};

/// Validated event-period values, separate from any searchable text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventPeriod {
    Point(DateTime<Tz>),
    Interval {
        start: DateTime<Tz>,
        end: DateTime<Tz>,
    },
}

fn parse_instant(value: &str) -> Option<DateTime<Tz>> {
    if let Ok(instant) = DateTime::parse_from_rfc3339(value) {
        return Some(instant.with_timezone(&Rome));
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    // Do not accept partial dates, suffixes or alternative free-text spellings.
    if date.format("%Y-%m-%d").to_string() != value {
        return None;
    }
    Rome.from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .single()
}

fn parse_event_period(value: &str) -> Option<EventPeriod> {
    if let Some((start, end)) = value.split_once('/') {
        let (start, end) = (parse_instant(start)?, parse_instant(end)?);
        return (start < end).then_some(EventPeriod::Interval { start, end });
    }
    parse_instant(value).map(EventPeriod::Point)
}

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
    pub created: Option<DateTime<Tz>>,
    pub updated: Option<DateTime<Tz>>,
    pub event_period: Option<EventPeriod>,
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
        let created = parse_instant(&metadata.created);
        let updated = parse_instant(&metadata.updated);
        let event_period = parse_event_period(&metadata.event_period);

        Self {
            id: metadata.id,
            r#type: metadata.r#type,
            status: metadata.status,
            trust: metadata.trust,
            project: metadata.project,
            created,
            updated,
            event_period,
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

/// The seven non-temporal allowlisted fields, in a fixed order. Temporal
/// strings are added only for Internal/Public below, never for Sensitive.
fn structured_fields_as_text(metadata: &NoteMetadata) -> String {
    let parts: Vec<String> = vec![
        metadata.id.clone(),
        metadata.schema_version.to_string(),
        type_as_str(metadata.r#type).to_string(),
        status_as_str(metadata.status).to_string(),
        trust_as_str(metadata.trust).to_string(),
        classification_as_str(metadata.classification).to_string(),
        metadata.project.clone(),
    ];
    parts.join(" ")
}

/// Builds `text_searchable` per the classification-gated allowlist.
fn build_text_searchable(metadata: &NoteMetadata, body: Option<&str>) -> String {
    let structured = structured_fields_as_text(metadata);

    match metadata.classification {
        Classification::Sensitive => {
            // No temporal strings, body, source or relations, without fallback.
            structured
        }
        Classification::Internal | Classification::Public => {
            let mut parts = vec![
                structured,
                metadata.created.clone(),
                metadata.updated.clone(),
                metadata.event_period.clone(),
            ];
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
        // Non-temporal allowlisted metadata remains searchable.
        assert!(doc.text_searchable.contains("project-x"));
        assert!(doc.text_searchable.contains("bug-2026-10-30-example"));
    }

    #[test]
    fn sensitive_searchable_metadata_retains_only_seven_non_temporal_field_values() {
        let doc = Document::new(
            base_metadata(Classification::Sensitive),
            Some("excluded-body"),
        );
        assert_eq!(
            doc.text_searchable.split_whitespace().collect::<Vec<_>>(),
            [
                "bug-2026-10-30-example",
                "1",
                "bug",
                "draft",
                "internal",
                "sensitive",
                "project-x",
            ]
        );
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

    #[test]
    fn sensitive_temporal_markers_never_enter_either_postings_index() {
        for field in ["created", "updated", "event_period"] {
            let marker = format!("TEMPORAL_CANARY_{field}_7391");
            let mut metadata = base_metadata(Classification::Sensitive);
            match field {
                "created" => metadata.created = marker.clone(),
                "updated" => metadata.updated = marker.clone(),
                _ => metadata.event_period = marker.clone(),
            }
            let doc = Document::new(metadata, None);
            let id = storage::NoteId::parse(&doc.id).unwrap();
            let mut trigram = crate::trigram::TrigramIndex::new();
            trigram.insert(id.clone(), &doc.text_searchable);
            let mut bm25 = crate::bm25::Bm25Index::new();
            bm25.insert(id, &doc.text_searchable);
            assert!(
                trigram.query_exact(&marker).is_empty(),
                "{field} trigram marker leak"
            );
            assert!(
                bm25.query_ranked(&marker, 10).is_empty(),
                "{field} BM25 marker leak"
            );
            assert!(!format!("{doc:?}").contains(&marker));
        }
    }

    #[test]
    fn sensitive_valid_dates_are_filter_metadata_only_not_searchable_text() {
        let mut metadata = base_metadata(Classification::Sensitive);
        metadata.created = "2031-05-14".into();
        metadata.updated = "2031-05-15".into();
        metadata.event_period = "2031-05-16/2031-05-18".into();
        let doc = Document::new(metadata, None);
        assert!(!doc.text_searchable.contains("2031-05"));
    }

    #[test]
    fn temporal_metadata_validates_points_and_nonempty_intervals() {
        let mut metadata = base_metadata(Classification::Sensitive);
        metadata.created = "2026-08-10".into();
        metadata.updated = "2026-08-11T10:00:00Z".into();
        metadata.event_period = "2026-07-31/2026-08-02".into();
        let doc = Document::new(metadata, None);
        assert_eq!(
            doc.created.unwrap().to_rfc3339(),
            "2026-08-10T00:00:00+02:00"
        );
        assert_eq!(
            doc.updated.unwrap().to_rfc3339(),
            "2026-08-11T12:00:00+02:00"
        );
        assert_eq!(
            doc.event_period,
            Some(EventPeriod::Interval {
                start: parse_instant("2026-07-31").unwrap(),
                end: parse_instant("2026-08-02").unwrap(),
            })
        );
        let point = Document::new(base_metadata(Classification::Sensitive), None);
        assert_eq!(point.event_period, point.created.map(EventPeriod::Point));
    }

    #[test]
    fn invalid_temporal_metadata_has_no_structured_or_text_fallback() {
        for value in [
            "",
            "DATE-CANARY",
            "2026-02-30",
            "2026-8-1",
            "2026-08-10 trailing",
            "2026-08-10/",
            "/2026-08-10",
            "2026-08-12/2026-08-10",
            "2026-08-10/2026-08-10",
            "2026-08-10/2026-08-11/2026-08-12",
        ] {
            let mut metadata = base_metadata(Classification::Sensitive);
            metadata.created = value.into();
            metadata.updated = value.into();
            metadata.event_period = value.into();
            let doc = Document::new(metadata, None);
            assert_eq!(doc.created, None, "{value}");
            assert_eq!(doc.updated, None, "{value}");
            assert_eq!(doc.event_period, None, "{value}");
        }
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
