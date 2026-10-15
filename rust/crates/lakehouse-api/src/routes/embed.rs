//! `POST /api/embed/data`, `GET /api/public/dashboard/{token}` — read-only
//! dashboard views for external viewers (signed embed & public share link).
//!
//! Ports `src/app/api/embed/data/route.ts` and
//! `src/app/api/public/dashboard/[token]/route.ts`. Both assemble the same
//! `{ board, layout, charts, results }` payload as `dashboard::get`, but
//! scoped to one board and with dashboard-wide filters *locked* (a signed
//! embed's JWT `params`, or a public board's own stored filters) rather
//! than caller-supplied.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_bi::store::{self, Board, FilterDef, StoredChartSpec};
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::policy_engine::PolicyEngineObligations;
use crate::routes::support::{
    mart_columns, render_stored_spec, run_spec_sql, sources_for, stored_chart_sql,
};
use crate::state::AppState;
use crate::upstream_error;

/// WS7 item D1: neither `POST /api/embed/data` nor
/// `GET /api/public/dashboard/{token}` carries a real principal —
/// `Policy::Public`, checked in `route_auth.rs`'s own table-driven loop
/// against `policy.rs`'s `POLICY_TABLE`. Both are governed as if the
/// viewer held exactly this seeded, least-privileged, dashboard-only role
/// (`0002_seed_identity.sql:46`), never as an unrestricted principal — a
/// masking/row-filter policy authored against `"Dashboard Viewer"` (or
/// against no role at all, which matches nobody) is the only way to
/// restrict what an external embed/public-link viewer can see through
/// these two routes. This is a deliberate mapping, documented here, not
/// an accidental "no principal, so no obligations" fail-open path (WS7
/// plan Hard Requirement 2).
pub(crate) const EMBED_VIEWER_ROLE: &str = "Dashboard Viewer";

/// `{ jwt }` — the `POST /api/embed/data` body shape.
#[derive(Debug, Default, Deserialize)]
struct EmbedDataBody {
    #[serde(default)]
    jwt: Option<String>,
}

/// The one message every refused signed token gets (`SEC-12`): a bad
/// signature, a missing or too-long lifetime, an expired or withdrawn token
/// and a token without a dashboard all read the same.
pub(crate) const TOKEN_REFUSED: &str = "embed token is invalid or expired";

/// What every signed-embed surface says when `EMBED_SECRET` is unset
/// (`SEC-12`): `POST /api/embed/data` answers it with `503`, and
/// `GET /api/dashboard/embed-info` as the `reason` of `supported: false`.
pub(crate) const EMBEDDING_NOT_CONFIGURED: &str = "embedding is not configured";

fn token_refused() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        ApiJson(json!({ "error": TOKEN_REFUSED })),
    )
        .into_response()
}

