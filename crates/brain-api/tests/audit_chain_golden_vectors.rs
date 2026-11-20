//! M6.2A: audit chain golden-vector baseline.
//!
//! **Purpose**: freeze today's byte-exact, hash-exact output of the
//! CURRENT `audit_chain`/`audit_record` implementation as literal
//! fixtures, captured independently of the assertions below -- so a
//! future `audit-chain-core` extraction (conditionally approved, not yet
//! started) can be proven byte-for-byte behavior-preserving by running
//! these same tests against the extracted code.
//!
//! **Strictly additive, read-only with respect to production code**: this
//! file adds no new production code, changes no serialization or hashing
//! implementation, and does not extract `audit-chain-core`. See the M6.2A
//! task authorization (continuity decision context) for the exact scope
//! constraints this file is bound by.
//!
//! **How these fixtures were captured** (NOT by calling the implementation
//! under test during assertions -- see the explicit constraint this task
//! was given): a throwaway, uncommitted example binary
//! (`_throwaway_capture_golden_vectors.rs`, deleted immediately after use,
//! never part of any commit) called `brain_api::serialize_record` once
//! per representative event, with fixed deterministic inputs
//! (`FIXED_TS`/`FIXED_TS_1`/`FIXED_TS_2`, `CORR_ID`, `ATTEMPT_ID`,
//! `PROPOSAL_ID` below), and printed the resulting line bytes and
//! `record_hash`. Those exact printed values were then hand-copied into
//! the `const` literals below. Every assertion in this file compares a
//! FRESH call's output against these frozen literals -- never the other
//! way around, and never regenerating an expected value from the
//! implementation at assertion time.
use brain_api::{
    event, parse_event, parse_line, serialize_record, AuditEventKind, CommitFailureCategory,
    CommitOutcomeKind, RejectionOutcomeKind, GENESIS_HASH,
};

const FIXED_TS: &str = "2026-01-01T00:00:00Z";
const FIXED_TS_1: &str = "2026-01-01T00:00:01Z";
const FIXED_TS_2: &str = "2026-01-01T00:00:02Z";
const CORR_ID: &str = "00000000-0000-0000-0000-0000000000c1";
const ATTEMPT_ID: &str = "00000000-0000-0000-0000-0000000000a1";
const PROPOSAL_ID: &str = "00000000-0000-0000-0000-0000000000b1";

fn corr() -> uuid::Uuid {
    uuid::Uuid::parse_str(CORR_ID).unwrap()
}

// --- Golden vectors: one per representative AuditEventKind variant family,
//     each at sequence 0 from GENESIS_HASH. Captured independently (see
//     this file's own top-level doc comment); never generated here. ---

const MCP_CONNECTION_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"d1e78dbc886df62a843a13c3b328b781362fecf3231902e13ba49de8a7025024\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"mcp_connection\\\",\\\"server\\\":\\\"ext-server-1\\\"},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const MCP_CONNECTION_HASH: &str = "d1e78dbc886df62a843a13c3b328b781362fecf3231902e13ba49de8a7025024";

const TOOL_CALL_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"5f874eab7e50b6b3c8bce3a9e28ff1b44d9bbd75653799316a5825077939ac3a\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"tool_call\\\",\\\"tool\\\":\\\"search\\\",\\\"allowed\\\":true},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const TOOL_CALL_HASH: &str = "5f874eab7e50b6b3c8bce3a9e28ff1b44d9bbd75653799316a5825077939ac3a";

const PERMISSION_DECISION_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"114aeef7475635e1cf87144b7870cb37a8f9db2d4d7643704a115f7d374cddcd\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"permission_decision\\\",\\\"operation\\\":\\\"ProposeWrite\\\",\\\"allowed\\\":false},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const PERMISSION_DECISION_HASH: &str = "114aeef7475635e1cf87144b7870cb37a8f9db2d4d7643704a115f7d374cddcd";

