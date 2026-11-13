// Golden corpus fixture — CALL-RES-A4 Roslyn differential extension,
// namespace-alias case. The real type a `using Alias = ...;` aliased
// using (see namespace_alias_caller.cs) points at.
namespace NsAlias
{
    public class AliasTarget
    {
        public void Invoke() { }
    }
}
