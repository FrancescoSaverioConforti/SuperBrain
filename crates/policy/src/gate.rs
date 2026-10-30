use domain::{Classification, LinkError, LinkState, NoteMetadata, ValidationError};
use vault_security::{EncryptedStoreAvailability, StoreUnavailableReason};

use crate::secrets::SecretPolicy;

/// Outcome of `Gate::evaluate`. `Pass` deliberately carries no payload — no
/// "proof the gate ran" witness token. An earlier design iteration tried
/// that (so `storage` could require it at compile time before allowing a
/// write), but it required either a `storage<->policy` dependency cycle or
/// an unenforceable visibility scheme. Instead, `storage` depends on both
/// `domain` and `policy` and calls `Gate::evaluate` directly from inside its
/// own crate (Task 9); the "can't bypass the gate" guarantee comes from
/// `storage` having no other public write path, not from a type-level token.
#[derive(Debug)]
pub enum GateDecision {
    Pass,
    Reject(GateRejection),
}

#[derive(Debug)]
pub enum GateRejection {
    Schema(ValidationError),
    Link(LinkError),
    SecretClassification,
    SensitiveRequiresEncryptedStore {
        note_id: String,
        reason: StoreUnavailableReason,
    },
    SecretContentDetected { note_id: String, pattern_id: String },
}

pub struct Gate {
    secret_policy: SecretPolicy,
}

impl Gate {
    pub fn new(secret_policy: SecretPolicy) -> Self {
        Self { secret_policy }
    }

    /// For the case where `parse_frontmatter` already failed before a
    /// `NoteMetadata` exists. `classification: secret` is intercepted at
    /// parse time (Task 2) and never becomes a valid in-memory value, so the
    /// gate surfaces it here as `GateRejection::SecretClassification`.
    pub fn evaluate_schema_error(&self, err: ValidationError) -> GateDecision {
        match err {
            ValidationError::SecretClassificationForbidden => {
                GateDecision::Reject(GateRejection::SecretClassification)
            }
            other => GateDecision::Reject(GateRejection::Schema(other)),
        }
    }

    /// Composes schema/link/classification checks and the secret-content
    /// scanner into one pass/reject decision. Short-circuits in this exact
    /// order: schema error -> link error -> classification/availability
    /// (Sensitive) -> secret content scan (ignoring only findings marked
    /// `excepted: true`) -> Pass.
    ///
    /// The `store` parameter is a plain, zero-I/O description of whether an
    /// encrypted store is available for this call — the caller (`storage`,
    /// Task 6) is responsible for having already tried to open the store
    /// elsewhere; this method never performs I/O itself. A `Sensitive` note
    /// is rejected only when `store` is `Unavailable`; this is NOT a
    /// bypass of the gate's authority — the gate still has the final say
    /// on every call, and when the store is available a `Sensitive` note
    /// falls through to the exact same secret-content scan every other
    /// classification already goes through (it is never skipped).
    pub fn evaluate(
        &self,
        note: &NoteMetadata,
        content: &str,
        link_state_result: Result<LinkState, LinkError>,
        store: EncryptedStoreAvailability,
    ) -> GateDecision {
        // Schema errors are handled by `evaluate_schema_error` before a
        // `NoteMetadata` exists at all (see that method), so by the time
        // `evaluate` runs, the schema step has already passed. Link error
        // comes next in the short-circuit order.
        if let Err(link_err) = link_state_result {
            return GateDecision::Reject(GateRejection::Link(link_err));
        }

        if matches!(note.classification, Classification::Sensitive) {
            if let EncryptedStoreAvailability::Unavailable { reason } = store {
                return GateDecision::Reject(GateRejection::SensitiveRequiresEncryptedStore {
                    note_id: note.id.clone(),
                    reason,
                });
            }
            // Available: fall through to the same secret-content scan
            // every other classification already goes through below.
        }

        let findings = self.secret_policy.scan(content, &note.id);
        if let Some(finding) = findings.iter().find(|f| !f.excepted) {
            return GateDecision::Reject(GateRejection::SecretContentDetected {
                note_id: note.id.clone(),
                pattern_id: finding.pattern_id.clone(),
            });
        }

        GateDecision::Pass
    }
}

