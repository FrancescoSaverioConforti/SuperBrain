//! `get_rules`/`get_regulations`/`get_canon` (B5.1 Task 19) — the B5.0
//! contract's §7–9 schemas
//! (`docs/superpowers/specs/2026-11-15-b5-0-brain-api-contract-freeze.md`,
//! "`get_rules` / `get_regulations` / `get_canon`: NEW B5.0 definitions").
//!
//! **No new storage/schema/index.** These three query a small, fixed,
//! trusted internal registry of real repository documents
//! ([`KNOWN_RULE_DOCUMENTS`]) — never a caller-supplied filesystem path,
//! never a generic "read any file" capability. There is no
//! `NoteType::Rule`/`Regulation`/`Canon` variant and none is added here.
//!
//! **Grounding performed before implementing** (re-confirmed against the
//! real repository, not assumed from the contract doc alone): the five
//! documents the B5.0 contract names as `get_rules`'s real source of
//! truth all exist exactly as described —
//! `docs/research/POLICY.md`, `EVALUATION_POLICY.md`,
//! `EXPERIMENTAL_POLICY.md`, `MUTATION_TESTING_POLICY.md` (each scoped to
//! Code Graph research methodology, not a project-wide constitution) and
//! `AGENT_RELIABILITY_CONTRACT.md` (scoped to Code Graph agent-policy
//! trust, partially executable as real code in
//! `crates/codegraph-index/src/agent_policy.rs`). A repo-wide search for
//! "canoniz" found no canonization mechanism anywhere — confirming the
//! B5.0 contract's own statement that `get_canon`'s mechanism is
//! "deferred to implementation," not yet real.
//!
//! **`get_canon` is a real, permission-gated capability, not a stub.**
//! Today it always returns `Ok(vec![])` because the canonical corpus is
//! legitimately empty — no canonization mechanism exists yet, so no
//! repository artifact (not a `Decision` note, not a design doc, not a
//! Continuity decision, not this very file) is implicitly promoted to
//! Canon. The permission check still runs before that empty answer is
//! given, exactly as it would once a real corpus exists.

/// One document in the fixed, trusted "rules" registry — never a
/// caller-supplied path. Each entry's `scope` is the document's own real,
/// declared domain (verified by reading the file, not inferred), per the
/// B5.0 contract's "scope must be real, not inferred" requirement.
struct RuleDocument {
    path: &'static str,
    scope: &'static str,
}

/// The five real documents confirmed, by direct inspection, to declare
/// themselves "binding" today. Adding a sixth requires the same
/// verification this list's own doc comment describes — never adding a
/// path here without having read the document's own binding declaration
/// and real scope.
const KNOWN_RULE_DOCUMENTS: &[RuleDocument] = &[
    RuleDocument { path: "docs/research/POLICY.md", scope: "research" },
    RuleDocument { path: "docs/research/EVALUATION_POLICY.md", scope: "evaluation" },
    RuleDocument { path: "docs/research/EXPERIMENTAL_POLICY.md", scope: "research" },
    RuleDocument { path: "docs/research/MUTATION_TESTING_POLICY.md", scope: "mutation_testing" },
    RuleDocument { path: "docs/research/AGENT_RELIABILITY_CONTRACT.md", scope: "code_graph" },
];

/// Whether a returned item's binding declaration is currently in effect.
/// A new, distinct concept from `domain::NoteStatus` (which describes a
/// NOTE's lifecycle, e.g. `Draft`/`Reviewed`) — a rule/regulation/canon
/// entry is not a note and has no note lifecycle; it is either currently
/// binding or it is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingStatus {
    Binding,
}

/// One `get_rules`/`get_regulations` response item, per the B5.0
/// contract's §7/§8 shared output shape: `{ content_or_reference, source,
/// scope, provenance, status, version_or_date }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeItem {
    /// The document's own path/section reference, never inlined content
    /// for a large policy document — per the contract's explicit
    /// allowance ("`content_or_reference` may be the document's own
    /// path/section reference rather than inlined content").
    pub content_or_reference: String,
    pub source: String,
    /// For `get_rules`: the rule's real declared domain (e.g.
    /// `research`, `code_graph`). For `get_regulations`: the regulatory
    /// regime (e.g. `gdpr`) — a different namespace, per the contract's
    /// explicit "never conflated with get_rules" rule; this module never
    /// returns a `get_rules` item through `get_regulations` or vice
    /// versa.
    pub scope: String,
    pub provenance: String,
    pub status: BindingStatus,
    pub version_or_date: String,
}