const DENIED_ACTION_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"f51abdb310271743f779b3551d5cc0f7edb9e9cf449984a3cc5f604d901357d2\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"denied_action\\\",\\\"operation\\\":\\\"ApproveProposal\\\",\\\"reason\\\":\\\"permission denied\\\"},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const DENIED_ACTION_HASH: &str = "f51abdb310271743f779b3551d5cc0f7edb9e9cf449984a3cc5f604d901357d2";

const WRITE_MEMORY_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"df6f700434c6d693e3e139680b5fa7855dc950f2420ac058565c2e71fe31160f\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"write_memory\\\",\\\"proposal_id\\\":\\\"00000000-0000-0000-0000-0000000000b1\\\"},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const WRITE_MEMORY_HASH: &str = "df6f700434c6d693e3e139680b5fa7855dc950f2420ac058565c2e71fe31160f";

const EXTERNAL_EXECUTION_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"49d13083a5388bb002aacaa2b4765ee399e12abff71b3d5d43d3dc4a9cfc263c\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"external_execution\\\",\\\"server\\\":\\\"ext-server-1\\\",\\\"tool\\\":\\\"echo\\\"},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const EXTERNAL_EXECUTION_HASH: &str = "49d13083a5388bb002aacaa2b4765ee399e12abff71b3d5d43d3dc4a9cfc263c";

const FALLBACK_OR_ERROR_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"97f7580084c2d599dd7942e3b4566e8d7ab3f870b512734ee46ac82b73c250d0\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"fallback_or_error\\\",\\\"context\\\":\\\"dense retrieval unavailable\\\"},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const FALLBACK_OR_ERROR_HASH: &str = "97f7580084c2d599dd7942e3b4566e8d7ab3f870b512734ee46ac82b73c250d0";

const APPROVAL_ATTEMPTED_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"a26ec411fb6ff864653da2789c01c382ae7792c531dd73337c9a1dd020006747\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"approval_attempted\\\",\\\"attempt_id\\\":\\\"00000000-0000-0000-0000-0000000000a1\\\",\\\"proposal_id\\\":\\\"00000000-0000-0000-0000-0000000000b1\\\"},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const APPROVAL_ATTEMPTED_HASH: &str = "a26ec411fb6ff864653da2789c01c382ae7792c531dd73337c9a1dd020006747";

const COMMIT_OUTCOME_COMMITTED_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"8ade306dcfc04b8508095d3680bbeed574122b1a4fee765aa1e57e7c77748682\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"commit_outcome\\\",\\\"attempt_id\\\":\\\"00000000-0000-0000-0000-0000000000a1\\\",\\\"outcome\\\":\\\"committed\\\"},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const COMMIT_OUTCOME_COMMITTED_HASH: &str = "8ade306dcfc04b8508095d3680bbeed574122b1a4fee765aa1e57e7c77748682";

const COMMIT_OUTCOME_GIT_PENDING_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"49fad28eb426eacf5c9f564ffcb3dde580f901226d1ce2c4d2e2ac8b9c2a8837\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"commit_outcome\\\",\\\"attempt_id\\\":\\\"00000000-0000-0000-0000-0000000000a1\\\",\\\"outcome\\\":\\\"git_pending\\\"},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const COMMIT_OUTCOME_GIT_PENDING_HASH: &str = "49fad28eb426eacf5c9f564ffcb3dde580f901226d1ce2c4d2e2ac8b9c2a8837";

const COMMIT_OUTCOME_FAILED_CONFLICT_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"0937b9ac0dc4c594ebd8615f6c1221cdd2ba0e44337884873a7d72ad9c3b9aa7\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"commit_outcome\\\",\\\"attempt_id\\\":\\\"00000000-0000-0000-0000-0000000000a1\\\",\\\"outcome\\\":{\\\"failed\\\":{\\\"category\\\":\\\"conflict\\\"}}},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const COMMIT_OUTCOME_FAILED_CONFLICT_HASH: &str = "0937b9ac0dc4c594ebd8615f6c1221cdd2ba0e44337884873a7d72ad9c3b9aa7";

