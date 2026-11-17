//! `GET /api/usage/queries`, `GET /api/usage/queries/export`,
//! `GET /api/usage/rates` and `POST /api/usage/rates`: the Usage page's
//! sums of recorded runs, the same sums as a CSV file, and the rates behind
//! the estimate in money (`OPS-8` part B, spec `docs/core/specs/ops-8.md`).
//!
//! # What the numbers are
//!
//! Every figure is a sum of recorded values (`lakehouse_store::usage`): the
//! runs in `query_history`, their scanned bytes and their wall-clock time.
//! The only money is an estimate from rates an administrator entered, and it
//! is `null` when no rate applies. Nothing is a constant of this code.
//!
//! A rate is entered as text, or as a whole number. A JSON number with digits
//! after the point is refused: the JSON parser holds it as a double, and a
//! double cannot be read back as the digits the caller wrote, so the money
//! would be stored rounded without anyone noticing.
//!
//! # Who sees which rows
//!
//! All three reads need `usage:read` and the rate write `settings:write`
//! (`POLICY_TABLE`). Whose runs a read sums is decided here, from the caller:
//!
//! - A caller sees the runs of ONE tenant: the one `X-Tenant` names, or its
//!   first ([`tenant_scope::resolve`], which answers 404 for a tenant the
//!   caller does not belong to). A caller that belongs to no tenant gets an
//!   empty answer, never every tenant's rows.
//! - A caller holding `*:*` ([`catalog::is_unrestricted`]) may send
//!   `tenant=all`: every tenant, and the runs with no tenant, which are
//!   grouped as `no tenant` under `group=tenant`. Any other caller sending
//!   `tenant=all` is refused with 403: a quiet fall back to its own tenant
//!   would let it read the page as "all of the deployment".
//! - The runs with no tenant (everything written before migration `0144`)
//!   are in no tenant's sums, because nobody recorded whose they were.
//!
//! # A person's name
//!
//! `group=person` keys a row by the owner's id. Its `label` is the name
//! recorded on the person's newest run when the caller holds `identity:read`
//! (the permission that opens the user list), and the id otherwise. A run with
//! no owner has only a recorded name, which is its key and its label.
//!
//! # The period
//!
//! `from` and `to` are `YYYY-MM-DD` UTC days, `to` included. Without them the
//! period is the last 30 days ending today. It is at most 366 days, and
//! `from` may not be after `to`. Bad input is a 400 with a fixed sentence that
//! never repeats what the caller sent.
//!
//! # The export
//!
//! The file holds the aggregated rows and their totals, so it has no row
//! limit of its own: at most 366 days, or as many people or engines as there
//! are. Reading it is a way out of the usage data, so it records a
//! `usage.export` audit row before the sums are read (like `audit.export`).
//! The rate write is a changing route: `AUDIT_TABLE` records it as
//! `usage.rate_create`, the refusals at the gate included.

