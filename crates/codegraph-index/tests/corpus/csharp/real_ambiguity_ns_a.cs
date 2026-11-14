// Golden corpus fixture — CALL-RES-A4 Roslyn differential extension,
// real-ambiguity case. One of two same-name extension methods (see
// real_ambiguity_ns_b.cs) with an IDENTICAL signature, each importable
// from a different namespace -- real_ambiguity_caller.cs `using`s
// BOTH namespaces and calls `Resolve()` on the same receiver, which
// Roslyn itself rejects as a genuine ambiguous call (CS0121), the
// real compilable counterpart to this project's own
// `resolve_calls_with_namespace_context` "real ambiguity must stay
// honestly Ambiguous" invariant (calls.rs's own
// `using_an_unrelated_third_namespace_never_manufactures_a_false_resolution`
// unit test).
namespace NsAmbiguousA
{
    public class Ambig { }

    public static class AmbigExtensionsA
    {
        public static void Resolve(this Ambig target) { }
    }
}
