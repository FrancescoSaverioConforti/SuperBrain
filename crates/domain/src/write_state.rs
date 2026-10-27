#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteState {
    IntentRecorded,
    ContentPersisted,
    JournalCommitted,
    GitCommitted,
    IndexCurrent,
    Recovering,
    Failed,
}

pub fn valid_transition(from: WriteState, to: WriteState) -> bool {
    use WriteState::*;
    matches!(
        (from, to),
        (IntentRecorded, ContentPersisted)
            | (ContentPersisted, JournalCommitted)
            | (JournalCommitted, GitCommitted)
            | (GitCommitted, IndexCurrent)
            | (IntentRecorded, Failed)
            | (ContentPersisted, Failed)
            | (IntentRecorded | ContentPersisted | JournalCommitted | GitCommitted | IndexCurrent, Recovering)
            // Recovering's outgoing set is narrowed to C1's crash-recovery table [c10b3, closed by A2].
            // Recovering -> IntentRecorded is removed: C1 has no row where a crash recovers back to
            // IntentRecorded; IntentRecorded's only crash outcome is a direct roll-back to Failed.
            // Recovering -> Failed is kept: reconcile marks a ContentPersisted row Recovering before
            // its rollback fix-up, so Failed is a legitimate resolved outcome of Recovering.
            // The incoming X -> Recovering edges above are deliberately unchanged and are pinned by
            // the test `every_non_failed_state_can_still_enter_recovering_after_the_a2_narrowing`.
            | (Recovering, ContentPersisted | JournalCommitted | GitCommitted | IndexCurrent | Failed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_recorded_can_advance_to_content_persisted() {
        assert!(valid_transition(WriteState::IntentRecorded, WriteState::ContentPersisted));
    }

    #[test]
    fn intent_recorded_can_fail() {
        assert!(valid_transition(WriteState::IntentRecorded, WriteState::Failed));
    }

    #[test]
    fn failed_is_terminal() {
        assert!(!valid_transition(WriteState::Failed, WriteState::IntentRecorded));
        assert!(!valid_transition(WriteState::Failed, WriteState::Recovering));
    }

    #[test]
    fn cannot_skip_journal_committed_to_index_current() {
        assert!(!valid_transition(WriteState::ContentPersisted, WriteState::IndexCurrent));
    }

    #[test]
    fn git_committed_can_reach_index_current() {
        assert!(valid_transition(WriteState::GitCommitted, WriteState::IndexCurrent));
    }

    #[test]
    fn recovering_can_resolve_to_failed_via_a_rollback() {
        // Recovering -> Failed is the real sequence reconcile produces when a
        // ContentPersisted row is rolled back: it is marked Recovering first,
        // then resolved to Failed. This is reaching Failed FROM Recovering,
        // not skipping Recovering altogether.
        assert!(valid_transition(WriteState::Recovering, WriteState::Failed));
    }

    #[test]
    fn content_persisted_can_fail_via_before_image_rollback() {
        // A write whose new content fails hash verification and is rolled back
        // to its before-image must end as Failed, not JournalCommitted.
        assert!(valid_transition(WriteState::ContentPersisted, WriteState::Failed));
    }

    #[test]
    fn recovering_cannot_roll_back_to_intent_recorded() {
        assert!(!valid_transition(WriteState::Recovering, WriteState::IntentRecorded));
    }

    #[test]
    fn recovering_can_still_roll_forward_to_journal_committed() {
        assert!(valid_transition(WriteState::Recovering, WriteState::JournalCommitted));
    }

    #[test]
    fn recovering_can_still_roll_forward_to_git_committed() {
        assert!(valid_transition(WriteState::Recovering, WriteState::GitCommitted));
    }

    #[test]
    fn recovering_can_still_reach_index_current() {
        assert!(valid_transition(WriteState::Recovering, WriteState::IndexCurrent));
    }

    #[test]
    fn intent_recorded_can_still_fail_directly() {
        assert!(valid_transition(WriteState::IntentRecorded, WriteState::Failed));
    }

    #[test]
    fn every_non_failed_state_can_still_enter_recovering_after_the_a2_narrowing() {
        // Pins the incoming X -> Recovering edges. Narrowing Recovering's
        // OUTGOING edges must not remove any of these.
        for from in [
            WriteState::IntentRecorded,
            WriteState::ContentPersisted,
            WriteState::JournalCommitted,
            WriteState::GitCommitted,
            WriteState::IndexCurrent,
        ] {
            assert!(valid_transition(from, WriteState::Recovering), "{from:?} -> Recovering must still be valid");
        }
    }
}
