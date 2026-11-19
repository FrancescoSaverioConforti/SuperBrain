//! B5.9 Task 28: external tool authorization model.
//!
//! **Classification, authorization, and execution are three distinct
//! concerns, never conflated:**
//! - `ToolClassification` (this module) describes WHAT KIND of effect a
//!   tool has (`Read`/`Write`/`Execute`) — a fact about the tool itself.
//! - `ApprovalPolicy` (this module) describes WHAT THE OPERATOR HAS
//!   DECIDED to permit — a human/operator decision, never derived from
//!   anything the external server itself said.
//! - `authorize` (this module) is the ONE function that combines the two
//!   into a yes/no answer. It does not execute anything, does not touch
//!   a transport, does not call `discover_tools` or `connect` — it is a
//!   pure decision function over data its caller already has.
//! - Actually invoking a tool is Task 29's job (`invoke.rs`, not yet
//!   implemented). This module produces no callable path to the external
//!   server at all.
//!
//! **External tool metadata is UNTRUSTED input.** `ExternalToolMetadata`
//! is NOT something `authorize` ever derives from a `DiscoveredTool`'s
//! own server-supplied `name`/`description`/`input_schema` (Task 27's
//! `discovery` module) — there is no function anywhere in this module, or
//! called by it, that reads a `DiscoveredTool` and produces an
//! `ExternalToolMetadata` automatically. A caller (a future, separate
//! piece of SuperBrain's own operator-facing configuration, not part of
//! this plan) is responsible for constructing `ExternalToolMetadata`
//! itself, supplying `classification`/`sensitivity`/`network_required`/
//! `filesystem_scope` as its OWN judgment about the tool — a server
//! cannot make itself more trusted by what it claims about its own name
//! or description. This module enforces that boundary structurally: it
//! has no code path that reads `discovery::DiscoveredTool` at all.
//!
//! **Deny-by-default, exhaustively:**
//! - No `ApprovalPolicy` variant means "allow everything" — pinned by a
//!   compile-time-exhaustive match test with no `_` arm.
//! - An unregistered/never-authorized tool is denied, never implicitly
//!   allowed by any policy's absence.
//! - `AskApprovalRequired` always denies on its own — `authorize` alone,
//!   with no separate, explicit prior-approval record, never succeeds
//!   for this policy. (A real approval-recording mechanism is a later,
//!   separate concern — not part of this task's own scope — so today
//!   `AskApprovalRequired` simply always denies; this module does not
//!   invent a fake "pretend approval" path to make it occasionally pass.)
//! - A missing/unknown `Sensitivity` is impossible to construct by
//!   accident: `ExternalToolMetadata::sensitivity` is a required field,
//!   never an `Option` with a defaulting fallback — there is no code path
//!   where an absent sensitivity silently becomes `Public`.

use crate::McpClientError;

/// What kind of effect a tool has — a fact about the tool, established by
/// the CALLER's own judgment (never inferred from the server's own
/// name/description/schema; see this module's own doc comment on
/// untrusted metadata).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClassification {
    Read,
    Write,
    Execute,
}

/// The data-exposure sensitivity of what a tool call might see or send —
/// a distinct axis from [`ToolClassification`] (which describes
/// operational risk: does it read, write, or execute). `Sensitivity`
/// describes exposure: how sensitive is the data this tool call could
/// see or transmit, regardless of whether the tool itself reads, writes,
/// or executes. Named `Public`/`Internal`/`Sensitive` for naming
/// consistency with `domain::Classification`'s own vocabulary, but this
/// is a genuinely SEPARATE type: `mcp-client` has no dependency on
/// `domain` (see `lib.rs`'s own B5.10 isolation doc comment), and this
/// enum is never constructed from or convertible to `domain::
/// Classification` — the two crates' concepts happen to share English
/// words, nothing more.
///
/// Never inferred from an external server's own tool description/schema
/// — the same untrusted-metadata rule this module's own doc comment
/// states for `ExternalToolMetadata` as a whole. There is no `Default`
/// impl and no fallback path: a caller constructing `ExternalToolMetadata`
/// must choose one explicitly; nothing here silently becomes `Public`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sensitivity {
    Public,
    Internal,
    Sensitive,
}

