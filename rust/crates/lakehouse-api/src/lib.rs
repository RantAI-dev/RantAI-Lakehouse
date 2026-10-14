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
//! `src/main.rs` is the thin binary over this library: it imports
//! `config`, `routes` and `state` from here with
//! `use lakehouse_api::{…}` instead of re-declaring the module tree with
//! `mod`. It used to re-declare all of it, which compiled every module,
//! and ran every unit test in it, twice (once per target); see
//! `docs/plans/CI-SPEED-PLAN.md` section 3.3. The binary's own tests (the
//! router registration tests and the `bootstrap_*` tests, which call
//! functions private to `main.rs`) stay in `main.rs` and compile there.
#![allow(clippy::multiple_crate_versions)]

pub mod auth;
pub mod bounded;
pub mod bronze_stats_cache;
pub mod catalog_search_cache;
pub mod config;
pub mod connector_deprovision;
pub mod connector_discover;
pub mod connector_probe;
pub mod connector_secret_store;
pub mod error;
pub mod gold_export;
pub mod gold_export_history;
pub mod gold_lock;
pub mod health;
pub mod internal_hosts;
pub mod json;
pub mod lakehouse_catalog;
pub mod lakekeeper_token;
pub mod next_run;
pub mod pipeline_source;
pub mod policy;
pub mod policy_engine;
pub mod routes;
pub mod rustfs_client;
pub mod sql_guard;
pub mod sql_rewrite;
pub mod state;
pub mod tenant;
pub mod tenant_scope;
pub mod transform_grammar;
pub mod upload_parquet;
pub mod upload_parse;
pub mod upload_store;
pub mod upload_workbook;
