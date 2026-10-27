use serde::Deserialize;
use serde_yaml::Value;

use crate::schema::{Classification, NoteStatus, NoteType, TrustLevel, ValidationError};

/// `deny_unknown_fields`: a typo'd relation key (e.g. `supprots`) must be a
/// loud error, never a silently ignored field whose links then vanish from
/// link validation (A1 final review, Finding I1). `parse_frontmatter` also
/// checks the keys itself first, against [`RELATION_FIELDS`], so it can
/// report the exact offending key as `ValidationError::UnknownField`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relations {
    #[serde(default)]
    pub supports: Vec<String>,
    #[serde(default)]
    pub contradicts: Vec<String>,
    #[serde(default)]
    pub derived_from: Vec<String>,
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default)]
    pub applies_to: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NoteMetadata {
    pub id: String,
    pub schema_version: u32,
    pub r#type: NoteType,
    pub status: NoteStatus,
    pub trust: TrustLevel,
    pub classification: Classification,
    pub project: String,
    pub source: String,
    pub created: String,
    pub updated: String,
    pub event_period: String,
    pub relations: Relations,
}

/// Fields are checked in this order; the first missing/invalid one determines
/// which `ValidationError` is returned.
const FIELD_ORDER: &[&str] = &[
    "id",
    "schema_version",
    "type",
    "status",
    "trust",
    "classification",
    "project",
    "source",
    "created",
    "updated",
    "event_period",
    "relations",
];

/// The only keys accepted inside `relations`.
const RELATION_FIELDS: &[&str] = &["supports", "contradicts", "derived_from", "supersedes", "applies_to"];

/// Returns the first key of `mapping` (in document order) that is not in
/// `allowed`, rendered as a string (non-string YAML keys are rendered via
/// their YAML serialization so they can still be named in the error).
fn first_unknown_key(mapping: &serde_yaml::Mapping, allowed: &[&str]) -> Option<String> {
    mapping.keys().find_map(|key| match key.as_str() {
        Some(name) if allowed.contains(&name) => None,
        Some(name) => Some(name.to_string()),
        None => Some(
            serde_yaml::to_string(key)
                .map(|s| s.trim_end().to_string())
                .unwrap_or_else(|_| format!("{key:?}")),
        ),
    })
}

pub fn parse_frontmatter(yaml: &str) -> Result<NoteMetadata, ValidationError> {
    let raw: Value =
        serde_yaml::from_str(yaml).map_err(|_| ValidationError::MissingField(FIELD_ORDER[0].to_string()))?;
    let mapping = raw.as_mapping().ok_or_else(|| ValidationError::MissingField(FIELD_ORDER[0].to_string()))?;

    // Unknown keys are checked before missing ones, so a misspelled
    // required key is reported by its actual (wrong) name.
    if let Some(unknown) = first_unknown_key(mapping, FIELD_ORDER) {
        return Err(ValidationError::UnknownField(unknown));
    }

    let get = |field: &str| -> Option<&Value> { mapping.get(Value::String(field.to_string())) };

    for field in FIELD_ORDER {
        if get(field).is_none() {
            return Err(ValidationError::MissingField(field.to_string()));
        }
    }

    let id = require_string(get("id").unwrap(), "id")?;

    let schema_version = get("schema_version")
        .unwrap()
        .as_u64()
        .ok_or_else(|| ValidationError::InvalidFieldType {
            field: "schema_version".to_string(),
            expected: "integer".to_string(),
        })? as u32;

    let type_raw = require_string(get("type").unwrap(), "type")?;
    let r#type = parse_enum(&type_raw, "type")?;

    let status_raw = require_string(get("status").unwrap(), "status")?;
    let status = parse_enum(&status_raw, "status")?;

    let trust_raw = require_string(get("trust").unwrap(), "trust")?;
    let trust = parse_enum(&trust_raw, "trust")?;

    let classification_raw = require_string(get("classification").unwrap(), "classification")?;
    if classification_raw == "secret" {
        return Err(ValidationError::SecretClassificationForbidden);
    }
    let classification = parse_enum(&classification_raw, "classification")?;

    let project = require_string(get("project").unwrap(), "project")?;
    let source = require_string(get("source").unwrap(), "source")?;
    let created = require_string(get("created").unwrap(), "created")?;
    let updated = require_string(get("updated").unwrap(), "updated")?;
    let event_period = require_string(get("event_period").unwrap(), "event_period")?;

    let relations_value = get("relations").unwrap().clone();
    if let Some(relations_mapping) = relations_value.as_mapping() {
        if let Some(unknown) = first_unknown_key(relations_mapping, RELATION_FIELDS) {
            return Err(ValidationError::UnknownField(format!("relations.{unknown}")));
        }
    }
    let relations: Relations =
        serde_yaml::from_value(relations_value).map_err(|_| ValidationError::InvalidFieldType {
            field: "relations".to_string(),
            expected: "mapping".to_string(),
        })?;

    Ok(NoteMetadata {
        id,
        schema_version,
        r#type,
        status,
        trust,
        classification,
        project,
        source,
        created,
        updated,
        event_period,
        relations,
    })
}

fn require_string(value: &Value, field: &str) -> Result<String, ValidationError> {
    value
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| ValidationError::InvalidFieldType {
            field: field.to_string(),
            expected: "string".to_string(),
        })
}

