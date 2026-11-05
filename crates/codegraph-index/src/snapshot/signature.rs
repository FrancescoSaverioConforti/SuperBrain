//! B2 Task 5: `BoundedSignature` — validated, bounded signature text.

use thiserror::Error;

/// Mirrors `extract::candidate::MAX_SIGNATURE_BYTES` — see
/// `name.rs`'s doc comment on why this is an independent constant rather
/// than an import of Task 4's.
pub const MAX_SIGNATURE_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoundedSignature(String);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BoundedSignatureError {
    #[error("signature text is {actual} bytes, maximum is {max}")]
    TooLong { max: usize, actual: usize },
}

impl BoundedSignature {
    /// Validates `raw` is within [`MAX_SIGNATURE_BYTES`]. Never
    /// truncates. Unlike [`crate::snapshot::name::ValidatedName`], an
    /// EMPTY signature is accepted — a symbol whose extractor reported
    /// no meaningful signature text beyond its own span is not
    /// necessarily invalid at this layer (Task 4's current extractors
    /// always populate it with the construct's own source text, so this
    /// case does not arise from them today, but this type does not
    /// assume that will always hold for every future extractor).
    pub fn parse(raw: &str) -> Result<Self, BoundedSignatureError> {
        if raw.len() > MAX_SIGNATURE_BYTES {
            return Err(BoundedSignatureError::TooLong {
                max: MAX_SIGNATURE_BYTES,
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
    fn a_normal_signature_is_accepted() {
        assert!(BoundedSignature::parse("fn foo()").is_ok());
    }

    #[test]
    fn an_empty_signature_is_accepted() {
        assert!(BoundedSignature::parse("").is_ok());
    }

    #[test]
    fn a_signature_exactly_at_the_limit_is_accepted() {
        let signature = "s".repeat(MAX_SIGNATURE_BYTES);
        assert!(BoundedSignature::parse(&signature).is_ok());
    }

    #[test]
    fn a_signature_one_byte_over_the_limit_is_rejected_not_truncated() {
        let signature = "s".repeat(MAX_SIGNATURE_BYTES + 1);
        assert_eq!(
            BoundedSignature::parse(&signature),
            Err(BoundedSignatureError::TooLong {
                max: MAX_SIGNATURE_BYTES,
                actual: MAX_SIGNATURE_BYTES + 1,
            })
        );
    }

    #[test]
    fn accepted_signature_is_returned_byte_for_byte() {
        let signature = "fn multi(\n    a: i32,\n) -> i32 { a }";
        assert_eq!(BoundedSignature::parse(signature).unwrap().as_str(), signature);
    }
}
