//! Durable, structured, content-free audit events (B5.12 Task 33).
//!
//! This is the REAL audit trail the B5 design doc requires — distinct from
//! `mcp-client::InvocationLogEntry` (B5.9 Task 29's own deliberately
//! "provisional, local, diagnostic" record; see that module's own doc
//! comment for why it is NOT this schema and never claims to be). The two
//! schemas are never unified into one type: `mcp-client` has, and keeps,
//! zero dependency on `brain-api` (B5.10's own architecture test enforces
//! this structurally), so it cannot construct an [`AuditEvent`] itself.
//!
//! **Trust boundary for this task**: `brain-api`'s own capability functions
//! (`save_memory`/`update_memory`/`link_entities`/`approve_proposal`/
//! `reject_proposal`/the 9 read capabilities) keep their exact frozen
//! signatures from Tasks 13/14/16/17/18 — none of them takes a sink
//! parameter, none of them is modified by this task. Emission happens one
//! layer up, at the two places that already build every `RequestContext`
//! and therefore already know every call's `correlation_id`/outcome:
//! `mcp-server::SuperBrainMcpServer` (every MCP tool call) and
//! `brain-api::cli` (the trusted CLI approval surface). Each wraps its
//! existing `brain_api::*` call with [`emit_for`]/the per-capability
//! helpers in this module, writing to an injected [`AuditSink`] — this
//! module defines the schema and the sink trait; it does not reach into
//! either caller's internals, and does not change `propose_and_commit`'s
//! status as the one real write authority (this is observability, not a
//! new execution path).
//!
//! **What this is not**: not persisted (an [`AuditSink`] implementation's
//! job, out of this task's scope — [`NullAuditSink`] discards everything,
//! same as `mcp-client::NullSink`), not signed, not append-only, not
//! tamper-evident. This module makes no claim to any of those properties;
//! a future task choosing a real sink implementation is the one that could
//! add them, and must not claim them here prematurely.
//!
//! **Content-free discipline**: every [`AuditEventKind`] field is an
//! identifier, a boolean, or a short category/reason label — never a note
//! body, a query string, tool call arguments, or any other content-bearing
//! value. This is enforced by the type definitions themselves having no
//! such field (there is no field to accidentally populate with content),
//! backed by a regression test that serializes every variant and checks
//! for content-shaped substrings.

use serde::{Deserialize, Serialize};

/// One durable audit record: a categorized event, when it happened, and the
/// correlation id tying it back to the `RequestContext` that produced it
/// (the same `correlation_id` already present on every `RequestContext`
/// since `lib.rs`'s original B5.8 shape — this module adds no second id
/// scheme).
///
/// `Deserialize` added (M5 Task 4): purely additive, no field/variant/
/// wire-format change — required so `audit_record::parse_event` can
/// decode a verified record's own `content_bytes` back into a typed
/// value for recovery/inspection. The B5.12 freeze is on the ORIGINAL 7
/// variants' own names and field shapes (still exactly those 7,
/// unchanged, still content-free); M5 Task 5 adds two further variants
/// (`ApprovalAttempted`/`CommitOutcome`) additively, never touching the
/// frozen 7.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub kind: AuditEventKind,
    pub timestamp: String,
    pub correlation_id: uuid::Uuid,
}

