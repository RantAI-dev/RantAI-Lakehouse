//! `GET /api/governance/audit/events`, `.../events/{id}` and
//! `.../export.csv`: search the audit trail at any age, read one event, and
//! take a copy of it out as a CSV file (`GOV-1` part B, feature page
//! Decisions 5 and 9).
//!
//! # Why three routes beside the old list
//!
//! `GET /api/governance/audit` (`routes::governance`) answers the newest 200
//! events with the last 50 pipeline runs merged in, and it stays as it is: the
//! assistant's `get_audit_history` tool and `ops/g3a` read it. These routes
//! read `audit_event` only, filter on the server, page with a keyset cursor
//! over `(at, id)` and have no age or row cut except the export's cap. Each
//! event is the same JSON object the old list shows
//! ([`governance::audit_event_row`]).
//!
//! # Permission
//!
//! All three need `audit:read` in `POLICY_TABLE` and nothing else. Every role
//! holds `governance:read`, which must not open the trail (Decision 9). They
//! are reads, so they are not in `AUDIT_TABLE`; the export is the one read
//! that records itself, see [`export`].
//!
//! # Bad input
//!
//! A query the extractor cannot read, a bad date, an outcome outside the
//! eight, a limit outside 1 to 500, a cursor nobody wrote, or a `from` that is
//! not before `to`, is a 400 with a fixed sentence. The sentence never repeats
//! what the caller sent and never carries serde's wording: a response never
//! shows a library's rendering of the caller's request (`AGENTS.md`
//! principle 4). An empty value is no value, like the other filter routes, so
//! a link built from empty inputs filters nothing instead of failing.

use std::io;
use std::sync::Arc;

use axum::Extension;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::audit::{
    self, AuditCursor, AuditEvent, AuditFilter, NewAuditEvent, SEARCH_DEFAULT_LIMIT,
    SEARCH_MAX_LIMIT,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::QUERY_STRING_UNREADABLE;
use super::governance::{actor_of, audit_event_row};
use crate::audit_coverage;
use crate::csv_export;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// The most events one export file holds (feature page Decision 5). More than
/// this match: the file holds the newest [`MAX_EXPORT_ROWS`] and the response
/// says so.
const MAX_EXPORT_ROWS: usize = 100_000;

/// How many events one read of the store brings in while the export streams.
/// [`audit::search`] cuts a page at its own maximum, so this is that maximum.
const EXPORT_PAGE_ROWS: usize = 500;

/// The longest a filter value is when the export records it in its own row.
const MAX_RECORDED_FILTER_CHARS: usize = 200;

/// The eight values `audit_event.outcome` may hold (`0024_audit_event.sql`).
const OUTCOMES: [&str; 8] = [
    "allowed",
    "executed",
    "refused",
    "needs_confirmation",
    "needs_approval",
    "approved",
    "rejected",
    "failed",
];

const BAD_TIME: &str = "from and to must be RFC 3339 times, for example 2026-11-01T00:00:00Z.";
const BAD_OUTCOME: &str = "outcome must be one of allowed, executed, refused, needs_confirmation, \
                           needs_approval, approved, rejected, failed.";
const BAD_LIMIT: &str = "limit must be a whole number from 1 to 500.";
const BAD_CURSOR: &str = "The cursor is not valid.";
const BAD_RANGE: &str = "from must be before to.";
const NO_SUCH_EVENT: &str = "No audit event has that id.";
const NO_DATABASE: &str = "The audit trail is not available: no database is configured.";

/// The query string of the search and of the export. The export reads the same
/// filters and ignores `limit` and `cursor`. Every field is a raw string so a
/// bad value becomes a fixed 400 of this module, never serde's wording.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditQuery {
    actor: Option<String>,
    action: Option<String>,
    resource_kind: Option<String>,
    resource_id: Option<String>,
    outcome: Option<String>,
    from: Option<String>,
    to: Option<String>,
    limit: Option<String>,
    cursor: Option<String>,
}

type RawQuery = Result<Query<AuditQuery>, axum::extract::rejection::QueryRejection>;

/// The answer of the search: one page of events, newest first, and where the
/// next page starts.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEventsPage {
    events: Vec<Value>,
    /// Opaque text to send back as `cursor`; `null` on the last page.
    next_cursor: Option<String>,
}

fn pool(state: &AppState) -> Result<&Arc<PgPool>, ApiError> {
    state
        .pg
        .as_ref()
        .ok_or_else(|| ApiError::Unavailable(NO_DATABASE.to_owned()))
}

