// Golden corpus fixture — CALL-RES-A4, C# namespace-aware candidate
// filtering. The UN-imported namespace's own same-named candidate —
// see namespace_aware_resolution_ns_a.cs's own doc comment for the
// full scenario. This class's own `Target()` must never be chosen by
// A4's namespace filtering, since the caller's own file only `using`s
// NsA, not NsB.
namespace NsB;

public class Foo
{
    public void Target() { }
}
