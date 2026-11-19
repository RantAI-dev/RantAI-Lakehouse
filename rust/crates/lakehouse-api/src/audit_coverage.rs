//! The audit coverage table and the central audit writer (`GOV-1`).
//!
//! # Why this exists
//!
//! The audit trail (`audit_event`, migration `0024`) used to record only the
//! routes whose handler remembered to call a writer. A new route that
//! forgot was recorded by nothing and caught by nothing. [`AUDIT_TABLE`] is
//! the one place that settles it: every route of
//! [`crate::policy::POLICY_TABLE`] that changes something (`POST`, `PUT`,
//! `PATCH`, `DELETE`) has exactly one entry here, and `tests/audit_coverage.rs`
//! fails until a new route has one. The entry says who writes the row:
//!
//! - [`Coverage::Handler`]: the handler writes its own row, with the
//!   `action` the entry names (the guard test checks that the action is a
//!   string literal in `src/routes/`). Handler rows keep what they record
//!   today, including the old and new values of a role, policy, table-access
//!   or grant change.
//! - [`Coverage::Central`]: [`audit_gate`] writes the row after the handler
//!   answers. No handler code is involved, so the routes added to the
//!   console later are covered the day they are listed.
//! - [`Coverage::Excluded`]: no row, for one of the reasons the feature page
//!   lists as its Decision 1 (read-like POSTs, "test" buttons, the public
//!   data and frame routes, machine callbacks, multipart upload parts,
//!   personal UI state). Nothing else may be excluded, and each entry
//!   carries its reason so the guard test can print the whole list.
//!
//! The table holds the four changing methods. `HEAD` and the `*` entries of
//! the Iceberg door are not in it: the door serves reads and refuses every
//! write itself (`routes::iceberg_door`), so no console change reaches it.
//!
//! # The central writer
//!
//! [`audit_gate`] is layered inside [`crate::policy::auth_gate`] (so the
//! [`lakehouse_auth::Principal`] is already in the request) and outside the
//! query scope. It acts only when the matched `(method, pattern)` is
//! `Central`. After the handler answers it writes one row: the principal,
//! the entry's `action` and `resource_kind`, the `resource_id` read from the
//! path parameter or the query-string parameter the entry names
//! ([`IdParam`]), the outcome by status (`2xx` is
//! `executed`, `5xx` is `failed`, every other status is `refused`), and the
//! status code as `detail`. `args` is `{"fields": [...]}`: the sorted
//! top-level key names of a JSON object body, never a value (feature page
//! Decision 3). A body is read only when its content type is JSON and its
//! declared length is at most [`MAX_BODY_PEEK_BYTES`]; any other body passes
//! through untouched and the row says `{"fields": null}`, so the per-route
//! body limits and the streaming uploads behave as before.
//!
//! # Refused at the gate
//!
//! When `auth_gate` answers `403` (missing permission, password rotation
//! pending) the handler never runs. [`record_gate_refusal`] writes a
//! `refused` row for every route that is not `Excluded`, `Handler` routes
//! included: nothing else wrote one, so there is no double row. The row is
//! built from the principal, the matched pattern, the method and the
//! permission text, never from a header or the body, so a client cannot
//! make the gate write a row it chose. A `401` writes nothing: nobody is
//! signed in, and a `404` or `405` never reaches a route.
//!
//! # A failed write
//!
//! [`record_best_effort`] is the one writer that does not undo or fail what
//! it records (feature page Decision 4): on a failed insert it logs the
//! action and the class of the store error, never `args`, and counts it in
//! [`AppState::audit_write_failure_count`] (since the process started). Paths that
//! must fail closed (`catalog_drop::audit_first`, `dashboard::
//! record_download_audit`, `report_runner`) keep their own writers.
//!
//! # Limits
//!
//! - A request the outer timeout layer cuts off writes no row: the
//!   handler's future is dropped before this layer can write.
//! - `resource_id` is read from a path parameter or a query-string parameter
//!   (`DELETE /api/dashboard/boards?id=`), percent-decoded and cut to 200
//!   characters. It is null for a create whose id exists only in the response,
//!   for a route whose id travels in the body, and for a request that did not
//!   send the query parameter. A value in a body or in a response is never
//!   read for it (feature page Decision 3): the row then names the kind and
//!   the action, not the object.
//! - A key name in a body is recorded as sent, except that a NUL character
//!   becomes U+FFFD. At most [`MAX_FIELD_NAMES`] names of at most
//!   [`MAX_FIELD_NAME_CHARS`] characters are kept, so one request cannot write
//!   a megabyte row.
//! - Postgres refuses a NUL in a `TEXT` column and in `jsonb`, and a client
//!   can put one in a key name (`{"\u0000": 1}`) or, percent-encoded, in an
//!   id (`?id=%00`). A row that kept it would fail to insert and the change
//!   would stand with no row, or a refusal at the gate would go unrecorded.
//!   [`without_nul`] is applied to every client-supplied string a row stores:
//!   the field names and the resource id. The other columns hold text the
//!   server wrote (the action and kind from the table, the status code, the
//!   missing permission, the principal's own name and id), so a client has no
//!   way to put a character in them.
//! - The assistant's tools and the other in-process callers call Rust
//!   functions, never a route over HTTP, so a `Central` row does not repeat
//!   a row the assistant wrote (verified at `212b431a6`).

use std::collections::BTreeMap;

use axum::body::Body;
use axum::extract::{FromRequestParts, MatchedPath, RawPathParams, Request, State};
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::StoreError;
use lakehouse_store::audit::NewAuditEvent;
use serde::de::IgnoredAny;
use serde_json::{Value, json};

use crate::error::ApiRejection;
use crate::state::AppState;

/// Who writes the audit row of a route, or why nobody does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// The handler writes its own row. `action` is the primary action it
    /// writes (a handler may write several); for the few routes whose row
    /// carries a name chosen at run time, it is the name the route's refusal
    /// row at the gate carries.
    Handler {
        /// The audit action, `module.verb` style where the module has one.
        action: &'static str,
    },
    /// [`audit_gate`] writes the row after the handler answers.
    Central {
        /// The audit action, `module.verb`.
        action: &'static str,
        /// The kind of object the route changes.
        resource_kind: &'static str,
        /// Where the object's id is in the request (see the module doc
        /// comment, "Limits").
        id_param: IdParam,
    },
    /// No row, for a reason the feature page's Decision 1 lists.
    Excluded {
        /// Which Decision 1 item the route falls under, and why.
        reason: &'static str,
    },
}