/// The query as the extractor read it, or the fixed sentence for one it could
/// not (a key sent twice, for instance). The rejection's own text is serde's
/// and goes to the log, never to the response.
fn read_query(query: RawQuery) -> Result<AuditQuery, ApiError> {
    let Query(raw) = query.map_err(|rejection| {
        tracing::warn!(%rejection, "an audit query string could not be read");
        ApiError::BadRequest(QUERY_STRING_UNREADABLE.to_owned())
    })?;
    Ok(raw)
}

/// A text filter: `None` for no value or an empty one. A NUL can never match
/// a stored value (`Postgres` refuses it in `TEXT`) and would make the query
/// itself fail, so it becomes U+FFFD, which matches nothing either.
fn text_filter(value: Option<&str>) -> Option<String> {
    value
        .filter(|v| !v.is_empty())
        .map(audit_coverage::without_nul)
}

fn time_filter(value: Option<&str>) -> Result<Option<OffsetDateTime>, ApiError> {
    match value {
        None | Some("") => Ok(None),
        Some(raw) => OffsetDateTime::parse(raw, &Rfc3339)
            .map(Some)
            .map_err(|_| ApiError::BadRequest(BAD_TIME.to_owned())),
    }
}

/// The filters of a query, checked. `limit` and `cursor` are the search's own
/// and are read by [`search`], not here.
fn read_filter(raw: &AuditQuery) -> Result<AuditFilter, ApiError> {
    let outcome = text_filter(raw.outcome.as_deref());
    if let Some(outcome) = &outcome
        && !OUTCOMES.contains(&outcome.as_str())
    {
        return Err(ApiError::BadRequest(BAD_OUTCOME.to_owned()));
    }
    let since = time_filter(raw.from.as_deref())?;
    let until = time_filter(raw.to.as_deref())?;
    if let (Some(since), Some(until)) = (since, until)
        && since >= until
    {
        return Err(ApiError::BadRequest(BAD_RANGE.to_owned()));
    }
    Ok(AuditFilter {
        principal_id: text_filter(raw.actor.as_deref()),
        action: text_filter(raw.action.as_deref()),
        resource_kind: text_filter(raw.resource_kind.as_deref()),
        resource_id: text_filter(raw.resource_id.as_deref()),
        outcome,
        since,
        until,
        ..AuditFilter::default()
    })
}

fn read_limit(raw: Option<&str>) -> Result<i64, ApiError> {
    match raw {
        None | Some("") => Ok(SEARCH_DEFAULT_LIMIT),
        Some(raw) => raw
            .parse::<i64>()
            .ok()
            .filter(|limit| (1..=SEARCH_MAX_LIMIT).contains(limit))
            .ok_or_else(|| ApiError::BadRequest(BAD_LIMIT.to_owned())),
    }
}

fn read_cursor(raw: Option<&str>) -> Result<Option<AuditCursor>, ApiError> {
    match raw {
        None | Some("") => Ok(None),
        Some(raw) => AuditCursor::decode(raw)
            .map(Some)
            .map_err(|_| ApiError::BadRequest(BAD_CURSOR.to_owned())),
    }
}

/// `GET /api/governance/audit/events`: one page of events, newest first, of
/// any age, narrowed by `actor` (a principal id), `action` (the whole action,
/// or a prefix when it ends in `.`), `resourceKind`, `resourceId`, `outcome`,
/// `from` (inclusive) and `to` (exclusive). `limit` is 1 to 500 and defaults to
/// 100; `cursor` is the `nextCursor` of the page before.
///
/// # Errors
///
/// 400 with a fixed sentence for a query string that cannot be read, a bad
/// date, an outcome outside the eight, a limit outside 1 to 500, a cursor that
/// is not one this route wrote, and a `from` that is not before `to`. 503 when
/// no database is configured; 500 (the fixed `database error`) when the read
/// fails.
pub async fn search(
    State(state): State<AppState>,
    query: RawQuery,
) -> ApiResult<ApiJson<AuditEventsPage>> {
    let raw = read_query(query)?;
    let mut filter = read_filter(&raw)?;
    filter.limit = read_limit(raw.limit.as_deref())?;
    filter.cursor = read_cursor(raw.cursor.as_deref())?;
    let page = audit::search(pool(&state)?, &filter).await?;
    Ok(ApiJson(AuditEventsPage {
        events: page.events.iter().map(audit_event_row).collect(),
        next_cursor: page.next_cursor.map(|cursor| cursor.encode()),
    }))
}

