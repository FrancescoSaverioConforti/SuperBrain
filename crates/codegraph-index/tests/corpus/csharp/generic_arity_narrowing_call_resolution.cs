// Golden corpus fixture — Semantic Resolution & Reliability Checkpoint,
// Priority 1 (arity-aware call resolution) and its required negative
// case. Minimized from the real App-vNext/Polly `Handle<TException>`
// overload family (src/Polly/Policy.HandleSyntax.cs), which is the
// real-world shape this fixture pins.
//
// `ArityResolvesAmbiguity`: two same-named candidates with DIFFERENT
// generic arity (`Pick()` vs `Pick<T>()`) — the call site's own
// generic arity (1, since it calls `Pick<TException>()`) must narrow
// the candidate set to exactly one survivor, resolving via
// NameHeuristic (arity-narrowing), never staying Ambiguous.
//
// `SameArityStillAmbiguous` (the required negative case): two
// same-named candidates that BOTH have generic arity 1
// (`Choose<T>(int)` vs `Choose<T>(string)`, differing only in their
// ORDINARY parameter type) — arity narrowing has no discriminating
// power here (both survive the arity filter), so the call must stay
// genuinely Ambiguous. This is the exact real-corpus shape of Polly's
// own `Handle<TException>()`/`Handle<TException>(Func<TException,bool>)`
// pair, which the real evaluation (B2-EVAL-006/008) confirms produces
// zero arity-narrowed resolutions on the real corpus. Named
// `Choose`, not `Pick`, specifically so this class's own candidate
// pool never collides by bare name with `Picker`'s `Pick`/`Pick<T>`
// above -- the two scenarios in this one file must stay independently
// measurable, each exercising exactly the candidate set its own
// comment describes.
namespace ArityNarrowing;

public class Picker
{
    public void Pick() { }
    public void Pick<T>() { }
}

public class AmbiguousPicker
{
    public void Choose<T>(int ordinary) { }
    public void Choose<T>(string ordinary) { }
}

public class Caller
{
    public void ArityResolvesAmbiguity(Picker picker)
    {
        picker.Pick<System.Exception>();
    }

    public void SameArityStillAmbiguous(AmbiguousPicker picker)
    {
        picker.Choose<System.Exception>(1);
    }
}
