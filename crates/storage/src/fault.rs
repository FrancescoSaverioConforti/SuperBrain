//! Fault-injection seam for crash-simulation tests.
//!
//! A [`FaultInjector`] is threaded through write steps. When it is armed for a
//! point, [`FaultInjector::check`] returns `Err(Interrupted::At(point))` right
//! after that point's effect has happened, so a test can simulate a crash at
//! an exact step boundary without killing a process. Production code always
//! uses [`FaultInjector::none`], which never interrupts.

/// The crash-injectable boundaries in the write pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FaultPoint {
    AfterIntentRecorded,
    AfterRenameBeforeJournalCommitted,
    // The remaining three variants have no production call site until Task
    // 5 (git) and Task 6 (index) extend write_record_unchecked's chain past
    // JournalCommitted, and Task 7's before-image rollback during recovery.
    #[allow(dead_code)]
    AfterJournalCommittedBeforeGit,
    #[allow(dead_code)]
    AfterGitCommittedBeforeIndex,
    #[allow(dead_code)]
    DuringBeforeImageRestore,
}

/// Returned when an armed injector interrupts a pipeline at `point`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Interrupted {
    At(FaultPoint),
}

/// Decides whether a pipeline should stop at a given [`FaultPoint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FaultInjector {
    trigger: Option<FaultPoint>,
}

impl FaultInjector {
    /// A no-op injector: no point ever interrupts.
    pub(crate) fn none() -> Self {
        Self { trigger: None }
    }

    /// An injector that interrupts when the pipeline reaches `point`.
    #[cfg(test)]
    pub(crate) fn trigger_at(point: FaultPoint) -> Self {
        Self { trigger: Some(point) }
    }

    /// Called by a pipeline immediately after `point`'s effect has happened.
    pub(crate) fn check(&self, point: FaultPoint) -> Result<(), Interrupted> {
        if self.trigger == Some(point) {
            Err(Interrupted::At(point))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test-local three-step synthetic pipeline. Each step records its effect,
    // then checks the injector at its named point before proceeding.
    fn run_synthetic_pipeline(injector: &FaultInjector, effects: &mut Vec<&'static str>) -> Result<(), Interrupted> {
        effects.push("intent_recorded");
        injector.check(FaultPoint::AfterIntentRecorded)?;
        effects.push("content_persisted");
        injector.check(FaultPoint::AfterRenameBeforeJournalCommitted)?;
        effects.push("journal_committed");
        injector.check(FaultPoint::AfterJournalCommittedBeforeGit)?;
        Ok(())
    }

    #[test]
    fn trigger_at_interrupts_immediately_after_the_named_point_not_before() {
        let injector = FaultInjector::trigger_at(FaultPoint::AfterIntentRecorded);
        let mut effects = vec![];
        let result = run_synthetic_pipeline(&injector, &mut effects);
        // the effect belonging to AfterIntentRecorded happened...
        assert_eq!(effects, vec!["intent_recorded"]);
        // ...but the pipeline stopped there, distinguishably, instead of continuing
        assert!(matches!(result, Err(Interrupted::At(FaultPoint::AfterIntentRecorded))));
    }

    #[test]
    fn none_injector_runs_the_whole_pipeline_uninterrupted() {
        let injector = FaultInjector::none();
        let mut effects = vec![];
        let result = run_synthetic_pipeline(&injector, &mut effects);
        assert_eq!(effects, vec!["intent_recorded", "content_persisted", "journal_committed"]);
        assert!(result.is_ok());
    }
}