/// Where a `Central` route's object id is found in the request. The id is
/// never read from a body or a response (module doc comment, "Limits").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdParam {
    /// The id is not in the path or the query string: the row names the kind
    /// but not the object.
    None,
    /// The id is the path parameter of this name, as the router matched it.
    Path(&'static str),
    /// The id is the query-string parameter of this name, percent-decoded
    /// (`DELETE /api/dashboard/boards?id=`). The table marks these
    /// explicitly so the guard test can still hold a [`IdParam::Path`] name
    /// to the route's own pattern.
    Query(&'static str),
}

/// The largest JSON body [`audit_gate`] reads to name its fields: axum's own
/// default limit for a body an extractor reads, so a body the handler would
/// accept is one the writer can read, and a larger one (a route that raised
/// its own limit for an upload) is never held in memory twice.
pub const MAX_BODY_PEEK_BYTES: usize = 2 * 1024 * 1024;

/// The most field names one row keeps. A body with more is cut here and the
/// row says `"fieldsTruncated": true`.
pub const MAX_FIELD_NAMES: usize = 200;

/// The most characters of one field name a row keeps.
pub const MAX_FIELD_NAME_CHARS: usize = 100;

/// The most characters of a resource id a row keeps. An id comes from the
/// request's own path, so it is bounded like any other client text.
const MAX_RESOURCE_ID_CHARS: usize = 200;

/// Who is named on a `Central` row written for a request with no principal.
/// No `Central` route is public, so this is the answer to a state that
/// should not occur, and it says so instead of inventing a person.
const ACTOR_NOT_SIGNED_IN: &str = "No signed-in caller";

const EX_PUBLIC: &str =
    "Decision 1: public data and frame route; it serves a view and changes nothing";
const EX_READ_LIKE: &str = "Decision 1: read-like POST (preview, validate, estimate, compile, lag-check, check or compare); it answers a question and records no change";
const EX_LAG_CHECK: &str = "Decision 1 lists lag-check among the read-like POSTs, but it changes state: the orchestrator calls it on a schedule, it advances the pipeline's lag streak and can send the behind-target alert, and no console user's change is behind it (the run a `run` decision asks for is launched by the orchestrator and kept in its run history)";
const EX_OVERVIEW: &str = "Read-like POST: `POST /api/overview` takes no body and only reads the latest audit rows (the orchestrator's run list when there is no database); it is a POST for historical reasons, not for a change";
const EX_TEST_BUTTON: &str = "Decision 1: a connection, email or notification \"test\" button; it sends a probe and changes no configuration";
/// `POST /api/alerts/run` (`GOV-1` acceptance follow-up, Decision 1). This is
/// not [`EX_MACHINE`]'s case: the route is closed to persons
/// (`tests/route_auth.rs`), and the caller is the alert scheduler on its own
/// tick, so no person's change is behind the run. The rule and its schedule
/// were audited where they were set up, and what the rule delivered is in
/// the alert's own delivery history, not in the trail.
const EX_ALERT_SCHEDULER: &str = "Decision 1: the alert scheduler runs the rule on its own tick, and no person is behind the run; the rule and its schedule were audited where they were set up, and what the rule delivered is in the alert's own history";
const EX_MACHINE: &str = "Decision 1: machine callback from an ingestion or orchestrator job; the console user's change it reports is audited where the job was started";
const EX_PART: &str =
    "Decision 1: one part of a multipart upload; the finished upload is audited by `upload.create`";
const EX_UI_STATE: &str = "Personal UI state: favourites, recent items, home layout, chat session create, rename and delete, and the read marks of mentions (`POST /api/dashboard/comments/mentions/read`) (Decision 1)";

/// Every route of [`crate::policy::POLICY_TABLE`] that changes something, in
/// the order of that table, with who writes its audit row. Keyed by the
/// method and the registered route pattern, as the policy table is.
#[rustfmt::skip]
pub const AUDIT_TABLE: &[(&str, &str, Coverage)] = &[
    ("POST", "/api/embed/data", Coverage::Excluded { reason: EX_PUBLIC }),
    ("POST", "/api/render/data", Coverage::Excluded { reason: EX_PUBLIC }),
    ("POST", "/api/embed/frame", Coverage::Excluded { reason: EX_PUBLIC }),
    ("POST", "/api/embed/values", Coverage::Excluded { reason: EX_PUBLIC }),
    ("POST", "/api/public/dashboard/{token}/unlock", Coverage::Handler { action: "dashboard.public_link_unlocked" }),
    ("POST", "/api/embed/maps/{id}/geojson", Coverage::Excluded { reason: EX_PUBLIC }),
    ("POST", "/api/auth/login", Coverage::Handler { action: "auth.login" }),
    ("POST", "/api/auth/logout", Coverage::Handler { action: "auth.logout" }),
    ("POST", "/api/auth/change-password", Coverage::Handler { action: "auth.password_change" }),
    ("DELETE", "/api/auth/sessions/{id}", Coverage::Handler { action: "auth.session.revoke" }),
    ("PUT", "/api/catalog/{id}/annotation", Coverage::Handler { action: "catalog.annotate" }),
    ("PUT", "/api/catalog/{id}/certification", Coverage::Handler { action: "catalog.certify" }),
    ("PUT", "/api/semantic/{asset}", Coverage::Handler { action: "semantic.confirm" }),
    ("POST", "/api/catalog/{id}/access-request", Coverage::Handler { action: "catalog.access_request" }),
    ("POST", "/api/catalog/access-requests/{id}/decide", Coverage::Handler { action: crate::routes::catalog::ACCESS_DECIDE_ACTION }),
    ("POST", "/api/catalog/{id}/grants", Coverage::Handler { action: "catalog.grant" }),
    ("DELETE", "/api/catalog/{id}/grants/{grant_id}", Coverage::Handler { action: "catalog.revoke" }),
    ("POST", "/api/catalog/{id}/restore", Coverage::Handler { action: "catalog.restore" }),
    ("POST", "/api/catalog/{id}/drop", Coverage::Handler { action: "catalog.drop" }),
    ("POST", "/api/lakehouse/tables/{ns}/{table}/maintenance", Coverage::Central { action: "table.maintenance_set", resource_kind: "iceberg_table", id_param: IdParam::Path("table") }),
    ("POST", "/api/overview", Coverage::Excluded { reason: EX_OVERVIEW }),
    ("POST", "/api/overview/alerts/{id}/acknowledge", Coverage::Central { action: "overview_alert.acknowledge", resource_kind: "overview_alert", id_param: IdParam::Path("id") }),
    ("POST", "/api/overview/alerts/{id}/resolve", Coverage::Central { action: "overview_alert.resolve", resource_kind: "overview_alert", id_param: IdParam::Path("id") }),
    ("POST", "/api/overview/alerts/{id}/silence", Coverage::Central { action: "overview_alert.silence", resource_kind: "overview_alert", id_param: IdParam::Path("id") }),
    ("POST", "/api/ops/workloads/{id}/cancel", Coverage::Central { action: "workload.cancel", resource_kind: "workload", id_param: IdParam::Path("id") }),
    ("POST", "/api/governance/policies", Coverage::Handler { action: "policy.create" }),
    ("POST", "/api/governance/policies/preview", Coverage::Excluded { reason: EX_READ_LIKE }),
    ("PUT", "/api/governance/policies/{id}/status", Coverage::Handler { action: "policy.enforce" }),
    ("DELETE", "/api/governance/policies/{id}", Coverage::Handler { action: "policy.delete" }),
    ("PUT", "/api/governance/sla", Coverage::Handler { action: "catalog.sla_set" }),
    ("DELETE", "/api/governance/sla/{table}", Coverage::Handler { action: "catalog.sla_remove" }),
    ("DELETE", "/api/governance/classification/{id}", Coverage::Handler { action: "catalog.declassify" }),
    ("POST", "/api/governance/tag-keys", Coverage::Handler { action: "governance.tag_key_create" }),
    ("PUT", "/api/governance/tag-keys/{key}", Coverage::Handler { action: "governance.tag_key_update" }),
    ("DELETE", "/api/governance/tag-keys/{key}", Coverage::Handler { action: "governance.tag_key_delete" }),
    ("POST", "/api/governance/quality/incidents/{id}/resolve", Coverage::Handler { action: "quality.incident_resolve" }),
    ("PUT", "/api/governance/quality/schedules/{asset}", Coverage::Handler { action: "quality.schedule_update" }),
    ("DELETE", "/api/governance/quality/schedules/{asset}", Coverage::Handler { action: "quality.schedule_reset" }),
    ("POST", "/api/governance/quality/{id}/run", Coverage::Central { action: "quality.rule_run", resource_kind: "quality_rule", id_param: IdParam::Path("id") }),
    ("PUT", "/api/governance/quality/{id}", Coverage::Handler { action: "quality.rule_update" }),
    ("DELETE", "/api/governance/quality/{id}", Coverage::Handler { action: "quality.rule_delete" }),
    ("POST", "/api/governance/{kind}", Coverage::Handler { action: "quality.rule_create" }),
    ("POST", "/api/storage/policies", Coverage::Central { action: "storage.policy_create", resource_kind: "storage_policy", id_param: IdParam::None }),
    ("POST", "/api/storage/restore", Coverage::Central { action: "storage.restore", resource_kind: "storage", id_param: IdParam::None }),
    ("POST", "/api/alerts", Coverage::Central { action: "alert.create", resource_kind: "alert", id_param: IdParam::None }),
    ("PUT", "/api/alerts", Coverage::Central { action: "alert.update", resource_kind: "alert", id_param: IdParam::None }),
    ("DELETE", "/api/alerts", Coverage::Central { action: "alert.delete", resource_kind: "alert", id_param: IdParam::Query("id") }),
    // `GOV-1` acceptance follow-up (Decision 1): the alert test is a probe
    // button, and the scheduler's run has no person behind it — see
    // `EX_TEST_BUTTON` and `EX_ALERT_SCHEDULER`. The rule itself and its
    // schedule were audited where they were set up.
    ("POST", "/api/alerts/test", Coverage::Excluded { reason: EX_TEST_BUTTON }),
    ("POST", "/api/alerts/run", Coverage::Excluded { reason: EX_ALERT_SCHEDULER }),
    ("POST", "/api/gold/export/{mart}", Coverage::Central { action: "gold.export", resource_kind: "gold_mart", id_param: IdParam::Path("mart") }),
    ("PUT", "/api/gold/export/{mart}/publication", Coverage::Handler { action: "gold.publication_set" }),
    ("POST", "/api/query/run", Coverage::Handler { action: "query.run" }),
    ("POST", "/api/query/estimate", Coverage::Excluded { reason: EX_READ_LIKE }),
    ("POST", "/api/query/saved", Coverage::Central { action: "saved_query.create", resource_kind: "saved_query", id_param: IdParam::None }),
    ("PATCH", "/api/query/saved/{id}", Coverage::Central { action: "saved_query.update", resource_kind: "saved_query", id_param: IdParam::Path("id") }),
    ("DELETE", "/api/query/saved/{id}", Coverage::Central { action: "saved_query.delete", resource_kind: "saved_query", id_param: IdParam::Path("id") }),
    ("POST", "/api/query/saved/{id}/duplicate", Coverage::Central { action: "saved_query.duplicate", resource_kind: "saved_query", id_param: IdParam::Path("id") }),
    ("POST", "/api/query/saved/{id}/restore", Coverage::Central { action: "saved_query.restore", resource_kind: "saved_query", id_param: IdParam::Path("id") }),
    ("POST", "/api/query/saved/{id}/versions/{version}/revert", Coverage::Central { action: "saved_query.revert", resource_kind: "saved_query", id_param: IdParam::Path("id") }),
    ("POST", "/api/query/folders", Coverage::Central { action: "query_folder.create", resource_kind: "query_folder", id_param: IdParam::None }),
    ("PATCH", "/api/query/folders/{id}", Coverage::Central { action: "query_folder.update", resource_kind: "query_folder", id_param: IdParam::Path("id") }),
    ("DELETE", "/api/query/folders/{id}", Coverage::Central { action: "query_folder.delete", resource_kind: "query_folder", id_param: IdParam::Path("id") }),
    ("PUT", "/api/query/saved/{id}/access", Coverage::Central { action: "saved_query.access_update", resource_kind: "saved_query", id_param: IdParam::Path("id") }),
    ("PUT", "/api/query/folders/{id}/access", Coverage::Central { action: "query_folder.access_update", resource_kind: "query_folder", id_param: IdParam::Path("id") }),
    ("POST", "/api/pipelines", Coverage::Handler { action: "pipeline.create" }),
    ("POST", "/api/pipelines/generate", Coverage::Handler { action: "pipeline.generate" }),
    ("POST", "/api/pipelines/preview", Coverage::Excluded { reason: EX_READ_LIKE }),
    ("PUT", "/api/pipelines/{id}", Coverage::Handler { action: "pipeline.update" }),
    ("PUT", "/api/pipelines/{id}/target-lag", Coverage::Handler { action: "pipeline.target_lag.set" }),
    ("DELETE", "/api/pipelines/{id}", Coverage::Handler { action: "pipeline.delete" }),
    ("POST", "/api/pipelines/runnable/{id}/lag-check", Coverage::Excluded { reason: EX_LAG_CHECK }),
    ("POST", "/api/pipelines/runnable/{id}/compile", Coverage::Excluded { reason: EX_READ_LIKE }),
    ("POST", "/api/pipelines/{id}/trigger", Coverage::Handler { action: "pipeline.trigger" }),
    ("POST", "/api/pipelines/{id}/status", Coverage::Central { action: "pipeline.status_set", resource_kind: "pipeline", id_param: IdParam::Path("id") }),
    ("POST", "/api/pipelines/{id}/pause", Coverage::Handler { action: "pipeline.pause" }),
    ("POST", "/api/pipelines/{id}/resume", Coverage::Handler { action: "pipeline.resume" }),
    ("POST", "/api/pipelines/runs/{runId}/cancel", Coverage::Central { action: "pipeline.run_cancel", resource_kind: "pipeline_run", id_param: IdParam::Path("runId") }),
    ("POST", "/api/pipelines/runs/{runId}/retry", Coverage::Central { action: "pipeline.run_retry", resource_kind: "pipeline_run", id_param: IdParam::Path("runId") }),
    ("PUT", "/api/pipelines/{id}/tenant", Coverage::Central { action: "pipeline.tenant_assign", resource_kind: "pipeline", id_param: IdParam::Path("id") }),
    ("PUT", "/api/pipelines/{id}/run-as", Coverage::Handler { action: "pipeline.run_as.assign" }),
    ("POST", "/api/pipelines/events/run-failed", Coverage::Excluded { reason: EX_MACHINE }),
    ("POST", "/api/pipelines/events/run-finished", Coverage::Excluded { reason: EX_MACHINE }),
    ("PUT", "/api/pipelines/{id}/sla", Coverage::Handler { action: "pipeline.sla_set" }),
    ("POST", "/api/pipelines/{id}/versions/{version}/restore", Coverage::Handler { action: "pipeline.restore.version" }),
    ("POST", "/api/dashboard/specs/preview", Coverage::Excluded { reason: EX_READ_LIKE }),
    ("POST", "/api/dashboard/specs", Coverage::Central { action: "dashboard_tile.create", resource_kind: "dashboard_tile", id_param: IdParam::None }),
    ("PUT", "/api/dashboard/specs", Coverage::Central { action: "dashboard_tile.update", resource_kind: "dashboard_tile", id_param: IdParam::None }),
    ("DELETE", "/api/dashboard/specs", Coverage::Central { action: "dashboard_tile.delete", resource_kind: "dashboard_tile", id_param: IdParam::Query("id") }),
    ("POST", "/api/dashboard/boards", Coverage::Central { action: "dashboard.create", resource_kind: "dashboard", id_param: IdParam::None }),
    ("PUT", "/api/dashboard/boards", Coverage::Central { action: "dashboard.update", resource_kind: "dashboard", id_param: IdParam::None }),
    ("DELETE", "/api/dashboard/boards", Coverage::Central { action: "dashboard.delete", resource_kind: "dashboard", id_param: IdParam::Query("id") }),
    ("POST", "/api/dashboard/folders", Coverage::Central { action: "dashboard_folder.create", resource_kind: "dashboard_folder", id_param: IdParam::None }),
    ("PUT", "/api/dashboard/folders", Coverage::Central { action: "dashboard_folder.update", resource_kind: "dashboard_folder", id_param: IdParam::None }),
    ("DELETE", "/api/dashboard/folders", Coverage::Central { action: "dashboard_folder.delete", resource_kind: "dashboard_folder", id_param: IdParam::Query("id") }),
    ("POST", "/api/dashboard/personal", Coverage::Central { action: "dashboard_folder.personal_create", resource_kind: "dashboard_folder", id_param: IdParam::None }),
    ("POST", "/api/dashboard/versions/revert", Coverage::Central { action: "dashboard.version_revert", resource_kind: "dashboard", id_param: IdParam::None }),
    ("POST", "/api/dashboard/trash/restore", Coverage::Central { action: "dashboard.restore", resource_kind: "dashboard", id_param: IdParam::None }),
    ("PUT", "/api/dashboard/favourites", Coverage::Excluded { reason: EX_UI_STATE }),
    ("POST", "/api/dashboard/recent", Coverage::Excluded { reason: EX_UI_STATE }),
    ("POST", "/api/dashboard/sources", Coverage::Central { action: "dashboard_source.create", resource_kind: "dashboard_source", id_param: IdParam::None }),
    ("PUT", "/api/dashboard/sources", Coverage::Central { action: "dashboard_source.update", resource_kind: "dashboard_source", id_param: IdParam::None }),
    ("DELETE", "/api/dashboard/sources", Coverage::Central { action: "dashboard_source.delete", resource_kind: "dashboard_source", id_param: IdParam::Query("id") }),
    ("POST", "/api/dashboard/sources/preview", Coverage::Excluded { reason: EX_READ_LIKE }),
    ("POST", "/api/dashboard/calc-fields", Coverage::Central { action: "calculated_field.create", resource_kind: "calculated_field", id_param: IdParam::None }),
    ("PUT", "/api/dashboard/calc-fields", Coverage::Central { action: "calculated_field.update", resource_kind: "calculated_field", id_param: IdParam::None }),
    ("DELETE", "/api/dashboard/calc-fields", Coverage::Central { action: "calculated_field.delete", resource_kind: "calculated_field", id_param: IdParam::Query("id") }),
    ("POST", "/api/dashboard/calc-fields/validate", Coverage::Excluded { reason: EX_READ_LIKE }),
    ("POST", "/api/dashboard/metrics", Coverage::Central { action: "metric.create", resource_kind: "metric", id_param: IdParam::None }),
    ("PUT", "/api/dashboard/metrics", Coverage::Central { action: "metric.update", resource_kind: "metric", id_param: IdParam::None }),
    ("DELETE", "/api/dashboard/metrics", Coverage::Central { action: "metric.delete", resource_kind: "metric", id_param: IdParam::Query("id") }),
    ("POST", "/api/dashboard/metrics/validate", Coverage::Excluded { reason: EX_READ_LIKE }),
    ("POST", "/api/dashboard/segments", Coverage::Central { action: "segment.create", resource_kind: "segment", id_param: IdParam::None }),
    ("PUT", "/api/dashboard/segments", Coverage::Central { action: "segment.update", resource_kind: "segment", id_param: IdParam::None }),
    ("DELETE", "/api/dashboard/segments", Coverage::Central { action: "segment.delete", resource_kind: "segment", id_param: IdParam::Query("id") }),
    ("POST", "/api/dashboard/hierarchies", Coverage::Central { action: "hierarchy.create", resource_kind: "hierarchy", id_param: IdParam::None }),
    ("PUT", "/api/dashboard/hierarchies", Coverage::Central { action: "hierarchy.update", resource_kind: "hierarchy", id_param: IdParam::None }),
    ("DELETE", "/api/dashboard/hierarchies", Coverage::Central { action: "hierarchy.delete", resource_kind: "hierarchy", id_param: IdParam::Query("id") }),
    ("POST", "/api/dashboard/params", Coverage::Central { action: "dashboard_param.create", resource_kind: "dashboard_param", id_param: IdParam::None }),
    ("PUT", "/api/dashboard/params", Coverage::Central { action: "dashboard_param.update", resource_kind: "dashboard_param", id_param: IdParam::None }),
    ("DELETE", "/api/dashboard/params", Coverage::Central { action: "dashboard_param.delete", resource_kind: "dashboard_param", id_param: IdParam::Query("id") }),
    ("POST", "/api/dashboard/import/preview", Coverage::Excluded { reason: EX_READ_LIKE }),
    ("POST", "/api/dashboard/import", Coverage::Central { action: "dashboard.import", resource_kind: "dashboard", id_param: IdParam::None }),
    ("POST", "/api/reports/subscriptions", Coverage::Central { action: "report_subscription.create", resource_kind: "report_subscription", id_param: IdParam::None }),
    ("PUT", "/api/reports/subscriptions/{id}", Coverage::Central { action: "report_subscription.update", resource_kind: "report_subscription", id_param: IdParam::Path("id") }),
    ("DELETE", "/api/reports/subscriptions/{id}", Coverage::Central { action: "report_subscription.delete", resource_kind: "report_subscription", id_param: IdParam::Path("id") }),
    ("POST", "/api/reports/subscriptions/{id}/send", Coverage::Central { action: "report_subscription.send", resource_kind: "report_subscription", id_param: IdParam::Path("id") }),
    ("POST", "/api/dashboard/embed-revoke", Coverage::Central { action: "dashboard.embed_revoke", resource_kind: "dashboard", id_param: IdParam::None }),
    ("PUT", "/api/dashboard/embed-settings", Coverage::Handler { action: "dashboard.embed_settings" }),
    ("PUT", "/api/dashboard/access", Coverage::Handler { action: "dashboard.access_change" }),
    ("PUT", "/api/dashboard/public-link", Coverage::Handler { action: "dashboard.public_link_create" }),
    ("DELETE", "/api/dashboard/public-link", Coverage::Handler { action: "dashboard.public_link_delete" }),
    ("POST", "/api/dashboard/comments", Coverage::Central { action: "dashboard_comment.create", resource_kind: "dashboard_comment", id_param: IdParam::None }),
    ("POST", "/api/dashboard/comments/mentions/read", Coverage::Excluded { reason: EX_UI_STATE }),
    ("PUT", "/api/dashboard/comments/{id}", Coverage::Central { action: "dashboard_comment.update", resource_kind: "dashboard_comment", id_param: IdParam::Path("id") }),
    ("DELETE", "/api/dashboard/comments/{id}", Coverage::Central { action: "dashboard_comment.delete", resource_kind: "dashboard_comment", id_param: IdParam::Path("id") }),
    ("PUT", "/api/settings/reporting", Coverage::Central { action: "settings.reporting.update", resource_kind: "settings", id_param: IdParam::None }),
    ("POST", "/api/usage/rates", Coverage::Central { action: "usage.rate_create", resource_kind: "settings", id_param: IdParam::None }),
    ("PUT", "/api/settings/email", Coverage::Handler { action: "settings.email.update" }),
    ("POST", "/api/settings/email/test", Coverage::Excluded { reason: EX_TEST_BUTTON }),
    ("PUT", "/api/settings/sign-in", Coverage::Handler { action: "settings.sign_in.update" }),
    ("PUT", "/api/settings/general", Coverage::Handler { action: "settings.general.update" }),
    ("DELETE", "/api/settings/general", Coverage::Handler { action: "settings.general.reset" }),
    ("PUT", "/api/settings/general/images/{kind}", Coverage::Handler { action: "settings.general.image_upload" }),
    ("DELETE", "/api/settings/general/images/{kind}", Coverage::Handler { action: "settings.general.image_remove" }),
    ("PUT", "/api/settings/assistant", Coverage::Handler { action: "settings.assistant.update" }),
    ("POST", "/api/settings/assistant/test", Coverage::Excluded { reason: EX_TEST_BUTTON }),
    ("PUT", "/api/settings/sso", Coverage::Handler { action: "settings.sso.update" }),
    ("POST", "/api/settings/sso/test", Coverage::Excluded { reason: EX_TEST_BUTTON }),
    ("POST", "/api/query/cancel", Coverage::Central { action: "query.cancel", resource_kind: "query_run", id_param: IdParam::None }),
    ("PUT", "/api/settings/limits", Coverage::Central { action: "settings.limits.update", resource_kind: "settings", id_param: IdParam::None }),
    ("POST", "/api/ai/chat", Coverage::Handler { action: crate::routes::ASSISTANT_CHAT_ACTION }),
    ("POST", "/api/ai/tool", Coverage::Handler { action: crate::routes::ASSISTANT_TOOL_ACTION }),
    ("POST", "/api/ai/sessions", Coverage::Excluded { reason: EX_UI_STATE }),
    ("PATCH", "/api/ai/sessions", Coverage::Excluded { reason: EX_UI_STATE }),
    ("DELETE", "/api/ai/sessions", Coverage::Excluded { reason: EX_UI_STATE }),
    ("PUT", "/api/ai/terms", Coverage::Central { action: "ai_term.set", resource_kind: "ai_term", id_param: IdParam::None }),
    ("DELETE", "/api/ai/terms", Coverage::Central { action: "ai_term.delete", resource_kind: "ai_term", id_param: IdParam::Query("term") }),
    ("PUT", "/api/home/layout", Coverage::Excluded { reason: EX_UI_STATE }),
    ("DELETE", "/api/home/layout", Coverage::Excluded { reason: EX_UI_STATE }),
    ("POST", "/api/identity/users", Coverage::Handler { action: "identity.user.invite" }),
    ("PATCH", "/api/identity/users/{id}", Coverage::Handler { action: "identity.user.update" }),
    ("PUT", "/api/identity/users/{id}/tenants/{tenant_id}", Coverage::Handler { action: "identity.user.tenant.add" }),
    ("DELETE", "/api/identity/users/{id}/tenants/{tenant_id}", Coverage::Handler { action: "identity.user.tenant.remove" }),
    ("PUT", "/api/identity/users/{id}/roles", Coverage::Handler { action: "identity.user.roles.set" }),
    ("POST", "/api/identity/users/{id}/deactivate", Coverage::Handler { action: "identity.user.deactivate" }),
    ("POST", "/api/identity/users/{id}/reactivate", Coverage::Handler { action: "identity.user.reactivate" }),
    ("POST", "/api/identity/users/{id}/reset-password", Coverage::Handler { action: "identity.user.password_reset" }),
    ("POST", "/api/identity/roles", Coverage::Central { action: "identity.role.create", resource_kind: "role", id_param: IdParam::None }),
    ("PUT", "/api/identity/roles/{id}", Coverage::Handler { action: "identity.role.update" }),
    ("DELETE", "/api/identity/roles/{id}", Coverage::Handler { action: "identity.role.delete" }),
    ("PUT", "/api/identity/roles/{id}/table-access", Coverage::Handler { action: "identity.role.table_access.update" }),
    ("POST", "/api/identity/tenants", Coverage::Handler { action: "identity.tenant.create" }),
    ("POST", "/api/identity/service-identities", Coverage::Central { action: "identity.service_identity.create", resource_kind: "service_identity", id_param: IdParam::None }),
    ("POST", "/api/identity/service-identities/{id}/rotate", Coverage::Handler { action: "identity.service_identity.rotate" }),
    ("POST", "/api/connectors", Coverage::Handler { action: "connector.create" }),
    ("DELETE", "/api/connectors/{id}", Coverage::Handler { action: "connector.delete" }),
    ("PATCH", "/api/connectors/{id}", Coverage::Handler { action: "connector.update" }),
    ("POST", "/api/connectors/{id}/test", Coverage::Handler { action: "connector.test" }),
    ("PUT", "/api/connectors/{id}/secret", Coverage::Handler { action: "connector.secret_rotate" }),
    ("PUT", "/api/connectors/{id}/credential", Coverage::Handler { action: "connector.credential_set" }),
    ("POST", "/api/connectors/{id}/discovery/{kind}", Coverage::Central { action: "connector.discover", resource_kind: "connector", id_param: IdParam::Path("id") }),
    ("PUT", "/api/connectors/{id}/ingest-spec", Coverage::Handler { action: "connector.update" }),
    ("POST", "/api/connectors/{id}/ingest/run", Coverage::Handler { action: "connector.ingest.run" }),
    ("POST", "/api/connectors/{id}/schema-observations", Coverage::Excluded { reason: EX_MACHINE }),
    ("POST", "/api/connectors/{id}/schema-observations/tables", Coverage::Excluded { reason: EX_MACHINE }),
    ("POST", "/api/connectors/{id}/resume", Coverage::Handler { action: "connector.resume" }),
    ("POST", "/api/connectors/{id}/pause", Coverage::Handler { action: "connector.pause" }),
    ("PUT", "/api/connectors/{id}/lag-limit", Coverage::Handler { action: "connector.lag_limit" }),
    ("POST", "/api/connectors/{id}/capture/observations", Coverage::Excluded { reason: EX_MACHINE }),
    ("POST", "/api/connectors/{id}/run-reports", Coverage::Excluded { reason: EX_MACHINE }),
    ("POST", "/api/connectors/{id}/run-reports/{runId}/attempts", Coverage::Excluded { reason: EX_MACHINE }),
    ("POST", "/api/connectors/{id}/run-reports/{runId}/log", Coverage::Excluded { reason: EX_MACHINE }),
    ("POST", "/api/connectors/{id}/run-reports/{runId}/end", Coverage::Excluded { reason: EX_MACHINE }),
    ("POST", "/api/connectors/{id}/discovery-results", Coverage::Handler { action: "connector.test" }),
    ("POST", "/api/connectors/{id}/oauth/start", Coverage::Handler { action: "connector.oauth_start" }),
    ("POST", "/api/connectors/{id}/oauth/sign-out", Coverage::Handler { action: "connector.oauth_signed_out" }),
    ("POST", "/api/connectors/{id}/oauth/refresh-token", Coverage::Handler { action: "connector.oauth_refresh_token_rotated" }),
    ("POST", "/api/connectors/{id}/schema-changes/approve", Coverage::Handler { action: "connector.schema_change.approve" }),
    ("PUT", "/api/connectors/{id}/tenant", Coverage::Central { action: "connector.tenant_assign", resource_kind: "connector", id_param: IdParam::Path("id") }),
    ("POST", "/api/uploads", Coverage::Handler { action: "upload.create" }),
    ("DELETE", "/api/uploads/{id}", Coverage::Handler { action: "upload.delete" }),
    ("POST", "/api/uploads/{id}/ingest", Coverage::Handler { action: "upload.ingest" }),
    ("POST", "/api/uploads/{id}/check", Coverage::Excluded { reason: EX_READ_LIKE }),
    ("POST", "/api/uploads/{id}/compare", Coverage::Excluded { reason: EX_READ_LIKE }),
    ("POST", "/api/uploads/sessions", Coverage::Central { action: "upload.session_start", resource_kind: "upload", id_param: IdParam::None }),
    ("PUT", "/api/uploads/{id}/parts/{n}", Coverage::Excluded { reason: EX_PART }),
    ("POST", "/api/uploads/{id}/complete", Coverage::Handler { action: "upload.create" }),
    ("POST", "/api/uploads/folders", Coverage::Handler { action: "upload.folder.create" }),
    ("PATCH", "/api/uploads/folders/{id}", Coverage::Handler { action: "upload.folder.rename" }),
    ("DELETE", "/api/uploads/folders/{id}", Coverage::Handler { action: "upload.folder.delete" }),
    ("POST", "/api/uploads/move", Coverage::Handler { action: "upload.move" }),
    ("POST", "/api/maps", Coverage::Handler { action: "map_upload" }),
    ("PATCH", "/api/maps/{id}", Coverage::Handler { action: "map_rename" }),
    ("DELETE", "/api/maps/{id}", Coverage::Handler { action: "map_delete" }),
    ("POST", "/api/knowledge/sources", Coverage::Central { action: "knowledge_source.create", resource_kind: "knowledge_source", id_param: IdParam::None }),
    ("POST", "/api/knowledge/vector-jobs", Coverage::Central { action: "knowledge_vector_job.create", resource_kind: "knowledge_vector_job", id_param: IdParam::None }),
    ("POST", "/api/agents/workflows", Coverage::Central { action: "agent_workflow.create", resource_kind: "agent_workflow", id_param: IdParam::None }),
    ("POST", "/api/agents/employees", Coverage::Central { action: "digital_employee.create", resource_kind: "digital_employee", id_param: IdParam::None }),
    ("POST", "/api/agents/employees/{id}/suspend", Coverage::Central { action: "digital_employee.suspend", resource_kind: "digital_employee", id_param: IdParam::Path("id") }),
    ("POST", "/api/agents/employees/{id}/resume", Coverage::Central { action: "digital_employee.resume", resource_kind: "digital_employee", id_param: IdParam::Path("id") }),
    ("POST", "/api/agents/employees/{id}/revoke", Coverage::Central { action: "digital_employee.revoke", resource_kind: "digital_employee", id_param: IdParam::Path("id") }),
    ("POST", "/api/agents/employees/{id}/run", Coverage::Central { action: "digital_employee.run", resource_kind: "digital_employee", id_param: IdParam::Path("id") }),
    ("POST", "/api/agents/approvals/{id}/decide", Coverage::Handler { action: crate::routes::agents::APPROVAL_DECIDE_ACTION }),
];

/// The coverage of `(method, pattern)`, `None` for a route not in
/// [`AUDIT_TABLE`] (a read, or a route nobody listed: the guard test fails
/// on the second).
#[must_use]
pub fn coverage_for(method: &str, pattern: &str) -> Option<Coverage> {
    AUDIT_TABLE
        .iter()
        .find(|(m, p, _)| *m == method && *p == pattern)
        .map(|(_, _, coverage)| *coverage)
}

/// The class of a store error, for a log line: the variant, never its text.
pub(crate) fn store_error_class(err: &StoreError) -> &'static str {
    match err {
        StoreError::Conflict | StoreError::FieldConflict(_) => "conflict",
        StoreError::ForeignKeyViolation => "foreign_key",
        StoreError::Unavailable => "unavailable",
        StoreError::Database(_) => "database",
        _ => "other",
    }
}

