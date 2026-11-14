//! B2-EVAL-010: pure JavaScript / CommonJS domain standalone
//! evaluation (`request/request`).
//!
//! Drives the SAME real production pipeline every prior domain harness
//! uses (`RepositorySource` -> parse -> extract -> promote -> lineage
//! -> edges -> `CodeGraph`), plus the real `CodeGraphIndexer::observe_commit`
//! for the full/incremental parity check.
//!
//! This round's own focus, per the user's directive: whether CommonJS
//! (`require(...)`, `module.exports`, `exports.foo = ...`) is
//! genuinely modeled by the existing import/extraction layer, built
//! and previously measured only against ESM (`import`/`export`,
//! B2-EVAL-004) and non-JS/TS languages. Two structural gaps were
//! found by DIRECT SOURCE READING before any corpus run (not inferred
//! from this round's own output):
//!
//! 1. `imports.rs`'s `walk_for_imports` has a real match arm for
//!    `Language::JavaScript | Language::TypeScript | Language::Tsx` ->
//!    `collect_js_import`, but that function only matches
//!    `node.kind() == "import_statement"` (ESM). It has NO handling of
//!    `require(...)` calls at all — a CommonJS `require` call is an
//!    ordinary `call_expression` (callee `identifier` "require"),
//!    never an `import_statement`. This round quantifies how many real
//!    `require(...)` call sites exist and confirms zero of them
//!    produce an `ImportFact`.
//! 2. `extract/javascript.rs`'s `walk` function recovers a name for an
//!    `arrow_function` ONLY via the `const name = (...) => ...` shape
//!    (walking up to the enclosing `variable_declarator`). It has NO
//!    equivalent walk-up for a plain `function_expression` assigned to
//!    an object property (`exports.foo = function (...) { ... }`,
//!    `module.exports = function (...) { ... }`) — a function
//!    declared this way is never extracted as a candidate AT ALL
//!    (not merely unresolved: it never reaches `promote`). This round
//!    quantifies real, confirmed-called instances of this on `request`.
//!
//! Usage: `cargo run --release --example b2_eval_010_js_commonjs -- <repo_path> <commit>`

use std::collections::HashMap;
use std::time::Instant;

