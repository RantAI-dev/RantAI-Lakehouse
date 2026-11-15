//! The `lakehouse-store` integration tests, as ONE test binary (`OPS-32`).
//!
//! Every file in this directory used to be its own test crate: Cargo built and
//! linked 41 binaries, each one linking `lakehouse-store`, `sqlx` and their
//! dependencies again, and every CI test partition paid for all 41 links before
//! its first test ran. This file declares each of those files as a module, and
//! the package sets `autotests = false` with one `[[test]] name = "it"` target,
//! so the crate links once -- one CI test partition builds it, not 41.
//!
//! No test changed in the move, and neither does isolation under CI: nextest
//! runs every test in a process of its own, so each still gets its own
//! `#[sqlx::test]` database and its own module-scoped `static MIGRATOR`.
//!
//! A new `tests/<name>.rs` gets a `mod <name>;` line below, in alphabetical
//! order. Without that line the file compiles into nothing and its tests
//! disappear silently, which is why `scripts/ci/check_test_modules.sh` fails
//! the `Repo lints` job for an undeclared file. An edit to an existing file
//! needs nothing.

mod agents;
mod alert_delivery;
mod alert_state;
mod annotation;
mod audit;
mod capture_lag;
mod chat_term;
mod column_lineage;
mod connector_discovery;
mod connector_ingest_spec;
mod connector_ingest_spec_seed;
mod connector_oauth;
mod connector_run;
mod connector_type;
mod connectors;
mod dashboard_access;
mod dashboard_comments;
mod geo_map;
mod gold_publication;
mod governance;
mod governed_tag;
mod home_layout;
mod identity;
mod knowledge;
mod maintenance_policy;
mod migrations_upgrade;
mod outside_reads;
mod overview;
mod pipelines;
mod public_links;
mod quality;
mod queries;
mod query_limits;
mod report_subscriptions;
mod reporting_settings;
mod schema;
mod schema_change;
mod semantic;
mod sessions;
mod upload_folders;
mod uploads;
