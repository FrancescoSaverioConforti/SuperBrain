// Golden corpus fixture — pins B2-ID-011 (C# generic method calls,
// both bare and receiver-qualified, must be extracted with callee
// simple name "Handle", never "Handle<TimeoutException>", the
// receiver, or omitted entirely). Minimized from the real
// App-vNext/Polly repro (`policy.Handle<TimeoutException>(...)`).
class Policy
{
    public void Handle<TException>()
    {
    }
}

class Caller
{
    void Handle<TException>()
    {
    }

    void BareGenericCall()
    {
        Handle<System.Exception>();
    }

    void ReceiverQualifiedGenericCall(Policy policy)
    {
        policy.Handle<System.Exception>();
    }
}
