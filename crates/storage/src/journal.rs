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
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
)";

// Columns in the order `row_from_sql` reads them.
const SELECT_ROWS: &str = "SELECT operation_id, note_id, state, expected_version, new_version, \
    content_hash, before_image_hash, before_image_path, git_commit_oid FROM operations";

// SQLite-side UTC timestamp with millisecond precision; keeps the journal free of a clock dependency.
const NOW_SQL: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";

#[derive(Debug, thiserror::Error)]
pub(crate) enum JournalError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid journal state: {0}")]
    InvalidState(String),
}

/// One journal row. `previous_state` arrives with the `Recovering` column in a later task.
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
}

/// Optional columns that `Journal::advance` may set alongside a state change.
/// `None` fields are left unchanged in the stored row.
#[derive(Debug, Default, Clone)]
pub(crate) struct AdvanceFields {
    new_version: Option<u64>,
    content_hash: Option<String>,
    before_image_hash: Option<String>,
    before_image_path: Option<String>,
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
        // The `state = ?8` guard makes the update a no-op if another writer moved the row first.
        let updated = self.conn.execute(
            &format!(
                "UPDATE operations SET \
                    state = ?2, \
                    new_version = COALESCE(?3, new_version), \
                    content_hash = COALESCE(?4, content_hash), \
                    before_image_hash = COALESCE(?5, before_image_hash), \
                    before_image_path = COALESCE(?6, before_image_path), \
                    updated_at = {NOW_SQL} \
                 WHERE operation_id = ?1 AND state = ?7"
            ),
            params![
                operation_id.to_string(),
                state_to_sql(to),
                fields.new_version.map(|v| v as i64),
                fields.content_hash,
                fields.before_image_hash,
                fields.before_image_path,
                state_to_sql(current.state),
            ],
        )?;
        if updated != 1 {
            return Err(JournalError::InvalidState(format!(
                "operation {operation_id} changed state concurrently; advance to {to:?} not applied"
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
    pub(crate) fn last_committed_for_note(
        &self,
        note_id: &str,
    ) -> Result<Option<OperationRow>, JournalError> {
        let row = self
            .conn
            .query_row(
                &format!(
                    "{SELECT_ROWS} WHERE note_id = ?1 AND state IN (?2, ?3, ?4) ORDER BY rowid DESC LIMIT 1"
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

    /// Every operation that is neither `IndexCurrent` nor `Failed`, oldest first.
    pub(crate) fn non_terminal_operations(&self) -> Result<Vec<OperationRow>, JournalError> {
        let mut stmt = self.conn.prepare(&format!(
            "{SELECT_ROWS} WHERE state NOT IN (?1, ?2) ORDER BY rowid ASC"
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