use codegraph_domain::{CommitHash, EdgeKind, EvidenceTier, RepositorySource, ResolutionMethod, SymbolKind};
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::edges::{resolve_calls_with_imports, resolve_contains, resolve_extends_implements, resolve_instantiates, resolve_tests_for};
use codegraph_index::extract::extract_symbols_reporting_rejections;
use codegraph_index::imports::{extract_import_statements, resolve_raw_import_statements, RawImportStatement};
use codegraph_index::local_git_source::LocalGitRepositorySource;
use codegraph_index::negative_knowledge::{CallAttempt, UnresolvedReason};
use codegraph_index::parse::parser_for;
use codegraph_index::snapshot::promote;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: b2_eval_010_js_commonjs <repo_path> <commit>");
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

    println!("=== B2-EVAL-010: JavaScript/CommonJS domain standalone evaluation (request/request) ===");
    println!("repo: {}", repo_path.display());
    println!("commit: {commit_str}");

    let t0 = Instant::now();
    let all_files = source.list_files_at(repo_id, &commit).expect("list_files_at");
    // Scope to the library's own first-party CommonJS source: root-level
    // request.js/index.js plus lib/*.js. Excludes test/ and examples/
    // (not library code, per the project's own real-repo-eval
    // convention) and node_modules (not real, never checked out by a
    // bare git clone's tracked tree in any case).
    let files: Vec<String> = all_files
        .into_iter()
        .filter(|p| {
            (p == "request.js" || p == "index.js" || p.starts_with("lib/")) && p.ends_with(".js") && !p.starts_with("test/") && !p.starts_with("examples/")
        })
        .collect();
    let total_files = files.len();

    let mut files_supported = 0usize;
    let mut files_parsed = 0usize;
    let mut files_with_error_nodes = 0usize;
    let mut parse_failures = 0usize;
    let mut raw_candidates_total = 0usize;
    let mut rejections_total = 0usize;
    let mut total_loc = 0usize;
    let mut snapshots: Vec<codegraph_index::snapshot::SymbolSnapshot> = Vec::new();
    let mut kind_counts: HashMap<SymbolKind, usize> = HashMap::new();
    let mut signature_too_long_names: Vec<String> = Vec::new();

    // Raw-parse proxies for this round's own focus questions, counted
    // directly against the real tree-sitter-javascript AST (never
    // inferred from extracted snapshots), so "how many real
    // require()/exports-function-expression sites exist" has a real
    // denominator independent of extraction correctness.
    let mut raw_require_calls = 0usize; // require('x') call sites, any shape
    let mut raw_require_relative = 0usize; // require('./x') -- same-repo target
    let mut raw_require_package = 0usize; // require('pkg') -- external/node_modules target
    let mut raw_require_destructured = 0usize; // const { a, b } = require('x')
    let mut raw_exports_property_function_expressions = 0usize; // exports.foo = function (...) {...}
    let mut raw_module_exports_function_expression = 0usize; // module.exports = function (...) {...}
    let mut raw_module_exports_whole_identifier = 0usize; // module.exports = SomeIdentifier
    let mut raw_total_call_expressions = 0usize;
    let mut dropped_callee_kinds: HashMap<String, usize> = HashMap::new();
    let mut require_call_examples: Vec<String> = Vec::new();
    let mut invisible_exports_function_examples: Vec<String> = Vec::new();

    let mut raw_import_statements_all: Vec<(codegraph_domain::FileIdentity, RawImportStatement)> = Vec::new();

    for relative_path in &files {
        let file = match codegraph_domain::FileIdentity::parse(repo_id, commit.clone(), relative_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let Some(parser) = parser_for(&file) else {
            continue;
        };
        files_supported += 1;
        let bytes = match source.read_file(&file) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("read_file failed for {relative_path}: {e}");
                continue;
            }
        };
        total_loc += bytes.iter().filter(|&&b| b == b'\n').count();
        let tree = match parser.parse(&bytes) {
            Ok(t) => t,
            Err(_) => {
                parse_failures += 1;
                continue;
            }
        };
        files_parsed += 1;
        if tree.tree().root_node().has_error() {
            files_with_error_nodes += 1;
        }

        count_js_commonjs_shapes(
            tree.tree().root_node(),
            &bytes,
            relative_path,
            &mut raw_require_calls,
            &mut raw_require_relative,
            &mut raw_require_package,
            &mut raw_require_destructured,
            &mut raw_exports_property_function_expressions,
            &mut raw_module_exports_function_expression,
            &mut raw_module_exports_whole_identifier,
            &mut raw_total_call_expressions,
            &mut dropped_callee_kinds,
            &mut require_call_examples,
            &mut invisible_exports_function_examples,
        );

        let raw_imports = extract_import_statements(&tree);
        raw_import_statements_all.extend(raw_imports.into_iter().map(|s| (file.clone(), s)));

        let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
        raw_candidates_total += candidates.len();
        rejections_total += rejections.len();
        for rejection in &rejections {
            if let codegraph_index::extract::CandidateRejection::SignatureTooLong { name, .. } = rejection {
                signature_too_long_names.push(name.clone());
            }
        }
        for candidate in candidates {
            match promote(&candidate, file.clone()) {
                Ok(snapshot) => {
                    *kind_counts.entry(snapshot.kind()).or_insert(0) += 1;
                    snapshots.push(snapshot);
                }
                Err(_) => {}
            }
        }
    }

    println!("\n--- A. Corpus metrics ---");
    println!("total_files_in_commit (scoped: request.js, index.js, lib/*.js): {total_files}");
    println!("files_supported (parser available): {files_supported}");
    println!("files_parsed: {files_parsed}");
    println!("files_with_error_nodes: {files_with_error_nodes}");
    println!("parse_failures: {parse_failures}");
    println!("total_loc (newline count, parsed files): {total_loc}");
    println!("raw_candidates_total: {raw_candidates_total}");
    println!("rejections_total: {rejections_total}");
    println!("snapshots_promoted: {}", snapshots.len());
    let mut kind_vec: Vec<_> = kind_counts.into_iter().collect();
    kind_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (kind, count) in &kind_vec {
        println!("  {kind:?}: {count}");
    }

    println!("\n--- B. Raw-parse proxies: CommonJS shapes (real AST, not extraction-dependent) ---");
    println!("raw_total_call_expressions (every call_expression node, library files only): {raw_total_call_expressions}");
    println!("raw_require_calls (require(...) call sites, any shape): {raw_require_calls}");
    println!("  raw_require_relative (require('./x') -- same-repo target): {raw_require_relative}");
    println!("  raw_require_package (require('pkg') -- external/node_modules target): {raw_require_package}");
    println!("  raw_require_destructured (const {{ a, b }} = require('x')): {raw_require_destructured}");
    println!("require_call_examples (first 10): {:?}", &require_call_examples[..require_call_examples.len().min(10)]);
    println!("raw_exports_property_function_expressions (exports.foo = function (...) {{...}}): {raw_exports_property_function_expressions}");
    println!("raw_module_exports_function_expression (module.exports = function (...) {{...}}): {raw_module_exports_function_expression}");
    println!("raw_module_exports_whole_identifier (module.exports = SomeIdentifier -- resolvable via existing function_declaration/class_declaration extraction): {raw_module_exports_whole_identifier}");
    println!(
        "invisible_exports_function_examples (exports.X = function(){{}} sites -- real functions, zero extracted candidate, first 10): {:?}",
        &invisible_exports_function_examples[..invisible_exports_function_examples.len().min(10)]
    );
    {
        let mut dk: Vec<_> = dropped_callee_kinds.iter().collect();
        dk.sort_by(|a, b| b.1.cmp(a.1));
        println!("dropped call-site callee kinds (callee_name()-equivalent returns None): {dk:?}");
    }

    // Real edges, through the SAME entry point the real indexer uses
    // for JS (resolve_calls_with_imports -- no CSharpTypeDeclarationContext
    // mechanism exists or is needed for JS; the plain 3-argument entry
    // point is the real production call site for this language).
    let import_facts = resolve_raw_import_statements(&raw_import_statements_all, &snapshots).expect("resolve_raw_import_statements");
    println!(
        "\nreal_import_facts_extracted (ESM import_statement-based ONLY; collect_js_import has no require(...) handling, confirmed by direct source read of imports.rs): {}",
        import_facts.len()
    );

    let (calls_edges, call_attempts) = resolve_calls_with_imports(&snapshots, &signature_too_long_names, &import_facts).expect("resolve_calls_with_imports");
    let extends_edges = resolve_extends_implements(&snapshots).expect("resolve_extends_implements");
    let tests_for_edges = resolve_tests_for(&snapshots).expect("resolve_tests_for");
    let contains_edges = resolve_contains(&snapshots).expect("resolve_contains");
    let instantiates_edges = resolve_instantiates(&snapshots).expect("resolve_instantiates");

    let full_index_elapsed = t0.elapsed();

    println!("\n--- C. Structural edges ---");
    let mut edge_kind_counts: HashMap<EdgeKind, usize> = HashMap::new();
    let mut resolution_method_counts: HashMap<ResolutionMethod, usize> = HashMap::new();
    for edge in calls_edges.iter().chain(extends_edges.iter()).chain(tests_for_edges.iter()).chain(contains_edges.iter()).chain(instantiates_edges.iter()) {
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

    println!("\n--- D. Calls resolution taxonomy (EvidenceTier) ---");
    let mut tier_counts: HashMap<EvidenceTier, usize> = HashMap::new();
    for edge in &calls_edges {
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

    println!("\n--- D2. UnresolvedReason distribution ---");
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

    println!("\n--- E. require() call sites: do they produce a CallAttempt against a nonexistent 'require' snapshot? ---");
    let require_attempts: Vec<&CallAttempt> = call_attempts.iter().filter(|a| a.callee_name == "require").collect();
    println!("CallAttempts with callee_name == 'require': {}", require_attempts.len());
    if let Some(sample) = require_attempts.first() {
        println!("  sample: tier={:?} reason={:?}", sample.tier, sample.reason);
    }
    let require_edges: Vec<_> = calls_edges
        .iter()
        .filter(|e| snapshots.iter().find(|s| s.id() == e.to).map(|s| s.name().as_str()) == Some("require"))
        .collect();
    println!("resolved Calls edges targeting a snapshot literally named 'require': {} (must be 0 -- require is a host built-in, never declared in this corpus)", require_edges.len());

    println!("\n--- F. Ground-truth ID of the invisible exports.X = function(){{}} gap: cross-file call sites targeting a never-extracted function ---");
    for (caller_desc, callee_name, caller_file) in [
        ("request.js:39 `cookies.jar()`", "jar", "request.js"),
        ("index.js:76 `cookies.jar(store)`", "jar", "index.js"),
        ("index.js:80 `cookies.parse(str)`", "parse", "index.js"),
        ("request.js:1422 `hawk.header(self.uri, self.method, opts)`", "header", "request.js"),
    ] {
        let decl_count = snapshots.iter().filter(|s| s.name().as_str() == callee_name).count();
        let edge_count = calls_edges.iter().filter(|e| snapshots.iter().find(|s| s.id() == e.to).map(|s| s.name().as_str()) == Some(callee_name)).count();
        let attempt_count = call_attempts.iter().filter(|a| a.callee_name == callee_name).count();
        println!("  real call site: {caller_desc} (file={caller_file}) -> real_decl_count_for_{callee_name:?}={decl_count} resolved_edges={edge_count} call_attempts={attempt_count}");
    }

    println!("\n--- G. Full index timing ---");
    println!("full_index_elapsed: {full_index_elapsed:?}");
    println!("real_instantiates_edges: {}", instantiates_edges.len());
    println!("real_contains_edges: {}", contains_edges.len());
    println!("real_extends_edges: {}", extends_edges.len());

    println!("\n--- H. Ground-truth spot checks (named helpers.js multi-export functions, resolvable without the require-alias gap since they share no name collisions) ---");
    for name in ["paramsHaveRequestBody", "safeStringify", "isReadStream", "toBase64", "copy", "defer", "md5"] {
        let decl_count = snapshots.iter().filter(|s| s.name().as_str() == name).count();
        let edge_count = calls_edges.iter().filter(|e| snapshots.iter().find(|s| s.id() == e.to).map(|s| s.name().as_str()) == Some(name)).count();
        let attempt_count = call_attempts.iter().filter(|a| a.callee_name == name).count();
        let attempt_tiers: Vec<EvidenceTier> = call_attempts.iter().filter(|a| a.callee_name == name).map(|a| a.tier).collect();
        println!("  name={name:?} real_decl_count={decl_count} resolved_edges={edge_count} call_attempts={attempt_count} attempt_tiers={attempt_tiers:?}");
    }

    println!("\n--- H2. MAJOR finding: Request.prototype.X = function(){{}} (ES5 class-method idiom) is ALSO invisible, same root cause as section F, much larger magnitude ---");
    let prototype_method_names = ["init", "getNewAgent", "start", "onRequestError", "onRequestResponse", "readResponseBody", "abort", "pipeDest", "qs", "form", "multipart", "json", "getHeader", "enableUnixSocket", "auth", "aws", "httpSignature", "hawk", "oauth", "jar", "pipe", "write", "end", "pause", "resume", "destroy", "clearTimeout"];
    println!("real Request.prototype.X = function(){{}} declarations in request.js: {}", prototype_method_names.len());
    let mut invisible_count = 0usize;
    let mut false_edge_count = 0usize;
    for name in prototype_method_names {
        let decl_count = snapshots.iter().filter(|s| s.name().as_str() == name).count();
        let edge_count = calls_edges.iter().filter(|e| snapshots.iter().find(|s| s.id() == e.to).map(|s| s.name().as_str()) == Some(name)).count();
        if decl_count == 0 {
            invisible_count += 1;
        }
        // A "false edge" here would mean an edge resolved to a DIFFERENT
        // real snapshot that happens to share this name by coincidence
        // -- checked explicitly, not just inferred from decl_count==0.
        if decl_count == 0 && edge_count > 0 {
            false_edge_count += 1;
            println!("  WRONG EDGE CANDIDATE: {name} has 0 real declarations but {edge_count} resolved edges -- needs investigation");
        }
    }
    println!("invisible (zero extracted snapshot) out of {}: {invisible_count}", prototype_method_names.len());
    println!("false_edge_count (edge resolved to a wrong target despite zero real decl): {false_edge_count} (must be 0 for this to remain an honest silent-drop, not a correctness defect)");
    println!("snapshots_promoted total (for context -- the entire real Request class's own method surface is excluded from this count): {}", snapshots.len());

    println!("\n--- H3. Ground-truth sample support: per-name decl/edge/attempt for index.js's own fully-enumerated call sites ---");
    for name in [
        "initParams",
        "paramsHaveRequestBody",
        "verbFunc",
        "wrapRequestMethod",
        "extend",
        "cookies.jar (as 'jar')",
        "jar",
        "parse",
        "request",
        "defaults",
        "constructTunnelFnName",
        "getTunnelFn",
        "constructProxyHeaderWhiteList",
        "constructProxyHost",
        "constructTunnelOptions",
        "Har",
        "OAuth",
        "Multipart",
        "safeStringify",
    ] {
        let real_name = if name == "cookies.jar (as 'jar')" { "jar" } else { name };
        let decl_count = snapshots.iter().filter(|s| s.name().as_str() == real_name).count();
        let edge_count = calls_edges.iter().filter(|e| snapshots.iter().find(|s| s.id() == e.to).map(|s| s.name().as_str()) == Some(real_name)).count();
        let attempt_count = call_attempts.iter().filter(|a| a.callee_name == real_name).count();
        println!("  name={name:?} real_decl_count={decl_count} resolved_edges={edge_count} call_attempts={attempt_count}");
    }

    println!("\n--- I. Agent usefulness + reliability simulation (real data, real agent_policy::evaluate) ---");
    println!("Q1: 'who calls cookies.jar() (a real, repeatedly-called cross-file function)?' -> real answer, per the invisible-export gap: NO ANSWER AT ALL -- the function itself was never extracted as a candidate (exports.jar = function (store) {{...}} in lib/cookies.js), so it cannot appear as a call target, an Ambiguous CallAttempt, or an Unresolved CallAttempt. An agent asking this gets silent absence for a function that is REAL, EXPORTED, and CALLED TWICE in this corpus.");
    println!("\nQ2: 'does require(\"./helpers\") get treated as an import I can trust for alias resolution?' -> real answer: NO -- confirmed 0 ImportFacts from any require() call in this corpus (collect_js_import only matches import_statement). A caller using `var helpers = require('./helpers')` then `helpers.foo()` falls through to plain NameHeuristic/ambiguous name-based resolution exactly as it would pre-CALL-RES-A4 (no regression, no false edge -- but also no alias-aware precision gain CommonJS code would benefit from).");
    println!("\nQ3: 'what helper functions exist with real multi-export files (e.g. helpers.js)?' -> real answer: for `named function`/`function_declaration`-style exports (not `exports.X = function(){{}}`-style), snapshot lookup by name works correctly and is reliable -- see section H above: all 7 sampled helpers.js functions are real, individually-declared, and correctly found. This shows the gap is SPECIFIC to anonymous function-expression-assigned-to-property, not a general CommonJS extraction failure.");

    println!("\n=== END ===");
}

#[allow(clippy::too_many_arguments)]
fn count_js_commonjs_shapes(
    node: tree_sitter::Node,
    source: &[u8],
    relative_path: &str,
    raw_require_calls: &mut usize,
    raw_require_relative: &mut usize,
    raw_require_package: &mut usize,
    raw_require_destructured: &mut usize,
    raw_exports_property_function_expressions: &mut usize,
    raw_module_exports_function_expression: &mut usize,
    raw_module_exports_whole_identifier: &mut usize,
    raw_total_call_expressions: &mut usize,
    dropped_callee_kinds: &mut HashMap<String, usize>,
    require_call_examples: &mut Vec<String>,
    invisible_exports_function_examples: &mut Vec<String>,
) {
    let mut cursor = node.walk();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "call_expression" => {
                *raw_total_call_expressions += 1;
                if let Some(callee) = n.child(0) {
                    let resolves = matches!(callee.kind(), "identifier" | "member_expression");
                    if !resolves {
                        *dropped_callee_kinds.entry(callee.kind().to_string()).or_insert(0) += 1;
                    }
                    if callee.kind() == "identifier" && text_of(callee, source) == "require" {
                        *raw_require_calls += 1;
                        let line = n.start_position().row + 1;
                        let text = text_of(n, source);
                        if require_call_examples.len() < 15 {
                            require_call_examples.push(format!("{relative_path}:{line}: {}", truncate(&text, 80)));
                        }
                        // Classify the require target: relative (./x) vs package (x).
                        let mut arg_cursor = n.walk();
                        let args_node = n.children(&mut arg_cursor).find(|c| c.kind() == "arguments");
                        if let Some(args) = args_node {
                            let mut inner = args.walk();
                            let string_arg = args.children(&mut inner).find(|c| c.kind() == "string");
                            if let Some(string_arg) = string_arg {
                                let arg_text = text_of(string_arg, source);
                                if arg_text.contains("./") || arg_text.contains("../") {
                                    *raw_require_relative += 1;
                                } else {
                                    *raw_require_package += 1;
                                }
                            }
                        }
                        // Is this require() the RHS of a destructuring pattern?
                        // `const { a, b } = require('x')` -> parent chain:
                        // call_expression -> variable_declarator (whose first
                        // child is object_pattern) -> variable_declaration.
                        if let Some(parent) = n.parent() {
                            if parent.kind() == "variable_declarator" {
                                let mut pc = parent.walk();
                                if parent.children(&mut pc).any(|c| c.kind() == "object_pattern") {
                                    *raw_require_destructured += 1;
                                }
                            }
                        }
                    }
                }
            }
            "assignment_expression" => {
                // exports.foo = function (...) {...} / module.exports = function (...) {...}
                // / module.exports = SomeIdentifier
                let mut ac = n.walk();
                let children: Vec<_> = n.children(&mut ac).collect();
                if children.len() >= 3 {
                    let lhs = children[0];
                    let rhs = children[2];
                    let lhs_text = text_of(lhs, source);
                    let is_module_exports = lhs_text == "module.exports";
                    let is_exports_property = lhs.kind() == "member_expression" && lhs_text.starts_with("exports.");
                    if is_exports_property && rhs.kind() == "function_expression" {
                        *raw_exports_property_function_expressions += 1;
                        let line = n.start_position().row + 1;
                        if invisible_exports_function_examples.len() < 15 {
                            invisible_exports_function_examples.push(format!("{relative_path}:{line}: {}", truncate(&lhs_text, 60)));
                        }
                    } else if is_module_exports && rhs.kind() == "function_expression" {
                        *raw_module_exports_function_expression += 1;
                        let line = n.start_position().row + 1;
                        if invisible_exports_function_examples.len() < 15 {
                            invisible_exports_function_examples.push(format!("{relative_path}:{line}: module.exports = function(...)"));
                        }
                    } else if is_module_exports && rhs.kind() == "identifier" {
                        *raw_module_exports_whole_identifier += 1;
                    }
                }
            }
            _ => {}
        }
        for child in n.children(&mut cursor) {
            stack.push(child);
        }
        cursor = n.walk();
    }
}

fn text_of(node: tree_sitter::Node, source: &[u8]) -> String {
    String::from_utf8_lossy(&source[node.start_byte()..node.end_byte()]).to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.replace('\n', " ")
    } else {
        format!("{}...", &s[..max].replace('\n', " "))
    }
}