const REJECTION_ATTEMPTED_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"30ce1df02fb51b3fa8dd18a74ce769f8c1dae1c6993bf891425cf1527bd57920\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"rejection_attempted\\\",\\\"attempt_id\\\":\\\"00000000-0000-0000-0000-0000000000a1\\\",\\\"proposal_id\\\":\\\"00000000-0000-0000-0000-0000000000b1\\\"},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const REJECTION_ATTEMPTED_HASH: &str = "30ce1df02fb51b3fa8dd18a74ce769f8c1dae1c6993bf891425cf1527bd57920";

const REJECTION_OUTCOME_REJECTED_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"96ab93173f0d7316932c5063419c3c2da6dd4a400729796eebcb2b075ea0a2cb\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"rejection_outcome\\\",\\\"attempt_id\\\":\\\"00000000-0000-0000-0000-0000000000a1\\\",\\\"outcome\\\":\\\"rejected\\\"},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const REJECTION_OUTCOME_REJECTED_HASH: &str = "96ab93173f0d7316932c5063419c3c2da6dd4a400729796eebcb2b075ea0a2cb";

const REJECTION_OUTCOME_FAILED_CONFLICT_LINE: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"d3526b0d05153b1fbd992b211d64deec79cc96ea9e7db93dd88d6f46f68a6a27\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"rejection_outcome\\\",\\\"attempt_id\\\":\\\"00000000-0000-0000-0000-0000000000a1\\\",\\\"outcome\\\":{\\\"failed\\\":{\\\"category\\\":\\\"conflict\\\"}}},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const REJECTION_OUTCOME_FAILED_CONFLICT_HASH: &str = "d3526b0d05153b1fbd992b211d64deec79cc96ea9e7db93dd88d6f46f68a6a27";

// --- Multi-record chain (3 records: ApprovalAttempted -> CommitOutcome(Committed) -> WriteMemory) ---

const CHAIN_LINE_0: &str = "{\"format_version\":1,\"sequence\":0,\"prev_record_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"record_hash\":\"a26ec411fb6ff864653da2789c01c382ae7792c531dd73337c9a1dd020006747\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"approval_attempted\\\",\\\"attempt_id\\\":\\\"00000000-0000-0000-0000-0000000000a1\\\",\\\"proposal_id\\\":\\\"00000000-0000-0000-0000-0000000000b1\\\"},\\\"timestamp\\\":\\\"2026-01-01T00:00:00Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const CHAIN_HASH_0: &str = "a26ec411fb6ff864653da2789c01c382ae7792c531dd73337c9a1dd020006747";

const CHAIN_LINE_1: &str = "{\"format_version\":1,\"sequence\":1,\"prev_record_hash\":\"a26ec411fb6ff864653da2789c01c382ae7792c531dd73337c9a1dd020006747\",\"record_hash\":\"e30a0278ea2f498f6e516fed869c40c4dd8965176e403a15114abd9ba46a6896\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"commit_outcome\\\",\\\"attempt_id\\\":\\\"00000000-0000-0000-0000-0000000000a1\\\",\\\"outcome\\\":\\\"committed\\\"},\\\"timestamp\\\":\\\"2026-01-01T00:00:01Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const CHAIN_HASH_1: &str = "e30a0278ea2f498f6e516fed869c40c4dd8965176e403a15114abd9ba46a6896";

const CHAIN_LINE_2: &str = "{\"format_version\":1,\"sequence\":2,\"prev_record_hash\":\"e30a0278ea2f498f6e516fed869c40c4dd8965176e403a15114abd9ba46a6896\",\"record_hash\":\"d851579f5e6a9b27478454eb5814ca312d89fad87c02e48343f5c28a14fe6b46\",\"event_json\":\"{\\\"kind\\\":{\\\"event\\\":\\\"write_memory\\\",\\\"proposal_id\\\":\\\"00000000-0000-0000-0000-0000000000b1\\\"},\\\"timestamp\\\":\\\"2026-01-01T00:00:02Z\\\",\\\"correlation_id\\\":\\\"00000000-0000-0000-0000-0000000000c1\\\"}\"}\n";
const CHAIN_HASH_2: &str = "d851579f5e6a9b27478454eb5814ca312d89fad87c02e48343f5c28a14fe6b46";

