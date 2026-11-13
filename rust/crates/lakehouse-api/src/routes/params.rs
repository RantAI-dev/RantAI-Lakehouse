//! `GET/POST/PUT/DELETE /api/dashboard/params` — parameters (`BI-11`, with
//! `AI-6`): a named value a viewer changes on a dashboard and a formula, a
//! top-N, a reference line or a column switch reads.
//!
//! # Posture
//!
//! - A definition is checked by `lakehouse_bi::params::validate_def`; a value
//!   is checked against its definition by `check_value` wherever one arrives
//!   (`params` on the dashboard, preview, records and export routes, a board's
//!   saved values). These routes never run a statement over data.
//! - Reading needs `dashboard:read`; defining, changing and deleting need
//!   `dashboard:write` (feature page, decision 2). The name and the type are
//!   immutable once created, because charts and formulas refer to both.
//! - A parameter that a calculated field, a chart or a dashboard uses cannot
//!   be deleted; the refusal names them by title, like a field's.
//! - A `ClickHouse` failure is classified; its text never leaves the log.
//! - [`load`] is the one place a read turns a request's `params` into values.
//!   A store that cannot be read does not fail the read: the values are empty,
//!   so a chart that names a parameter shows its own plain sentence and every
//!   other tile is untouched (fail closed on data, degrade per tile). A value
//!   that does not fit is a 400 for every read but the dashboard page's own
//!   ([`OnRefusal::Fail`]); that one keeps the value in force and lists the
//!   refusal ([`OnRefusal::Report`], `BI-52`).

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Query, State};
use lakehouse_auth::Principal;
use lakehouse_bi::fields;
use lakehouse_bi::formula::compile::params_in;
use lakehouse_bi::params::{self, BoardParam, ParamDef, ParamValues};
use lakehouse_bi::store;
use lakehouse_clickhouse::ChClient;
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::calc_fields::{author_name, classify_ch};
use crate::routes::dashboard_access::require_entitled;
use crate::state::AppState;
use crate::upstream_error;

/// What a read loaded for its parameters.
pub(crate) struct Loaded {
    /// Every definition (empty when the store could not be read).
    pub defs: Vec<ParamDef>,
    /// The value each has for this read.
    pub values: ParamValues,
    /// The parameters whose requested value did not fit (`BI-52`, plan D26):
    /// empty unless the read was loaded with [`OnRefusal::Report`].
    pub refused: Vec<params::RefusedParam>,
    /// The classified message of a failed read of the definitions, for the
    /// payload's `storeError`.
    pub store_error: Option<String>,
}

/// What a read does with a parameter value that does not fit its definition
/// (`BI-52`, plan D23).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnRefusal {
    /// Refuse the whole read with a 400: a document that silently used another
    /// value would be a different document, and a read that decides something
    /// from the numbers must not decide from a value nobody asked for.
    Fail,
    /// Keep the read: the parameter takes the value it has without the
    /// request, and the refusal is listed in [`Loaded::refused`] so the page
    /// can say why and draw the rest.
    Report,
}

/// The parameter values of one read: the request's `params` (JSON text),
/// else the board's saved values, else each default.
///
/// `on_refusal` decides what a value that does not fit does: a 400 for the
/// whole read, or the value in force plus an entry in [`Loaded::refused`]
/// (`BI-52`, plan D23–D26). A name no definition has is a 400 for a strict
/// read and ignored for a reported one (plan D25).
///
/// # Errors
///
/// 400 with a fixed sentence when `request` is not a JSON object, and when a
/// value does not fit its definition, or a name no definition has, while
/// `on_refusal` is [`OnRefusal::Fail`] (the sentence names the parameter,
/// never repeats the value).
pub(crate) async fn load(
    ch: &ChClient,
    saved: &[BoardParam],
    request: Option<&str>,
    on_refusal: OnRefusal,
) -> Result<Loaded, ApiError> {
    let sent = params::parse_request(request).map_err(ApiError::BadRequest)?;
    match params::list_params(ch).await {
        Ok(defs) => match on_refusal {
            OnRefusal::Fail => {
                let values = ParamValues::resolve(&defs, saved, sent.as_ref())
                    .map_err(ApiError::BadRequest)?;
                Ok(Loaded {
                    defs,
                    values,
                    refused: Vec::new(),
                    store_error: None,
                })
            }
            OnRefusal::Report => {
                let outcome = ParamValues::resolve_each(&defs, saved, sent.as_ref());
                // A name no definition has (a link kept after a parameter was
                // deleted) is ignored here, as decision D25 says: the address
                // is not a request the person meant to make a claim with.
                Ok(Loaded {
                    defs,
                    values: outcome.values,
                    refused: outcome.refused,
                    store_error: None,
                })
            }
        },
        Err(err) => Ok(Loaded {
            defs: Vec::new(),
            values: ParamValues::default(),
            refused: Vec::new(),
            store_error: Some(
                upstream_error::report_ch(&upstream_error::DATABASE, &err).to_string(),
            ),
        }),
    }
}

