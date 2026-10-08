//! Shared domain types and errors for the `RantAI` Lakehouse backend.

pub mod error;
pub mod ident;
pub mod secret;
pub mod status;

pub use error::ApiError;

#[test]
fn deliberately_failing_probe_test() {
    assert_eq!(2 + 2, 5, "deliberate failure to verify ci-required gate catches broken Rust test");
}
