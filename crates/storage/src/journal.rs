//! SQLite-backed write-operation journal.
//!
//! One row per write operation, keyed by `operation_id`. `Journal` is a pure
//! persistence and query layer: it enforces `domain::valid_transition` on every
//! state change, and knows nothing about the vault, the filesystem, or git.
//! It is crate-private; `propose_and_commit` remains the only public write entry point.

use std::path::Path;

use domain::{valid_transition, WriteState};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS operations (
    operation_id TEXT PRIMARY KEY,
    note_id TEXT NOT NULL,
    state TEXT NOT NULL,
    expected_version INTEGER NOT NULL,
    new_version INTEGER,
    content_hash TEXT,
    before_image_hash TEXT,
    before_image_path TEXT,
    git_commit_oid TEXT,
    previous_state TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
)";

// Columns in the order `row_from_sql` reads them.
const SELECT_ROWS: &str = "SELECT operation_id, note_id, state, expected_version, new_version, \
    content_hash, before_image_hash, before_image_path, git_commit_oid, previous_state FROM operations";

// SQLite-side UTC timestamp with millisecond precision; keeps the journal free of a clock dependency.
const NOW_SQL: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";

#[derive(Debug, thiserror::Error)]
pub(crate) enum JournalError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid journal state: {0}")]
    InvalidState(String),
}

/// One journal row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OperationRow {
    pub(crate) operation_id: Uuid,
    pub(crate) note_id: String,
    pub(crate) state: WriteState,
    pub(crate) expected_version: u64,
    pub(crate) new_version: Option<u64>,
    pub(crate) content_hash: Option<String>,
    pub(crate) before_image_hash: Option<String>,
    pub(crate) before_image_path: Option<String>,
    pub(crate) git_commit_oid: Option<String>,
    /// The state this row was in when `recovery::reconcile` marked it
    /// `Recovering`. Written on the SAME journal write that moves the row to
    /// `Recovering`, so a row found at `Recovering` (a crash interrupted a
    /// previous reconcile pass) always says which fault-matrix row to resume
    /// from. Not cleared once the row resolves — it stays as an audit trail.
    /// `None` for a row that was never marked `Recovering`.
    pub(crate) previous_state: Option<WriteState>,
}

/// Optional columns that `Journal::advance` may set alongside a state change.
/// `None` fields are left unchanged in the stored row.
#[derive(Debug, Default, Clone)]
pub(crate) struct AdvanceFields {
    new_version: Option<u64>,
    content_hash: Option<String>,
    before_image_hash: Option<String>,
    before_image_path: Option<String>,
    git_commit_oid: Option<String>,
    previous_state: Option<WriteState>,
}

impl AdvanceFields {
    pub(crate) fn none() -> Self {
        Self::default()
    }

    pub(crate) fn content(new_version: u64, content_hash: impl Into<String>) -> Self {
        Self {
            new_version: Some(new_version),
            content_hash: Some(content_hash.into()),
            ..Self::default()
        }
    }

    pub(crate) fn content_with_before_image(
        new_version: u64,
        content_hash: impl Into<String>,
        before_image_hash: impl Into<String>,
    ) -> Self {
        Self {
            before_image_hash: Some(before_image_hash.into()),
            ..Self::content(new_version, content_hash)
        }
    }

    /// Records the git commit `Oid` for this operation on the SAME journal
    /// write that advances its state to `GitCommitted` — not a separate
    /// write, so there is no window where the state says `GitCommitted` but
    /// `git_commit_oid` is still unset (Task 5's chain-extension invariant).
    pub(crate) fn git_oid(oid: git2::Oid) -> Self {
        Self {
            git_commit_oid: Some(oid.to_string()),
            ..Self::default()
        }
    }

    /// Records the state a row is leaving when `recovery::reconcile` moves it
    /// to `Recovering`, on the SAME journal write as that state change — so
    /// there is no window where a row is at `Recovering` without knowing what
    /// to resume from.
    pub(crate) fn previous_state(state: WriteState) -> Self {
        Self {
            previous_state: Some(state),
            ..Self::default()
        }
    }
}

pub(crate) struct Journal {
    conn: Connection,
}

impl Journal {
    pub(crate) fn open(path: &Path) -> Result<Self, JournalError> {
        Self::from_connection(Connection::open(path)?)
    }

