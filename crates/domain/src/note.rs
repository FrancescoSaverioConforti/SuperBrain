use serde::Deserialize;
use serde_yaml::Value;

use crate::schema::{Classification, NoteStatus, NoteType, TrustLevel, ValidationError};

#[derive(Debug, Clone, Default, Deserialize)]
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

pub fn parse_frontmatter(yaml: &str) -> Result<NoteMetadata, ValidationError> {
    let raw: Value =
        serde_yaml::from_str(yaml).map_err(|_| ValidationError::MissingField(FIELD_ORDER[0].to_string()))?;
    let mapping = raw.as_mapping().ok_or_else(|| ValidationError::MissingField(FIELD_ORDER[0].to_string()))?;

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
