//! Quality & Semantic Reliability Hardening, Phase 2 — the Roslyn
//! differential-testing PoC, extended per Phase 1's own
//! `DIFFERENTIAL_TESTING_ARCHITECTURE.md` design into a small,
//! AUTOMATED comparison: this harness (a) invokes `tools/roslyn-oracle`
//! via `dotnet run` (never embedded into the production indexer —
//! `tools/roslyn-oracle` is referenced by no `Cargo.toml` anywhere in
//! this workspace, confirmed by grep before writing this file), (b)
//! runs this pipeline's OWN real extraction against the identical
//! golden-corpus C# fixtures, and (c) produces a structured,
//! machine-readable diff between the two — never merely two separate
//! JSON emissions a human has to eyeball side by side, which is what
//! the Phase 1 PoC alone produced.
//!
//! # Not a `#[test]`, deliberately
//!
//! This is an `example`, run manually (`cargo run --example
//! roslyn_differential_suite`) or by a future, explicitly-configured
//! CI job — never part of `cargo test`'s own default run. A `#[test]`
//! that silently requires a working `dotnet` SDK to be installed on
//! every machine running this crate's test suite would violate the
//! "never a runtime dependency of the actual product" instruction in
//! spirit even if not in the literal production binary: it would make
//! `cargo test` itself depend on an external toolchain this project's
//! own `Cargo.toml` never declares. Running it as an example keeps the
//! oracle fully OPT-IN.
//!
//! # Scope
//!
//! Covers the SAME four golden-corpus fixture groups
//! `tools/roslyn-oracle/Program.cs` already defines (method names —
//! B2-ID-009; partial classes — B2-ID-010; generic-arity collision —
//! the still-open Moderate gap; generic method calls — B2-ID-011).
//! Per the Phase 2 authorization's own instruction ("don't try to
//! cover all of C#"), this does not attempt overload/call-target
//! comparison beyond what the existing PoC's own `call_site`
//! observations already provide — a genuinely richer overload oracle
//! (comparing Roslyn's own resolved overload against this pipeline's
//! `resolve_calls`/`resolve_calls_with_negative_knowledge` output for
//! the SAME ambiguous-overload call site) is named as real future
//! work in this harness's own closing report, not attempted here.

use std::collections::HashMap;
use std::process::Command;

use codegraph_index::csharp_declarations::extract_csharp_type_declaration_contexts;
use codegraph_index::extract::extract_symbols_reporting_rejections;
use codegraph_index::parse::{CSharpParser, LanguageParser};