use axum::Extension;
use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::audit::NewAuditEvent;
use lakehouse_store::usage::{
    self, Group, NewUsageRate, TenantScope, UsageQuery, UsageRate, UsageReport, UsageRow,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{Date, Duration, Month, OffsetDateTime};

use super::{QUERY_STRING_UNREADABLE, catalog};
use crate::audit_coverage;
use crate::csv_export;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::tenant_scope;
use crate::upstream_error;

/// How many days a report covers when the caller names no period.
const DEFAULT_DAYS: i64 = 30;

/// The longest period one request may ask for, in days.
const MAX_DAYS: i64 = 366;

/// The most digits before the decimal point of a rate (`NUMERIC(18,6)`).
const MAX_INTEGER_DIGITS: usize = 12;

/// The most decimal places of a rate (`NUMERIC(18,6)`).
const MAX_DECIMALS: usize = 6;

const NO_DATABASE: &str = "Usage needs a database, and none is configured.";
const BAD_DATE: &str = "from and to must be days written YYYY-MM-DD.";
const BAD_RANGE: &str = "from must not be after to.";
const TOO_LONG: &str = "A period is at most 366 days.";
const BAD_GROUP: &str = "group must be day, person, engine or tenant.";
const BAD_TENANT: &str = "tenant must be all, or left out.";
const NOT_ALL: &str = "Only an administrator of the whole deployment may ask for all tenants.";
const RATE_BODY_SHAPE: &str = "body must be {\"kind\": \"scan_tb\"|\"engine_hour\"|\"storage_gb_month\", \
                               \"currency\": three capital letters, \"amount\": text or a whole number, \
                               \"effectiveFrom\": \"YYYY-MM-DD\"}.";
const BAD_AMOUNT: &str = "amount must be a decimal number, not negative, with at most 12 digits before the point and 6 after.";
/// A JSON number that is not a whole number. `serde_json` holds such a number
/// as a double, and a double cannot be read back as the digits the caller
/// wrote, so it is refused instead of stored rounded.
const AMOUNT_AS_TEXT: &str = "amount must be written as text, so its digits are kept. A number with a point or an exponent is refused, because its digits cannot be read back as they were written.";
const BAD_CURRENCY: &str = "currency must be three capital letters, for example USD.";
const BAD_KIND: &str = "kind must be scan_tb, engine_hour or storage_gb_month.";
const MIXED_CURRENCY: &str = "Rates use one currency. The rates already entered use another one, so this rate was not added.";

/// The query string of the sums and of the export. Every field is a raw
/// string so a bad value becomes this module's fixed 400, never serde's
/// wording. An empty value is no value.
#[derive(Debug, Deserialize)]
pub struct UsageParams {
    from: Option<String>,
    to: Option<String>,
    group: Option<String>,
    tenant: Option<String>,
}

type RawParams = Result<Query<UsageParams>, axum::extract::rejection::QueryRejection>;

/// What a request asked for, after the checks.
struct Request {
    from: Date,
    to: Date,
    group: Group,
    /// Whose runs. `None` for a caller that belongs to no tenant: the answer
    /// is empty and the store is not asked.
    scope: Option<TenantScope>,
}

impl Request {
    /// The store's question, or `None` when the answer is empty by rule.
    fn store_query(&self) -> Option<UsageQuery> {
        self.scope.map(|scope| UsageQuery {
            from: self.from,
            to: self.to,
            group: self.group,
            scope,
        })
    }
}

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state
        .pg
        .as_deref()
        .ok_or_else(|| ApiError::Unavailable(NO_DATABASE.to_owned()))
}

fn non_empty(value: Option<&String>) -> Option<&str> {
    value.map(String::as_str).filter(|v| !v.is_empty())
}

/// `YYYY-MM-DD` as a date: four digits, a dash, two, a dash, two, and a day
/// that exists.
fn parse_day(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    let shaped = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if !shaped {
        return None;
    }
    let year: i32 = text[0..4].parse().ok()?;
    let month: u8 = text[5..7].parse().ok()?;
    let day: u8 = text[8..10].parse().ok()?;
    Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()
}

fn day_text(day: Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        day.year(),
        u8::from(day.month()),
        day.day()
    )
}

fn read_params(params: RawParams) -> Result<UsageParams, ApiError> {
    let Query(raw) = params.map_err(|rejection| {
        tracing::warn!(%rejection, "a usage query string could not be read");
        ApiError::BadRequest(QUERY_STRING_UNREADABLE.to_owned())
    })?;
    Ok(raw)
}