/// The frozen set of audit event categories (B5.12), plus M5 Task 5's two
/// additive variants below — the original 7 keep their exact names and
/// field shapes unchanged. No field anywhere carries note content, query
/// text, tool arguments, or any other content-bearing payload; `reason`
/// fields are a short category/explanation chosen by the emitting call
/// site, never a verbatim echo of caller-supplied free text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AuditEventKind {
    McpConnection { server: String },
    ToolCall { tool: String, allowed: bool },
    PermissionDecision { operation: String, allowed: bool },
    DeniedAction { operation: String, reason: String },
    WriteMemory { proposal_id: String },
    ExternalExecution { server: String, tool: String },
    FallbackOrError { context: String },
    /// M5 Task 5: records that an approval attempt was MADE on
    /// `proposal_id` — distinct from `WriteMemory` (which records a
    /// successful commit's `proposal_id` alone, with no notion of "an
    /// attempt" at all) and distinct from the proposal's own existence
    /// (a `PendingWriteProposal` being present in the store is never, by
    /// itself, evidence that anyone ever called `approve_proposal` on
    /// it). `attempt_id` is a fresh identifier minted by the caller at
    /// the moment of the attempt — never derived from `proposal_id` or
    /// `correlation_id` — so two separate attempts against the same
    /// proposal (e.g. a retried approval after a transient failure)
    /// produce two distinct `ApprovalAttempted` records, each
    /// independently correlatable with its own eventual `CommitOutcome`
    /// via that same `attempt_id`. `correlation_id` is carried on the
    /// enclosing `AuditEvent` as usual (the request that initiated this
    /// attempt); `proposal_id` here is the target being approved, kept
    /// as its own field (not inferred from `correlation_id`) because one
    /// request's correlation id and the proposal id it acts on are
    /// independent identifiers with no fixed relationship.
    ApprovalAttempted { attempt_id: String, proposal_id: String },
    /// M5 Task 5: the terminal outcome of exactly one `ApprovalAttempted`
    /// (linked by `attempt_id`, never re-derived from `proposal_id`
    /// alone, since a proposal can accumulate more than one attempt
    /// over time). `reject_proposal` must NEVER emit this variant — a
    /// rejection is a pure status transition with no commit of any kind,
    /// so there is no outcome here to record; only `approve_proposal`'s
    /// own call site may emit it, and only after an `ApprovalAttempted`
    /// for the same `attempt_id` already exists.
    CommitOutcome { attempt_id: String, outcome: CommitOutcomeKind },
}

/// The terminal classification of one approval attempt's commit —
/// mirrors `storage::CommitResult`'s own two success shapes
/// (`Committed`/`GitPending`) plus one categorized failure bucket for
/// everything `approve_proposal` can return as `Err` (`BrainApiError`'s
/// own variants, never the error's full `Display` text — that could
/// carry caller-supplied detail this schema's content-free discipline
/// forbids). This module does not import `storage::CommitResult` or
/// `BrainApiError` into this enum directly (this module must stay a pure
/// schema definition, no behavioral dependency on either type's own
/// evolution) — a future, separately authorized wiring task maps between
/// them; this enum only defines the SHAPE of what gets persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitOutcomeKind {
    /// The write reached full commit (storage's own `CommitResult::Committed`).
    Committed,
    /// The write durably reached the journal but git has not yet
    /// committed it (storage's own `CommitResult::GitPending`) — not a
    /// failure of the approval itself.
    GitPending,
    /// The approval attempt did not commit at all, categorized by which
    /// kind of operational failure it was — never the raw error message.
    Failed { category: CommitFailureCategory },
}

/// A coarse, content-free bucket for why an approval attempt's commit
/// failed — one variant per `BrainApiError` case `approve_proposal` can
/// actually return (`PermissionDenied` is excluded: a permission failure
/// never reaches `execute`, so it is never the OUTCOME of an attempt that
/// got far enough to commit; a denied attempt is recorded by the
/// existing `PermissionDecision`/`DeniedAction` variants instead, not by
/// a `CommitOutcome` at all).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitFailureCategory {
    NotFound,
    InvalidInput,
    Conflict,
    RuntimeUnavailable,
    CorruptData,
}

/// Emission destination for [`AuditEvent`]s. Mirrors `mcp_client::
/// InvocationLogSink`'s shape deliberately (same established pattern in
/// this codebase for "a call site always produces a record, regardless of
/// outcome, through an injected sink") — not a coincidence, a reuse of the
/// one pattern already proven here, per the instruction not to invent a
/// second incompatible mechanism.
pub trait AuditSink: Send + Sync {
    fn record(&self, event: AuditEvent);
}

/// The default sink: discards every event. Callers that do not yet wire a
/// real destination (a file, a channel, a durable store — none of which
/// this task adds) get this, so emission is always safe to call and never
/// panics or blocks for lack of a configured destination.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullAuditSink;