// --- GENESIS_HASH golden vector: pins the exact 64-zero-char literal
//     every other fixture above depends on as its own prev_record_hash. ---

#[test]
fn genesis_hash_matches_the_frozen_golden_literal() {
    const EXPECTED: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    assert_eq!(GENESIS_HASH, EXPECTED);
}

// --- One test per golden vector: asserts serialize_record's FRESH output
//     against the frozen literal above, byte-for-byte and hash-for-hash. ---

macro_rules! golden_vector_test {
    ($name:ident, $kind:expr, $expected_line:expr, $expected_hash:expr) => {
        #[test]
        fn $name() {
            let e = event($kind, FIXED_TS, corr());
            let (line, hash) = serialize_record(&e, 0, GENESIS_HASH);
            let line_str = String::from_utf8(line).unwrap();
            assert_eq!(line_str, $expected_line, "serialized line must match the frozen golden vector byte-for-byte");
            assert_eq!(hash, $expected_hash, "record_hash must match the frozen golden vector");
        }
    };
}

golden_vector_test!(
    golden_vector_mcp_connection,
    AuditEventKind::McpConnection { server: "ext-server-1".into() },
    MCP_CONNECTION_LINE,
    MCP_CONNECTION_HASH
);

golden_vector_test!(
    golden_vector_tool_call,
    AuditEventKind::ToolCall { tool: "search".into(), allowed: true },
    TOOL_CALL_LINE,
    TOOL_CALL_HASH
);

golden_vector_test!(
    golden_vector_permission_decision,
    AuditEventKind::PermissionDecision { operation: "ProposeWrite".into(), allowed: false },
    PERMISSION_DECISION_LINE,
    PERMISSION_DECISION_HASH
);

golden_vector_test!(
    golden_vector_denied_action,
    AuditEventKind::DeniedAction { operation: "ApproveProposal".into(), reason: "permission denied".into() },
    DENIED_ACTION_LINE,
    DENIED_ACTION_HASH
);

golden_vector_test!(
    golden_vector_write_memory,
    AuditEventKind::WriteMemory { proposal_id: PROPOSAL_ID.into() },
    WRITE_MEMORY_LINE,
    WRITE_MEMORY_HASH
);

golden_vector_test!(
    golden_vector_external_execution,
    AuditEventKind::ExternalExecution { server: "ext-server-1".into(), tool: "echo".into() },
    EXTERNAL_EXECUTION_LINE,
    EXTERNAL_EXECUTION_HASH
);

golden_vector_test!(
    golden_vector_fallback_or_error,
    AuditEventKind::FallbackOrError { context: "dense retrieval unavailable".into() },
    FALLBACK_OR_ERROR_LINE,
    FALLBACK_OR_ERROR_HASH
);

golden_vector_test!(
    golden_vector_approval_attempted,
    AuditEventKind::ApprovalAttempted { attempt_id: ATTEMPT_ID.into(), proposal_id: PROPOSAL_ID.into() },
    APPROVAL_ATTEMPTED_LINE,
    APPROVAL_ATTEMPTED_HASH
);

golden_vector_test!(
    golden_vector_commit_outcome_committed,
    AuditEventKind::CommitOutcome { attempt_id: ATTEMPT_ID.into(), outcome: CommitOutcomeKind::Committed },
    COMMIT_OUTCOME_COMMITTED_LINE,
    COMMIT_OUTCOME_COMMITTED_HASH
);

golden_vector_test!(
    golden_vector_commit_outcome_git_pending,
    AuditEventKind::CommitOutcome { attempt_id: ATTEMPT_ID.into(), outcome: CommitOutcomeKind::GitPending },
    COMMIT_OUTCOME_GIT_PENDING_LINE,
    COMMIT_OUTCOME_GIT_PENDING_HASH
);