/// `POST /api/embed/data` — signed-embed (Metabase-style) dashboard data.
pub async fn data(State(state): State<AppState>, body: Bytes) -> Response {
    // `try { jwt = String((await req.json())?.jwt ?? "") } catch { /* ignore */ }`
    // — an unparseable body is swallowed, not a 400; it just yields an
    // empty jwt, which the emptiness check below rejects the normal way.
    let jwt = serde_json::from_slice::<EmbedDataBody>(&body)
        .ok()
        .and_then(|b| b.jwt)
        .filter(|s| !s.is_empty());
    let Some(jwt) = jwt else {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "jwt is required" })),
        )
            .into_response();
    };

    // SEC-12: the secret is configuration only. Unset means signed embedding
    // is unavailable, said plainly; nothing is generated or stored.
    let Some(secret) = state.config.embed_secret.as_deref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": EMBEDDING_NOT_CONFIGURED })),
        )
            .into_response();
    };
    let claims = match lakehouse_embed::verify_embed(
        &jwt,
        secret,
        state.config.embed_token_max_lifetime_seconds,
        lakehouse_embed::unix_now(),
    ) {
        Ok(claims) => claims,
        Err(reason) => {
            // SEC-12: the reason is for the log only; the response is one
            // fixed message so a caller cannot tell which check failed.
            tracing::info!(%reason, "signed embed token refused");
            return token_refused();
        }
    };

    let Some(board_id) = claims
        .resource
        .and_then(|r| r.dashboard)
        .filter(|d| !d.is_empty())
    else {
        return token_refused();
    };

    let board = match store::get_board(&state.clickhouse, &board_id).await {
        Ok(b) => b,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
            )
                .into_response();
        }
    };
    let Some(board) = board.filter(|b| b.embed_enabled.unwrap_or(false)) else {
        return (
            StatusCode::FORBIDDEN,
            ApiJson(json!({ "error": "embedding_disabled" })),
        )
            .into_response();
    };
    // SEC-12: withdrawal is read from the board on every request (a `FINAL`
    // read, no cache in this process), so "withdraw all" and "withdraw one"
    // take effect on the next request. A token with no `iat` cannot get here
    // (`verify_embed` refuses it); `NEG_INFINITY` would be refused if it did.
    if board.embed_access.is_withdrawn(
        claims.iat.unwrap_or(f64::NEG_INFINITY),
        claims.jti.as_deref(),
    ) {
        return token_refused();
    }

    let mut filters = board.filters.clone().unwrap_or_default();
    filters.extend(params_to_filters(claims.params));

    let time = match crate::routes::settings::time_context(&state).await {
        Ok(t) => t,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    let payload = render_board_payload(
        &state.clickhouse,
        &PolicyEngineObligations::from_state(&state),
        &board,
        &board_id,
        &filters,
        &time,
    )
    .await;
    match payload {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
        )
            .into_response(),
    }
}

/// Body of `POST /api/embed/frame`: the token the embed page was opened
/// with, either a signed `jwt` (`/embed/signed/<jwt>`) or the public link's
/// `token` (`/embed/dashboard/<token>`).
#[derive(Debug, Default, Deserialize)]
struct FrameBody {
    #[serde(default)]
    jwt: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

/// `POST /api/embed/frame` — the sites allowed to frame the embed page opened
/// with this token (`SEC-12`). Public, like the page itself: the token is
/// the credential.
///
/// The console's `proxy.ts` calls this before it serves `/embed/*` and turns
/// the answer into `Content-Security-Policy: frame-ancestors ...`. It returns
/// ONLY the origin list. A token that is invalid, expired, withdrawn, for a
/// board with embedding off or for no board at all gets the same answer as a
/// board with no sites listed, `{"origins": []}`, so this route is no oracle
/// for whether a token is good. A store failure is a `500` with the fixed
/// database message; the caller treats anything but `200` as "no site".
pub async fn frame_origins(State(state): State<AppState>, body: Bytes) -> Response {
    let parsed = serde_json::from_slice::<FrameBody>(&body).unwrap_or_default();
    match resolve_frame_origins(&state, parsed).await {
        Ok(origins) => (StatusCode::OK, ApiJson(json!({ "origins": origins }))).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
        )
            .into_response(),
    }
}

async fn resolve_frame_origins(state: &AppState, body: FrameBody) -> Result<Vec<String>, ChError> {
    if let Some(jwt) = body.jwt.filter(|s| !s.is_empty()) {
        // Same verification `data` applies, so a token that would be refused
        // there is not given a framing list here.
        let Some(secret) = state.config.embed_secret.as_deref() else {
            return Ok(Vec::new());
        };
        let Ok(claims) = lakehouse_embed::verify_embed(
            &jwt,
            secret,
            state.config.embed_token_max_lifetime_seconds,
            lakehouse_embed::unix_now(),
        ) else {
            return Ok(Vec::new());
        };
        let Some(board_id) = claims.resource.and_then(|r| r.dashboard) else {
            return Ok(Vec::new());
        };
        let board = store::get_board(&state.clickhouse, &board_id).await?;
        return Ok(board
            .filter(|b| {
                b.embed_enabled.unwrap_or(false)
                    && !b.embed_access.is_withdrawn(
                        claims.iat.unwrap_or(f64::NEG_INFINITY),
                        claims.jti.as_deref(),
                    )
            })
            .map(|b| b.embed_access.origins)
            .unwrap_or_default());
    }
    if let Some(token) = body.token.filter(|s| !s.is_empty()) {
        let board = store::get_board_by_token(&state.clickhouse, &token).await?;
        return Ok(board.map(|b| b.embed_access.origins).unwrap_or_default());
    }
    Ok(Vec::new())
}

