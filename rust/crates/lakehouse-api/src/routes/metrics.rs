//! `GET/POST/PUT/DELETE /api/dashboard/metrics`, `POST
//! /api/dashboard/metrics/validate`, `GET /api/dashboard/metrics/{id}/usage`,
//! and `GET/POST/PUT/DELETE /api/dashboard/segments` — reusable metrics and
//! segments (`BI-2`, with `AI-7`).
//!
//! # Posture
//!
//! - **Who may do what.** Reading, validating and asking for the usage need
//!   `dashboard:read`; creating, changing and deleting a metric or a segment
//!   needs `metric:write`, a permission introduced by the rules in the policy
//!   module doc (no seeded role holds it; Platform Admin's `*:*` satisfies it).
//!   Using a metric in a chart needs nothing beyond seeing the dashboard.
//! - **A metric lends no access.** Nothing stored with a metric records its
//!   author's roles and nothing here reads them. A metric is only text (a
//!   formula, filters, a source); when a chart, the live value or the
//!   assistant's `query_metric` reads it, the statement it ends up in goes
//!   through the guard and the role rewrite of the *caller* (`value_query`
//!   below runs the caller's roles, placeholders and obligations), so a mask,
//!   a row filter or a refused table applies to the viewer exactly as it does
//!   to a chart that does not use a metric.
//! - **Immutable parts.** A metric's `name` and its source cannot change (charts
//!   store the name); a segment's too. Delete is refused with 409 while
//!   anything uses it, naming what, so a chart never loses its number.
//! - **Errors.** A mistake in the person's own definition gets our sentence
//!   with a character position; a `ClickHouse` failure is classified and its
//!   text stays in the log (`AGENTS.md` principle 4).

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use lakehouse_auth::Principal;
use lakehouse_bi::builder::{ReadContext, RelationColumns};
use lakehouse_bi::fields::{self, FieldCatalog, FieldDef, SourceKind};
use lakehouse_bi::filters::FilterDef;
use lakehouse_bi::metrics::{self, MetricContext, MetricDef, SegmentDef, ValueQuery};
use lakehouse_bi::store;
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::policy_engine::PolicyEngineObligations;
use crate::routes::calc_fields::{Resolved, author_name, resolve};
use crate::state::AppState;
use crate::upstream_error;

/// Fixed words for a metric read or write that failed (SEC-11).
const METRIC: upstream_error::Context = upstream_error::Context::new(
    "metric",
    "The metric could not be read or saved.",
    "ClickHouse is unavailable.",
);

pub(crate) fn classify_ch(failure: &ChError) -> ApiError {
    upstream_error::ch_error(&METRIC, failure)
}

/// `GET` query: a source to narrow to, or none for every metric.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    mart: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

/// `DELETE` query.
#[derive(Debug, Deserialize)]
pub struct IdQuery {
    #[serde(default)]
    id: Option<String>,
}

/// `POST` / `PUT` / `validate` body of a metric.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricBody {
    /// `PUT`: the metric to change.
    #[serde(default)]
    id: Option<String>,
    /// `POST` and `validate`: the mart, or ...
    #[serde(default)]
    mart: Option<String>,
    /// ... the SQL source id.
    #[serde(default)]
    source: Option<String>,
    /// `POST`: the name (kept for good once saved).
    #[serde(default)]
    name: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    formula: String,
    #[serde(default)]
    filters: Vec<FilterDef>,
    #[serde(default)]
    time_column: Option<String>,
    #[serde(default)]
    grain: Option<String>,
    #[serde(default)]
    format: Option<String>,
    /// `validate`: also read the number as the caller.
    #[serde(default)]
    value: bool,
}

/// `POST` / `PUT` body of a segment.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    mart: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    filters: Vec<FilterDef>,
}

fn parse<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_unparsed| ApiError::BadRequest("body JSON is invalid".to_owned()))
}

