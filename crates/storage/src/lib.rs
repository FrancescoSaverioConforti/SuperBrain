mod atomic;

pub use atomic::{atomic_write, AtomicWriteError};

#[test]
fn crate_builds() { assert!(true); }
