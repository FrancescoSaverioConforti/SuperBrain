//! B2-ID-017 (Critical): `parser_for` -- the one dispatch EVERY
//! production call site uses (`incremental::pipeline::index_files`,
//! `edges::calls`, `edges::extends_implements`, `edges::instantiates`,
//! `edges::tests_for`, `edges::js_prototype_ownership`, `agent_policy`) --
//! had no match arm for "html"/"css", so every HTML/CSS file was
//! silently dropped as an unsupported extension, exactly like a
//! genuinely unknown language. This despite `HtmlParser`/`CssParser` and
//! `extract::html`/`extract::css` already being fully implemented and
//! unit-tested in isolation: the extraction/promotion layer was always
//! correct, only the single dispatcher that decides which parser a file
//! gets was missing the two arms.
//!
//! These tests exercise the fix through the REAL `CodeGraphIndexer`
//! (not an isolated extractor call), confirming HTML/CSS symbols now
//! actually reach the published graph in production, not just in the
//! extractor's own `#[cfg(test)]` module. `parser_for`'s own dispatch-
//! level unit tests (plus the pre-fix-failing regression pin) live in
//! `codegraph_index::parse::tests`, next to the function itself.

use std::collections::HashMap;

use codegraph_domain::{CommitHash, FileIdentity, RepositoryId, RepositoryRegistry, RepositoryRoot, RepositorySource, SourceError, SymbolKind};
use codegraph_index::conformance::compare_full_incremental_state;
use codegraph_index::incremental::CodeGraphIndexer;

/// Same test double as `full_incremental_equivalence.rs`'s own
/// `MultiCommitSource` (that file's copy is private to its own crate,
/// so this is an independent, equivalent definition, not a shared
/// import -- matching this project's existing convention for this
/// exact situation).
struct MultiCommitSource {
    files: HashMap<(RepositoryId, String), Vec<String>>,
    contents: HashMap<FileIdentity, Vec<u8>>,
}

impl MultiCommitSource {
    fn new() -> Self {
        Self { files: HashMap::new(), contents: HashMap::new() }
    }

    fn with_file(mut self, file: FileIdentity, bytes: &[u8]) -> Self {
        self.files.entry((file.repository(), file.commit().as_str().to_string())).or_default().push(file.relative_path().to_string());
        self.contents.insert(file, bytes.to_vec());
        self
    }
}

impl RepositorySource for MultiCommitSource {
    fn list_files_at(&self, repository: RepositoryId, commit: &CommitHash) -> Result<Vec<String>, SourceError> {
        self.files.get(&(repository, commit.as_str().to_string())).cloned().ok_or(SourceError::UnknownCommit { repo: repository.as_uuid(), commit: commit.as_str().to_string() })
    }

    fn read_file(&self, file: &FileIdentity) -> Result<Vec<u8>, SourceError> {
        self.contents.get(file).cloned().ok_or_else(|| SourceError::UnknownFile {
            repo: file.repository().as_uuid(),
            commit: file.commit().as_str().to_string(),
            file: file.relative_path().to_string(),
        })
    }
}

fn test_repo() -> RepositoryId {
    let mut registry = RepositoryRegistry::new();
    registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
}

fn assert_full_equals_incremental(full: &codegraph_index::incremental::ObserveOutcome, incremental: &codegraph_index::incremental::ObserveOutcome, context: &str) {
    let divergences = compare_full_incremental_state(full, incremental);
    assert!(divergences.is_empty(), "{context}: full/incremental divergence(s) found:\n{divergences:#?}");
}

const INDEX_HTML: &[u8] = b"<!doctype html>\n<html>\n<body>\n<div id=\"main\"><p id=\"intro\">hello</p></div>\n</body>\n</html>\n";
const STYLE_CSS: &[u8] = b"#main { color: red; }\n#intro { font-weight: bold; }\n@keyframes fade { from { opacity: 0; } to { opacity: 1; } }\n";

/// Test 3 (required): a real `.html` fixture, indexed through the real
/// `CodeGraphIndexer::observe_commit`, must actually produce `Symbol`s
/// in the published graph -- not merely parse without error. Before the
/// fix, `files_supported` for this exact file was 0 and the graph had
/// zero HTML snapshots (B2-EVAL-011's own repro on tbreuss/html-boilerplate).
#[test]
fn b2_id_017_real_indexer_extracts_html_id_elements() {
    let repo = test_repo();
    let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
    let source = MultiCommitSource::new().with_file(FileIdentity::parse(repo, commit.clone(), "index.html").unwrap(), INDEX_HTML);

    let indexer = CodeGraphIndexer::new();
    let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

    let html_snapshots: Vec<_> = outcome.graph.all_snapshots().into_iter().filter(|s| s.kind() == SymbolKind::HtmlIdElement).collect();
    assert_eq!(html_snapshots.len(), 2, "expected exactly the two id-bearing elements (main, intro); got: {html_snapshots:#?}");
    let names: Vec<&str> = html_snapshots.iter().map(|s| s.name().as_str()).collect();
    assert!(names.contains(&"main"), "expected an HtmlIdElement named 'main'");
    assert!(names.contains(&"intro"), "expected an HtmlIdElement named 'intro'");
}

