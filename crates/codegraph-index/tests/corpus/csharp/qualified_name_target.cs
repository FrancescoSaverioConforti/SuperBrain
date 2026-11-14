// Golden corpus fixture — CALL-RES-A4 Roslyn differential extension,
// fully-qualified-name case. The real type qualified_name_caller.cs
// calls through its full dotted name (`NsQualified.QualifiedTarget`),
// with no `using` directive at all.
namespace NsQualified
{
    public class QualifiedTarget
    {
        public static void Invoke() { }
    }
}
