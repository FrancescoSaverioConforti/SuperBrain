//! B2 Task 4: [`RawSymbolCandidate`] — the raw, unvalidated output of
//! per-language extraction, and [`ByteSpan`] — this module's one span
//! convention.
//!
//! # Span convention
//!
//! [`ByteSpan`] is zero-based, end-EXCLUSIVE byte offsets into the
//! [`crate::parse::ParseTree`]'s own source bytes — exactly
//! `tree_sitter::Node::start_byte()`/`end_byte()`'s own convention
//! (`source[start_byte..end_byte]` yields the node's exact text; verified
//! empirically against real parses of every one of the nine supported
//! languages before writing this module, not assumed from documentation,
//! which does not state the half-open convention explicitly). This
//! module never derives a line/column position from these offsets: doing
//! so would require walking the source counting line breaks under an
//! implicit UTF-8-vs-raw-bytes assumption this module does not want to
//! bake in at the Raw/Task-4 layer. A line/column conversion, if ever
//! needed, belongs in a user-facing presentation layer built on top of
//! these byte offsets plus the exact same source bytes, which this
//! module does not attempt to be.

use thiserror::Error;

/// A zero-based, end-exclusive byte range into some source. See this
/// module's doc comment for the convention and why it is never converted
/// to line/column here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ByteSpan {
    pub start: usize,
    pub end: usize,
}

impl ByteSpan {
    pub fn from_node(node: &tree_sitter::Node) -> Self {
        Self {
            start: node.start_byte(),
            end: node.end_byte(),
        }
    }
}

/// Maximum accepted length, in bytes, of [`RawSymbolCandidate::signature_text`]
/// and [`RawSymbolCandidate::name`]. Per the task's instruction: exceeded
/// text is never silently truncated. This module's chosen semantics (see
/// [`extract_symbols`]'s doc comment in `mod.rs`): a candidate whose
/// `signature_text` or `name` would exceed this bound is DROPPED from the
/// returned list, not truncated and not surfaced as a hard error for the
/// whole file — one oversized construct should not fail extraction for
/// every other, normal-sized symbol in the same file. The drop reason is
/// recorded in [`CandidateRejection`] for a caller that wants to know what
/// was omitted and why, rather than silently losing the information.
pub const MAX_NAME_BYTES: usize = 512;
pub const MAX_SIGNATURE_BYTES: usize = 4096;

/// The raw, unvalidated output of one per-language extractor's walk over
/// a [`crate::parse::ParseTree`]. Deliberately a DTO: `kind_hint` is a
/// raw string (e.g. `"function_definition"`, this module's own per-
/// language vocabulary, not yet `codegraph_domain::SymbolKind`), and
/// nothing about this type is "validated" in the sense Task 5 will
/// require. There is no constructor that could be mistaken for a
/// validating one — [`RawSymbolCandidate::new`] performs exactly one
/// check (the bounded-text rule above) and otherwise accepts its
/// fields as-is; it does not canonicalize `name`, does not map
/// `kind_hint` against any enum, and does not touch `signature_text`
/// beyond the length check. This type must never cross a public API
/// boundary or be persisted as a canonical schema — Task 5's `promote`
/// function is the only sanctioned path from a `RawSymbolCandidate` to
/// anything durable or exposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSymbolCandidate {
    pub name: String,
    pub kind_hint: String,
    pub span: ByteSpan,
    pub signature_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CandidateRejection {
    /// `name` itself is what exceeded the bound, so the oversized text
    /// is deliberately NOT carried here (echoing it back would defeat
    /// the whole point of a bounded-text discipline) — a future
    /// consumer wanting to correlate this rejection with a specific
    /// call site has no usable name to match against, confirmed as a
    /// real, distinct residual limitation from `SignatureTooLong`'s
    /// own case below (see `name` there).
    #[error("candidate name is {actual} bytes, maximum accepted is {max}")]
    NameTooLong { max: usize, actual: usize },
    /// Semantic Resolution & Reliability Checkpoint, Priority 3: `name`
    /// carries the candidate's own (bound-respecting, already-short)
    /// name — the signature text is what was rejected, not the name
    /// itself, so it is always safe and available to echo here. This
    /// is the ONE piece of data `CallAttempt::unresolved_with_extraction_rejection`
    /// needs to link a downstream `Unresolved` call attempt back to
    /// the real extraction-time cause, per the Priority 3 finding that
    /// a `SignatureTooLong`-dropped symbol previously looked
    /// indistinguishable from a genuine external target.
    #[error("candidate {name:?}'s signature_text is {actual} bytes, maximum accepted is {max}")]
    SignatureTooLong { name: String, max: usize, actual: usize },
}