/// The period, group and scope of a request, or the 400, 403 or 404 that
/// refuses it. Nothing is read from the store here.
fn read_request(
    principal: &Principal,
    headers: &HeaderMap,
    raw: &UsageParams,
) -> Result<Request, ApiError> {
    let today = OffsetDateTime::now_utc().date();
    let to = match non_empty(raw.to.as_ref()) {
        Some(text) => parse_day(text).ok_or_else(|| ApiError::BadRequest(BAD_DATE.to_owned()))?,
        None => today,
    };
    let from = match non_empty(raw.from.as_ref()) {
        Some(text) => parse_day(text).ok_or_else(|| ApiError::BadRequest(BAD_DATE.to_owned()))?,
        None => to - Duration::days(DEFAULT_DAYS - 1),
    };
    if from > to {
        return Err(ApiError::BadRequest(BAD_RANGE.to_owned()));
    }
    if (to - from).whole_days() + 1 > MAX_DAYS {
        return Err(ApiError::BadRequest(TOO_LONG.to_owned()));
    }
    let group = match non_empty(raw.group.as_ref()) {
        None | Some("day") => Group::Day,
        Some("person") => Group::Person,
        Some("engine") => Group::Engine,
        Some("tenant") => Group::Tenant,
        Some(_) => return Err(ApiError::BadRequest(BAD_GROUP.to_owned())),
    };
    let all = match non_empty(raw.tenant.as_ref()) {
        None => false,
        Some("all") => true,
        Some(_) => return Err(ApiError::BadRequest(BAD_TENANT.to_owned())),
    };
    // The tenant is resolved first, so a forged `X-Tenant` is the same 404 as
    // on every other tenant-scoped route, whatever else the request says.
    let own = tenant_scope::resolve(principal, headers)?;
    let scope = if all {
        if !catalog::is_unrestricted(principal) {
            return Err(ApiError::PermissionDenied(NOT_ALL.to_owned()));
        }
        Some(TenantScope::Everyone)
    } else {
        // Belongs to no tenant: `None`, an empty answer, never every tenant's rows.
        own.map(TenantScope::Tenant)
    };
    Ok(Request {
        from,
        to,
        group,
        scope,
    })
}

/// The answer for a caller with no tenant: no rows and zero totals.
fn empty_report() -> UsageReport {
    UsageReport {
        rows: Vec::new(),
        totals: UsageRow {
            key: String::new(),
            label: String::new(),
            queries: 0,
            completed: 0,
            failed: 0,
            blocked: 0,
            cancelled: 0,
            scanned_bytes: 0,
            engine_ms: 0,
            estimate: None,
        },
    }
}

/// The sums of a request and the currency of the stored rates.
async fn sums(
    state: &AppState,
    principal: &Principal,
    request: &Request,
) -> Result<(UsageReport, Option<String>), ApiError> {
    let pool = pool(state)?;
    let mut report = match request.store_query() {
        Some(query) => usage::report(pool, &query)
            .await
            .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?,
        None => empty_report(),
    };
    if request.group == Group::Person && !principal.has("identity:read") {
        // The name is the user list's: a caller that may not read the list
        // sees the id the row is keyed by.
        for row in &mut report.rows {
            row.label.clone_from(&row.key);
        }
    }
    let currency = usage::list_rates(pool)
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?
        .into_iter()
        .next()
        .map(|rate| rate.currency);
    Ok((report, currency))
}

/// One row of an answer, as the console reads it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RowBody {
    key: String,
    label: String,
    queries: i64,
    completed: i64,
    failed: i64,
    blocked: i64,
    cancelled: i64,
    scanned_bytes: i64,
    /// Wall-clock milliseconds, not CPU time.
    engine_ms: i64,
    /// A decimal with six places, or `null` when no rate applied.
    estimate: Option<String>,
}

impl From<UsageRow> for RowBody {
    fn from(row: UsageRow) -> Self {
        Self {
            key: row.key,
            label: row.label,
            queries: row.queries,
            completed: row.completed,
            failed: row.failed,
            blocked: row.blocked,
            cancelled: row.cancelled,
            scanned_bytes: row.scanned_bytes,
            engine_ms: row.engine_ms,
            estimate: row.estimate,
        }
    }
}

/// The answer of `GET /api/usage/queries`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBody {
    from: String,
    to: String,
    group: &'static str,
    /// `"all"` for every tenant, `"tenant"` for the caller's own.
    scope: &'static str,
    /// The currency of the estimate; `null` while no rate is entered.
    currency: Option<String>,
    rows: Vec<RowBody>,
    totals: RowBody,
}