/// Body of `POST /api/dashboard/embed-revoke`.
#[derive(Debug, Default, Deserialize)]
struct RevokeBody {
    #[serde(default)]
    board: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

/// `POST /api/dashboard/embed-revoke` — withdraw ONE signed embed token of a
/// dashboard (`SEC-12`). Needs `dashboard:write`.
///
/// The caller presents the whole token, never a bare `jti`: the server
/// verifies the signature and reads the `jti` and `exp` itself, so nobody can
/// withdraw (or pre-withdraw) an id they do not hold a signed token for. A
/// token without a `jti` cannot be withdrawn singly, and the answer says so.
///
/// # Errors
///
/// `400` for a missing board or token, a token that does not verify (the
/// same fixed message as everywhere), a token for another dashboard, a token
/// with no `jti`, an unknown dashboard or a full withdrawal list; `503` when
/// `EMBED_SECRET` is unset; the usual classified store errors otherwise.
pub async fn revoke_token(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let parsed: RevokeBody = serde_json::from_slice(&body)
        .map_err(|_| ApiError::BadRequest("Body must be JSON {board, token}.".to_owned()))?;
    let board = parsed.board.as_deref().map(str::trim).unwrap_or_default();
    let token = parsed.token.as_deref().map(str::trim).unwrap_or_default();
    if board.is_empty() || board == store::DEFAULT_BOARD_ID || token.is_empty() {
        return Err(ApiError::BadRequest("board and token are required.".to_owned()).into());
    }
    let Some(secret) = state.config.embed_secret.as_deref() else {
        return Err(ApiError::Unavailable(EMBEDDING_NOT_CONFIGURED.to_owned()).into());
    };
    let now = lakehouse_embed::unix_now();
    let claims = lakehouse_embed::verify_embed(
        token,
        secret,
        state.config.embed_token_max_lifetime_seconds,
        now,
    )
    .map_err(|reason| {
        tracing::info!(%reason, "embed token to withdraw refused");
        ApiError::BadRequest(TOKEN_REFUSED.to_owned())
    })?;
    let for_board = claims
        .resource
        .as_ref()
        .and_then(|r| r.dashboard.as_deref());
    if for_board != Some(board) {
        return Err(
            ApiError::BadRequest("that token is for a different dashboard.".to_owned()).into(),
        );
    }
    let Some(jti) = claims.jti.as_deref() else {
        return Err(ApiError::BadRequest(
            "that token has no id (jti), so it cannot be withdrawn on its own; withdraw all the dashboard's embed tokens instead.".to_owned(),
        )
        .into());
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "exp is finite and within a day of now (verify_embed bounds it); rounded up, clamped at 0"
    )]
    let exp = claims.exp.unwrap_or(0.0).max(0.0).ceil() as u64;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "tolerance is the constant 60.0"
    )]
    let tolerance = lakehouse_embed::CLOCK_TOLERANCE_SECONDS as u64;
    store::withdraw_embed_token(
        &state.clickhouse,
        board,
        jti,
        exp,
        now_seconds(now),
        tolerance,
    )
    .await
    .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
    Ok(ApiJson(json!({ "ok": true, "withdrawn": jti })))
}

/// `now` (Unix seconds, fractional) as whole seconds, rounded down.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a Unix time in seconds is positive and far below u64::MAX"
)]
pub(crate) fn now_seconds(now: f64) -> u64 {
    now.max(0.0).floor() as u64
}

