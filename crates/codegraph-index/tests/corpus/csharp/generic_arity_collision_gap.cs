// Golden corpus fixture — pins the documented, Moderate,
// out-of-B2-ID-010-scope generic-arity-collision gap. `IBulkheadPolicy`
// (arity 0, non-partial) and `IBulkheadPolicy<TResult>` (arity 1,
// non-partial) share one bare name; `BulkheadPolicy : IBulkheadPolicy`
// must stay AMBIGUOUS (no edge) because this pipeline's base-name
// lookup has no generic-arity-aware disambiguation. Minimized from the
// real App-vNext/Polly repro (src/Polly/Bulkhead/IBulkheadPolicy.cs).
// This fixture deliberately asserts NO Implements edge is produced —
// a future generic-arity-aware fix would need to flip this.
namespace Polly.Bulkhead;

public interface IBulkheadPolicy
{
}

public interface IBulkheadPolicy<TResult> : IBulkheadPolicy
{
}

public class BulkheadPolicy : IBulkheadPolicy
{
}