#[cfg(test)]
mod tests {
    use domain::{NoteStatus, NoteType, Relations, TrustLevel};

    use super::*;
    use crate::secrets::{sha256_hex, SecretException};
    use vault_security::{EncryptedStoreAvailability, StoreUnavailableReason};

    fn valid_note_with(classification: Classification) -> NoteMetadata {
        NoteMetadata {
            id: "bug-2026-10-23-fastapi-cors".to_string(),
            schema_version: 1,
            r#type: NoteType::Bug,
            status: NoteStatus::Draft,
            trust: TrustLevel::Internal,
            classification,
            project: "project-x".to_string(),
            source: "agent:coder@prompt-v3".to_string(),
            created: "2026-10-23".to_string(),
            updated: "2026-10-23".to_string(),
            event_period: "2026-10-23".to_string(),
            relations: Relations::default(),
        }
    }

    #[test]
    fn note_with_no_relations_passes_gate_on_that_basis_alone() {
        let note = valid_note_with(Classification::Internal);
        let gate = Gate::new(SecretPolicy::new(1));
        let decision = gate.evaluate(
            &note,
            "plain content",
            Ok(LinkState::Unlinked),
            EncryptedStoreAvailability::Unavailable {
                reason: StoreUnavailableReason::NotYetConfigured,
            },
        );
        assert!(matches!(decision, GateDecision::Pass));
    }

    #[test]
    fn sensitive_classification_is_rejected_pending_encrypted_store() {
        let note = valid_note_with(Classification::Sensitive);
        let gate = Gate::new(SecretPolicy::new(1));
        let decision = gate.evaluate(
            &note,
            "content",
            Ok(LinkState::Unlinked),
            EncryptedStoreAvailability::Unavailable {
                reason: StoreUnavailableReason::NotYetConfigured,
            },
        );
        assert!(matches!(
            decision,
            GateDecision::Reject(GateRejection::SensitiveRequiresEncryptedStore { .. })
        ));
    }

    #[test]
    fn real_secret_pattern_is_rejected() {
        let note = valid_note_with(Classification::Internal);
        let gate = Gate::new(SecretPolicy::new(1));
        let content = "aws_key = AKIAABCDEFGHIJKLMNOP";
        let decision = gate.evaluate(
            &note,
            content,
            Ok(LinkState::Unlinked),
            EncryptedStoreAvailability::Unavailable {
                reason: StoreUnavailableReason::NotYetConfigured,
            },
        );
        assert!(matches!(
            decision,
            GateDecision::Reject(GateRejection::SecretContentDetected { .. })
        ));
    }

    #[test]
    fn fictitious_example_secret_in_prose_without_matching_pattern_is_not_flagged() {
        let note = valid_note_with(Classification::Internal);
        let gate = Gate::new(SecretPolicy::new(1));
        let content =
            "Example: never commit a key like AKIAEXAMPLE1234567 (fake, 19 chars, fails the 16-char pattern)";
        let decision = gate.evaluate(
            &note,
            content,
            Ok(LinkState::Unlinked),
            EncryptedStoreAvailability::Unavailable {
                reason: StoreUnavailableReason::NotYetConfigured,
            },
        );
        assert!(matches!(decision, GateDecision::Pass));
    }

    #[test]
    fn false_positive_with_registered_exception_for_that_exact_occurrence_passes() {
        let note = valid_note_with(Classification::Internal);
        let content = "aws_key = AKIAABCDEFGHIJKLMNOP";
        let matched_text_hash = sha256_hex("AKIAABCDEFGHIJKLMNOP");
        let exception = SecretException {
            pattern_id: "aws-access-key".into(),
            matched_text_hash,
            reason: "documentation placeholder, verified non-functional".into(),
            approved_by: "francesco".into(),
            note_id: note.id.clone(),
        };
        let policy = SecretPolicy::new(1).with_exception(exception);
        let gate = Gate::new(policy);
        let decision = gate.evaluate(
            &note,
            content,
            Ok(LinkState::Unlinked),
            EncryptedStoreAvailability::Unavailable {
                reason: StoreUnavailableReason::NotYetConfigured,
            },
        );
        assert!(matches!(decision, GateDecision::Pass));
    }

