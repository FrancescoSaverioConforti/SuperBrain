// Golden corpus fixture — CALL-RES-A4 Roslyn differential extension,
// real-ambiguity case. The second same-name, same-signature extension
// method -- see real_ambiguity_ns_a.cs's own doc comment for the full
// scenario. `Ambig` itself is declared only in NsAmbiguousA; this
// namespace supplies only the colliding extension method, applicable
// to that same type.
namespace NsAmbiguousB
{
    public static class AmbigExtensionsB
    {
        public static void Resolve(this NsAmbiguousA.Ambig target) { }
    }
}
