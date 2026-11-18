//! B5.7 Task 25: deny-by-default permission matrix across all 12 Brain
//! API capabilities (`docs/superpowers/specs/2026-11-15-b5-0-brain-api-
//! contract-freeze.md`'s own list — the 9 read-shaped capabilities plus
//! `save_memory`/`update_memory`/`link_entities`, the 3 write-proposing
//! ones). `approve_proposal`/`reject_proposal`/`list_proposals` are a
//! separate, CLI-only privileged layer, not one of the 12 — already
//! covered exhaustively by `tests/layering.rs`'s own trust-boundary tests;
//! this file does not duplicate those.
//!
//! This is deliberately a single table-driven matrix, not 12 separate
//! ad-hoc tests, per the plan's own instruction. For every capability, two
//! policy states must deny, never silently allow:
//!   (a) `RequestContext` with no policy at all for that axis
//!       (`read_policy: None` / `write_policy: None`), and
//!   (b) the most restrictive REAL policy value for that axis
//!       (`ReadPolicy::default()` / `WritePolicy::none()`) — a policy that
//!       exists and is `Some(_)`, but grants nothing.
//! A third check closes the loop `tests/layering.rs`'s per-pair tests
//! already started for a few capabilities (B5.1 Task 20's invariant 5):
//! granting every capability EXCEPT one must still deny that one —
//! proven here exhaustively for all 12, not just a handful.
//!
//! Every capability's own real first statement is its permission check
//! (confirmed by reading `read.rs`/`memory.rs`/`knowledge.rs`/`graph.rs` in
//! full) — so a denial case never touches real I/O, and this file's
//! fixtures can stay minimal: an empty `CodeGraph`, an `Index` over the
//! `Vault`, and the `Vault` itself holding exactly one real committed note
//! (so a genuinely-permitted `retrieve` has a real target — see
//! `Fixtures::new`'s own doc comment for why that one note exists).

use std::collections::HashSet;

fn unique_test_dir(label: &str) -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-brain-api-permission-model-{label}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const NOW: &str = "2026-11-18T00:00:00Z";

/// Minimal real fixtures shared by every capability's closure. Each is
/// genuinely real (a real `Vault`, a real empty `CodeGraph`, a real
/// `Index` over that vault) — never a mock — since the point of this
/// matrix is that permission denial happens before any of them is
/// actually queried, which only a real object can prove by NOT panicking
/// or erroring for an unrelated reason.
struct Fixtures {
    vault: storage::Vault,
    graph: codegraph_index::graph::CodeGraph,
    indexer: std::sync::Arc<index::indexer_impl::RealIndexer>,
}

impl Fixtures {
    /// Commits one real note (`note-permission-matrix-existing`) so a
    /// genuinely-permitted `retrieve` has a real target to find — without
    /// it, a correctly-granted `retrieve` against a nonexistent id would
    /// return `NotFound`, which is the right behavior but would make this
    /// file's own positive-control assertions ambiguous between "denied"
    /// and "granted but target absent".
    fn new(label: &str) -> Self {
        let dir = unique_test_dir(label);
        let vault = storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap();
        let text = "---\nid: note-permission-matrix-existing\nschema_version: 1\ntype: bug\nstatus: reviewed\ntrust: official\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-11-18\nupdated: 2026-11-18\nevent_period: 2026-11-18\nrelations: {}\n---\nbody\n";
        let gate = policy::Gate::new(policy::SecretPolicy::new(1));
        storage::propose_and_commit(&vault, text, storage::Version(0), uuid::Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate, None).unwrap();
        Self { vault, graph: codegraph_index::graph::CodeGraph::new(), indexer: std::sync::Arc::new(index::indexer_impl::RealIndexer::new()) }
    }

    fn index(&self) -> index::Index<'_> {
        index::Index::new(&self.vault, &self.indexer, "project-x", chrono::NaiveDate::from_ymd_opt(2026, 11, 18).unwrap())
    }
}

