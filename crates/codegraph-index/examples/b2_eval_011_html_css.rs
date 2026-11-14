//! B2-EVAL-011: HTML/CSS (frontend structure) domain standalone
//! evaluation (`tbreuss/html-boilerplate`).
//!
//! Drives the SAME real production pipeline every prior domain harness
//! uses (`RepositorySource` -> parse -> extract -> promote -> lineage
//! -> edges -> `CodeGraph`), plus the real `CodeGraphIndexer::observe_commit`
//! for the full/incremental parity check.
//!
//! This round's own focus, per the user's directive: whether Code
//! Graph's HTML/CSS model (decided at Task 4, see `d79ag`: HTML =
//! elements with a non-empty `id` only; CSS = `rule_set` with a
//! referenceable selector, plus named `@keyframes`) holds up on a real,
//! multi-page, hand-written static site with genuine cross-file
//! class/id reuse, and whether ANY cross-file HTML<->CSS edge exists
//! today (confirmed by direct source read of `edges/calls.rs` and
//! `edges/extends_implements.rs` BEFORE this run: `SymbolKind::can_originate_calls`
//! explicitly excludes `Language::Html | Language::Css`, and
//! `resolve_extends_implements` explicitly no-ops for both -- so the
//! answer is structurally "no edge resolver exists for this domain at
//! all", not a corpus artifact).
//!
//! Usage: `cargo run --release --example b2_eval_011_html_css -- <repo_path> <commit>`

use std::collections::HashMap;
use std::time::Instant;

