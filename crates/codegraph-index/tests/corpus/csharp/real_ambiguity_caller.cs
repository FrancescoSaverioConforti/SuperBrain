// Golden corpus fixture — CALL-RES-A4 Roslyn differential extension,
// real-ambiguity case. `using`s BOTH colliding-extension-method
// namespaces (see real_ambiguity_ns_a.cs/_ns_b.cs) and calls
// `target.Resolve()` -- a genuine, real-C#-compiler-rejected ambiguous
// call (CS0121). Roslyn's own semantic model reports this call site's
// `SymbolInfo.CandidateReason` as `Ambiguous` with 2 candidate symbols
// and no single resolved target -- the real Roslyn-side counterpart
// to this project's own "real ambiguity must stay honestly Ambiguous,
// never silently narrowed to a false resolution" invariant.
using NsAmbiguousA;
using NsAmbiguousB;

public class RealAmbiguityCaller
{
    public void Run(Ambig target)
    {
        target.Resolve();
    }
}
