//! `GET/POST/PUT/DELETE /api/alerts`, `GET/POST /api/alerts/run` — threshold
//! alerts & scheduled digests.
//!
//! Ports `src/app/api/alerts/route.ts` and
//! `src/app/api/alerts/run/route.ts`.

use axum::body::Bytes;
use axum::extract::{Extension, Query, State};
use axum::http::HeaderMap;
use lakehouse_alerts::AlertRuleInput;
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_core::ApiError;
use lakehouse_notify::{EmailSender, SmtpConfig};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::Config;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// `GET /api/alerts` — list every alert & digest rule.
///
/// The `TypeScript` handler's `catch` returns a 500 with `e.message`
/// (`alerts/route.ts`'s `GET`), unlike `POST`/`PUT` which return 400 for
/// the same kind of failure — the status code here depends on which route
/// caught the error, not on the error's own type, so it is chosen at each
/// call site rather than baked into a single `From` conversion.
///
/// # Errors
///
/// Returns a 500 [`ApiError::Internal`] on a `ClickHouse` failure.
pub async fn list(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let rules = lakehouse_alerts::list_rules(&state.clickhouse)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(ApiJson(json!({ "rules": rules })))
}

/// Parse the raw request body as JSON into an [`AlertRuleInput`].
///
/// The `TypeScript` handler's `catch` around `await req.json()` swallows
/// *any* failure from that expression — a genuinely malformed body, but
/// also (irrelevantly here) a body read error — and reports it at 400 with
/// `e.message`. Bun's JSON parser produces a distinctive message (e.g.
/// `JSON Parse error: Unexpected identifier "not"`) that `serde_json`
/// cannot reproduce; see `rust/tests/parity/README.md`'s "Known
/// non-deterministic captures" section, which already documents this exact
/// pair of routes (`alerts-create-bad-body`,
/// `dashboard-boards-create-bad-body`) as un-portable runtime error text
/// and normalizes the `error` field for them in the parity harness. This
/// produces a sensible equivalent — a 400 naming the parse failure — rather
/// than contorting the parser to chase Bun's exact wording.
fn parse_body(body: &Bytes) -> Result<AlertRuleInput, ApiError> {
    serde_json::from_slice(body)
        .map_err(|err| ApiError::BadRequest(format!("JSON tidak valid: {err}")))
}

/// `POST /api/alerts` — create a rule.
///
/// # Errors
///
/// Returns a 400 [`ApiError::BadRequest`] on an unparseable body or a
/// validation failure (see `lakehouse_alerts::save_rule`) — matching the
/// `TypeScript`'s single `catch` around both.
pub async fn create(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let input = parse_body(&body)?;
    let rule = lakehouse_alerts::save_rule(&state.clickhouse, &input, None)
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    Ok(ApiJson(json!({ "ok": true, "rule": rule })))
}

/// `PUT /api/alerts` — update a rule (body must include `id`).
///
/// # Errors
///
/// Returns a 400 [`ApiError::BadRequest`] on an unparseable body, a missing
/// `id`, or a validation failure — matching the `TypeScript`.
pub async fn update(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let input = parse_body(&body)?;
    let Some(id) = input.id.clone() else {
        return Err(ApiError::BadRequest("id wajib".to_owned()).into());
    };
    let rule = lakehouse_alerts::save_rule(&state.clickhouse, &input, Some(&id))
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    Ok(ApiJson(json!({ "ok": true, "rule": rule })))
}

/// Query parameters for `DELETE /api/alerts` (`?id=`).
#[derive(Debug, Deserialize)]
pub struct DeleteQuery {
    id: Option<String>,
}

/// `DELETE /api/alerts?id=` — soft-delete a rule.
///
/// # Errors
///
/// Returns a 400 [`ApiError::BadRequest`] when `id` is missing, or a 500
/// [`ApiError::Internal`] on a `ClickHouse` failure — matching the
/// `TypeScript`'s pre-`try` `id` check (400) vs. its `catch` (500).
pub async fn delete(
    State(state): State<AppState>,
    Query(query): Query<DeleteQuery>,
) -> ApiResult<ApiJson<Value>> {
    let Some(id) = query.id else {
        return Err(ApiError::BadRequest("id wajib".to_owned()).into());
    };
    lakehouse_alerts::delete_rule(&state.clickhouse, &id)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(ApiJson(json!({ "ok": true })))
}

