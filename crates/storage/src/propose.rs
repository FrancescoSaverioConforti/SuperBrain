use std::collections::HashSet;

use domain::Classification;
use uuid::Uuid;
use vault_security::{PiiPointer, PiiStore};

use crate::note_id::{NoteId, NoteIdError};
use crate::vault::{PendingDeletion, StorageError, Vault, Version};

/// A reserved body marker that signals "this `Sensitive`-note write is a
/// DELETION, not a content update" to `propose_and_commit`'s sensitive
/// branch. Deliberately not a new enum parameter on `propose_and_commit`'s
/// public signature (which remains the one and only public write entry
/// point in the whole workspace, per the Global Constraints): a caller
/// requests deletion of a sensitive note by calling `propose_and_commit`
/// with a note whose BODY is exactly this sentinel. The gate still scans the
/// whole note text (the sentinel contains no secret-shaped content, so it
/// always passes the secret scanner), and `propose_and_commit`'s own check
/// for this exact body happens only AFTER the gate has passed, so a
/// malformed or rejected note can never be mistaken for a tombstone.
///
/// Chosen to be exceedingly unlikely to collide with real note content
/// (an HTML-comment-shaped marker, which `domain::parse_frontmatter` never
/// treats specially and which a human would never type as the entire body
/// of a real note by accident).
pub const SENSITIVE_TOMBSTONE_BODY: &str = "<!-- SUPERBRAIN:TOMBSTONE -->";

/// Every way `propose_and_commit` can fail to commit a note, in the exact
/// short-circuit order the function evaluates them: schema parse (surfaced
/// via the gate as `Rejected`), note-id validation, link classification,
/// gate rejection, then storage.
#[derive(Debug)]
pub enum ProposeError {
    Schema(domain::ValidationError),
    /// A note id in the frontmatter — the note's own `id`, or a relation
    /// target that is not declared in `pending_ids` — is not a canonical
    /// [`NoteId`]. `field` names where it came
    /// from (`"id"` or `"relations.<name>"`), `value` is the raw string.
    /// Raised before the vault is ever touched: in particular a case
    /// variant of an existing id (e.g. `NOTE-A` vs `note-a`), which would
    /// alias the same record file on a case-insensitive filesystem, ends
    /// here as `NoteIdError::MustBeLowercase` and never reaches storage.
    InvalidNoteId {
        field: String,
        value: String,
        error: NoteIdError,
    },
    Link(domain::LinkError),
    Rejected(policy::GateRejection),
    Storage(StorageError),
}

/// What a successful [`propose_and_commit`] reports back to the caller.
///
/// `link_state` is the `domain::classify_links` result for the committed
/// note's relations: `Unlinked` (no relation targets), `Resolved` (every
/// target is a known note) or `Pending` (at least one target was accepted
/// because the caller declared it in `pending_ids`). Spec contract C6 says a
/// pending reference is "accettato e segnalato" — accepted *and flagged* —
/// and this field is the flag: a caller can tell a commit that still carries
/// unresolved references from a fully resolved one without re-classifying
/// the note itself. (`Broken` cannot appear here: a broken link is
/// `ProposeError::Link` and nothing is committed.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitOutcome {
    /// The note's new version, as returned by the vault.
    pub version: Version,
    pub link_state: domain::LinkState,
}

/// `propose_and_commit`'s `Ok` type. The spec requires `commit_write` to
/// distinguish a full success from "git_pending, non un successo" — a
/// distinction `Result<CommitOutcome, ProposeError>` alone cannot express,
/// since git unavailability is an expected, non-error outcome, not a
/// failure of the proposal itself.
///
/// `ProposeError::Storage(StorageError::GitPending { .. })` is caught
/// inside `propose_and_commit` and converted to `Ok(CommitResult::GitPending { .. })`
/// rather than propagated as an error: the note's content IS durably
/// persisted and journal-readable (it reached `WriteState::JournalCommitted`)
/// even though the write has not yet reached `WriteState::GitCommitted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitResult {
    /// The write reached `WriteState::GitCommitted` (and, if applicable,
    /// beyond) — a full, ordinary success.
    Committed(CommitOutcome),
    /// The write reached `WriteState::JournalCommitted` and is durably
    /// persisted and readable, but git could not commit it (the vault root
    /// is not yet a git repository, the repository has no commits yet, or
    /// git otherwise failed), so it could not be advanced to
    /// `WriteState::GitCommitted`. A later retry of the SAME `operation_id`,
    /// or the next `Vault::open`, advances it the rest of the way once git
    /// can commit.
    GitPending { note_id: String, operation_id: Uuid },
}