/// `paramsToFilters` in `embed/data/route.ts`: a signed embed's locked JWT
/// `params` (`{ col: val | [vals] }`) become dashboard filters the viewer
/// cannot override.
fn params_to_filters(params: Option<std::collections::HashMap<String, Value>>) -> Vec<FilterDef> {
    let Some(params) = params else {
        return Vec::new();
    };
    params
        .into_iter()
        .map(|(column, v)| {
            let values = match v {
                Value::Array(items) => items.iter().map(value_to_string).collect(),
                other => vec![value_to_string(&other)],
            };
            FilterDef::in_values(column, values)
        })
        .collect()
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `GET /api/public/dashboard/{token}` — read-only public share view.
pub async fn public_dashboard(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Response {
    let board = match store::get_board_by_token(&state.clickhouse, &token).await {
        Ok(b) => b,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
            )
                .into_response();
        }
    };
    let Some(board) = board else {
        return (
            StatusCode::NOT_FOUND,
            ApiJson(json!({ "error": "not_found" })),
        )
            .into_response();
    };

    let filters = board.filters.clone().unwrap_or_default();
    let board_id = board.id.clone();
    let time = match crate::routes::settings::time_context(&state).await {
        Ok(t) => t,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    let payload = render_board_payload(
        &state.clickhouse,
        &PolicyEngineObligations::from_state(&state),
        &board,
        &board_id,
        &filters,
        &time,
    )
    .await;
    match payload {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
        )
            .into_response(),
    }
}