/// Query parameters for `GET`/`POST /api/alerts/run`.
#[derive(Debug, Deserialize, Default)]
pub struct RunQuery {
    /// Run only this rule id, when given.
    id: Option<String>,
    /// Shared token, as a query-string fallback to the `x-run-token`
    /// header.
    token: Option<String>,
}

/// The `/api/alerts/run` guard, split out of the handler so it can be
/// exercised without a live `ClickHouse` connection or ever calling
/// `run_rules`. Was a straight port of `alerts/run/route.ts:16-20`; is now
/// (D4) fail-closed instead of fail-open — see the module-level "D4" doc
/// comment on [`run`] for the full rationale.
///
/// # Security property
///
/// * `configured: Some(token)` — unchanged from the `TypeScript`: the
///   caller's header/query token must match, or this is a 401.
/// * `configured: None` (`ALERTS_RUN_TOKEN` unset) — the `TypeScript`
///   (and this handler, pre-D4) skipped the check entirely, making the
///   route completely unauthenticated: any caller could trigger a real
///   evaluation run that sends real webhooks/emails. D4 closes that: with
///   no shared token configured, only a **service-identity** principal
///   (`PrincipalId::Service` — the cron/scheduler's own credential, not a
///   logged-in human's session) is allowed through; anyone/anything else
///   is refused with 503. A human session principal is deliberately NOT
///   sufficient here even though the router's `Policy::RequiresAuth`
///   already requires ONE — this guard's whole job is to stop an ordinary
///   authenticated user (any signed-up account, not just an operator) from
///   firing every registered alert channel just by hitting this URL.
fn check_run_token(
    configured: Option<&str>,
    header_token: Option<&str>,
    query_token: Option<&str>,
    principal: Option<&Principal>,
) -> Result<(), ApiError> {
    if let Some(need) = configured {
        return if header_token.or(query_token) == Some(need) {
            Ok(())
        } else {
            Err(ApiError::unauthorized())
        };
    }
    match principal {
        Some(p) if matches!(p.id, PrincipalId::Service(_)) => Ok(()),
        _ => Err(ApiError::Unavailable(
            "alerts run tidak dikonfigurasi: set ALERTS_RUN_TOKEN, atau panggil dengan \
             kredensial service identity (bukan sesi pengguna manusia)"
                .to_owned(),
        )),
    }
}

fn smtp_config(config: &Config) -> SmtpConfig {
    SmtpConfig {
        host: config.smtp_host.clone(),
        port: config.smtp_port,
        secure: config.smtp_secure,
        user: config.smtp_user.clone(),
        pass: config.smtp_pass.clone(),
        from: config.smtp_from.clone(),
    }
}

