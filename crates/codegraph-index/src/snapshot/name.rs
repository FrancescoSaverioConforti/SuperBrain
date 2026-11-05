//! B2 Task 5: `ValidatedName` — a validated LOCAL name, deliberately NOT
//! called `QualifiedName`.
//!
//! Task 4's extractors report a symbol's bare name as tree-sitter sees it
//! (`"method"`, `"foo"`, `"#header"`) — never a fully qualified path
//! (e.g. `module::Class::method` or `package.Class.method`), because
//! building a real qualified name requires resolving a symbol's
//! enclosing scope chain, which none of Task 4's single-node extractors
//! attempt (recognizing that a `function_item` sits inside an
//! `impl_item` is the closest any extractor comes, and even that is not
//! threaded into the reported name). Calling this type `QualifiedName`
//! would assert a capability this pipeline does not have yet. A future
//! resolver task that DOES walk the scope chain can introduce a real
//! `QualifiedName` type then, built from one or more `ValidatedName`
//! values plus resolved scope information — never by silently
//! reinterpreting this type's existing values as if they had always
//! been qualified.

use thiserror::Error;

/// Mirrors `extract::candidate::MAX_NAME_BYTES` (Task 4's own bound) —
/// kept as an independent constant rather than importing Task 4's,
/// because this is a validation Task 5's promotion boundary must enforce
/// on its own terms, not inherited trust in an upstream crate having
/// already checked it (a `RawSymbolCandidate` constructed by some other,
/// future caller of `extract` is not guaranteed to have gone through
/// `RawSymbolCandidate::new`'s own check).
pub const MAX_NAME_BYTES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValidatedName(String);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidatedNameError {
    #[error("symbol name must not be empty")]
    Empty,
    #[error("symbol name is {actual} bytes, maximum is {max}")]
    TooLong { max: usize, actual: usize },
}

impl ValidatedName {
    /// Validates `raw`: non-empty, within [`MAX_NAME_BYTES`]. Never
    /// truncates — a name over the limit is rejected whole, not cut to
    /// fit. Never normalizes (no trimming, no case-folding): a name's
    /// byte-for-byte form as the language extractor reported it is
    /// preserved exactly, mirroring `NoteId::parse`'s discipline.
    pub fn parse(raw: &str) -> Result<Self, ValidatedNameError> {
        if raw.is_empty() {
            return Err(ValidatedNameError::Empty);
        }
        if raw.len() > MAX_NAME_BYTES {
            return Err(ValidatedNameError::TooLong {
                max: MAX_NAME_BYTES,
                actual: raw.len(),
            });
        }
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_name_is_accepted() {
        assert!(ValidatedName::parse("foo").is_ok());
    }

    #[test]
    fn an_empty_name_is_rejected() {
        assert_eq!(ValidatedName::parse(""), Err(ValidatedNameError::Empty));
    }

    #[test]
    fn a_name_exactly_at_the_limit_is_accepted() {
        let name = "a".repeat(MAX_NAME_BYTES);
        assert!(ValidatedName::parse(&name).is_ok());
    }

    #[test]
    fn a_name_one_byte_over_the_limit_is_rejected_not_truncated() {
        let name = "a".repeat(MAX_NAME_BYTES + 1);
        assert_eq!(
            ValidatedName::parse(&name),
            Err(ValidatedNameError::TooLong {
                max: MAX_NAME_BYTES,
                actual: MAX_NAME_BYTES + 1,
            })
        );
    }

    #[test]
    fn accepted_name_is_returned_byte_for_byte_with_no_normalization() {
        assert_eq!(ValidatedName::parse("Foo_Bar123").unwrap().as_str(), "Foo_Bar123");
    }
}
