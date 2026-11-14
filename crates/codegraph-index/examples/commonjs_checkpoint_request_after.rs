//! CommonJS Extraction Checkpoint: AFTER remeasurement on the real
//! `request/request` corpus, same pinned commit as B2-EVAL-010
//! (`3c0cddc7c8eb60b470e9519da85896ed7ee0081e`), through the REAL,
//! fully-wired production entry point
//! (`resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge`,
//! which now includes `resolve_js_prototype_ownership`) rather than
//! B2-EVAL-010's own narrower `resolve_calls_with_imports`-only call —
//! this is the real indexer's own resolver set, not a subset.
//!
//! Usage: `cargo run --release --example commonjs_checkpoint_request_after -- <repo_path> <commit>`

use std::collections::HashMap;
use std::time::Instant;

use codegraph_domain::{CommitHash, EdgeKind, EvidenceTier, RepositorySource, ResolutionMethod, SymbolKind};
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::edges::resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge;
use codegraph_index::extract::extract_symbols_reporting_rejections;
use codegraph_index::imports::{extract_import_statements, resolve_raw_import_statements, RawImportStatement};
use codegraph_index::local_git_source::LocalGitRepositorySource;
use codegraph_index::negative_knowledge::{CallAttempt, UnresolvedReason};
use codegraph_index::parse::parser_for;
use codegraph_index::snapshot::promote;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: commonjs_checkpoint_request_after <repo_path> <commit>");
        std::process::exit(2);
    }
    let repo_path = std::fs::canonicalize(&args[1]).expect("repo_path must exist");
    let commit_str = &args[2];

    let mut allowlist = RepositoryAllowlist::new();
    let allowlisted_root = allowlist.authorize(&repo_path).expect("authorize repo root");
    let mut registry = codegraph_domain::RepositoryRegistry::new();
    let repo_root = codegraph_domain::RepositoryRoot::parse(&repo_path).unwrap();
    let repo_id = registry.register(repo_root);

    let root_for_id = move |id: codegraph_domain::RepositoryId| -> Option<codegraph_index::allowlist::AllowlistedRoot> {
        if id == repo_id {
            Some(allowlisted_root.clone())
        } else {
            None
        }
    };
    let source = LocalGitRepositorySource::new(&allowlist, &root_for_id);
    let commit = CommitHash::parse(commit_str).expect("commit must be a valid SHA-1");

    println!("=== CommonJS Extraction Checkpoint: AFTER remeasurement (request/request) ===");
    println!("repo: {}", repo_path.display());
    println!("commit: {commit_str}");

    let t0 = Instant::now();
    let all_files = source.list_files_at(repo_id, &commit).expect("list_files_at");
    let files: Vec<String> = all_files.into_iter().filter(|p| (p == "request.js" || p == "index.js" || p.starts_with("lib/")) && p.ends_with(".js") && !p.starts_with("test/") && !p.starts_with("examples/")).collect();
    let total_files = files.len();

    let mut files_parsed = 0usize;
    let mut raw_candidates_total = 0usize;
    let mut total_loc = 0usize;
    let mut snapshots: Vec<codegraph_index::snapshot::SymbolSnapshot> = Vec::new();
    let mut kind_counts: HashMap<SymbolKind, usize> = HashMap::new();
    let mut signature_too_long_names: Vec<String> = Vec::new();
    let mut raw_import_statements_all: Vec<(codegraph_domain::FileIdentity, RawImportStatement)> = Vec::new();

    for relative_path in &files {
        let file = match codegraph_domain::FileIdentity::parse(repo_id, commit.clone(), relative_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let Some(parser) = parser_for(&file) else { continue };
        let bytes = match source.read_file(&file) {
            Ok(b) => b,
            Err(_) => continue,
        };
        total_loc += bytes.iter().filter(|&&b| b == b'\n').count();
        let Ok(tree) = parser.parse(&bytes) else { continue };
        files_parsed += 1;

        let raw_imports = extract_import_statements(&tree);
        raw_import_statements_all.extend(raw_imports.into_iter().map(|s| (file.clone(), s)));

        let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
        raw_candidates_total += candidates.len();
        for rejection in &rejections {
            if let codegraph_index::extract::CandidateRejection::SignatureTooLong { name, .. } = rejection {
                signature_too_long_names.push(name.clone());
            }
        }
        for candidate in candidates {
            if let Ok(snapshot) = promote(&candidate, file.clone()) {
                *kind_counts.entry(snapshot.kind()).or_insert(0) += 1;
                snapshots.push(snapshot);
            }
        }
    }

    println!("\n--- A. Corpus metrics (AFTER) ---");
    println!("total_files_in_commit (scoped): {total_files}");
    println!("files_parsed: {files_parsed}");
    println!("total_loc: {total_loc}");
    println!("raw_candidates_total: {raw_candidates_total}");
    println!("snapshots_promoted: {}", snapshots.len());
    let mut kind_vec: Vec<_> = kind_counts.into_iter().collect();
    kind_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (kind, count) in &kind_vec {
        println!("  {kind:?}: {count}");
    }

    let import_facts = resolve_raw_import_statements(&raw_import_statements_all, &snapshots).expect("resolve_raw_import_statements");
    println!("\nreal_import_facts_extracted (AFTER, require()+import_statement both handled): {}", import_facts.len());
    let mut resolution_counts: HashMap<&'static str, usize> = HashMap::new();
    for fact in &import_facts {
        let label = match fact.resolution {
            codegraph_index::imports::ImportResolution::Internal(_) => "Internal",
            codegraph_index::imports::ImportResolution::External => "External",
            codegraph_index::imports::ImportResolution::Ambiguous => "Ambiguous",
        };
        *resolution_counts.entry(label).or_insert(0) += 1;
    }
    for (label, count) in &resolution_counts {
        println!("  {label}: {count}");
    }

    // The REAL production entry point -- same function the real
    // CodeGraphIndexer calls on both full and incremental paths. Empty
    // csharp_contexts is correct for a JS-only corpus (byte-identical
    // to the plain entry point per that function's own equivalence
    // guarantee).
    let (edges, call_attempts) = resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge(&snapshots, &[], &signature_too_long_names, &import_facts).expect("resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge");

    let full_index_elapsed = t0.elapsed();

    println!("\n--- C. Structural edges (AFTER, real production resolver set) ---");
    let mut edge_kind_counts: HashMap<EdgeKind, usize> = HashMap::new();
    let mut resolution_method_counts: HashMap<ResolutionMethod, usize> = HashMap::new();
    for edge in &edges {
        *edge_kind_counts.entry(edge.kind).or_insert(0) += 1;
        *resolution_method_counts.entry(edge.evidence.method()).or_insert(0) += 1;
    }
    let mut ek_vec: Vec<_> = edge_kind_counts.into_iter().collect();
    ek_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (kind, count) in &ek_vec {
        println!("  {kind:?}: {count}");
    }
    println!("resolution_method breakdown (all edge kinds combined):");
    let mut rm_vec: Vec<_> = resolution_method_counts.into_iter().collect();
    rm_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (method, count) in &rm_vec {
        println!("  {method:?}: {count}");
    }

    println!("\n--- D. Calls resolution taxonomy (EvidenceTier), AFTER ---");
    let mut tier_counts: HashMap<EvidenceTier, usize> = HashMap::new();
    for edge in edges.iter().filter(|e| e.kind == EdgeKind::Calls) {
        let tier = EvidenceTier::from_resolution_method(edge.evidence.method());
        *tier_counts.entry(tier).or_insert(0) += 1;
    }
    let ambiguous_attempts: Vec<&CallAttempt> = call_attempts.iter().filter(|a| a.tier == EvidenceTier::Ambiguous).collect();
    let unresolved_attempts: Vec<&CallAttempt> = call_attempts.iter().filter(|a| a.tier == EvidenceTier::Unresolved).collect();
    println!("resolved Calls edges by tier:");
    let mut tv: Vec<_> = tier_counts.into_iter().collect();
    tv.sort_by_key(|(t, _)| format!("{t:?}"));
    for (tier, count) in &tv {
        println!("  {tier:?}: {count}");
    }
    println!("total_call_attempts (negative knowledge): {}", call_attempts.len());
    println!("  Ambiguous: {}", ambiguous_attempts.len());
    println!("  Unresolved: {}", unresolved_attempts.len());

    println!("\n--- D2. UnresolvedReason distribution (AFTER) ---");
    let mut reason_counts: HashMap<&'static str, usize> = HashMap::new();
    for attempt in &call_attempts {
        let label = match &attempt.reason {
            UnresolvedReason::Unknown => "Unknown",
            UnresolvedReason::ExtractionRejected { .. } => "ExtractionRejected",
            UnresolvedReason::AmbiguousCandidates => "AmbiguousCandidates",
            UnresolvedReason::ImportTargetExternal => "ImportTargetExternal",
            UnresolvedReason::ImportedSymbolAmbiguous => "ImportedSymbolAmbiguous",
            UnresolvedReason::NoMatchingImportAlias => "NoMatchingImportAlias",
        };
        *reason_counts.entry(label).or_insert(0) += 1;
    }
    let mut rc_vec: Vec<_> = reason_counts.into_iter().collect();
    rc_vec.sort_by_key(|(k, _)| k.to_string());
    for (reason, count) in &rc_vec {
        println!("  {reason}: {count}");
    }

    println!("\n--- F. Ground-truth ID of the real cross-file call sites (AFTER: should now resolve, per B2-ID-014 fix) ---");
    println!("Note: 'jar' now has TWO real distinct declarations (lib/cookies.js's exports.jar, B2-ID-014; AND request.js's own Request.prototype.jar, B2-ID-015) -- a genuine real-corpus name collision this checkpoint's own fixes newly exposed, not a measurement artifact. decl_count below is therefore intentionally corpus-wide (2), while a per-caller resolved_edges/call_attempts lookup is also shown to confirm the real per-call-site outcome.");
    println!("Note: index.js's own request.jar/request.cookie (caller_name below) are NOT found by this lookup -- new finding B2-ID-016: `Foo.X = function(){{}}` where Foo is a plain local identifier (not exports/module.exports/.prototype) is a real, distinct CommonJS idiom outside this checkpoint's own authorized 4-point scope. request.js:39's own module-level call site is separately invisible for an unrelated, pre-existing reason (no Module/File caller-eligible snapshot exists in this pipeline at all).");
    for (caller_desc, callee_name, caller_name, caller_file) in [
        ("request.js:39 `cookies.jar()` (module-level, outside any function)", "jar", None, "request.js"),
        ("request.js:301 `self.jar(...)` inside Request.prototype.init", "jar", Some("init"), "request.js"),
        ("index.js:76 `cookies.jar(store)` inside request.jar (B2-ID-016, out of scope)", "jar", Some("jar"), "index.js"),
        ("index.js:80 `cookies.parse(str)` inside request.cookie (B2-ID-016, out of scope)", "parse", Some("cookie"), "index.js"),
        ("request.js:1422 `hawk.header(...)` inside Request.prototype.hawk", "header", Some("hawk"), "request.js"),
    ] {
        let decl_count = snapshots.iter().filter(|s| s.name().as_str() == callee_name).count();
        let caller_snapshot = caller_name.and_then(|cn| snapshots.iter().find(|s| s.name().as_str() == cn && s.file().relative_path() == caller_file));
        let (edge_count, attempt_count) = match caller_snapshot {
            Some(caller) => {
                let e = edges.iter().filter(|e| e.kind == EdgeKind::Calls && e.from == caller.id() && snapshots.iter().find(|s| s.id() == e.to).map(|s| s.name().as_str()) == Some(callee_name)).count();
                let a = call_attempts.iter().filter(|a| a.caller == caller.id() && a.callee_name == callee_name).count();
                (e, a)
            }
            None => (0, 0),
        };
        println!("  real call site: {caller_desc} -> real_decl_count_for_{callee_name:?}(corpus-wide)={decl_count} caller_found={} resolved_edges_from_this_caller={edge_count} call_attempts_from_this_caller={attempt_count}", caller_snapshot.is_some());
    }

    println!("\n--- G. Full index timing (AFTER) ---");
    println!("full_index_elapsed: {full_index_elapsed:?}");
    println!("real_contains_edges: {}", edges.iter().filter(|e| e.kind == EdgeKind::Contains).count());
    println!("real_extends_edges: {}", edges.iter().filter(|e| e.kind == EdgeKind::Extends).count());
    println!("real_instantiates_edges: {}", edges.iter().filter(|e| e.kind == EdgeKind::Instantiates).count());

    println!("\n--- H2. Visibility of Request.prototype.X = function(){{}} (AFTER: should now be visible AND owned, per B2-ID-015 fix) ---");
    let prototype_method_names = [
        "init", "getNewAgent", "start", "onRequestError", "onRequestResponse", "readResponseBody", "abort", "pipeDest", "qs", "form", "multipart", "json", "getHeader", "enableUnixSocket", "auth", "aws", "httpSignature", "hawk", "oauth", "jar", "pipe", "write", "end", "pause", "resume", "destroy", "clearTimeout",
    ];
    let request_ctor = snapshots.iter().find(|s| s.name().as_str() == "Request" && s.kind() == SymbolKind::Function);
    let mut visible_count = 0usize;
    let mut owned_count = 0usize;
    let mut invisible_names: Vec<&str> = Vec::new();
    for name in prototype_method_names {
        // Scoped to request.js specifically -- a bare name match alone
        // would conflate this corpus's own real Request.prototype.X
        // declaration with an unrelated same-named Method elsewhere in
        // the corpus (confirmed: lib/querystring.js has its own real,
        // unrelated "init" Method -- the exact same cross-file
        // name-collision class CRC-001/002 already document elsewhere
        // in this project).
        let decl = snapshots.iter().find(|s| s.name().as_str() == name && s.kind() == SymbolKind::Method && s.file().relative_path() == "request.js");
        if let Some(decl) = decl {
            visible_count += 1;
            if let Some(ctor) = request_ctor {
                let owned = edges.iter().any(|e| e.kind == EdgeKind::Contains && e.from == ctor.id() && e.to == decl.id());
                if owned {
                    owned_count += 1;
                }
            }
        } else {
            invisible_names.push(name);
        }
    }
    println!("visible (real extracted Method snapshot in request.js specifically) out of {}: {visible_count}", prototype_method_names.len());
    println!("invisible names: {invisible_names:?}");
    println!("owned by Request (real Contains edge) out of {visible_count} visible: {owned_count}");
    println!("SignatureTooLong rejections this run: {signature_too_long_names:?}");

    println!("\n=== END ===");
}
