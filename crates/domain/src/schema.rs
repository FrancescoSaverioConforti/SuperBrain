use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoteType {
    Code,
    Bug,
    Decision,
    Pattern,
    Research,
    Session,
    Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoteStatus {
    Draft,
    TestPassed,
    Reviewed,
    Verified,
    Superseded,
    Contradicted,
}

// Declaration order is the correctness requirement: derive(Ord) orders by
// variant position, and the spec requires web < internal < official.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrustLevel {
    Web,
    Internal,
    Official,
}

// No `Secret` variant: `classification: secret` must never become a valid
// in-memory value. It is intercepted at parse time and rejected via
// `ValidationError::SecretClassificationForbidden` instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Classification {
    Public,
    Internal,
    Sensitive,
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("missing field: {0}")]
    MissingField(String),
    #[error("invalid type for field {field}: expected {expected}")]
    InvalidFieldType { field: String, expected: String },
    #[error("invalid value for field {field}: {value}")]
    InvalidEnumValue { field: String, value: String },
    #[error("classification 'secret' is always forbidden")]
    SecretClassificationForbidden,
    /// A frontmatter key that is not part of the schema. Top-level keys are
    /// reported by name (`"totally_unknown"`); keys inside `relations` are
    /// reported with a `relations.` prefix (`"relations.supprots"`), so a
    /// typo'd relation is a loud error instead of a silently dropped link.
    #[error("unknown field: {0}")]
    UnknownField(String),
}
