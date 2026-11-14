//! The `lakehouse-api` integration tests, as ONE test binary (`OPS-32`).
//!
//! Every file in this directory used to be its own test crate: Cargo built and
//! linked 122 binaries for this package alone, each one linking the whole of
//! `lakehouse-api` and its dependency tree again (about 400 MB per binary on
//! CI, and 7 to 9 minutes of every test partition's compile before the first
//! test runs). This file declares each of those files as a module, and the
//! package sets `autotests = false` with one `[[test]] name = "it"` target, so
//! Cargo links the crate once instead of 119 times: three CI test partitions
//! plus the Coverage job each build this binary, not 119.
//!
//! Nothing about a test changed in the move, and neither does isolation under
//! CI: nextest runs every test in a process of its own, so a test still shares
//! no memory, no `DATABASE_URL` and no `ClickHouse` container with another.
//!
//! What does change is running this binary outside nextest (`cargo test`,
//! `cargo llvm-cov`, which the Coverage workflow runs): those run every test
//! of a binary in one process, so
//!
//! - the five modules that start their own `ClickHouse` container
//!   (`ai_sessions`, `chart_numbers`, `breakdown_cap`, `gold_publication`,
//!   `query_result_cap`) can have five containers up at once — memory, not
//!   correctness, since each one is scoped to its module;
//! - `crypto_provider`'s regression check asserts that installing the provider
//!   when one is already set is an `Ok`, and inside one process another test
//!   may have set it first, so that check is weaker there. Under nextest (CI)
//!   it is exactly as strong as before.
//!
//! Three files are deliberately NOT modules here; each is its own `[[test]]`
//! target, because the process it runs in is part of what it tests:
//!
//! - `hostile_uploads` re-runs itself with `--exact` and a bare test name under
//!   `ulimit -v` (`SEC-22`), and must not link the Postgres bootstrap;
//! - `identity_password` installs a global `tracing` subscriber, which
//!   `report_runner` inside this binary also does: one process can hold one;
//! - `parity` is opt-in tooling that reads the operator's own `DATABASE_URL`,
//!   which the test-support bootstrap would overwrite.
//!
//! A new `tests/<name>.rs` gets a `mod <name>;` line below, in alphabetical
//! order, and a `mod common;` in it becomes `use crate::common;`. Without that
//! line the file compiles into nothing and its tests disappear silently, which
//! is why `scripts/ci/check_test_modules.sh` fails the `Repo lints` job for an
//! undeclared file. An edit to an existing file needs nothing.
//!
//! That guard only understands a plain `mod x;` line at column 0 of this file:
//! `#[path = "..."] mod x;` and a `mod` inside a `cfg` block are not parsed
//! here, so neither belongs in this file.

// The shared harness: one declaration for every module below, which each one
// reaches as `crate::common` (its own `use crate::common;`, `OPS-32` D4).
mod common;

mod agents_approval;
mod agents_run;
mod ai_alert_tools;
mod ai_ask_back_turn;
mod ai_build_tool_list;
mod ai_cache_rule;
mod ai_chart_draft;
mod ai_chat_stream;
mod ai_citations;
mod ai_comment_tool;
mod ai_dashboard_reach;
mod ai_embed_tools;
mod ai_export_tools;
mod ai_next_step;
mod ai_report_tools;
mod ai_sessions;
mod ai_sql_help;
mod ai_terms;
mod ai_tool;
mod ai_written_tools;
mod alert_targets_hidden;
mod alert_test_route;
mod assistant_settings_route;
mod auth_password_rules;
mod breakdown_cap;
mod catalog_grants;
mod catalog_restore;
mod chart_alerts;
mod chart_lineage_routes;
mod chart_numbers;
mod chart_png;
mod column_lineage_pipeline;
mod column_lineage_route;
mod column_lineage_sql_source;
mod connector_create_checks;
mod connector_delete_deprovision;
mod connector_discovery_launch;
mod connector_discovery_routes;
mod connector_ingestible_tenant;
mod connector_not_testable;
mod connector_oauth_flow;
mod connector_probe_history_route;
mod connector_repoint;
mod connector_s3_preview;
mod connector_secret_reference;
mod connector_secret_rotate;
mod connector_tenant_access;
mod connector_upload_table;
mod crypto_provider;
mod dashboard_access_routes;
mod dashboard_comments_routes;
mod dashboard_export;
mod dashboard_import;
mod dashboard_reports;
mod dashboard_templates;
mod dashboard_tenant_scope;
mod deployment_settings_route;
mod email_settings_route;
mod embed_tokens;
mod explain_rewrite;
mod gold_export_auth;
mod gold_publication;
mod home_layout;
mod iceberg_door;
mod identity_deactivate;
mod identity_roles;
mod identity_table_access;
mod identity_user_tenants;
mod ingest_run;
mod lakehouse_maintenance;
mod login_throttle;
mod maps_routes;
mod ops_workloads;
mod pdf_export;
mod pdf_real_gotenberg;
mod permission_catalog;
mod pipeline_compile_route;
mod pipeline_definition_save;
mod pipeline_lag_check;
mod pipeline_lag_page;
mod pipeline_preview;
mod pipeline_routing;
mod public_links;
mod quality_anomalies;
mod quality_incidents;
mod quality_on_load;
mod quality_schedules;
mod query_author_exception;
mod query_cancel_route;
mod query_download;
mod query_estimate_read_only;
mod query_history;
mod query_history_statement;
mod query_limits_route;
mod query_result_cap;
mod query_run_limits;
mod query_settings_rule;
mod query_tenant_scope;
mod render_data;
mod report_routes;
mod report_runner;
mod reporting_settings_route;
mod route_auth;
mod saved_query_access;
mod sec11_guard;
mod sec41_verify_ca;
mod security_regressions;
mod semantic_api;
mod settings_resolvers;
mod sign_in_settings_route;
mod sql_lex_corpus;
mod sso_settings_route;
mod table_access;
mod tenant_provisioning;
mod test_connection_route;
mod tier1_saved_query_guard;
mod tier1_writehigh_approval;
mod tier2_governance_drafts;
mod tier2_writehigh_approval;
mod upload_folder_routes;
mod upload_routes;
mod upstream_errors;
