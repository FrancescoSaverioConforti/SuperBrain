//! B2-EVAL-012: TypeScript backend/Node.js server-side domain
//! standalone evaluation (`Sairyss/domain-driven-hexagon`).
//!
//! Drives the SAME real production pipeline every prior domain harness
//! uses, via `CodeGraphIndexer::observe_commit` (the real
//! parse -> extract -> promote -> lineage -> edges -> CodeGraph chain,
//! including import-aware Calls resolution and negative-knowledge
//! CallAttempt tracking), distinguishing this round from B2-EVAL-004
//! (Frontend/UI TypeScript) and B2-EVAL-010 (JS/CommonJS): this corpus
//! is a NestJS-style DDD backend -- decorators, async/await, DI via
//! constructor injection, generics, no React/JSX anywhere.
//!
//! Usage: `cargo run --release --example b2_eval_012_ts_backend -- <repo_path> <commit>`

use std::collections::HashMap;
use std::time::Instant;

use codegraph_domain::{CommitHash, EdgeKind, EvidenceTier, RepositoryId, SymbolKind};
use codegraph_index::allowlist::RepositoryAllowlist;
use codegraph_index::incremental::CodeGraphIndexer;
use codegraph_index::local_git_source::LocalGitRepositorySource;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: b2_eval_012_ts_backend <repo_path> <commit>");
        std::process::exit(2);
    }
    let repo_path = std::fs::canonicalize(&args[1]).expect("repo_path must exist");
    let commit_str = &args[2];

    let mut allowlist = RepositoryAllowlist::new();
    let allowlisted_root = allowlist.authorize(&repo_path).expect("authorize repo root");
    let mut registry = codegraph_domain::RepositoryRegistry::new();
    let repo_root = codegraph_domain::RepositoryRoot::parse(&repo_path).unwrap();
    let repo_id = registry.register(repo_root);

    let root_for_id = move |id: RepositoryId| -> Option<codegraph_index::allowlist::AllowlistedRoot> {
        if id == repo_id {
            Some(allowlisted_root.clone())
        } else {
            None
        }
    };
    let source = LocalGitRepositorySource::new(&allowlist, &root_for_id);
    let commit = CommitHash::parse(commit_str).expect("commit must be a valid SHA-1");

    println!("=== B2-EVAL-012: TypeScript backend/Node.js domain (Sairyss/domain-driven-hexagon) ===");
    println!("repo: {}", repo_path.display());
    println!("commit: {commit_str}");

    let indexer = CodeGraphIndexer::new();
    let t0 = Instant::now();
    let outcome = indexer.observe_commit(&source, repo_id, &commit, None, &[]).expect("full index");
    let full_elapsed = t0.elapsed();

    println!("\n--- A. Corpus / symbol metrics (real production pipeline) ---");
    println!("snapshots: {}", outcome.graph.snapshot_count());
    println!("edges: {}", outcome.graph.edge_count());
    println!("import_facts: {}", outcome.import_facts.len());
    println!("impl_facts: {}", outcome.impl_facts.len());
    println!("call_attempts: {}", outcome.call_attempts.len());

    let mut kind_counts: HashMap<SymbolKind, usize> = HashMap::new();
    for snap in outcome.graph.all_snapshots() {
        *kind_counts.entry(snap.kind()).or_insert(0) += 1;
    }
    let mut kind_vec: Vec<_> = kind_counts.into_iter().collect();
    kind_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (kind, count) in &kind_vec {
        println!("  {kind:?}: {count}");
    }

    println!("\n--- B. Edge kind breakdown ---");
    let mut edge_kind_counts: HashMap<EdgeKind, usize> = HashMap::new();
    for edge in outcome.graph.all_edges() {
        *edge_kind_counts.entry(edge.kind).or_insert(0) += 1;
    }
    let mut ek_vec: Vec<_> = edge_kind_counts.into_iter().collect();
    ek_vec.sort_by_key(|(k, _)| format!("{k:?}"));
    for (kind, count) in &ek_vec {
        println!("  {kind:?}: {count}");
    }

    println!("\n--- C. Negative knowledge: CallAttempt tier + reason breakdown ---");
    let mut tier_counts: HashMap<String, usize> = HashMap::new();
    let mut reason_counts: HashMap<String, usize> = HashMap::new();
    for attempt in outcome.call_attempts.iter() {
        *tier_counts.entry(format!("{:?}", attempt.tier)).or_insert(0) += 1;
        *reason_counts.entry(format!("{:?}", attempt.reason)).or_insert(0) += 1;
    }
    let mut tier_vec: Vec<_> = tier_counts.into_iter().collect();
    tier_vec.sort();
    println!("by tier:");
    for (tier, count) in &tier_vec {
        println!("  {tier}: {count}");
    }
    let mut reason_vec: Vec<_> = reason_counts.into_iter().collect();
    reason_vec.sort();
    println!("by reason:");
    for (reason, count) in &reason_vec {
        println!("  {reason}: {count}");
    }

    // A5 evidence: how many unresolved/ambiguous CallAttempts are
    // caused by REAL overload/generic-arity ambiguity between 2+ real
    // candidates (AmbiguousCandidates tier/reason -- the one category
    // full type inference (A5) would actually help resolve), versus
    // everything else (external targets, no matching name/import,
    // extraction rejections -- none of which A5 fixes).
    let ambiguous_real_candidates = outcome.call_attempts.iter().filter(|a| a.tier == EvidenceTier::Ambiguous).count();
    let total_attempts = outcome.call_attempts.len();
    println!("\n--- D. A5 (type inference) evidence ---");
    println!("CallAttempts with tier=Ambiguous (2+ real candidates, name-based matching cannot disambiguate -- the category A5 would target): {ambiguous_real_candidates}/{total_attempts}");
    println!("(All other tiers reflect zero-candidate cases -- external targets, no import match, extraction rejection -- which A5/type-inference does not address; only Ambiguous reflects a real same-name overload/generic collision.)");

    println!("\n--- E. Full index timing ---");
    println!("full_index_elapsed: {full_elapsed:?}");
    println!("(see b2_eval_012_ts_backend_parity for the real two-commit full/incremental parity check)");
}