    #[test]
    fn exception_for_one_occurrence_does_not_cover_a_different_matched_value() {
        let note = valid_note_with(Classification::Internal);
        let excepted_occurrence_hash = sha256_hex("AKIAABCDEFGHIJKLMNOP");
        let exception = SecretException {
            pattern_id: "aws-access-key".into(),
            matched_text_hash: excepted_occurrence_hash,
            reason: "documentation placeholder, verified non-functional".into(),
            approved_by: "francesco".into(),
            note_id: note.id.clone(),
        };
        let policy = SecretPolicy::new(1).with_exception(exception);
        let gate = Gate::new(policy);
        // a different AWS-shaped key, same pattern_id, same note — must still be caught
        let content = "aws_key = AKIAZZZZZZZZZZZZZZZZ";
        let decision = gate.evaluate(
            &note,
            content,
            Ok(LinkState::Unlinked),
            EncryptedStoreAvailability::Unavailable {
                reason: StoreUnavailableReason::NotYetConfigured,
            },
        );
        assert!(matches!(
            decision,
            GateDecision::Reject(GateRejection::SecretContentDetected { .. })
        ));
    }

    #[test]
    fn sensitive_note_is_rejected_when_the_store_is_unavailable_exactly_as_before() {
        // Same assertion as the old A1 test
        // (sensitive_classification_is_rejected_pending_encrypted_store),
        // but now explicit about which input produces it: an Unavailable
        // store still unconditionally rejects a Sensitive note.
        let note = valid_note_with(Classification::Sensitive);
        let gate = Gate::new(SecretPolicy::new(1));
        let decision = gate.evaluate(
            &note,
            "content",
            Ok(LinkState::Unlinked),
            EncryptedStoreAvailability::Unavailable {
                reason: StoreUnavailableReason::NotYetConfigured,
            },
        );
        assert!(matches!(
            decision,
            GateDecision::Reject(GateRejection::SensitiveRequiresEncryptedStore { .. })
        ));
    }

    #[test]
    fn sensitive_note_passes_the_classification_step_when_the_store_is_available_and_still_runs_the_secret_content_scan(
    ) {
        let note = valid_note_with(Classification::Sensitive);
        let gate = Gate::new(SecretPolicy::new(1));

        // Clean sensitive content reaches Pass when the store is available.
        let clean_decision = gate.evaluate(
            &note,
            "plain sensitive content, no secrets",
            Ok(LinkState::Unlinked),
            EncryptedStoreAvailability::Available,
        );
        assert!(matches!(clean_decision, GateDecision::Pass));

        // Secret-shaped content in a sensitive note is still caught — the
        // classification step passing must not skip the secret scanner.
        let secret_content = "aws_key = AKIAABCDEFGHIJKLMNOP";
        let secret_decision = gate.evaluate(
            &note,
            secret_content,
            Ok(LinkState::Unlinked),
            EncryptedStoreAvailability::Available,
        );
        assert!(matches!(
            secret_decision,
            GateDecision::Reject(GateRejection::SecretContentDetected { .. })
        ));
    }

    #[test]
    fn secret_classification_is_rejected_before_content_is_even_scanned() {
        // Per Task 2, parse_frontmatter already converts `classification:
        // secret` to ValidationError::SecretClassificationForbidden before a
        // NoteMetadata exists, so this verifies Gate::evaluate_schema_error
        // surfaces GateRejection::SecretClassification when fed that error
        // via the schema path.
        let decision = Gate::new(SecretPolicy::new(1))
            .evaluate_schema_error(ValidationError::SecretClassificationForbidden);
        assert!(matches!(
            decision,
            GateDecision::Reject(GateRejection::SecretClassification)
        ));
    }
}