fn blank(s: Option<&String>) -> Option<&str> {
    s.map(String::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

async fn resolve_current(ch: &ChClient, kind: SourceKind, id: &str) -> Result<Resolved, ApiError> {
    match kind {
        SourceKind::Mart => resolve(ch, Some(id), None).await,
        SourceKind::SqlSource => resolve(ch, None, Some(id)).await,
    }
}

/// The metrics and segments of one source as the checks need them.
async fn of_source(
    ch: &ChClient,
    kind: SourceKind,
    id: &str,
) -> Result<(Vec<FieldDef>, Vec<SegmentDef>), ApiError> {
    let fields = fields::list_fields_for(ch, kind, id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    let segments = metrics::list_segments(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?
        .into_iter()
        .filter(|s| s.source_kind == kind && s.source_id == id)
        .collect();
    Ok((fields, segments))
}

fn unprocessable(problem: &metrics::MetricProblem) -> ApiError {
    ApiError::Unprocessable(format!("{problem}"))
}

/// What a metric's name is used by, as sentences a refusal carries, and as
/// structured lists for the usage route.
struct Usage {
    /// BI-2: alert rules that watch the metric, as `(id, name)`.
    alerts: Vec<(String, String)>,
    charts: Vec<String>,
    chart_rows: Vec<Value>,
    metrics: Vec<String>,
    dashboards: Vec<String>,
}

impl Usage {
    fn is_empty(&self) -> bool {
        self.charts.is_empty() && self.metrics.is_empty() && self.alerts.is_empty()
    }

    fn sentence(&self) -> String {
        let mut all = self.charts.clone();
        all.extend(self.metrics.iter().map(|m| format!("the metric {m}")));
        all.extend(
            self.alerts
                .iter()
                .map(|(_, n)| format!("the alert rule \"{n}\"")),
        );
        all.join(", ")
    }
}

async fn usage_of(
    ch: &ChClient,
    pick: impl Fn(&[store::StoredChartSpec]) -> Vec<String>,
    metrics_naming: Vec<String>,
    viewer: &lakehouse_bi::access::Viewer,
) -> Result<Usage, ApiError> {
    // BI-18 part C, part 2: a chart on somebody's personal board still counts
    // as a use (deleting would break it) but is neither named nor listed.
    let seen = crate::routes::dashboard_organise::Seen::read(ch, viewer)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    let hidden_use = !pick(&seen.hidden_charts()).is_empty();
    let (charts, boards) = (seen.charts, seen.boards);
    let ids = pick(&charts);
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let board_name = |id: &str| {
        if id.is_empty() || id == store::DEFAULT_BOARD_ID {
            "Main".to_owned()
        } else {
            boards
                .iter()
                .find(|b| b.id == id)
                .map_or_else(String::new, |b| b.name.clone())
        }
    };
    let mut chart_rows = Vec::new();
    let mut dashboards: Vec<String> = Vec::new();
    for c in charts.iter().filter(|c| ids.contains(&c.spec.id)) {
        let board = board_name(&c.board);
        chart_rows.push(json!({
            "id": c.spec.id,
            "title": c.spec.title,
            "kind": c.spec.kind,
            "board": c.board,
            "boardName": board,
        }));
        if !board.is_empty() && !dashboards.contains(&board) {
            dashboards.push(board);
        }
    }
    dashboards.sort();
    let mut named = fields::name_charts(&charts, &boards, &id_refs);
    if hidden_use {
        named.push(crate::routes::dashboard_organise::HIDDEN_USE.to_owned());
    }
    Ok(Usage {
        alerts: Vec::new(),
        charts: named,
        chart_rows,
        metrics: metrics_naming,
        dashboards,
    })
}

async fn metric_usage(
    ch: &ChClient,
    m: &MetricDef,
    viewer: &lakehouse_bi::access::Viewer,
) -> Result<Usage, ApiError> {
    let (kind, source, name) = (m.source_kind, m.source_id.clone(), m.name.clone());
    // BI-2: a rule that watches the metric uses it too. A rule list that cannot
    // be read refuses the delete (nothing can be shown to be unused).
    let alerts: Vec<(String, String)> = lakehouse_alerts::list_rules(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?
        .into_iter()
        .filter(|r| r.metric.as_deref() == Some(m.name.as_str()))
        .map(|r| (r.id, r.name))
        .collect();
    // BI-2 review fix (SHOULD-FIX) R2: the usage applies the reader's rule for
    // a measure without the marker, which needs the source's fields and
    // columns. A source that cannot be read leaves the columns empty: more
    // charts count as users, so a delete is refused, never wrongly allowed.
    let (fields, _) = of_source(ch, kind, &source).await?;
    let cols = resolve_current(ch, kind, &source)
        .await
        .map(|r| r.cols)
        .unwrap_or_default();
    let mut used = usage_of(
        ch,
        move |charts| {
            metrics::charts_using_metric(charts, (kind, &source), (&fields, &cols), &name)
                .into_iter()
                .map(str::to_owned)
                .collect()
        },
        Vec::new(),
        viewer,
    )
    .await?;
    used.alerts = alerts;
    Ok(used)
}

async fn segment_usage(
    ch: &ChClient,
    s: &SegmentDef,
    viewer: &lakehouse_bi::access::Viewer,
) -> Result<Usage, ApiError> {
    let (kind, source, name) = (s.source_kind, s.source_id.clone(), s.name.clone());
    let named_by: Vec<String> = metrics::list_metrics(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?
        .into_iter()
        .filter(|m| {
            m.source_kind == s.source_kind
                && m.source_id == s.source_id
                && metrics::segment_names(&m.filters).any(|n| n == s.name)
        })
        .map(|m| m.name)
        .collect();
    usage_of(
        ch,
        move |charts| {
            metrics::charts_using_segment(charts, kind, &source, &name)
                .into_iter()
                .map(str::to_owned)
                .collect()
        },
        named_by,
        viewer,
    )
    .await
}

/// The fields and columns of each source the metrics sit on, for the usage
/// rule (`BI-2` review fix (SHOULD-FIX) R2). A source that cannot be read has
/// none of either, as for the usage route: more charts count as users.
type SourceRead = ((SourceKind, String), (Vec<FieldDef>, RelationColumns));

async fn source_reads(ch: &ChClient, shown: &[MetricDef]) -> Vec<SourceRead> {
    let mut read: Vec<SourceRead> = Vec::new();
    for m in shown {
        let key = (m.source_kind, m.source_id.clone());
        if read.iter().any(|(k, _)| *k == key) {
            continue;
        }
        let fields = fields::list_fields_for(ch, m.source_kind, &m.source_id)
            .await
            .unwrap_or_default();
        let cols = resolve_current(ch, m.source_kind, &m.source_id)
            .await
            .map(|r| r.cols)
            .unwrap_or_default();
        read.push((key, (fields, cols)));
    }
    read
}

/// `GET /api/dashboard/metrics[?mart=|source=]` — the metrics, each with the
/// number of charts that use it and its source's title.
///
/// # Errors
///
/// A fixed 422/503 for a database failure.
pub async fn list(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Query(q): Query<ListQuery>,
) -> ApiResult<ApiJson<Value>> {
    let ch = &state.clickhouse;
    let all = metrics::list_metrics(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    let narrow = match (blank(q.mart.as_ref()), blank(q.source.as_ref())) {
        (Some(m), None) => Some((
            SourceKind::Mart,
            m.strip_prefix("serving.").unwrap_or(m).to_owned(),
        )),
        (None, Some(s)) => Some((SourceKind::SqlSource, s.to_owned())),
        _ => None,
    };
    let shown: Vec<MetricDef> = all
        .into_iter()
        .filter(|m| {
            narrow
                .as_ref()
                .is_none_or(|(k, id)| m.source_kind == *k && m.source_id == *id)
        })
        .collect();
    // The counts and the SQL sources' titles are a courtesy: when they cannot
    // be read the list still answers.
    // The count is of the charts the caller may see (BI-18 part C, part 2).
    let charts = store::list_stored_charts(
        ch,
        &crate::routes::calc_fields::viewer_or_anonymous(principal.as_ref()),
    )
    .await
    .unwrap_or_default();
    let titles: std::collections::HashMap<String, String> = lakehouse_bi::sources::list_sources(ch)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|s| (s.id, s.title))
        .collect();
    // R2: the count follows the same rule as the usage route.
    let read = source_reads(ch, &shown).await;
    let rows: Vec<Value> = shown
        .iter()
        .map(|m| {
            let empty = (Vec::new(), RelationColumns::default());
            let (fields, cols) = read
                .iter()
                .find(|(k, _)| k.0 == m.source_kind && k.1 == m.source_id)
                .map_or(&empty, |(_, v)| v);
            let used = metrics::charts_using_metric(
                &charts,
                (m.source_kind, &m.source_id),
                (fields, cols),
                &m.name,
            )
            .len();
            let title = match m.source_kind {
                SourceKind::Mart => m.source_id.clone(),
                SourceKind::SqlSource => titles
                    .get(&m.source_id)
                    .cloned()
                    .unwrap_or_else(|| m.source_id.clone()),
            };
            let mut row = serde_json::to_value(m).unwrap_or_else(|_| json!({}));
            row["usedBy"] = json!(used);
            row["sourceTitle"] = json!(title);
            row
        })
        .collect();
    Ok(ApiJson(json!({ "metrics": rows })))
}

/// The checks every save and validate shares: the source, the name, the
/// definition. Returns the metric to save and the source it was checked on.
async fn checked(
    state: &AppState,
    def: MetricDef,
    source: &Resolved,
) -> Result<(MetricDef, metrics::CheckedMetric), ApiError> {
    let ch = &state.clickhouse;
    let time = crate::routes::settings::time_context(state).await?;
    let time = crate::routes::params::with_defaults(ch, time).await;
    let (fields, segments) = of_source(ch, source.kind, &source.id).await?;
    let ctx = MetricContext {
        cols: &source.cols,
        fields: &fields,
        segments: &segments,
        time: &time,
    };
    let ok = metrics::check_metric(&def, &ctx).map_err(|problem| unprocessable(&problem))?;
    Ok((def, ok))
}

fn definition(body: MetricBody, source: &Resolved, name: String, id: String) -> MetricDef {
    MetricDef {
        id,
        name,
        label: body.label.trim().to_owned(),
        description: body.description.trim().to_owned(),
        source_kind: source.kind,
        source_id: source.id.clone(),
        formula: body.formula,
        filters: body.filters,
        time_column: blank(body.time_column.as_ref()).map(str::to_owned),
        grain: blank(body.grain.as_ref()).map(str::to_owned),
        format: blank(body.format.as_ref()).map(str::to_owned),
        created_by: String::new(),
        updated_at: None,
    }
}

/// `POST /api/dashboard/metrics` — save a new metric.
///
/// # Errors
///
/// 400 for a bad body, name or source; 409 for a name already used by a metric
/// of the deployment; 422 for a definition that does not check out; a fixed
/// 422/503 for a database failure.
pub async fn create(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: MetricBody = parse(&body)?;
    let ch = &state.clickhouse;
    let source = resolve(ch, parsed.mart.as_deref(), parsed.source.as_deref()).await?;
    let name = parsed.name.trim().to_owned();
    metrics::validate_name(&name, &source.cols).map_err(ApiError::BadRequest)?;
    let existing = metrics::list_metrics(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    if existing.iter().any(|m| m.name == name) {
        return Err(ApiError::Conflict(format!("a metric called '{name}' already exists.")).into());
    }
    if existing.len() >= metrics::MAX_PER_KIND {
        return Err(ApiError::BadRequest(format!(
            "a deployment has at most {} metrics.",
            metrics::MAX_PER_KIND
        ))
        .into());
    }
    let def = definition(parsed, &source, name, metrics::new_metric_id());
    let (mut def, _ok) = checked(&state, def, &source).await?;
    def.created_by = author_name(principal.as_ref());
    metrics::save_metric(ch, &def)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true, "metric": def })))
}

/// `PUT /api/dashboard/metrics` — change a metric (`id` plus the definition).
/// The name and the source stay: charts refer to them.
///
/// # Errors
///
/// 400 for a bad body, 404 for an unknown id, 422 for a definition that does
/// not check out.
pub async fn update(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: MetricBody = parse(&body)?;
    let ch = &state.clickhouse;
    let id = blank(parsed.id.as_ref())
        .ok_or_else(|| ApiError::BadRequest("id is required.".to_owned()))?
        .to_owned();
    let current = metrics::get_metric(ch, &id)
        .await
        .map_err(|failure| classify_ch(&failure))?
        .ok_or_else(|| ApiError::NotFound(format!("metric '{id}' not found.")))?;
    let source = resolve_current(ch, current.source_kind, &current.source_id).await?;
    let def = definition(parsed, &source, current.name.clone(), current.id.clone());
    let (mut def, _ok) = checked(&state, def, &source).await?;
    def.created_by = author_name(principal.as_ref());
    metrics::save_metric(ch, &def)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true, "metric": def })))
}