/// The caller-supplied, non-server-derived facts `authorize` decides
/// against. Every field here is the CALLER's own judgment about one
/// specific tool on one specific server — never read off a
/// `discovery::DiscoveredTool` automatically (this module has no function
/// that does that conversion; see the module doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalToolMetadata {
    pub server: String,
    pub tool_name: String,
    pub classification: ToolClassification,
    pub network_required: bool,
    pub filesystem_scope: Option<String>,
    pub sensitivity: Sensitivity,
}

/// The boundaries an `AllowScopedExecution` policy permits. Only the two
/// fields `authorize`'s own checks in THIS task actually compare against
/// `ExternalToolMetadata`'s matching fields — no field is added here
/// speculatively for a check a later task might want.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeLimits {
    /// Whether a tool requiring network access may be authorized at all
    /// under this scope.
    pub allow_network: bool,
    /// The filesystem scope this policy permits, compared against
    /// [`ExternalToolMetadata::filesystem_scope`] by EXACT equality
    /// (never a prefix/subpath check — a later task's job if that
    /// semantic is ever wanted; this task does not invent it). `None`
    /// here means the scope permits ONLY a tool that itself declares no
    /// filesystem scope at all (`filesystem_scope: None`) — the narrowest,
    /// not the widest, reading of an absent limit.
    pub allowed_filesystem_scope: Option<String>,
}

/// An operator's own decision about what to permit for one tool — never
/// derived from anything the external server said about itself.
/// Deliberately has NO variant meaning "allow everything" (pinned by
/// `no_approval_policy_variant_means_allow_everything` below, a
/// compile-time-exhaustive match with no `_` arm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalPolicy {
    /// Never authorized, regardless of the tool's metadata. The
    /// unconditional deny.
    Deny,
    /// Authorizes only a [`ToolClassification::Read`]-classified tool.
    /// `Write`/`Execute` are denied regardless of any other field.
    AllowReadOnly,
    /// Always denies through `authorize` alone, by this task's own
    /// design (see the module doc comment's "Deny-by-default,
    /// exhaustively" section): a real approval-recording mechanism is
    /// out of this task's scope, so this variant exists as a distinct,
    /// named state from `Deny` (documenting operator intent: "this tool
    /// NEEDS a human to look at it," not "this tool is refused
    /// outright") without inventing a fake approval path that would let
    /// `authorize` pass on its own.
    AskApprovalRequired,
    /// Authorizes any classification, but only within `ScopeLimits`'
    /// own boundaries — denied if the tool's `network_required`/
    /// `filesystem_scope` exceeds what the scope permits.
    AllowScopedExecution(ScopeLimits),
}

