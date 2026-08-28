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
    // ── Public: the ONLY four routes in the whole service. ──────────────
    ("GET",  "/health",                          Policy::Public),
    ("POST", "/api/embed/data",                   Policy::Public),
    ("GET",  "/api/public/dashboard/{token}",     Policy::Public),
    ("POST", "/api/auth/login",                   Policy::Public),

    // ── Auth domain (this task) ──────────────────────────────────────────
    ("POST", "/api/auth/logout",                  Policy::RequiresAuth),
    ("GET",  "/api/auth/me",                      Policy::RequiresAuth),
    ("POST", "/api/auth/change-password",         Policy::RequiresAuth),

    // ── Catalog: seeded Analyst permission `catalog:read`. ───────────────
    ("GET", "/api/catalog",       Policy::RequiresPermission("catalog:read")),
    ("GET", "/api/catalog/{id}",  Policy::RequiresPermission("catalog:read")),

    // ── Overview / alerts: no seeded resource maps to these — auth only. ─
    ("GET",  "/api/overview",                                Policy::RequiresAuth),
    ("POST", "/api/overview",                                Policy::RequiresAuth),
    ("GET",  "/api/overview/alerts",                          Policy::RequiresAuth),
    ("POST", "/api/overview/alerts/{id}/acknowledge",         Policy::RequiresAuth),
    ("POST", "/api/overview/alerts/{id}/resolve",             Policy::RequiresAuth),

    // ── Ops: `{kind}` fans out to several logical resources with no single
    //    seeded permission — auth only (see module doc comment). ─────────
    ("GET",  "/api/ops/{kind}",                    Policy::RequiresAuth),
    ("POST", "/api/ops/workloads/{id}/cancel",     Policy::RequiresAuth),

    // ── Governance: seeded `policy:*` / `lineage:read`. `{kind}` (quality,
    //    classification, audit, residency) has the same fan-out problem as
    //    ops — auth only for that one. ────────────────────────────────────
    ("GET",  "/api/governance/lineage",   Policy::RequiresPermission("lineage:read")),
    ("GET",  "/api/governance/policies",  Policy::RequiresPermission("policy:read")),
    ("POST", "/api/governance/policies",  Policy::RequiresPermission("policy:write")),
    ("GET",  "/api/governance/{kind}",    Policy::RequiresAuth),
    ("POST", "/api/governance/{kind}",    Policy::RequiresAuth),

    // ── Storage: no seeded resource — auth only. ─────────────────────────
    ("GET",  "/api/storage",             Policy::RequiresAuth),
    ("GET",  "/api/storage/policies",    Policy::RequiresAuth),
    ("POST", "/api/storage/policies",    Policy::RequiresAuth),
    ("GET",  "/api/storage/operations",  Policy::RequiresAuth),
    ("POST", "/api/storage/restore",     Policy::RequiresAuth),

    // ── Alerts (CRUD + run trigger): no seeded resource — auth only. Note
    //    `/api/alerts/run` ALSO still runs its own, stricter
    //    `routes::alerts::check_run_token` guard inside the handler (D4):
    //    with `ALERTS_RUN_TOKEN` set, a matching token is required as
    //    before; with it unset, only a `PrincipalId::Service` principal is
    //    allowed through, not merely `RequiresAuth`'s "any authenticated
    //    principal" — this table entry is a floor, not the whole guard. ──
    ("GET",    "/api/alerts",      Policy::RequiresAuth),
    ("POST",   "/api/alerts",      Policy::RequiresAuth),
    ("PUT",    "/api/alerts",      Policy::RequiresAuth),
    ("DELETE", "/api/alerts",      Policy::RequiresAuth),
    ("GET",    "/api/alerts/run",  Policy::RequiresAuth),
    ("POST",   "/api/alerts/run",  Policy::RequiresAuth),

    // ── Query: seeded Analyst permission `query:read`. `collaboration` has
    //    no seeded resource — auth only. ──────────────────────────────────
    ("POST", "/api/query/run",            Policy::RequiresPermission("query:read")),
    ("POST", "/api/query/estimate",       Policy::RequiresPermission("query:read")),
    ("GET",  "/api/query/saved",          Policy::RequiresPermission("query:read")),
    ("GET",  "/api/query/history",        Policy::RequiresPermission("query:read")),
    ("GET",  "/api/query/collaboration",  Policy::RequiresAuth),
    ("POST", "/api/query/collaboration",  Policy::RequiresAuth),

    // ── Pipelines: seeded Data Engineer permission `pipeline:*`. ─────────
    ("GET",  "/api/pipelines",                        Policy::RequiresPermission("pipeline:read")),
    ("POST", "/api/pipelines",                        Policy::RequiresPermission("pipeline:write")),
    ("POST", "/api/pipelines/generate",               Policy::RequiresPermission("pipeline:write")),
    ("GET",  "/api/pipelines/{id}/runs",              Policy::RequiresPermission("pipeline:read")),
    ("POST", "/api/pipelines/{id}/trigger",           Policy::RequiresPermission("pipeline:write")),
    ("POST", "/api/pipelines/{id}/pause",             Policy::RequiresPermission("pipeline:write")),
    ("POST", "/api/pipelines/{id}/resume",            Policy::RequiresPermission("pipeline:write")),
    ("POST", "/api/pipelines/runs/{runId}/cancel",    Policy::RequiresPermission("pipeline:write")),
    ("POST", "/api/pipelines/runs/{runId}/retry",     Policy::RequiresPermission("pipeline:write")),

    // ── Dashboard: seeded Dashboard Viewer permission `dashboard:read`.
    //    `dashboard:write` is the natural write counterpart — see module
    //    doc comment for why it's grounded despite no role being granted
    //    it in the seed (Platform Admin's `*:*` still satisfies it). ──────
    ("GET",    "/api/dashboard",              Policy::RequiresPermission("dashboard:read")),
    ("GET",    "/api/dashboard/specs",        Policy::RequiresPermission("dashboard:read")),
    ("POST",   "/api/dashboard/specs",        Policy::RequiresPermission("dashboard:write")),
    ("PUT",    "/api/dashboard/specs",        Policy::RequiresPermission("dashboard:write")),
    ("DELETE", "/api/dashboard/specs",        Policy::RequiresPermission("dashboard:write")),
    ("GET",    "/api/dashboard/boards",       Policy::RequiresPermission("dashboard:read")),
    ("POST",   "/api/dashboard/boards",       Policy::RequiresPermission("dashboard:write")),
    ("PUT",    "/api/dashboard/boards",       Policy::RequiresPermission("dashboard:write")),
    ("DELETE", "/api/dashboard/boards",       Policy::RequiresPermission("dashboard:write")),
    ("GET",    "/api/dashboard/fields",       Policy::RequiresPermission("dashboard:read")),
    ("GET",    "/api/dashboard/records",      Policy::RequiresPermission("dashboard:read")),
    ("GET",    "/api/dashboard/values",       Policy::RequiresPermission("dashboard:read")),
    ("GET",    "/api/dashboard/export",       Policy::RequiresPermission("dashboard:read")),
    ("GET",    "/api/dashboard/embed-info",   Policy::RequiresPermission("dashboard:read")),

    // ── Agent / AI: no seeded resource for free-form ask/chat — auth only.
    ("POST", "/api/agent/ask",           Policy::RequiresAuth),
    ("POST", "/api/agent/query",         Policy::RequiresAuth),
    ("POST", "/api/agent/text-to-sql",   Policy::RequiresAuth),
    ("POST", "/api/ai/chat",             Policy::RequiresAuth),
    ("GET",    "/api/ai/sessions",       Policy::RequiresAuth),
    ("POST",   "/api/ai/sessions",       Policy::RequiresAuth),
    ("DELETE", "/api/ai/sessions",       Policy::RequiresAuth),
    ("GET",  "/api/ai/build-status",     Policy::RequiresAuth),

    // ── Identity (Phase 2 directory): permission-gated (D1 fix). Reads
    //    require `identity:read`, mutations (create user/role/tenant/
    //    service-identity — the last of which can mint a `*:*` role and
    //    attach it to a caller) require `identity:write`. Only Platform
    //    Admin's `*:*` grants either today; see the module doc comment. ───
    ("GET",  "/api/identity/users",                  Policy::RequiresPermission("identity:read")),
    ("POST", "/api/identity/users",                  Policy::RequiresPermission("identity:write")),
    ("GET",  "/api/identity/roles",                  Policy::RequiresPermission("identity:read")),
    ("POST", "/api/identity/roles",                  Policy::RequiresPermission("identity:write")),
    ("GET",  "/api/identity/tenants",                Policy::RequiresPermission("identity:read")),
    ("POST", "/api/identity/tenants",                Policy::RequiresPermission("identity:write")),
    ("GET",  "/api/identity/service-identities",     Policy::RequiresPermission("identity:read")),
    ("POST", "/api/identity/service-identities",     Policy::RequiresPermission("identity:write")),
    ("GET",  "/api/identity/workspace-settings",     Policy::RequiresPermission("identity:read")),

    // ── Connectors: seeded Data Engineer permission `connector:manage`. ──
    ("GET",  "/api/connectors",             Policy::RequiresPermission("connector:manage")),
    ("POST", "/api/connectors",             Policy::RequiresPermission("connector:manage")),
    ("GET",  "/api/connectors/{id}",        Policy::RequiresPermission("connector:manage")),
    ("POST", "/api/connectors/{id}/test",   Policy::RequiresPermission("connector:manage")),

    // ── Knowledge: no seeded resource — auth only. ───────────────────────
    ("GET",  "/api/knowledge/sources",       Policy::RequiresAuth),
    ("POST", "/api/knowledge/sources",       Policy::RequiresAuth),
    ("GET",  "/api/knowledge/vector-jobs",   Policy::RequiresAuth),
    ("POST", "/api/knowledge/vector-jobs",   Policy::RequiresAuth),

    // ── Agents (digital employees / workflows / tools / runs): no seeded
    //    resource for most of this domain — auth only. `approvals` maps to
    //    the seeded Approver permission `agent:approve`. ──────────────────
    ("GET",  "/api/agents/workflows",                Policy::RequiresAuth),
    ("POST", "/api/agents/workflows",                Policy::RequiresAuth),
    ("GET",  "/api/agents/employees",                Policy::RequiresAuth),
    ("POST", "/api/agents/employees",                Policy::RequiresAuth),
    ("GET",  "/api/agents/employees/{id}",           Policy::RequiresAuth),
    ("POST", "/api/agents/employees/{id}/suspend",   Policy::RequiresAuth),
    ("POST", "/api/agents/employees/{id}/resume",    Policy::RequiresAuth),
    ("POST", "/api/agents/employees/{id}/revoke",    Policy::RequiresAuth),
    ("GET",  "/api/agents/tools",                    Policy::RequiresAuth),
    ("POST", "/api/agents/tools",                    Policy::RequiresAuth),
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

/// The one middleware layered onto the whole router (`routes::router`).
/// Looks up [`POLICY_TABLE`] for the request's matched route and either
/// lets it through (`Policy::Public`), requires and checks a
/// [`crate::auth::AuthenticatedPrincipal`] (`Policy::RequiresAuth`/
/// `Policy::RequiresPermission`), or — if the route has no policy entry at
/// all — refuses it outright rather than defaulting to public.
///
/// # Errors
///
/// Returns a 500 `route_policy_unclassified` [`ApiRejection`] if the
/// matched route has no [`POLICY_TABLE`] entry, a 401 if
/// [`crate::auth::AuthenticatedPrincipal`] extraction fails, or a 403 if
/// the extracted principal lacks a required permission.
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
            if let Policy::RequiresPermission(permission) = policy {
                if !principal.has(permission) {
                    return Err(ApiError::PermissionDenied(permission.to_owned()).into());
                }
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
    fn exactly_four_public_entries_exist() {
        let public_count = POLICY_TABLE
            .iter()
            .filter(|(_, _, policy)| *policy == Policy::Public)
            .count();
        assert_eq!(public_count, 4);
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
