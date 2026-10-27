mod gate;
mod secrets;

pub use gate::{Gate, GateDecision, GateRejection};
pub use secrets::{SecretException, SecretFinding, SecretPolicy};

#[test]
fn crate_builds() { assert!(true); }
