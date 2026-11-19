//! `run_cli` (B5.1 Task 15): the minimal CLI approval surface's logic,
//! extracted from the thin `examples/approve_cli.rs` binary so it is
//! testable as a library call (`crates/brain-api/tests/cli_approval_e2e.rs`)
//! without spawning a subprocess. Per the design doc's own scope limit —
//! "NON costruire la UI in B5; CLI approval minima se necessaria per
//! verificare E2E" — this is deliberately a 3-command (`list`/`approve`/
//! `reject`) surface, nothing more: no flags, no config file, no new error
//! category, no capability beyond the 12 frozen ones plus the
//! approve/reject pair Task 14 already implemented.
//!
//! **Trust boundary**: `run_cli` is itself the trusted local CLI entry
//! point the B5.0 contract's trust model names (`docs/superpowers/specs/
//! 2026-11-15-b5-0-brain-api-contract-freeze.md`'s "Only a trusted human
//! approval surface (CLI now; B6 UI later) may move a proposal to
//! Approved/Rejected"). It is the ONE place in this crate that constructs
//! `RequestContext { write_policy: Some(WritePolicy::trusted_cli()),
//! origin: CallOrigin::TrustedLocal(TrustedLocalToken::new()), .. }` —
//! never from `args`, never from any value this function receives from its
//! caller. `args` can only ever name a subcommand and a proposal id/reason;
//! there is no argument shape through which a caller could set
//! `write_policy` or `origin` to anything else, because this function's
//! own signature has no such parameter and builds the trusted context
//! itself, every call, unconditionally. Both `WritePolicy::trusted_cli`
//! and `TrustedLocalToken::new` are `pub(crate)` (B5 layering review): this
//! module is the only place in the crate's public surface that can reach
//! either, and nothing outside `brain-api` — including a future
//! `mcp-server` crate — can call either one at all.

use crate::audit::{allowed_from_result, record_permission_decision, record_tool_call, AuditSink, NullAuditSink};
use crate::{BrainApiError, CallOrigin, ProposalId, RequestContext, TrustedLocalToken, WritePolicy};

fn trusted_ctx() -> RequestContext {
    RequestContext {
        schema_version: crate::SCHEMA_VERSION,
        correlation_id: uuid::Uuid::new_v4(),
        write_policy: Some(WritePolicy::trusted_cli()),
        // This CLI surface only ever calls approve_proposal/reject_proposal/
        // list_proposals (Task 14/15) -- none of which check read_policy --
        // so it has no need for read permission. Left None (principle of
        // least privilege): this context can never be handed to search/
        // retrieve/get_code_context and have it silently succeed.
        read_policy: None,
        origin: CallOrigin::TrustedLocal(TrustedLocalToken::new()),
    }
}

/// Runs one CLI command against `vault`, returning a human-readable
/// result string on success. `now` is the caller's own timestamp source
/// (the thin example binary uses the real wall clock; tests use a fixed
/// string) — never parsed out of `args` itself.
///
/// Recognized commands (exactly these three; anything else is
/// `InvalidInput`, never a panic):
/// - `list`: lists every pending/expired-at-list-time proposal.
/// - `approve <id>`: approves and executes the named proposal.
/// - `reject <id> <reason>`: rejects the named proposal with `reason`.
pub fn run_cli(args: &[String], vault: &storage::Vault, now: &str) -> Result<String, BrainApiError> {
    run_cli_with_audit(args, vault, now, &NullAuditSink)
}

/// Same as [`run_cli`], recording one `ToolCall`/`PermissionDecision`/
/// `WriteMemory` event per command to `sink` (B5.12 Task 33) — `run_cli`
/// itself keeps its exact, separately-pinned `(args, vault, now)` signature
/// (`tests/layering.rs`'s own compile-time property test) and is now a
/// thin wrapper over this function with [`NullAuditSink`], so every
/// existing caller of `run_cli` is unaffected. Each command gets its own
/// fresh `correlation_id` (a CLI invocation has no `RequestContext` to
/// reuse across multiple commands the way one MCP tool call does — `list`/
/// `approve`/`reject` are each their own trusted-local call here).
pub fn run_cli_with_audit(args: &[String], vault: &storage::Vault, now: &str, sink: &dyn AuditSink) -> Result<String, BrainApiError> {
    let ctx = trusted_ctx();
    let result = match args {
        [cmd] if cmd == "list" => run_list(&ctx, vault, now),
        [cmd, id] if cmd == "approve" => run_approve(&ctx, vault, id, now, sink),
        [cmd, id, reason] if cmd == "reject" => run_reject(&ctx, vault, id, reason.clone(), now, sink),
        [] => Err(BrainApiError::InvalidInput("no command given — expected one of: list, approve <id>, reject <id> <reason>".to_string())),
        [cmd, ..] => Err(BrainApiError::InvalidInput(format!("unknown command '{cmd}' — expected one of: list, approve <id>, reject <id> <reason>"))),
    };
    let tool = args.first().map(String::as_str).unwrap_or("unknown");
    record_tool_call(sink, tool, allowed_from_result(&result), now, ctx.correlation_id);
    result
}

fn run_list(ctx: &RequestContext, vault: &storage::Vault, now: &str) -> Result<String, BrainApiError> {
    let proposals = crate::list_proposals(ctx, vault, now)?;
    if proposals.is_empty() {
        return Ok("no proposals".to_string());
    }
    let mut lines = Vec::with_capacity(proposals.len());
    for proposal in &proposals {
        lines.push(format!("{} [{:?}] from {}", proposal.id, proposal.status(), proposal.originating_identity));
    }
    Ok(lines.join("\n"))
}

fn run_approve(ctx: &RequestContext, vault: &storage::Vault, id: &str, now: &str, sink: &dyn AuditSink) -> Result<String, BrainApiError> {
    let id = parse_id(id)?;
    let result = crate::approve_proposal(ctx, vault, &id, now);
    record_permission_decision(sink, "ApproveProposal", allowed_from_result(&result), Some("permission denied"), now, ctx.correlation_id);
    let result = result?;
    Ok(format!("approved {id}: {result:?}"))
}

fn run_reject(ctx: &RequestContext, vault: &storage::Vault, id: &str, reason: String, now: &str, sink: &dyn AuditSink) -> Result<String, BrainApiError> {
    let id = parse_id(id)?;
    let result = crate::reject_proposal(ctx, vault, &id, reason, now);
    record_permission_decision(sink, "RejectProposal", allowed_from_result(&result), Some("permission denied"), now, ctx.correlation_id);
    result?;
    Ok(format!("rejected {id}"))
}

fn parse_id(raw: &str) -> Result<ProposalId, BrainApiError> {
    ProposalId::parse(raw).map_err(|_| BrainApiError::InvalidInput("not a valid proposal id".to_string()))
}