/// The one function that turns (`metadata`, `policy`) into a yes/no
/// answer. Pure decision logic: no transport, no `discovery`/`connect`
/// call, no side effect. Every path through this function either returns
/// `Ok(())` (authorized) or `Err(McpClientError::PermissionDenied)` (the
/// only error variant this function ever returns) — never any other
/// `McpClientError` variant, since this function performs no I/O that
/// could fail for a different reason.
pub fn authorize(metadata: &ExternalToolMetadata, policy: &ApprovalPolicy) -> Result<(), McpClientError> {
    match policy {
        ApprovalPolicy::Deny => Err(McpClientError::PermissionDenied),
        ApprovalPolicy::AllowReadOnly => {
            if metadata.classification == ToolClassification::Read {
                Ok(())
            } else {
                Err(McpClientError::PermissionDenied)
            }
        }
        ApprovalPolicy::AskApprovalRequired => Err(McpClientError::PermissionDenied),
        ApprovalPolicy::AllowScopedExecution(scope) => {
            if metadata.network_required && !scope.allow_network {
                return Err(McpClientError::PermissionDenied);
            }
            if metadata.filesystem_scope != scope.allowed_filesystem_scope {
                return Err(McpClientError::PermissionDenied);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(classification: ToolClassification) -> ExternalToolMetadata {
        ExternalToolMetadata {
            server: "server-a".to_string(),
            tool_name: "some-tool".to_string(),
            classification,
            network_required: false,
            filesystem_scope: None,
            sensitivity: Sensitivity::Internal,
        }
    }

    // --- deny-by-default: no policy variant means "allow everything" ---

    /// Compile-time-exhaustive property: this match has no `_` arm, so it
    /// fails to COMPILE (not just fails at runtime) the moment a new
    /// `ApprovalPolicy` variant is added without updating this test —
    /// forcing a reviewer to look at it and confirm the new variant is
    /// not an "allow everything" escape hatch.
    #[test]
    fn no_approval_policy_variant_means_allow_everything() {
        fn classify(policy: &ApprovalPolicy) -> &'static str {
            match policy {
                ApprovalPolicy::Deny => "unconditional deny",
                ApprovalPolicy::AllowReadOnly => "read-only only",
                ApprovalPolicy::AskApprovalRequired => "requires approval, never passes alone",
                ApprovalPolicy::AllowScopedExecution(_) => "bounded by explicit scope limits",
            }
        }
        for policy in [ApprovalPolicy::Deny, ApprovalPolicy::AllowReadOnly, ApprovalPolicy::AskApprovalRequired, ApprovalPolicy::AllowScopedExecution(ScopeLimits { allow_network: true, allowed_filesystem_scope: None })] {
            assert_ne!(classify(&policy), "allow everything", "no variant may ever classify as unconditional allow");
        }
    }

    // --- Deny: unconditional ---

    #[test]
    fn deny_policy_denies_every_classification() {
        for classification in [ToolClassification::Read, ToolClassification::Write, ToolClassification::Execute] {
            assert_eq!(authorize(&metadata(classification), &ApprovalPolicy::Deny), Err(McpClientError::PermissionDenied));
        }
    }

    // --- AllowReadOnly: Read passes, Write/Execute denied ---

    #[test]
    fn allow_read_only_policy_authorizes_a_read_classified_tool() {
        assert_eq!(authorize(&metadata(ToolClassification::Read), &ApprovalPolicy::AllowReadOnly), Ok(()));
    }

    #[test]
    fn allow_read_only_policy_rejects_a_write_classified_tool() {
        assert_eq!(authorize(&metadata(ToolClassification::Write), &ApprovalPolicy::AllowReadOnly), Err(McpClientError::PermissionDenied));
    }

    #[test]
    fn allow_read_only_policy_rejects_an_execute_classified_tool() {
        assert_eq!(authorize(&metadata(ToolClassification::Execute), &ApprovalPolicy::AllowReadOnly), Err(McpClientError::PermissionDenied));
    }

    // --- AskApprovalRequired: authorize() alone never succeeds ---

    #[test]
    fn ask_approval_required_never_succeeds_through_authorize_alone_for_any_classification() {
        for classification in [ToolClassification::Read, ToolClassification::Write, ToolClassification::Execute] {
            assert_eq!(authorize(&metadata(classification), &ApprovalPolicy::AskApprovalRequired), Err(McpClientError::PermissionDenied), "AskApprovalRequired must never pass without a separate, explicit prior approval -- which does not exist in this task's scope");
        }
    }

    // --- AllowScopedExecution: bounded by ScopeLimits ---

    #[test]
    fn allow_scoped_execution_authorizes_within_its_limits() {
        let scope = ScopeLimits { allow_network: true, allowed_filesystem_scope: Some("/sandbox".to_string()) };
        let mut m = metadata(ToolClassification::Execute);
        m.network_required = true;
        m.filesystem_scope = Some("/sandbox".to_string());
        assert_eq!(authorize(&m, &ApprovalPolicy::AllowScopedExecution(scope)), Ok(()));
    }

    #[test]
    fn allow_scoped_execution_rejects_network_access_the_scope_does_not_permit() {
        let scope = ScopeLimits { allow_network: false, allowed_filesystem_scope: None };
        let mut m = metadata(ToolClassification::Execute);
        m.network_required = true;
        assert_eq!(authorize(&m, &ApprovalPolicy::AllowScopedExecution(scope)), Err(McpClientError::PermissionDenied));
    }

    #[test]
    fn allow_scoped_execution_rejects_a_filesystem_scope_mismatch() {
        let scope = ScopeLimits { allow_network: false, allowed_filesystem_scope: Some("/sandbox".to_string()) };
        let mut m = metadata(ToolClassification::Execute);
        m.filesystem_scope = Some("/different-path".to_string());
        assert_eq!(authorize(&m, &ApprovalPolicy::AllowScopedExecution(scope)), Err(McpClientError::PermissionDenied));
    }

    #[test]
    fn allow_scoped_execution_rejects_a_tool_with_a_filesystem_scope_when_the_policy_permits_none() {
        // allowed_filesystem_scope: None is the NARROWEST reading -- it
        // permits only a tool that itself declares NO scope, never "any
        // scope goes".
        let scope = ScopeLimits { allow_network: false, allowed_filesystem_scope: None };
        let mut m = metadata(ToolClassification::Execute);
        m.filesystem_scope = Some("/anything".to_string());
        assert_eq!(authorize(&m, &ApprovalPolicy::AllowScopedExecution(scope)), Err(McpClientError::PermissionDenied));
    }

    #[test]
    fn allow_scoped_execution_authorizes_a_tool_with_no_filesystem_scope_when_the_policy_permits_none() {
        let scope = ScopeLimits { allow_network: false, allowed_filesystem_scope: None };
        let m = metadata(ToolClassification::Execute);
        assert_eq!(authorize(&m, &ApprovalPolicy::AllowScopedExecution(scope)), Ok(()));
    }

    // --- identical tool names on different servers are independent
    //     (metadata.server is part of the caller's own judgment, never
    //     used by authorize() to look anything up implicitly -- but a
    //     distinct ExternalToolMetadata per server must still be
    //     evaluated independently, never conflated) ---

    #[test]
    fn two_tools_with_the_same_name_on_different_servers_are_authorized_independently() {
        let read_only = ApprovalPolicy::AllowReadOnly;
        let mut a = metadata(ToolClassification::Read);
        a.server = "server-a".to_string();
        a.tool_name = "echo".to_string();
        let mut b = metadata(ToolClassification::Write);
        b.server = "server-b".to_string();
        b.tool_name = "echo".to_string();

        assert_eq!(authorize(&a, &read_only), Ok(()), "server-a's Read-classified echo must be authorized");
        assert_eq!(authorize(&b, &read_only), Err(McpClientError::PermissionDenied), "server-b's Write-classified echo, despite sharing a name, must be denied independently");
    }

    // --- missing/unknown sensitivity cannot silently become Public: this
    //     is a type-level property, proven by this file's own
    //     compilation -- ExternalToolMetadata::sensitivity is a required
    //     field with no Default impl anywhere in this module, so there is
    //     no expression that constructs one without explicitly naming a
    //     Sensitivity variant ---

    #[test]
    fn sensitivity_has_no_default_and_every_variant_is_distinct() {
        // This test's real content is compile-time: `ExternalToolMetadata`
        // has no `Default` derive and `Sensitivity` has no `Default` impl
        // in this module -- there is no `..Default::default()` expression
        // this file could even write. The runtime assertions below are a
        // secondary, weaker confirmation that the three variants are at
        // least distinguishable from each other.
        assert_ne!(Sensitivity::Public, Sensitivity::Internal);
        assert_ne!(Sensitivity::Internal, Sensitivity::Sensitive);
        assert_ne!(Sensitivity::Public, Sensitivity::Sensitive);
    }

    #[test]
    fn sensitivity_is_a_distinct_type_from_domain_classification_never_coupled() {
        // Compile-time property, proven by this file's own successful
        // compilation without ever naming `domain::Classification`: this
        // module has no `use domain::...` anywhere, and `mcp-client`
        // itself has no dependency on the `domain` crate at all (see
        // `Cargo.toml` and `lib.rs`'s own B5.10 isolation doc comment) --
        // `Sensitivity::Public`/`Internal`/`Sensitive` share vocabulary
        // with `domain::Classification` by deliberate naming choice only,
        // never by shared type, conversion, or dependency.
        let _: Sensitivity = Sensitivity::Public;
    }

    // --- authorize never reads discovery::DiscoveredTool: structural
    //     proof by this file's own imports ---

    #[test]
    fn authorize_takes_no_discovered_tool_and_derives_nothing_from_server_supplied_metadata() {
        // Compile-time + structural property: this test file never
        // imports `crate::DiscoveredTool`/`crate::discovery`, and
        // `authorize`'s own signature (confirmed above,
        // `fn(&ExternalToolMetadata, &ApprovalPolicy) -> ...`) has no
        // parameter through which a `DiscoveredTool` could be passed.
        // There is no function in `permission.rs` that reads a
        // `DiscoveredTool`'s `name`/`description`/`input_schema` and
        // produces a `ToolClassification`/`Sensitivity` from them --
        // confirmed by reading `permission.rs` in full, not merely by
        // this test's own narrow signature check.
        let _: fn(&ExternalToolMetadata, &ApprovalPolicy) -> Result<(), McpClientError> = authorize;
    }
}