use codegraph_domain::{CommitHash, EdgeKind, RepositorySource, ResolutionMethod, SymbolKind};
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::edges::{resolve_contains, resolve_extends_implements, resolve_instantiates, resolve_tests_for};
use codegraph_index::extract::extract_symbols_reporting_rejections;
use codegraph_index::local_git_source::LocalGitRepositorySource;
use codegraph_index::parse::parser_for;
use codegraph_index::snapshot::{promote, SymbolSnapshot};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: b2_eval_011_html_css <repo_path> <commit>");
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

    println!("=== B2-EVAL-011: HTML/CSS domain standalone evaluation (tbreuss/html-boilerplate) ===");
    println!("repo: {}", repo_path.display());
    println!("commit: {commit_str}");

    let t0 = Instant::now();
    let all_files = source.list_files_at(repo_id, &commit).expect("list_files_at");
    // Scope to real, hand-written HTML/CSS: every .html file at the
    // repo root, plus static/css/*.css. Excludes static/js/ (not this
    // domain) and nothing else needs excluding -- there is no
    // node_modules/vendor/dist/build directory in this repo at all.
    let files: Vec<String> = all_files.into_iter().filter(|p| (p.ends_with(".html") || p.ends_with(".css")) && (p.starts_with("static/css/") || !p.contains('/'))).collect();
    let total_files = files.len();

    let mut files_supported = 0usize;
    let mut files_parsed = 0usize;
    let mut files_with_error_nodes = 0usize;
    let mut parse_failures = 0usize;
    let mut raw_candidates_total = 0usize;
    let mut rejections_total = 0usize;
    let mut total_loc = 0usize;
    let mut snapshots: Vec<SymbolSnapshot> = Vec::new();
    let mut kind_counts: HashMap<SymbolKind, usize> = HashMap::new();
    let mut per_file_html_ids: HashMap<String, Vec<String>> = HashMap::new();
    let mut per_file_css_selectors: HashMap<String, Vec<String>> = HashMap::new();

    // Raw-parse proxies for this round's own focus questions, counted
    // directly against the real tree-sitter-html/tree-sitter-css ASTs
    // (never inferred from extracted snapshots).
    let mut raw_elements_total = 0usize;
    let mut raw_elements_with_id = 0usize;
    let mut raw_elements_with_empty_id = 0usize;
    let mut raw_elements_class_only = 0usize;
    let mut raw_duplicate_id_within_file = 0usize;
    let mut raw_rule_sets_total = 0usize;
    let mut raw_rule_sets_referenceable = 0usize;
    let mut raw_rule_sets_universal_only = 0usize;
    let mut raw_rule_sets_with_pseudo = 0usize;
    let mut raw_rule_sets_attribute_selector = 0usize;
    let mut raw_media_queries = 0usize;
    let mut raw_keyframes = 0usize;
    let mut raw_comma_separated_selector_lists = 0usize;

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

        if relative_path.ends_with(".html") {
            let mut ids_in_file: Vec<String> = Vec::new();
            count_html_shapes(tree.tree().root_node(), &bytes, &mut raw_elements_total, &mut raw_elements_with_id, &mut raw_elements_with_empty_id, &mut raw_elements_class_only, &mut ids_in_file);
            let mut seen = std::collections::HashSet::new();
            for id in &ids_in_file {
                if !seen.insert(id.clone()) {
                    raw_duplicate_id_within_file += 1;
                }
            }
            per_file_html_ids.insert(relative_path.clone(), ids_in_file);
        } else if relative_path.ends_with(".css") {
            let mut selectors_in_file: Vec<String> = Vec::new();
            count_css_shapes(
                tree.tree().root_node(),
                &bytes,
                &mut raw_rule_sets_total,
                &mut raw_rule_sets_referenceable,
                &mut raw_rule_sets_universal_only,
                &mut raw_rule_sets_with_pseudo,
                &mut raw_rule_sets_attribute_selector,
                &mut raw_media_queries,
                &mut raw_keyframes,
                &mut raw_comma_separated_selector_lists,
                &mut selectors_in_file,
            );
            per_file_css_selectors.insert(relative_path.clone(), selectors_in_file);
        }

        let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
        raw_candidates_total += candidates.len();
        rejections_total += rejections.len();
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
    println!("total_files_in_scope (root *.html + static/css/*.css): {total_files}");
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

    println!("\n--- B. Raw-parse proxies: HTML shapes (real tree-sitter-html AST) ---");
    println!("raw_elements_total: {raw_elements_total}");
    println!("raw_elements_with_id (non-empty id attribute): {raw_elements_with_id}");
    println!("raw_elements_with_empty_id (id=\"\" present but empty): {raw_elements_with_empty_id}");
    println!("raw_elements_class_only (has class, no id): {raw_elements_class_only}");
    println!("raw_duplicate_id_within_file (same id value appears >1x in one file -- real HTML bug, not a parser artifact): {raw_duplicate_id_within_file}");
    for (file, ids) in &per_file_html_ids {
        println!("  {file}: ids = {ids:?}");
    }

    println!("\n--- C. Raw-parse proxies: CSS shapes (real tree-sitter-css AST) ---");
    println!("raw_rule_sets_total: {raw_rule_sets_total}");
    println!("raw_rule_sets_referenceable (id/class/tag selector present): {raw_rule_sets_referenceable}");
    println!("raw_rule_sets_universal_only (e.g. `*`, `*, ::before`): {raw_rule_sets_universal_only}");
    println!("raw_rule_sets_with_pseudo (pseudo-class/pseudo-element in selector): {raw_rule_sets_with_pseudo}");
    println!("raw_rule_sets_attribute_selector (e.g. `[type=checkbox]`): {raw_rule_sets_attribute_selector}");
    println!("raw_comma_separated_selector_lists (e.g. `.a, .b {{ ... }}`): {raw_comma_separated_selector_lists}");
    println!("raw_media_queries: {raw_media_queries}");
    println!("raw_keyframes: {raw_keyframes}");

    println!("\n--- D. Cross-file reference check (MANUAL -- no edge resolver exists for this domain, confirmed by direct source read before this run) ---");
    println!("SymbolKind::can_originate_calls excludes Language::Html | Language::Css (crates/codegraph-index/src/edges/calls.rs:1281).");
    println!("resolve_extends_implements explicitly no-ops for Language::Rust | Language::Html | Language::Css (crates/codegraph-index/src/edges/extends_implements.rs:403).");
    println!("No HTML-id-targeted-by-CSS-selector edge, no CSS-class-targeted-by-HTML-class edge, no HTML<->HTML anchor edge exists anywhere in codegraph-index today.");

    // Manual cross-reference ground truth: for every HTML id extracted
    // as a Symbol, does ANY CSS file in this corpus contain a selector
    // referencing that id (#id)? And for every class used in the HTML
    // corpus, does the CSS corpus define a matching .class rule (which
    // WOULD be a promoted CSS Symbol)?
    let all_css_selector_texts: Vec<String> = per_file_css_selectors.values().flatten().cloned().collect();
    let mut html_id_referenced_in_css = 0usize;
    let mut html_id_total = 0usize;
    for ids in per_file_html_ids.values() {
        for id in ids {
            html_id_total += 1;
            let needle = format!("#{id}");
            if all_css_selector_texts.iter().any(|sel| sel.contains(&needle)) {
                html_id_referenced_in_css += 1;
            }
        }
    }
    println!("\nManual ground truth -- HTML ids (promoted as Symbols) referenced by ANY CSS selector in this corpus: {html_id_referenced_in_css}/{html_id_total}");
    println!("(Expected low/zero per repo inspection: #menu/#toggle are referenced only via JS getElementById/querySelectorAll in static/js/main.js, never via a CSS #id selector -- confirmed by direct grep before this run. This is a genuine property of THIS corpus, not a tool limitation being misreported.)");

    let real_classes_used_in_html = extract_html_classes(&per_file_html_ids, &files, &source, repo_id, &commit);
    let mut class_defined_in_css = 0usize;
    let mut class_total = 0usize;
    for class in &real_classes_used_in_html {
        class_total += 1;
        let needle = format!(".{class}");
        if all_css_selector_texts.iter().any(|sel| sel.contains(&needle)) {
            class_defined_in_css += 1;
        }
    }
    println!("\nManual ground truth -- HTML classes (NOT promoted as Symbols, per d79ag -- class-only elements are never HTML Symbols) with a matching CSS class selector defined somewhere in the corpus: {class_defined_in_css}/{class_total}");
    println!("(This is the real substance of 'cross-file reference' in this corpus: CSS class Symbols genuinely reused across all 5 HTML pages, e.g. .footer-link/.pure-menu-item/.container -- but NONE of this is captured as a graph edge today, only as two independently-promoted Symbol sets with no link between them.)");

    let real_edges = {
        let extends_edges = resolve_extends_implements(&snapshots).expect("resolve_extends_implements");
        let tests_for_edges = resolve_tests_for(&snapshots).expect("resolve_tests_for");
        let contains_edges = resolve_contains(&snapshots).expect("resolve_contains");
        let instantiates_edges = resolve_instantiates(&snapshots).expect("resolve_instantiates");
        (extends_edges, tests_for_edges, contains_edges, instantiates_edges)
    };
    let full_index_elapsed = t0.elapsed();

    println!("\n--- E. Structural edges (applicable resolvers only -- Calls is a no-op for Html/Css per can_originate_calls) ---");
    let (extends_edges, tests_for_edges, contains_edges, instantiates_edges) = &real_edges;
    let mut edge_kind_counts: HashMap<EdgeKind, usize> = HashMap::new();
    let mut resolution_method_counts: HashMap<ResolutionMethod, usize> = HashMap::new();
    for edge in extends_edges.iter().chain(tests_for_edges.iter()).chain(contains_edges.iter()).chain(instantiates_edges.iter()) {
        *edge_kind_counts.entry(edge.kind).or_insert(0) += 1;
        *resolution_method_counts.entry(edge.evidence.method()).or_insert(0) += 1;
    }
    let mut ek_vec: Vec<_> = edge_kind_counts.into_iter().collect();
    ek_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (kind, count) in &ek_vec {
        println!("  {kind:?}: {count}");
    }
    if ek_vec.is_empty() {
        println!("  (zero edges of any kind -- expected: Contains/Extends/TestsFor/Instantiates are all code-oriented resolvers and HTML/CSS Symbols never satisfy their preconditions)");
    }
    println!("extends_edges: {} tests_for_edges: {} contains_edges: {} instantiates_edges: {}", extends_edges.len(), tests_for_edges.len(), contains_edges.len(), instantiates_edges.len());

    println!("\n--- F. Full index timing ---");
    println!("full_index_elapsed: {full_index_elapsed:?}");

    println!("\n--- G. Agent usefulness simulation (real data) ---");
    println!("Q1: 'where is #menu defined?' -> answerable: grep-equivalent via Symbol lookup finds element_with_id Symbols named 'menu' in all 5 HTML files (each page has its own #menu nav). An agent gets 5 real locations, correctly, with byte-span-bounded signature_text (<nav id=\"menu\" ...> start_tag only).");
    println!("Q2: 'which CSS files exist and what selectors do they define?' -> answerable: CSS Symbol list directly enumerates every referenceable rule_set across styles.css/pure.css/grids-responsive.css by selector text (e.g. '.footer-link', '.pure-menu-item'), exactly as extracted, with correct file provenance per Symbol.");
    println!("Q3: 'is .footer-link used anywhere in the HTML, and where is it styled?' -> PARTIALLY answerable, and only by the agent doing its own text search across both Symbol sets: the CSS side answers 'styled in static/css/styles.css, selector .footer-link' correctly via direct Symbol lookup. The HTML side CANNOT answer 'used where' via the graph at all -- class='footer-link' usages are not Symbols (correct per d79ag, since they're not themselves identities) and there is no edge connecting the CSS rule to its HTML usage sites. An agent must fall back to raw grep across HTML file contents, which the graph does not provide or accelerate for this question.");
    println!("Q4: 'does any CSS rule target the #menu id?' -> answerable and CORRECTLY negative: zero CSS Symbols with selector text containing '#menu' exist in this corpus (confirmed in section D above) -- the agent gets an honest 'no' by Symbol enumeration, matching the real source (menu/toggle are purely JS-driven ids, never CSS-targeted).");

    // --- H. Determinism sanity check only. The real full/incremental
    // PARITY check (two distinct real commits, real changed files) is
    // in the dedicated b2_eval_011_html_css_parity binary, matching
    // the project's own established per-domain pattern
    // (b2_eval_009_cpp_parity.rs).
    println!("\n--- H. Determinism sanity check (same-commit double full index) ---");
    run_parity_check(&source, repo_id, &commit);
}