/// Writes one audit row and never fails the caller: on a failed insert it
/// logs the action and the error's class (never `args`, never the error's
/// own text) and counts the failure in
/// [`AppState::audit_write_failure_count`] (`GOV-1`, feature page Decision 4).
///
/// A deployment with no Postgres pool counts as a failed write too: the
/// change happened and its row did not.
pub async fn record_best_effort(state: &AppState, event: NewAuditEvent) {
    let action = event.action.clone();
    let class = match state.pg.as_deref() {
        Some(pool) => match lakehouse_store::audit::insert(pool, event).await {
            Ok(_) => return,
            Err(err) => store_error_class(&err),
        },
        None => "unavailable",
    };
    state.count_audit_write_failure();
    tracing::error!(
        action = %action,
        error_class = class,
        "an audit row could not be written; the change it records stands"
    );
}

/// The path parameters of a request as `(name, value)` pairs, empty when the
/// route has none. Called after routing, so the router has set them.
pub(crate) async fn path_params(parts: &mut Parts) -> Vec<(String, String)> {
    match RawPathParams::from_request_parts(parts, &()).await {
        Ok(params) => params
            .iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// `text` with every NUL character replaced by U+FFFD (`GOV-1 review`,
/// blocker 1). Postgres refuses a NUL in a `TEXT` column and in `jsonb`, so a
/// client string that reaches an audit row goes through this first, or the
/// insert fails and the change stands with no row. The replacement keeps the
/// character count, so the bounds of this module still hold.
pub(crate) fn without_nul(text: &str) -> String {
    text.replace('\0', "\u{fffd}")
}

/// The object id a request names, cut to [`MAX_RESOURCE_ID_CHARS`]: the path
/// parameter, or the first query-string parameter of that name,
/// percent-decoded as the handler's own `Query` extractor decodes it, with a
/// NUL replaced ([`without_nul`]). `None` when the route has no id there, or
/// the request did not send one.
fn object_id(
    id_param: IdParam,
    params: &[(String, String)],
    query: Option<&str>,
) -> Option<String> {
    let value = match id_param {
        IdParam::None => return None,
        IdParam::Path(name) => params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())?,
        IdParam::Query(name) => url::form_urlencoded::parse(query?.as_bytes())
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())?,
    };
    Some(
        without_nul(&value)
            .chars()
            .take(MAX_RESOURCE_ID_CHARS)
            .collect(),
    )
}