/// `GET /api/governance/audit/events/{id}`: one event, as `{ "event": ... }`.
///
/// # Errors
///
/// 404 with a fixed sentence when no event has the id; 503 when no database is
/// configured; 500 (the fixed `database error`) when the read fails.
pub async fn one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    // GOV-1 review R-2: Postgres TEXT cannot hold a NUL byte (`%00` in the
    // path), so no event can have such an id; answer before the query.
    if id.contains('\0') {
        return Err(ApiError::NotFound(NO_SUCH_EVENT.to_owned()).into());
    }
    let event = audit::get(pool(&state)?, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(NO_SUCH_EVENT.to_owned()))?;
    Ok(ApiJson(json!({ "event": audit_event_row(&event) })))
}

/// The filters a request used, as the plain text its own `audit.export` row
/// records. Only the filters that were set, and never the cursor or the limit
/// (the export has neither). The values are cut to
/// [`MAX_RECORDED_FILTER_CHARS`]; the NUL replacement is already done by
/// [`text_filter`].
fn recorded_filters(filter: &AuditFilter) -> Value {
    let cut = |text: &str| -> Value {
        json!(
            text.chars()
                .take(MAX_RECORDED_FILTER_CHARS)
                .collect::<String>()
        )
    };
    let stamp = |at: OffsetDateTime| -> Value {
        json!(
            at.to_offset(time::UtcOffset::UTC)
                .format(&Rfc3339)
                .unwrap_or_default()
        )
    };
    let mut args = Map::new();
    if let Some(actor) = &filter.principal_id {
        args.insert("actor".to_owned(), cut(actor));
    }
    if let Some(action) = &filter.action {
        args.insert("action".to_owned(), cut(action));
    }
    if let Some(kind) = &filter.resource_kind {
        args.insert("resourceKind".to_owned(), cut(kind));
    }
    if let Some(id) = &filter.resource_id {
        args.insert("resourceId".to_owned(), cut(id));
    }
    if let Some(outcome) = &filter.outcome {
        args.insert("outcome".to_owned(), cut(outcome));
    }
    if let Some(since) = filter.since {
        args.insert("from".to_owned(), stamp(since));
    }
    if let Some(until) = filter.until {
        args.insert("to".to_owned(), stamp(until));
    }
    Value::Object(args)
}

/// The header row of the file, in the order of [`export_row`].
const EXPORT_COLUMNS: [&str; 9] = [
    "at",
    "actor",
    "principal_kind",
    "action",
    "resource_kind",
    "resource_id",
    "outcome",
    "detail",
    "args",
];

/// One event as the nine cells of the file. Every cell is text from the trail,
/// which can hold a name or a detail a person typed, so every one goes through
/// the one CSV rule, [`csv_export::neutralise`].
fn export_row(event: &AuditEvent) -> Vec<String> {
    let cell = |text: Option<&str>| csv_export::neutralise(text.unwrap_or_default());
    vec![
        cell(Some(event.at.as_str())),
        // The name the audit page shows for the actor, so the file and the
        // page agree: the stored label, then the principal id.
        csv_export::neutralise(&actor_of(
            event.actor_label.as_deref(),
            event.principal_id.as_deref(),
            event.principal_kind.as_deref(),
        )),
        cell(event.principal_kind.as_deref()),
        cell(Some(event.action.as_str())),
        cell(event.resource_kind.as_deref()),
        cell(event.resource_id.as_deref()),
        cell(Some(event.outcome.as_str())),
        cell(event.detail.as_deref()),
        cell(Some(event.args.to_string().as_str())),
    ]
}

/// Where the export's stream is: the filter with the keyset cursor that moves
/// it down the trail, and how many events it may still send.
struct ExportState {
    pool: Arc<PgPool>,
    filter: AuditFilter,
    remaining: usize,
    header_sent: bool,
    done: bool,
}