const fn group_word(group: Group) -> &'static str {
    match group {
        Group::Day => "day",
        Group::Person => "person",
        Group::Engine => "engine",
        Group::Tenant => "tenant",
    }
}

/// `GET /api/usage/queries?from=&to=&group=&tenant=`: the runs of the period
/// summed per day, person, engine or tenant, with the totals of the period.
///
/// # Errors
///
/// 400 with a fixed sentence for an unreadable query string, a bad date, a
/// `from` after `to`, a period over 366 days, or a bad `group` or `tenant`;
/// 403 for `tenant=all` without `*:*`; 404 for an `X-Tenant` the caller does
/// not belong to; 503 without a Postgres pool; 500 (a fixed sentence and a
/// reference) when the database fails.
pub async fn queries(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    params: RawParams,
) -> ApiResult<ApiJson<UsageBody>> {
    let request = read_request(&principal, &headers, &read_params(params)?)?;
    let (report, currency) = sums(&state, &principal, &request).await?;
    Ok(ApiJson(UsageBody {
        from: day_text(request.from),
        to: day_text(request.to),
        group: group_word(request.group),
        scope: scope_word(request.scope),
        currency,
        rows: report.rows.into_iter().map(RowBody::from).collect(),
        totals: report.totals.into(),
    }))
}

/// `"all"` for every tenant, `"tenant"` for the caller's own, also when the
/// caller has none and the answer is empty.
const fn scope_word(scope: Option<TenantScope>) -> &'static str {
    match scope {
        Some(TenantScope::Everyone) => "all",
        Some(TenantScope::Tenant(_)) | None => "tenant",
    }
}

/// Whose runs an export took out, for the audit row: the tenant's id, `all`,
/// or `none` for a caller that belongs to no tenant.
///
/// PR review fix (SHOULD-FIX): the row said `tenant` for every caller, which
/// does not tell an auditor which tenant's data left.
fn audited_tenant(scope: Option<TenantScope>) -> String {
    match scope {
        Some(TenantScope::Everyone) => "all".to_owned(),
        Some(TenantScope::Tenant(id)) => id.to_string(),
        None => "none".to_owned(),
    }
}

fn download_header(text: &str) -> Result<HeaderValue, ApiError> {
    HeaderValue::from_str(text)
        .map_err(|_| ApiError::Internal("a download header could not be built".to_owned()))
}

/// A number of the file, as the digits the store holds.
fn number(value: i64) -> String {
    value.to_string()
}

/// One line of the file. The cells that are text a person typed (a name, an
/// engine, a tenant) go through the one CSV rule, [`csv_export::neutralise`];
/// the others are digits.
fn export_line(row: &UsageRow, group: Group, currency: &str, total: bool) -> String {
    let text = |value: &str| csv_export::neutralise(value);
    let mut cells = vec![if total {
        "Total".to_owned()
    } else {
        text(&row.label)
    }];
    if matches!(group, Group::Person | Group::Tenant) {
        cells.push(text(&row.key));
    }
    cells.extend([
        number(row.queries),
        number(row.completed),
        number(row.failed),
        number(row.blocked),
        number(row.cancelled),
        number(row.scanned_bytes),
        number(row.engine_ms),
        row.estimate.clone().unwrap_or_default(),
        if row.estimate.is_some() {
            text(currency)
        } else {
            String::new()
        },
    ]);
    csv_export::line(&cells)
}

/// The file of one report: a header, one line per row, and a `Total` line.
fn export_file(report: &UsageReport, group: Group, currency: &str) -> String {
    let mut labels = vec![group_word(group).to_owned()];
    if matches!(group, Group::Person | Group::Tenant) {
        labels.push("id".to_owned());
    }
    labels.extend(
        [
            "queries",
            "completed",
            "failed",
            "blocked",
            "cancelled",
            "scanned_bytes",
            "engine_ms",
            "estimate",
            "currency",
        ]
        .map(str::to_owned),
    );
    let mut file = csv_export::header(&labels);
    for row in &report.rows {
        file.push_str(&export_line(row, group, currency, false));
    }
    file.push_str(&export_line(&report.totals, group, currency, true));
    file
}