fn parse_enum<T: serde::de::DeserializeOwned>(raw: &str, field: &str) -> Result<T, ValidationError> {
    serde_yaml::from_value(Value::String(raw.to_string())).map_err(|_| ValidationError::InvalidEnumValue {
        field: field.to_string(),
        value: raw.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_yaml_with_classification(classification: &str) -> String {
        format!(
            r#"
id: bug-2026-10-23-fastapi-cors
schema_version: 1
type: bug
status: draft
trust: internal
classification: {classification}
project: project-x
source: agent:coder@prompt-v3
created: 2026-10-23
updated: 2026-10-23
event_period: 2026-10-23
relations:
  supports: []
  contradicts: []
  derived_from: []
  supersedes: []
  applies_to: []
"#
        )
    }

    fn base_yaml_with_created_as_number() -> String {
        r#"
id: bug-2026-10-23-fastapi-cors
schema_version: 1
type: bug
status: draft
trust: internal
classification: internal
project: project-x
source: agent:coder@prompt-v3
created: 20261023
updated: 2026-10-23
event_period: 2026-10-23
relations:
  supports: []
  contradicts: []
  derived_from: []
  supersedes: []
  applies_to: []
"#
        .to_string()
    }

    #[test]
    fn valid_note_is_accepted() {
        let yaml = r#"
id: bug-2026-10-23-fastapi-cors
schema_version: 1
type: bug
status: draft
trust: internal
classification: internal
project: project-x
source: agent:coder@prompt-v3
created: 2026-10-23
updated: 2026-10-23
event_period: 2026-10-23
relations:
  supports: []
  contradicts: []
  derived_from: []
  supersedes: []
  applies_to: []
"#;
        let note = parse_frontmatter(yaml).unwrap();
        assert_eq!(note.id, "bug-2026-10-23-fastapi-cors");
    }

    #[test]
    fn missing_field_names_the_field() {
        let yaml = "id: x\nschema_version: 1\n"; // missing type, status, etc.
        let err = parse_frontmatter(yaml).unwrap_err();
        match err {
            ValidationError::MissingField(f) => assert_eq!(f, "type"),
            other => panic!("expected MissingField(type), got {other:?}"),
        }
    }

    #[test]
    fn secret_classification_is_always_rejected() {
        let yaml = base_yaml_with_classification("secret");
        let err = parse_frontmatter(&yaml).unwrap_err();
        assert!(matches!(err, ValidationError::SecretClassificationForbidden));
    }

    #[test]
    fn unknown_classification_value_is_named_field_error() {
        let yaml = base_yaml_with_classification("top-secret");
        let err = parse_frontmatter(&yaml).unwrap_err();
        match err {
            ValidationError::InvalidEnumValue { field, value } => {
                assert_eq!(field, "classification");
                assert_eq!(value, "top-secret");
            }
            other => panic!("expected InvalidEnumValue, got {other:?}"),
        }
    }

    // --- A1 final review, Finding I1: unknown keys must be loud, not ignored ---

    #[test]
    fn unknown_top_level_key_is_rejected_with_named_unknown_field() {
        let yaml = format!("{}totally_unknown: 1\n", base_yaml_with_classification("internal"));
        let err = parse_frontmatter(&yaml).unwrap_err();
        match err {
            ValidationError::UnknownField(f) => assert_eq!(f, "totally_unknown"),
            other => panic!("expected UnknownField(totally_unknown), got {other:?}"),
        }
    }

    #[test]
    fn typo_in_relation_key_is_rejected_not_silently_dropped() {
        // "supprots" (typo of "supports") used to be silently ignored, so
        // the broken link inside it vanished instead of failing link
        // validation.
        let yaml = base_yaml_with_classification("internal")
            .replace("  supports: []", "  supprots: [\"missing-id\"]");
        let err = parse_frontmatter(&yaml).unwrap_err();
        match err {
            ValidationError::UnknownField(f) => assert_eq!(f, "relations.supprots"),
            other => panic!("expected UnknownField(relations.supprots), got {other:?}"),
        }
    }

    #[test]
    fn misspelled_top_level_field_is_reported_as_unknown_not_just_missing() {
        // A typo'd required key names the actual offending key, rather than
        // only the required field it was presumably meant to be.
        let yaml = base_yaml_with_classification("internal").replace("project:", "projetc:");
        let err = parse_frontmatter(&yaml).unwrap_err();
        match err {
            ValidationError::UnknownField(f) => assert_eq!(f, "projetc"),
            other => panic!("expected UnknownField(projetc), got {other:?}"),
        }
    }

    #[test]
    fn relations_struct_itself_denies_unknown_fields_when_deserialized_directly() {
        // Defense in depth: Relations is public and Deserialize, so it must
        // reject unknown keys even outside parse_frontmatter.
        assert!(serde_yaml::from_str::<Relations>("supprots: []").is_err());
        assert!(serde_yaml::from_str::<Relations>("supports: []").is_ok());
    }

    #[test]
    fn wrong_type_for_created_is_named_field_error() {
        let yaml = base_yaml_with_created_as_number(); // created: 20261023 (int, not string)
        let err = parse_frontmatter(&yaml).unwrap_err();
        match err {
            ValidationError::InvalidFieldType { field, .. } => assert_eq!(field, "created"),
            other => panic!("expected InvalidFieldType(created), got {other:?}"),
        }
    }
}