/// `time` with every parameter at its default: what a formula is checked and
/// a field is saved against. A store that cannot be read leaves no parameters,
/// so a formula that names one fails with its own positioned sentence.
pub(crate) async fn with_defaults(
    ch: &ChClient,
    time: lakehouse_bi::grain::TimeContext,
) -> lakehouse_bi::grain::TimeContext {
    match params::list_params(ch).await {
        Ok(defs) => time.with_params(ParamValues::from_defaults(&defs)),
        Err(err) => {
            let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
            time
        }
    }
}

/// `GET` response row for one definition.
fn listing(defs: &[ParamDef]) -> Value {
    json!({ "params": defs })
}

/// The route's handler: [`list`] behind the entry check of `BI-19` part B1
/// (plan D8). The router floor is "signed in"; this refuses somebody who holds
/// neither `dashboard:read` nor a rule naming them or one of their roles. Kept
/// apart from [`list`] because the assistant calls that one directly, after
/// its own gate.
pub async fn list_guarded(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    require_entitled(&state, &principal).await?;
    list(State(state)).await
}

/// `GET /api/dashboard/params` — every parameter.
///
/// # Errors
///
/// A fixed 422/503 for a database failure.
pub async fn list(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let defs = params::list_params(&state.clickhouse)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(listing(&defs)))
}

/// `POST` / `PUT` body: a definition; `PUT` names the parameter by `id`.
fn parse_def(body: &Bytes) -> Result<ParamDef, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_unparsed| ApiError::BadRequest("body JSON is invalid".to_owned()))
}

/// `POST /api/dashboard/params` — define a parameter.
///
/// # Errors
///
/// 400 for a bad body or definition, 409 for a name already used, a fixed
/// 422/503 for a database failure.
pub async fn create(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let ch = &state.clickhouse;
    let parsed = parse_def(&body)?;
    let mut def = params::validate_def(&parsed).map_err(ApiError::BadRequest)?;
    let existing = params::list_params(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    if existing.iter().any(|p| p.name == def.name) {
        return Err(ApiError::Conflict(format!(
            "a parameter called '{}' already exists.",
            def.name
        ))
        .into());
    }
    if existing.len() >= params::MAX_PARAMS {
        return Err(ApiError::BadRequest(format!(
            "a deployment has at most {} parameters.",
            params::MAX_PARAMS
        ))
        .into());
    }
    def.id = params::new_id();
    def.created_by = author_name(principal.as_ref());
    def.updated_at = None;
    params::save_param(ch, &def)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true, "param": def })))
}

/// `PUT /api/dashboard/params` — change a parameter's label, default, bounds
/// or values. Its name and type stay.
///
/// # Errors
///
/// 400 for a bad body or definition, or a changed name or type; 404 for an
/// unknown id.
pub async fn update(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let ch = &state.clickhouse;
    let parsed = parse_def(&body)?;
    if parsed.id.trim().is_empty() {
        return Err(ApiError::BadRequest("id is required.".to_owned()).into());
    }
    let existing = params::list_params(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    let current = existing
        .iter()
        .find(|p| p.id == parsed.id)
        .ok_or_else(|| ApiError::NotFound("parameter not found.".to_owned()))?;
    if parsed.name.trim() != current.name || parsed.ty != current.ty {
        return Err(ApiError::BadRequest(
            "the name and the type of a parameter cannot be changed.".to_owned(),
        )
        .into());
    }
    let mut def = params::validate_def(&parsed).map_err(ApiError::BadRequest)?;
    def.id.clone_from(&current.id);
    def.created_by = author_name(principal.as_ref());
    def.updated_at = None;
    params::save_param(ch, &def)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true, "param": def })))
}

/// `DELETE` query.
#[derive(Debug, Deserialize)]
pub struct IdQuery {
    #[serde(default)]
    id: Option<String>,
}

/// What uses a parameter, by name for a person: the calculated fields whose
/// formula names it, the charts that name it, the dashboards that show it.
fn users_of(
    name: &str,
    all_fields: &[fields::FieldDef],
    charts: &[store::StoredChartSpec],
    boards: &[store::Board],
) -> Vec<String> {
    let mut used: Vec<String> = Vec::new();
    for f in all_fields.iter().filter(|f| !f.is_group()) {
        if params_in(&f.formula).contains(name) {
            used.push(format!("the calculated field '{}'", f.name));
        }
    }
    let ids = params::charts_using(charts, name);
    used.extend(fields::name_charts(charts, boards, &ids));
    for b in boards {
        if b.params.iter().flatten().any(|p| p.name == name) {
            used.push(format!("the dashboard \"{}\"", b.name));
        }
    }
    used
}