/// The single public write path out of `storage`, and therefore out of the
/// whole workspace (the raw primitives `atomic_write` and
/// `Vault::write_record_unchecked` are both `pub(crate)`): parses the
/// frontmatter of `note_text`, validates the note's id and every non-pending
/// relation target as a canonical [`NoteId`], classifies its relations
/// against `known_ids`/`pending_ids`, runs `gate.evaluate` over the **whole**
/// `note_text` (or `gate.evaluate_schema_error` if parsing already failed),
/// and — only on `policy::GateDecision::Pass` — calls the crate-private
/// `Vault::write_record_unchecked` with the whole `note_text`. Every
/// rejection path (schema, note id, link, gate) returns before the vault is
/// ever touched, so a rejected note's record file is never created. Once the
/// write has reached `WriteState::JournalCommitted` (durably happened), a
/// git failure is never reported as a `ProposeError`: it is
/// `Ok(CommitResult::GitPending { .. })`, whatever the git failure was
/// (final whole-branch review, finding I2). The `ProposeError::Storage`
/// variants that can still follow a durable write are journal (SQLite)
/// failures in the steps after `JournalCommitted`; the operation then rests
/// at a recoverable state that a retry of the same `operation_id`, or the
/// next `Vault::open`, completes.
///
/// `note_text` is a complete note, in one of two forms (the private helper
/// `frontmatter_of` documents the exact splitting rule):
///
/// - **Full note**: a first line that is exactly `---`, the YAML
///   frontmatter, a closing line that is exactly `---`, then an optional
///   body. The body is opaque: it is never parsed as YAML or Markdown and
///   never validated, but it *is* scanned by the gate's secret scanner.
/// - **Frontmatter only** (the original form): bare YAML with no opening
///   `---` line and no body.
///
/// Either way, the stored record content is `note_text` byte-for-byte —
/// delimiters, frontmatter and body exactly as given — so
/// `Vault::read_note` returns exactly what the caller passed in. Frontmatter
/// and body are one record with one [`Version`]: an update replaces both
/// together, and a stale `expected_version` is the same
/// `StorageError::Conflict` whether or not a body is present.
///
/// Identity — deliberately asymmetric:
///
/// - `known_ids` are real notes in this vault, so they are canonical
///   [`NoteId`]s, the same identity the vault uses for its lock key and
///   record path. A relation target that is not declared pending must parse
///   as a `NoteId` (else `ProposeError::InvalidNoteId`), and then either
///   resolves against `known_ids` or is a `domain::LinkError::BrokenLink`.
/// - `pending_ids` are **opaque strings**, not note ids: per spec contract
///   C6, a pending reference may be an external source (a URL, a DOI), a
///   note not yet imported, or an unresolved code symbol
///   (`src/main.rs::CorsMiddleware`). A relation target that is present
///   verbatim in `pending_ids` is accepted as pending and is never run
///   through `NoteId::parse`. Pending targets never touch the filesystem, so
///   they cannot alias a record file. Matching is exact (byte-for-byte):
///   the caller declares exactly which references are pending.
///
/// On success the result is a [`CommitResult`]. The ordinary case is
/// `CommitResult::Committed(CommitOutcome)`: the new [`Version`] plus the
/// note's link state, so a note committed with pending references is
/// reported as `LinkState::Pending` rather than accepted silently. If git
/// cannot commit (no repository at the vault root yet, no commits yet, or any
/// other git failure), the write still durably persists and reaches
/// `WriteState::JournalCommitted`, but cannot advance to
/// `WriteState::GitCommitted`; that case is reported as
/// `Ok(CommitResult::GitPending { .. })`, not an error — git unavailability
/// is an expected, non-error outcome per spec, not a failure of the
/// proposal itself.
///
/// `pii_store` is the `PiiStore` this call uses to answer the gate's
/// `EncryptedStoreAvailability` question and, for a `Sensitive` note, to
/// actually stage and commit its ciphertext (Task 6). `None` means no
/// encrypted store is configured/reachable for this call — the gate then
/// sees `EncryptedStoreAvailability::Unavailable { NotYetConfigured }` and
/// rejects a `Sensitive` note exactly as it always has, per Task 4's
/// design, while an `Internal`/`Public` write proceeds completely
/// unaffected (A3 review round, IMPORTANT finding #2: a mandatory
/// `&PiiStore` would have made even non-sensitive writes depend on a
/// working encrypted store, which is backwards — Task 4 exists precisely
/// so a broken/unconfigured store blocks ONLY `Sensitive` writes). It is
/// the caller's job to open it once (typically at `<vault_root>/.pii-store`,
/// mirroring how `vault.rs`'s `BEFORE_IMAGE_DIR` is a fixed, documented
/// sibling of the vault root) and pass the SAME `PiiStore` across calls, so
/// repeated calls share one master key and one on-disk store rather than
/// re-deriving a fingerprint and potentially re-keying on every call.
#[allow(clippy::too_many_arguments)]
pub fn propose_and_commit(
    vault: &Vault,
    note_text: &str,
    expected_version: Version,
    operation_id: Uuid,
    known_ids: &HashSet<NoteId>,
    pending_ids: &HashSet<String>,
    gate: &policy::Gate,
    pii_store: Option<&PiiStore>,
) -> Result<CommitResult, ProposeError> {
    let note = match domain::parse_frontmatter(frontmatter_of(note_text)) {
        Ok(note) => note,
        Err(err) => {
            return match gate.evaluate_schema_error(err) {
                policy::GateDecision::Reject(rejection) => Err(ProposeError::Rejected(rejection)),
                policy::GateDecision::Pass => {
                    unreachable!("evaluate_schema_error never returns Pass")
                }
            };
        }
    };

    let note_id = parse_note_id("id", &note.id)?;
    let relations = &note.relations;
    for (relation, targets) in [
        ("supports", &relations.supports),
        ("contradicts", &relations.contradicts),
        ("derived_from", &relations.derived_from),
        ("supersedes", &relations.supersedes),
        ("applies_to", &relations.applies_to),
    ] {
        for target in targets {
            // A caller-declared pending reference is opaque (URL, DOI,
            // code symbol, ...) and is never forced through NoteId syntax.
            if !pending_ids.contains(target) {
                parse_note_id(&format!("relations.{relation}"), target)?;
            }
        }
    }

    // Every non-pending target is now a validated canonical NoteId string
    // (NoteId::parse never normalizes, so `as_str()` of a parsed id equals
    // the raw input), which makes `domain::classify_links`' plain string
    // comparison against `known` an exact canonical-identity comparison.
    // Pending targets are compared verbatim against the caller's opaque set.
    let known: HashSet<String> = known_ids.iter().map(|id| id.as_str().to_string()).collect();

    // propose_and_commit's contract exposes broken/self-referential links as
    // their own domain::LinkError directly (ProposeError::Link), not wrapped
    // in a GateRejection, so callers can match on the precise domain error
    // without unwrapping a policy-level rejection first. This check happens
    // before the gate call so a link error never reaches
    // GateRejection::Link by a different path.
    let link_state = match domain::classify_links(&note, &known, pending_ids) {
        Ok(state) => state,
        Err(link_err) => return Err(ProposeError::Link(link_err)),
    };

    // Task 6 (A3 review round, IMPORTANT finding #2): whether the encrypted
    // store is available for THIS call, computed for real rather than
    // hardcoded. `pii_store: None` means no store is configured/reachable
    // at all -> `NotYetConfigured`. A `Some` value already in hand (the
    // caller opened it successfully) is `Available` — there is nothing
    // left to try; `vault_security::unavailable_reason_for`'s mapping is
    // for a caller that has NOT yet managed to open one at all, exercised
    // by whatever constructs the `PiiStore` before calling in, not here.
    let availability = match pii_store {
        Some(_) => vault_security::EncryptedStoreAvailability::Available,
        None => vault_security::EncryptedStoreAvailability::Unavailable {
            reason: vault_security::StoreUnavailableReason::NotYetConfigured,
        },
    };

    // The gate scans the WHOLE note text, body included, so a secret that
    // appears only in the body is still caught before the vault is touched.
    match gate.evaluate(&note, note_text, Ok(link_state), availability) {
        policy::GateDecision::Reject(rejection) => Err(ProposeError::Rejected(rejection)),
        policy::GateDecision::Pass => {
            if matches!(note.classification, Classification::Sensitive) {
                // The gate already required `Available` to reach `Pass`
                // for a `Sensitive` note (see `availability` above), so
                // `pii_store` is always `Some` here — this `expect` can
                // never fire without the gate itself having a bug.
                let pii_store = pii_store.expect("gate passed a Sensitive note without an available PiiStore");
                commit_sensitive(vault, &note_id, note_text, expected_version, operation_id, pii_store, link_state)
            } else {
                // `pii_store` is threaded through so a catch-up of an
                // earlier git-pending SENSITIVE write on this note can clean
                // up its PII before-image (A3 review round 3, item 1(b)).
                match vault.write_record_unchecked_with_pii_store(&note_id, note_text, expected_version, operation_id, pii_store) {
                    Ok(version) => Ok(CommitResult::Committed(CommitOutcome { version, link_state })),
                    Err(StorageError::GitPending { note_id }) => {
                        Ok(CommitResult::GitPending { note_id, operation_id })
                    }
                    Err(other) => Err(ProposeError::Storage(other)),
                }
            }
        }
    }
}

