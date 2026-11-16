//! `BrainApiError`: the frozen domain/service error vocabulary for
//! `brain-api` (B5.0/B5.14 — `docs/superpowers/specs/2026-11-15-b5-0-brain-api-contract-freeze.md`'s
//! "Frozen typed error set" section). Exactly these 10 variants; no
//! capability or internal mapping may introduce an 11th without a new
//! contract freeze.
//!
//! This is the DOMAIN/SERVICE error vocabulary — independent of how
//! `storage`, the graph, retrieval, MCP transport, or proposal persistence
//! report their own internal errors. Every one of THOSE must be mapped
//! explicitly onto one of these 10 variants at the boundary where
//! `brain-api` calls into them (a later task's job, per capability); this
//! module only defines the vocabulary they map onto.
//!
//! ## Decided mappings (not enforced by this module itself, since the
//! mapping sites don't exist yet — recorded here so every future mapping
//! site is consistent, not reinvented per call site):
//!
//! - **Invalid proposal transition** (e.g. approving an already-`Approved`/
//!   `Rejected`/`Expired` proposal) → [`BrainApiError::Conflict`]. The
//!   request itself may be perfectly well-formed; it is the resource's
//!   CURRENT STATE that makes the operation impossible right now — a state
//!   conflict, not malformed input.
//! - **Expired proposal** (at approval time, regardless of persisted
//!   status — the design doc's defense-in-depth requirement) →
//!   [`BrainApiError::InvalidInput`] with the message `"proposal expired"`.
//!   Unlike the transition case above, this is about the REQUEST no longer
//!   being actionable (the window closed), not a conflicting resource
//!   state — kept exactly as already decided at plan Task 14, not changed
//!   here.
//! - **Internal/unexpected error** with no more specific category →
//!   [`BrainApiError::RuntimeUnavailable`]. No `Internal` variant exists in
//!   the frozen set and none is added by this task. Use `CorruptData` only
//!   when there is actual evidence of data/integrity corruption; use
//!   `DenseUnavailable` only for the dense retrieval backend specifically;
//!   `RuntimeUnavailable` is the catch-all for backend/runtime/invariant
//!   failures with no more specific category, including a caught panic or
//!   an FFI failure — never surfacing the panic payload, a stack trace, a
//!   filesystem path, SQL, or any internal identifier in the public
//!   `Display` text.
//! - **Unsupported schema/version** → [`BrainApiError::UnsupportedOperation`].
//!   The frozen set has no dedicated `UnsupportedSchema` variant (unlike
//!   `api::ApiError`, which does) — version mismatches map onto the one
//!   general "this operation is not supported" variant.
//!
//! ## Security semantics: `PermissionDenied` vs `NotFound`
//!
//! These are deliberately distinct variants with distinct `Display` text.
//! Whether a specific capability's own boundary code must collapse them
//! (so an unauthorized caller cannot distinguish "exists but denied" from
//! "does not exist") is a PER-CAPABILITY decision made at that capability's
//! own implementation site, following the design doc's existing example
//! (`approve_proposal`/`reject_proposal` check permission before loading
//! the proposal, so an unauthorized caller gets `PermissionDenied` without
//! ever learning whether the id exists). This module defines both
//! variants with genuinely different text; it is not where that
//! collapsing decision is made or enforced.

/// The frozen Brain API error vocabulary. Exactly 10 variants (B5.0/B5.14).
/// No variant carries a filesystem path, SQL fragment, stack trace,
/// internal object identifier, or secret material — `InvalidInput`'s
/// `String` payload is caller-facing explanatory text, not a dump of
/// internal error detail (diagnostic detail belongs in a future internal
/// audit/log facade, not this type).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrainApiError {
    NotFound,
    InvalidInput(String),
    PermissionDenied,
    RuntimeUnavailable,
    DenseUnavailable,
    ExternalToolUnavailable,
    Timeout,
    Conflict,
    CorruptData,
    UnsupportedOperation,
}

impl std::fmt::Display for BrainApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("resource not found"),
            Self::InvalidInput(reason) => write!(f, "invalid input: {reason}"),
            Self::PermissionDenied => f.write_str("permission denied"),
            Self::RuntimeUnavailable => f.write_str("runtime unavailable"),
            Self::DenseUnavailable => f.write_str("dense retrieval backend unavailable"),
            Self::ExternalToolUnavailable => f.write_str("external tool unavailable"),
            Self::Timeout => f.write_str("operation timed out"),
            Self::Conflict => f.write_str("conflict with current resource state"),
            Self::CorruptData => f.write_str("data integrity could not be verified"),
            Self::UnsupportedOperation => f.write_str("unsupported operation"),
        }
    }
}