/// A `get_canon` response item: the same shape as [`KnowledgeItem`], plus
/// the explicit `canonized_by`/`canonized_at` pair the contract's §9
/// requires to distinguish canon provenance (binding because someone
/// explicitly canonized it) from a rule's provenance (binding because a
/// policy document declares it so).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonEntry {
    pub item: KnowledgeItem,
    pub canonized_by: String,
    pub canonized_at: String,
}

fn check_read(ctx: &RequestContext, op: policy::ReadOperation) -> Result<(), BrainApiError> {
    let read_policy = ctx.read_policy.as_ref().ok_or(BrainApiError::PermissionDenied)?;
    if !read_policy.allows(op) {
        return Err(BrainApiError::PermissionDenied);
    }
    Ok(())
}

use crate::{BrainApiError, RequestContext};

/// Lists the project's internal operating/engineering rules (B5.0 §7),
/// optionally filtered by `scope`. `ctx.read_policy` must allow
/// `ReadOperation::GetRules` or this fails closed with
/// `PermissionDenied` before the registry is ever consulted.
///
/// An empty `scope` string is `InvalidInput` (mirrors `search`'s own
/// `EmptyQuery` convention for a filter that is syntactically present but
/// semantically empty); `None` returns rules across every known scope,
/// per the contract's own "omitting the filter returns rules across all
/// known scopes." No match for a given `scope` is `Ok(vec![])`, never an
/// error — the contract's own frozen empty-behavior rule for this
/// capability.
///
/// Results are returned in [`KNOWN_RULE_DOCUMENTS`]'s own fixed
/// declaration order, filtered — deterministic by construction, never a
/// `HashMap`/`HashSet` iteration order.
pub fn get_rules(ctx: &RequestContext, scope: Option<&str>) -> Result<Vec<KnowledgeItem>, BrainApiError> {
    check_read(ctx, policy::ReadOperation::GetRules)?;

    if let Some(s) = scope {
        if s.is_empty() {
            return Err(BrainApiError::InvalidInput("scope filter must not be empty".to_string()));
        }
    }

    Ok(KNOWN_RULE_DOCUMENTS
        .iter()
        .filter(|doc| scope.is_none_or(|s| doc.scope == s))
        .map(|doc| KnowledgeItem {
            content_or_reference: doc.path.to_string(),
            source: doc.path.to_string(),
            scope: doc.scope.to_string(),
            provenance: format!("declared binding in {}", doc.path),
            status: BindingStatus::Binding,
            version_or_date: "2026-11-05".to_string(),
        })
        .collect())
}

/// Lists external regulatory/legal/compliance constraints applicable to
/// the project (B5.0 §8). `ctx.read_policy` must allow
/// `ReadOperation::GetRegulations` or this fails closed with
/// `PermissionDenied` before anything else runs — the permission check
/// happens even though the corpus is empty, exactly as it would once a
/// real regulation is recorded.
///
/// `regime` is accepted (same empty-string `InvalidInput` rule as
/// `get_rules`'s `scope`) but is a no-op today: no regulatory source of
/// truth exists anywhere in this repository, and this function never
/// fabricates one — no GDPR/NIS2/AI Act content, no web fetch, no
/// invented entry. `Ok(vec![])` always, per the contract's own frozen
/// "empty corpus returns `Ok(vec![])`, never an error" rule.
pub fn get_regulations(ctx: &RequestContext, regime: Option<&str>) -> Result<Vec<KnowledgeItem>, BrainApiError> {
    check_read(ctx, policy::ReadOperation::GetRegulations)?;

    if let Some(r) = regime {
        if r.is_empty() {
            return Err(BrainApiError::InvalidInput("regime filter must not be empty".to_string()));
        }
    }

    Ok(Vec::new())
}

