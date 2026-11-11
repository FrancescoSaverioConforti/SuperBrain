// Golden corpus fixture — pins B2-ID-009 (C# `method_declaration`
// must extract its own NAME via the grammar's named `name` field,
// never a positional "first identifier child" scan, which would
// misname this method as its own return type "PolicyBuilder"). Exact
// shape from the real repro (App-vNext/Polly's Policy.HandleSyntax.cs,
// `public static PolicyBuilder Handle<TException>()`).
class Policy
{
    public static PolicyBuilder Handle()
    {
        return null;
    }
}

class PolicyBuilder
{
}