/// `GET /api/usage/queries/export`: the sums of `GET /api/usage/queries` for
/// the same parameters, as a CSV file `usage-<from>-<to>.csv`. The columns
/// are the group's name, for a person or a tenant its id, then `queries`,
/// `completed`, `failed`, `blocked`, `cancelled`, `scanned_bytes`,
/// `engine_ms`, `estimate` and `currency`, and the last line is the totals.
///
/// # Why this read records itself
///
/// `AUDIT_TABLE` lists the changing methods, and this is a `GET`. It is still
/// a copy of the usage data leaving the console, so the trail says who took
/// it and for which tenant (its id, or `all`), period and group: one
/// `usage.export` row, written after the parameters are accepted and before
/// the sums are read.
///
/// # Errors
///
/// The errors of [`queries`], and then no row is written for a refused
/// request.
pub async fn export(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    params: RawParams,
) -> ApiResult<Response> {
    let request = read_request(&principal, &headers, &read_params(params)?)?;
    let (principal_id, principal_kind, actor_label) = audit_coverage::actor(Some(&principal));
    audit_coverage::record_best_effort(
        &state,
        NewAuditEvent {
            principal_id,
            principal_kind,
            actor_label: Some(actor_label),
            action: "usage.export".to_owned(),
            resource_kind: Some("usage".to_owned()),
            args: Some(json!({
                "from": day_text(request.from),
                "to": day_text(request.to),
                "group": group_word(request.group),
                "tenant": audited_tenant(request.scope),
            })),
            outcome: "executed".to_owned(),
            ..NewAuditEvent::default()
        },
    )
    .await;

    let (report, currency) = sums(&state, &principal, &request).await?;
    let file = export_file(
        &report,
        request.group,
        currency.as_deref().unwrap_or_default(),
    );
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    response_headers.insert(
        header::CONTENT_DISPOSITION,
        download_header(&format!(
            "attachment; filename=\"usage-{}-{}.csv\"",
            day_text(request.from),
            day_text(request.to)
        ))?,
    );
    response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((StatusCode::OK, response_headers, Body::from(file)).into_response())
}

/// One stored rate, as the console reads it. The table also records who
/// entered it (`created_by`, a principal id), but a tenant administrator with
/// `usage:read` reads this answer too, so the id is not sent (PR review fix,
/// SHOULD-FIX).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RateBody {
    id: String,
    kind: String,
    currency: String,
    /// A decimal with six places.
    amount: String,
    effective_from: String,
    created_at: String,
}

impl From<UsageRate> for RateBody {
    fn from(rate: UsageRate) -> Self {
        Self {
            id: rate.id.to_string(),
            kind: rate.kind,
            currency: rate.currency,
            amount: rate.amount,
            effective_from: day_text(rate.effective_from),
            created_at: rate
                .created_at
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default(),
        }
    }
}

/// The answer of `GET /api/usage/rates`.
#[derive(Debug, Serialize)]
pub struct RatesBody {
    /// The currency every rate uses; `null` while none is entered.
    currency: Option<String>,
    /// Newest `effectiveFrom` first, then the newest entered.
    rates: Vec<RateBody>,
}

/// `GET /api/usage/rates`: every rate entered, newest first.
///
/// # Errors
///
/// 503 without a Postgres pool; 500 (a fixed sentence and a reference) when
/// the database fails.
pub async fn list_rates(State(state): State<AppState>) -> ApiResult<ApiJson<RatesBody>> {
    let rates = usage::list_rates(pool(&state)?)
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    Ok(ApiJson(RatesBody {
        currency: rates.first().map(|rate| rate.currency.clone()),
        rates: rates.into_iter().map(RateBody::from).collect(),
    }))
}

