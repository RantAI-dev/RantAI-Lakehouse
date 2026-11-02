//! ADM-4 T1: an administrator changes what a user may do — a user's role
//! set (D1) and a user's tenant membership (D2) — under the escalation
//! rules of D5, and every change is audited (D6).
//!
//! Before this, roles and tenants could be granted only at invite time
//! (`POST /api/identity/users`) and a tenant could be added afterwards
//! (`PUT /api/identity/users/{id}/tenants/{tenant_id}`), but nothing
//! could take a role away or take a tenant away.
//!
//! The tests live in a module named `identity` so the plan's
//! `cargo test -p lakehouse-api identity` filter runs them: an
//! integration test's name is its function's path inside the test
//! binary, and a bare function name would never match that filter.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use common::{TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up};

mod identity {
    use super::*;

    /// The seeded `Meridian Group` tenant (`0002_seed_identity.sql`).
    const MERIDIAN_GROUP: &str = "11111111-1111-4111-8111-000000000001";

    /// The seeded `Meridian Retail` tenant (`0002_seed_identity.sql`).
    const MERIDIAN_RETAIL: &str = "11111111-1111-4111-8111-000000000002";

    /// One audit row, as the audit test reads it: the action, the
    /// resource it names, the principal that acted, and its arguments.
    type AuditRow = (String, Option<String>, Option<String>, Option<Value>);

    // ── helpers ──────────────────────────────────────────────────────────