/// `DELETE /api/dashboard/metrics?id=` — remove a metric nothing uses.
///
/// # Errors
///
/// 400 without an id, 404 for an unknown one, 409 (naming what uses it) while
/// a chart uses it.
pub async fn delete(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Query(q): Query<IdQuery>,
) -> ApiResult<ApiJson<Value>> {
    let ch = &state.clickhouse;
    let id =
        blank(q.id.as_ref()).ok_or_else(|| ApiError::BadRequest("id is required.".to_owned()))?;
    let current = metrics::get_metric(ch, id)
        .await
        .map_err(|failure| classify_ch(&failure))?
        .ok_or_else(|| ApiError::NotFound(format!("metric '{id}' not found.")))?;
    let used = metric_usage(
        ch,
        &current,
        &crate::routes::calc_fields::viewer_or_anonymous(principal.as_ref()),
    )
    .await?;
    if !used.is_empty() {
        return Err(ApiError::Conflict(format!(
            "the metric is still used by {}.",
            used.sentence()
        ))
        .into());
    }
    metrics::delete_metric(ch, &current.id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true })))
}

/// `GET /api/dashboard/metrics/{id}/usage` — what uses the metric, by title.
///
/// # Errors
///
/// 404 for an unknown id; a fixed 422/503 for a database failure.
pub async fn usage(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let ch = &state.clickhouse;
    let current = metrics::get_metric(ch, &id)
        .await
        .map_err(|failure| classify_ch(&failure))?
        .ok_or_else(|| ApiError::NotFound("metric not found.".to_owned()))?;
    let used = metric_usage(
        ch,
        &current,
        &crate::routes::calc_fields::viewer_or_anonymous(principal.as_ref()),
    )
    .await?;
    Ok(ApiJson(json!({
        "metric": current.name,
        "charts": used.chart_rows,
        "alerts": used.alerts.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
        "dashboards": used.dashboards,
    })))
}

