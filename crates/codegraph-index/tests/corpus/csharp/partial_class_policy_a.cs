// Golden corpus fixture — pins B2-ID-010 (C# `partial class`
// fragmentation must be reconciled for Extends/Implements resolution).
// Minimized from the real App-vNext/Polly repro: `Policy` split across
// multiple files (here, two), `BulkheadPolicy : Policy` must resolve
// despite Policy's own fragmentation. Paired with
// partial_class_policy_b.cs and partial_class_bulkhead.cs.
namespace Polly;

public partial class Policy
{
}
