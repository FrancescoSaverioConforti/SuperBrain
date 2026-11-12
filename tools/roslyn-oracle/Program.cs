// Quality & Semantic Reliability Hardening, Phase 1, deliverable 10 --
// a real, working Roslyn-based differential-testing proof-of-concept.
//
// What this does: parses the SAME golden-corpus C# fixtures this
// project's own tests/corpus/csharp/ directory already contains
// (cited explicitly per file below), using Roslyn's real C# parser
// and (where a case needs symbol resolution, not just syntax) its
// real semantic model -- never re-implementing C# semantics by hand.
// Emits normalized JSON Lines, one object per declaration/call-target
// observation, in a shape directly comparable to this pipeline's own
// SymbolSnapshot/Edge output (name, kind, namespace, base-list names,
// resolved-call-target name) -- NOT identical field names (Roslyn's
// own vocabulary differs), but diffable by a human or a future script
// reading both outputs side by side.
//
// This tool is NEVER a runtime dependency of SuperBrain/codegraph-index
// itself -- it lives under tools/, is not referenced by any Cargo
// manifest, and exists purely as a test/validation oracle, per the
// user's own explicit instruction: "Non rendere SuperBrain dipendente
// runtime da questi strumenti. Sono oracle di test/validation, non
// necessariamente componenti production."

using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;
using System.Text.Json;

var corpusDir = Path.Combine(FindRepoRoot(), "crates", "codegraph-index", "tests", "corpus", "csharp");

// The SAME fixture set as tests/golden_corpus.rs's own
// b2_id_009/b2_id_010/generic_arity_collision/b2_id_011 tests --
// deliberately reusing the identical real-repository-minimized
// fixtures, not separate Roslyn-specific ones, so the two outputs are
// a genuine diff of the SAME input, not two different inputs that
// happen to look similar.
var fixtureGroups = new (string Name, string[] Files)[]
{
    ("b2_id_009_method_name_vs_return_type", new[] { "method_name_vs_return_type.cs" }),
    ("b2_id_010_partial_class_policy", new[] { "partial_class_policy_a.cs", "partial_class_policy_b.cs", "partial_class_bulkhead.cs" }),
    ("generic_arity_collision_gap", new[] { "generic_arity_collision_gap.cs" }),
    ("b2_id_011_generic_method_call", new[] { "generic_method_call_extraction.cs" }),
    // B2-EVAL-008 resumption (Semantic Reliability Hardening Phase 2
    // stress-test): the REAL, unminimized App-vNext/Polly source files
    // this evaluation's own findings were discovered against, not the
    // golden-corpus's own minimized synthetics above -- covers method
    // names (B2-ID-009), the real 8-way partial-class `Policy` plus
    // `BulkheadPolicy : Policy, IBulkheadPolicy` (B2-ID-010), the real
    // `IBulkheadPolicy`/`IBulkheadPolicy<TResult>` generic-arity
    // collision, real namespaces (`Polly`/`Polly.Bulkhead`), and the
    // real `Handle<TException>` overload family, all together in one
    // compilation unit so Roslyn's own cross-file semantic binding is
    // exercised, not just single-file syntax. Many of these real
    // files' own dependencies (`Context`, `BulkheadEngine`, etc.) are
    // NOT included here and will not resolve -- Roslyn's own `?.`-
    // guarded null results for those specific symbols are the HONEST,
    // expected outcome (a real external/unindexed reference, the same
    // shape `EvidenceTier::Unresolved` exists for on this project's
    // own side), not a harness defect.
    ("b2_eval_008_real_polly_files", new[] { "b2_eval_008_real/real_policy_context_and_keys.cs", "b2_eval_008_real/real_policy_handle_syntax.cs", "b2_eval_008_real/real_is_policy.cs", "b2_eval_008_real/real_ibulkhead_policy.cs", "b2_eval_008_real/real_bulkhead_policy.cs" }),
};

var jsonOptions = new JsonSerializerOptions { WriteIndented = false };