/// `POST /api/dashboard/metrics/validate` — check a definition and say what it
/// would be, or where it is wrong. Saves nothing. With `value: true` it also
/// reads the number as the caller (their roles, masks and row filters).
///
/// # Errors
///
/// 400 for a bad body or source; a mistake in the definition is a normal
/// `200` with `ok: false`.
pub async fn validate(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: MetricBody = parse(&body)?;
    let want_value = parsed.value;
    let ch = &state.clickhouse;
    let source = resolve(ch, parsed.mart.as_deref(), parsed.source.as_deref()).await?;
    let own = parsed.name.trim().to_owned();
    let name = if lakehouse_core::ident::Ident::new(own.as_str()).is_ok() {
        own
    } else {
        "metric".to_owned()
    };
    let def = definition(parsed, &source, name, String::new());
    let (def, _ok) = match checked(&state, def, &source).await {
        Ok(found) => found,
        Err(ApiError::Unprocessable(message)) => {
            return Ok(ApiJson(
                json!({ "ok": false, "error": { "message": message } }),
            ));
        }
        Err(other) => return Err(other.into()),
    };
    let mut out = json!({ "ok": true });
    if want_value && let Some(Extension(principal)) = principal {
        // The candidate stands in the catalog beside the source's saved fields.
        let (fields, segments) = of_source(ch, source.kind, &source.id).await?;
        let mut catalog_fields = fields;
        catalog_fields.extend(metrics::catalog_entries(
            std::slice::from_ref(&def),
            &segments,
        ));
        let query = ValueQuery {
            metric: &def.name,
            source_kind: source.kind,
            source_id: &source.id,
            group_by: None,
            grain: None,
            filters: Vec::new(),
        };
        out["value"] = value_query(&state, &principal, &source, catalog_fields, &query).await?;
    }
    Ok(ApiJson(out))
}

