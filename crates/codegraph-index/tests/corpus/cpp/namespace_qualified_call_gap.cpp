// Golden corpus fixture — pins the NEW residual limitation B2-ID-011's
// own independent review found: a namespace-qualified call
// (`ns::foo()`) is invisible to extraction regardless of whether the
// callee is generic/templated, confirmed pre-existing and orthogonal
// to B2-ID-011's own fix (not fixed there, not fixed here — this
// fixture exists so a future fix has an exact minimal corpus case).
// This fixture deliberately asserts NO Calls edge is produced for the
// namespace-qualified call.
namespace ns {
    void foo() {}
}

void caller() {
    ns::foo();
}
