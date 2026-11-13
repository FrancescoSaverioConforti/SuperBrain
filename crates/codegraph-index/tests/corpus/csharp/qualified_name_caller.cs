// Golden corpus fixture — CALL-RES-A4 Roslyn differential extension,
// fully-qualified-name case. Calls `NsQualified.QualifiedTarget.Invoke()`
// through its complete dotted namespace-qualified name, with NO
// `using` directive at all -- Roslyn's real semantic model resolves
// this purely from the qualified text itself, independent of any
// using/visibility reasoning. This project's own
// `is_receiver_qualified`/call-site extraction (calls.rs) treats a
// `member_access_expression` callee the same way regardless of
// whether the receiver segment is a real namespace-qualified
// expression or a plain local variable -- it extracts the bare
// trailing method name ("Invoke") and matches on name alone, so a
// fully-qualified call site is expected to resolve identically to an
// unqualified one with the same bare target name, which is the
// genuinely differential question this fixture exists to pose.
public class QualifiedNameCaller
{
    public void Run()
    {
        NsQualified.QualifiedTarget.Invoke();
    }
}