    /// Send a request with an optional session cookie and an optional
    /// JSON body; return the status and the parsed body.
    async fn request(
        app: &TestApp,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let payload = body.map_or_else(String::new, |value| value.to_string());
        let response = app
            .router
            .clone()
            .oneshot(builder.body(Body::from(payload)).expect("build request"))
            .await
            .expect("router never fails a request outright");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// `PUT /api/identity/users/{id}/roles` with the given role names.
    async fn set_roles(
        app: &TestApp,
        cookie: &str,
        user: Uuid,
        roles: &[&str],
    ) -> (StatusCode, Value) {
        request(
            app,
            "PUT",
            &format!("/api/identity/users/{user}/roles"),
            Some(cookie),
            Some(json!({ "roles": roles })),
        )
        .await
    }

    /// `DELETE /api/identity/users/{id}/tenants/{tenant_id}`.
    async fn remove_from_tenant(
        app: &TestApp,
        cookie: &str,
        user: Uuid,
        tenant: &str,
    ) -> (StatusCode, Value) {
        request(
            app,
            "DELETE",
            &format!("/api/identity/users/{user}/tenants/{tenant}"),
            Some(cookie),
            None,
        )
        .await
    }

    /// The role names a user holds, read straight from the database (not
    /// through the route), so a test can tell "the route answered with
    /// this" from "the route answered with this AND the database agrees".
    async fn stored_roles(pool: &PgPool, user: Uuid) -> Vec<String> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT r.name FROM app_user_role ur JOIN role r ON r.id = ur.role_id \
             WHERE ur.user_id = $1 ORDER BY r.name",
        )
        .bind(user)
        .fetch_all(pool)
        .await
        .expect("read the user's role memberships");
        rows.into_iter().map(|(name,)| name).collect()
    }

    /// Leave the deployment with exactly ONE user holding the unrestricted
    /// `*:*` grant: the seeded Platform Admin (`fajar@meridian.example`)
    /// gives theirs up.
    ///
    /// Needed by the "the last administrator cannot be demoted" test: with
    /// the seeded admin still in place, there would always be a second
    /// administrator and the rule could never be reached.
    async fn leave_one_administrator(pool: &PgPool) {
        sqlx::query(
            "DELETE FROM app_user_role \
             WHERE role_id = (SELECT id FROM role WHERE name = 'Platform Admin')",
        )
        .execute(pool)
        .await
        .expect("the seeded Platform Admin gives up the unrestricted role");
    }

    // ── the tests ────────────────────────────────────────────────────────

    /// ADM-4 D1: the route REPLACES the user's local role set with exactly
    /// the names the request carries, and the database agrees.
    #[tokio::test]
    async fn setting_the_roles_replaces_the_users_role_set() {
        let app = spin_up().await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;
        let target = create_principal_with_permissions(&app.pool, "query:read").await;
        assert_eq!(stored_roles(&app.pool, target).await.len(), 1);

        let (status, body) = set_roles(&app, &admin_cookie, target, &["Analyst", "Approver"]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["roles"], json!(["Analyst", "Approver"]));

        let stored = stored_roles(&app.pool, target).await;
        assert_eq!(stored, vec!["Analyst".to_owned(), "Approver".to_owned()]);

        // A second call replaces the set again — it does not add to it.
        let (status, body) = set_roles(&app, &admin_cookie, target, &["Data Engineer"]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["roles"], json!(["Data Engineer"]));
        assert_eq!(
            stored_roles(&app.pool, target).await,
            vec!["Data Engineer".to_owned()]
        );
    }

    /// ADM-4 D1: an unknown role name is a 400, and the user's roles are
    /// left exactly as they were.
    #[tokio::test]
    async fn an_unknown_role_name_is_a_400_and_changes_nothing() {
        let app = spin_up().await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;
        let target = create_principal_with_permissions(&app.pool, "query:read").await;
        let before = stored_roles(&app.pool, target).await;

        let (status, body) = set_roles(&app, &admin_cookie, target, &["No Such Role"]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"]
                .as_str()
                .unwrap_or_default()
                .contains("No Such Role"),
            "the refusal names the role that does not exist: {body}"
        );
        assert_eq!(
            stored_roles(&app.pool, target).await,
            before,
            "a refused change must leave the roles alone"
        );
    }

    /// ADM-4 D5: only an unrestricted caller may GRANT a role that holds
    /// `*:*`. Anyone else gets the same 404 an unknown user gets, body for
    /// body, so a probe learns nothing.
    #[tokio::test]
    async fn a_caller_without_the_unrestricted_grant_gets_404_for_granting_platform_admin() {
        let app = spin_up().await;
        let caller = create_principal_with_permissions(&app.pool, "identity:write").await;
        let caller_cookie = session_cookie_for_user(&app.pool, caller).await;
        let target = create_principal_with_permissions(&app.pool, "query:read").await;
        let before = stored_roles(&app.pool, target).await;

        let (status, body) = set_roles(&app, &caller_cookie, target, &["Platform Admin"]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

        // The same answer as for a user id that does not exist at all.
        let (unknown_status, unknown_body) = request(
            &app,
            "PUT",
            &format!("/api/identity/users/{}/roles", Uuid::new_v4()),
            Some(&caller_cookie),
            Some(json!({ "roles": ["Platform Admin"] })),
        )
        .await;
        assert_eq!(unknown_status, StatusCode::NOT_FOUND, "{unknown_body}");
        assert_eq!(
            body, unknown_body,
            "no oracle for which accounts hold the unrestricted grant"
        );

        assert_eq!(
            stored_roles(&app.pool, target).await,
            before,
            "a refused change must leave the roles alone"
        );
    }

    /// ADM-4 D5, the other side of the same rule: only an unrestricted
    /// caller may change the roles of a user who HOLDS `*:*`. The rule is
    /// about the TARGET here, not about the role being granted.
    #[tokio::test]
    async fn a_caller_without_the_unrestricted_grant_gets_404_for_an_unrestricted_user() {
        let app = spin_up().await;
        let caller = create_principal_with_permissions(&app.pool, "identity:write").await;
        let caller_cookie = session_cookie_for_user(&app.pool, caller).await;
        let target = create_principal_with_permissions(&app.pool, "*:*").await;

        let (status, body) = set_roles(&app, &caller_cookie, target, &["Analyst"]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(
            stored_roles(&app.pool, target).await.len(),
            1,
            "the unrestricted user keeps its role"
        );
    }

    /// ADM-4 D5: a change that would leave NO active user holding `*:*`
    /// is refused with a 409, and the change is rolled back with it.
    ///
    /// The deployment is left with exactly one user holding `*:*` — the
    /// one making the call — so the change it asks for is the one that
    /// would leave nobody.
    #[tokio::test]
    async fn the_last_unrestricted_user_cannot_lose_the_unrestricted_grant() {
        let app = spin_up().await;
        leave_one_administrator(&app.pool).await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        // The deployment now has exactly one unrestricted user: `admin`.
        let (status, body) = set_roles(&app, &admin_cookie, admin, &["Analyst"]).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(
            stored_roles(&app.pool, admin).await.len(),
            1,
            "the refused change must be rolled back"
        );
    }

    /// ADM-4 D5: a caller may not remove their OWN unrestricted grant, even
    /// when the deployment would still have an administrator afterwards.
    /// The refusal is a 400 about the request — the caller is the target,
    /// so it is not a "no such record" answer.
    #[tokio::test]
    async fn a_caller_may_not_remove_their_own_unrestricted_grant() {
        let app = spin_up().await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;
        // A second administrator, so the deployment-wide rule (409) does
        // not apply and the self-rule does.
        let _second = create_principal_with_permissions(&app.pool, "*:*").await;

        let (status, body) = set_roles(&app, &admin_cookie, admin, &["Analyst"]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            stored_roles(&app.pool, admin).await.len(),
            1,
            "the caller keeps the unrestricted role"
        );
    }

    /// ADM-4 D2: a user can be taken out of a tenant, and the membership
    /// really goes away.
    #[tokio::test]
    async fn removing_a_user_from_a_tenant_takes_the_membership_away() {
        let app = spin_up().await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;
        let target = create_principal_with_permissions(&app.pool, "query:read").await;
        sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
            .bind(target)
            .bind(uuid::Uuid::parse_str(MERIDIAN_GROUP).unwrap())
            .execute(&app.pool)
            .await
            .expect("put the target in a tenant first");

        let (status, body) = remove_from_tenant(&app, &admin_cookie, target, MERIDIAN_GROUP).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body["tenants"]
                .as_array()
                .is_some_and(|tenants| !tenants.iter().any(|t| t == MERIDIAN_GROUP)),
            "the tenant is gone from the user's tenants: {body}"
        );

        let memberships: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM app_user_tenant WHERE user_id = $1 AND tenant_id = $2",
        )
        .bind(target)
        .bind(uuid::Uuid::parse_str(MERIDIAN_GROUP).unwrap())
        .fetch_one(&app.pool)
        .await
        .expect("count the memberships");
        assert_eq!(memberships, 0, "the membership row is gone");
    }

    /// ADM-4 D2: the "who may" rule is the add's rule — an unrestricted
    /// caller, or a member of that tenant. Anyone else gets the same 404
    /// as for a tenant that does not exist.
    #[tokio::test]
    async fn removing_a_user_from_a_tenant_needs_the_caller_to_be_allowed_there() {
        let app = spin_up().await;
        let caller = create_principal_with_permissions(&app.pool, "identity:write").await;
        let caller_cookie = session_cookie_for_user(&app.pool, caller).await;
        let target = create_principal_with_permissions(&app.pool, "query:read").await;

        let (status, body) = remove_from_tenant(&app, &caller_cookie, target, MERIDIAN_GROUP).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

        // The same answer as for a tenant that does not exist.
        let (unknown_status, unknown_body) = remove_from_tenant(
            &app,
            &caller_cookie,
            target,
            "99999999-9999-4999-8999-000000000099",
        )
        .await;
        assert_eq!(unknown_status, StatusCode::NOT_FOUND, "{unknown_body}");
        assert_eq!(body, unknown_body, "one body for both");
    }

    /// ADM-4 D6: both changes are audited, naming the actor, the target,
    /// and — for a role change — the role names before and after.
    #[tokio::test]
    async fn the_changes_are_audited_with_the_actor_the_target_and_the_roles() {
        let app = spin_up().await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;
        let target = create_principal_with_permissions(&app.pool, "query:read").await;
        sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
            .bind(target)
            .bind(uuid::Uuid::parse_str(MERIDIAN_RETAIL).unwrap())
            .execute(&app.pool)
            .await
            .expect("put the target in a tenant first");

        let (status, body) = set_roles(&app, &admin_cookie, target, &["Analyst"]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) = remove_from_tenant(&app, &admin_cookie, target, MERIDIAN_RETAIL).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let rows: Vec<AuditRow> = sqlx::query_as(
            "SELECT action, resource_id, principal_id, args FROM audit_event \
             WHERE action IN ('identity.user.roles.set', 'identity.user.tenant.remove') \
             ORDER BY at",
        )
        .fetch_all(&app.pool)
        .await
        .expect("read the audit rows");
        assert_eq!(rows.len(), 2, "one row per change: {rows:?}");

        let roles_row = rows
            .iter()
            .find(|(action, _, _, _)| action == "identity.user.roles.set")
            .expect("the role change is audited");
        assert_eq!(roles_row.1.as_deref(), Some(target.to_string().as_str()));
        assert_eq!(roles_row.2.as_deref(), Some(admin.to_string().as_str()));
        let args = roles_row.3.as_ref().expect("the role change carries args");
        let old_roles = args["oldRoles"]
            .as_array()
            .expect("the audit names the roles the user held");
        assert_eq!(old_roles.len(), 1, "one role before the change: {args}");
        assert_eq!(args["newRoles"], json!(["Analyst"]), "new: {args}");

        let tenant_row = rows
            .iter()
            .find(|(action, _, _, _)| action == "identity.user.tenant.remove")
            .expect("the tenant removal is audited");
        assert_eq!(tenant_row.1.as_deref(), Some(target.to_string().as_str()));
        assert_eq!(tenant_row.2.as_deref(), Some(admin.to_string().as_str()));
    }
}