golden_vector_test!(
    golden_vector_commit_outcome_failed_conflict,
    AuditEventKind::CommitOutcome { attempt_id: ATTEMPT_ID.into(), outcome: CommitOutcomeKind::Failed { category: CommitFailureCategory::Conflict } },
    COMMIT_OUTCOME_FAILED_CONFLICT_LINE,
    COMMIT_OUTCOME_FAILED_CONFLICT_HASH
);

golden_vector_test!(
    golden_vector_rejection_attempted,
    AuditEventKind::RejectionAttempted { attempt_id: ATTEMPT_ID.into(), proposal_id: PROPOSAL_ID.into() },
    REJECTION_ATTEMPTED_LINE,
    REJECTION_ATTEMPTED_HASH
);

golden_vector_test!(
    golden_vector_rejection_outcome_rejected,
    AuditEventKind::RejectionOutcome { attempt_id: ATTEMPT_ID.into(), outcome: RejectionOutcomeKind::Rejected },
    REJECTION_OUTCOME_REJECTED_LINE,
    REJECTION_OUTCOME_REJECTED_HASH
);

golden_vector_test!(
    golden_vector_rejection_outcome_failed_conflict,
    AuditEventKind::RejectionOutcome { attempt_id: ATTEMPT_ID.into(), outcome: RejectionOutcomeKind::Failed { category: CommitFailureCategory::Conflict } },
    REJECTION_OUTCOME_FAILED_CONFLICT_LINE,
    REJECTION_OUTCOME_FAILED_CONFLICT_HASH
);

// --- Multi-record chain: fresh serialize_record calls must reproduce the
//     frozen per-record lines/hashes, AND the chain must verify cleanly
//     end-to-end via parse_line + scan_chain. ---

#[test]
fn golden_vector_multi_record_chain_matches_frozen_lines_and_hashes() {
    let e0 = event(
        AuditEventKind::ApprovalAttempted { attempt_id: ATTEMPT_ID.into(), proposal_id: PROPOSAL_ID.into() },
        FIXED_TS,
        corr(),
    );
    let (line0, hash0) = serialize_record(&e0, 0, GENESIS_HASH);
    assert_eq!(String::from_utf8(line0.clone()).unwrap(), CHAIN_LINE_0);
    assert_eq!(hash0, CHAIN_HASH_0);

    let e1 = event(
        AuditEventKind::CommitOutcome { attempt_id: ATTEMPT_ID.into(), outcome: CommitOutcomeKind::Committed },
        FIXED_TS_1,
        corr(),
    );
    let (line1, hash1) = serialize_record(&e1, 1, &hash0);
    assert_eq!(String::from_utf8(line1.clone()).unwrap(), CHAIN_LINE_1);
    assert_eq!(hash1, CHAIN_HASH_1);

    let e2 = event(AuditEventKind::WriteMemory { proposal_id: PROPOSAL_ID.into() }, FIXED_TS_2, corr());
    let (line2, hash2) = serialize_record(&e2, 2, &hash1);
    assert_eq!(String::from_utf8(line2.clone()).unwrap(), CHAIN_LINE_2);
    assert_eq!(hash2, CHAIN_HASH_2);

    // Round trip each frozen line through parse_line and verify the whole
    // chain links cleanly via scan_chain (deterministic ordering + stable
    // encoding + parsing/verification behavior, items 5/6 of Phase 1).
    let record0 = parse_line(&line0[..line0.len() - 1]).unwrap();
    let record1 = parse_line(&line1[..line1.len() - 1]).unwrap();
    let record2 = parse_line(&line2[..line2.len() - 1]).unwrap();
    assert_eq!(record0.record_hash, CHAIN_HASH_0);
    assert_eq!(record1.record_hash, CHAIN_HASH_1);
    assert_eq!(record2.record_hash, CHAIN_HASH_2);

    let lines = vec![
        brain_api::LineOutcome::Parsed(record0),
        brain_api::LineOutcome::Parsed(record1),
        brain_api::LineOutcome::Parsed(record2),
    ];
    let scan = brain_api::scan_chain(&lines);
    assert!(scan.stop_reason.is_none(), "the frozen 3-record chain must verify cleanly end to end, got {:?}", scan.stop_reason);
    assert_eq!(scan.verified_records.len(), 3);
    assert_eq!(scan.last_verified, Some((2, CHAIN_HASH_2.to_string())));

    // Deterministic ordering / stable encoding (item 6): re-serializing
    // the same three events in the same order a second time must
    // reproduce the identical frozen lines, not merely equal hashes.
    let (line0_again, hash0_again) = serialize_record(&e0, 0, GENESIS_HASH);
    assert_eq!(line0_again, line0, "re-serializing the same event must reproduce byte-identical output");
    assert_eq!(hash0_again, hash0);
}