/// `DELETE /api/dashboard/params?id=` — remove a parameter that nothing uses.
///
/// # Errors
///
/// 400 without an id, 404 for an unknown one, 409 naming what still uses it.
pub async fn delete(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Query(q): Query<IdQuery>,
) -> ApiResult<ApiJson<Value>> {
    let ch = &state.clickhouse;
    let id =
        q.id.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ApiError::BadRequest("id is required.".to_owned()))?;
    let existing = params::list_params(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    let current = existing
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| ApiError::NotFound("parameter not found.".to_owned()))?;
    // A store that cannot be read refuses the delete: nothing can be shown to
    // be unused.
    // BI-19 B1 review R3: the resolved viewer, not the baseline.
    let viewer = crate::routes::dashboard_access::usage_viewer(&state, principal.as_ref()).await?;
    let system = lakehouse_bi::access::Viewer::system();
    let (all_fields, seen, default_row) = tokio::try_join!(
        fields::list_fields(ch),
        crate::routes::dashboard_organise::Seen::read(ch, &viewer),
        store::get_board(ch, store::DEFAULT_BOARD_ID, &system),
    )
    .map_err(|failure| classify_ch(&failure))?;
    // BI-18 part C, part 2: what the caller may not see still blocks the
    // delete but is not named.
    let mut shown = seen.boards.clone();
    shown.extend(default_row);
    let mut used = users_of(&current.name, &all_fields, &seen.charts, &shown);
    if !users_of(
        &current.name,
        &[],
        &seen.hidden_charts(),
        &seen.hidden_boards(),
    )
    .is_empty()
    {
        used.push(crate::routes::dashboard_organise::HIDDEN_USE.to_owned());
    }
    if !used.is_empty() {
        return Err(ApiError::Conflict(format!(
            "the parameter '{}' is used by {}. Remove it there first.",
            current.name,
            used.join(", ")
        ))
        .into());
    }
    params::delete_param(ch, &current.id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true })))
}

/// A board's `params` as saved: every name a defined parameter, every value
/// one that fits its definition. Returns the list with each value in its
/// canonical form (a number as a number, a date as `YYYY-MM-DD`).
///
/// # Errors
///
/// 400 with a fixed sentence for an unknown name, a repeated one, or a value
/// that does not fit.
pub(crate) async fn check_board_params(
    ch: &ChClient,
    wanted: &[BoardParam],
) -> Result<Vec<BoardParam>, ApiError> {
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let defs = params::list_params(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    let mut out: Vec<BoardParam> = Vec::new();
    for p in wanted {
        let def = defs.iter().find(|d| d.name == p.name).ok_or_else(|| {
            ApiError::BadRequest("a dashboard parameter does not exist.".to_owned())
        })?;
        if out.iter().any(|o| o.name == p.name) {
            return Err(ApiError::BadRequest(
                "a parameter is listed twice on the dashboard.".to_owned(),
            ));
        }
        let value = match p.value.as_ref().filter(|v| !v.is_null()) {
            Some(raw) => Some(
                params::check_value(def, raw)
                    .map_err(ApiError::BadRequest)?
                    .to_json(),
            ),
            None => None,
        };
        out.push(BoardParam {
            name: p.name.clone(),
            value,
        });
    }
    Ok(out)
}

/// [`check_board_params`] for an edit that may not carry `params`.
///
/// # Errors
///
/// As [`check_board_params`].
pub(crate) async fn check_board_params_opt(
    ch: &ChClient,
    wanted: Option<&[BoardParam]>,
) -> Result<Option<Vec<BoardParam>>, ApiError> {
    match wanted {
        Some(list) => check_board_params(ch, list).await.map(Some),
        None => Ok(None),
    }
}

/// The values of a read that has no request: the board's saved ones, else the
/// defaults (embeds and public links). A store that cannot be read leaves
/// none, logged under a reference id, so only a chart that names a parameter
/// shows an error.
pub(crate) async fn saved_or_default(ch: &ChClient, board: &store::Board) -> ParamValues {
    let saved = board.params.clone().unwrap_or_default();
    match params::list_params(ch).await {
        Ok(defs) => ParamValues::resolve(&defs, &saved, None).unwrap_or_default(),
        Err(err) => {
            let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
            ParamValues::default()
        }
    }
}

/// The controls a board shows, with the value of this read, in the board's
/// order. A name whose definition is gone is left out.
pub(crate) fn controls(shown: &[BoardParam], values: &ParamValues) -> Value {
    Value::Array(
        shown
            .iter()
            .filter_map(|b| values.get(&b.name))
            .map(params::control_json)
            .collect(),
    )
}

/// The value each shown parameter has without a request: the board's saved
/// one, else the default. The console compares it with the current value to
/// offer "Save as default" and Reset.
pub(crate) fn saved_values(defs: &[ParamDef], shown: &[BoardParam]) -> Value {
    let by_default = ParamValues::resolve(defs, shown, None).unwrap_or_default();
    let mut out = Map::new();
    for b in shown {
        if let Some(r) = by_default.get(&b.name) {
            out.insert(b.name.clone(), r.value.to_json());
        }
    }
    Value::Object(out)
}