/// Shared `{ board, layout, charts, results }` assembly for both
/// [`data`] and [`public_dashboard`]: every stored chart belonging to
/// `board_id`, filtered by `filters` (never by caller-supplied years —
/// neither route accepts a `year` parameter).
async fn render_board_payload(
    ch: &ChClient,
    obligations: &PolicyEngineObligations<'_>,
    board: &Board,
    board_id: &str,
    filters: &[FilterDef],
    time: &lakehouse_bi::grain::TimeContext,
) -> Result<Value, lakehouse_clickhouse::ChError> {
    let roles = [EMBED_VIEWER_ROLE.to_owned()];
    let placeholders = crate::sql_rewrite::PlaceholderValues::none();
    let stored = store::list_stored_charts(ch).await?;
    // BI-8: an embed or a public link shows a chart that uses a calculated
    // field exactly as the dashboard does (same builder, same role rewrite
    // with the embed viewer's role).
    //
    // PR #104 CI fix: a catalog that cannot be read must not take the whole
    // embed down (it used to render, with a per-tile fixed message for a tile
    // that failed). Like the dashboard route, a failed read leaves the catalog
    // empty: a chart that names a calculated field then fails on its own
    // statement with the fixed tile message and a reference, never showing a
    // wrong number, and the failure is logged under its own reference.
    let field_catalog = match lakehouse_bi::fields::list_fields(ch).await {
        Ok(found) => lakehouse_bi::fields::FieldCatalog::from_fields(found),
        Err(err) => {
            let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
            lakehouse_bi::fields::FieldCatalog::default()
        }
    };
    let stored_for_board: Vec<&StoredChartSpec> = stored
        .iter()
        .filter(|c| {
            if c.board.is_empty() {
                board_id == "default"
            } else {
                c.board == board_id
            }
        })
        .collect();

    // Column types are also what decides whether a chart's grain still fits
    // its dimension (BI-9).
    let need_cols = filters.iter().any(FilterDef::is_active)
        || stored_for_board
            .iter()
            .any(|c| c.def.needs_columns() || field_catalog.chart_uses(&c.def));
    let cols = if need_cols {
        mart_columns(ch).await?
    } else {
        std::collections::HashMap::new()
    };

    let mut results = serde_json::Map::new();
    let mut charts_out = Vec::with_capacity(stored_for_board.len());
    let sources = sources_for(ch, stored_for_board.iter().copied()).await?;
    // BI-9: embeds and public links offer no grain switch; they use the one
    // the board saved, and each chart's own where there is none.
    let read = lakehouse_bi::builder::ReadContext {
        time,
        grain: board
            .grain
            .as_deref()
            .and_then(lakehouse_bi::grain::Grain::parse)
            .filter(|g| g.is_truncation()),
        fields: &field_catalog,
    };
    for c in &stored_for_board {
        match stored_chart_sql(c, &[], filters, &cols, &sources, &read) {
            Ok(filtered) => {
                let (id, mut val) = run_spec_sql(
                    ch,
                    &c.spec.id,
                    &filtered.sql,
                    &roles,
                    &placeholders,
                    obligations,
                )
                .await;
                crate::routes::support::annotate_grain(&mut val, &filtered, c);
                // BI-16 part A: the first page of a raw table, with its
                // total; an embed and a public link offer no paging.
                crate::routes::support::annotate_table(
                    ch,
                    &mut val,
                    &filtered,
                    (&roles, &placeholders, obligations),
                )
                .await;
                results.insert(id, val);
            }
            Err(msg) => {
                results.insert(c.spec.id.clone(), json!({ "error": msg }));
            }
        }

        let mut rendered = render_stored_spec(&c.spec, c.source);
        rendered["board"] = json!(c.board);
        rendered["def"] = serde_json::to_value(&c.def).unwrap_or_else(|_| json!({}));
        // BI-18·B: embeds and public links have no click menu and no
        // destinations, so the editor's click setting (a board id, a column
        // or a URL) is not sent to someone who cannot use it.
        if let Some(def) = rendered["def"].as_object_mut() {
            def.remove("click");
        }
        charts_out.push(rendered);
    }

    let layout = board.layout.as_ref().map_or_else(
        || json!({}),
        |l| {
            let mut m = serde_json::Map::new();
            for (k, b) in l {
                m.insert(k.clone(), json!({ "x": b.x, "y": b.y, "w": b.w, "h": b.h }));
            }
            Value::Object(m)
        },
    );

    Ok(json!({
        "board": { "id": board.id, "name": board.name },
        "layout": layout,
        "charts": charts_out,
        "results": results,
        // BI-9: the zone and first day the buckets were cut with, so the
        // page labels them the same way.
        "reporting": { "timeZone": time.zone(), "weekStart": time.week_start().as_str() },
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn params_to_filters_none_yields_empty() {
        assert!(params_to_filters(None).is_empty());
    }

    #[test]
    fn params_to_filters_wraps_single_value_in_array() {
        let mut params = std::collections::HashMap::new();
        params.insert("kawasan".to_owned(), json!("Asia"));
        let filters = params_to_filters(Some(params));
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].column, "kawasan");
        assert_eq!(filters[0].values, vec!["Asia".to_owned()]);
    }

    #[test]
    fn params_to_filters_passes_through_array_values() {
        let mut params = std::collections::HashMap::new();
        params.insert("tahun".to_owned(), json!(["2023", "2024"]));
        let filters = params_to_filters(Some(params));
        assert_eq!(
            filters[0].values,
            vec!["2023".to_owned(), "2024".to_owned()]
        );
    }
}

#[cfg(test)]
mod typed_filters {
    //! A board saved with a typed filter renders publicly with it applied
    //! (BI-18): the public and embed paths share `render_board_payload`.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    async fn answer(server: &MockServer, has: &str, meta: &[&str], data: Vec<Value>) {
        let meta: Vec<Value> = meta
            .iter()
            .map(|n| json!({ "name": n, "type": "String" }))
            .collect();
        let rows = data.len();
        Mock::given(method("POST"))
            .and(body_string_contains(has))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": meta, "data": data, "rows": rows })),
            )
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn a_board_saved_with_a_typed_filter_renders_publicly_with_it_applied() {
        let server = MockServer::start().await;
        let spec = json!({
            "id": "c1", "title": "T", "kind": "bar", "mart": "mart_a",
            "sql": "SELECT 1", "x": "kab", "y": "visitors"
        });
        let def = json!({
            "title": "T", "mart": "mart_a", "kind": "bar",
            "dimension": "kab", "measures": ["visitors"],
            "click": { "kind": "url", "url": "/somewhere/{value}" }
        });
        answer(
            &server,
            "FROM console.bi_chart FINAL",
            &["id", "board", "created_by", "created_at", "spec_json"],
            vec![json!({
                "id": "c1", "board": "b1", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
                "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
            })],
        )
        .await;
        answer(
            &server,
            "SELECT table, name, type FROM system.columns",
            &["table", "name", "type"],
            vec![
                json!({ "table": "mart_a", "name": "kab", "type": "String" }),
                json!({ "table": "mart_a", "name": "visitors", "type": "UInt32" }),
            ],
        )
        .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let filters: Vec<FilterDef> = serde_json::from_str(
            r#"[{"column":"visitors","op":"between","min":"100","max":"900"},
                {"column":"kab","op":"contains","text":"ub"}]"#,
        )
        .unwrap();
        let board = Board {
            id: "b1".to_owned(),
            name: "B".to_owned(),
            description: None,
            created_by: None,
            layout: None,
            filters: Some(filters.clone()),
            refresh_seconds: None,
            grain: None,
            created_at: None,
            updated_at: None,
            public_token: None,
            embed_enabled: None,
            folder_id: None,
            embed_access: lakehouse_bi::embed_access::EmbedAccess::default(),
        };

        let body = render_board_payload(
            &ch,
            &PolicyEngineObligations::new(None, &ch),
            &board,
            "b1",
            &filters,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await
        .unwrap();

        assert_eq!(body["board"]["id"], "b1");
        // BI-18·B: the editor's click setting is not sent to an embed.
        assert!(body["charts"][0]["def"].get("click").is_none());
        assert_eq!(body["charts"][0]["def"]["dimension"], "kab");
        let sent: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        let tile = sent
            .iter()
            .find(|s| s.contains("FROM serving.mart_a WHERE"))
            .unwrap_or_else(|| panic!("tile query not sent: {sent:?}"));
        assert!(
            tile.contains("(visitors >= 100 AND visitors <= 900)"),
            "{tile}"
        );
        assert!(
            tile.contains("positionCaseInsensitiveUTF8(toString(kab), 'ub') > 0"),
            "{tile}"
        );
    }

    /// BI-9: an embed or public link has no grain switch; it uses the grain
    /// the board saved, in the deployment's time zone, and a grained chart is
    /// rebuilt rather than served from its stored SQL.
    #[tokio::test]
    async fn an_embed_uses_the_boards_saved_grain_and_never_the_stored_sql() {
        let server = MockServer::start().await;
        let spec = json!({
            "id": "c1", "title": "T", "kind": "line", "mart": "mart_a",
            "sql": "SELECT stored_sql_marker", "x": "seen_at", "y": "visitors"
        });
        let def = json!({
            "title": "T", "mart": "mart_a", "kind": "line", "dimension": "seen_at",
            "measures": ["visitors"], "limit": 20, "order": "none", "grain": "day"
        });
        answer(
            &server,
            "FROM console.bi_chart FINAL",
            &["id", "board", "created_by", "created_at", "spec_json"],
            vec![json!({
                "id": "c1", "board": "b1", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
                "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
            })],
        )
        .await;
        answer(
            &server,
            "SELECT table, name, type FROM system.columns",
            &["table", "name", "type"],
            vec![
                json!({ "table": "mart_a", "name": "seen_at", "type": "DateTime" }),
                json!({ "table": "mart_a", "name": "visitors", "type": "UInt32" }),
            ],
        )
        .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let board = Board {
            id: "b1".to_owned(),
            name: "B".to_owned(),
            description: None,
            created_by: None,
            layout: None,
            filters: None,
            refresh_seconds: None,
            grain: Some("month".to_owned()),
            created_at: None,
            updated_at: None,
            public_token: None,
            embed_enabled: None,
            folder_id: None,
            embed_access: lakehouse_bi::embed_access::EmbedAccess::default(),
        };
        let time = lakehouse_bi::grain::TimeContext::new(
            "Europe/Berlin",
            lakehouse_bi::grain::WeekStart::Monday,
        )
        .unwrap();
        render_board_payload(
            &ch,
            &PolicyEngineObligations::new(None, &ch),
            &board,
            "b1",
            &[],
            &time,
        )
        .await
        .unwrap();
        let sent: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        assert!(
            sent.iter().any(|s| s
                .contains("date_trunc('month', toTimeZone(seen_at, 'Europe/Berlin')) AS seen_at")),
            "{sent:?}"
        );
        assert!(
            sent.iter().all(|s| !s.contains("stored_sql_marker")),
            "{sent:?}"
        );
    }
}