/// The `Sensitive`-note branch of `propose_and_commit` (Task 6). `note_text`
/// is the gate-accepted full note (frontmatter plus body); its body decides
/// whether this write is an ordinary content update or a deletion
/// ([`SENSITIVE_TOMBSTONE_BODY`]):
///
/// - Ordinary update: the real body is staged into `pii_store` as
///   ciphertext, and the Markdown record that actually lands is the
///   frontmatter (verbatim, so relations/classification/metadata stay
///   queryable) plus a `pii://<note_id>` pointer body — see
///   [`pointer_record`].
/// - Deletion (the body is exactly the tombstone marker): a
///   [`PendingDeletion`] is built here and handed into
///   `Vault::write_sensitive_record_unchecked`, which calls
///   `DeletionLog::record_deletion` itself, AFTER its own per-note lock is
///   held and its own version check has passed (A3 review round, IMPORTANT
///   finding #1 — see `PendingDeletion`'s doc comment in `vault.rs` for why
///   this must not happen here, before the lock). The deletion log's own
///   three-step protocol is a SEPARATE, independently-authenticated
///   mechanism per spec contract C5 — it is not folded into the A2
///   journal's `WriteState` machine — but running it only once this
///   operation has already proven it is a legitimate write means a reader
///   can never observe a committed tombstone pointer whose deletion was
///   never logged, NOR a logged deletion whose tombstone write was
///   actually a stale, rejected attempt. The tombstone's "plaintext" staged
///   into `PiiStore` is empty: a deleted note has no more sensitive content
///   to protect, and `PiiStore::get` on it is never a meaningful operation
///   again once the deletion log also marks it deleted.
fn commit_sensitive(
    vault: &Vault,
    note_id: &NoteId,
    note_text: &str,
    expected_version: Version,
    operation_id: Uuid,
    pii_store: &PiiStore,
    link_state: domain::LinkState,
) -> Result<CommitResult, ProposeError> {
    let body = body_of(note_text);
    let is_deletion = body.trim() == SENSITIVE_TOMBSTONE_BODY;

    let frontmatter = frontmatter_of(note_text);
    let pointer_content = pointer_record(frontmatter, note_id);
    let plaintext: &[u8] = if is_deletion { b"" } else { body.as_bytes() };

    let deletion_log;
    let trusted_ref;
    let deletion = if is_deletion {
        deletion_log = vault_security::DeletionLog::open(&pii_store.deletion_log_path())
            .map_err(|e| ProposeError::Storage(StorageError::DeletionLog(e.to_string())))?;
        trusted_ref = vault_security::DpapiTrustedReferenceStore::new(pii_store.deletion_log_reference_path());
        Some(PendingDeletion { log: &deletion_log, trusted_ref: &trusted_ref })
    } else {
        None
    };

    match vault.write_sensitive_record_unchecked(note_id, &pointer_content, plaintext, expected_version, operation_id, pii_store, deletion) {
        Ok(version) => Ok(CommitResult::Committed(CommitOutcome { version, link_state })),
        Err(StorageError::GitPending { note_id }) => Ok(CommitResult::GitPending { note_id, operation_id }),
        Err(other) => Err(ProposeError::Storage(other)),
    }
}