    #[cfg(test)]
    pub(crate) fn open_in_memory() -> Result<Self, JournalError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, JournalError> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub(crate) fn insert_intent(
        &self,
        operation_id: Uuid,
        note_id: &str,
        expected_version: u64,
    ) -> Result<(), JournalError> {
        self.conn.execute(
            &format!(
                "INSERT INTO operations (operation_id, note_id, state, expected_version, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, {NOW_SQL}, {NOW_SQL})"
            ),
            params![
                operation_id.to_string(),
                note_id,
                state_to_sql(WriteState::IntentRecorded),
                expected_version as i64,
            ],
        )?;
        Ok(())
    }

    /// Moves an operation to `to`, after checking `domain::valid_transition` against its current state.
    pub(crate) fn advance(
        &self,
        operation_id: Uuid,
        to: WriteState,
        fields: AdvanceFields,
    ) -> Result<(), JournalError> {
        let current = self.get(operation_id)?.ok_or_else(|| {
            JournalError::InvalidState(format!("no journal row for operation {operation_id}"))
        })?;
        if !valid_transition(current.state, to) {
            return Err(JournalError::InvalidState(format!(
                "{:?} -> {:?} is not a legal transition for operation {operation_id}",
                current.state, to
            )));
        }
        self.update_row(operation_id, current.state, to, fields)
            .map_err(|e| match e {
                JournalError::InvalidState(_) => JournalError::InvalidState(format!(
                    "operation {operation_id} changed state concurrently; advance to {to:?} not applied"
                )),
                other => other,
            })
    }

    /// Records `fields` on a row that is at `at`, WITHOUT changing its state.
    ///
    /// Used by the write path to persist an operation's intended
    /// `new_version`, `content_hash` and `before_image_hash` on its
    /// still-`IntentRecorded` row BEFORE the rename that lands the new bytes
    /// (final whole-branch review, finding C1). A crash between that rename
    /// and the `ContentPersisted` advance then leaves a row recovery can
    /// verify against the record on disk, instead of one indistinguishable
    /// from "nothing was ever written". No transition happens, so
    /// `valid_transition` is not consulted; the `state = at` guard still
    /// refuses the write if the row has moved on.
    pub(crate) fn record_fields(
        &self,
        operation_id: Uuid,
        at: WriteState,
        fields: AdvanceFields,
    ) -> Result<(), JournalError> {
        self.update_row(operation_id, at, at, fields).map_err(|e| match e {
            JournalError::InvalidState(_) => JournalError::InvalidState(format!(
                "operation {operation_id} is not at {at:?}; fields not recorded"
            )),
            other => other,
        })
    }

    /// The single UPDATE behind `advance` and `record_fields`: sets the state
    /// to `to` and every `Some` field in `fields`, only if the row is still at
    /// `from`. Returns `InvalidState` if no row matched.
    fn update_row(
        &self,
        operation_id: Uuid,
        from: WriteState,
        to: WriteState,
        fields: AdvanceFields,
    ) -> Result<(), JournalError> {
        // The `state = ?8` guard makes the update a no-op if another writer moved the row first.
        let updated = self.conn.execute(
            &format!(
                "UPDATE operations SET \
                    state = ?2, \
                    new_version = COALESCE(?3, new_version), \
                    content_hash = COALESCE(?4, content_hash), \
                    before_image_hash = COALESCE(?5, before_image_hash), \
                    before_image_path = COALESCE(?6, before_image_path), \
                    git_commit_oid = COALESCE(?7, git_commit_oid), \
                    previous_state = COALESCE(?9, previous_state), \
                    updated_at = {NOW_SQL} \
                 WHERE operation_id = ?1 AND state = ?8"
            ),
            params![
                operation_id.to_string(),
                state_to_sql(to),
                fields.new_version.map(|v| v as i64),
                fields.content_hash,
                fields.before_image_hash,
                fields.before_image_path,
                fields.git_commit_oid,
                state_to_sql(from),
                fields.previous_state.map(state_to_sql),
            ],
        )?;
        if updated != 1 {
            return Err(JournalError::InvalidState(format!(
                "operation {operation_id} is not at {from:?}"
            )));
        }
        Ok(())
    }

    pub(crate) fn get(&self, operation_id: Uuid) -> Result<Option<OperationRow>, JournalError> {
        let row = self
            .conn
            .query_row(
                &format!("{SELECT_ROWS} WHERE operation_id = ?1"),
                params![operation_id.to_string()],
                row_from_sql,
            )
            .optional()?;
        Ok(row)
    }