/// The file's bytes, a page of the store at a time. The whole file is never in
/// memory: at most one page of [`EXPORT_PAGE_ROWS`] events is.
///
/// A store failure after the first byte ends the stream with an error, which
/// closes the connection, so a client sees a broken download and not a file
/// that looks complete. The log gets the class of the error, never its text.
fn export_stream(
    pool: Arc<PgPool>,
    filter: AuditFilter,
) -> impl futures::Stream<Item = Result<Bytes, io::Error>> {
    let start = ExportState {
        pool,
        filter,
        remaining: MAX_EXPORT_ROWS,
        header_sent: false,
        done: false,
    };
    futures::stream::unfold(start, |mut state| async move {
        if !state.header_sent {
            state.header_sent = true;
            let labels: Vec<String> = EXPORT_COLUMNS.iter().map(|c| (*c).to_owned()).collect();
            return Some((Ok(Bytes::from(csv_export::header(&labels))), state));
        }
        if state.done || state.remaining == 0 {
            return None;
        }
        let page_rows = state.remaining.min(EXPORT_PAGE_ROWS);
        state.filter.limit = i64::try_from(page_rows).unwrap_or(SEARCH_MAX_LIMIT);
        match audit::search(&state.pool, &state.filter).await {
            Ok(page) => {
                state.remaining = state.remaining.saturating_sub(page.events.len());
                state.done = page.next_cursor.is_none();
                state.filter.cursor = page.next_cursor;
                let chunk: String = page
                    .events
                    .iter()
                    .map(|event| csv_export::line(&export_row(event)))
                    .collect();
                if chunk.is_empty() {
                    return None;
                }
                Some((Ok(Bytes::from(chunk)), state))
            }
            Err(err) => {
                tracing::error!(
                    error_class = audit_coverage::store_error_class(&err),
                    "the audit export stopped on a store error"
                );
                state.done = true;
                Some((Err(io::Error::other("the audit export stopped")), state))
            }
        }
    })
}

fn download_header(text: &str) -> Result<HeaderValue, ApiError> {
    HeaderValue::from_str(text)
        .map_err(|_| ApiError::Internal("a download header could not be built".to_owned()))
}

/// `GET /api/governance/audit/export.csv`: the events the search filters pick,
/// newest first, as a CSV file of at most [`MAX_EXPORT_ROWS`] rows with the
/// columns `at`, `actor`, `principal_kind`, `action`, `resource_kind`,
/// `resource_id`, `outcome`, `detail` and `args`. It takes the search's
/// filters and ignores `limit` and `cursor`.
///
/// When more events match than the file holds, it holds the newest ones and
/// the response carries `x-audit-export-truncated: true` and
/// `x-audit-export-total: <how many match>`; otherwise neither header is sent.
///
/// # Why this read records itself
///
/// The central audit writer records changes, and `AUDIT_TABLE` lists only
/// `POST`, `PUT`, `PATCH` and `DELETE`. An export changes nothing, yet it is a
/// way out of the trail and a read of everything in it, so the trail must say
/// who took a copy and with which filters (feature page Decision 5). This is
/// the one read that writes its own `audit.export` row, before the body starts.
/// The row is written before the count, so the count and the file agree about
/// the row itself: with no filter it is the newest event in both.
///
/// # Errors
///
/// 400 with a fixed sentence for a query string that cannot be read, a bad
/// date, an outcome outside the eight, and a `from` that is not before `to`
/// (and then no row is written). 503 when no database is configured; 500 (the
/// fixed `database error`) when the count fails. A store failure while the
/// file streams ends the response early instead.
pub async fn export(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    query: RawQuery,
) -> ApiResult<Response> {
    let raw = read_query(query)?;
    let filter = read_filter(&raw)?;
    let pool = Arc::clone(pool(&state)?);

    let (principal_id, principal_kind, actor_label) = audit_coverage::actor(Some(&principal));
    audit_coverage::record_best_effort(
        &state,
        NewAuditEvent {
            principal_id,
            principal_kind,
            actor_label: Some(actor_label),
            action: "audit.export".to_owned(),
            resource_kind: Some("audit".to_owned()),
            args: Some(recorded_filters(&filter)),
            outcome: "executed".to_owned(),
            ..NewAuditEvent::default()
        },
    )
    .await;

    let matching = audit::count_matching(&pool, &filter).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        download_header(&format!(
            "attachment; filename=\"audit-events-{}.csv\"",
            OffsetDateTime::now_utc().date()
        ))?,
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if usize::try_from(matching).is_ok_and(|matching| matching > MAX_EXPORT_ROWS) {
        headers.insert(
            HeaderName::from_static("x-audit-export-truncated"),
            HeaderValue::from_static("true"),
        );
        headers.insert(
            HeaderName::from_static("x-audit-export-total"),
            download_header(&matching.to_string())?,
        );
    }
    let body = Body::from_stream(export_stream(pool, filter));
    Ok((StatusCode::OK, headers, body).into_response())
}
