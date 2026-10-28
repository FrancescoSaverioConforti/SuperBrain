//! Fault-injection seam for crash-simulation tests.
//!
//! A [`FaultInjector`] is threaded through write steps. When it is armed for a
//! point, [`FaultInjector::check`] returns `Err(Interrupted::At(point))` right
//! after that point's effect has happened, so a test can simulate a crash at
//! an exact step boundary without killing a process. A test can instead
//! *pause* a pipeline at a point (`FaultInjector::pause_at`) and act while it
//! is stopped there. Production code always uses [`FaultInjector::none`],
//! which never interrupts or pauses.

/// The crash-injectable boundaries in the write pipeline and in recovery.
///
/// Nominally `pub` (not `pub(crate)`) only because the public
/// `StorageError::Interrupted` variant carries it: a `pub(crate)` type in a
/// `pub` enum's field trips the `private_interfaces` lint under `-D warnings`.
/// This module is private and the type is not re-exported, so it stays
/// unnameable outside this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    /// `write_record_unchecked`: the intent row exists; nothing on disk changed.
    AfterIntentRecorded,
    /// `write_record_unchecked`: `record_path_checked`'s containment check has
    /// already run and returned a path it judged to resolve inside the vault
    /// root, but `atomic_write`'s own create/rename has not yet touched that
    /// path. Test-infrastructure-only seam (Task 9): a test can pause here
    /// and swap the path for a symlink during the window, to characterize
    /// whether SEC-001's check-then-use TOCTOU race still lands bytes through
    /// an attacker-controlled link. Never armed in production, which always
    /// passes `FaultInjector::none()`.
    AfterPathCheckedBeforeAtomicWrite,
    /// `write_record_unchecked`: the new record was renamed into place, but
    /// the row is still `IntentRecorded` (the `ContentPersisted` advance has
    /// not run). This is the spec's "Dopo il rename, prima di
    /// journal_committed" window at its earliest point. The row already
    /// carries the operation's intended `new_version`/`content_hash` (and
    /// `before_image_hash`), recorded before the rename, so recovery can tell
    /// a completed rename from one that never happened (final whole-branch
    /// review, finding C1).
    AfterRenameBeforeContentPersisted,
    /// `write_record_unchecked`: the record was renamed into place and the row
    /// advanced to `ContentPersisted` with its hash, before verification and
    /// before `JournalCommitted`.
    AfterRenameBeforeJournalCommitted,
    /// `write_record_unchecked`: the row is `JournalCommitted`; no git commit
    /// has been attempted.
    AfterJournalCommittedBeforeGit,
    /// `write_record_unchecked`: the row is `GitCommitted` (Oid recorded,
    /// before-image deleted); the index step has not run.
    AfterGitCommittedBeforeIndex,
    /// `recovery::reconcile` row 2: the before-image was written back over the
    /// record and verified, but the before-image is not yet deleted and the
    /// row is not yet `Failed`.
    DuringBeforeImageRestore,
    /// Checked inside `recovery::reconcile`, immediately after it marks a
    /// non-terminal row `Recovering` (recording `previous_state`) and before
    /// that row's fix-up runs. Lets a test leave a row genuinely AT
    /// `Recovering`, as a crash mid-recovery would. Only ever armed by tests
    /// (`trigger_at` is `#[cfg(test)]`).
    AfterMarkedRecoveringBeforeFixup,
}

/// Returned when an armed injector interrupts a pipeline at `point`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Interrupted {
    At(FaultPoint),
}

/// Decides whether a pipeline should stop at a given [`FaultPoint`].
///
/// Not `Clone`/`Copy`: the test-only pause variant owns channel endpoints.
/// Callers pass it by reference.
#[derive(Debug)]
pub(crate) struct FaultInjector {
    trigger: Option<FaultPoint>,
    #[cfg(test)]
    pause: Option<Pause>,
}

/// Test-only: where to pause, and the two channel ends used to tell the test
/// "paused" and to wait for "resume".
#[cfg(test)]
#[derive(Debug)]
struct Pause {
    point: FaultPoint,
    paused: std::sync::mpsc::Sender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}

impl FaultInjector {
    /// A no-op injector: no point ever interrupts.
    pub(crate) fn none() -> Self {
        Self {
            trigger: None,
            #[cfg(test)]
            pause: None,
        }
    }

    /// An injector that interrupts when the pipeline reaches `point`.
    #[cfg(test)]
    pub(crate) fn trigger_at(point: FaultPoint) -> Self {
        Self { trigger: Some(point), pause: None }
    }

    /// An injector that, when the pipeline reaches `point`, sends `()` on
    /// `paused` and then blocks until `()` arrives on `resume` (or the
    /// resume sender is dropped), and then lets the pipeline continue. It
    /// never returns `Interrupted`. Lets a test act deterministically while
    /// a pipeline is stopped at an exact step boundary, for example while a
    /// writer still holds its per-note lock.
    #[cfg(test)]
    pub(crate) fn pause_at(
        point: FaultPoint,
        paused: std::sync::mpsc::Sender<()>,
        resume: std::sync::mpsc::Receiver<()>,
    ) -> Self {
        Self { trigger: None, pause: Some(Pause { point, paused, resume }) }
    }

    /// Called by a pipeline immediately after `point`'s effect has happened.
    pub(crate) fn check(&self, point: FaultPoint) -> Result<(), Interrupted> {
        #[cfg(test)]
        if let Some(pause) = &self.pause {
            if pause.point == point {
                // A test that already went away is not an error here: the
                // pipeline just continues.
                let _ = pause.paused.send(());
                let _ = pause.resume.recv();
            }
        }
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
    fn pause_at_blocks_right_after_the_named_point_until_resumed_then_continues() {
        // Task 8: the pause variant stops the pipeline at the same boundary
        // trigger_at would, but instead of returning Interrupted it signals
        // the test and waits, then lets the pipeline run to completion.
        let (paused_tx, paused_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let injector = FaultInjector::pause_at(FaultPoint::AfterRenameBeforeJournalCommitted, paused_tx, resume_rx);
        let effects = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let pipeline_effects = std::sync::Arc::clone(&effects);
        let pipeline = std::thread::spawn(move || {
            let mut local = Vec::new();
            let step = |name: &'static str, local: &mut Vec<&'static str>| {
                local.push(name);
                pipeline_effects.lock().unwrap().push(name);
            };
            step("intent_recorded", &mut local);
            injector.check(FaultPoint::AfterIntentRecorded)?;
            step("content_persisted", &mut local);
            injector.check(FaultPoint::AfterRenameBeforeJournalCommitted)?;
            step("journal_committed", &mut local);
            injector.check(FaultPoint::AfterJournalCommittedBeforeGit)?;
            Ok::<_, Interrupted>(local)
        });

        paused_rx.recv().unwrap();
        assert_eq!(*effects.lock().unwrap(), vec!["intent_recorded", "content_persisted"], "paused right after the named point's effect");
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!pipeline.is_finished(), "the pipeline must stay paused until resumed");
        assert_eq!(effects.lock().unwrap().len(), 2, "no further step ran while paused");

        resume_tx.send(()).unwrap();
        let result = pipeline.join().unwrap();
        assert_eq!(result.unwrap(), vec!["intent_recorded", "content_persisted", "journal_committed"], "a paused point never interrupts: the pipeline completes");
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
