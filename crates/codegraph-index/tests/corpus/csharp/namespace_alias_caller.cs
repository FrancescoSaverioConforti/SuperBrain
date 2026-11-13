// Golden corpus fixture — CALL-RES-A4 Roslyn differential extension,
// namespace-alias case. Real C# `using Alias = NsAlias.AliasTarget;`
// — an ALIASED using, never a bare namespace import. This project's
// own `bare_using_namespaces_in_file` (calls.rs) deliberately excludes
// aliased usings from namespace-visibility filtering (see
// `bare_using_namespaces_in_file_excludes_aliased_usings_and_other_files`),
// so a receiver-qualified call through this alias is expected to
// resolve, if at all, only through plain name matching on the real
// declared type -- the alias name itself is never a namespace this
// pipeline's own candidate-visibility filtering reasons about.
using Alias = NsAlias.AliasTarget;

public class AliasCaller
{
    public void Run(Alias target)
    {
        target.Invoke();
    }
}