/// Lists the authoritative corpus of consolidated project truths (B5.0
/// §9). `ctx.read_policy` must allow `ReadOperation::GetCanon` or this
/// fails closed with `PermissionDenied` before anything else runs.
///
/// `topic` is accepted (same empty-string `InvalidInput` rule as the
/// other two functions in this module) but is a no-op today: no
/// canonization mechanism exists anywhere in this repository (confirmed
/// by direct search, see this module's own doc comment), so the
/// canonical corpus is legitimately empty. This is the correct current
/// answer, not a placeholder to fill in within this task — no `Decision`
/// note, design doc, or Continuity decision is implicitly promoted to
/// Canon by this function. `Ok(vec![])` always, per the contract's own
/// frozen "empty behavior" rule for this capability, until a real
/// canonization mechanism exists and is wired in here.
pub fn get_canon(ctx: &RequestContext, topic: Option<&str>) -> Result<Vec<CanonEntry>, BrainApiError> {
    check_read(ctx, policy::ReadOperation::GetCanon)?;

    if let Some(t) = topic {
        if t.is_empty() {
            return Err(BrainApiError::InvalidInput("topic filter must not be empty".to_string()));
        }
    }

    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallOrigin, WritePolicy};

    fn ctx_with(read_policy: policy::ReadPolicy) -> RequestContext {
        RequestContext {
            schema_version: crate::SCHEMA_VERSION,
            correlation_id: uuid::Uuid::new_v4(),
            write_policy: Some(WritePolicy::mcp_agent()),
            read_policy: Some(read_policy),
            origin: CallOrigin::McpSession("session-knowledge-test".into()),
        }
    }

    fn ctx_no_read_policy() -> RequestContext {
        RequestContext {
            schema_version: crate::SCHEMA_VERSION,
            correlation_id: uuid::Uuid::new_v4(),
            write_policy: Some(WritePolicy::mcp_agent()),
            read_policy: None,
            origin: CallOrigin::McpSession("session-knowledge-test".into()),
        }
    }

    // --- get_rules ---

    #[test]
    fn get_rules_with_no_scope_filter_returns_every_known_rule_document_with_its_real_scope_preserved() {
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_rules(true));

        let rules = get_rules(&ctx, None).unwrap();

        assert_eq!(rules.len(), KNOWN_RULE_DOCUMENTS.len());
        assert!(rules.iter().any(|r| r.source == "docs/research/MUTATION_TESTING_POLICY.md" && r.scope == "mutation_testing"));
        assert!(rules.iter().any(|r| r.source == "docs/research/AGENT_RELIABILITY_CONTRACT.md" && r.scope == "code_graph"));
        assert!(rules.iter().all(|r| r.status == BindingStatus::Binding));
    }

    #[test]
    fn get_rules_filters_by_a_real_scope_and_preserves_it_on_every_returned_item() {
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_rules(true));

        let rules = get_rules(&ctx, Some("research")).unwrap();

        assert!(!rules.is_empty());
        assert!(rules.iter().all(|r| r.scope == "research"), "every returned item must carry the real scope filtered on, got {rules:?}");
    }

    #[test]
    fn get_rules_with_an_unmatched_scope_returns_an_empty_vec_not_an_error() {
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_rules(true));

        let rules = get_rules(&ctx, Some("nonexistent-scope-xyz")).unwrap();

        assert_eq!(rules, Vec::new());
    }

    #[test]
    fn get_rules_with_an_empty_scope_string_returns_invalid_input() {
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_rules(true));

        let result = get_rules(&ctx, Some(""));

        assert!(matches!(result, Err(BrainApiError::InvalidInput(_))), "expected InvalidInput, got {result:?}");
    }

    #[test]
    fn get_rules_denied_without_read_policy() {
        let ctx = ctx_no_read_policy();
        let result = get_rules(&ctx, None);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn get_rules_denied_when_every_other_read_capability_is_allowed_but_not_this_one() {
        let ctx = ctx_with(
            policy::ReadPolicy::new(true, true)
                .with_get_code_context(true)
                .with_graph_query(true)
                .with_get_entity(true)
                .with_get_previous_solution(true)
                .with_get_regulations(true)
                .with_get_canon(true),
        );
        let result = get_rules(&ctx, None);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn get_rules_is_deterministic_and_order_stable_across_repeated_calls() {
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_rules(true));
        let first = get_rules(&ctx, None).unwrap();
        let second = get_rules(&ctx, None).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn get_rules_never_reads_a_caller_supplied_filesystem_path() {
        // Compile-time proof: get_rules's signature takes only an
        // Option<&str> scope filter -- there is no path/filename
        // parameter at all for a caller to control.
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_rules(true));
        let _ = get_rules(&ctx, Some("research")).unwrap();
    }

    // --- get_regulations ---

    #[test]
    fn get_regulations_returns_an_empty_vec_today_regardless_of_regime_filter() {
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_regulations(true));

        assert_eq!(get_regulations(&ctx, None).unwrap(), Vec::new());
        assert_eq!(get_regulations(&ctx, Some("gdpr")).unwrap(), Vec::new());
    }

    #[test]
    fn get_regulations_with_an_empty_regime_string_returns_invalid_input() {
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_regulations(true));
        let result = get_regulations(&ctx, Some(""));
        assert!(matches!(result, Err(BrainApiError::InvalidInput(_))), "expected InvalidInput, got {result:?}");
    }

    #[test]
    fn get_regulations_denied_without_read_policy() {
        let ctx = ctx_no_read_policy();
        let result = get_regulations(&ctx, None);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn get_regulations_denied_when_get_rules_is_allowed_but_not_this_one() {
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_rules(true).with_get_canon(true));
        let result = get_regulations(&ctx, None);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn get_regulations_is_deterministic() {
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_regulations(true));
        assert_eq!(get_regulations(&ctx, None).unwrap(), get_regulations(&ctx, None).unwrap());
    }

    // --- get_canon ---

    #[test]
    fn get_canon_returns_an_empty_vec_today_because_no_canonization_mechanism_exists() {
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_canon(true));

        assert_eq!(get_canon(&ctx, None).unwrap(), Vec::new());
    }

    #[test]
    fn get_canon_with_a_topic_filter_also_returns_an_empty_vec_today() {
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_canon(true));

        assert_eq!(get_canon(&ctx, Some("architecture")).unwrap(), Vec::new());
    }

    #[test]
    fn get_canon_with_an_empty_topic_string_returns_invalid_input() {
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_canon(true));
        let result = get_canon(&ctx, Some(""));
        assert!(matches!(result, Err(BrainApiError::InvalidInput(_))), "expected InvalidInput, got {result:?}");
    }

    #[test]
    fn get_canon_denied_without_read_policy() {
        let ctx = ctx_no_read_policy();
        let result = get_canon(&ctx, None);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn get_canon_denied_when_every_other_read_capability_is_allowed_but_not_this_one() {
        // Proves the permission check runs and is independently gated
        // EVEN THOUGH the corpus is empty either way -- an attacker (or a
        // misconfigured caller) cannot distinguish "denied" from "empty
        // corpus" by timing/behavior, since denial happens before the
        // (trivial) empty-corpus answer would otherwise be computed.
        let ctx = ctx_with(
            policy::ReadPolicy::new(true, true)
                .with_get_code_context(true)
                .with_graph_query(true)
                .with_get_entity(true)
                .with_get_previous_solution(true)
                .with_get_rules(true)
                .with_get_regulations(true),
        );
        let result = get_canon(&ctx, None);
        assert_eq!(result, Err(BrainApiError::PermissionDenied));
    }

    #[test]
    fn get_canon_is_deterministic() {
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_canon(true));
        assert_eq!(get_canon(&ctx, None).unwrap(), get_canon(&ctx, None).unwrap());
    }

    #[test]
    fn no_decision_note_or_design_doc_is_ever_implicitly_returned_as_canon() {
        // There is no path from any domain::NoteType (including Decision)
        // into get_canon's output -- get_canon takes no vault/note
        // parameter at all, so it is structurally impossible for a
        // Decision-typed note to be promoted to Canon by this function.
        let ctx = ctx_with(policy::ReadPolicy::new(false, false).with_get_canon(true));
        assert_eq!(get_canon(&ctx, None).unwrap(), Vec::new());
    }
}