// --- Parsing / typed decoding of a frozen golden vector (item 5: parsing
//     and verification behavior) ---

#[test]
fn golden_vector_parses_and_decodes_back_to_the_original_typed_event() {
    let without_newline = &APPROVAL_ATTEMPTED_LINE.as_bytes()[..APPROVAL_ATTEMPTED_LINE.len() - 1];
    let record = parse_line(without_newline).unwrap();
    assert_eq!(record.record_hash, APPROVAL_ATTEMPTED_HASH);
    assert_eq!(record.sequence, 0);
    assert_eq!(record.prev_record_hash, GENESIS_HASH);

    let decoded = parse_event(&record.content_bytes).unwrap();
    assert_eq!(
        decoded.kind,
        AuditEventKind::ApprovalAttempted { attempt_id: ATTEMPT_ID.into(), proposal_id: PROPOSAL_ID.into() }
    );
    assert_eq!(decoded.timestamp, FIXED_TS);
    assert_eq!(decoded.correlation_id, corr());
}

// --- Malformed / tampered record rejection (item 8) ---

#[test]
fn a_tampered_golden_vector_hash_is_rejected_by_scan_chain() {
    // Flip the last hex digit of the frozen record_hash field -- parse_line
    // itself does not re-verify the hash (that is scan_chain/verify_link's
    // job, per this module's own documented layering), so the tamper must
    // be caught one layer up: scan_chain must reject this frozen vector's
    // tampered line as a broken record hash, never silently accept it.
    let tampered_hash = {
        let mut h = APPROVAL_ATTEMPTED_HASH.to_string();
        let last = h.pop().unwrap();
        let flipped = if last == '0' { '1' } else { '0' };
        h.push(flipped);
        h
    };
    let tampered_line = APPROVAL_ATTEMPTED_LINE.replace(APPROVAL_ATTEMPTED_HASH, &tampered_hash);
    let without_newline = &tampered_line.as_bytes()[..tampered_line.len() - 1];
    let record = parse_line(without_newline).unwrap();
    assert_eq!(record.record_hash, tampered_hash);

    let scan = brain_api::scan_chain(&[brain_api::LineOutcome::Parsed(record)]);
    assert_eq!(scan.verified_records.len(), 0, "a tampered record_hash must never be trusted into the verified prefix");
    assert!(
        matches!(
            scan.stop_reason,
            Some(brain_api::ScanStopReason::LinkBroken { at_input_index: 0, error: brain_api::ChainLinkError::BrokenRecordHash })
        ),
        "tampering the frozen golden vector's record_hash must be classified as BrokenRecordHash, got {:?}",
        scan.stop_reason
    );
}

#[test]
fn a_structurally_malformed_golden_vector_line_is_rejected() {
    // Corrupt the frozen line's JSON structure outright (not just a field
    // value) -- must fail to parse at all, never silently accepted.
    let malformed = APPROVAL_ATTEMPTED_LINE.replace('{', "");
    let result = parse_line(malformed.trim_end().as_bytes());
    assert!(result.is_err(), "a structurally malformed golden-vector-derived line must be rejected, got {result:?}");
}