/// The audit columns that name who did it: the id, the kind and the display
/// name of the principal, all text the server holds (`GOV-1 review`, so the
/// handler rows of `routes::auth` fill them the same way).
pub(crate) fn actor(principal: Option<&Principal>) -> (Option<String>, Option<String>, String) {
    match principal {
        Some(p) => (
            Some(p.id.uuid().to_string()),
            Some(p.kind_for_audit().to_owned()),
            p.display_name.clone(),
        ),
        None => (None, None, ACTOR_NOT_SIGNED_IN.to_owned()),
    }
}

/// Writes the `refused` row of a route `auth_gate` turned away with `403`
/// (see the module doc comment, "Refused at the gate"). Writes nothing for a
/// route that is not in [`AUDIT_TABLE`] or is `Excluded`.
///
/// `params` are the request's path parameters and `query` its raw query
/// string; the id of a `Central` route is read from them. `detail` is the missing permission or the fixed
/// password-rotation sentence, text the gate wrote itself.
pub async fn record_gate_refusal(
    state: &AppState,
    principal: &Principal,
    method: &str,
    pattern: &str,
    params: &[(String, String)],
    query: Option<&str>,
    detail: &str,
) {
    let (action, resource_kind, resource_id) = match coverage_for(method, pattern) {
        Some(Coverage::Handler { action }) => (action, None, None),
        Some(Coverage::Central {
            action,
            resource_kind,
            id_param,
        }) => (
            action,
            Some(resource_kind.to_owned()),
            object_id(id_param, params, query),
        ),
        Some(Coverage::Excluded { .. }) | None => return,
    };
    let (principal_id, principal_kind, actor_label) = actor(Some(principal));
    record_best_effort(
        state,
        NewAuditEvent {
            principal_id,
            principal_kind,
            actor_label: Some(actor_label),
            action: action.to_owned(),
            resource_kind,
            resource_id,
            args: Some(json!({})),
            outcome: "refused".to_owned(),
            detail: Some(detail.to_owned()),
            ..Default::default()
        },
    )
    .await;
}

