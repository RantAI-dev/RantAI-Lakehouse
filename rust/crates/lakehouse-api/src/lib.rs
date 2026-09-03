//! Library-target mirror of the `lakehouse-api` binary's module tree.
//!
//! # Why this file exists
//!
//! `src/main.rs` is a `[[bin]]`-only crate root (`mod auth; mod config;
//! ...`), so before this file existed there was no way for an integration
//! test in `tests/` to link against `routes::router`, `state::AppState`,
//! or `policy::POLICY_TABLE` — a `tests/*.rs` file can only see items
//! reachable through a crate's public API, and a binary crate exposes
//! none. `tests/parity.rs` works around exactly this by treating the
//! service as an opaque HTTP black box (spawn it out-of-process, talk to
//! it over a socket); seeing that limitation called out in its own module
//! doc comment is what this file exists to remove for every OTHER
//! integration test, which needs the real in-process
//! `axum::Router` (via `tower::ServiceExt::oneshot` — no bound port) that
//! only a library target can hand out.
//!
//! `src/main.rs` itself is untouched: it keeps its own private `mod`
//! declarations and compiles as its own, separate crate, exactly as
//! before. This file simply re-declares the same modules as `pub`, from
//! the same source files, as a second, independent compilation of the
//! package — the standard "thin bin, real lib" split, applied without
//! editing the bin side at all.
#![allow(clippy::multiple_crate_versions)]

pub mod auth;
pub mod config;
pub mod connector_probe;
pub mod error;
pub mod gold_export;
pub mod json;
pub mod policy;
pub mod routes;
pub mod state;
pub mod tenant;