/// Test 4 (required): same proof for `.css` -- real indexer, real
/// published graph, both `CssRule` and `CssKeyframes` kinds confirmed
/// present (not just symbol count > 0, which could hide a kind-mapping
/// gap).
#[test]
fn b2_id_017_real_indexer_extracts_css_rules_and_keyframes() {
    let repo = test_repo();
    let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
    let source = MultiCommitSource::new().with_file(FileIdentity::parse(repo, commit.clone(), "style.css").unwrap(), STYLE_CSS);

    let indexer = CodeGraphIndexer::new();
    let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

    let css_rules: Vec<_> = outcome.graph.all_snapshots().into_iter().filter(|s| s.kind() == SymbolKind::CssRule).collect();
    let css_keyframes: Vec<_> = outcome.graph.all_snapshots().into_iter().filter(|s| s.kind() == SymbolKind::CssKeyframes).collect();

    assert_eq!(css_rules.len(), 2, "expected exactly the two id selectors (#main, #intro); got: {css_rules:#?}");
    assert_eq!(css_keyframes.len(), 1, "expected exactly the one @keyframes (fade); got: {css_keyframes:#?}");
    assert_eq!(css_keyframes[0].name().as_str(), "fade");
}

/// Test 5 (required): full/incremental parity for a mixed HTML+CSS
/// corpus. Incremental: commit A has both files; commit B adds a new,
/// genuinely changed CSS rule, leaving `index.html` untouched (exercising
/// snapshot reuse, not just fresh extraction). Full: a from-scratch
/// indexer observing commit B directly. Both sides must agree exactly
/// (multiset-equal snapshots/edges/facts), per this project's existing
/// equivalence gate.
#[test]
fn b2_id_017_full_equals_incremental_for_mixed_html_css_corpus() {
    let repo = test_repo();
    let commit_a = CommitHash::parse(&"a".repeat(40)).unwrap();
    let commit_b = CommitHash::parse(&"b".repeat(40)).unwrap();

    const STYLE_A: &[u8] = b"#main { color: red; }\n";
    const STYLE_B: &[u8] = b"#main { color: red; }\n#footer { color: blue; }\n";

    let incremental_source_a = MultiCommitSource::new()
        .with_file(FileIdentity::parse(repo, commit_a.clone(), "index.html").unwrap(), INDEX_HTML)
        .with_file(FileIdentity::parse(repo, commit_a.clone(), "style.css").unwrap(), STYLE_A);
    let incremental_indexer = CodeGraphIndexer::new();
    incremental_indexer.observe_commit(&incremental_source_a, repo, &commit_a, None, &[]).unwrap();

    let incremental_source_b = incremental_source_a
        .with_file(FileIdentity::parse(repo, commit_b.clone(), "index.html").unwrap(), INDEX_HTML)
        .with_file(FileIdentity::parse(repo, commit_b.clone(), "style.css").unwrap(), STYLE_B);
    let incremental = incremental_indexer.observe_commit(&incremental_source_b, repo, &commit_b, Some(&commit_a), &["style.css".to_string()]).unwrap();

    let full_repo = test_repo();
    let full_source = MultiCommitSource::new()
        .with_file(FileIdentity::parse(full_repo, commit_b.clone(), "index.html").unwrap(), INDEX_HTML)
        .with_file(FileIdentity::parse(full_repo, commit_b.clone(), "style.css").unwrap(), STYLE_B);
    let full_indexer = CodeGraphIndexer::new();
    let full = full_indexer.observe_commit(&full_source, full_repo, &commit_b, None, &[]).unwrap();

    assert_full_equals_incremental(&full, &incremental, "mixed HTML+CSS corpus, full vs. incremental");

    // Sanity: confirm this isn't a vacuous pass -- both sides actually
    // produced the real symbols (2 HTML id-elements + 2 CSS rules each).
    let html_count = full.graph.all_snapshots().into_iter().filter(|s| s.kind() == SymbolKind::HtmlIdElement).count();
    let css_count = full.graph.all_snapshots().into_iter().filter(|s| s.kind() == SymbolKind::CssRule).count();
    assert_eq!(html_count, 2, "equivalence check above must not be vacuously true for HTML");
    assert_eq!(css_count, 2, "equivalence check above must not be vacuously true for CSS");
}

/// Confirms no edge type is fabricated as a side effect of this fix:
/// HTML/CSS symbols now exist, but no cross-reference edge resolver in
/// this codebase targets HtmlIdElement/CssRule/CssKeyframes today (that
/// is explicitly a later, separate task per [d79ag] -- this fix is pure
/// dispatch wiring, not new edge semantics). Guards against a future
/// change accidentally widening this fix's scope without a deliberate
/// decision.
#[test]
fn b2_id_017_fix_introduces_no_edges_for_html_or_css_symbols() {
    let repo = test_repo();
    let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
    let source = MultiCommitSource::new()
        .with_file(FileIdentity::parse(repo, commit.clone(), "index.html").unwrap(), INDEX_HTML)
        .with_file(FileIdentity::parse(repo, commit.clone(), "style.css").unwrap(), STYLE_CSS);

    let indexer = CodeGraphIndexer::new();
    let outcome = indexer.observe_commit(&source, repo, &commit, None, &[]).unwrap();

    for snapshot in outcome.graph.all_snapshots() {
        if matches!(snapshot.kind(), SymbolKind::HtmlIdElement | SymbolKind::CssRule | SymbolKind::CssKeyframes) {
            let outgoing = outcome.graph.outgoing_edges(snapshot.id(), None);
            let incoming = outcome.graph.incoming_edges(snapshot.id(), None);
            assert!(outgoing.is_empty(), "no edge resolver targets HTML/CSS symbols yet; found outgoing: {outgoing:#?}");
            assert!(incoming.is_empty(), "no edge resolver targets HTML/CSS symbols yet; found incoming: {incoming:#?}");
        }
    }
}
