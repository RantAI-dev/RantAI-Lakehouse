#![cfg_attr(
    test,
    allow(
        clippy::assert_is_empty,
        reason = "test assertions checking collection emptiness are idiomatic and avoid empty array type casts"
    )
)]
//! Shared domain types and errors for the `RantAI` Lakehouse backend.

pub mod error;
pub mod ident;
pub mod secret;
pub mod status;

pub use error::ApiError;