/// The body of `POST /api/usage/rates`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NewRateBody {
    kind: String,
    currency: String,
    /// Text, or a whole number. A JSON number with digits after the point is
    /// refused: the JSON parser holds it as a double, and a double cannot be
    /// read back as the digits the caller wrote.
    amount: Value,
    effective_from: String,
}

/// The digits of an amount, checked: digits, then optionally a point and one
/// to six more digits, with at most twelve before the point. A sign, an
/// exponent, a space or an empty text is refused.
///
/// A JSON number is read only when it is a whole number, because `serde_json`
/// holds a number with a point or an exponent as a double and a double cannot
/// be read back as the digits the caller wrote (`serde_json` is built here
/// without `arbitrary_precision`). Text keeps every digit, so the console
/// sends the amount as text.
///
/// # Errors
///
/// 400 with [`AMOUNT_AS_TEXT`] when the amount is a JSON number that is not a
/// whole number, and with [`BAD_AMOUNT`] when it is not a decimal of the
/// shape above.
fn amount_text(amount: &Value) -> Result<String, ApiError> {
    let text = match amount {
        Value::String(s) => s.clone(),
        // A whole number is exact: `serde_json` keeps an integer literal as an
        // integer, not as a double.
        Value::Number(n) if n.is_i64() || n.is_u64() => n.to_string(),
        Value::Number(_) => return Err(ApiError::BadRequest(AMOUNT_AS_TEXT.to_owned())),
        _ => return Err(ApiError::BadRequest(BAD_AMOUNT.to_owned())),
    };
    let (whole, decimals) = match text.split_once('.') {
        Some((whole, decimals)) => (whole, Some(decimals)),
        None => (text.as_str(), None),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let ok = digits(whole)
        && whole.len() <= MAX_INTEGER_DIGITS
        && decimals.is_none_or(|d| digits(d) && d.len() <= MAX_DECIMALS);
    if ok {
        Ok(text)
    } else {
        Err(ApiError::BadRequest(BAD_AMOUNT.to_owned()))
    }
}

/// `POST /api/usage/rates`: add a rate, `{kind, currency, amount,
/// effectiveFrom}`. A rate is never changed or removed: a new price is a new
/// row, and the one with the latest `effectiveFrom` on or before a day prices
/// that day. The answer is the rate as stored, with status 201.
///
/// # Errors
///
/// 400 with a fixed sentence when the body is not that shape, the kind is not
/// one of the three, the currency is not three capital letters, the amount is
/// a JSON number that is not a whole number or is not a non-negative decimal,
/// the date is not a day, or the currency differs from the rates already
/// entered; 503 without a Postgres pool; 500 (a fixed sentence and a
/// reference) when the database fails.
pub async fn create_rate(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<RateBody>)> {
    // Parsed by hand so a malformed body gets this module's sentence, not serde's.
    let parsed: NewRateBody = serde_json::from_slice(&body)
        .map_err(|_| ApiError::BadRequest(RATE_BODY_SHAPE.to_owned()))?;
    if !["scan_tb", "engine_hour", "storage_gb_month"].contains(&parsed.kind.as_str()) {
        return Err(ApiError::BadRequest(BAD_KIND.to_owned()).into());
    }
    if parsed.currency.len() != 3 || !parsed.currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(ApiError::BadRequest(BAD_CURRENCY.to_owned()).into());
    }
    let amount = amount_text(&parsed.amount)?;
    let effective_from = parse_day(&parsed.effective_from)
        .ok_or_else(|| ApiError::BadRequest(BAD_DATE.to_owned()))?;
    let stored = usage::add_rate(
        pool(&state)?,
        &NewUsageRate {
            kind: &parsed.kind,
            currency: &parsed.currency,
            amount: &amount,
            effective_from,
            created_by: &principal.id.uuid().to_string(),
        },
    )
    .await
    .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?
    .ok_or_else(|| ApiError::BadRequest(MIXED_CURRENCY.to_owned()))?;
    Ok((StatusCode::CREATED, ApiJson(stored.into())))
}