impl RawSymbolCandidate {
    /// Builds a candidate, or reports exactly why it was rejected. Never
    /// truncates `name`/`signature_text` to fit — a value either fits
    /// within [`MAX_NAME_BYTES`]/[`MAX_SIGNATURE_BYTES`] as given, or the
    /// whole candidate is rejected with a typed reason. This is the ONLY
    /// bounds check this type performs; it is not a validation of
    /// `kind_hint` or of `name`'s content (e.g. it does not reject an
    /// empty name — a per-language extractor decides whether an empty
    /// name is even reachable for a given `kind_hint`, since that is a
    /// language-semantics question, not a universal DTO-shape one).
    pub fn new(name: String, kind_hint: String, span: ByteSpan, signature_text: String) -> Result<Self, CandidateRejection> {
        if name.len() > MAX_NAME_BYTES {
            return Err(CandidateRejection::NameTooLong {
                max: MAX_NAME_BYTES,
                actual: name.len(),
            });
        }
        if signature_text.len() > MAX_SIGNATURE_BYTES {
            return Err(CandidateRejection::SignatureTooLong {
                name,
                max: MAX_SIGNATURE_BYTES,
                actual: signature_text.len(),
            });
        }
        Ok(Self {
            name,
            kind_hint,
            span,
            signature_text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_candidate_within_bounds_is_accepted() {
        let result = RawSymbolCandidate::new(
            "foo".to_string(),
            "function_definition".to_string(),
            ByteSpan { start: 0, end: 10 },
            "fn foo()".to_string(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn a_name_exactly_at_the_limit_is_accepted() {
        let name = "a".repeat(MAX_NAME_BYTES);
        let result = RawSymbolCandidate::new(name, "k".to_string(), ByteSpan { start: 0, end: 1 }, "s".to_string());
        assert!(result.is_ok());
    }

    #[test]
    fn a_name_one_byte_over_the_limit_is_rejected_not_truncated() {
        let name = "a".repeat(MAX_NAME_BYTES + 1);
        let result = RawSymbolCandidate::new(name, "k".to_string(), ByteSpan { start: 0, end: 1 }, "s".to_string());
        assert_eq!(
            result,
            Err(CandidateRejection::NameTooLong {
                max: MAX_NAME_BYTES,
                actual: MAX_NAME_BYTES + 1,
            })
        );
    }

    #[test]
    fn a_signature_exactly_at_the_limit_is_accepted() {
        let signature = "s".repeat(MAX_SIGNATURE_BYTES);
        let result = RawSymbolCandidate::new("n".to_string(), "k".to_string(), ByteSpan { start: 0, end: 1 }, signature);
        assert!(result.is_ok());
    }

    #[test]
    fn a_signature_one_byte_over_the_limit_is_rejected_not_truncated() {
        let signature = "s".repeat(MAX_SIGNATURE_BYTES + 1);
        let result = RawSymbolCandidate::new("n".to_string(), "k".to_string(), ByteSpan { start: 0, end: 1 }, signature);
        assert_eq!(
            result,
            Err(CandidateRejection::SignatureTooLong {
                name: "n".to_string(),
                max: MAX_SIGNATURE_BYTES,
                actual: MAX_SIGNATURE_BYTES + 1,
            })
        );
    }

    #[test]
    fn signature_too_long_carries_the_real_candidate_name_for_downstream_provenance_linkage() {
        // Semantic Resolution & Reliability Checkpoint, Priority 3:
        // confirms the rejection's own `name` field is the REAL
        // candidate name, not a placeholder -- this is the one piece
        // of data that lets a downstream CallAttempt correctly link
        // "this callee name was rejected at extraction" instead of
        // looking identical to a genuine external/unknown target.
        let signature = "s".repeat(MAX_SIGNATURE_BYTES + 1);
        let result = RawSymbolCandidate::new("Handle".to_string(), "k".to_string(), ByteSpan { start: 0, end: 1 }, signature);
        match result {
            Err(CandidateRejection::SignatureTooLong { name, .. }) => assert_eq!(name, "Handle"),
            other => panic!("expected SignatureTooLong, got {other:?}"),
        }
    }

    #[test]
    fn byte_span_from_node_matches_the_nodes_own_start_and_end() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        let source = b"fn main() {}";
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        let span = ByteSpan::from_node(&root);
        assert_eq!(span.start, root.start_byte());
        assert_eq!(span.end, root.end_byte());
        assert_eq!(&source[span.start..span.end], source);
    }
}