/// One normalized observation this harness can compare across both
/// sides — a small, deliberately lossy projection of each side's own
/// richer native representation, kept to exactly the fields both
/// sides can express (SuperBrain's own `SymbolSnapshot`/
/// `CSharpTypeDeclarationContext` vs. Roslyn's own declared-symbol
/// JSON), so the diff below is never comparing apples to a field that
/// only exists on one side.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct NormalizedTypeObservation {
    file: String,
    name: String,
    declaration_kind: String, // "class" | "interface"
    is_partial: bool,
    namespace_name: String,
    generic_arity: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct NormalizedMethodObservation {
    file: String,
    name: String,
    generic_arity: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Divergence {
    TypeObservationOnlyIn { side: &'static str, observation: NormalizedTypeObservation },
    MethodObservationOnlyIn { side: &'static str, observation: NormalizedMethodObservation },
}

fn main() {
    let repo_root = find_repo_root();
    let corpus_dir = repo_root.join("crates/codegraph-index/tests/corpus/csharp");
    let oracle_dir = repo_root.join("tools/roslyn-oracle");

    println!("=== Roslyn differential suite ===");
    println!("corpus: {}", corpus_dir.display());
    println!("oracle: {}", oracle_dir.display());

    let roslyn_output = run_roslyn_oracle(&oracle_dir);
    let (roslyn_types, roslyn_methods) = parse_roslyn_output(&roslyn_output);

    let (superbrain_types, superbrain_methods) = extract_superbrain_observations(&corpus_dir);

    println!("\nRoslyn observed {} type declarations, {} method declarations", roslyn_types.len(), roslyn_methods.len());
    println!("SuperBrain observed {} type declarations, {} method declarations", superbrain_types.len(), superbrain_methods.len());

    let divergences = compare(&roslyn_types, &superbrain_types, &roslyn_methods, &superbrain_methods);

    println!("\n--- Divergences found: {} ---", divergences.len());
    for d in &divergences {
        println!("  {d:?}");
    }

    if divergences.is_empty() {
        println!("\nNo divergences: SuperBrain's own type/method declaration extraction agrees with Roslyn's real semantic model on every fixture in this corpus slice.");
    } else {
        println!("\nAt least one divergence found -- see docs/research/DIFFERENTIAL_TESTING_ARCHITECTURE.md for how to interpret this (a divergence here is NOT automatically a SuperBrain bug -- Roslyn resolves full .NET semantics this pipeline deliberately does not attempt; review each one against that document's own residual-limitations section before treating it as a finding).");
    }
}

fn run_roslyn_oracle(oracle_dir: &std::path::Path) -> String {
    let output = Command::new("dotnet").arg("run").current_dir(oracle_dir).output().expect("`dotnet run` must succeed -- confirm the .NET SDK is installed; this harness is an opt-in oracle, never a hard dependency of this crate's own test suite");
    if !output.status.success() {
        eprintln!("dotnet run stderr:\n{}", String::from_utf8_lossy(&output.stderr));
        panic!("tools/roslyn-oracle exited with a non-zero status");
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Minimal, schema-scoped JSON Lines reading for the Roslyn oracle's
/// own fixed output shape -- deliberately reuses the SAME hand-rolled
/// parsing discipline `codegraph_index::ground_truth` already
/// established for the identical reason (no `serde_json` dependency
/// exists in this workspace; introducing one for this one example is
/// not this Phase 2 pass's own call to make unilaterally).
fn parse_roslyn_output(output: &str) -> (Vec<NormalizedTypeObservation>, Vec<NormalizedMethodObservation>) {
    let mut types = Vec::new();
    let mut methods = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let fields = parse_flat_json_line(line);
        match fields.get("kind").map(String::as_str) {
            Some("type_declaration") => {
                types.push(NormalizedTypeObservation {
                    file: fields.get("file").cloned().unwrap_or_default(),
                    name: fields.get("name").cloned().unwrap_or_default(),
                    declaration_kind: fields.get("declaration_kind").cloned().unwrap_or_default(),
                    is_partial: fields.get("is_partial").map(|v| v == "true").unwrap_or(false),
                    namespace_name: fields.get("namespace_name").cloned().unwrap_or_default(),
                    generic_arity: fields.get("generic_arity").and_then(|v| v.parse().ok()).unwrap_or(0),
                });
            }
            Some("method_declaration") => {
                methods.push(NormalizedMethodObservation {
                    file: fields.get("file").cloned().unwrap_or_default(),
                    name: fields.get("name").cloned().unwrap_or_default(),
                    generic_arity: fields.get("generic_arity").and_then(|v| v.parse().ok()).unwrap_or(0),
                });
            }
            _ => {}
        }
    }
    (types, methods)
}

/// Extracts `"key": value` pairs from one flat JSON object line,
/// values as raw strings (quotes stripped for string values, left
/// as-is for bool/number) -- sufficient for this harness's own fixed,
/// known field set; not a general JSON parser.
fn parse_flat_json_line(line: &str) -> HashMap<String, String> {
    let inner = line.trim().trim_start_matches('{').trim_end_matches('}');
    let mut fields = HashMap::new();
    // Split on top-level commas only (never inside a `[...]` array
    // value, e.g. base_list_names) -- tracked via bracket depth.
    let mut depth = 0i32;
    let mut current = String::new();
    let mut parts = Vec::new();
    for c in inner.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(current.clone());
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    for part in parts {
        if let Some((key, value)) = part.split_once(':') {
            let key = key.trim().trim_matches('"').to_string();
            let value = value.trim();
            let value = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 { value[1..value.len() - 1].to_string() } else { value.to_string() };
            fields.insert(key, value);
        }
    }
    fields
}

/// Runs SuperBrain's own real extraction
/// (`extract_symbols_reporting_rejections` + `promote` +
/// `extract_csharp_type_declaration_contexts`) against every fixture
/// file in `corpus_dir`, normalized to the SAME comparable shape
/// `parse_roslyn_output` produces.
fn extract_superbrain_observations(corpus_dir: &std::path::Path) -> (Vec<NormalizedTypeObservation>, Vec<NormalizedMethodObservation>) {
    let mut types = Vec::new();
    let mut methods = Vec::new();

    let mut entries: Vec<_> = std::fs::read_dir(corpus_dir).expect("corpus dir must exist").filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|ext| ext == "cs")).collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let tree = CSharpParser.parse(&bytes).unwrap();

        let (candidates, _rejections) = extract_symbols_reporting_rejections(&tree);
        for candidate in &candidates {
            if candidate.kind_hint == "method_declaration" {
                // generic_arity for a method isn't tracked by
                // csharp_declarations.rs (that module only covers
                // class_declaration/interface_declaration) -- derive
                // it the same way the Roslyn oracle's own
                // MethodDeclarationSyntax.TypeParameterList does, by
                // checking the signature text for a `<...>` right
                // after the method name (a deliberately simple,
                // comparison-only heuristic; this is NOT a claim about
                // how production extraction itself should represent
                // method generic arity, which is out of this
                // example's own scope).
                let arity = method_generic_arity_from_signature(&candidate.signature_text, &candidate.name);
                methods.push(NormalizedMethodObservation { file: file_name.clone(), name: candidate.name.clone(), generic_arity: arity });
            }
        }

        for context in extract_csharp_type_declaration_contexts(&tree) {
            let declaration_kind = match context.type_kind {
                codegraph_index::csharp_declarations::CSharpTypeKind::Class => "class",
                codegraph_index::csharp_declarations::CSharpTypeKind::Interface => "interface",
            };
            types.push(NormalizedTypeObservation {
                file: file_name.clone(),
                name: context.simple_name.clone(),
                declaration_kind: declaration_kind.to_string(),
                is_partial: context.is_partial,
                namespace_name: context.namespace.clone(),
                generic_arity: context.generic_arity,
            });
        }
    }

    types.sort();
    methods.sort();
    (types, methods)
}

/// Comparison-only heuristic: looks for `<name_literal><...>` in the
/// signature text to recover a method's own generic arity for THIS
/// harness's own diff purposes. Confirmed sufficient for this corpus's
/// own fixture set by direct inspection; not proposed as a production
/// extraction mechanism.
fn method_generic_arity_from_signature(signature_text: &str, name: &str) -> usize {
    let Some(name_pos) = signature_text.find(name) else {
        return 0;
    };
    let after_name = &signature_text[name_pos + name.len()..];
    let Some(rest) = after_name.strip_prefix('<') else {
        return 0;
    };
    let Some(close) = rest.find('>') else {
        return 0;
    };
    rest[..close].split(',').filter(|s| !s.trim().is_empty()).count()
}

fn compare(roslyn_types: &[NormalizedTypeObservation], superbrain_types: &[NormalizedTypeObservation], roslyn_methods: &[NormalizedMethodObservation], superbrain_methods: &[NormalizedMethodObservation]) -> Vec<Divergence> {
    let mut divergences = Vec::new();

    let roslyn_type_set: std::collections::HashSet<_> = roslyn_types.iter().cloned().collect();
    let superbrain_type_set: std::collections::HashSet<_> = superbrain_types.iter().cloned().collect();
    for only_roslyn in roslyn_type_set.difference(&superbrain_type_set) {
        divergences.push(Divergence::TypeObservationOnlyIn { side: "roslyn", observation: only_roslyn.clone() });
    }
    for only_superbrain in superbrain_type_set.difference(&roslyn_type_set) {
        divergences.push(Divergence::TypeObservationOnlyIn { side: "superbrain", observation: only_superbrain.clone() });
    }

    let roslyn_method_set: std::collections::HashSet<_> = roslyn_methods.iter().cloned().collect();
    let superbrain_method_set: std::collections::HashSet<_> = superbrain_methods.iter().cloned().collect();
    for only_roslyn in roslyn_method_set.difference(&superbrain_method_set) {
        divergences.push(Divergence::MethodObservationOnlyIn { side: "roslyn", observation: only_roslyn.clone() });
    }
    for only_superbrain in superbrain_method_set.difference(&roslyn_method_set) {
        divergences.push(Divergence::MethodObservationOnlyIn { side: "superbrain", observation: only_superbrain.clone() });
    }

    divergences
}

fn find_repo_root() -> std::path::PathBuf {
    let mut dir = std::env::current_dir().unwrap();
    loop {
        if dir.join("crates").is_dir() {
            return dir;
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => panic!("could not find repo root (no ancestor directory contains 'crates')"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Self-test discipline, same as `conformance.rs`'s own adversarial
    /// case: before trusting this harness's own real "0 divergences"
    /// result against the real Roslyn oracle, prove `compare()` itself
    /// actually DETECTS a real divergence when one genuinely exists,
    /// rather than vacuously reporting "equivalent" no matter what two
    /// inputs it receives. This test never invokes `dotnet` and never
    /// touches the real corpus -- synthetic data only.
    #[test]
    fn compare_detects_a_genuine_type_observation_divergence() {
        let roslyn_types = vec![NormalizedTypeObservation { file: "a.cs".to_string(), name: "Foo".to_string(), declaration_kind: "class".to_string(), is_partial: false, namespace_name: "".to_string(), generic_arity: 0 }];
        let superbrain_types = vec![NormalizedTypeObservation { file: "a.cs".to_string(), name: "Foo".to_string(), declaration_kind: "class".to_string(), is_partial: true, namespace_name: "".to_string(), generic_arity: 0 }];

        let divergences = compare(&roslyn_types, &superbrain_types, &[], &[]);

        assert_eq!(divergences.len(), 2, "a genuine field-level mismatch (is_partial) must be reported as two divergences (one 'only in roslyn', one 'only in superbrain'), never silently reported as equivalent");
    }

    #[test]
    fn compare_reports_zero_divergences_for_genuinely_identical_input() {
        let types = vec![NormalizedTypeObservation { file: "a.cs".to_string(), name: "Foo".to_string(), declaration_kind: "class".to_string(), is_partial: false, namespace_name: "".to_string(), generic_arity: 0 }];
        let divergences = compare(&types, &types, &[], &[]);
        assert!(divergences.is_empty(), "genuinely identical input must report zero divergences -- the true-negative side of the same invariant");
    }

    #[test]
    fn parse_flat_json_line_handles_array_fields_without_splitting_on_their_internal_commas() {
        let line = r#"{"kind":"type_declaration","file":"x.cs","name":"Foo","base_list_names":["A","B"],"generic_arity":0}"#;
        let fields = parse_flat_json_line(line);
        assert_eq!(fields.get("name"), Some(&"Foo".to_string()));
        assert_eq!(fields.get("generic_arity"), Some(&"0".to_string()), "a field AFTER an array field must still parse correctly, confirming the array's own internal comma did not corrupt the split");
    }

    #[test]
    fn method_generic_arity_from_signature_counts_real_type_parameters() {
        assert_eq!(method_generic_arity_from_signature("public void Handle<TException>() {}", "Handle"), 1);
        assert_eq!(method_generic_arity_from_signature("public void Handle<T1, T2>() {}", "Handle"), 2);
        assert_eq!(method_generic_arity_from_signature("public void Plain() {}", "Plain"), 0);
    }
}
