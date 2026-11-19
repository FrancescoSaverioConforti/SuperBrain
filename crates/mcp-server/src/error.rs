//! Deterministic `brain_api::BrainApiError` → `rmcp::ErrorData` mapping
//! (Task 22). The one place this conversion happens, so every tool wrapper
//! maps errors identically — no tool invents its own ad hoc mapping.
//!
//! Every `ErrorData` produced here carries only the `BrainApiError`
//! variant's own caller-facing `Display` text (already proven, by
//! `brain-api`'s own `error.rs` test suite, to contain no filesystem path,
//! SQL fragment, stack trace, or internal identifier) — never a
//! `BrainApiError::InvalidInput` payload extended with anything this
//! module adds, and never the `Debug` representation of anything.
//! `PermissionDenied` is never softened into an empty success or any other
//! fallback: it always becomes a distinct, typed MCP error.

use rmcp::ErrorData as McpError;

/// Maps `err` onto the one fixed `rmcp::model::ErrorCode` its
/// `BrainApiError` variant corresponds to. `NotFound`/`PermissionDenied`
/// both map to `INVALID_PARAMS` (not `RESOURCE_NOT_FOUND`): the frozen B5.0
/// `PermissionDenied`-vs-`NotFound` distinction (`brain-api`'s own
/// `error.rs` doc comment) is a *domain* distinction, not an MCP
/// transport-error-code distinction — `brain-api`'s own per-capability
/// code already decides, case by case, whether to collapse the two before
/// this mapping ever runs (e.g. `approve_proposal` checks permission
/// before loading, so an unauthorized caller never learns which one it
/// got); this function does not re-introduce a transport-level tell by
/// giving them different MCP codes.
pub fn map_brain_api_error(err: brain_api::BrainApiError) -> McpError {
    use brain_api::BrainApiError::*;
    let message = err.to_string();
    match err {
        InvalidInput(_) => McpError::invalid_params(message, None),
        NotFound | PermissionDenied => McpError::invalid_params(message, None),
        Conflict | UnsupportedOperation => McpError::invalid_params(message, None),
        RuntimeUnavailable | DenseUnavailable | ExternalToolUnavailable | Timeout | CorruptData => {
            McpError::internal_error(message, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_api::BrainApiError;
    use rmcp::model::ErrorCode;

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
    fn every_frozen_variant_maps_to_a_deterministic_mcp_error_code() {
        // Calling this twice for the same variant must produce the same
        // code every time -- the mapping is a pure function of the
        // variant, never randomized or state-dependent.
        for case in all_variants() {
            let first = map_brain_api_error(case.clone());
            let second = map_brain_api_error(case);
            assert_eq!(first.code, second.code);
        }
    }

    #[test]
    fn permission_denied_maps_to_a_typed_error_never_a_silent_empty_success() {
        let mapped = map_brain_api_error(BrainApiError::PermissionDenied);
        assert_eq!(mapped.code, ErrorCode::INVALID_PARAMS);
        assert!(mapped.message.contains("permission denied"));
    }

    #[test]
    fn not_found_maps_to_the_same_code_as_permission_denied_no_transport_level_tell() {
        let not_found = map_brain_api_error(BrainApiError::NotFound);
        let denied = map_brain_api_error(BrainApiError::PermissionDenied);
        assert_eq!(not_found.code, denied.code, "the MCP transport layer must not add a NEW distinguishing signal beyond whatever brain-api's own capability code already decided to reveal");
    }

    #[test]
    fn message_text_is_exactly_the_brain_api_error_display_text_no_extra_detail_appended() {
        let err = BrainApiError::InvalidInput("proposal expired".to_string());
        let expected = err.to_string();
        let mapped = map_brain_api_error(err);
        assert_eq!(mapped.message, expected);
    }

    #[test]
    fn no_mapped_message_contains_a_path_like_or_sql_like_substring() {
        for case in all_variants() {
            if matches!(case, BrainApiError::InvalidInput(_)) {
                continue; // caller-supplied payload text, not this module's own fixed mapping
            }
            let mapped = map_brain_api_error(case.clone());
            assert!(!mapped.message.contains('\\') && !mapped.message.contains('/'), "{case:?} mapped to a path-like message: {:?}", mapped.message);
            assert!(!mapped.message.to_uppercase().contains("SELECT"), "{case:?} mapped to a SQL-like message: {:?}", mapped.message);
        }
    }

    #[test]
    fn runtime_level_failures_map_to_internal_error_not_invalid_params() {
        for case in [
            BrainApiError::RuntimeUnavailable,
            BrainApiError::DenseUnavailable,
            BrainApiError::ExternalToolUnavailable,
            BrainApiError::Timeout,
            BrainApiError::CorruptData,
        ] {
            let mapped = map_brain_api_error(case);
            assert_eq!(mapped.code, ErrorCode::INTERNAL_ERROR);
        }
    }

    #[test]
    fn caller_input_level_failures_map_to_invalid_params_not_internal_error() {
        for case in [
            BrainApiError::NotFound,
            BrainApiError::InvalidInput("x".into()),
            BrainApiError::PermissionDenied,
            BrainApiError::Conflict,
            BrainApiError::UnsupportedOperation,
        ] {
            let mapped = map_brain_api_error(case);
            assert_eq!(mapped.code, ErrorCode::INVALID_PARAMS);
        }
    }
}