/// `GET`/`POST /api/alerts/run` — evaluate rules and deliver alerts/
/// digests that fire.
///
/// # Warning
///
/// This is a live, side-effecting endpoint: on success it queries
/// `serving.*` marts and, for every rule that fires, sends a real webhook
/// `POST` or a real `SMTP` email. It is NOT captured in the parity corpus
/// for exactly this reason — see `rust/tests/parity/README.md`'s
/// "Deliberate omissions" section. Do not call this against production
/// infrastructure while testing; only the rejection paths (wrong/missing
/// token with `ALERTS_RUN_TOKEN` set; no token configured and no
/// service-identity principal) are safe to exercise.
///
/// # D4: fail closed when `ALERTS_RUN_TOKEN` is unset
///
/// The `TypeScript` original — and this handler, before this fix — skipped
/// its shared-token check entirely when `ALERTS_RUN_TOKEN` was unset,
/// which is precisely the state of this environment: the guard was a
/// no-op, and any caller (any signed-up user, once auth landed; literally
/// anyone, before it) could trigger every alert rule and fire real
/// webhooks/emails. [`check_run_token`] now fails closed: with no token
/// configured, only a `PrincipalId::Service` principal — a
/// `service_identity` credential meant for the cron/scheduler that runs
/// this on a timer, not a human's browser session — is let through;
/// everyone else gets a 503. The 503 (not 401) is deliberate: this is a
/// missing-configuration state ("nobody set up how this route should be
/// called"), the same idiom `routes::identity::pool` uses for "no
/// `DATABASE_URL`" — not a bad-credential state, which is what 401 means
/// everywhere else in this crate. The token path (`ALERTS_RUN_TOKEN` set)
/// is unchanged, so an existing cron/scheduler integration keeps working
/// exactly as before.
///
/// # Errors
///
/// Returns a 401 [`ApiError::unauthorized`] when `ALERTS_RUN_TOKEN` is set
/// and the caller's token doesn't match; a 503 [`ApiError::Unavailable`]
/// when it is unset and the caller is not an authenticated service-identity
/// principal; or a 500 [`ApiError::Internal`] on a `ClickHouse` failure
/// while listing rules.
pub async fn run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RunQuery>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let header_token = headers.get("x-run-token").and_then(|v| v.to_str().ok());
    check_run_token(
        state.config.alerts_run_token.as_deref(),
        header_token,
        query.token.as_deref(),
        principal.as_ref().map(|Extension(p)| p),
    )?;

    let http = reqwest::Client::new();
    let email = EmailSender::new(smtp_config(&state.config));
    let results =
        lakehouse_alerts::run_rules(&state.clickhouse, &http, &email, query.id.as_deref())
            .await
            .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(ApiJson(json!({ "ran": results.len(), "results": results })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_auth::PermissionSet;
    use uuid::Uuid;

    use super::*;

    fn service_principal() -> Principal {
        Principal {
            id: PrincipalId::Service(Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "alerts-cron".to_owned(),
            permissions: PermissionSet::default(),
            provider: "service".to_owned(),
        }
    }

    fn user_principal() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "Rina Wijaya".to_owned(),
            permissions: PermissionSet::parse("*:*"),
            provider: "session".to_owned(),
        }
    }

    #[test]
    fn run_endpoint_rejects_wrong_token_with_401() {
        let err = check_run_token(Some("secret"), None, Some("wrong"), None).unwrap_err();
        assert_eq!(err.to_string(), "unauthorized");
        assert_eq!(err.status(), 401);
    }

    #[test]
    fn run_endpoint_rejects_missing_token_with_401() {
        let err = check_run_token(Some("secret"), None, None, None).unwrap_err();
        assert_eq!(err.to_string(), "unauthorized");
    }

    #[test]
    fn run_endpoint_accepts_matching_header_token() {
        assert!(check_run_token(Some("secret"), Some("secret"), None, None).is_ok());
    }

    #[test]
    fn run_endpoint_accepts_matching_query_token_as_fallback() {
        assert!(check_run_token(Some("secret"), None, Some("secret"), None).is_ok());
    }

    #[test]
    fn run_endpoint_prefers_header_token_over_query_token() {
        // `req.headers.get("x-run-token") || url.searchParams.get("token")`
        // — the header wins when both are present, matching JS `||`.
        assert!(check_run_token(Some("secret"), Some("secret"), Some("wrong"), None).is_ok());
    }

    /// A token, once configured, is still checked even when the caller
    /// happens to also be a service-identity principal — the token path and
    /// the no-token-configured fallback are mutually exclusive branches,
    /// not additive.
    #[test]
    fn configured_token_still_required_even_for_a_service_principal() {
        let service = service_principal();
        let err = check_run_token(Some("secret"), None, Some("wrong"), Some(&service)).unwrap_err();
        assert_eq!(err.status(), 401);
    }

    /// D4 regression: with `ALERTS_RUN_TOKEN` unset, the guard used to pass
    /// unconditionally (see git history / the pre-D4 doc comment) —
    /// including for a caller presenting no token at all and no principal.
    /// It must now fail closed: 503, not a silent pass.
    #[test]
    fn run_endpoint_fails_closed_when_token_unset_and_no_principal() {
        let err = check_run_token(None, None, None, None).unwrap_err();
        assert_eq!(err.status(), 503);
        let err_with_wrong_token = check_run_token(None, Some("anything"), None, None).unwrap_err();
        assert_eq!(err_with_wrong_token.status(), 503);
    }

    /// D4: a human session principal (however highly privileged — even
    /// `*:*`) is still refused when no token is configured. This guard's
    /// whole point is that "some authenticated user" is not enough for a
    /// route that fires real webhooks/emails; only the dedicated
    /// service-identity door is.
    #[test]
    fn run_endpoint_fails_closed_for_a_human_principal_even_with_wildcard_permissions() {
        let user = user_principal();
        let err = check_run_token(None, None, None, Some(&user)).unwrap_err();
        assert_eq!(err.status(), 503);
    }

    /// D4: the new, intended long-term door — a service-identity principal
    /// (e.g. the cron/scheduler's own credential) is let through even with
    /// no shared token configured.
    #[test]
    fn run_endpoint_allows_a_service_identity_principal_when_token_unset() {
        let service = service_principal();
        assert!(check_run_token(None, None, None, Some(&service)).is_ok());
    }
}
