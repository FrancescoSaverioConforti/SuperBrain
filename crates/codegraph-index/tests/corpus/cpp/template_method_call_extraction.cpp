// Golden corpus fixture — pins B2-ID-011's own C++ half (the same
// extraction bug found in `template_function`/`template_method`
// during the C# fix's own cross-language audit). Bare and
// receiver-qualified template calls must extract callee simple name
// "handle", never the receiver or the full "handle<int>" text.
class Policy {
public:
    void handle() {}
};

void bare_template_call() {
    handle<int>();
}

void qualified_template_call(Policy policy) {
    policy.handle<int>();
}