/// Builds the Markdown record that actually lands for a `Sensitive` note:
/// the original frontmatter, delimited exactly as a full note's frontmatter
/// always is, followed by the `pii://<note_id>` pointer as the body. This
/// keeps every frontmatter-driven feature (relations, classification,
/// status, link classification on a later read) working unchanged, while
/// ensuring the real content never reaches the Markdown vault, git history,
/// the A2 journal, or any before-image — only the pointer URI does.
fn pointer_record(frontmatter: &str, note_id: &NoteId) -> String {
    let pointer = PiiPointer::for_note(note_id.as_str());
    format!("---\n{frontmatter}---\n{}", pointer.uri)
}

/// Returns the body portion of `note_text` — everything after the closing
/// frontmatter delimiter line, or the empty string if there is none (bare
/// YAML with no body). Mirrors `frontmatter_of`'s own delimiter-finding
/// logic exactly, so the two always agree on where the frontmatter ends.
fn body_of(note_text: &str) -> &str {
    let Some(after_open) = strip_delimiter_line(note_text) else {
        return "";
    };
    let mut offset = 0;
    for line in after_open.split_inclusive('\n') {
        if let Some(after_close) = strip_delimiter_line(line) {
            // `after_close` is the remainder of THIS line after the closing
            // `---`; the body starts at the NEXT line, i.e. right after the
            // full line `line` (which already includes its own trailing
            // newline from split_inclusive, or is the last, newline-less
            // line of the text).
            let _ = after_close;
            return &after_open[offset + line.len()..];
        }
        offset += line.len();
    }
    ""
}