/// The number of a metric (or its numbers by one column or a grain) read as
/// `principal`: the synthetic chart is built by the ordinary builder and its
/// statement goes through the guard and the role rewrite of the caller's roles.
/// `catalog_fields` holds the source's fields with the metric (saved or
/// candidate) among them. The tile's own JSON comes back, with its `error` and
/// reference when the read failed.
pub(crate) async fn value_query(
    state: &AppState,
    principal: &Principal,
    source: &Resolved,
    catalog_fields: Vec<FieldDef>,
    query: &ValueQuery<'_>,
) -> Result<Value, ApiError> {
    use std::collections::HashMap;
    let ch = &state.clickhouse;
    let time = crate::routes::settings::time_context(state).await?;
    let time = crate::routes::params::with_defaults(ch, time).await;
    let catalog = FieldCatalog::from_fields(catalog_fields);
    let read = ReadContext {
        time: &time,
        grain: None,
        fields: &catalog,
    };
    let chart = metrics::value_chart(query);
    let (marts, sources): (HashMap<String, RelationColumns>, _) = match source.kind {
        SourceKind::Mart => (
            HashMap::from([(source.id.clone(), source.cols.clone())]),
            HashMap::new(),
        ),
        SourceKind::SqlSource => {
            let found = lakehouse_bi::sources::get_source(ch, &source.id)
                .await
                .map_err(|failure| classify_ch(&failure))?
                .ok_or_else(|| ApiError::NotFound("the SQL source no longer exists.".to_owned()))?;
            (HashMap::new(), HashMap::from([(found.id.clone(), found)]))
        }
    };
    let filtered =
        crate::routes::support::stored_chart_sql(&chart, &[], &[], &marts, &sources, &read)
            .map_err(|message| ApiError::Unprocessable(message.to_owned()))?;
    if filtered.sql.is_empty() {
        return Err(ApiError::Unprocessable(
            "this metric cannot be read as asked.".to_owned(),
        ));
    }
    let placeholders = crate::sql_rewrite::PlaceholderValues {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_tenant_ids: principal
            .tenant_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
    };
    let obligations = PolicyEngineObligations::from_state(state);
    let (_, mut answer) = crate::routes::support::run_spec_sql(
        ch,
        &chart.spec.id,
        &filtered.sql,
        &principal.role_names,
        &placeholders,
        &obligations,
    )
    .await;
    crate::routes::support::annotate_grain(&mut answer, &filtered, &chart);
    Ok(answer)
}

