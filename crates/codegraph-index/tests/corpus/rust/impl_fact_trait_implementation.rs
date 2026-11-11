// Golden corpus fixture — pins B2-ID-008 (Rust `impl Trait for Type`
// must produce an Implements edge via the ImplFact mechanism, since
// `impl_item` is never extracted as a symbol of its own). This is the
// exact canonical repro shape used throughout this project's own test
// suite for B2-ID-008 (`impls.rs`, `extends_implements.rs`,
// `incremental/indexer.rs`), reused verbatim here for consistency.
trait Shape {
    fn area(&self) -> f64;
}

struct Circle;

impl Shape for Circle {
    fn area(&self) -> f64 {
        1.0
    }
}
