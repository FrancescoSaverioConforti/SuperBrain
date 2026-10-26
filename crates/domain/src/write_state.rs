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
    match (from, to) {
        (IntentRecorded, ContentPersisted) => true,
        (ContentPersisted, JournalCommitted) => true,
        (JournalCommitted, GitCommitted) => true,
        (GitCommitted, IndexCurrent) => true,
        (IntentRecorded, Failed) => true,
        (IntentRecorded | ContentPersisted | JournalCommitted | GitCommitted | IndexCurrent, Recovering) => true,
        (Recovering, IntentRecorded | ContentPersisted | JournalCommitted | GitCommitted | IndexCurrent) => true,
        _ => false,
    }
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
}
