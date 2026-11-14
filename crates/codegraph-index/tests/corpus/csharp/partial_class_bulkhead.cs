// Third file of the B2-ID-010 pin — the real headline case:
// BulkheadPolicy : Policy, where Policy is split across
// partial_class_policy_a.cs and partial_class_policy_b.cs. Must
// resolve to exactly one Extends edge, never zero (the pre-fix bug)
// and never more than one (which would indicate a reconciliation
// regression).
namespace Polly.Bulkhead;

public class BulkheadPolicy : Policy
{
}
