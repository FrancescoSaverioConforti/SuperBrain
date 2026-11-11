// Golden corpus fixture — pins the documented, known Rust macro-
// opacity gap (`edges/calls.rs`'s own module doc comment): a call
// wrapped inside a macro invocation's argument list (`assert_eq!(add(1,
// 2), 3)`) is NOT found by `resolve_calls`, because `add(1, 2)` inside
// `assert_eq!(...)`'s argument list is an opaque `token_tree` to
// tree-sitter, never a real `call_expression` node. This fixture
// deliberately does NOT assert a Calls edge from `uses_macro` to
// `add` (there should be none, per the known, documented gap) — it
// exists so a future macro-aware heuristic has an exact, minimal,
// already-wired-up corpus case to flip once built.
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn uses_macro() {
    assert_eq!(add(1, 2), 3);
}