/// The audit outcome of a response status.
fn outcome_for(status: u16) -> &'static str {
    match status {
        200..=299 => "executed",
        500..=599 => "failed",
        _ => "refused",
    }
}

/// Whether the content type is JSON (`application/json`, or a `+json` type).
fn is_json(req: &Request) -> bool {
    req.headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .is_some_and(|essence| essence == "application/json" || essence.ends_with("+json"))
}

/// The declared body length, when the request states one.
fn declared_length(req: &Request) -> Option<usize> {
    req.headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
}

/// The sorted top-level key names of a JSON object, `None` for anything else
/// (not an object, not JSON). Values are skipped without being kept.
#[allow(
    clippy::zero_sized_map_values,
    reason = "`IgnoredAny` makes serde skip every value without building it, so a body's values \
              are never held; a map is the one serde shape that yields the keys alone, and a \
              `BTreeSet` cannot be deserialized from an object"
)]
fn field_names(bytes: &[u8]) -> Option<Vec<String>> {
    let object: BTreeMap<String, IgnoredAny> = serde_json::from_slice(bytes).ok()?;
    Some(object.into_keys().collect())
}

/// `args` for a row: the field names, cut to the bounds of the module doc
/// comment, or `{"fields": null}` when the body was not read.
fn args_for(fields: Option<Vec<String>>) -> Value {
    let Some(names) = fields else {
        return json!({ "fields": null });
    };
    let truncated = names.len() > MAX_FIELD_NAMES;
    let names: Vec<String> = names
        .into_iter()
        .take(MAX_FIELD_NAMES)
        .map(|name| {
            without_nul(&name)
                .chars()
                .take(MAX_FIELD_NAME_CHARS)
                .collect()
        })
        .collect();
    if truncated {
        json!({ "fields": names, "fieldsTruncated": true })
    } else {
        json!({ "fields": names })
    }
}

