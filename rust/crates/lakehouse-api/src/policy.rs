//! Task 3.2: the route authorization policy, expressed as data.
//!
//! # Why data, and why deny-by-default
//!
//! [`POLICY_TABLE`] is the single source of truth for which of the
//! service's routes are public, which merely require *some* authenticated
//! [`lakehouse_auth::Principal`], and which require a specific permission.
//! [`auth_gate`] — the one middleware layered onto the whole router in
//! `routes::router` — looks up every request's `(method, matched route
//! pattern)` in this table before the handler ever runs.
//!
//! Critically, a `(method, path)` pair that is NOT in [`POLICY_TABLE`] is
//! never treated as public. [`policy_for`] returns `None`, and
//! [`auth_gate`] turns that into a 500 (`route_policy_unclassified`) rather
//! than letting the request through — see `route_policy_completeness`
//! (`tests/route_policy.rs`) for the regression test that proves this: a
//! route registered in `routes::router` without a matching entry here
//! fails loudly (both the test and, in production, every real request to
//! it) instead of silently defaulting to open. This is a deliberately
//! stronger guarantee than a router literally *built from* this table
//! would need — it holds regardless of how a future route gets added to
//! `routes::router`.
//!
//! # Where each permission string comes from
//!
//! Every [`Policy::RequiresPermission`] value below is one of the real
//! seeded `resource:action` tokens documented on
//! `lakehouse_auth::permissions` (sourced from `0002_seed_identity.sql`) —
//! `catalog:read`, `lineage:read`, `policy:*`-shaped (`policy:read`/
//! `policy:write`, satisfied by Governance Admin's literal `policy:*`),
//! `query:read`, `pipeline:*`-shaped (`pipeline:read`/`pipeline:write`,
//! satisfied by Data Engineer's literal `pipeline:*`), `connector:manage`,
//! `dashboard:read` — plus `dashboard:write`, the natural write-side
//! counterpart to the seeded `dashboard:read` (no seeded role is granted
//! it; only Platform Admin's `*:*` satisfies it today, which is the
//! intended, deliberately narrow default for dashboard mutation). No
//! permission string here names a resource the seed data doesn't already
//! use. Routes with no such grounding require only *some* authenticated
//! principal ([`Policy::RequiresAuth`]) rather than a guessed permission —
//! see this task's final report for the explicit list of which routes that
//! applies to and why.
//!
//! `identity:read`/`identity:write` are the one pair of permission strings
//! in this table that are NOT copied from `0002_seed_identity.sql` — no
//! seeded role grants either token by name. They exist to close a real
//! privilege-escalation hole: before this change, every `/api/identity/*`
//! route (create users, mint roles with arbitrary permission strings
//! including `*:*`, attach roles to users, register service identities)
//! was merely `Policy::RequiresAuth`, so any authenticated principal —
//! including a low-privilege Analyst — could create a `*:*` role and
//! attach it to itself. Introducing `identity:read`/`identity:write`
//! requires no seed-data migration: Platform Admin's existing `"*:*"`
//! already satisfies both by the resource-wildcard rule in
//! `lakehouse_auth::permissions`, and no other seeded role's tokens
//! (`policy:*`, `residency:*`, `audit:read`, `pipeline:*`, `catalog:write`,
//! `connector:manage`, `query:read`, `catalog:read`, `lineage:read`,
//! `agent:approve`, `dashboard:read`) match `identity:*` under those
//! semantics — see `identity_permissions_require_no_seed_change` below.
//!
//! # `settings:write`
//!
//! Introduced for `PUT /api/settings/reporting` (`BI-9`) by the same rules as
//! `identity:*` and `storage:restore`: the string is listed in the table
//! below, no seeded role is granted it (so no migration touches the role
//! rows), and Platform Admin's `*:*` satisfies it by the resource-wildcard
//! rule. Reading the settings stays `RequiresAuth`.
//!
//! # `metric:write`
//!
//! Introduced for defining, changing and deleting reusable metrics and
//! segments (`BI-2`, `/api/dashboard/metrics` and `/api/dashboard/segments`) by
//! the same rules as `settings:write`: the string is listed in the table below,
//! no seeded role is granted it (so no migration touches the role rows), and
//! Platform Admin's `*:*` satisfies it by the resource-wildcard rule. A metric
//! is shared by every dashboard and by the assistant, so changing one is not
//! the same act as editing a chart (`dashboard:write`, which is not enough).
//! Listing, validating and the usage of a metric need `dashboard:read` only.
//!
//! # `agent:manage` / `workload:cancel` / `storage:restore` / `alert:write`
//!
//! Four more route groups had the identical problem: any authenticated
//! principal, including a read-only Analyst, could suspend/resume/revoke a
//! digital employee, cancel a running `ClickHouse` workload, restore storage
//! from a snapshot, or mutate alert rules that fire real webhooks and
//! emails. `migrations/0020_extend_role_grants.sql` extends the seed data
//! alongside these table entries rather than gating them behind a token
//! nothing can hold:
//!
//! * `agent:manage` (`/api/agents/employees/{id}/{suspend,resume,revoke}`)
//!   — same `agent` resource family as Approver's seeded `agent:approve`,
//!   but a different action: lifecycle control of the employee identity
//!   itself (a kill switch), not approving one action it proposed.
//!   Deliberately gets NO seed grant: Approver's remit is narrow
//!   per-decision approval, not blanket administration of the employees
//!   themselves, and no other seeded role plausibly owns "agent
//!   operations" — reachable only via Platform Admin's `*:*`, same as
//!   `identity:write`.
//! * `workload:cancel` (`/api/ops/workloads/{id}/cancel`) — granted to
//!   Data Engineer (`0020`) alongside its existing `pipeline:*`: Data
//!   Engineer is the role that operates pipelines end-to-end and is the
//!   one that would need to kill a runaway query a pipeline it owns
//!   launched.
//! * `storage:restore` (`/api/storage/restore`) — deliberately gets NO
//!   seed grant. It is destructive/irreversible and no seeded role
//!   represents storage/backup operations at all (Data Engineer's
//!   `pipeline:*`/`catalog:write`/`connector:manage` stop short of
//!   storage); reachable only via Platform Admin's `*:*`.
//! * `alert:write` (`/api/alerts` `POST`/`PUT`/`DELETE`) — granted to Data
//!   Engineer (`0020`): alert rules are operational tooling over the same
//!   pipelines/connectors Data Engineer already manages, and it is the
//!   natural owner of "define a threshold alert on a data pipeline".
//!
//! See `migrations/0020_extend_role_grants.sql` for the exact `UPDATE`
//! statements and `agent_workload_storage_alert_permissions_seed_alignment`
//! below for the corresponding grant assertions.

