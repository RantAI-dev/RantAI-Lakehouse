//! `GOV-7` D7: the API that sets a role's read access per schema and table.
//!
//! An administrator reads a role's scope with `GET
//! /api/identity/roles/{id}/table-access` and replaces it with `PUT`. The
//! default is `"all"` (D1), a `*:*` role cannot be narrowed (D4), a name that
//! is not an identifier is refused, and the change is audited (`AC11`).
//!
//! The functions live in a module named `identity` so the plan's
//! `cargo test -p lakehouse-api identity` filter runs them: an integration
//! test's name is its function's path inside the test binary.

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

    /// The seeded built-in `Platform Admin` role (`0002_seed_identity.sql`),
    /// whose permissions are `*:*`.
    const ADMIN_ROLE: &str = "22222222-2222-4222-8222-000000000006";

    /// One audit row as the audit test reads it.
    type AuditRow = (String, Option<String>, Option<Value>);

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

    /// An `identity:read, identity:write` caller and its cookie.
    async fn administrator(app: &TestApp) -> String {
        let user =
            create_principal_with_permissions(&app.pool, "identity:read, identity:write").await;
        session_cookie_for_user(&app.pool, user).await
    }

    /// A fresh role with `permissions`, optionally already narrowed.
    async fn role(pool: &PgPool, permissions: &str) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO role (id, name, permissions, description) VALUES ($1, $2, $3, 'test')",
        )
        .bind(id)
        .bind(format!("GOV-7 role {id}"))
        .bind(permissions)
        .execute(pool)
        .await
        .expect("insert the role");
        id
    }

    async fn get(app: &TestApp, cookie: &str, role: Uuid) -> (StatusCode, Value) {
        request(
            app,
            "GET",
            &format!("/api/identity/roles/{role}/table-access"),
            Some(cookie),
            None,
        )
        .await
    }

    async fn put(app: &TestApp, cookie: &str, role: Uuid, body: Value) -> (StatusCode, Value) {
        request(
            app,
            "PUT",
            &format!("/api/identity/roles/{role}/table-access"),
            Some(cookie),
            Some(body),
        )
        .await
    }

    /// D1: every role starts on "All tables", so an upgrade changes nobody.
    #[tokio::test]
    async fn a_new_role_reads_every_table() {
        let app = spin_up().await;
        let cookie = administrator(&app).await;
        let role = role(&app.pool, "query:read").await;

        let (status, body) = get(&app, &cookie, role).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["scope"], json!("all"), "{body}");
        assert_eq!(body["grants"], json!([]), "{body}");
    }

    /// D7: the scope and the grants are written together, and read back.
    #[tokio::test]
    async fn narrowing_a_role_stores_its_schemas_and_tables() {
        let app = spin_up().await;
        let cookie = administrator(&app).await;
        let role = role(&app.pool, "query:read").await;

        let (status, body) = put(
            &app,
            &cookie,
            role,
            json!({
                "scope": "listed",
                "grants": [
                    {"schema": "finance", "table": "*"},
                    {"schema": "silver", "table": "orders"},
                    {"schema": "silver", "table": "orders"},
                ],
            }),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["scope"], json!("listed"), "{body}");
        assert_eq!(
            body["grants"],
            json!([
                {"schema": "finance", "table": "*"},
                {"schema": "silver", "table": "orders"},
            ]),
            "the duplicate is stored once, sorted: {body}"
        );
        let (_, read_back) = get(&app, &cookie, role).await;
        assert_eq!(read_back, body, "the read answers what was written");
    }

    /// Going back to "All tables" stores no grants: stale rows would come
    /// back the next time somebody narrows the role.
    #[tokio::test]
    async fn going_back_to_all_tables_clears_the_grants() {
        let app = spin_up().await;
        let cookie = administrator(&app).await;
        let role = role(&app.pool, "query:read").await;
        put(
            &app,
            &cookie,
            role,
            json!({"scope": "listed", "grants": [{"schema": "finance", "table": "*"}]}),
        )
        .await;

        let (status, body) = put(
            &app,
            &cookie,
            role,
            json!({"scope": "all", "grants": [{"schema": "finance", "table": "*"}]}),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["scope"], json!("all"), "{body}");
        assert_eq!(body["grants"], json!([]), "{body}");
        let stored: i64 =
            sqlx::query_scalar("SELECT count(*) FROM role_read_grant WHERE role_id = $1")
                .bind(role)
                .fetch_one(&app.pool)
                .await
                .expect("count the grants");
        assert_eq!(stored, 0, "an 'all' role stores no grants");
    }

    /// `GOV-7-AC8` / D4: a `*:*` role cannot be narrowed.
    #[tokio::test]
    async fn a_full_access_role_cannot_be_narrowed() {
        let app = spin_up().await;
        let cookie = administrator(&app).await;

        let (status, body) = put(
            &app,
            &cookie,
            Uuid::parse_str(ADMIN_ROLE).expect("the seeded id"),
            json!({"scope": "listed", "grants": [{"schema": "finance", "table": "*"}]}),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            body["error"],
            json!("A role with full access cannot be narrowed"),
            "{body}"
        );
    }

    /// A name the rewrite could not bind a statement to is a 400, not a grant
    /// that silently never matches.
    #[tokio::test]
    async fn a_bad_identifier_is_refused() {
        let app = spin_up().await;
        let cookie = administrator(&app).await;
        let role = role(&app.pool, "query:read").await;

        for grant in [
            json!({"schema": "finance; drop", "table": "*"}),
            json!({"schema": "finance", "table": "orders where 1=1"}),
            json!({"schema": "", "table": "*"}),
            json!({"schema": "finance", "table": "1orders"}),
        ] {
            let (status, body) = put(
                &app,
                &cookie,
                role,
                json!({"scope": "listed", "grants": [grant]}),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{grant}: {body}");
        }
        // Nothing was written by any of the refusals.
        let scope: String = sqlx::query_scalar("SELECT read_scope FROM role WHERE id = $1")
            .bind(role)
            .fetch_one(&app.pool)
            .await
            .expect("read the scope");
        assert_eq!(scope, "all");
    }

    /// An unknown scope is a 400, and an unknown role is the 404 every role
    /// route answers.
    #[tokio::test]
    async fn an_unknown_scope_or_role_is_refused() {
        let app = spin_up().await;
        let cookie = administrator(&app).await;
        let role = role(&app.pool, "query:read").await;

        let (status, body) = put(&app, &cookie, role, json!({"scope": "some", "grants": []})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

        let unknown = Uuid::new_v4();
        let (status, body) = get(&app, &cookie, unknown).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        let (status, body) = put(&app, &cookie, unknown, json!({"scope": "all"})).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    }

    /// `GOV-7-AC11`: the audit names the role and the scope and grants before
    /// and after.
    #[tokio::test]
    async fn the_change_is_audited_with_before_and_after() {
        let app = spin_up().await;
        let cookie = administrator(&app).await;
        let role = role(&app.pool, "query:read").await;
        put(
            &app,
            &cookie,
            role,
            json!({"scope": "listed", "grants": [{"schema": "finance", "table": "orders"}]}),
        )
        .await;

        let rows: Vec<AuditRow> = sqlx::query_as(
            "SELECT action, resource_id, args FROM audit_event \
             WHERE action = 'identity.role.table_access.update'",
        )
        .fetch_all(&app.pool)
        .await
        .expect("read the audit rows");
        assert_eq!(rows.len(), 1, "one row per change: {rows:?}");
        let (action, resource_id, args) = &rows[0];
        assert_eq!(action, "identity.role.table_access.update");
        assert_eq!(resource_id.as_deref(), Some(role.to_string().as_str()));
        let args = args.as_ref().expect("the audit names what changed");
        assert_eq!(args["oldScope"], json!("all"), "{args}");
        assert_eq!(args["newScope"], json!("listed"), "{args}");
        assert_eq!(args["oldGrants"], json!([]), "{args}");
        assert_eq!(
            args["newGrants"],
            json!([{"schema": "finance", "table": "orders"}]),
            "{args}"
        );
        assert!(
            args["name"]
                .as_str()
                .is_some_and(|n| n.starts_with("GOV-7 role")),
            "{args}"
        );
    }
}
