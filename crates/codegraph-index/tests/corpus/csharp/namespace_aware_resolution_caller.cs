// Golden corpus fixture — CALL-RES-A4, C# namespace-aware candidate
// filtering (Roslyn differential companion). See
// namespace_aware_resolution_ns_a.cs/_ns_b.cs for the two same-named
// candidates this disambiguates between.
//
// IMPORTANT, found while extending the Roslyn differential suite to
// this case (not previously checked): the real
// `calls.rs::namespace_import_excludes_the_non_imported_namespace_candidate`
// unit test's own fixture (`"using NsA;\nclass Caller { public void
// Run() { Target(); } }"`) is NOT valid, compilable C# — `Target()`
// called bare inside `Caller.Run()` with no qualifying receiver and
// no static-using is not a legal call to `NsA.Foo.Target()` at all; a
// real C# compiler (and Roslyn) would reject it as "`Target` does not
// exist in the current context," for a reason entirely UNRELATED to
// namespace filtering. That unit test only exercises this project's
// OWN pipeline in isolation (constructing synthetic, non-compiling
// extraction input is valid for a unit test of the extractor/resolver
// alone), but it cannot be used AS WRITTEN for a real Roslyn
// differential, which requires genuinely compilable C#.
//
// This fixture instead uses the smallest REAL, compilable C# shape
// that exercises the identical cross-namespace disambiguation: a
// member-access call through a real receiver of static type `Foo`,
// inside a method body `using NsA;` brings into scope. Roslyn's own
// semantic model resolves `foo.Target()` to `NsA.Foo.Target` (the
// `using`-visible one), exactly mirroring what this project's own
// `resolve_calls_with_namespace_context` is expected to resolve to —
// genuinely diffable against this project's own output for the
// SAME real disambiguation question, even though the call-site
// SYNTAX differs slightly (qualified-by-receiver vs. bare) from the
// non-compiling unit-test fixture.
using NsA;

public class Caller
{
    public void Run(Foo foo)
    {
        foo.Target();
    }
}