/// What [`peek_fields`] found: the field names (`None` when the body was not
/// read) and the request to hand to the handler.
type Peeked = Result<(Option<Vec<String>>, Request), ApiError>;

/// Reads the field names of a small JSON body and rebuilds the request with
/// the same bytes. Any other body is handed on untouched.
async fn peek_fields(req: Request) -> Peeked {
    let readable = is_json(&req) && declared_length(&req).is_some_and(|n| n <= MAX_BODY_PEEK_BYTES);
    if !readable {
        return Ok((None, req));
    }
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, MAX_BODY_PEEK_BYTES)
        .await
        .map_err(|_| ApiError::BadRequest("The request body could not be read.".to_owned()))?;
    let fields = field_names(&bytes);
    Ok((fields, Request::from_parts(parts, Body::from(bytes))))
}

/// The central audit writer (see the module doc comment). Layered inside
/// `auth_gate` and outside the query scope in `routes::gated`.
pub async fn audit_gate(
    State(state): State<AppState>,
    matched_path: MatchedPath,
    req: Request,
    next: Next,
) -> Response {
    let Some(Coverage::Central {
        action,
        resource_kind,
        id_param,
    }) = coverage_for(req.method().as_str(), matched_path.as_str())
    else {
        return next.run(req).await;
    };
    let (mut parts, body) = req.into_parts();
    let principal = parts.extensions.get::<Principal>().cloned();
    let params = path_params(&mut parts).await;
    let resource_id = object_id(id_param, &params, parts.uri.query());
    let req = Request::from_parts(parts, body);

    let (fields, response) = match peek_fields(req).await {
        Ok((fields, req)) => (fields, next.run(req).await),
        Err(err) => (None, ApiRejection(err).into_response()),
    };
    let status = response.status();
    let (principal_id, principal_kind, actor_label) = actor(principal.as_ref());
    record_best_effort(
        &state,
        NewAuditEvent {
            principal_id,
            principal_kind,
            actor_label: Some(actor_label),
            action: action.to_owned(),
            resource_kind: Some(resource_kind.to_owned()),
            resource_id,
            args: Some(args_for(fields)),
            outcome: outcome_for(status.as_u16()).to_owned(),
            detail: Some(status.as_u16().to_string()),
            ..Default::default()
        },
    )
    .await;
    response
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::{Arc, Mutex};

    use super::*;

    /// A log writer that keeps what it is given.
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn event(action: &str) -> NewAuditEvent {
        NewAuditEvent {
            action: action.to_owned(),
            args: Some(json!({ "fields": ["distinctive-field-name"] })),
            outcome: "executed".to_owned(),
            ..Default::default()
        }
    }

    /// `GOV-1` Decision 4: a write that fails is counted and logged with the
    /// action and the class of the error, never the row's `args`, and the
    /// caller is not told. A pool that is closed fails every insert at once;
    /// a deployment with no pool fails them the same way.
    #[tokio::test]
    async fn a_failed_audit_write_is_counted_and_logged_without_its_args() {
        let captured = Captured::default();
        let _logging = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(captured.clone())
                .with_ansi(false)
                .finish(),
        );

        let mut state = AppState::without_a_database();
        assert_eq!(state.audit_write_failure_count(), 0);

        record_best_effort(&state, event("saved_query.update")).await;
        assert_eq!(state.audit_write_failure_count(), 1, "no pool is a failure");

        let pool = lakehouse_store::connect_lazy("postgres://nobody@127.0.0.1:1/none").unwrap();
        pool.close().await;
        state.pg = Some(Arc::new(pool));
        record_best_effort(&state, event("saved_query.delete")).await;
        assert_eq!(
            state.audit_write_failure_count(),
            2,
            "a closed pool is a failure"
        );

        let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert!(log.contains("saved_query.update"), "{log}");
        assert!(log.contains("saved_query.delete"), "{log}");
        assert!(log.contains("error_class"), "{log}");
        assert!(
            !log.contains("distinctive-field-name"),
            "the log never carries args: {log}"
        );
    }

    #[test]
    fn the_outcome_follows_the_status_class() {
        assert_eq!(outcome_for(200), "executed");
        assert_eq!(outcome_for(201), "executed");
        assert_eq!(outcome_for(204), "executed");
        assert_eq!(outcome_for(400), "refused");
        assert_eq!(outcome_for(403), "refused");
        assert_eq!(outcome_for(404), "refused");
        assert_eq!(outcome_for(409), "refused");
        assert_eq!(outcome_for(422), "refused");
        assert_eq!(outcome_for(429), "refused");
        assert_eq!(outcome_for(302), "refused");
        assert_eq!(outcome_for(500), "failed");
        assert_eq!(outcome_for(503), "failed");
    }

    #[test]
    fn field_names_are_sorted_keys_and_a_value_is_never_read_into_them() {
        assert_eq!(
            field_names(br#"{"b": {"nested": 1}, "a": "secret-value", "c": [1, 2]}"#),
            Some(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()])
        );
        assert_eq!(field_names(b"[1, 2]"), None);
        assert_eq!(field_names(b"not json"), None);
        assert_eq!(field_names(b"\"text\""), None);
    }

    #[test]
    fn a_long_list_of_names_is_cut_and_says_so() {
        let names: Vec<String> = (0..MAX_FIELD_NAMES + 5)
            .map(|n| format!("f{n:04}"))
            .collect();
        let args = args_for(Some(names));
        assert_eq!(args["fields"].as_array().unwrap().len(), MAX_FIELD_NAMES);
        assert_eq!(args["fieldsTruncated"], json!(true));
        let long = args_for(Some(vec!["x".repeat(MAX_FIELD_NAME_CHARS + 50)]));
        assert_eq!(
            long["fields"][0].as_str().unwrap().chars().count(),
            MAX_FIELD_NAME_CHARS
        );
        assert!(long.get("fieldsTruncated").is_none());
    }
}