/// Returns the YAML frontmatter portion of `note_text`, which is what gets
/// handed (unchanged) to `domain::parse_frontmatter`.
///
/// - If the first line of `note_text` is exactly `---` (LF or CRLF line
///   ending), the frontmatter is everything after that line up to (not
///   including) the next line that is exactly `---`; everything after that
///   closing line is the body and is ignored here.
/// - Otherwise — no opening `---` line, or an opening line with no closing
///   line — the whole of `note_text` is the frontmatter (the original,
///   bare-YAML input form; a lone leading `---` is just a YAML
///   document-start marker), and there is no body.
///
/// This is unambiguous: inside YAML frontmatter a line consisting of exactly
/// `---` at column 0 can only be a YAML document separator, never part of a
/// field value (a `---` inside a block scalar is indented, so it is not
/// exactly `---`). Hence the first such line always closes the frontmatter,
/// and any later `---` line (e.g. a Markdown thematic break) is body.
fn frontmatter_of(note_text: &str) -> &str {
    let Some(after_open) = strip_delimiter_line(note_text) else {
        return note_text;
    };
    let mut offset = 0;
    for line in after_open.split_inclusive('\n') {
        if strip_delimiter_line(line).is_some() {
            return &after_open[..offset];
        }
        offset += line.len();
    }
    note_text
}

/// If `text` starts with a line that is exactly `---` (terminated by `\n`,
/// `\r\n`, or end of text), returns the text after that line.
fn strip_delimiter_line(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("---")?;
    if rest.is_empty() {
        return Some(rest);
    }
    rest.strip_prefix('\n').or_else(|| rest.strip_prefix("\r\n"))
}

fn parse_note_id(field: &str, raw: &str) -> Result<NoteId, ProposeError> {
    NoteId::parse(raw).map_err(|error| ProposeError::InvalidNoteId {
        field: field.to_string(),
        value: raw.to_string(),
        error,
    })
}

#[cfg(test)]
mod tests {
    use super::frontmatter_of;

    #[test]
    fn bare_yaml_without_delimiters_is_entirely_frontmatter() {
        assert_eq!(frontmatter_of("\nid: x\nproject: p\n"), "\nid: x\nproject: p\n");
    }

    #[test]
    fn delimited_frontmatter_excludes_delimiters_and_body() {
        assert_eq!(frontmatter_of("---\nid: x\n---\n# Body\nprose\n"), "id: x\n");
    }

    #[test]
    fn empty_frontmatter_between_delimiters_is_empty() {
        assert_eq!(frontmatter_of("---\n---\nbody"), "");
    }

    #[test]
    fn crlf_delimiters_are_recognized() {
        assert_eq!(frontmatter_of("---\r\nid: x\r\n---\r\nbody\r\n"), "id: x\r\n");
    }

    #[test]
    fn closing_delimiter_at_end_of_text_without_newline_closes() {
        assert_eq!(frontmatter_of("---\nid: x\n---"), "id: x\n");
    }

    #[test]
    fn only_the_first_closing_line_counts_later_ones_are_body() {
        assert_eq!(frontmatter_of("---\na: 1\n---\nb\n---\nc: 2\n"), "a: 1\n");
    }

    #[test]
    fn indented_dashes_inside_a_block_scalar_do_not_close_the_frontmatter() {
        assert_eq!(frontmatter_of("---\na: |\n  ---\n---\nbody"), "a: |\n  ---\n");
    }

    #[test]
    fn lines_that_merely_start_with_dashes_are_not_delimiters() {
        // Not an opening line: whole text is (bare) frontmatter.
        assert_eq!(frontmatter_of("---x\nid: x\n"), "---x\nid: x\n");
        // Not a closing line: `--- ` (trailing space) and `----` are skipped.
        assert_eq!(frontmatter_of("---\na: 1\n--- \n----\n---\nbody"), "a: 1\n--- \n----\n");
    }

    #[test]
    fn opening_line_without_closing_line_falls_back_to_whole_text() {
        // A lone leading `---` is a YAML document-start marker on bare YAML.
        assert_eq!(frontmatter_of("---\nid: x\n"), "---\nid: x\n");
    }
}