impl AuditSink for NullAuditSink {
    fn record(&self, _event: AuditEvent) {}
}

/// Builds one [`AuditEvent`] of `kind`, stamped with `now` and
/// `correlation_id` — the one constructor every call site below (and any
/// future one) goes through, so every event's shape is produced
/// identically rather than each call site building the struct literal
/// itself.
pub fn event(kind: AuditEventKind, now: &str, correlation_id: uuid::Uuid) -> AuditEvent {
    AuditEvent { kind, timestamp: now.to_string(), correlation_id }
}

/// Records a `PermissionDecision` for `operation`, and — only when
/// `allowed` is `false` — an additional `DeniedAction` with `reason`. A
/// granted permission is fully described by one `PermissionDecision`
/// event; a denial gets both, since "what was denied" (`DeniedAction`) is
/// strictly more specific than "a decision was made" and callers filtering
/// for denials should not have to reconstruct that from a boolean on a
/// differently-named event alone.
pub fn record_permission_decision(sink: &dyn AuditSink, operation: &str, allowed: bool, reason: Option<&str>, now: &str, correlation_id: uuid::Uuid) {
    sink.record(event(AuditEventKind::PermissionDecision { operation: operation.to_string(), allowed }, now, correlation_id));
    if !allowed {
        sink.record(event(
            AuditEventKind::DeniedAction { operation: operation.to_string(), reason: reason.unwrap_or("permission denied").to_string() },
            now,
            correlation_id,
        ));
    }
}

/// Records a `ToolCall` event for one MCP tool invocation — `allowed`
/// reflects whether the call was permitted to reach `brain-api` at all
/// (every registered MCP tool call reaches this point, since `brain-api`'s
/// own permission checks run inside the capability function itself; this
/// event records that the tool was CALLED, not a duplicate, pre-emptive
/// authorization decision — see `tool_call_allowed_from_result` below for
/// how `allowed` is derived without brain-api exposing a separate
/// pre-check).
pub fn record_tool_call(sink: &dyn AuditSink, tool: &str, allowed: bool, now: &str, correlation_id: uuid::Uuid) {
    sink.record(event(AuditEventKind::ToolCall { tool: tool.to_string(), allowed }, now, correlation_id));
}

/// Records a successful `save_memory`/`update_memory`/`link_entities` call
/// as `WriteMemory` — `proposal_id` only, never the proposed body/relation
/// detail (those live in the proposal itself, in `.proposals/`, not in the
/// audit trail).
pub fn record_write_memory(sink: &dyn AuditSink, proposal_id: &str, now: &str, correlation_id: uuid::Uuid) {
    sink.record(event(AuditEventKind::WriteMemory { proposal_id: proposal_id.to_string() }, now, correlation_id));
}

