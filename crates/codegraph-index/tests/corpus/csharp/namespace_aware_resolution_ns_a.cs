// Golden corpus fixture — CALL-RES-A4, C# namespace-aware candidate
// filtering. Pins the exact real shape `calls.rs`'s own
// `namespace_import_excludes_the_non_imported_namespace_candidate`
// unit test already covers: two classes, same simple name, same
// method name, declared in two DIFFERENT namespaces. The caller (see
// namespace_aware_resolution_caller.cs) `using`s ONLY this namespace,
// so A4 must exclude the other namespace's own same-named candidate
// (namespace_aware_resolution_ns_b.cs), leaving exactly one real
// target.
namespace NsA;

public class Foo
{
    public void Target() { }
}