    /// Most recent row for `note_id` at `JournalCommitted` or later. Rows still at
    /// `IntentRecorded`/`ContentPersisted`/`Recovering` are ignored.
    ///
    /// Ordered by `new_version DESC`, not `rowid DESC` (storage Task 4
    /// review, finding I1): `rowid` reflects `insert_intent` call order,
    /// which happens BEFORE the per-note lock in `Vault::write_record_unchecked`
    /// and is therefore not serialized across concurrent callers for the
    /// same note. Two operations can call `insert_intent` in one order but
    /// acquire the per-note lock — and so actually commit — in the other
    /// order; `new_version` is assigned strictly `+1` per note under that
    /// lock regardless of `insert_intent` timing, so it is the real
    /// linearization order and `rowid` is not a reliable proxy for it.
    pub(crate) fn last_committed_for_note(
        &self,
        note_id: &str,
    ) -> Result<Option<OperationRow>, JournalError> {
        let row = self
            .conn
            .query_row(
                &format!(
                    "{SELECT_ROWS} WHERE note_id = ?1 AND state IN (?2, ?3, ?4) ORDER BY new_version DESC LIMIT 1"
                ),
                params![
                    note_id,
                    state_to_sql(WriteState::JournalCommitted),
                    state_to_sql(WriteState::GitCommitted),
                    state_to_sql(WriteState::IndexCurrent),
                ],
                row_from_sql,
            )
            .optional()?;
        Ok(row)
    }