// ── segments ────────────────────────────────────────────────────────────

/// `GET /api/dashboard/segments[?mart=|source=]` — the segments.
///
/// # Errors
///
/// A fixed 422/503 for a database failure.
pub async fn list_segments(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> ApiResult<ApiJson<Value>> {
    let ch = &state.clickhouse;
    let all = metrics::list_segments(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    let narrow = match (blank(q.mart.as_ref()), blank(q.source.as_ref())) {
        (Some(m), None) => Some((
            SourceKind::Mart,
            m.strip_prefix("serving.").unwrap_or(m).to_owned(),
        )),
        (None, Some(s)) => Some((SourceKind::SqlSource, s.to_owned())),
        _ => None,
    };
    let shown: Vec<SegmentDef> = all
        .into_iter()
        .filter(|s| {
            narrow
                .as_ref()
                .is_none_or(|(k, id)| s.source_kind == *k && s.source_id == *id)
        })
        .collect();
    Ok(ApiJson(json!({ "segments": shown })))
}

fn segment_definition(
    body: SegmentBody,
    source: &Resolved,
    name: String,
    id: String,
) -> SegmentDef {
    SegmentDef {
        id,
        name,
        label: body.label.trim().to_owned(),
        source_kind: source.kind,
        source_id: source.id.clone(),
        filters: body.filters,
        created_by: String::new(),
        updated_at: None,
    }
}

async fn check_segment(
    state: &AppState,
    def: &SegmentDef,
    source: &Resolved,
) -> Result<(), ApiError> {
    let time = crate::routes::settings::time_context(state).await?;
    metrics::check_segment(def, &source.cols, &time).map_err(ApiError::Unprocessable)
}

/// `POST /api/dashboard/segments` — save a new segment.
///
/// # Errors
///
/// 400 for a bad body, name or source; 409 for a name already used on that
/// source; 422 for filters that cannot apply to the source.
pub async fn create_segment(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: SegmentBody = parse(&body)?;
    let ch = &state.clickhouse;
    let source = resolve(ch, parsed.mart.as_deref(), parsed.source.as_deref()).await?;
    let name = parsed.name.trim().to_owned();
    metrics::validate_name(&name, &source.cols).map_err(ApiError::BadRequest)?;
    let existing = metrics::list_segments(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    if existing
        .iter()
        .any(|s| s.name == name && s.source_kind == source.kind && s.source_id == source.id)
    {
        return Err(ApiError::Conflict(format!(
            "a segment called '{name}' already exists on this source."
        ))
        .into());
    }
    if existing.len() >= metrics::MAX_PER_KIND {
        return Err(ApiError::BadRequest(format!(
            "a deployment has at most {} segments.",
            metrics::MAX_PER_KIND
        ))
        .into());
    }
    let mut def = segment_definition(parsed, &source, name, metrics::new_segment_id());
    check_segment(&state, &def, &source).await?;
    def.created_by = author_name(principal.as_ref());
    metrics::save_segment(ch, &def)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true, "segment": def })))
}