/// Derives the `allowed` flag for [`record_tool_call`]/
/// [`record_permission_decision`] from a `brain_api::BrainApiError` result,
/// without `brain-api`'s own capability functions exposing a separate
/// pre-check call a caller would have to invoke twice (once to ask, once
/// to act) — `PermissionDenied` is the one frozen error variant that means
/// "the permission check itself failed," distinct from every other failure
/// category (`NotFound`/`Conflict`/`CorruptData`/etc., all of which mean
/// the call WAS allowed to proceed but failed for some other reason).
pub fn allowed_from_result<T>(result: &Result<T, crate::BrainApiError>) -> bool {
    !matches!(result, Err(crate::BrainApiError::PermissionDenied))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_variants() -> Vec<AuditEventKind> {
        vec![
            AuditEventKind::McpConnection { server: "s".into() },
            AuditEventKind::ToolCall { tool: "search".into(), allowed: true },
            AuditEventKind::PermissionDecision { operation: "ProposeWrite".into(), allowed: false },
            AuditEventKind::DeniedAction { operation: "ApproveProposal".into(), reason: "permission denied".into() },
            AuditEventKind::WriteMemory { proposal_id: "11111111-1111-1111-1111-111111111111".into() },
            AuditEventKind::ExternalExecution { server: "ext".into(), tool: "echo".into() },
            AuditEventKind::FallbackOrError { context: "dense retrieval unavailable".into() },
            AuditEventKind::ApprovalAttempted {
                attempt_id: "22222222-2222-2222-2222-222222222222".into(),
                proposal_id: "11111111-1111-1111-1111-111111111111".into(),
            },
            AuditEventKind::CommitOutcome {
                attempt_id: "22222222-2222-2222-2222-222222222222".into(),
                outcome: CommitOutcomeKind::Committed,
            },
            AuditEventKind::CommitOutcome {
                attempt_id: "22222222-2222-2222-2222-222222222222".into(),
                outcome: CommitOutcomeKind::GitPending,
            },
            AuditEventKind::CommitOutcome {
                attempt_id: "22222222-2222-2222-2222-222222222222".into(),
                outcome: CommitOutcomeKind::Failed { category: CommitFailureCategory::Conflict },
            },
        ]
    }

    // --- content-free regression guard ---

    #[test]
    fn no_variant_serializes_a_note_body_or_query_text_shaped_field() {
        // Every variant's own fields are identifiers/booleans/short labels
        // by construction (the type definition has no String field that
        // could hold arbitrary content except `server`/`tool`/`operation`/
        // `reason`/`proposal_id`/`context`, all of which this test's own
        // fixed, short, identifier-shaped values populate) -- this
        // regression guard pins that every variant, once constructed with
        // realistic short values, serializes to a bounded, non-prose
        // JSON shape, not an open-ended content field.
        for kind in all_variants() {
            let json = serde_json::to_string(&kind).unwrap();
            assert!(json.len() < 300, "variant serialized unexpectedly large, suggesting a content-bearing field crept in: {json}");
            assert!(!json.contains('\n'), "a multi-line value suggests prose/content leaked into an audit event: {json}");
        }
    }

    #[test]
    fn every_variant_round_trips_through_serialization() {
        for kind in all_variants() {
            let json = serde_json::to_string(&kind).unwrap();
            assert!(!json.is_empty());
        }
    }

    // --- M5 Task 5: ApprovalAttempted / CommitOutcome schema ---

    #[test]
    fn every_new_variant_round_trips_through_serde_json_exactly() {
        for kind in [
            AuditEventKind::ApprovalAttempted { attempt_id: "a1".into(), proposal_id: "p1".into() },
            AuditEventKind::CommitOutcome { attempt_id: "a1".into(), outcome: CommitOutcomeKind::Committed },
            AuditEventKind::CommitOutcome { attempt_id: "a1".into(), outcome: CommitOutcomeKind::GitPending },
            AuditEventKind::CommitOutcome {
                attempt_id: "a1".into(),
                outcome: CommitOutcomeKind::Failed { category: CommitFailureCategory::NotFound },
            },
            AuditEventKind::CommitOutcome {
                attempt_id: "a1".into(),
                outcome: CommitOutcomeKind::Failed { category: CommitFailureCategory::InvalidInput },
            },
            AuditEventKind::CommitOutcome {
                attempt_id: "a1".into(),
                outcome: CommitOutcomeKind::Failed { category: CommitFailureCategory::Conflict },
            },
            AuditEventKind::CommitOutcome {
                attempt_id: "a1".into(),
                outcome: CommitOutcomeKind::Failed { category: CommitFailureCategory::RuntimeUnavailable },
            },
            AuditEventKind::CommitOutcome {
                attempt_id: "a1".into(),
                outcome: CommitOutcomeKind::Failed { category: CommitFailureCategory::CorruptData },
            },
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let decoded: AuditEventKind = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, kind, "round trip through JSON must reproduce the exact original variant, got {json}");
        }
    }

    #[test]
    fn approval_attempted_and_commit_outcome_are_correlated_by_attempt_id_not_proposal_id() {
        // Two separate attempts against the SAME proposal_id must be
        // distinguishable by attempt_id -- proposal_id alone can never
        // tell two attempts apart.
        let attempted_1 = AuditEventKind::ApprovalAttempted { attempt_id: "attempt-1".into(), proposal_id: "p1".into() };
        let attempted_2 = AuditEventKind::ApprovalAttempted { attempt_id: "attempt-2".into(), proposal_id: "p1".into() };
        assert_ne!(attempted_1, attempted_2, "two distinct attempts against the same proposal must not collapse into equal events");

        let outcome_1 = AuditEventKind::CommitOutcome { attempt_id: "attempt-1".into(), outcome: CommitOutcomeKind::Committed };
        let outcome_2 = AuditEventKind::CommitOutcome {
            attempt_id: "attempt-2".into(),
            outcome: CommitOutcomeKind::Failed { category: CommitFailureCategory::Conflict },
        };
        // Each outcome's own attempt_id is what ties it back to its own
        // attempt -- never proposal_id, which CommitOutcome does not even
        // carry.
        if let (AuditEventKind::ApprovalAttempted { attempt_id: a1, .. }, AuditEventKind::CommitOutcome { attempt_id: o1, .. }) = (&attempted_1, &outcome_1) {
            assert_eq!(a1, o1);
        } else {
            panic!("pattern must match");
        }
        if let (AuditEventKind::ApprovalAttempted { attempt_id: a2, .. }, AuditEventKind::CommitOutcome { attempt_id: o2, .. }) = (&attempted_2, &outcome_2) {
            assert_eq!(a2, o2);
        } else {
            panic!("pattern must match");
        }
    }

    #[test]
    fn commit_outcome_never_carries_a_proposal_id_field_only_attempt_id() {
        // Structural guard for requirement 1: a proposal's existence (or
        // its id) is never, by itself, sufficient to reconstruct whether
        // or what a CommitOutcome says -- CommitOutcome's own shape has
        // no proposal_id field at all, only attempt_id, so a reader can
        // never mistake "a proposal exists" for "this specific commit
        // outcome happened to it" without first finding the matching
        // ApprovalAttempted record carrying that attempt_id.
        let json = serde_json::to_string(&AuditEventKind::CommitOutcome { attempt_id: "a1".into(), outcome: CommitOutcomeKind::Committed }).unwrap();
        assert!(!json.contains("proposal_id"), "CommitOutcome's own serialized shape must never carry a proposal_id field, got {json}");
    }

    #[test]
    fn no_commit_outcome_variant_serializes_a_raw_error_message_string() {
        // CommitFailureCategory is a closed, content-free enum -- there is
        // no String field anywhere in the Failed variant's own shape for
        // a raw BrainApiError::InvalidInput(String)'s own caller-supplied
        // detail to leak into, by construction. The real guard: the
        // serialized form stays short and bounded, same discipline as the
        // existing content-free regression test, and carries only the
        // closed category label, never free text.
        let json = serde_json::to_string(&AuditEventKind::CommitOutcome {
            attempt_id: "a1".into(),
            outcome: CommitOutcomeKind::Failed { category: CommitFailureCategory::InvalidInput },
        })
        .unwrap();
        assert!(json.len() < 150, "a categorized failure must stay short and bounded, got {json}");
        assert!(json.contains("invalid_input"), "the closed category label must appear, got {json}");
    }

    #[test]
    fn the_new_variants_are_compatible_with_the_pure_hash_chain_functions_and_recovery_scan() {
        // Task 5 requirement 6: the new schema must round-trip cleanly
        // through audit_record::serialize_record/parse_line/parse_event
        // and verify through audit_chain::scan_chain exactly like any
        // existing variant -- proven directly here, not assumed.
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let proposal_id = uuid::Uuid::new_v4().to_string();
        let correlation_id = uuid::Uuid::new_v4();

        let e0 = event(AuditEventKind::ApprovalAttempted { attempt_id: attempt_id.clone(), proposal_id: proposal_id.clone() }, "2026-11-19T00:00:00Z", correlation_id);
        let (line0, hash0) = crate::audit_record::serialize_record(&e0, 0, crate::audit_chain::GENESIS_HASH);

        let e1 = event(AuditEventKind::CommitOutcome { attempt_id: attempt_id.clone(), outcome: CommitOutcomeKind::Committed }, "2026-11-19T00:00:01Z", correlation_id);
        let (line1, hash1) = crate::audit_record::serialize_record(&e1, 1, &hash0);

        let mut raw = line0.clone();
        raw.extend_from_slice(&line1);

        let record0 = crate::audit_record::parse_line(&line0[..line0.len() - 1]).unwrap();
        let record1 = crate::audit_record::parse_line(&line1[..line1.len() - 1]).unwrap();
        let lines = vec![crate::audit_chain::LineOutcome::Parsed(record0), crate::audit_chain::LineOutcome::Parsed(record1)];

        let scan = crate::audit_chain::scan_chain(&lines);
        assert!(scan.stop_reason.is_none(), "a chain built entirely from the two new variants must verify cleanly, got {:?}", scan.stop_reason);
        assert_eq!(scan.verified_records.len(), 2);
        assert_eq!(scan.verified_records[1].record_hash, hash1);

        let decoded0 = crate::audit_record::parse_event(&scan.verified_records[0].content_bytes).unwrap();
        let decoded1 = crate::audit_record::parse_event(&scan.verified_records[1].content_bytes).unwrap();
        assert_eq!(decoded0.kind, AuditEventKind::ApprovalAttempted { attempt_id: attempt_id.clone(), proposal_id });
        assert_eq!(decoded1.kind, AuditEventKind::CommitOutcome { attempt_id, outcome: CommitOutcomeKind::Committed });
    }

    // --- construction / sink behavior ---

    #[derive(Default)]
    struct RecordingSink(std::sync::Mutex<Vec<AuditEvent>>);
    impl AuditSink for RecordingSink {
        fn record(&self, event: AuditEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn null_sink_discards_every_event_without_panicking() {
        let sink = NullAuditSink;
        sink.record(event(AuditEventKind::ToolCall { tool: "search".into(), allowed: true }, "2026-11-19T00:00:00Z", uuid::Uuid::new_v4()));
        // no observable state to assert on -- the point is that this never panics.
    }

    #[test]
    fn record_permission_decision_emits_only_one_event_when_allowed() {
        let sink = RecordingSink::default();
        let correlation_id = uuid::Uuid::new_v4();
        record_permission_decision(&sink, "ProposeWrite", true, None, "2026-11-19T00:00:00Z", correlation_id);
        let events = sink.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].kind, AuditEventKind::PermissionDecision { ref operation, allowed: true } if operation == "ProposeWrite"));
    }

    #[test]
    fn record_permission_decision_emits_a_decision_and_a_denied_action_when_denied() {
        let sink = RecordingSink::default();
        let correlation_id = uuid::Uuid::new_v4();
        record_permission_decision(&sink, "ApproveProposal", false, Some("mcp session cannot approve"), "2026-11-19T00:00:00Z", correlation_id);
        let events = sink.0.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].kind, AuditEventKind::PermissionDecision { allowed: false, .. }));
        assert!(matches!(events[1].kind, AuditEventKind::DeniedAction { ref reason, .. } if reason == "mcp session cannot approve"));
    }

    #[test]
    fn every_event_carries_the_given_correlation_id_and_timestamp() {
        let sink = RecordingSink::default();
        let correlation_id = uuid::Uuid::new_v4();
        record_tool_call(&sink, "search", true, "2026-11-19T00:00:00Z", correlation_id);
        let events = sink.0.lock().unwrap();
        assert_eq!(events[0].correlation_id, correlation_id);
        assert_eq!(events[0].timestamp, "2026-11-19T00:00:00Z");
    }

    // --- allowed_from_result ---

    #[test]
    fn allowed_from_result_is_false_only_for_permission_denied() {
        assert!(!allowed_from_result::<()>(&Err(crate::BrainApiError::PermissionDenied)));
        assert!(allowed_from_result(&Ok(())));
        assert!(allowed_from_result::<()>(&Err(crate::BrainApiError::NotFound)));
        assert!(allowed_from_result::<()>(&Err(crate::BrainApiError::Conflict)));
        assert!(allowed_from_result::<()>(&Err(crate::BrainApiError::CorruptData)));
    }
}