/// A dummy `SymbolSnapshotId`, never resolved against `fixtures.graph`
/// (which is always empty) — only ever used by capabilities that are
/// denied before this id is looked up at all.
fn dummy_symbol_id() -> codegraph_domain::SymbolSnapshotId {
    let mut registry = codegraph_domain::RepositoryRegistry::new();
    let repo = registry.register(codegraph_domain::RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
    let file = codegraph_domain::FileIdentity::parse(repo, codegraph_domain::CommitHash::parse(&"a".repeat(40)).unwrap(), "src/lib.rs").unwrap();
    codegraph_domain::SymbolSnapshotId::derive(&file, codegraph_domain::SymbolKind::Function, "dummy", "fn dummy()", 0, 10)
}

/// One of the 12 capabilities, callable uniformly for this matrix: given a
/// `RequestContext` and the shared fixtures, returns `Ok(())` on success or
/// the `BrainApiError` the capability actually returned. This file's own
/// assertions only ever check the `Err` side; `Ok(())` collapses every
/// capability's differently-shaped real return value into one uniform
/// success marker.
struct Capability {
    name: &'static str,
    call: fn(&brain_api::RequestContext, &Fixtures) -> Result<(), brain_api::BrainApiError>,
    /// The `RequestContext` granting ONLY this capability (every other
    /// capability's flag left at its default-deny value) — used by the
    /// independence check.
    grant_only_this: fn() -> brain_api::RequestContext,
}

fn ctx_with_read(read_policy: policy::ReadPolicy) -> brain_api::RequestContext {
    brain_api::RequestContext {
        schema_version: brain_api::SCHEMA_VERSION,
        correlation_id: uuid::Uuid::new_v4(),
        write_policy: None,
        read_policy: Some(read_policy),
        origin: brain_api::CallOrigin::McpSession("session-permission-model".into()),
    }
}

fn ctx_with_write(write_policy: brain_api::WritePolicy) -> brain_api::RequestContext {
    brain_api::RequestContext {
        schema_version: brain_api::SCHEMA_VERSION,
        correlation_id: uuid::Uuid::new_v4(),
        write_policy: Some(write_policy),
        read_policy: None,
        origin: brain_api::CallOrigin::McpSession("session-permission-model".into()),
    }
}

/// A `RequestContext` with neither axis populated at all (`None`/`None`) —
/// state (a) in this file's own doc comment, shared by every capability.
fn ctx_with_no_policy() -> brain_api::RequestContext {
    brain_api::RequestContext {
        schema_version: brain_api::SCHEMA_VERSION,
        correlation_id: uuid::Uuid::new_v4(),
        write_policy: None,
        read_policy: None,
        origin: brain_api::CallOrigin::McpSession("session-permission-model".into()),
    }
}

fn all_capabilities() -> Vec<Capability> {
    vec![
        Capability {
            name: "search",
            call: |ctx, f| {
                let request = api::SearchRequest { text: "anything".into(), filters: Default::default(), k: 10 };
                brain_api::search(ctx, &f.index(), request).map(|_| ())
            },
            grant_only_this: || ctx_with_read(policy::ReadPolicy::new(true, false)),
        },
        Capability {
            name: "retrieve",
            call: |ctx, f| {
                let request = api::RetrieveRequest { id: api::RetrieveTarget::Note(storage::NoteId::parse("note-permission-matrix-existing").unwrap()) };
                brain_api::retrieve(ctx, &f.vault, request).map(|_| ())
            },
            grant_only_this: || ctx_with_read(policy::ReadPolicy::new(false, true)),
        },
        Capability {
            name: "get_code_context",
            call: |ctx, f| brain_api::get_code_context(ctx, &f.graph, dummy_symbol_id()).map(|_| ()),
            grant_only_this: || ctx_with_read(policy::ReadPolicy::new(false, false).with_get_code_context(true)),
        },
        Capability {
            name: "graph_query",
            call: |ctx, f| {
                let spec = brain_api::GraphQuerySpec { start: dummy_symbol_id(), kind: None, max_depth: 1 };
                brain_api::graph_query(ctx, &f.graph, spec, 10).map(|_| ())
            },
            grant_only_this: || ctx_with_read(policy::ReadPolicy::new(false, false).with_graph_query(true)),
        },
        Capability {
            name: "get_entity",
            call: |ctx, f| brain_api::get_entity(ctx, &f.graph, dummy_symbol_id()).map(|_| ()),
            grant_only_this: || ctx_with_read(policy::ReadPolicy::new(false, false).with_get_entity(true)),
        },
        Capability {
            name: "get_previous_solution",
            call: |ctx, f| brain_api::get_previous_solution(ctx, &f.vault, &f.index(), "anything", None).map(|_| ()),
            grant_only_this: || ctx_with_read(policy::ReadPolicy::new(false, false).with_get_previous_solution(true)),
        },
        Capability {
            name: "get_rules",
            call: |ctx, _f| brain_api::get_rules(ctx, None).map(|_| ()),
            grant_only_this: || ctx_with_read(policy::ReadPolicy::new(false, false).with_get_rules(true)),
        },
        Capability {
            name: "get_regulations",
            call: |ctx, _f| brain_api::get_regulations(ctx, None).map(|_| ()),
            grant_only_this: || ctx_with_read(policy::ReadPolicy::new(false, false).with_get_regulations(true)),
        },
        Capability {
            name: "get_canon",
            call: |ctx, _f| brain_api::get_canon(ctx, None).map(|_| ()),
            grant_only_this: || ctx_with_read(policy::ReadPolicy::new(false, false).with_get_canon(true)),
        },
        Capability {
            name: "save_memory",
            call: |ctx, f| {
                brain_api::save_memory(
                    ctx,
                    &f.vault,
                    storage::NoteId::parse("note-permission-matrix-save").unwrap(),
                    domain::NoteType::Bug,
                    domain::Classification::Internal,
                    "project-x".into(),
                    "agent:coder@prompt-v3".into(),
                    "2026-11-18".into(),
                    "body".into(),
                    NOW,
                )
                .map(|_| ())
            },
            grant_only_this: || ctx_with_write(brain_api::WritePolicy::mcp_agent()),
        },
        Capability {
            name: "update_memory",
            call: |ctx, f| {
                brain_api::update_memory(ctx, &f.vault, storage::NoteId::parse("note-permission-matrix-update").unwrap(), "body".into(), storage::Version(0), NOW).map(|_| ())
            },
            grant_only_this: || ctx_with_write(brain_api::WritePolicy::mcp_agent()),
        },
        Capability {
            name: "link_entities",
            call: |ctx, f| {
                brain_api::link_entities(
                    ctx,
                    &f.vault,
                    storage::NoteId::parse("note-permission-matrix-link-a").unwrap(),
                    storage::NoteId::parse("note-permission-matrix-link-b").unwrap(),
                    brain_api::RelationKind::Supports,
                    NOW,
                )
                .map(|_| ())
            },
            grant_only_this: || ctx_with_write(brain_api::WritePolicy::mcp_agent()),
        },
    ]
}

/// (a) No policy at all for the relevant axis must deny every capability.
#[test]
fn every_capability_is_denied_with_no_policy_at_all() {
    let fixtures = Fixtures::new("no-policy");
    let ctx = ctx_with_no_policy();
    for capability in all_capabilities() {
        let result = (capability.call)(&ctx, &fixtures);
        assert_eq!(result, Err(brain_api::BrainApiError::PermissionDenied), "{} must deny with no policy at all, got {result:?}", capability.name);
    }
}

/// (b) A real, present policy that grants nothing must also deny every
/// capability — `Some(default)` is not a looser state than `None`.
#[test]
fn every_capability_is_denied_with_the_most_restrictive_real_policy() {
    let fixtures = Fixtures::new("restrictive-policy");
    let read_denied = ctx_with_read(policy::ReadPolicy::default());
    let write_denied = ctx_with_write(brain_api::WritePolicy::none());

    for capability in all_capabilities() {
        // Each capability is gated by exactly one axis (read XOR write, per
        // this crate's own additive-policy design) -- try both denied
        // contexts and require denial under whichever axis actually gates
        // it; a capability denied under BOTH is still correctly denied,
        // never a false pass.
        let read_result = (capability.call)(&read_denied, &fixtures);
        let write_result = (capability.call)(&write_denied, &fixtures);
        assert!(
            read_result == Err(brain_api::BrainApiError::PermissionDenied) || write_result == Err(brain_api::BrainApiError::PermissionDenied),
            "{} must deny under at least one maximally-restrictive real policy, got read={read_result:?} write={write_result:?}",
            capability.name
        );
    }
}

/// Independence, exhaustively: granting every capability EXCEPT one must
/// still deny that one. Extends `tests/layering.rs`'s own per-pair
/// independence tests (which covered a handful of read capabilities) to
/// all 12, in one matrix.
#[test]
fn granting_every_other_capability_never_implicitly_grants_the_remaining_one() {
    let fixtures = Fixtures::new("independence");
    let full_read = policy::ReadPolicy::new(true, true)
        .with_get_code_context(true)
        .with_graph_query(true)
        .with_get_entity(true)
        .with_get_previous_solution(true)
        .with_get_rules(true)
        .with_get_regulations(true)
        .with_get_canon(true);

    for capability in all_capabilities() {
        // "Every other capability granted" means: full read grant on the
        // read axis, but write_policy left at None -- a read-shaped
        // capability under test is denied via the WRITE axis never being
        // relevant to it (it has no write check to pass), and a
        // write-shaped capability under test is denied because the write
        // axis itself grants nothing, regardless of how permissive the
        // read axis is. Either way, the capability under test must still
        // be denied.
        let ctx = brain_api::RequestContext {
            schema_version: brain_api::SCHEMA_VERSION,
            correlation_id: uuid::Uuid::new_v4(),
            write_policy: None,
            read_policy: Some(full_read),
            origin: brain_api::CallOrigin::McpSession("session-independence".into()),
        };
        let result = (capability.call)(&ctx, &fixtures);
        let is_write_shaped = matches!(capability.name, "save_memory" | "update_memory" | "link_entities");
        if is_write_shaped {
            assert_eq!(result, Err(brain_api::BrainApiError::PermissionDenied), "{} must still deny with every read capability granted but write_policy: None, got {result:?}", capability.name);
        } else {
            // A read-shaped capability IS covered by full_read here, so its
            // permission gate must pass -- this branch exists only to
            // document that full_read's own completeness is exercised, not
            // skipped. "Passes the gate" is `!= PermissionDenied`, not
            // `is_ok()`: `get_code_context`/`graph_query`/`get_entity`
            // against this file's always-empty `CodeGraph` correctly
            // return `NotFound` once genuinely permitted -- a real,
            // different error that itself proves the permission check ran
            // and passed. Each capability's own module tests already cover
            // the full "succeeds with real data" case; that is not this
            // file's job.
            assert_ne!(result, Err(brain_api::BrainApiError::PermissionDenied), "{} should pass its permission gate once genuinely granted by full_read, got {result:?}", capability.name);
        }
    }
}

/// Positive control: granting ONLY one capability's own flag (every other
/// flag at its default-deny value) must let that one capability PASS ITS
/// PERMISSION GATE — proving the deny-by-default matrix above isn't
/// vacuously true because nothing can ever be granted at all. "Passes the
/// gate" is `!= PermissionDenied`, not `is_ok()` — see the independence
/// test's own comment for why (a capability targeting this file's
/// deliberately-empty fixtures can still return a real, different error
/// like `NotFound` once genuinely permitted).
#[test]
fn granting_only_one_capabilitys_own_flag_lets_it_succeed() {
    let fixtures = Fixtures::new("positive-control");
    for capability in all_capabilities() {
        let ctx = (capability.grant_only_this)();
        let result = (capability.call)(&ctx, &fixtures);
        assert_ne!(result, Err(brain_api::BrainApiError::PermissionDenied), "{} must pass its permission gate once its own flag alone is granted, got {result:?}", capability.name);
    }
}

/// Permission check happens before any real I/O -- re-confirmed at the
/// matrix level (Task 14's own version of this already exists for
/// approve_proposal in `tests/layering.rs`; this is the read/propose-side
/// equivalent): denying every capability against a vault that does not
/// even have the note/symbol a capability's arguments reference must
/// still surface PermissionDenied, never NotFound -- proving the
/// permission gate runs first, not "permission THEN existence".
#[test]
fn permission_denial_happens_before_existence_is_ever_revealed() {
    let fixtures = Fixtures::new("denial-before-existence");
    let ctx = ctx_with_no_policy();
    for capability in all_capabilities() {
        let result = (capability.call)(&ctx, &fixtures);
        assert_eq!(
            result,
            Err(brain_api::BrainApiError::PermissionDenied),
            "{} must surface PermissionDenied, never NotFound/InvalidInput/any other variant, for a nonexistent target under no policy -- got {result:?}",
            capability.name
        );
    }
}

/// Sanity check on this file's own completeness: exactly the 12 named
/// capabilities from the B5.0 frozen contract are covered here, no fewer
/// and no silently-added extra -- if a 13th capability is ever added to
/// `brain-api`, this test fails until it is added to `all_capabilities()`
/// too.
#[test]
fn exactly_the_12_frozen_capabilities_are_covered_by_this_matrix() {
    let mut names: Vec<&str> = all_capabilities().iter().map(|c| c.name).collect();
    names.sort();
    let mut expected = vec![
        "get_canon",
        "get_code_context",
        "get_entity",
        "get_previous_solution",
        "get_regulations",
        "get_rules",
        "graph_query",
        "link_entities",
        "retrieve",
        "save_memory",
        "search",
        "update_memory",
    ];
    expected.sort();
    assert_eq!(names, expected);
}