/// `PUT /api/dashboard/segments` — change a segment's label and filters.
///
/// # Errors
///
/// 400 for a bad body, 404 for an unknown id, 422 for filters that cannot
/// apply.
pub async fn update_segment(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: SegmentBody = parse(&body)?;
    let ch = &state.clickhouse;
    let id = blank(parsed.id.as_ref())
        .ok_or_else(|| ApiError::BadRequest("id is required.".to_owned()))?
        .to_owned();
    let current = metrics::get_segment(ch, &id)
        .await
        .map_err(|failure| classify_ch(&failure))?
        .ok_or_else(|| ApiError::NotFound(format!("segment '{id}' not found.")))?;
    let source = resolve_current(ch, current.source_kind, &current.source_id).await?;
    let mut def = segment_definition(parsed, &source, current.name.clone(), current.id.clone());
    check_segment(&state, &def, &source).await?;
    def.created_by = author_name(principal.as_ref());
    metrics::save_segment(ch, &def)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true, "segment": def })))
}

/// `DELETE /api/dashboard/segments?id=` — remove a segment nothing uses.
///
/// # Errors
///
/// 400 without an id, 404 for an unknown one, 409 (naming what uses it) while
/// a chart or a metric does.
pub async fn delete_segment(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Query(q): Query<IdQuery>,
) -> ApiResult<ApiJson<Value>> {
    let ch = &state.clickhouse;
    let id =
        blank(q.id.as_ref()).ok_or_else(|| ApiError::BadRequest("id is required.".to_owned()))?;
    let current = metrics::get_segment(ch, id)
        .await
        .map_err(|failure| classify_ch(&failure))?
        .ok_or_else(|| ApiError::NotFound(format!("segment '{id}' not found.")))?;
    let used = segment_usage(
        ch,
        &current,
        &crate::routes::calc_fields::viewer_or_anonymous(principal.as_ref()),
    )
    .await?;
    if !used.is_empty() {
        return Err(ApiError::Conflict(format!(
            "the segment is still used by {}.",
            used.sentence()
        ))
        .into());
    }
    metrics::delete_segment(ch, &current.id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::{Value, json};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{Usage, usage_of};
    use crate::routes::dashboard_organise::HIDDEN_USE;

    fn chart(id: &str, board: &str) -> Value {
        let spec = json!({ "id": id, "title": id, "kind": "bar", "mart": "m", "sql": "SELECT 1", "x": "d", "y": "v" });
        let def =
            json!({ "title": id, "mart": "m", "kind": "bar", "dimension": "d", "measures": ["v"] });
        json!({
            "id": id, "board": board, "created_by": "ui", "created_at": "2026-01-01 00:00:00",
            "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
        })
    }

    async fn answer(server: &MockServer, needles: &[&str], data: Vec<Value>) {
        let mut mock = Mock::given(method("POST"));
        for needle in needles {
            mock = mock.and(body_string_contains(*needle));
        }
        mock.respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "meta": [], "data": data, "rows": data.len() })),
        )
        .mount(server)
        .await;
    }

    /// BI-18 part C, part 2: a chart on somebody's personal board uses a metric
    /// as much as any other, so a delete is blocked, but the caller is not told
    /// which board or chart it is: only that something they cannot see uses it.
    #[tokio::test]
    async fn a_chart_the_caller_cannot_see_blocks_a_delete_without_being_named() {
        let server = MockServer::start().await;
        let mine = "owner_id != '' AND owner_id != 'u_me'";
        answer(
            &server,
            &["FROM console.bi_chart FINAL", mine],
            vec![chart("c_public", "b_public")],
        )
        .await;
        answer(
            &server,
            &["FROM console.bi_chart FINAL"],
            vec![chart("c_public", "b_public"), chart("c_secret", "b_draft")],
        )
        .await;
        answer(&server, &["FROM console.bi_board FINAL"], vec![]).await;
        let ch =
            lakehouse_clickhouse::ChClient::new(server.uri(), "default".to_owned(), String::new());
        let pick = |charts: &[lakehouse_bi::store::StoredChartSpec]| -> Vec<String> {
            charts.iter().map(|c| c.spec.id.clone()).collect()
        };

        let me = lakehouse_bi::access::Viewer::user("u_me", false);
        let used = usage_of(&ch, pick, Vec::new(), &me).await.unwrap();
        assert!(!used.is_empty());
        assert_eq!(
            used.chart_rows.len(),
            1,
            "only the chart they may see is listed"
        );
        assert!(
            used.charts.iter().any(|c| c.contains("c_public")),
            "{:?}",
            used.charts
        );
        assert!(
            used.charts.contains(&HIDDEN_USE.to_owned()),
            "{:?}",
            used.charts
        );
        assert!(!used.sentence().contains("c_secret"), "{}", used.sentence());

        // An administrator sees every space: both charts are named, no marker.
        let admin = lakehouse_bi::access::Viewer::user("u_admin", true);
        let used = usage_of(&ch, pick, Vec::new(), &admin).await.unwrap();
        assert_eq!(used.chart_rows.len(), 2);
        assert!(!used.charts.contains(&HIDDEN_USE.to_owned()));
    }

    /// BI-2 (T3): a metric watched by an alert rule cannot be deleted, and the
    /// refusal names the rule.
    #[test]
    fn a_metric_watched_by_a_rule_is_in_use_and_the_refusal_names_the_rule() {
        let used = Usage {
            alerts: vec![("al_1".to_owned(), "Visitors too low".to_owned())],
            charts: Vec::new(),
            chart_rows: Vec::new(),
            metrics: Vec::new(),
            dashboards: Vec::new(),
        };
        assert!(!used.is_empty());
        assert_eq!(used.sentence(), "the alert rule \"Visitors too low\"");
        let none = Usage {
            alerts: Vec::new(),
            charts: Vec::new(),
            chart_rows: Vec::new(),
            metrics: Vec::new(),
            dashboards: Vec::new(),
        };
        assert!(none.is_empty());
    }
}
