//! `GET/POST/PUT/DELETE /api/alerts`, `GET/POST /api/alerts/run` — threshold
//! alerts & scheduled digests.
//!
//! Ports `src/app/api/alerts/route.ts` and
//! `src/app/api/alerts/run/route.ts`.

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use lakehouse_alerts::AlertRuleInput;
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

/// The shared-token guard for `/api/alerts/run`, split out of the handler
/// so it can be exercised without a live `ClickHouse` connection or ever
/// calling `run_rules`. Ports `alerts/run/route.ts:16-20`.
///
/// # Security property
///
/// When `configured` is `None` (`ALERTS_RUN_TOKEN` unset), this ALWAYS
/// returns `Ok(())` — the endpoint is completely unauthenticated, and any
/// caller can trigger a real evaluation run that can send real webhooks/
/// emails. This is not a bug: it is the exact `TypeScript` contract
/// (`if (need) { ... }` — the whole check is skipped when the env var is
/// unset), reproduced faithfully. Operators who want this endpoint guarded
/// must set `ALERTS_RUN_TOKEN`.
fn check_run_token(
    configured: Option<&str>,
    header_token: Option<&str>,
    query_token: Option<&str>,
) -> Result<(), ApiError> {
    let Some(need) = configured else {
        return Ok(());
    };
    if header_token.or(query_token) == Some(need) {
        Ok(())
    } else {
        Err(ApiError::unauthorized())
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
/// infrastructure while testing; only the 401-rejection path (wrong/
/// missing token, with `ALERTS_RUN_TOKEN` set) is safe to exercise.
///
/// # Errors
///
/// Returns a 401 [`ApiError::unauthorized`] when `ALERTS_RUN_TOKEN` is set
/// and the caller's token doesn't match, or a 500 [`ApiError::Internal`] on
/// a `ClickHouse` failure while listing rules.
pub async fn run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RunQuery>,
) -> ApiResult<ApiJson<Value>> {
    let header_token = headers.get("x-run-token").and_then(|v| v.to_str().ok());
    check_run_token(
        state.config.alerts_run_token.as_deref(),
        header_token,
        query.token.as_deref(),
    )?;
    if state.config.alerts_run_token.is_none() {
        tracing::warn!(
            "/api/alerts/run was called with ALERTS_RUN_TOKEN unset — this endpoint is \
             completely unauthenticated and any caller can trigger real webhook/email \
             deliveries"
        );
    }

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

    use super::*;

    #[test]
    fn run_endpoint_rejects_wrong_token_with_401() {
        let err = check_run_token(Some("secret"), None, Some("wrong")).unwrap_err();
        assert_eq!(err.to_string(), "unauthorized");
        assert_eq!(err.status(), 401);
    }

    #[test]
    fn run_endpoint_rejects_missing_token_with_401() {
        let err = check_run_token(Some("secret"), None, None).unwrap_err();
        assert_eq!(err.to_string(), "unauthorized");
    }

    #[test]
    fn run_endpoint_accepts_matching_header_token() {
        assert!(check_run_token(Some("secret"), Some("secret"), None).is_ok());
    }

    #[test]
    fn run_endpoint_accepts_matching_query_token_as_fallback() {
        assert!(check_run_token(Some("secret"), None, Some("secret")).is_ok());
    }

    #[test]
    fn run_endpoint_prefers_header_token_over_query_token() {
        // `req.headers.get("x-run-token") || url.searchParams.get("token")`
        // — the header wins when both are present, matching JS `||`.
        assert!(check_run_token(Some("secret"), Some("secret"), Some("wrong")).is_ok());
    }

    /// H1 / security property: with `ALERTS_RUN_TOKEN` unset, the guard
    /// passes regardless of what token (if any) the caller supplies —
    /// including a deliberately wrong one. This is the exact `TypeScript`
    /// contract (`alerts/run/route.ts:16-20`'s `if (need) { ... }`), not an
    /// oversight; see the doc comment on `check_run_token`.
    #[test]
    fn run_endpoint_is_unguarded_when_token_unset() {
        assert!(check_run_token(None, None, None).is_ok());
        assert!(check_run_token(None, Some("anything"), None).is_ok());
        assert!(check_run_token(None, None, Some("wrong")).is_ok());
    }
}