impl std::error::Error for BrainApiError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_variants() -> Vec<BrainApiError> {
        vec![
            BrainApiError::NotFound,
            BrainApiError::InvalidInput("x".into()),
            BrainApiError::PermissionDenied,
            BrainApiError::RuntimeUnavailable,
            BrainApiError::DenseUnavailable,
            BrainApiError::ExternalToolUnavailable,
            BrainApiError::Timeout,
            BrainApiError::Conflict,
            BrainApiError::CorruptData,
            BrainApiError::UnsupportedOperation,
        ]
    }

    #[test]
    fn every_frozen_variant_exists_and_has_a_stable_nonempty_display_message() {
        for case in all_variants() {
            assert!(!case.to_string().is_empty(), "{case:?} produced an empty Display message");
        }
    }

    #[test]
    fn display_messages_are_distinct_across_variants() {
        // No two variants should be indistinguishable to a caller reading
        // only the Display text -- each of the 10 must be identifiable.
        let messages: Vec<String> = all_variants().iter().map(|e| e.to_string()).collect();
        let unique: std::collections::HashSet<&String> = messages.iter().collect();
        assert_eq!(messages.len(), unique.len(), "two variants produced the same Display text: {messages:?}");
    }

    #[test]
    fn equality_is_deterministic_and_structural() {
        assert_eq!(BrainApiError::NotFound, BrainApiError::NotFound);
        assert_eq!(BrainApiError::InvalidInput("same".into()), BrainApiError::InvalidInput("same".into()));
        assert_ne!(BrainApiError::InvalidInput("a".into()), BrainApiError::InvalidInput("b".into()));
        assert_ne!(BrainApiError::NotFound, BrainApiError::PermissionDenied);
    }

    // --- security semantics: PermissionDenied and NotFound are genuinely
    //     distinct, non-conflatable variants ---

    #[test]
    fn permission_denied_and_not_found_are_distinct_variants_with_distinct_text() {
        assert_ne!(BrainApiError::PermissionDenied, BrainApiError::NotFound);
        assert_ne!(BrainApiError::PermissionDenied.to_string(), BrainApiError::NotFound.to_string());
    }

    // --- no sensitive detail in the public Display representation ---

    #[test]
    fn invalid_input_display_contains_exactly_the_given_reason_no_more() {
        // The caller-supplied reason string is echoed verbatim (it is
        // explanatory text the capability itself chose to surface, e.g.
        // "proposal expired") -- this is NOT a channel for leaking internal
        // detail; a capability's own boundary code is responsible for
        // never passing a filesystem path, SQL fragment, or internal id as
        // this string. Pinned here: the Display impl itself adds no extra
        // internal detail beyond the given reason.
        let err = BrainApiError::InvalidInput("proposal expired".into());
        assert_eq!(err.to_string(), "invalid input: proposal expired");
    }

    #[test]
    fn no_variant_display_text_contains_path_like_or_sql_like_substrings() {
        // A coarse but real guard: none of the 9 non-InvalidInput variants'
        // fixed Display text should ever contain a path separator, a
        // drive-letter-colon, or a SQL keyword -- since those variants'
        // text is fully controlled by this module (unlike InvalidInput's
        // caller-supplied payload), this is a check this module can and
        // should hold itself to exactly.
        for case in all_variants() {
            if matches!(case, BrainApiError::InvalidInput(_)) {
                continue; // caller-supplied payload, not this module's own fixed text
            }
            let text = case.to_string();
            assert!(!text.contains('\\') && !text.contains('/'), "{case:?}'s Display text looks path-like: {text:?}");
            assert!(!text.to_uppercase().contains("SELECT"), "{case:?}'s Display text looks SQL-like: {text:?}");
        }
    }

    // --- decided mappings, pinned as documentation-by-test (not enforced
    //     by this module -- the mapping SITES don't exist yet -- but these
    //     tests fix the vocabulary's own intended meaning so a future
    //     mapping site has something concrete to match against) ---

    #[test]
    fn invalid_proposal_transition_maps_to_conflict_not_invalid_input() {
        // Decided mapping (user, 2026-11-16): an invalid proposal state
        // transition (e.g. approving an already-resolved proposal) is a
        // Conflict -- the request is well-formed, the resource's current
        // state makes it inapplicable now.
        let invalid_transition_outcome = BrainApiError::Conflict;
        assert_eq!(invalid_transition_outcome, BrainApiError::Conflict);
        assert_ne!(invalid_transition_outcome, BrainApiError::InvalidInput("already approved".into()));
    }

    #[test]
    fn expired_proposal_maps_to_invalid_input_with_the_exact_frozen_message() {
        // Decided mapping (kept from plan Task 14, unchanged by this task):
        // an expired proposal, checked at approval time regardless of
        // persisted status, is InvalidInput("proposal expired") -- not
        // Conflict, since the issue is the REQUEST no longer being
        // actionable (the approval window closed), not a conflicting
        // resource state.
        let expired_outcome = BrainApiError::InvalidInput("proposal expired".into());
        assert_eq!(expired_outcome, BrainApiError::InvalidInput("proposal expired".into()));
    }

    #[test]
    fn unspecified_internal_failure_maps_to_runtime_unavailable_not_a_new_variant() {
        // Decided mapping (user, 2026-11-16): no Internal variant exists or
        // is added. An internal/unexpected failure with no more specific
        // category (caught panic, FFI failure, etc.) maps to
        // RuntimeUnavailable.
        let internal_failure_outcome = BrainApiError::RuntimeUnavailable;
        assert_eq!(internal_failure_outcome, BrainApiError::RuntimeUnavailable);
    }

    #[test]
    fn unsupported_schema_or_version_maps_to_unsupported_operation() {
        // Decided mapping: the frozen set has no dedicated
        // UnsupportedSchema variant (unlike api::ApiError, which has one);
        // a version/schema mismatch maps onto UnsupportedOperation.
        let unsupported_schema_outcome = BrainApiError::UnsupportedOperation;
        assert_eq!(unsupported_schema_outcome, BrainApiError::UnsupportedOperation);
    }

    #[test]
    fn implements_std_error() {
        fn assert_is_error<E: std::error::Error>(_: &E) {}
        assert_is_error(&BrainApiError::NotFound);
    }
}