use axum::extract::{FromRequestParts, MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use lakehouse_core::ApiError;

use crate::auth::AuthenticatedPrincipal;
use crate::error::ApiRejection;
use crate::state::AppState;

/// What a route requires of a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// No authentication at all.
    Public,
    /// Any authenticated [`lakehouse_auth::Principal`], regardless of its
    /// permissions.
    RequiresAuth,
    /// An authenticated [`lakehouse_auth::Principal`] whose
    /// [`lakehouse_auth::Principal::has`] this exact string.
    RequiresPermission(&'static str),
}

/// `(HTTP method, axum route pattern, policy)`, one entry per route
/// registered by `crate::routes::router`. See the module doc comment.
///
/// The path here is the REGISTERED PATTERN (`/api/pipelines/{id}/trigger`),
/// not a concrete request path — matched against
/// [`axum::extract::MatchedPath`], which axum resolves after routing, so
/// this table never needs to know about path-parameter values.
#[rustfmt::skip]
pub const POLICY_TABLE: &[(&str, &str, Policy)] = &[
    // ── Public: the ONLY eight routes in the whole service. (Was "six" —
    // corrected when the seventh was added, since Task A5's
    // `/api/auth/oidc/callback` had already made the old count stale; the
    // eighth is `POST /api/embed/frame`, SEC-12.) ───────────────────────
    ("GET",  "/health",                          Policy::Public),
    ("POST", "/api/embed/data",                   Policy::Public),
    // BI-1 (D3): the PDF's print view has no session; its data route is
    // authenticated by the signed render token in the body, and reads as the
    // person the token names (never as the embed viewer role).
    ("POST", "/api/render/data",                  Policy::Public),
    // SEC-12: the console's `proxy.ts` asks which sites may frame an embed
    // page, with the token the page was opened with. Public because the
    // token is the credential; it answers only an origin list, and the same
    // empty list for a token that is invalid.
    ("POST", "/api/embed/frame",                  Policy::Public),
    // BI-26 (D5): the values of an editable filter's dropdown. Public because the
    // signed token is the credential; it answers only for a column that is
    // editable for that token, as the fixed viewer role, with the token's
    // locked values applied.
    ("POST", "/api/embed/values",                 Policy::Public),
    ("GET",  "/api/public/dashboard/{token}",     Policy::Public),
    // BI-19 part A (D4): trade a public link's password for a ticket. Public
    // because the visitor has no account; the token and the password are the
    // credentials, five wrong tries lock the link for 15 minutes, and a locked
    // link, a wrong password and a made-up token all fail alike.
    ("POST", "/api/public/dashboard/{token}/unlock", Policy::Public),
    ("POST", "/api/auth/login",                   Policy::Public),
    // Redirect-only, grants nothing — it only ever
    // 302s to a fixed, config-sourced IdP URL and sets a single-use flow
    // cookie. Mirrors `/api/auth/login`'s existing `Policy::Public`.
    ("GET",  "/api/auth/oidc/start",              Policy::Public),
    // Unauthenticated by necessity — the caller has no
    // session at this point in the login flow. Every verification gap
    // (missing/expired flow cookie, `state` mismatch, failed token
    // exchange, failed id-token verification) fails closed to 401 inside
    // the handler itself; this route grants nothing on its own, same as
    // `/api/auth/oidc/start` above.
    ("GET",  "/api/auth/oidc/callback",           Policy::Public),
    // Unauthenticated by necessity — the login page
    // calls this before any session exists to decide whether to render an
    // SSO button. It exposes only a boolean and an operator-chosen label
    // (`OIDC_PROVIDER_NAME`), never a secret or the issuer/client id
    // `AuthState::oidc`/`Config` also carry — see `auth::providers`'s own
    // doc comment.
    ("GET",  "/api/auth/providers",               Policy::Public),

    // ── Auth domain (this task) ──────────────────────────────────────────
    ("POST", "/api/auth/logout",                  Policy::RequiresAuth),
    ("GET",  "/api/auth/me",                      Policy::RequiresAuth),
    ("POST", "/api/auth/change-password",         Policy::RequiresAuth),
    // Route-level `Policy::RequiresAuth` is coarse —
    // every authenticated caller may list their own sessions. The
    // `identity:sessions:manage` split (own vs. every live session in
    // the deployment) lives in `routes::auth::sessions` itself: it lists
    // only the caller's own sessions unless the caller holds
    // `identity:sessions:manage`; see `routes::auth::SessionsDecision`.
    ("GET",  "/api/auth/sessions",                Policy::RequiresAuth),
    // Revoke a single live session by id. Same floor
    // as the list route — every authenticated caller may hit this, and
    // the admin-vs-own split (`identity:sessions:manage` holders revoke
    // any session; everyone else revokes only their own) lives inside
    // `routes::auth::revoke_session`, mirroring `SessionsDecision` on
    // the list side. The 404 body for "this session id belongs to
    // someone else" and "this session id does not exist" is identical —
    // see the route's doc comment for why the route is not an
    // enumeration oracle.
    ("DELETE", "/api/auth/sessions/{id}",          Policy::RequiresAuth),

    // ── Catalog: seeded Analyst permission `catalog:read`. ───────────────
    ("GET", "/api/catalog",       Policy::RequiresPermission("catalog:read")),
    ("GET", "/api/catalog/query", Policy::RequiresPermission("catalog:read")),
    ("GET", "/api/catalog/{id}",  Policy::RequiresPermission("catalog:read")),
    // Column profile: aggregates over the asset's rows (a `max` IS a value),
    // so it needs the same `query:read` as running that SELECT yourself —
    // and goes through the same policy rewrite (`routes::catalog_profile`).
    ("GET", "/api/catalog/{id}/profile", Policy::RequiresPermission("query:read")),
    // More sample rows than the detail body carries: rows are data.
    ("GET", "/api/catalog/{id}/sample", Policy::RequiresPermission("query:read")),

    // ── Catalog annotations (WS2 §13): console-only owner/steward/tags/
    //    description. PUT reuses the already-seeded `catalog:write`
    //    permission (Data Engineer role, 0002_seed_identity.sql) — no new
    //    permission string, no grant migration (WS2 plan review W8). ─────
    ("GET", "/api/catalog/{id}/annotation", Policy::RequiresPermission("catalog:read")),
    ("PUT", "/api/catalog/{id}/annotation", Policy::RequiresPermission("catalog:write")),
    // DATA-12 D1: the certified / deprecated mark is its own write, behind
    // `governance:write`, so a `catalog:write` holder who edits the details
    // cannot vouch for a table.
    ("PUT", "/api/catalog/{id}/certification", Policy::RequiresPermission("governance:write")),

    // ── Semantic layer (AI-16): plain-words descriptions of tables and
    //    columns, read by the Copilot's DATA MAP. The same two permissions
    //    as the annotations above. ───────────────────────────────────────
    ("GET", "/api/semantic", Policy::RequiresPermission("catalog:read")),
    ("GET", "/api/semantic/{asset}", Policy::RequiresPermission("catalog:read")),
    ("PUT", "/api/semantic/{asset}", Policy::RequiresPermission("catalog:write")),

    // ── Access requests (WS7 item E2/E3): anyone who can see a catalog
    //    entry may request more access to it; `access:approve` (minted by
    //    migration 0040, held only by Governance Admin) is a distinct
    //    governance authority from `agent:approve` that decides it. ───────
    ("POST", "/api/catalog/{id}/access-request",         Policy::RequiresPermission("catalog:read")),
    ("POST", "/api/catalog/access-requests/{id}/decide", Policy::RequiresPermission("access:approve")),
    // DATA-15 F3: grant and revoke from the asset's page. The same
    // authority as deciding a request, and no weaker: listing shows who
    // holds what, so it is not `catalog:read`.
    ("GET", "/api/catalog/{id}/grants",                  Policy::RequiresPermission("access:approve")),
    ("POST", "/api/catalog/{id}/grants",                 Policy::RequiresPermission("access:approve")),
    ("DELETE", "/api/catalog/{id}/grants/{grant_id}",    Policy::RequiresPermission("access:approve")),

    // DATA-15 F4: whether a dropped table can come back (anyone who can see
    // the asset), and bringing it back (a governance decision, like the
    // certification mark and unlike an edit of the details).
    ("GET", "/api/catalog/{id}/restore",                 Policy::RequiresPermission("catalog:read")),
    ("POST", "/api/catalog/{id}/restore",                Policy::RequiresPermission("governance:write")),

    // ── Lakehouse (WS2 §4): the read-only Iceberg warehouse/namespace/
    //    table surface, gated by the same seeded `catalog:read` permission
    //    as the dataset registry above. ────────────────────────────────────
    ("GET", "/api/lakehouse/warehouses",                      Policy::RequiresPermission("catalog:read")),
    ("GET", "/api/lakehouse/namespaces",                      Policy::RequiresPermission("catalog:read")),
    ("GET", "/api/lakehouse/tables",                          Policy::RequiresPermission("catalog:read")),
    ("GET", "/api/lakehouse/tables/{ns}/{table}",             Policy::RequiresPermission("catalog:read")),
    ("GET", "/api/lakehouse/tables/{ns}/{table}/maintenance", Policy::RequiresPermission("catalog:read")),
    ("POST", "/api/lakehouse/tables/{ns}/{table}/maintenance", Policy::RequiresPermission("governance:write")),
    ("GET", "/api/lakehouse/capacity",                        Policy::RequiresPermission("catalog:read")),
    ("GET", "/api/lakehouse/maintenance-policies",            Policy::RequiresAuth),

    // ── Overview / alerts: no seeded resource maps to these — auth only. ─
    ("GET",  "/api/overview",                                Policy::RequiresAuth),
    ("POST", "/api/overview",                                Policy::RequiresAuth),
    ("GET",  "/api/overview/alerts",                          Policy::RequiresAuth),
    ("POST", "/api/overview/alerts/{id}/acknowledge",         Policy::RequiresAuth),
    ("POST", "/api/overview/alerts/{id}/resolve",             Policy::RequiresAuth),
    ("POST", "/api/overview/alerts/{id}/silence",             Policy::RequiresAuth),

    // ── Ops: `{kind}` fans out to several logical resources with no single
    //    seeded permission — auth only (see module doc comment). Cancelling
    //    a workload (kills a real ClickHouse query) requires the seeded
    //    Data Engineer permission `workload:cancel` (0020). `/api/ops/logs`
    //    (WS5 item G1) is registered ahead of the `{kind}` wildcard, same
    //    literal-before-parameter precedent as `/api/governance/lineage`
    //    below — router-level floor is `RequiresAuth`; the ClickHouse
    //    branch's stricter `"ops:logs"` check (only Platform Admin's
    //    `*:*` satisfies it — `system.text_log` carries every query's SQL
    //    text verbatim) lives inside the handler, matching
    //    `routes/alerts.rs::check_run_token`'s "auth-only router gate,
    //    stricter handler guard" pattern for `/api/alerts/run`. ─────────
    ("GET",  "/api/ops/logs",                      Policy::RequiresAuth),
    ("GET",  "/api/ops/{kind}",                    Policy::RequiresAuth),
    ("POST", "/api/ops/workloads/{id}/cancel",     Policy::RequiresPermission("workload:cancel")),

    // ── Governance: seeded `policy:*` / `lineage:read`. `{kind}` (quality,
    //    classification, audit, residency) has the same fan-out problem as
    //    ops — auth only for that one. ────────────────────────────────────
    ("GET",  "/api/governance/lineage",   Policy::RequiresPermission("lineage:read")),
    // DATA-13: column-level lineage reads the same records' lineage, so the
    // same permission; charts and dashboards in the answer are listed only
    // to `dashboard:read` inside the handler.
    ("GET",  "/api/governance/lineage/columns", Policy::RequiresPermission("lineage:read")),
    // WS3 item 17: surfaces per-connector ingest history, the same
    // audience as the base `/api/connectors` routes above.
    ("GET",  "/api/governance/ingest-runs", Policy::RequiresPermission("connector:manage")),
    ("GET",  "/api/governance/policies",  Policy::RequiresPermission("policy:read")),
    ("POST", "/api/governance/policies",  Policy::RequiresPermission("policy:write")),
    // WS7 item A5: only someone who could actually save the policy may
    // preview its effect.
    ("POST", "/api/governance/policies/preview", Policy::RequiresPermission("policy:write")),
    // Enforcing, stopping or removing a policy is as much authoring it as
    // creating it is.
    ("PUT", "/api/governance/policies/{id}/status", Policy::RequiresPermission("policy:write")),
    ("DELETE", "/api/governance/policies/{id}", Policy::RequiresPermission("policy:write")),
    // WS5 item E1 (Y6): per-table freshness SLA. `governance:write` is the
    // existing grant `0030_table_maintenance_policy.sql` already gives the
    // `Governance Admin` role (not re-granted here) — same permission the
    // Iceberg maintenance write above uses.
    ("GET",  "/api/governance/sla",       Policy::RequiresPermission("policy:read")),
    ("PUT",  "/api/governance/sla",       Policy::RequiresPermission("governance:write")),
    // Taking a target or a classification back is the same decision as
    // setting one.
    ("DELETE", "/api/governance/sla/{table}", Policy::RequiresPermission("governance:write")),
    ("DELETE", "/api/governance/classification/{id}", Policy::RequiresPermission("governance:write")),
    // DATA-12 part 2: governed tag keys. Reading the list is `catalog:read`
    // because an editor with `catalog:write` needs it to render the value
    // pickers and a definition is not sensitive; defining, changing and
    // removing a key is a governance act, like a mark (decision 1).
    ("GET",  "/api/governance/tag-keys",        Policy::RequiresPermission("catalog:read")),
    ("POST", "/api/governance/tag-keys",        Policy::RequiresPermission("governance:write")),
    ("PUT",  "/api/governance/tag-keys/{key}",  Policy::RequiresPermission("governance:write")),
    ("DELETE", "/api/governance/tag-keys/{key}", Policy::RequiresPermission("governance:write")),
    // DATA-14 (F2): quality incidents. Listing them is open to any login
    // like the rule list; resolving one is a governance decision, like
    // editing or deleting a rule.
    ("GET",  "/api/governance/quality/incidents", Policy::RequiresAuth),
    ("POST", "/api/governance/quality/incidents/{id}/resolve", Policy::RequiresPermission("governance:write")),
    // DATA-14 (F3): per-table schedules. Reading is open to any login like
    // the rule list; setting one changes when real checks (and their
    // incidents and alerts) run, so it is a governance decision.
    ("GET",  "/api/governance/quality/schedules", Policy::RequiresAuth),
    ("PUT",  "/api/governance/quality/schedules/{asset}", Policy::RequiresPermission("governance:write")),
    // DATA-14 (F5): a table's learned-or-learning state; open to any login
    // like the quality list it is shown beside.
    ("GET",  "/api/governance/quality/learning/{asset}", Policy::RequiresAuth),
    // DATA-14 (F7): a rule's run history; the rule list it belongs to is open to
    // any login, and the history shows what that list shows (a verdict).
    ("GET",  "/api/governance/quality/{id}/runs", Policy::RequiresAuth),
    // Running a rule reads the table it names, so it takes what reading
    // that table takes — the same grant as the column profile.
    ("POST", "/api/governance/quality/{id}/run", Policy::RequiresPermission("query:read")),
    // Rewriting or removing a rule is a governance decision, like setting an SLA.
    ("PUT", "/api/governance/quality/{id}", Policy::RequiresPermission("governance:write")),
    ("DELETE", "/api/governance/quality/{id}", Policy::RequiresPermission("governance:write")),
    // SEC-23 (F1): reading rules stays open to any login, but adding one
    // (quality, classification, residency) takes `governance:write` — the
    // grant that rewriting or removing the same rule already takes above.
    // It used to be `RequiresAuth`, so anyone logged in could add a rule
    // they could not then edit or delete.
    ("GET",  "/api/governance/{kind}",    Policy::RequiresAuth),
    ("POST", "/api/governance/{kind}",    Policy::RequiresPermission("governance:write")),

    // ── Storage: no seeded resource — auth only. Restore (destructive)
    //    requires `storage:restore`, deliberately Platform-Admin-only —
    //    see module doc comment. ───────────────────────────────────────
    ("GET",  "/api/storage",             Policy::RequiresAuth),
    ("GET",  "/api/storage/policies",    Policy::RequiresAuth),
    ("POST", "/api/storage/policies",    Policy::RequiresAuth),
    ("GET",  "/api/storage/operations",  Policy::RequiresAuth),
    ("POST", "/api/storage/restore",     Policy::RequiresPermission("storage:restore")),

    // ── Alerts (CRUD + run trigger): reads have no seeded resource — auth
    //    only. Rule CRUD (fires real webhooks/emails) requires the seeded
    //    Data Engineer permission `alert:write` (0020). Note
    //    `/api/alerts/run` ALSO still runs its own, stricter
    //    `routes::alerts::check_run_token` guard inside the handler (D4):
    //    with `ALERTS_RUN_TOKEN` set, a matching token is required as
    //    before; with it unset, only a `PrincipalId::Service` principal is
    //    allowed through, not merely `RequiresAuth`'s "any authenticated
    //    principal" — this table entry is a floor, not the whole guard. ──
    ("GET",    "/api/alerts",      Policy::RequiresAuth),
    ("POST",   "/api/alerts",      Policy::RequiresPermission("alert:write")),
    ("PUT",    "/api/alerts",      Policy::RequiresPermission("alert:write")),
    ("DELETE", "/api/alerts",      Policy::RequiresPermission("alert:write")),
    // `SRC-7` D9: whether run reports can arrive; auth only, it reads no rule.
    ("GET",    "/api/alerts/status", Policy::RequiresAuth),
    ("GET",    "/api/alerts/run",  Policy::RequiresAuth),
    ("POST",   "/api/alerts/run",  Policy::RequiresAuth),

    // ── Gold export (ADR 0010, Rust/Phase-P6-only — no TypeScript route to
    //    port): same "floor" shape as `/api/alerts/run` — `RequiresAuth`
    //    here, PLUS `routes::gold::check_export_token`'s own stricter D4
    //    guard inside the handler (shared-token match, or a service-
    //    identity principal when `GOLD_EXPORT_RUN_TOKEN` is unset). ──────
    ("GET",  "/api/gold/export/{mart}",  Policy::RequiresAuth),
    ("POST", "/api/gold/export/{mart}",  Policy::RequiresAuth),
    // History (WS6 item 3): same RequiresAuth floor as export/read-back —
    // row counts and run timing, not public data.
    ("GET",  "/api/gold/exports",        Policy::RequiresAuth),
    // Consumers (WS6 item 4): an honest supported:false stub — no
    // ClickHouse/Iceberg/Trino data is ever returned, but the floor stays
    // RequiresAuth (not a public route) to match every other route under
    // /api/gold/export/{mart}.
    ("GET",  "/api/gold/export/{mart}/consumers", Policy::RequiresAuth),

    // DATA-1 — per-mart publish-to-Iceberg switches. `publications` is
    // what the scheduler reads (RequiresAuth floor PLUS
    // `check_export_token` in the handler, same two-layer shape as the
    // export routes above); the detail GET is read-only state for the
    // asset page (RequiresAuth floor, `canEdit` reports — never implies —
    // the permission); the PUT is the one WRITE, floored at the existing
    // `gold:export` permission so whoever may trigger an export by hand
    // is exactly who may switch automatic publishing on. Switching off
    // flips a flag only; no route under /api/gold ever drops Iceberg
    // data.
    ("GET", "/api/gold/publications",              Policy::RequiresAuth),
    ("GET",  "/api/gold/export/{mart}/publication", Policy::RequiresAuth),
    ("PUT",  "/api/gold/export/{mart}/publication", Policy::RequiresPermission("gold:export")),

    // ── Query: seeded Analyst permission `query:read`. ───────────────────
    ("POST", "/api/query/run",            Policy::RequiresPermission("query:read")),
    ("POST", "/api/query/estimate",       Policy::RequiresPermission("query:read")),
    ("GET",  "/api/query/saved",          Policy::RequiresPermission("query:read")),
    ("POST", "/api/query/saved",          Policy::RequiresPermission("query:read")),
    ("GET",  "/api/query/history",        Policy::RequiresPermission("query:read")),
    ("GET",  "/api/query/run/{id}/download", Policy::RequiresPermission("query:read")),
    ("GET",  "/api/query/scheduling",     Policy::RequiresPermission("query:read")),

    // ── Pipelines: seeded Data Engineer permission `pipeline:*`. ─────────
    ("GET",  "/api/pipelines",                        Policy::RequiresPermission("pipeline:read")),
    ("POST", "/api/pipelines",                        Policy::RequiresPermission("pipeline:write")),
    ("POST", "/api/pipelines/generate",               Policy::RequiresPermission("pipeline:write")),
    ("GET",  "/api/pipelines/{id}",                    Policy::RequiresPermission("pipeline:read")),
    ("PUT",  "/api/pipelines/{id}",                    Policy::RequiresPermission("pipeline:write")),
    ("PUT",  "/api/pipelines/{id}/target-lag",         Policy::RequiresPermission("pipeline:write")),
    ("DELETE", "/api/pipelines/{id}",                  Policy::RequiresPermission("pipeline:write")),
    ("GET",  "/api/pipelines/{id}/schedule-ticks",     Policy::RequiresPermission("pipeline:read")),
    // Every tenant's authored definitions, for the orchestrator's job
    // factory. `pipeline:read` is the floor; the handler also refuses
    // anyone but a service identity (routes::authored_pipelines).
    ("GET",  "/api/pipelines/runnable",                Policy::RequiresPermission("pipeline:read")),
    // BLD-2: the target-lag sensor's one decision-maker (plan D5). Same
    // floor as `runnable`, and the same handler-side refusal: only the
    // orchestrator's service identity (or a Platform Admin) may ask
    // whether a pipeline is behind its target. `pipeline:read` rather
    // than `pipeline:write` because a check READS state and asks for a
    // run the orchestrator launches itself — it does not mutate the
    // pipeline, which is what the `pipeline:write` routes are for.
    ("POST", "/api/pipelines/runnable/{id}/lag-check", Policy::RequiresPermission("pipeline:read")),
    // BLD-1 / D3: the orchestrator posts a pipeline's `SELECT` here and
    // gets back the rewrite that pipeline's AUTHOR is allowed to run.
    // Same floor as `runnable` above (`pipeline:read`), and the same
    // handler-level narrowing to a service identity or a Platform Admin --
    // this route reveals how a governed table rewrites for any pipeline in
    // any tenant, so it cannot be reachable by someone who merely holds
    // `pipeline:read`.
    ("POST", "/api/pipelines/runnable/{id}/enforce",   Policy::RequiresPermission("pipeline:read")),
    ("GET",  "/api/pipelines/{id}/source",             Policy::RequiresPermission("pipeline:read")),
    ("GET",  "/api/pipelines/{id}/runs",              Policy::RequiresPermission("pipeline:read")),
    // Plan 1c (R2, day-1): runs × steps matrix for the recovery UI.
    // `pipeline:read` matches `{id}/runs` and `{runId}/steps` — the
    // matrix is "runs for this pipeline" + "steps for each run"
    // composed into one round trip. axum's `matchit` router
    // resolves static segments ahead of path parameters regardless
    // of registration order (PR #57 review F1.10), so the literal
    // `runs/steps` path reaches the matrix handler and a path with
    // a real `runId` reaches the per-run handler — the route
    // registration order in `routes::pipelines_router` is a
    // readability aid, not a correctness requirement, and the
    // matrix-vs-`run_steps` resolution is pinned by
    // `tests/pipeline_routing.rs::both_paths_resolve_to_their_own_handler`.
    ("GET",  "/api/pipelines/{id}/runs/steps",        Policy::RequiresPermission("pipeline:read")),
    ("GET",  "/api/pipelines/{id}/runs/{runId}/steps", Policy::RequiresPermission("pipeline:read")),
    ("GET",  "/api/pipelines/{id}/runs/{runId}/logs",  Policy::RequiresPermission("pipeline:read")),
    ("POST", "/api/pipelines/{id}/trigger",           Policy::RequiresPermission("pipeline:write")),
    // R4 plan 2c: read-side companion to trigger. The handler refuses
    // unknown ids and returns `{ defaultConfig: null, defaultConfigYaml:
    // <yaml>, hasConfig: true }` for any job in Dagster — `pipeline:read`
    // matches `list_pipelines`/`list_pipeline_runs`, the only other
    // places a model can read pipeline state.
    ("GET",  "/api/pipelines/{id}/config-schema",      Policy::RequiresPermission("pipeline:read")),
    ("POST", "/api/pipelines/{id}/status",            Policy::RequiresPermission("pipeline:write")),
    ("POST", "/api/pipelines/{id}/pause",             Policy::RequiresPermission("pipeline:write")),
    ("POST", "/api/pipelines/{id}/resume",            Policy::RequiresPermission("pipeline:write")),
    ("POST", "/api/pipelines/runs/{runId}/cancel",    Policy::RequiresPermission("pipeline:write")),
    ("POST", "/api/pipelines/runs/{runId}/retry",     Policy::RequiresPermission("pipeline:write")),
    // Same posture as the connector tenant-assignment route below —
    // `identity:write`, not `pipeline:write`: this is a governance
    // decision about who may see the row, not a pipeline-operation grant.
    ("PUT",  "/api/pipelines/{id}/tenant",            Policy::RequiresPermission("identity:write")),
    // BLD-1 / D2: same posture as the line above — assigning the user a
    // pipeline reads as decides whose permissions every future run of that
    // row is governed by, which is a governance decision rather than a
    // pipeline-operation grant, so `identity:write` (Platform Admin's
    // `*:*`; no seeded role holds it by name) rather than `pipeline:write`.
    ("PUT",  "/api/pipelines/{id}/run-as",            Policy::RequiresPermission("identity:write")),
    // Dagster `run_failure_sensor` posts to this; the handler then
    // refuses non-service `pipeline:write` holders, mirroring
    // `authored_pipelines::runnable` (a `pipeline:write` user is still
    // not the orchestrator).
    ("POST", "/api/pipelines/events/run-failed",     Policy::RequiresPermission("pipeline:write")),
    // Same posture as the run-failed route above: the
    // `run_status_sensor(SUCCESS)` posts here, and the handler
    // refuses a non-service `pipeline:write` holder. Plan 1f.
    ("POST", "/api/pipelines/events/run-finished",   Policy::RequiresPermission("pipeline:write")),
    // Plan 1f: per-pipeline SLA. `pipeline:write` is the floor — a
    // `pipeline:read` holder can see the row's existence via the runs
    // route but cannot list or set the SLA thresholds themselves.
    ("GET",  "/api/pipelines/{id}/sla",               Policy::RequiresPermission("pipeline:write")),
    ("PUT",  "/api/pipelines/{id}/sla",               Policy::RequiresPermission("pipeline:write")),
    // Plan R4 2b: definition version history. List/get take the same
    // floor as the runs and source routes (`pipeline:read`); restore
    // takes `pipeline:write` because it mutates the row.
    ("GET",  "/api/pipelines/{id}/versions",         Policy::RequiresPermission("pipeline:read")),
    ("GET",  "/api/pipelines/{id}/versions/{version}", Policy::RequiresPermission("pipeline:read")),
    ("POST", "/api/pipelines/{id}/versions/{version}/restore", Policy::RequiresPermission("pipeline:write")),
    // Volume history: the same authorization posture as `/runs` (which
    // is `pipeline:read`); a pipeline-write holder can already set the
    // SLA, so reading the resulting row counts is `pipeline:read`.
    ("GET",  "/api/pipelines/{id}/volume",            Policy::RequiresPermission("pipeline:read")),

    // ── Dashboard: seeded Dashboard Viewer permission `dashboard:read`.
    //    `dashboard:write` is the natural write counterpart — see module
    //    doc comment for why it's grounded despite no role being granted
    //    it in the seed (Platform Admin's `*:*` still satisfies it).
    //
    //    BI-19 part B1 (plan D8): the READ routes a person needs to see a
    //    dashboard are `RequiresAuth` with a handler guard ("auth-only router
    //    gate, stricter handler guard", as `/api/alerts/run`). Somebody can be
    //    given one dashboard without holding `dashboard:read`, so the router
    //    cannot ask for it; every handler below calls
    //    `routes::dashboard_access::viewer_for` (or `require_entitled` for the
    //    catalog reads that need no dashboard), which refuses a caller who
    //    holds neither `dashboard:read` nor a rule naming them or one of their
    //    roles with the same 403 as before, and otherwise shows only what the
    //    rules give. Each moved row is asserted both ways in
    //    `tests/route_auth.rs` (no rule: 403; a rule: past the guard). The
    //    rows still `dashboard:read` here are either writes or reads the
    //    chart builder and the Share dialog use, which B2 moves with the
    //    write floors. ─────────────────────────────────────────────────────
    ("GET",    "/api/dashboard",              Policy::RequiresAuth),
    ("GET",    "/api/dashboard/specs",        Policy::RequiresPermission("dashboard:read")),
    ("POST",   "/api/dashboard/specs/preview", Policy::RequiresPermission("dashboard:read")),
    ("POST",   "/api/dashboard/specs",        Policy::RequiresPermission("dashboard:write")),
    ("PUT",    "/api/dashboard/specs",        Policy::RequiresPermission("dashboard:write")),
    ("DELETE", "/api/dashboard/specs",        Policy::RequiresPermission("dashboard:write")),
    ("GET",    "/api/dashboard/boards",       Policy::RequiresAuth),
    ("POST",   "/api/dashboard/boards",       Policy::RequiresPermission("dashboard:write")),
    ("PUT",    "/api/dashboard/boards",       Policy::RequiresPermission("dashboard:write")),
    ("DELETE", "/api/dashboard/boards",       Policy::RequiresPermission("dashboard:write")),
    // Dashboard folders (plan §4): filing dashboards is editing them.
    ("GET",    "/api/dashboard/folders",      Policy::RequiresAuth),
    ("POST",   "/api/dashboard/folders",      Policy::RequiresPermission("dashboard:write")),
    ("PUT",    "/api/dashboard/folders",      Policy::RequiresPermission("dashboard:write")),
    ("DELETE", "/api/dashboard/folders",      Policy::RequiresPermission("dashboard:write")),
    // BI-18 part C, part 2: the personal space is created on first use (writing
    // is what makes a dashboard to put in it); history and the trash are read
    // with the dashboard, and going back or restoring edit it, so they need the
    // permission that edits and deletes. Favourites and recent items are the
    // caller's own preferences, never an edit of a dashboard: `dashboard:read`.
    ("POST",   "/api/dashboard/personal",        Policy::RequiresPermission("dashboard:write")),
    ("GET",    "/api/dashboard/versions",        Policy::RequiresAuth),
    ("POST",   "/api/dashboard/versions/revert", Policy::RequiresPermission("dashboard:write")),
    ("GET",    "/api/dashboard/trash",           Policy::RequiresPermission("dashboard:read")),
    ("POST",   "/api/dashboard/trash/restore",   Policy::RequiresPermission("dashboard:write")),
    ("GET",    "/api/dashboard/shortcuts",       Policy::RequiresAuth),
    ("PUT",    "/api/dashboard/favourites",      Policy::RequiresAuth),
    ("POST",   "/api/dashboard/recent",          Policy::RequiresAuth),
    ("GET",    "/api/dashboard/search",          Policy::RequiresAuth),
    // Dashboard SQL sources: listing is a read; authoring and previewing
    // arbitrary SQL is its own permission, separate from dashboard:write
    // (plan §1, the Metabase "native query" split).
    ("GET",    "/api/dashboard/sources",      Policy::RequiresPermission("dashboard:read")),
    ("POST",   "/api/dashboard/sources",      Policy::RequiresPermission("dashboard:sql")),
    ("PUT",    "/api/dashboard/sources",      Policy::RequiresPermission("dashboard:sql")),
    ("DELETE", "/api/dashboard/sources",      Policy::RequiresPermission("dashboard:sql")),
    ("POST",   "/api/dashboard/sources/preview", Policy::RequiresPermission("dashboard:sql")),
    // BI-8: calculated fields. Validate and the function catalog only read
    // (a source's column list, a fixed table); saving needs the same
    // permission as saving a chart.
    ("GET",    "/api/dashboard/calc-fields",          Policy::RequiresAuth),
    ("POST",   "/api/dashboard/calc-fields",          Policy::RequiresPermission("dashboard:write")),
    ("PUT",    "/api/dashboard/calc-fields",          Policy::RequiresPermission("dashboard:write")),
    ("DELETE", "/api/dashboard/calc-fields",          Policy::RequiresPermission("dashboard:write")),
    ("POST",   "/api/dashboard/calc-fields/validate", Policy::RequiresPermission("dashboard:read")),
    ("GET",    "/api/dashboard/calc-fields/functions", Policy::RequiresPermission("dashboard:read")),
    // BI-2: metrics and segments. Using one needs only the dashboard; defining
    // or changing one is its own permission, `metric:write` (see the module doc).
    ("GET",    "/api/dashboard/metrics",              Policy::RequiresAuth),
    ("POST",   "/api/dashboard/metrics",              Policy::RequiresPermission("metric:write")),
    ("PUT",    "/api/dashboard/metrics",              Policy::RequiresPermission("metric:write")),
    ("DELETE", "/api/dashboard/metrics",              Policy::RequiresPermission("metric:write")),
    ("POST",   "/api/dashboard/metrics/validate",     Policy::RequiresPermission("dashboard:read")),
    ("GET",    "/api/dashboard/metrics/{id}/usage",   Policy::RequiresPermission("dashboard:read")),
    ("GET",    "/api/dashboard/segments",             Policy::RequiresAuth),
    ("POST",   "/api/dashboard/segments",             Policy::RequiresPermission("metric:write")),
    ("PUT",    "/api/dashboard/segments",             Policy::RequiresPermission("metric:write")),
    ("DELETE", "/api/dashboard/segments",             Policy::RequiresPermission("metric:write")),
    ("GET",    "/api/dashboard/fields",       Policy::RequiresAuth),
    // BI-7: join suggestions read column metadata only, like `fields`.
    ("GET",    "/api/dashboard/join-suggestions", Policy::RequiresPermission("dashboard:read")),
    ("GET",    "/api/dashboard/hierarchies",      Policy::RequiresAuth),
    ("POST",   "/api/dashboard/hierarchies",      Policy::RequiresPermission("dashboard:write")),
    ("PUT",    "/api/dashboard/hierarchies",      Policy::RequiresPermission("dashboard:write")),
    ("DELETE", "/api/dashboard/hierarchies",      Policy::RequiresPermission("dashboard:write")),
    // BI-11: parameters are read with the dashboard and defined, changed and
    // deleted with `dashboard:write`; a viewer's value is a request parameter,
    // not a write.
    ("GET",    "/api/dashboard/params",           Policy::RequiresAuth),
    ("POST",   "/api/dashboard/params",           Policy::RequiresPermission("dashboard:write")),
    ("PUT",    "/api/dashboard/params",           Policy::RequiresPermission("dashboard:write")),
    ("DELETE", "/api/dashboard/params",           Policy::RequiresPermission("dashboard:write")),
    ("GET",    "/api/dashboard/records",      Policy::RequiresAuth),
    ("GET",    "/api/dashboard/values",       Policy::RequiresAuth),
    ("GET",    "/api/dashboard/export",       Policy::RequiresAuth),
    // BI-16 part A (T8): the whole result of a raw table as CSV. Same
    // permission as seeing the tile; every export is audited.
    ("GET",    "/api/dashboard/table-export", Policy::RequiresAuth),
    // BI-1 (D6): any chart's data as Excel or CSV. Same permission as seeing
    // the tile, narrowed by the roles an administrator switched off from
    // downloading (checked in the handler, D7); every download is audited.
    ("GET",    "/api/dashboard/chart-export", Policy::RequiresAuth),
    // BI-1 (D5): a dashboard as a PDF. `dashboard:read`, narrowed by the roles
    // switched off from downloading (D7), audited before the file is sent.
    ("GET",    "/api/dashboard/export/pdf",   Policy::RequiresAuth),
    ("GET",    "/api/dashboard/embed-info",   Policy::RequiresPermission("dashboard:read")),
    // RPT-1 (D7): scheduled reports. `dashboard:read` gets past the gate; the
    // handlers then require the download right to create, and the owner or an
    // administrator (`settings:write`) to read, change, pause, send or delete.
    ("GET",    "/api/reports/subscriptions",  Policy::RequiresPermission("dashboard:read")),
    ("POST",   "/api/reports/subscriptions",  Policy::RequiresPermission("dashboard:read")),
    ("GET",    "/api/reports/subscriptions/{id}", Policy::RequiresPermission("dashboard:read")),
    ("PUT",    "/api/reports/subscriptions/{id}", Policy::RequiresPermission("dashboard:read")),
    ("DELETE", "/api/reports/subscriptions/{id}", Policy::RequiresPermission("dashboard:read")),
    ("POST",   "/api/reports/subscriptions/{id}/send", Policy::RequiresPermission("dashboard:read")),
    ("GET",    "/api/reports/subscriptions/{id}/deliveries", Policy::RequiresPermission("dashboard:read")),
    ("GET",    "/api/reports/recipients",     Policy::RequiresPermission("dashboard:read")),
    // SEC-12: withdrawing a token changes who can read the dashboard, so it
    // is a write, like the embed toggle on `PUT /api/dashboard/boards`.
    ("POST",   "/api/dashboard/embed-revoke", Policy::RequiresPermission("dashboard:write")),
    // BI-26 (D8): which filters an embed viewer sees, changes or never sees is
    // what a viewer may do, so it is a write (manage on the dashboard once
    // `BI-19` part B ships, feature decision 11).
    ("PUT",    "/api/dashboard/embed-settings", Policy::RequiresPermission("dashboard:write")),
    // BI-19 part B1 (D10): the permissions of a dashboard or folder. "Signed in"
    // at the router and "manage on the item" in the handler
    // (`dashboard_access::managed_subject`), because a person can manage one
    // dashboard by a rule without holding `dashboard:write`; somebody with
    // neither `dashboard:read` nor a rule gets 403 there.
    ("GET",    "/api/dashboard/access",       Policy::RequiresAuth),
    ("PUT",    "/api/dashboard/access",       Policy::RequiresAuth),
    // The move dialog's "who gains and who loses access" (feature B7): manage
    // on the dashboard, in the handler, like the access routes.
    ("GET",    "/api/dashboard/access-preview", Policy::RequiresAuth),
    // BI-19 part A (D5): making, changing and ending a dashboard's public link
    // changes who can read it, so all three are writes (reading its state is
    // too: the response carries the link's token). Part B of the plan moves
    // these to "manage on the dashboard" with a handler check.
    ("GET",    "/api/dashboard/public-link",  Policy::RequiresPermission("dashboard:write")),
    ("PUT",    "/api/dashboard/public-link",  Policy::RequiresPermission("dashboard:write")),
    ("DELETE", "/api/dashboard/public-link",  Policy::RequiresPermission("dashboard:write")),

    // ── Reporting settings (`BI-9`, `routes::settings`). Reading is auth-only:
    //    the console needs the zone and the first day to label every date
    //    axis, whoever is viewing. Changing them moves every date on every
    //    dashboard, so it is its own permission, `settings:write`, introduced
    //    by the same rules as `identity:*` and `storage:restore` above: no
    //    seed grant, so only a `*:*` holder (Platform Admin) has it until an
    //    operator grants it to a role; see
    //    `settings_write_needs_no_seed_change_and_no_seeded_role_has_it`. ────
    ("GET",    "/api/settings/reporting",     Policy::RequiresAuth),
    ("PUT",    "/api/settings/reporting",     Policy::RequiresPermission("settings:write")),

    // ── Agent / AI: no seeded resource for free-form ask/chat — auth only.
    ("POST", "/api/ai/chat",             Policy::RequiresAuth),
    ("POST", "/api/ai/tool",             Policy::RequiresAuth),
    ("GET",    "/api/ai/sessions",       Policy::RequiresAuth),
    ("POST",   "/api/ai/sessions",       Policy::RequiresAuth),
    ("PATCH",  "/api/ai/sessions",       Policy::RequiresAuth),
    ("DELETE", "/api/ai/sessions",       Policy::RequiresAuth),
    ("GET",  "/api/ai/build-status",     Policy::RequiresAuth),
    // The caller's own remembered words for the chat (`routes::ai::terms`):
    // keyed by the principal like Copilot sessions, so the `RequiresAuth`
    // floor is the whole policy. A caller can only ever reach their own rows.
    ("GET",    "/api/ai/terms",          Policy::RequiresAuth),
    ("PUT",    "/api/ai/terms",          Policy::RequiresAuth),
    ("DELETE", "/api/ai/terms",          Policy::RequiresAuth),

    // WS5 item F1: navbar bell — same `RequiresAuth` floor as every other
    // authenticated-but-not-permission-scoped read in this table (e.g.
    // `/api/overview`, `/api/ai/build-status` immediately above); every
    // principal sees their own deployment's open alerts/pending approvals,
    // not a permission-scoped subset.
    ("GET",  "/api/notifications",       Policy::RequiresAuth),

    // The caller's own Home layout (`routes::home`): per-user display
    // preference, keyed by the principal like Copilot sessions, so the
    // `RequiresAuth` floor is the whole policy. A caller can only ever
    // reach their own row.
    ("GET",    "/api/home/layout",       Policy::RequiresAuth),
    ("PUT",    "/api/home/layout",       Policy::RequiresAuth),
    ("DELETE", "/api/home/layout",       Policy::RequiresAuth),

    // ── Identity (Phase 2 directory): permission-gated (D1 fix). Reads
    //    require `identity:read`, mutations (create user/role/tenant/
    //    service-identity — the last of which can mint a `*:*` role and
    //    attach it to a caller) require `identity:write`. Only Platform
    //    Admin's `*:*` grants either today; see the module doc comment. ───
    ("GET",  "/api/identity/users",                  Policy::RequiresPermission("identity:read")),
    ("POST", "/api/identity/users",                  Policy::RequiresPermission("identity:write")),
    ("PUT",  "/api/identity/users/{id}/tenants/{tenant_id}", Policy::RequiresPermission("identity:write")),
    ("GET",  "/api/identity/roles",                  Policy::RequiresPermission("identity:read")),
    ("POST", "/api/identity/roles",                  Policy::RequiresPermission("identity:write")),
    ("GET",  "/api/identity/tenants",                Policy::RequiresPermission("identity:read")),
    ("POST", "/api/identity/tenants",                Policy::RequiresPermission("identity:write")),
    ("GET",  "/api/identity/service-identities",     Policy::RequiresPermission("identity:read")),
    ("POST", "/api/identity/service-identities",     Policy::RequiresPermission("identity:write")),
    // Rotation is a write-side
    // identity-domain operation — reuses `identity:write` exactly the same
    // way every other service-identity write does above, per AGENTS.md rule
    // 4 (no new permission token minted for a route that already has a
    // well-grounded one).
    ("POST", "/api/identity/service-identities/{id}/rotate", Policy::RequiresPermission("identity:write")),

    // ── Connectors: seeded Data Engineer permission `connector:manage`. ──
    ("GET",  "/api/connectors",             Policy::RequiresPermission("connector:manage")),
    // `ingest:read` (`0033_connector_ingest_spec.sql`'s grant) is a
    // STRICTLY NARROWER read-only permission than `connector:manage` --
    // the whole reason this is a separate route rather than a query param
    // on `/api/connectors` above (see `routes::mod::connectors_router`'s
    // comment). An `ingest:read`-only caller (the Dagster ingest service
    // identity, `dagster/dispar_orchestrate/ingest_factory.py`) must
    // never be able to call the base `/api/connectors` route.
    // SEC-16: who sees which rows is decided in the handler, not here: a
    // service identity and a `*:*` principal see every tenant's connectors,
    // anyone else only the active tenant's (none, if in no tenant).
    ("GET",  "/api/connectors/ingestible",  Policy::RequiresPermission("ingest:read")),
    // Gap fix (WS3 item 33): the creation wizard's connector-type list --
    // a small reference table feeding directly into `POST
    // /api/connectors` immediately below, which is `connector:manage`-
    // gated, not the narrower `ingest:read` scope minted only for the
    // Dagster ingest service identity.
    ("GET",  "/api/connectors/types",       Policy::RequiresPermission("connector:manage")),
    // SRC-12 part B: what the credential form may offer (the installation's
    // secret manager, and whether typing a value is allowed). Names no
    // address, sign-in or secret; read by whoever can set a credential.
    ("GET",  "/api/connectors/credential-settings", Policy::RequiresPermission("connector:manage")),
    ("POST", "/api/connectors",             Policy::RequiresPermission("connector:manage")),
    ("GET",  "/api/connectors/{id}",        Policy::RequiresPermission("connector:manage")),
    // Was missing entirely while `routes::mod` registered `.delete(...)` on
    // this pattern, so `auth_gate` classified every DELETE as unclassified and
    // returned a hard 500 — the deny-by-default branch doing exactly its job,
    // on a route that was simply never added here. See
    // `tests/route_auth.rs::every_registered_route_has_a_policy_entry`.
    ("DELETE", "/api/connectors/{id}",      Policy::RequiresPermission("connector:manage")),
    // The console's edit page: name/direction/environment/residency/host.
    // Type, tenant, credential and dial each keep their own route.
    ("PATCH", "/api/connectors/{id}",       Policy::RequiresPermission("connector:manage")),
    ("POST", "/api/connectors/{id}/test",   Policy::RequiresPermission("connector:manage")),
    // Same tier as `GET /api/connectors/{id}`: probe history is read-only
    // and carries nothing more sensitive than the `/test` route's own
    // result message already returns to this same permission.
    ("GET",  "/api/connectors/{id}/probe-history", Policy::RequiresPermission("connector:manage")),
    // Rewrites which credential reference the connector dials with next
    // (probe-first — see `routes::connectors::rotate_secret`'s doc
    // comment) — a MUTATING action over the same credential-adjacent
    // surface `/test` and `/discovery/{kind}` already gate on `connector:manage`,
    // not a new, narrower permission.
    ("PUT",  "/api/connectors/{id}/secret", Policy::RequiresPermission("connector:manage")),
    // ADR 0002 Addendum 4: the user-supplied credential write path — same
    // permission as every other connector mutation (probe-first too, see
    // `routes::connectors::set_credential`'s doc comment).
    ("PUT",  "/api/connectors/{id}/credential", Policy::RequiresPermission("connector:manage")),
    // `SRC-13` (D11): list and preview a source's tables, files, endpoints, topics
    // and collections, the same audience as the connection test. It replaced
    // `POST /api/connectors/{id}/discover` (WS3 item 14), which read a SQL
    // source's live schema with the same sensitivity as `/test` (a real,
    // credentialed connection to the connector's own target) and so gated on the
    // same permission; the console moved to these routes and that route is gone.
    // The tenant gate is the route layer.
    ("POST", "/api/connectors/{id}/discovery/{kind}", Policy::RequiresPermission("connector:manage")),
    ("GET",  "/api/connectors/{id}/discovery/{kind}", Policy::RequiresPermission("connector:manage")),
    ("GET",  "/api/connectors/{id}/debezium-properties", Policy::RequiresPermission("connector:manage")),
    // `ingest-spec` GET/PUT deliberately carry DIFFERENT permissions, unlike
    // every other connector route above: `connector:manage` is a broad
    // write-capable grant, and a read-only caller (Phase G's ingest
    // service identity, minted with only `ingest:read` — see
    // `0033_connector_ingest_spec.sql`) must never be handed it just to
    // read a `dial`. `PermissionSet::has` (`lakehouse-auth/src/
    // permissions.rs`) matches resource+action exactly, so `ingest:read`
    // and `connector:manage` are two independent grants — the seeded Data
    // Engineer role holds both (`0033`'s `UPDATE`), an ingest-only caller
    // holds only the first. See `tests/route_auth.rs` for the three-way
    // assertion this split requires.
    ("GET",  "/api/connectors/{id}/ingest-spec", Policy::RequiresPermission("ingest:read")),
    ("PUT",  "/api/connectors/{id}/ingest-spec", Policy::RequiresPermission("connector:manage")),
    // Launches a real Dagster run (or, for a cdc connector, only reports
    // why it can't) -- a MUTATING action, so `connector:manage`, not
    // `ingest:read`. The ingest:read-scoped Dagster ingest-schedule-
    // factory service identity (`main::bootstrap_ingest_run_service`) is
    // a READER of `/ingestible`/`ingest-spec`, it never calls this route
    // itself (WS3 item 29).
    ("POST", "/api/connectors/{id}/ingest/run", Policy::RequiresPermission("connector:manage")),
    // The connector's own ingest runs, read from Dagster: the same
    // audience as the per-table results (`/api/governance/ingest-runs`)
    // and the probe history above.
    // `SRC-8`: the orchestrator reports a source table's columns before it
    // loads the table and obeys the answer. `ingest:read` is the one
    // permission its ingest identity holds (`main::bootstrap_ingest_run_service`,
    // scoped to `ingest:read` ONLY); a user can hold it too (Data Engineer), so
    // the handler adds the `run_failed_event` check: a service identity or an
    // unrestricted administrator only. No new permission was invented.
    ("POST", "/api/connectors/{id}/schema-observations", Policy::RequiresPermission("ingest:read")),
    // `SRC-8` decision D2: the orchestrator reports tables that appeared in a
    // schema the connector loads from (policy `apply_all` only). Same gate and
    // the same handler-side service check as the route above.
    ("POST", "/api/connectors/{id}/schema-observations/tables", Policy::RequiresPermission("ingest:read")),
    // `SRC-11`: the run history, a run's log (JSON or a text download) and
    // Resume, for a person (decision D8); the tenant gate is the route layer.
    // Resume lifts only the pause an auto-disable set.
    ("GET",  "/api/connectors/{id}/runs", Policy::RequiresPermission("connector:manage")),
    ("GET",  "/api/connectors/{id}/runs/{runId}/log", Policy::RequiresPermission("connector:manage")),
    ("POST", "/api/connectors/{id}/resume", Policy::RequiresPermission("connector:manage")),
    // `SRC-11`: the orchestrator reports a run's start, each attempt, a line of
    // the closed log list, and the run's end. `ingest:read`, the one
    // permission its service identity holds; a user can hold it too (Data
    // Engineer), so each handler adds `schema_changes::require_orchestrator`
    // (a service identity or an unrestricted administrator only), as the two
    // `schema-observations` routes above do.
    ("POST", "/api/connectors/{id}/run-reports", Policy::RequiresPermission("ingest:read")),
    ("POST", "/api/connectors/{id}/run-reports/{runId}/attempts", Policy::RequiresPermission("ingest:read")),
    ("POST", "/api/connectors/{id}/run-reports/{runId}/log", Policy::RequiresPermission("ingest:read")),
    ("POST", "/api/connectors/{id}/run-reports/{runId}/end", Policy::RequiresPermission("ingest:read")),
    // `SRC-13`: the orchestrator posts the answer to a discovery request it was
    // launched for. `ingest:read` is the one permission its service identity
    // holds; a user can hold it too (Data Engineer), so the handler adds
    // `schema_changes::require_orchestrator`, as the run reports above do.
    ("POST", "/api/connectors/{id}/discovery-results", Policy::RequiresPermission("ingest:read")),
    // `SRC-12` part C: signing a REST connector in with `OAuth2`. A person
    // reads the sign-in status, starts a sign-in and signs out; the tenant gate
    // is the route layer.
    ("GET",  "/api/connectors/{id}/oauth", Policy::RequiresPermission("connector:manage")),
    ("POST", "/api/connectors/{id}/oauth/start", Policy::RequiresPermission("connector:manage")),
    ("POST", "/api/connectors/{id}/oauth/sign-out", Policy::RequiresPermission("connector:manage")),
    // The provider's redirect carries the person's session cookie and nothing
    // else, so it needs a signed-in user and no more here: it names no connector.
    // The handler consumes the one-time `state`, then requires that user to be
    // the one who started the sign-in, to belong to the connector's tenant and to
    // hold `connector:manage` (tests/connector_oauth_flow.rs).
    ("GET",  "/api/connectors/oauth/callback", Policy::RequiresAuth),
    // The orchestrator reports a refresh token the provider rotated.
    // `ingest:read` is the one permission its service identity holds; a user can
    // hold it too (Data Engineer), so the handler adds
    // `schema_changes::require_orchestrator`, as the run reports do.
    ("POST", "/api/connectors/{id}/oauth/refresh-token", Policy::RequiresPermission("ingest:read")),
    // A person reads what changed at the source and approves what waits
    // (`SRC-8` decision D6). The tenant gate is the route layer.
    ("GET",  "/api/connectors/{id}/schema-changes", Policy::RequiresPermission("connector:manage")),
    ("POST", "/api/connectors/{id}/schema-changes/approve", Policy::RequiresPermission("connector:manage")),
    // The tenant-assignment route. `identity:
    // write`, not `connector:manage` — this is a governance decision about
    // WHO may see the row, the same permission every other tenant-
    // membership write already requires (`POST /api/identity/tenants`
    // above), not the broader "operate this connector" grant. Held today
    // only by Platform Admin's `*:*` (`0002_seed_identity.sql`).
    ("PUT", "/api/connectors/{id}/tenant", Policy::RequiresPermission("identity:write")),

    // ── Uploaded files (ADR 0014, decision 6): the existing
    //    `connector:manage`, no new permission and no role grant — bringing a
    //    file in is the same authority as defining a source. Every
    //    `/api/uploads/{id}*` route additionally answers 404 for an upload
    //    outside the caller's tenants (`routes::uploads::
    //    require_upload_in_tenants`, mounted in `routes::uploads_router`), so
    //    this permission alone never reaches another tenant's files. ───────
    ("GET",  "/api/uploads",                 Policy::RequiresPermission("connector:manage")),
    ("POST", "/api/uploads",                 Policy::RequiresPermission("connector:manage")),
    ("GET",  "/api/uploads/{id}",            Policy::RequiresPermission("connector:manage")),
    ("DELETE", "/api/uploads/{id}",          Policy::RequiresPermission("connector:manage")),
    ("GET",  "/api/uploads/{id}/preview",    Policy::RequiresPermission("connector:manage")),
    ("POST", "/api/uploads/{id}/ingest",     Policy::RequiresPermission("connector:manage")),

    // ── Knowledge: no seeded resource — auth only. ───────────────────────
    ("GET",  "/api/knowledge/sources",       Policy::RequiresAuth),
    ("POST", "/api/knowledge/sources",       Policy::RequiresAuth),
    ("GET",  "/api/knowledge/vector-jobs",   Policy::RequiresAuth),
    ("POST", "/api/knowledge/vector-jobs",   Policy::RequiresAuth),

    // ── Agents (digital employees / workflows / tools / runs): no seeded
    //    resource for most of this domain — auth only. `approvals` maps to
    //    the seeded Approver permission `agent:approve`. Employee lifecycle
    //    control (suspend/resume/revoke) requires `agent:manage`,
    //    deliberately Platform-Admin-only — see module doc comment. ──────
    ("GET",  "/api/agents/workflows",                Policy::RequiresAuth),
    ("POST", "/api/agents/workflows",                Policy::RequiresAuth),
    ("GET",  "/api/agents/employees",                Policy::RequiresAuth),
    ("POST", "/api/agents/employees",                Policy::RequiresAuth),
    ("GET",  "/api/agents/employees/{id}",           Policy::RequiresAuth),
    ("POST", "/api/agents/employees/{id}/suspend",   Policy::RequiresPermission("agent:manage")),
    ("POST", "/api/agents/employees/{id}/resume",    Policy::RequiresPermission("agent:manage")),
    ("POST", "/api/agents/employees/{id}/revoke",    Policy::RequiresPermission("agent:manage")),
    // `/employees/{id}/run` (T3.2, copilot-operations-handover plan): same
    // "floor" shape as `/api/alerts/run`/`/api/gold/export/{mart}` —
    // `RequiresAuth` here, PLUS `routes::agents::check_employee_run_auth`'s
    // own stricter guard inside the handler (a matching `x-run-token`
    // against `AGENT_RUN_TOKEN`, OR an authenticated principal holding
    // `agent:manage` — unlike gold/alerts, the no-token fallback is a
    // PERMISSION check, not a service-identity check, because a human
    // clicking "Run now" is a legitimate caller here, not only a
    // scheduler). ────────────────────────────────────────────────────────
    ("POST", "/api/agents/employees/{id}/run",       Policy::RequiresAuth),
    // POST /api/agents/tools removed (WS7 item G4): the real tool
    // registry (ai_registry::TOOLS) has no "register" concept.
    ("GET",  "/api/agents/tools",                    Policy::RequiresAuth),
    ("GET",  "/api/agents/runs",                     Policy::RequiresAuth),
    ("GET",  "/api/agents/runs/{id}",                Policy::RequiresAuth),
    ("GET",  "/api/agents/approvals",                Policy::RequiresPermission("agent:approve")),
    ("POST", "/api/agents/approvals/{id}/decide",    Policy::RequiresPermission("agent:approve")),
];

/// Look up `(method, path)`'s policy. `None` means "not classified" — see
/// the module doc comment for why [`auth_gate`] treats that as deny, not
/// allow.
#[must_use]
pub fn policy_for(method: &str, path: &str) -> Option<Policy> {
    POLICY_TABLE
        .iter()
        .find(|(m, p, _)| *m == method && *p == path)
        .map(|(_, _, policy)| *policy)
}

/// `(method, registered route pattern)` pairs a
/// [`lakehouse_auth::Principal`] with `must_change_password == true` may
/// still reach. See [`auth_gate`]'s doc comment for why exactly these
/// three and no others.
const ALLOWED_WHILE_MUST_CHANGE_PASSWORD: &[(&str, &str)] = &[
    ("POST", "/api/auth/change-password"),
    ("POST", "/api/auth/logout"),
    ("GET", "/api/auth/me"),
];

/// Whether `(method, path)` stays reachable for a principal that still
/// must rotate its password. See [`auth_gate`]'s doc comment.
#[must_use]
fn allowed_while_must_change_password(method: &str, path: &str) -> bool {
    ALLOWED_WHILE_MUST_CHANGE_PASSWORD
        .iter()
        .any(|(m, p)| *m == method && *p == path)
}

/// The one middleware layered onto the whole router (`routes::router`).
/// Looks up [`POLICY_TABLE`] for the request's matched route and either
/// lets it through (`Policy::Public`), requires and checks a
/// [`crate::auth::AuthenticatedPrincipal`] (`Policy::RequiresAuth`/
/// `Policy::RequiresPermission`), or — if the route has no policy entry at
/// all — refuses it outright rather than defaulting to public.
///
/// # `must_change_password` enforcement
///
/// A [`lakehouse_auth::Principal`] whose `must_change_password` is `true`
/// (a bootstrapped local identity, or a session minted from one, that has
/// never rotated its credential — see
/// [`lakehouse_auth::Principal::must_change_password`]'s doc comment) is
/// refused with a 403 on every route EXCEPT the three in
/// [`ALLOWED_WHILE_MUST_CHANGE_PASSWORD`]:
/// `POST /api/auth/change-password` (the only way out of this state),
/// `POST /api/auth/logout` (a caller stuck here must still be able to sign
/// out), and `GET /api/auth/me` (the frontend `AuthProvider` calls this on
/// every page load to learn who is signed in and whether a rotation is
/// pending — refusing it would break a plain page refresh mid-rotation; it
/// only ever returns the caller's own identity and this same flag, so
/// allowing it leaks nothing a rejected request wouldn't already have
/// implied). This check runs AFTER `Policy::RequiresPermission`'s own
/// check and applies regardless of that route's permission — even a
/// Platform Admin bootstrap account cannot use `*:*` to route around its
/// own pending rotation.
///
/// # Errors
///
/// Returns a 500 `route_policy_unclassified` [`ApiRejection`] if the
/// matched route has no [`POLICY_TABLE`] entry, a 401 if
/// [`crate::auth::AuthenticatedPrincipal`] extraction fails, or a 403 if
/// the extracted principal lacks a required permission, or still has a
/// password rotation pending and this route isn't one of the three above.
pub async fn auth_gate(
    State(state): State<AppState>,
    matched_path: MatchedPath,
    req: Request,
    next: Next,
) -> Result<Response, ApiRejection> {
    let method = req.method().as_str().to_owned();
    let Some(policy) = policy_for(&method, matched_path.as_str()) else {
        tracing::error!(
            method = %method,
            path = matched_path.as_str(),
            "route has no auth policy classification; denying by default"
        );
        return Err(ApiError::Internal("route_policy_unclassified".to_owned()).into());
    };

    match policy {
        Policy::Public => Ok(next.run(req).await),
        Policy::RequiresAuth | Policy::RequiresPermission(_) => {
            let (mut parts, body) = req.into_parts();
            let AuthenticatedPrincipal(principal) =
                AuthenticatedPrincipal::from_request_parts(&mut parts, &state).await?;
            if let Policy::RequiresPermission(permission) = policy
                && !principal.has(permission)
            {
                return Err(ApiError::PermissionDenied(permission.to_owned()).into());
            }
            if principal.must_change_password
                && !allowed_while_must_change_password(&method, matched_path.as_str())
            {
                return Err(ApiError::PermissionDenied(
                    "password rotation required before this route is reachable".to_owned(),
                )
                .into());
            }
            parts.extensions.insert(principal);
            let req = Request::from_parts(parts, body);
            Ok(next.run(req).await)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `DATA-12` D1: the certified / deprecated mark is gated by
    /// `governance:write`, not by the `catalog:write` of the details.
    #[test]
    fn the_certification_write_needs_governance_write() {
        assert_eq!(
            policy_for("PUT", "/api/catalog/{id}/certification"),
            Some(Policy::RequiresPermission("governance:write"))
        );
    }

    /// `DATA-13`: column-level lineage needs `lineage:read`, like the table
    /// graph, and the route is classified (an unclassified route is a 500).
    #[test]
    fn column_lineage_needs_lineage_read() {
        assert_eq!(
            policy_for("GET", "/api/governance/lineage/columns"),
            Some(Policy::RequiresPermission("lineage:read"))
        );
    }

    /// `DATA-15` F4: the restore status is `catalog:read`; undropping a table
    /// is `governance:write`.
    #[test]
    fn restore_status_needs_catalog_read_and_the_undrop_needs_governance_write() {
        assert_eq!(
            policy_for("GET", "/api/catalog/{id}/restore"),
            Some(Policy::RequiresPermission("catalog:read"))
        );
        assert_eq!(
            policy_for("POST", "/api/catalog/{id}/restore"),
            Some(Policy::RequiresPermission("governance:write"))
        );
    }

    /// `DATA-15` F3: listing, making and ending a grant from an asset's page
    /// all need `access:approve`, the authority that decides a request.
    #[test]
    fn the_grant_routes_need_access_approve() {
        for (method, path) in [
            ("GET", "/api/catalog/{id}/grants"),
            ("POST", "/api/catalog/{id}/grants"),
            ("DELETE", "/api/catalog/{id}/grants/{grant_id}"),
        ] {
            assert_eq!(
                policy_for(method, path),
                Some(Policy::RequiresPermission("access:approve")),
                "{method} {path}"
            );
        }
    }

    /// `DATA-12` part 2: reading the tag-key list is `catalog:read` (editors
    /// need it to render pickers); defining, changing and removing a key is
    /// `governance:write`, and nothing under `tag-keys` is left unclassified.
    #[test]
    fn tag_key_reads_need_catalog_read_and_writes_need_governance_write() {
        assert_eq!(
            policy_for("GET", "/api/governance/tag-keys"),
            Some(Policy::RequiresPermission("catalog:read"))
        );
        for (method, path) in [
            ("POST", "/api/governance/tag-keys"),
            ("PUT", "/api/governance/tag-keys/{key}"),
            ("DELETE", "/api/governance/tag-keys/{key}"),
        ] {
            assert_eq!(
                policy_for(method, path),
                Some(Policy::RequiresPermission("governance:write")),
                "{method} {path}"
            );
        }
    }

    /// `DATA-14` (F2): listing quality incidents needs only a login, like the
    /// quality list; resolving one needs `governance:write`.
    #[test]
    fn quality_incidents_are_listed_by_any_login_and_resolved_with_governance_write() {
        assert_eq!(
            policy_for("GET", "/api/governance/quality/incidents"),
            Some(Policy::RequiresAuth)
        );
        assert_eq!(
            policy_for("POST", "/api/governance/quality/incidents/{id}/resolve"),
            Some(Policy::RequiresPermission("governance:write"))
        );
    }

    /// `DATA-14` (F3): reading schedules needs only a login; setting one needs
    /// `governance:write`.
    #[test]
    fn quality_schedules_are_read_by_any_login_and_set_with_governance_write() {
        assert_eq!(
            policy_for("GET", "/api/governance/quality/schedules"),
            Some(Policy::RequiresAuth)
        );
        assert_eq!(
            policy_for("PUT", "/api/governance/quality/schedules/{asset}"),
            Some(Policy::RequiresPermission("governance:write"))
        );
    }

    /// `DATA-14` (F7): a rule's run history is readable by any login.
    #[test]
    fn a_rules_run_history_is_readable_by_any_login() {
        assert_eq!(
            policy_for("GET", "/api/governance/quality/{id}/runs"),
            Some(Policy::RequiresAuth)
        );
    }

    /// `DATA-14` (F5): the learning state is readable by any login.
    #[test]
    fn the_learning_state_is_readable_by_any_login() {
        assert_eq!(
            policy_for("GET", "/api/governance/quality/learning/{asset}"),
            Some(Policy::RequiresAuth)
        );
    }

    #[test]
    fn health_is_public() {
        assert_eq!(policy_for("GET", "/health"), Some(Policy::Public));
    }

    #[test]
    fn embed_data_is_public() {
        assert_eq!(policy_for("POST", "/api/embed/data"), Some(Policy::Public));
    }

    #[test]
    fn public_dashboard_is_public() {
        assert_eq!(
            policy_for("GET", "/api/public/dashboard/{token}"),
            Some(Policy::Public)
        );
    }

    #[test]
    fn login_is_public() {
        assert_eq!(policy_for("POST", "/api/auth/login"), Some(Policy::Public));
    }

    #[test]
    fn exactly_eleven_public_entries_exist() {
        // Pre-existing drift found while landing the `/api/auth/providers`
        // route: this assertion was still pinned at 4 even though the two
        // earlier OIDC public routes (`/api/auth/oidc/start`,
        // `/api/auth/oidc/callback`) had already grown POLICY_TABLE's
        // `Policy::Public` block to 6 entries (this test was failing —
        // `left: 6, right: 4` — before this commit's `/api/auth/providers`
        // addition made it `left: 7`). Fixed here rather than left broken,
        // per AGENTS.md rule 2 ("never weaken a test... if a test is
        // wrong, fix it and say why").
        let public_count = POLICY_TABLE
            .iter()
            .filter(|(_, _, policy)| *policy == Policy::Public)
            .count();
        // SEC-12 added `POST /api/embed/frame`: 7 -> 8.
        // BI-1 (D3) added `POST /api/render/data`, the PDF print view's data
        // route, authenticated by its signed render token: 8 -> 9.
        // BI-19 part A (D4) added `POST /api/public/dashboard/{token}/unlock`,
        // where a public link's password is traded for a ticket: 9 -> 10.
        // BI-26 (D5) added `POST /api/embed/values`, the dropdown of an
        // editable embed filter, authenticated by the signed token: 10 -> 11.
        assert_eq!(public_count, 11);
    }

    /// SEC-23 (F1): adding a governance rule needs `governance:write`, the
    /// same grant as changing or removing one; listing rules needs a login.
    #[test]
    fn adding_a_governance_rule_needs_governance_write_but_listing_needs_a_login() {
        assert_eq!(
            policy_for("POST", "/api/governance/{kind}"),
            Some(Policy::RequiresPermission("governance:write"))
        );
        assert_eq!(
            policy_for("GET", "/api/governance/{kind}"),
            Some(Policy::RequiresAuth)
        );
    }

    #[test]
    fn an_unregistered_route_has_no_policy() {
        assert_eq!(policy_for("GET", "/api/does-not-exist"), None);
    }

    #[test]
    fn catalog_requires_the_seeded_permission() {
        assert_eq!(
            policy_for("GET", "/api/catalog"),
            Some(Policy::RequiresPermission("catalog:read"))
        );
    }

    /// D1 regression: `/api/identity/*` must be permission-gated, not
    /// merely `RequiresAuth` — see the module doc comment.
    #[test]
    fn identity_routes_require_identity_permissions() {
        assert_eq!(
            policy_for("GET", "/api/identity/roles"),
            Some(Policy::RequiresPermission("identity:read"))
        );
        assert_eq!(
            policy_for("POST", "/api/identity/roles"),
            Some(Policy::RequiresPermission("identity:write"))
        );
        assert_eq!(
            policy_for("POST", "/api/identity/users"),
            Some(Policy::RequiresPermission("identity:write"))
        );
        assert_eq!(
            policy_for("POST", "/api/identity/service-identities"),
            Some(Policy::RequiresPermission("identity:write"))
        );
    }

    /// D1: proves the new `identity:read`/`identity:write` tokens need no
    /// seed-data migration — every currently-seeded role's grants (per
    /// `lakehouse_auth::permissions`'s module doc comment) either matches
    /// neither, or (Platform Admin's `"*:*"`) matches both.
    #[test]
    fn identity_permissions_require_no_seed_change() {
        use lakehouse_auth::PermissionSet;

        let non_admin_roles = [
            ("Analyst", "query:read, catalog:read, lineage:read"),
            ("Approver", "agent:approve, policy:review"),
            ("Governance Admin", "policy:*, residency:*, audit:read"),
            (
                "Data Engineer",
                "pipeline:*, catalog:write, connector:manage",
            ),
            ("Data Scientist", "query:read, feature:write, notebook:run"),
            ("Dashboard Viewer", "dashboard:read"),
        ];
        for (name, raw) in non_admin_roles {
            let set = PermissionSet::parse(raw);
            assert!(
                !set.has("identity:read") && !set.has("identity:write"),
                "{name} unexpectedly grants an identity permission"
            );
        }

        let platform_admin = PermissionSet::parse("*:*");
        assert!(platform_admin.has("identity:read"));
        assert!(platform_admin.has("identity:write"));
    }

    /// D1: the same 403-vs-200 distinction `auth_gate` enforces
    /// (`Principal::has` on the route's `RequiresPermission` string),
    /// exercised directly against an Analyst-shaped and a Platform-Admin-
    /// shaped permission set — an Analyst must be denied
    /// `POST /api/identity/roles` (privilege escalation: minting a `*:*`
    /// role) while a `*:*` principal is let through.
    #[test]
    fn analyst_is_denied_identity_write_platform_admin_is_allowed() {
        use lakehouse_auth::PermissionSet;

        let Some(Policy::RequiresPermission(required)) = policy_for("POST", "/api/identity/roles")
        else {
            panic!("POST /api/identity/roles must require a permission");
        };

        let analyst = PermissionSet::parse("query:read, catalog:read, lineage:read");
        assert!(
            !analyst.has(required),
            "an Analyst must be denied (403) POST /api/identity/roles"
        );

        let platform_admin = PermissionSet::parse("*:*");
        assert!(
            platform_admin.has(required),
            "a Platform Admin (*:*) must be allowed through POST /api/identity/roles"
        );
    }

    /// `BI-9`: `settings:write` follows the rules for introducing a permission
    /// string at the top of this file: it is satisfied by Platform Admin's
    /// `*:*` and by none of the seeded roles' grants, so it needs no seed
    /// migration, and the read route stays open to any signed-in caller.
    #[test]
    fn settings_write_needs_no_seed_change_and_no_seeded_role_has_it() {
        use lakehouse_auth::PermissionSet;

        let non_admin_roles = [
            ("Analyst", "query:read, catalog:read, lineage:read"),
            ("Approver", "agent:approve, policy:review"),
            ("Governance Admin", "policy:*, residency:*, audit:read"),
            (
                "Data Engineer",
                "pipeline:*, catalog:write, connector:manage",
            ),
            ("Data Scientist", "query:read, feature:write, notebook:run"),
            ("Dashboard Viewer", "dashboard:read"),
        ];
        for (name, raw) in non_admin_roles {
            assert!(
                !PermissionSet::parse(raw).has("settings:write"),
                "{name} unexpectedly grants settings:write"
            );
        }
        // Holding `dashboard:write` is not enough either.
        assert_denied_then_allowed(
            "PUT",
            "/api/settings/reporting",
            &PermissionSet::parse("dashboard:write, dashboard:read"),
            "an editor with dashboard:write",
            &PermissionSet::parse("*:*"),
            "a Platform Admin (*:*)",
        );
        assert_eq!(
            policy_for("GET", "/api/settings/reporting"),
            Some(Policy::RequiresAuth)
        );
    }

    /// `BI-2`: `metric:write` follows the same rules: no seeded role holds it,
    /// `dashboard:write` is not enough, Platform Admin's `*:*` is, and the read
    /// routes need only `dashboard:read`.
    #[test]
    fn metric_write_needs_no_seed_change_and_no_seeded_role_has_it() {
        use lakehouse_auth::PermissionSet;

        let non_admin_roles = [
            ("Analyst", "query:read, catalog:read, lineage:read"),
            ("Approver", "agent:approve, policy:review"),
            ("Governance Admin", "policy:*, residency:*, audit:read"),
            (
                "Data Engineer",
                "pipeline:*, catalog:write, connector:manage",
            ),
            ("Data Scientist", "query:read, feature:write, notebook:run"),
            ("Dashboard Viewer", "dashboard:read"),
        ];
        for (name, raw) in non_admin_roles {
            assert!(
                !PermissionSet::parse(raw).has("metric:write"),
                "{name} unexpectedly grants metric:write"
            );
        }
        for (method, path) in [
            ("POST", "/api/dashboard/metrics"),
            ("PUT", "/api/dashboard/metrics"),
            ("DELETE", "/api/dashboard/metrics"),
            ("POST", "/api/dashboard/segments"),
            ("PUT", "/api/dashboard/segments"),
            ("DELETE", "/api/dashboard/segments"),
        ] {
            assert_denied_then_allowed(
                method,
                path,
                &PermissionSet::parse("dashboard:write, dashboard:read"),
                "an editor with dashboard:write",
                &PermissionSet::parse("*:*"),
                "a Platform Admin (*:*)",
            );
        }
        for (method, path) in [
            ("POST", "/api/dashboard/metrics/validate"),
            ("GET", "/api/dashboard/metrics/{id}/usage"),
        ] {
            assert_eq!(
                policy_for(method, path),
                Some(Policy::RequiresPermission("dashboard:read")),
                "{method} {path}"
            );
        }
        // BI-19 part B1 (plan D8): listing metrics and segments moved to
        // "signed in" with an entry guard in the handler, so somebody given one
        // dashboard can still use the metrics its charts name.
        for (method, path) in [
            ("GET", "/api/dashboard/metrics"),
            ("GET", "/api/dashboard/segments"),
        ] {
            assert_eq!(
                policy_for(method, path),
                Some(Policy::RequiresAuth),
                "{method} {path}"
            );
        }
    }

    /// Shared shape for the four new-in-this-task route groups: a
    /// `PermissionSet` WITHOUT the route's required permission must be
    /// denied (this is exactly `auth_gate`'s 403 branch —
    /// `!principal.has(permission)`), and the intended role's
    /// `PermissionSet` must be allowed. Mirrors
    /// `analyst_is_denied_identity_write_platform_admin_is_allowed` above.
    fn assert_denied_then_allowed(
        method: &str,
        path: &str,
        denied: &lakehouse_auth::PermissionSet,
        denied_label: &str,
        allowed: &lakehouse_auth::PermissionSet,
        allowed_label: &str,
    ) {
        let Some(Policy::RequiresPermission(required)) = policy_for(method, path) else {
            panic!("{method} {path} must require a permission");
        };
        assert!(
            !denied.has(required),
            "{denied_label} must be denied (403) {method} {path}, required {required}"
        );
        assert!(
            allowed.has(required),
            "{allowed_label} must be allowed through {method} {path}, required {required}"
        );
    }

    /// #1: digital-employee lifecycle control (`agent:manage`) is
    /// deliberately Platform-Admin-only — an Analyst (and even Approver,
    /// whose `agent:approve` is a narrower per-decision grant) must be
    /// denied; only `*:*` gets through.
    #[test]
    fn agent_lifecycle_routes_deny_analyst_and_approver_allow_platform_admin() {
        use lakehouse_auth::PermissionSet;

        for path in [
            "/api/agents/employees/{id}/suspend",
            "/api/agents/employees/{id}/resume",
            "/api/agents/employees/{id}/revoke",
        ] {
            assert_denied_then_allowed(
                "POST",
                path,
                &PermissionSet::parse("query:read, catalog:read, lineage:read"),
                "an Analyst",
                &PermissionSet::parse("*:*"),
                "a Platform Admin (*:*)",
            );
            assert_denied_then_allowed(
                "POST",
                path,
                &PermissionSet::parse("agent:approve, policy:review"),
                "an Approver (agent:approve only)",
                &PermissionSet::parse("*:*"),
                "a Platform Admin (*:*)",
            );
        }
    }

    /// #2: cancelling a workload (`workload:cancel`) is granted to Data
    /// Engineer (0020) — an Analyst must be denied, Data Engineer allowed.
    #[test]
    fn workload_cancel_denies_analyst_allows_data_engineer() {
        use lakehouse_auth::PermissionSet;

        assert_denied_then_allowed(
            "POST",
            "/api/ops/workloads/{id}/cancel",
            &PermissionSet::parse("query:read, catalog:read, lineage:read"),
            "an Analyst",
            &PermissionSet::parse("pipeline:*, catalog:write, connector:manage, workload:cancel"),
            "a Data Engineer",
        );
    }

    /// #3: storage restore (`storage:restore`) is deliberately
    /// Platform-Admin-only — Data Engineer, despite operating pipelines and
    /// connectors, must still be denied; only `*:*` gets through.
    #[test]
    fn storage_restore_denies_data_engineer_allows_platform_admin() {
        use lakehouse_auth::PermissionSet;

        assert_denied_then_allowed(
            "POST",
            "/api/storage/restore",
            &PermissionSet::parse("pipeline:*, catalog:write, connector:manage"),
            "a Data Engineer",
            &PermissionSet::parse("*:*"),
            "a Platform Admin (*:*)",
        );
    }

    /// #4: alert rule CRUD (`alert:write`) is granted to Data Engineer
    /// (0020) — an Analyst must be denied on all three mutating methods,
    /// Data Engineer allowed.
    #[test]
    fn alert_crud_denies_analyst_allows_data_engineer() {
        use lakehouse_auth::PermissionSet;

        for method in ["POST", "PUT", "DELETE"] {
            assert_denied_then_allowed(
                method,
                "/api/alerts",
                &PermissionSet::parse("query:read, catalog:read, lineage:read"),
                "an Analyst",
                &PermissionSet::parse("pipeline:*, catalog:write, connector:manage, alert:write"),
                "a Data Engineer",
            );
        }
    }

    /// Proves `0020_extend_role_grants.sql`'s intent at the `PermissionSet`
    /// level (mirrors `identity_permissions_require_no_seed_change` above):
    /// `agent:manage` and `storage:restore` are satisfied by NO non-admin
    /// seeded role (deliberately Platform-Admin-only), while
    /// `workload:cancel` and `alert:write` are satisfied by Data Engineer
    /// specifically (once the roles it's granted are read post-migration)
    /// and by no other non-admin role.
    #[test]
    fn agent_workload_storage_alert_permissions_seed_alignment() {
        use lakehouse_auth::PermissionSet;

        let non_admin_roles = [
            ("Analyst", "query:read, catalog:read, lineage:read"),
            ("Approver", "agent:approve, policy:review"),
            ("Governance Admin", "policy:*, residency:*, audit:read"),
            ("Data Scientist", "query:read, feature:write, notebook:run"),
            ("Dashboard Viewer", "dashboard:read"),
        ];
        for (name, raw) in non_admin_roles {
            let set = PermissionSet::parse(raw);
            assert!(
                !set.has("agent:manage"),
                "{name} unexpectedly grants agent:manage"
            );
            assert!(
                !set.has("workload:cancel"),
                "{name} unexpectedly grants workload:cancel"
            );
            assert!(
                !set.has("storage:restore"),
                "{name} unexpectedly grants storage:restore"
            );
            assert!(
                !set.has("alert:write"),
                "{name} unexpectedly grants alert:write"
            );
        }

        // Data Engineer, post-0020: gains workload:cancel and alert:write,
        // but never the two deliberately admin-only tokens.
        let data_engineer = PermissionSet::parse(
            "pipeline:*, catalog:write, connector:manage, workload:cancel, alert:write",
        );
        assert!(data_engineer.has("workload:cancel"));
        assert!(data_engineer.has("alert:write"));
        assert!(!data_engineer.has("agent:manage"));
        assert!(!data_engineer.has("storage:restore"));

        let platform_admin = PermissionSet::parse("*:*");
        assert!(platform_admin.has("agent:manage"));
        assert!(platform_admin.has("workload:cancel"));
        assert!(platform_admin.has("storage:restore"));
        assert!(platform_admin.has("alert:write"));
    }

    #[test]
    fn no_duplicate_method_path_entries() {
        let mut seen = std::collections::HashSet::new();
        for (method, path, _) in POLICY_TABLE {
            assert!(
                seen.insert((*method, *path)),
                "duplicate policy entry for {method} {path}"
            );
        }
    }
}