foreach (var group in fixtureGroups)
{
    Console.WriteLine($"=== {group.Name} ===");

    var syntaxTrees = new List<SyntaxTree>();
    foreach (var file in group.Files)
    {
        var path = Path.Combine(corpusDir, file);
        var text = File.ReadAllText(path);
        syntaxTrees.Add(CSharpSyntaxTree.ParseText(text, path: path));
    }

    // A real semantic model requires a real Compilation -- referencing
    // the actual .NET runtime's own core library so names like
    // "System.Exception" resolve, exactly as a real C# compiler would
    // need. This is the genuinely "differential" part of this PoC:
    // Roslyn's own symbol resolution, not tree-sitter's syntax-only
    // view, backs every observation below.
    var compilation = CSharpCompilation.Create(
        assemblyName: group.Name,
        syntaxTrees: syntaxTrees,
        references: new[] { MetadataReference.CreateFromFile(typeof(object).Assembly.Location) },
        options: new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));

    foreach (var tree in syntaxTrees)
    {
        var model = compilation.GetSemanticModel(tree);
        var root = tree.GetRoot();

        // --- Type declarations (class/interface), with namespace and
        // base-list names -- directly comparable to this pipeline's
        // own SymbolSnapshot (name/kind) and Extends/Implements edges
        // (base-list names, pre-disambiguation).
        foreach (var typeDecl in root.DescendantNodes().OfType<TypeDeclarationSyntax>())
        {
            var symbol = model.GetDeclaredSymbol(typeDecl);
            var observation = new
            {
                kind = "type_declaration",
                file = Path.GetFileName(tree.FilePath),
                name = typeDecl.Identifier.Text,
                declaration_kind = typeDecl.Keyword.Text, // "class" | "interface"
                is_partial = typeDecl.Modifiers.Any(m => m.Text == "partial"),
                // Roslyn's OWN namespace resolution -- the real
                // semantic-model answer, not a hand-rolled
                // canonicalization like csharp_declarations.rs's own
                // namespace_of. Comparable to that function's own
                // output for the SAME fixture.
                namespace_name = symbol?.ContainingNamespace is { IsGlobalNamespace: false } ns ? ns.ToDisplayString() : "",
                generic_arity = typeDecl.TypeParameterList?.Parameters.Count ?? 0,
                // Base list names exactly as WRITTEN (syntax-level,
                // pre-disambiguation) -- directly comparable to this
                // project's own base_names_in/collect_csharp_base_list.
                base_list_names = typeDecl.BaseList?.Types.Select(t => t.Type.ToString()).ToArray() ?? Array.Empty<string>(),
                // The REAL semantic-level answer Roslyn's own symbol
                // resolution gives for "what does this type actually
                // extend/implement" -- the genuinely differential
                // signal this PoC exists to provide: this project's
                // own pipeline's conclusion can be checked against
                // THIS, not just against another syntax-level reading.
                semantic_base_type = symbol?.BaseType is { SpecialType: SpecialType.None } bt ? bt.ToDisplayString() : null,
                semantic_interfaces = symbol?.Interfaces.Select(i => i.ToDisplayString()).ToArray() ?? Array.Empty<string>(),
            };
            Console.WriteLine(JsonSerializer.Serialize(observation, jsonOptions));
        }

        // --- Method declarations, with name (never confused with
        // return type -- the EXACT B2-ID-009 shape) and generic arity.
        foreach (var methodDecl in root.DescendantNodes().OfType<MethodDeclarationSyntax>())
        {
            var observation = new
            {
                kind = "method_declaration",
                file = Path.GetFileName(tree.FilePath),
                name = methodDecl.Identifier.Text,
                return_type = methodDecl.ReturnType.ToString(),
                generic_arity = methodDecl.TypeParameterList?.Parameters.Count ?? 0,
            };
            Console.WriteLine(JsonSerializer.Serialize(observation, jsonOptions));
        }

        // --- Call sites, with Roslyn's own REAL semantic resolution
        // of the target -- the genuinely differential signal for
        // B2-ID-011's own generic-call case: this pipeline's own
        // resolve_calls uses pure name matching; this shows what a
        // real semantic model resolves the SAME call site to.
        foreach (var invocation in root.DescendantNodes().OfType<InvocationExpressionSyntax>())
        {
            var symbolInfo = model.GetSymbolInfo(invocation);
            var resolvedMethod = symbolInfo.Symbol as IMethodSymbol;
            var observation = new
            {
                kind = "call_site",
                file = Path.GetFileName(tree.FilePath),
                syntax_text = invocation.ToString(),
                // Roslyn's OWN resolved target -- null means Roslyn
                // itself could not bind the call (e.g. a genuinely
                // ambiguous or unresolved reference, the same
                // real-world shape this pipeline's own Ambiguous/
                // Unresolved tiers exist for).
                roslyn_resolved_target = resolvedMethod?.Name,
                roslyn_resolved_containing_type = resolvedMethod?.ContainingType?.ToDisplayString(),
                roslyn_candidate_count = symbolInfo.CandidateSymbols.Length,
                roslyn_resolution_candidate_reason = symbolInfo.CandidateReason.ToString(),
            };
            Console.WriteLine(JsonSerializer.Serialize(observation, jsonOptions));
        }
    }
    Console.WriteLine();
}

static string FindRepoRoot()
{
    var dir = new DirectoryInfo(Directory.GetCurrentDirectory());
    while (dir != null && !Directory.Exists(Path.Combine(dir.FullName, "crates")))
    {
        dir = dir.Parent;
    }
    return dir?.FullName ?? throw new DirectoryNotFoundException("could not find repo root (no ancestor directory contains 'crates')");
}