    /// Every operation on `note_id` whose recorded `before_image_hash` equals
    /// `before_image_hash`, in ascending `new_version` order. These are the
    /// operations that overwrote content with that hash. Used by
    /// `recovery::own_content` to find a superseded operation's own bytes in
    /// its successor's before-image (Task 7 fix round, finding I-2). Any
    /// match whose before-image verifies is equally good, so the order is
    /// not load-bearing here; it is `new_version`, not `rowid`, only for
    /// consistency with the other per-note queries (final whole-branch
    /// review, M2).
    pub(crate) fn operations_with_before_image_hash(
        &self,
        note_id: &str,
        before_image_hash: &str,
    ) -> Result<Vec<OperationRow>, JournalError> {
        let mut stmt = self.conn.prepare(&format!(
            "{SELECT_ROWS} WHERE note_id = ?1 AND before_image_hash = ?2 ORDER BY new_version ASC, rowid ASC"
        ))?;
        let rows = stmt.query_map(params![note_id, before_image_hash], row_from_sql)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Every operation that is neither `IndexCurrent` nor `Failed`, in
    /// ascending `new_version` order (rows with no `new_version` yet first,
    /// then `rowid` as the tie-break). Consumed by `recovery::reconcile` on
    /// `Vault::open`.
    ///
    /// Not `rowid` order (final whole-branch review, M2): `rowid` is
    /// `insert_intent` order, which runs BEFORE the per-note lock, so a
    /// later-committing operation on a note can have the lower `rowid` (the
    /// same reason `last_committed_for_note` orders by `new_version`, Task 4
    /// I1). Within one note, recovery must resolve operations oldest version
    /// first: a git-pending operation's own bytes may survive only in its
    /// successor's before-image, which the successor deletes once it
    /// commits, so committing the successor first would strand the earlier
    /// operation (finding I1). Across different notes the order is
    /// irrelevant.
    pub(crate) fn non_terminal_operations(&self) -> Result<Vec<OperationRow>, JournalError> {
        let mut stmt = self.conn.prepare(&format!(
            "{SELECT_ROWS} WHERE state NOT IN (?1, ?2) ORDER BY new_version ASC, rowid ASC"
        ))?;
        let rows = stmt.query_map(
            params![
                state_to_sql(WriteState::IndexCurrent),
                state_to_sql(WriteState::Failed),
            ],
            row_from_sql,
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Every operation on `note_id` resting at `JournalCommitted` (git
    /// pending) with a `new_version` below `below_version`, in ascending
    /// `new_version` order. The git step commits these before an operation
    /// at `below_version`, so that operation never deletes a before-image an
    /// earlier pending operation still needs (final whole-branch review,
    /// finding I1).
    pub(crate) fn git_pending_operations_before(
        &self,
        note_id: &str,
        below_version: u64,
    ) -> Result<Vec<OperationRow>, JournalError> {
        let mut stmt = self.conn.prepare(&format!(
            "{SELECT_ROWS} WHERE note_id = ?1 AND state = ?2 AND new_version < ?3 ORDER BY new_version ASC"
        ))?;
        let rows = stmt.query_map(
            params![note_id, state_to_sql(WriteState::JournalCommitted), below_version as i64],
            row_from_sql,
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

// WriteState is stored as an explicit snake_case SQL string via this manual pair, not its Debug
// output, so the on-disk format survives a variant rename. The round-trip is pinned by the tests.
fn state_to_sql(state: WriteState) -> &'static str {
    match state {
        WriteState::IntentRecorded => "intent_recorded",
        WriteState::ContentPersisted => "content_persisted",
        WriteState::JournalCommitted => "journal_committed",
        WriteState::GitCommitted => "git_committed",
        WriteState::IndexCurrent => "index_current",
        WriteState::Recovering => "recovering",
        WriteState::Failed => "failed",
    }
}

fn state_from_sql(s: &str) -> Result<WriteState, String> {
    Ok(match s {
        "intent_recorded" => WriteState::IntentRecorded,
        "content_persisted" => WriteState::ContentPersisted,
        "journal_committed" => WriteState::JournalCommitted,
        "git_committed" => WriteState::GitCommitted,
        "index_current" => WriteState::IndexCurrent,
        "recovering" => WriteState::Recovering,
        "failed" => WriteState::Failed,
        other => return Err(format!("unknown write state {other:?}")),
    })
}

fn conversion_error(index: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, message.into())
}

fn row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<OperationRow> {
    let operation_id: String = row.get(0)?;
    let state: String = row.get(2)?;
    Ok(OperationRow {
        operation_id: Uuid::parse_str(&operation_id)
            .map_err(|e| conversion_error(0, e.to_string()))?,
        note_id: row.get(1)?,
        state: state_from_sql(&state).map_err(|e| conversion_error(2, e))?,
        expected_version: row.get::<_, i64>(3)? as u64,
        new_version: row.get::<_, Option<i64>>(4)?.map(|v| v as u64),
        content_hash: row.get(5)?,
        before_image_hash: row.get(6)?,
        before_image_path: row.get(7)?,
        git_commit_oid: row.get(8)?,
        previous_state: row
            .get::<_, Option<String>>(9)?
            .map(|s| state_from_sql(&s).map_err(|e| conversion_error(9, e)))
            .transpose()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_intent_then_get_returns_intent_recorded_row() {
        let journal = Journal::open_in_memory().unwrap();
        let op = Uuid::new_v4();
        journal.insert_intent(op, "note-a", 0).unwrap();
        let row = journal.get(op).unwrap().unwrap();
        assert_eq!(row.state, WriteState::IntentRecorded);
        assert_eq!(row.note_id, "note-a");
        assert_eq!(row.expected_version, 0);
    }

    #[test]
    fn insert_intent_with_a_duplicate_operation_id_is_rejected_cleanly_not_a_panic() {
        // Task 8: the second, SQL-level guard behind write_record_unchecked's
        // own "operation_id already exists" check.
        let journal = Journal::open_in_memory().unwrap();
        let op = Uuid::new_v4();
        journal.insert_intent(op, "note-a", 0).unwrap();
        journal.advance(op, WriteState::Failed, AdvanceFields::none()).unwrap();
        let err = journal.insert_intent(op, "note-a", 0).unwrap_err();
        assert!(matches!(err, JournalError::Sqlite(_)), "got {err:?}");
        assert_eq!(journal.get(op).unwrap().unwrap().state, WriteState::Failed, "the existing row is untouched");
    }

    #[test]
    fn advance_through_the_full_chain_updates_state_each_time() {
        let journal = Journal::open_in_memory().unwrap();
        let op = Uuid::new_v4();
        journal.insert_intent(op, "note-a", 0).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, "hash1")).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op, WriteState::GitCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op, WriteState::IndexCurrent, AdvanceFields::none()).unwrap();
        let row = journal.get(op).unwrap().unwrap();
        assert_eq!(row.state, WriteState::IndexCurrent);
        assert_eq!(row.new_version, Some(1));
    }

    #[test]
    fn record_fields_sets_fields_without_changing_state_and_only_at_the_named_state() {
        let journal = Journal::open_in_memory().unwrap();
        let op = Uuid::new_v4();
        journal.insert_intent(op, "note-a", 0).unwrap();
        journal.record_fields(op, WriteState::IntentRecorded, AdvanceFields::content_with_before_image(1, "h", "b")).unwrap();
        let row = journal.get(op).unwrap().unwrap();
        assert_eq!(row.state, WriteState::IntentRecorded, "no transition");
        assert_eq!((row.new_version, row.content_hash.as_deref(), row.before_image_hash.as_deref()), (Some(1), Some("h"), Some("b")));
        let err = journal.record_fields(op, WriteState::ContentPersisted, AdvanceFields::content(9, "x")).unwrap_err();
        assert!(matches!(err, JournalError::InvalidState(_)), "a row not at the named state is refused");
        assert_eq!(journal.get(op).unwrap().unwrap().new_version, Some(1), "and left unchanged");
    }

    #[test]
    fn advance_to_an_illegal_target_state_is_rejected() {
        let journal = Journal::open_in_memory().unwrap();
        let op = Uuid::new_v4();
        journal.insert_intent(op, "note-a", 0).unwrap();
        // IntentRecorded -> IndexCurrent skips the whole chain
        let err = journal.advance(op, WriteState::IndexCurrent, AdvanceFields::none()).unwrap_err();
        assert!(matches!(err, JournalError::InvalidState(_)));
    }

    #[test]
    fn last_committed_for_note_ignores_rows_stuck_before_journal_committed() {
        let journal = Journal::open_in_memory().unwrap();
        let op = Uuid::new_v4();
        journal.insert_intent(op, "note-a", 0).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, "hash1")).unwrap();
        assert!(journal.last_committed_for_note("note-a").unwrap().is_none());
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        let row = journal.last_committed_for_note("note-a").unwrap().unwrap();
        assert_eq!(row.operation_id, op);
    }

    #[test]
    fn last_committed_for_note_resolves_by_new_version_not_insertion_order() {
        // Storage Task 4 review, finding I1: insert_intent runs BEFORE the
        // per-note lock in Vault::write_record_unchecked, so two operations
        // for the same note can call insert_intent in one rowid order but
        // actually acquire the lock, and so commit, in the OTHER order.
        // Reproduces that directly against the Journal API: op_b's row is
        // inserted (and so gets a LOWER rowid) before op_a's, but op_a
        // commits first (lower new_version), and op_b commits second with
        // the higher new_version — exactly as would happen if op_b called
        // insert_intent first but lost the race for the per-note lock.
        let journal = Journal::open_in_memory().unwrap();
        let op_b = Uuid::new_v4();
        journal.insert_intent(op_b, "note-a", 1).unwrap(); // lower rowid
        let op_a = Uuid::new_v4();
        journal.insert_intent(op_a, "note-a", 0).unwrap(); // higher rowid

        // op_a committed first in real time, producing v2.
        journal.advance(op_a, WriteState::ContentPersisted, AdvanceFields::content(2, "hash-a")).unwrap();
        journal.advance(op_a, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();

        // op_b committed second, producing v3 — the actually-current version.
        journal.advance(op_b, WriteState::ContentPersisted, AdvanceFields::content(3, "hash-b")).unwrap();
        journal.advance(op_b, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();

        // op_b has the LOWER rowid (inserted first) but the HIGHER
        // new_version (committed last). The resolved row must be op_b's:
        // `ORDER BY rowid DESC` would wrongly return op_a's row here.
        let row = journal.last_committed_for_note("note-a").unwrap().unwrap();
        assert_eq!(row.operation_id, op_b, "must resolve to the row with the highest new_version, not the highest rowid");
        assert_eq!(row.new_version, Some(3));
    }

    #[test]
    fn non_terminal_operations_excludes_failed_and_index_current() {
        let journal = Journal::open_in_memory().unwrap();
        let done = Uuid::new_v4();
        journal.insert_intent(done, "note-a", 0).unwrap();
        journal.advance(done, WriteState::ContentPersisted, AdvanceFields::content(1, "h")).unwrap();
        journal.advance(done, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        journal.advance(done, WriteState::GitCommitted, AdvanceFields::none()).unwrap();
        journal.advance(done, WriteState::IndexCurrent, AdvanceFields::none()).unwrap();

        let stuck = Uuid::new_v4();
        journal.insert_intent(stuck, "note-b", 0).unwrap();
        journal.advance(stuck, WriteState::ContentPersisted, AdvanceFields::content(1, "h2")).unwrap();

        let failed = Uuid::new_v4();
        journal.insert_intent(failed, "note-c", 0).unwrap();
        journal.advance(failed, WriteState::Failed, AdvanceFields::none()).unwrap();

        let pending = journal.non_terminal_operations().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].operation_id, stuck);
    }
}
