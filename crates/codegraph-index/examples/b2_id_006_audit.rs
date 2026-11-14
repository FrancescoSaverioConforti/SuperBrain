//! B2-ID-006 contamination audit (throwaway diagnostic, not a
//! production feature, not a test). Verifies, directly against the
//! real `extract_symbols_reporting_rejections` + `promote` +
//! `resolve_calls` pipeline (no synthetic placeholder-body fixtures),
//! whether class-as-caller duplication (confirmed in B2-EVAL-003 on a
//! real repository) ALSO occurs for: (1) a nested function inside a
//! function, and (2) a Rust trait with a default method body — to
//! check whether the root cause is Python/Class-specific or a general
//! "container signature_text contains nested executable code" defect
//! across SymbolKinds and languages, per the open question in B2-ID-006.

use std::collections::HashMap;

use codegraph_domain::{FileIdentity, RepositorySource, SymbolKind};
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::edges::resolve_calls;
use codegraph_index::extract::extract_symbols_reporting_rejections;
use codegraph_index::local_git_source::LocalGitRepositorySource;
use codegraph_index::parse::parser_for;
use codegraph_index::snapshot::promote;

/// Real-corpus contamination count: given a cloned repo + commit,
/// counts exactly how many `Calls` edges originate from a
/// non-`Function`/non-`Method` snapshot (i.e. a container kind that
/// should never itself be "the caller"), and how many of those are
/// confirmed duplicates of an edge already attributed to a real
/// Function/Method child with the SAME target.
fn contamination_count(repo_path: &str, commit_str: &str) {
    println!("\n=== Full-corpus contamination count: {repo_path} @ {commit_str} ===");
    let repo_path = std::fs::canonicalize(repo_path).expect("repo_path must exist");
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
    let commit = codegraph_domain::CommitHash::parse(commit_str).expect("commit must be a valid SHA-1");

    let files = source.list_files_at(repo_id, &commit).expect("list_files_at");
    let mut snapshots: Vec<codegraph_index::snapshot::SymbolSnapshot> = Vec::new();
    for relative_path in &files {
        let file = match FileIdentity::parse(repo_id, commit.clone(), relative_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let Some(parser) = parser_for(&file) else { continue };
        let Ok(bytes) = source.read_file(&file) else { continue };
        let Ok(tree) = parser.parse(&bytes) else { continue };
        let (candidates, _rejections) = extract_symbols_reporting_rejections(&tree);
        for candidate in candidates {
            if let Ok(snapshot) = promote(&candidate, file.clone()) {
                snapshots.push(snapshot);
            }
        }
    }
    println!("  total snapshots: {}", snapshots.len());

    let calls_edges = resolve_calls(&snapshots).expect("resolve_calls");
    println!("  total Calls edges: {}", calls_edges.len());

    let mut by_kind: HashMap<SymbolKind, usize> = HashMap::new();
    for edge in &calls_edges {
        if let Some(from_snap) = snapshots.iter().find(|s| s.id() == edge.from) {
            *by_kind.entry(from_snap.kind()).or_insert(0) += 1;
        }
    }
    let mut by_kind_vec: Vec<_> = by_kind.into_iter().collect();
    by_kind_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    println!("  Calls edges by caller SymbolKind:");
    for (kind, count) in &by_kind_vec {
        println!("    {kind:?}: {count}");
    }

    if std::env::args().any(|a| a == "--dump-edges") {
        let mut lines: Vec<String> = calls_edges
            .iter()
            .map(|e| {
                let from = snapshots.iter().find(|s| s.id() == e.from).map(|s| format!("{}::{}[{:?}]", s.file().relative_path(), s.name().as_str(), s.kind()));
                let to = snapshots.iter().find(|s| s.id() == e.to).map(|s| format!("{}::{}[{:?}]", s.file().relative_path(), s.name().as_str(), s.kind()));
                format!("{from:?} -> {to:?}")
            })
            .collect();
        lines.sort();
        println!("\n--- FULL EDGE DUMP (sorted) ---");
        for line in &lines {
            println!("{line}");
        }
    }

    // Confirmed-duplicate count: a container-kind (Class/Struct/
    // Interface/Enum) caller edge (container, to) is a confirmed
    // duplicate if some OTHER snapshot nested inside that container's
    // span (same file, strictly inside) also has an edge to the SAME
    // `to` target.
    let container_kinds = [SymbolKind::Class, SymbolKind::Struct, SymbolKind::Interface, SymbolKind::Enum];
    let mut confirmed_duplicates = 0usize;
    let mut container_originated = 0usize;
    for edge in &calls_edges {
        let Some(from_snap) = snapshots.iter().find(|s| s.id() == edge.from) else { continue };
        if !container_kinds.contains(&from_snap.kind()) {
            continue;
        }
        container_originated += 1;
        let has_nested_duplicate = calls_edges.iter().any(|other| {
            if other.to != edge.to || other.from == edge.from {
                return false;
            }
            let Some(other_from) = snapshots.iter().find(|s| s.id() == other.from) else { return false };
            other_from.file().relative_path() == from_snap.file().relative_path()
        });
        if has_nested_duplicate {
            confirmed_duplicates += 1;
        }
    }
    println!("  container-kind-originated Calls edges (Class/Struct/Interface/Enum as caller): {container_originated}");
    println!("  of those, confirmed duplicate of a same-file edge to the same target from a different caller: {confirmed_duplicates}");
    println!(
        "  container-originated as % of total Calls edges: {:.1}%",
        if !calls_edges.is_empty() { 100.0 * container_originated as f64 / calls_edges.len() as f64 } else { 0.0 }
    );
}

fn run_case(label: &str, relative_path: &str, source: &str) {
    println!("\n=== Case: {label} ({relative_path}) ===");
    let mut registry = codegraph_domain::RepositoryRegistry::new();
    let cwd = std::env::current_dir().unwrap();
    let repo_root = codegraph_domain::RepositoryRoot::parse(&cwd).unwrap();
    let repo_id = registry.register(repo_root);
    let commit = codegraph_domain::CommitHash::parse("0000000000000000000000000000000000000000").unwrap();
    let file = FileIdentity::parse(repo_id, commit, relative_path).unwrap();
    let Some(parser) = parser_for(&file) else {
        println!("  no parser for {relative_path}, skipping");
        return;
    };
    let tree = parser.parse(source.as_bytes()).expect("parse must succeed for this hand-written snippet");
    let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
    println!("  raw_candidates: {}, rejections: {}", candidates.len(), rejections.len());
    for c in &candidates {
        println!("    candidate: {} [{}] span={:?}", c.name, c.kind_hint, c.span);
    }
    let mut snapshots = Vec::new();
    for c in candidates {
        match promote(&c, file.clone()) {
            Ok(s) => snapshots.push(s),
            Err(e) => println!("    promotion_failure: {e}"),
        }
    }
    let edges = resolve_calls(&snapshots).expect("resolve_calls");
    println!("  calls_edges: {}", edges.len());
    for edge in &edges {
        let from = snapshots.iter().find(|s| s.id() == edge.from);
        let to = snapshots.iter().find(|s| s.id() == edge.to);
        println!(
            "    {:?} [{:?}] -> {:?} :: {}",
            from.map(|s| format!("{}[{:?}]", s.name().as_str(), s.kind())),
            edge.evidence.method(),
            to.map(|s| format!("{}[{:?}]", s.name().as_str(), s.kind())),
            edge.evidence.note()
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 {
        // Full-corpus contamination count mode:
        // `b2_id_006_audit <repo_path> <commit>`
        contamination_count(&args[1], &args[2]);
        return;
    }

    // Case 1: Python class with ONE method calling ONE target — the
    // exact real-world shape found in B2-EVAL-003's
    // CommentsRepository::_get_comment_from_db_record. Expect (if the
    // bug is real): 2 edges to the same target (Method->target,
    // Class->target), not 1.
    run_case(
        "Python class, single method, single call (B2-EVAL-003 real shape)",
        "src/repo.py",
        r#"
class CommentsRepository:
    async def _get_comment_from_db_record(self, comment_row):
        return Comment(id_=comment_row["id"])


class Comment:
    pass
"#,
    );

    // Case 2: Python function containing a NESTED function, where only
    // the nested function makes the call. Expect (if the bug
    // generalizes beyond Class): 2 edges (inner->target correct,
    // outer->target spurious), not 1.
    run_case(
        "Python nested function (outer defines inner; only inner calls target)",
        "src/nested.py",
        r#"
def outer():
    def inner():
        target()
    return inner


def target():
    pass
"#,
    );

    // Case 3: Rust trait with a default method body that itself calls
    // a free function. Expect (if the bug generalizes to Rust
    // Interface/trait_item): 2 edges (method->target correct,
    // trait->target spurious), not 1.
    run_case(
        "Rust trait with default method body calling a free function",
        "src/trait_default.rs",
        r#"
trait Greeter {
    fn greet(&self) {
        helper();
    }
}

fn helper() {}
"#,
    );

    // Case 4: sibling methods in the same class calling the SAME
    // target, to check whether class-level duplication multiplies
    // (one spurious Class->target edge per distinct target, or one per
    // call SITE before Edge dedup collapses it) rather than being
    // capped by the existing per-target HashMap dedup already in
    // `resolve_calls` (Task/B2-ID-002's per_target collapse, documented
    // in calls.rs, which collapses MULTIPLE occurrences of the SAME
    // (from,to) pair into one edge — this only collapses WITHIN one
    // caller's own occurrences, so it would NOT by itself collapse the
    // separate Class-attributed edge, which is a DIFFERENT `from`).
    run_case(
        "Python class, two sibling methods, both call the same target",
        "src/siblings.py",
        r#"
class Svc:
    def a(self):
        target()

    def b(self):
        target()


def target():
    pass
"#,
    );

    // Case 5: a Field whose RHS is a call with a nested call argument
    // (self.x = f(g())). Checks the open question from B2-ID-006: is a
    // Field's own call-originating edge a legitimate, sole observation
    // (nothing else could have produced it — the field assignment
    // IS the only executable statement at this granularity), or does
    // it suffer the same over-broad-span duplication? Expect (if Field
    // is a genuine, non-overlapping single-statement scope): exactly 2
    // edges (init_like->f, init_like->g), neither being a duplicate of
    // anything else, since no other snapshot's span could also contain
    // this assignment statement.
    run_case(
        "Python Field with nested call in RHS (self.x = f(g()))",
        "src/field_rhs.py",
        r#"
class M:
    def __init__(self):
        self.x = f(g())


def f(v):
    return v


def g():
    return 1
"#,
    );
}
