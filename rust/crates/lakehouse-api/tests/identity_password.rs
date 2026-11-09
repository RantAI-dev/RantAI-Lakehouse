//! ADM-3: an invite hands out a temporary password, and an admin can reset
//! one.
//!
//! Before this, `POST /api/identity/users` created a directory row with no
//! credential at all, so a site without an identity provider could give an
//! account to nobody but the bootstrap admin. ADM-3 D1 gives every invite
//! a server-generated temporary password, returned exactly once; D2 gives
//! an admin a way to reset one; D3 stops a reset from being an
//! escalation path; D5 audits both without ever carrying the password.
//!
//! The tests live in a module named `identity` so the plan's
//! `cargo test -p lakehouse-api identity` filter runs them: an
//! integration test's name is its function's path inside the test
//! binary, and a bare function name would never match that filter.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use common::{TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up};

mod identity {

    /// ADM-11 D7 / ADM-11 review 1 S1: missing or blank username returns 400 "username is required".
    #[tokio::test]
    async fn invite_with_missing_or_blank_username_returns_400() {
        let app = spin_up().await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        for body in [
            json!({
                "name": "Missing Username",
                "email": "missing.username@example.com",
                "roles": ["Analyst"],
            }),
            json!({
                "name": "Blank Username",
                "username": "   ",
                "email": "blank.username@example.com",
                "roles": ["Analyst"],
            }),
        ] {
            let (status, resp_body, _) = post_json(
                &app,
                Some(&admin_cookie),
                "/api/identity/users",
                &body,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(
                resp_body.get("message").and_then(|m| m.as_str()),
                Some("username is required")
            );
        }
    }

    use super::*;

    /// The email every invited user in this file gets. `.example` is the
    /// RFC 2606 reserved TLD and can never route to a real mailbox.
    const INVITED_EMAIL: &str = "rina.guest@meridian.example";

    /// The password the invited user replaces the temporary one with.
    const CHOSEN_PASSWORD: &str = "a-chosen-password-for-rina-2026";

    // ── helpers ──────────────────────────────────────────────────────────

    /// POST a JSON body; return the status, the parsed body, and the
    /// `Set-Cookie` header the response carried (the login route's
    /// session cookie lives in it).
    async fn post_json(
        app: &TestApp,
        cookie: Option<&str>,
        uri: &str,
        body: &Value,
    ) -> (StatusCode, Value, Option<String>) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        let response = app
            .router
            .clone()
            .oneshot(
                builder
                    .body(Body::from(body.to_string()))
                    .expect("build request"),
            )
            .await
            .expect("router never fails a request outright");
        let status = response.status();
        // Read the header BEFORE the body: `axum::body::Body` is not
        // `Clone`, so the response cannot be read twice.
        let set_cookie = response
            .headers()
            .get("set-cookie")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            set_cookie,
        )
    }

    /// GET a resource; return the status and the parsed body.
    async fn get_json(app: &TestApp, cookie: Option<&str>, uri: &str) -> (StatusCode, Value) {
        let mut builder = Request::builder().method("GET").uri(uri);
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        let response = app
            .router
            .clone()
            .oneshot(builder.body(Body::empty()).expect("build request"))
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

    /// The `Cookie` header value carrying the session a login response
    /// handed out — the first `name=value` pair of its `Set-Cookie`.
    fn session_cookie_from(set_cookie: Option<String>) -> String {
        let header = set_cookie.expect("a successful login carries a session cookie");
        header
            .split(';')
            .next()
            .expect("the cookie header carries a name=value pair")
            .trim()
            .to_owned()
    }

    /// Invite a user and return the 201 body.
    async fn invite(app: &TestApp, cookie: &str) -> Value {
        let (status, body, _) = post_json(
            app,
            Some(cookie),
            "/api/identity/users",
            &json!({
                "name": "Rina Guest",
                "username": "rina.guest",
                "email": INVITED_EMAIL,
                "roles": ["Analyst"],
                "tenants": ["Meridian Retail"],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body
    }

    /// The temporary password out of an invite (or a reset) body.
    fn temporary_password(body: &Value) -> String {
        body["temporaryPassword"]
            .as_str()
            .expect("the response carries a temporary password")
            .to_owned()
    }

    // ── the log capture ─────────────────────────────────────────────────

    /// An in-memory `tracing` writer: every line the process logs is kept
    /// in memory so a test can read it.
    #[derive(Clone)]
    struct LogCapture(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for LogCapture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("the log buffer lock")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
        type Writer = LogCapture;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// The process-wide log buffer, and the install of the `tracing`
    /// subscriber that fills it.
    ///
    /// The global subscriber can only be set once per process, so the
    /// second caller finds it already installed and simply reuses the same
    /// buffer. The install has to happen BEFORE the request under test,
    /// or the lines that request wrote would not be in the buffer.
    fn log_capture() -> &'static Arc<Mutex<Vec<u8>>> {
        static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
        LOGS.get_or_init(|| {
            let buffer = Arc::new(Mutex::new(Vec::new()));
            let _ = tracing_subscriber::fmt()
                .with_max_level(tracing::Level::TRACE)
                .with_ansi(false)
                .with_writer(LogCapture(Arc::clone(&buffer)))
                .try_init();
            buffer
        })
    }

    /// Install the process-wide `tracing` capture.
    fn install_log_capture() {
        let _ = log_capture();
    }

    /// Everything the process has logged so far.
    fn captured_logs() -> String {
        let bytes = log_capture().lock().expect("the log buffer lock").clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    // ── the tests ───────────────────────────────────────────────────────

    /// ADM-3 D1 + D4, end to end: the invite returns a password, that
    /// password signs in, the sign-in is FORCED through the
    /// change-password screen, and after the change the temporary password
    /// is dead.
    #[tokio::test]
    async fn an_invite_returns_a_password_that_signs_in_and_must_then_change_it() {
        let app = spin_up().await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        let body = invite(&app, &admin_cookie).await;
        let temporary = temporary_password(&body);
        assert!(
            temporary.chars().count() >= 20,
            "a temporary password is at least 20 characters, got {}",
            temporary.chars().count()
        );
        // The user is flattened next to the password, not nested under it.
        assert_eq!(body["email"], json!(INVITED_EMAIL));
        assert_eq!(body["roles"], json!(["Analyst"]));
        assert_eq!(body["tenants"], json!(["Meridian Retail"]));

        // The temporary password signs in.
        let (status, login, set_cookie) = post_json(
            &app,
            None,
            "/api/auth/login",
            &json!({ "email": INVITED_EMAIL, "password": temporary }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{login}");
        assert_eq!(login["mustChangePassword"], json!(true));
        let session = session_cookie_from(set_cookie);

        // D4: while the rotation is pending, every route but the three
        // allowed ones is refused. `GET /api/home/layout` needs no
        // permission of its own (`Policy::RequiresAuth`), so the refusal
        // it gets is the ROTATION's, not a missing grant — which is what
        // makes it the right probe here.
        let (status, body) = get_json(&app, Some(&session), "/api/home/layout").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body["error"]
                .as_str()
                .unwrap_or_default()
                .contains("password rotation required"),
            "the refusal says why: {body}"
        );

        // The forced rotation itself is reachable, and needs no old
        // password — the temporary one is treated as "good for logging in
        // and nothing else".
        let (status, body, _) = post_json(
            &app,
            Some(&session),
            "/api/auth/change-password",
            &json!({ "newPassword": CHOSEN_PASSWORD }),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

        // The chosen password signs in, and no longer asks for a change.
        let (status, login, _) = post_json(
            &app,
            None,
            "/api/auth/login",
            &json!({ "email": INVITED_EMAIL, "password": CHOSEN_PASSWORD }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{login}");
        assert_eq!(login["mustChangePassword"], json!(false));

        // The temporary password is dead.
        let (status, _, _) = post_json(
            &app,
            None,
            "/api/auth/login",
            &json!({ "email": INVITED_EMAIL, "password": temporary }),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    /// ADM-3 D1: the temporary password is returned ONCE. The directory
    /// listing never carries it, the audit rows never carry it, and the
    /// process never logs it.
    #[tokio::test]
    async fn the_temporary_password_is_absent_from_the_list_the_audit_rows_and_the_logs() {
        install_log_capture();
        let app = spin_up().await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        let body = invite(&app, &admin_cookie).await;
        let temporary = temporary_password(&body);

        // The directory listing never carries it.
        let (status, list) = get_json(&app, Some(&admin_cookie), "/api/identity/users").await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            !list.to_string().contains(&temporary),
            "the user list must not carry the temporary password"
        );

        // The audit rows never carry it — not the invite's, and not the
        // reset's either.
        let args: Vec<Value> = sqlx::query_scalar(
            "SELECT args FROM audit_event \
             WHERE action IN ('identity.user.invite', 'identity.user.password_reset')",
        )
        .fetch_all(&app.pool)
        .await
        .expect("read the audit rows");
        assert!(!args.is_empty(), "the invite must be audited at all");
        for row in &args {
            assert!(
                !row.to_string().contains(&temporary),
                "an audit row must not carry the temporary password: {row}"
            );
        }

        // The process never logs it.
        let logs = captured_logs();
        assert!(
            !logs.contains(&temporary),
            "the temporary password must never reach a log line"
        );
    }

    /// ADM-3 D1: the invite is ONE transaction. A role or tenant name
    /// that cannot be resolved leaves no user and no credential behind —
    /// the credential can never outlive a failed invite.
    #[tokio::test]
    async fn a_failed_role_or_tenant_name_leaves_no_user_and_no_credential_behind() {
        let app = spin_up().await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        for (roles, tenants, label) in [
            (json!(["No Such Role"]), json!([]), "role"),
            (json!([]), json!(["No Such Tenant"]), "tenant"),
        ] {
            let (status, body, _) = post_json(
                &app,
                Some(&admin_cookie),
                "/api/identity/users",
                &json!({
                    "name": "Half Created",
                    "username": "half.created",
                    "email": INVITED_EMAIL,
                    "roles": roles,
                    "tenants": tenants,
                }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{label}: {body}");

            let users: i64 = sqlx::query_scalar("SELECT count(*) FROM app_user WHERE email = $1")
                .bind(INVITED_EMAIL)
                .fetch_one(&app.pool)
                .await
                .expect("count the users");
            assert_eq!(users, 0, "{label}: no user may be left behind");

            let identities: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM auth_identity \
                 WHERE app_user_id IN (SELECT id FROM app_user WHERE email = $1)",
            )
            .bind(INVITED_EMAIL)
            .fetch_one(&app.pool)
            .await
            .expect("count the credentials");
            assert_eq!(identities, 0, "{label}: no credential may be left behind");
        }
    }

    /// ADM-3 D2: a reset hands out a NEW password AND ends the target's
    /// live sessions. A reset that left the old sessions alive would not
    /// have locked the target out of anything.
    #[tokio::test]
    async fn a_reset_returns_a_new_password_and_ends_the_targets_sessions() {
        let app = spin_up().await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        let invited = invite(&app, &admin_cookie).await;
        let temporary = temporary_password(&invited);
        let user_id = invited["id"].as_str().expect("the invite carries an id");

        // The invitee signs in.
        let (status, _, set_cookie) = post_json(
            &app,
            None,
            "/api/auth/login",
            &json!({ "email": INVITED_EMAIL, "password": temporary }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let session = session_cookie_from(set_cookie);
        let (status, _) = get_json(&app, Some(&session), "/api/auth/me").await;
        assert_eq!(status, StatusCode::OK, "the session works before the reset");

        // The admin resets the password.
        let (status, body, _) = post_json(
            &app,
            Some(&admin_cookie),
            &format!("/api/identity/users/{user_id}/reset-password"),
            &json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let reset = temporary_password(&body);
        assert_ne!(reset, temporary, "a reset hands out a NEW password");

        // The target's session is dead.
        let (status, _) = get_json(&app, Some(&session), "/api/auth/me").await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "a reset must end the target's sessions"
        );

        // The new password signs in, and asks for a change of its own.
        let (status, login, _) = post_json(
            &app,
            None,
            "/api/auth/login",
            &json!({ "email": INVITED_EMAIL, "password": reset }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{login}");
        assert_eq!(login["mustChangePassword"], json!(true));

        // The old password no longer works.
        let (status, _, _) = post_json(
            &app,
            None,
            "/api/auth/login",
            &json!({ "email": INVITED_EMAIL, "password": temporary }),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    /// ADM-3 D3: only an unrestricted (`*:*`) caller may reset the
    /// password of a user who holds `*:*`. Everyone else gets the SAME 404
    /// an unknown user gets, body for body, so a probe learns nothing.
    #[tokio::test]
    async fn a_caller_without_the_unrestricted_grant_gets_404_for_an_unrestricted_user() {
        let app = spin_up().await;
        let caller = create_principal_with_permissions(&app.pool, "identity:write").await;
        let caller_cookie = session_cookie_for_user(&app.pool, caller).await;
        let target = create_principal_with_permissions(&app.pool, "*:*").await;

        let (status, body, _) = post_json(
            &app,
            Some(&caller_cookie),
            &format!("/api/identity/users/{target}/reset-password"),
            &json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

        // The same answer as for a user who does not exist at all.
        let (unknown_status, unknown_body, _) = post_json(
            &app,
            Some(&caller_cookie),
            &format!("/api/identity/users/{}/reset-password", Uuid::new_v4()),
            &json!({}),
        )
        .await;
        assert_eq!(
            unknown_status,
            StatusCode::NOT_FOUND,
            "unknown user body: {unknown_body}"
        );
        assert_eq!(
            body, unknown_body,
            "no oracle for which users hold the unrestricted grant"
        );
    }

    /// ADM-3 D3, the other side: an unrestricted caller MAY reset an
    /// unrestricted user. The rule is about the CALLER, not about the
    /// target.
    #[tokio::test]
    async fn an_unrestricted_caller_may_reset_an_unrestricted_user() {
        let app = spin_up().await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;
        let target = create_principal_with_permissions(&app.pool, "*:*").await;

        let (status, body, _) = post_json(
            &app,
            Some(&admin_cookie),
            &format!("/api/identity/users/{target}/reset-password"),
            &json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            temporary_password(&body).chars().count() >= 20,
            "the reset hands out a temporary password"
        );
    }

    /// ADM-3 D3: a caller may not reset their OWN password here — that is
    /// what `POST /api/auth/change-password` is for, and it proves the old
    /// password instead of replacing it behind the holder's back.
    async fn patch_json(
        app: &TestApp,
        cookie: Option<&str>,
        uri: &str,
        body: &Value,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method("PATCH")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        let response = app
            .router
            .clone()
            .oneshot(
                builder
                    .body(Body::from(body.to_string()))
                    .expect("build request"),
            )
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

    /// ADM-11 D7: `set` with `mustChangePassword: false` signs in directly,
    /// returns no `temporaryPassword`, and can immediately reach a gated route.
    #[tokio::test]
    async fn invite_with_set_and_must_change_password_false_works_directly() {
        let app = spin_up().await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        let typed_pass = "AdminTypedPassword123";
        let (status, body, _) = post_json(
            &app,
            Some(&admin_cookie),
            "/api/identity/users",
            &json!({
                "name": "Ana Set",
                "username": "ana.set",
                "roles": ["Analyst"],
                "tenants": ["Meridian Retail"],
                "password": { "mode": "set", "value": typed_pass },
                "mustChangePassword": false
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert!(
            body.get("temporaryPassword").is_none(),
            "set mode must not return a temporary password: {body}"
        );

        // Sign in with the typed password
        let (login_status, login_body, cookie_header) = post_json(
            &app,
            None,
            "/api/auth/login",
            &json!({
                "identifier": "ana.set",
                "password": typed_pass
            }),
        )
        .await;
        assert_eq!(login_status, StatusCode::OK, "{login_body}");
        assert_eq!(login_body["mustChangePassword"], json!(false));
        let user_cookie = session_cookie_from(cookie_header);

        // Can reach a gated route immediately
        let (gate_status, _) = get_json(&app, Some(&user_cookie), "/api/home/layout").await;
        assert_eq!(gate_status, StatusCode::OK);

        // Audit row holds no password and no email
        let row: Value = sqlx::query_scalar(
            "SELECT args FROM audit_event WHERE action = 'identity.user.invite' AND resource_id = $1",
        )
        .bind(body["id"].as_str().unwrap())
        .fetch_one(&app.pool)
        .await
        .expect("audit row");
        assert_eq!(row["username"], json!("ana.set"));
        assert_eq!(row["hasEmail"], json!(false));
        assert_eq!(row["passwordMode"], json!("set"));
        assert_eq!(row["mustChangePassword"], json!(false));
        assert!(row.get("email").is_none());
        assert!(!row.to_string().contains(typed_pass));
    }

    /// ADM-11 D8: rename lets the new username sign in and the old one not.
    #[tokio::test]
    async fn rename_lets_new_username_sign_in_and_old_one_not() {
        let app = spin_up().await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        let pass = "TypedPassWord123";
        let (create_status, create_body, _) = post_json(
            &app,
            Some(&admin_cookie),
            "/api/identity/users",
            &json!({
                "name": "Budi Test",
                "username": "budi.original",
                "roles": ["Analyst"],
                "tenants": ["Meridian Retail"],
                "password": { "mode": "set", "value": pass },
                "mustChangePassword": false
            }),
        )
        .await;
        assert_eq!(create_status, StatusCode::CREATED);
        let user_id = create_body["id"].as_str().unwrap();

        // Rename via PATCH /api/identity/users/{id}
        let (patch_status, patch_body) = patch_json(
            &app,
            Some(&admin_cookie),
            &format!("/api/identity/users/{user_id}"),
            &json!({
                "username": "budi.renamed"
            }),
        )
        .await;
        assert_eq!(patch_status, StatusCode::OK, "{patch_body}");
        assert_eq!(patch_body["username"], "budi.renamed");

        // New username signs in
        let (status_new, _, _) = post_json(
            &app,
            None,
            "/api/auth/login",
            &json!({
                "identifier": "budi.renamed",
                "password": pass
            }),
        )
        .await;
        assert_eq!(status_new, StatusCode::OK);

        // Old username no longer signs in
        let (status_old, _, _) = post_json(
            &app,
            None,
            "/api/auth/login",
            &json!({
                "identifier": "budi.original",
                "password": pass
            }),
        )
        .await;
        assert_eq!(status_old, StatusCode::UNAUTHORIZED);

        // Audit row for update holds changedFields, no password, no email
        let audit: Value = sqlx::query_scalar(
            "SELECT args FROM audit_event WHERE action = 'identity.user.update' AND resource_id = $1",
        )
        .bind(user_id)
        .fetch_one(&app.pool)
        .await
        .expect("update audit row");
        assert_eq!(audit["changedFields"], json!(["username"]));
        assert!(audit.get("email").is_none());
        assert!(!audit.to_string().contains(pass));
    }

    /// ADM-11 D8: a non-*:* caller renaming a *:* user gets 404 (same as for an unknown user).
    #[tokio::test]
    async fn non_unrestricted_caller_renaming_unrestricted_user_gets_404() {
        let app = spin_up().await;
        let unrestricted_target = create_principal_with_permissions(&app.pool, "*:*").await;
        let scoped_caller = create_principal_with_permissions(&app.pool, "identity:write").await;
        let caller_cookie = session_cookie_for_user(&app.pool, scoped_caller).await;

        let (status, body) = patch_json(
            &app,
            Some(&caller_cookie),
            &format!("/api/identity/users/{unrestricted_target}"),
            &json!({
                "username": "hacked.admin"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], "record not found");
    }

    #[tokio::test]
    async fn a_caller_may_not_reset_its_own_password_here() {
        let app = spin_up().await;
        let caller = create_principal_with_permissions(&app.pool, "*:*").await;
        let caller_cookie = session_cookie_for_user(&app.pool, caller).await;

        let (status, body, _) = post_json(
            &app,
            Some(&caller_cookie),
            &format!("/api/identity/users/{caller}/reset-password"),
            &json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"]
                .as_str()
                .unwrap_or_default()
                .contains("change-password"),
            "the refusal points at the route that does belong here: {body}"
        );
    }
}