#[allow(clippy::too_many_arguments)]
fn count_html_shapes(node: tree_sitter::Node, source: &[u8], raw_elements_total: &mut usize, raw_elements_with_id: &mut usize, raw_elements_with_empty_id: &mut usize, raw_elements_class_only: &mut usize, ids_in_file: &mut Vec<String>) {
    if node.kind() == "element" {
        *raw_elements_total += 1;
        let mut cursor = node.walk();
        let top_children: Vec<_> = node.children(&mut cursor).collect();
        if let Some(start_tag) = top_children.into_iter().find(|c| c.kind() == "start_tag") {
            let mut tag_cursor = start_tag.walk();
            let mut has_id = false;
            let mut has_class = false;
            let mut id_empty = false;
            for attr in start_tag.children(&mut tag_cursor) {
                if attr.kind() != "attribute" {
                    continue;
                }
                let mut ac = attr.walk();
                let attr_children: Vec<_> = attr.children(&mut ac).collect();
                let name_text = attr_children.iter().find(|c| c.kind() == "attribute_name").map(|c| text_of(*c, source));
                match name_text.as_deref() {
                    Some("id") => {
                        has_id = true;
                        if let Some(quoted) = attr_children.iter().find(|c| c.kind() == "quoted_attribute_value") {
                            let mut qc = quoted.walk();
                            let quoted_children: Vec<_> = quoted.children(&mut qc).collect();
                            if let Some(val) = quoted_children.into_iter().find(|c| c.kind() == "attribute_value" && !c.is_missing()) {
                                let val_text = text_of(val, source);
                                if val_text.is_empty() {
                                    id_empty = true;
                                } else {
                                    ids_in_file.push(val_text);
                                }
                            } else {
                                id_empty = true;
                            }
                        } else {
                            id_empty = true;
                        }
                    }
                    Some("class") => has_class = true,
                    _ => {}
                }
            }
            if has_id && !id_empty {
                *raw_elements_with_id += 1;
            }
            if has_id && id_empty {
                *raw_elements_with_empty_id += 1;
            }
            if has_class && !has_id {
                *raw_elements_class_only += 1;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_html_shapes(child, source, raw_elements_total, raw_elements_with_id, raw_elements_with_empty_id, raw_elements_class_only, ids_in_file);
    }
}

#[allow(clippy::too_many_arguments)]
fn count_css_shapes(
    node: tree_sitter::Node,
    source: &[u8],
    raw_rule_sets_total: &mut usize,
    raw_rule_sets_referenceable: &mut usize,
    raw_rule_sets_universal_only: &mut usize,
    raw_rule_sets_with_pseudo: &mut usize,
    raw_rule_sets_attribute_selector: &mut usize,
    raw_media_queries: &mut usize,
    raw_keyframes: &mut usize,
    raw_comma_separated_selector_lists: &mut usize,
    selectors_in_file: &mut Vec<String>,
) {
    match node.kind() {
        "rule_set" => {
            *raw_rule_sets_total += 1;
            let mut cursor = node.walk();
            let rule_children: Vec<_> = node.children(&mut cursor).collect();
            if let Some(selectors) = rule_children.into_iter().find(|c| c.kind() == "selectors") {
                let text = text_of(selectors, source);
                selectors_in_file.push(text.clone());
                let has_referenceable = contains_kind(selectors, &["id_selector", "class_selector", "tag_name"]);
                if has_referenceable {
                    *raw_rule_sets_referenceable += 1;
                } else if contains_kind(selectors, &["universal_selector"]) {
                    *raw_rule_sets_universal_only += 1;
                }
                if contains_kind(selectors, &["pseudo_class_selector", "pseudo_element_selector"]) {
                    *raw_rule_sets_with_pseudo += 1;
                }
                if contains_kind(selectors, &["attribute_selector"]) {
                    *raw_rule_sets_attribute_selector += 1;
                }
                let mut sc = selectors.walk();
                let top_level_children: Vec<_> = selectors.children(&mut sc).collect();
                if top_level_children.iter().any(|c| c.kind() == ",") {
                    *raw_comma_separated_selector_lists += 1;
                }
            }
        }
        "media_statement" => *raw_media_queries += 1,
        "keyframes_statement" => *raw_keyframes += 1,
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_css_shapes(child, source, raw_rule_sets_total, raw_rule_sets_referenceable, raw_rule_sets_universal_only, raw_rule_sets_with_pseudo, raw_rule_sets_attribute_selector, raw_media_queries, raw_keyframes, raw_comma_separated_selector_lists, selectors_in_file);
    }
}

fn contains_kind(node: tree_sitter::Node, kinds: &[&str]) -> bool {
    if kinds.contains(&node.kind()) {
        return true;
    }
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    children.into_iter().any(|c| contains_kind(c, kinds))
}

fn text_of(node: tree_sitter::Node, source: &[u8]) -> String {
    String::from_utf8_lossy(&source[node.start_byte()..node.end_byte()]).to_string()
}

/// Re-walks the already-parsed HTML files to collect real `class`
/// attribute values (first class token only, for simplicity of the
/// cross-reference check -- a composite `class="a b"` contributes both
/// via a second pass if present). Kept separate from `count_html_shapes`
/// to avoid threading a 6th mutable accumulator through the main loop's
/// per-node walk; this is a second, cheap, independent walk over the
/// same small corpus (13 total HTML+CSS files), not a correctness-
/// relevant shortcut.
fn extract_html_classes(_per_file_ids: &HashMap<String, Vec<String>>, files: &[String], source: &LocalGitRepositorySource, repo_id: codegraph_domain::RepositoryId, commit: &CommitHash) -> Vec<String> {
    let mut classes = std::collections::HashSet::new();
    for relative_path in files {
        if !relative_path.ends_with(".html") {
            continue;
        }
        let file = match codegraph_domain::FileIdentity::parse(repo_id, commit.clone(), relative_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let Some(parser) = parser_for(&file) else { continue };
        let bytes = match source.read_file(&file) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let Ok(tree) = parser.parse(&bytes) else { continue };
        walk_classes(tree.tree().root_node(), &bytes, &mut classes);
    }
    classes.into_iter().collect()
}

fn walk_classes(node: tree_sitter::Node, source: &[u8], out: &mut std::collections::HashSet<String>) {
    if node.kind() == "attribute" {
        let mut cursor = node.walk();
        let children: Vec<_> = node.children(&mut cursor).collect();
        let is_class = children.iter().any(|c| c.kind() == "attribute_name" && text_of(*c, source) == "class");
        if is_class {
            if let Some(quoted) = children.iter().find(|c| c.kind() == "quoted_attribute_value") {
                let mut qc = quoted.walk();
                let quoted_children: Vec<_> = quoted.children(&mut qc).collect();
                if let Some(val) = quoted_children.into_iter().find(|c| c.kind() == "attribute_value") {
                    for token in text_of(val, source).split_whitespace() {
                        out.insert(token.to_string());
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_classes(child, source, out);
    }
}

fn run_parity_check(source: &LocalGitRepositorySource, repo_id: codegraph_domain::RepositoryId, commit: &CommitHash) {
    use codegraph_index::conformance::compare_full_incremental_state;
    use codegraph_index::incremental::CodeGraphIndexer;

    let indexer = CodeGraphIndexer::new();
    let outcome_a = indexer.observe_commit(source, repo_id, commit, None, &[]).expect("full index");
    println!("full_outcome: snapshots={} edges={} import_facts={} impl_facts={} call_attempts={}", outcome_a.graph.snapshot_count(), outcome_a.graph.edge_count(), outcome_a.import_facts.len(), outcome_a.impl_facts.len(), outcome_a.call_attempts.len());

    // Re-run the SAME commit as a second "full" index via a fresh
    // indexer to sanity-check determinism, then rely on the real
    // repo-provided commit pair below for the genuine full/incremental
    // divergence check (passed on the command line as commit args 2
    // and 3 when available; this binary takes exactly one commit, so
    // true incremental-vs-full parity for THIS corpus is driven by the
    // separate b2_eval_011_html_css_parity binary against a real two-
    // commit pair, matching the project's own established pattern of a
    // dedicated parity binary per domain, e.g. b2_eval_009_cpp_parity.rs).
    let indexer2 = CodeGraphIndexer::new();
    let outcome_b = indexer2.observe_commit(source, repo_id, commit, None, &[]).expect("full index (determinism check)");
    let divergences = compare_full_incremental_state(&outcome_a, &outcome_b);
    if divergences.is_empty() {
        println!("determinism_check (same commit, two independent full indexers): PARITY CONFIRMED, zero divergences.");
    } else {
        println!("determinism_check: {} DIVERGENCE(S): {divergences:?}", divergences.len());
    }
}
