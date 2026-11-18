//! Repository layer for `audit_event` — the append-only sink every copilot
//! gate decision and executed tool call writes to (plan section 3.3 of
//! `docs/superpowers/plans/2026-09-08-copilot-operations-handover.md`).
//!
//! # Append-only, by convention
//!
//! There is deliberately no `update`/`delete` function in this module. A
//! row records one thing that happened at one point in time (a gate
//! allowed a call, a call executed, an approval was requested, an approver
//! decided); a later development in the same story (the approval being
//! decided, the tool then actually running) is a NEW row, not an edit of
//! this one. This matches the migration's header comment and keeps the
//! table trustworthy as a record of what happened, not what the current
//! state is.
//!
//! # Redaction is the caller's job
//!
//! [`insert`] writes `args` verbatim. It does not — and cannot, without
//! knowing what a given tool's arguments mean — strip secrets or bulky
//! query results out of them. Every call site MUST redact before building
//! a [`NewAuditEvent`]: never pass a `secretRef`'s resolved plaintext,
//! never pass raw SQL result rows, only the shape of what was asked for.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use serde_json::Value;
use sqlx::types::Json;
use sqlx::{FromRow, Postgres, QueryBuilder};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{PgPool, StoreError};

fn iso_millis(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    )
}

/// One row of `audit_event`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    /// `audit_event.id` — a generated `audit-<uuid>` string, not caller
    /// supplied (see [`insert`]).
    pub id: String,
    /// When this event was recorded, ISO 8601.
    pub at: String,
    /// Who/what performed the action, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    /// `"user" | "service" | "copilot" | "schedule"`, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_kind: Option<String>,
    /// Display name of the principal, or, for an event nobody signed in
    /// caused, what caused it (see [`ACTOR_SIGN_IN_FORM`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_label: Option<String>,
    /// The tool name or route verb this event is about.
    pub action: String,
    /// The kind of resource the action targets, if any (e.g.
    /// `"connector"`, `"alert_rule"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_kind: Option<String>,
    /// The specific resource id the action targets, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    /// The (caller-redacted) arguments the action was invoked with.
    pub args: Value,
    /// `"allowed" | "executed" | "refused" | "needs_confirmation" |
    /// "needs_approval" | "approved" | "rejected" | "failed"`.
    pub outcome: String,
    /// Free-text detail (e.g. a refusal reason), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The `agent_run` this event is connected to, if any. `NULL`s out if
    /// that run is later deleted (`ON DELETE SET NULL`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// The `approval_item` this event is connected to, if any. `NULL`s out
    /// if that approval is later deleted (`ON DELETE SET NULL`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    /// The chat session this event happened within, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(FromRow)]
struct AuditRow {
    id: String,
    at: OffsetDateTime,
    principal_id: Option<String>,
    principal_kind: Option<String>,
    actor_label: Option<String>,
    action: String,
    resource_kind: Option<String>,
    resource_id: Option<String>,
    args: Json<Value>,
    outcome: String,
    detail: Option<String>,
    run_id: Option<String>,
    approval_id: Option<String>,
    session_id: Option<String>,
}

impl From<AuditRow> for AuditEvent {
    fn from(row: AuditRow) -> Self {
        Self {
            id: row.id,
            at: iso_millis(row.at),
            principal_id: row.principal_id,
            principal_kind: row.principal_kind,
            actor_label: row.actor_label,
            action: row.action,
            resource_kind: row.resource_kind,
            resource_id: row.resource_id,
            args: row.args.0,
            outcome: row.outcome,
            detail: row.detail,
            run_id: row.run_id,
            approval_id: row.approval_id,
            session_id: row.session_id,
        }
    }
}

/// The column list shared by every audit read/write. No `FROM`/`WHERE`, so
/// it is reusable after both a `SELECT` ([`list`]) and a `RETURNING`
/// ([`insert`]).
const AUDIT_COLUMNS: &str = "id, at, principal_id, principal_kind, actor_label, action, \
     resource_kind, resource_id, args, outcome, detail, run_id, approval_id, session_id";

/// Which rows of the audit trail one reader may see (`SEC-20`).
///
/// The table holds no tenant column: it is append-only
/// (`0142_audit_event_append_only.sql`) and its rows must stay readable as
/// they were written, so the tenant is read from the ACTOR, at read time.
/// A row is in the reach of a tenant when the person who caused it belongs
/// to that tenant, which `app_user_tenant` records.
///
/// Three reaches, and no fourth:
///
/// - [`All`](Self::All): every row. An unrestricted caller (`*:*`) sees
///   everything, and so does a deployment that holds a single tenant —
///   there is no other tenant's row to keep out of it.
/// - [`Tenant`](Self::Tenant): the rows of the actors of one tenant, plus
///   the deployment's own rows (the ones nobody signed in caused: a
///   `Dagster` run, a service identity, a schedule) when that tenant owns
///   the shared catalog. The deployment's rows belong to the deployment, so
///   the tenant that owns the shared catalog reads them and no other does.
/// - [`NoTenant`](Self::NoTenant): no row at all. A caller that belongs to
///   no tenant of a deployment that holds several owns nobody's row, and it
///   does not own the shared catalog either, so neither a tenant's rows nor
///   the deployment's own rows are in its reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditScope {
    /// Every row of the trail.
    All,
    /// The rows of the actors of one tenant, and — when that tenant owns
    /// the shared catalog — the deployment's own rows.
    Tenant {
        /// The tenant the caller's own rows come from, as the text the
        /// store compares with `app_user_tenant.tenant_id`.
        tenant_id: String,
        /// Whether that tenant owns the shared catalog, and so reads the
        /// deployment's own rows.
        owns_catalog: bool,
    },
    /// No row at all.
    NoTenant,
}

impl AuditScope {
    /// Whether the deployment's own rows — the ones nobody signed in caused
    /// — are in this reach.
    ///
    /// The rule is the shared catalog's own: the tenant that owns the
    /// shared catalog reads them, and so does an unrestricted caller. A
    /// tenant that does not own it reads no row of the deployment.
    #[must_use]
    pub fn sees_deployment_rows(&self) -> bool {
        match self {
            Self::All => true,
            Self::Tenant { owns_catalog, .. } => *owns_catalog,
            Self::NoTenant => false,
        }
    }
}

/// The clause that narrows a read of `audit_event` to `scope`.
///
/// The ONE implementation of the rule [`AuditScope`] states: every reader of
/// the trail goes through it, so no route and no store function can answer
/// with a different reach.
///
/// The clause names the table, so the statement must read `audit_event`
/// itself: an alias would not resolve `audit_event.principal_id`.
///
/// Every value is bound. The tenant is compared as text on both sides
/// (`app_user_tenant.tenant_id` is a `UUID`, the scope carries text), so a
/// scope that names no tenant matches nothing instead of failing the query.
/// The actor is matched on `app_user.id` rendered as text, because
/// `audit_event.principal_id` holds the person's id as text — the audit
/// writers store `principal.id.uuid().to_string()`.
///
/// The deployment's own rows are the rows whose actor is not a PERSON at
/// all: no principal, a service identity, a schedule — nobody signed in
/// caused them, so they belong to the deployment rather than to an
/// organisation. A person who belongs to no tenant is not one of them: the
/// row is nobody's, and it is visible to nobody but an unrestricted reader
/// (fail closed).
pub(crate) fn push_scope_clause(qb: &mut QueryBuilder<Postgres>, scope: &AuditScope) {
    match scope {
        AuditScope::All => {}
        AuditScope::NoTenant => {
            // The honest clause, not a shortcut around the query: a caller
            // that belongs to no tenant of a deployment that holds several
            // owns nobody's row, and it does not own the shared catalog
            // either, so the deployment's own rows are not in its reach
            // either. The clause matches no row of the table.
            qb.push(" AND false");
        }
        AuditScope::Tenant {
            tenant_id,
            owns_catalog,
        } => {
            qb.push(
                " AND (EXISTS (SELECT 1 FROM app_user u \
                 JOIN app_user_tenant ut ON ut.user_id = u.id \
                 WHERE u.id::text = audit_event.principal_id \
                 AND ut.tenant_id::text = ",
            )
            .push_bind(tenant_id.clone())
            .push(") OR (")
            .push_bind(*owns_catalog)
            .push(
                "::boolean AND NOT EXISTS (SELECT 1 FROM app_user u \
                 WHERE u.id::text = audit_event.principal_id)))",
            );
        }
    }
}

/// What caused an event nobody signed in caused.
///
/// `audit_event.principal_kind` may only be `user`, `service`, `copilot` or
/// `schedule` (`0024_audit_event.sql`), and none of the four is true for
/// something that happened before anybody signed in — a sign-in that was
/// refused, a public link that locked, a single sign-on link that was
/// refused. Such an event keeps `principal_id` and `principal_kind` empty
/// (that is the truth) and names its cause in `actor_label`, using one of
/// the constants below. The governance audit list reads the label as the
/// actor and reports the kind as `service`, never as the assistant
/// (`SEC-70`, QA `LH-SEC-2-06`).
pub const ACTOR_SIGN_IN_FORM: &str = "Sign-in form";

/// See [`ACTOR_SIGN_IN_FORM`]: a visitor locked a dashboard's public link.
pub const ACTOR_PUBLIC_LINK: &str = "Public link";

/// See [`ACTOR_SIGN_IN_FORM`]: a single sign-on link was refused.
pub const ACTOR_SINGLE_SIGN_ON: &str = "Single sign-on";

/// Everything [`insert`] needs. Mirrors the migration's columns, minus the
/// generated `id` and defaulted `at`.
///
/// A dedicated input struct (rather than passing `AuditEvent` and ignoring
/// its `id`/`at`) makes "the id and timestamp are generated, not caller
/// input" a compile-time fact instead of a documented-but-unenforced
/// convention — there is no `AuditEvent` value a caller could construct
/// with a bogus id and hand to `insert` by mistake.
#[derive(Debug, Clone, Default)]
pub struct NewAuditEvent {
    /// Who/what performed the action, if known.
    pub principal_id: Option<String>,
    /// `"user" | "service" | "copilot" | "schedule"`, if known.
    pub principal_kind: Option<String>,
    /// Display name of the principal, or, for an event nobody signed in
    /// caused, what caused it (see [`ACTOR_SIGN_IN_FORM`]).
    pub actor_label: Option<String>,
    /// The tool name or route verb this event is about.
    pub action: String,
    /// The kind of resource the action targets, if any.
    pub resource_kind: Option<String>,
    /// The specific resource id the action targets, if any.
    pub resource_id: Option<String>,
    /// The (caller-redacted) arguments the action was invoked with.
    /// Defaults to `{}` when not given.
    pub args: Option<Value>,
    /// `"allowed" | "executed" | "refused" | "needs_confirmation" |
    /// "needs_approval" | "approved" | "rejected" | "failed"`.
    pub outcome: String,
    /// Free-text detail (e.g. a refusal reason), if any.
    pub detail: Option<String>,
    /// The `agent_run` this event is connected to, if any. Must reference
    /// an existing row or [`StoreError::ForeignKeyViolation`] is returned.
    pub run_id: Option<String>,
    /// The `approval_item` this event is connected to, if any. Must
    /// reference an existing row or [`StoreError::ForeignKeyViolation`] is
    /// returned.
    pub approval_id: Option<String>,
    /// The chat session this event happened within, if any.
    pub session_id: Option<String>,
}

/// Append one audit row and return it as written.
///
/// # Redaction
///
/// This function writes `event.args` VERBATIM. It performs no redaction of
/// its own — the caller must have already stripped secrets (never a
/// resolved `secretRef` value) and bulky/sensitive payloads (never raw SQL
/// result rows) before constructing [`NewAuditEvent`]. See the module doc
/// comment.
///
/// # Errors
///
/// Returns [`StoreError::ForeignKeyViolation`] if `run_id` or
/// `approval_id` is set but does not reference an existing row.
/// Returns [`StoreError::Database`] for any other failure, including a
/// `principal_kind`/`outcome` value outside the set the migration's
/// `CHECK` constraints allow.
pub async fn insert(pool: &PgPool, event: NewAuditEvent) -> Result<AuditEvent, StoreError> {
    let id = format!("audit-{}", Uuid::new_v4());
    let args = event
        .args
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let sql = format!(
        "INSERT INTO audit_event (id, principal_id, principal_kind, actor_label, action, \
         resource_kind, resource_id, args, outcome, detail, run_id, approval_id, session_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
         RETURNING {AUDIT_COLUMNS}"
    );
    // safe: only the constant `AUDIT_COLUMNS` is formatted in; values are bound.
    let row: AuditRow = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(&id)
        .bind(&event.principal_id)
        .bind(&event.principal_kind)
        .bind(&event.actor_label)
        .bind(&event.action)
        .bind(&event.resource_kind)
        .bind(&event.resource_id)
        .bind(Json(args))
        .bind(&event.outcome)
        .bind(&event.detail)
        .bind(&event.run_id)
        .bind(&event.approval_id)
        .bind(&event.session_id)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// Filters accepted by [`list`], [`search`] and [`count_matching`]. Every
/// field but `limit` narrows the result when set; the narrowings combine
/// with AND.
#[derive(Debug, Clone)]
pub struct AuditFilter {
    /// Maximum number of rows to return. [`search`] clamps it, see there.
    pub limit: i64,
    /// Restrict to this resource kind, if given.
    pub resource_kind: Option<String>,
    /// Restrict to this resource id, if given. Callers that mean "one
    /// specific resource" should set both `resource_kind` and
    /// `resource_id`.
    pub resource_id: Option<String>,
    /// Restrict to this principal id, if given.
    pub principal_id: Option<String>,
    /// Restrict to events at or after this time, if given (inclusive).
    pub since: Option<OffsetDateTime>,
    /// Restrict to events before this time, if given (exclusive, so two
    /// adjacent ranges never share an event).
    pub until: Option<OffsetDateTime>,
    /// Restrict to this action. A value that ends in `.` is a prefix
    /// (`dashboard.` matches `dashboard.update`); any other value must
    /// match the whole action. Never interpreted as a pattern: `_` and `%`
    /// in it match themselves.
    pub action: Option<String>,
    /// Restrict to this outcome, if given.
    pub outcome: Option<String>,
    /// Continue after the row this cursor names, newest first. [`count_matching`]
    /// ignores it.
    pub cursor: Option<AuditCursor>,
}

impl Default for AuditFilter {
    /// A generous but bounded default `limit`, matching the 50/500-row caps
    /// used elsewhere in this crate's list queries.
    fn default() -> Self {
        Self {
            limit: 200,
            resource_kind: None,
            resource_id: None,
            principal_id: None,
            since: None,
            until: None,
            action: None,
            outcome: None,
            cursor: None,
        }
    }
}

/// The page size [`search`] uses when the filter's `limit` is below 1.
pub const SEARCH_DEFAULT_LIMIT: i64 = 100;

/// The largest page [`search`] returns, whatever `limit` asks for.
pub const SEARCH_MAX_LIMIT: i64 = 500;

/// The spot in the newest-first order that a page ended at: the `at` of the
/// last row at full precision, and its id.
///
/// `at` is a `TIMESTAMPTZ`, which keeps microseconds, while an event shown
/// to a caller carries `at` as an ISO string cut to milliseconds. A cursor
/// built from that string would sit before every other row of the same
/// millisecond and skip them, so the cursor comes from the stored value,
/// never from [`AuditEvent::at`]. The id breaks ties between rows with the
/// same `at`: the order is `(at DESC, id DESC)`, and the next page is the
/// rows strictly after `(at, id)` in it.
///
/// Opaque to callers: they hold it and pass it back as [`encode`](Self::encode)d
/// text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditCursor {
    at_micros: i64,
    id: String,
}

/// Text that is not a cursor [`AuditCursor::encode`] wrote. The message is a
/// fixed sentence, safe to show a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the cursor is not valid")]
pub struct AuditCursorError;

impl AuditCursor {
    /// A cursor at `at_micros` microseconds since the Unix epoch and `id`.
    #[must_use]
    pub fn from_parts(at_micros: i64, id: impl Into<String>) -> Self {
        Self {
            at_micros,
            id: id.into(),
        }
    }

    /// The cursor as url-safe text: base64 of `<microseconds>:<id>`. The id
    /// may itself hold a `:`, so [`decode`](Self::decode) splits at the first one.
    #[must_use]
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(format!("{}:{}", self.at_micros, self.id))
    }

    /// Read a cursor back from [`encode`](Self::encode)'s text.
    ///
    /// # Errors
    ///
    /// Returns [`AuditCursorError`] for anything [`encode`](Self::encode) could
    /// not have produced: bad base64, text that is not UTF-8, no `:`, a
    /// microsecond count that is not a number or is outside years 1 to 9999,
    /// an empty id, an id with a NUL byte.
    pub fn decode(text: &str) -> Result<Self, AuditCursorError> {
        let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| AuditCursorError)?;
        let text = String::from_utf8(bytes).map_err(|_| AuditCursorError)?;
        let (micros, id) = text.split_once(':').ok_or(AuditCursorError)?;
        let at_micros: i64 = micros.parse().map_err(|_| AuditCursorError)?;
        // GOV-1 review R-1: both checks keep a hand-made cursor away from
        // Postgres, which would otherwise refuse the bound value and turn a
        // bad request into a 500. A TEXT value cannot hold a NUL byte.
        if id.is_empty() || id.contains('\0') || !micros_in_audit_range(at_micros) {
            return Err(AuditCursorError);
        }
        Ok(Self::from_parts(at_micros, id))
    }
}

/// Years 1 to 9999. Postgres `timestamptz` reaches from 4713 BC to 294276 AD
/// and the `time` crate from year -9999 to 9999, so `time` alone accepts the
/// years from -9999 to 4714 BC that Postgres refuses. Years 1 to 9999 are
/// inside both, and any real audit row falls inside them.
fn micros_in_audit_range(micros: i64) -> bool {
    micros_to_time(micros).is_some_and(|at| (1..=9999).contains(&at.year()))
}

fn micros_to_time(micros: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000).ok()
}

fn time_to_micros(at: OffsetDateTime) -> Option<i64> {
    i64::try_from(at.unix_timestamp_nanos().div_euclid(1_000)).ok()
}

/// One page of [`search`] results.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditPage {
    /// Up to the requested number of events, newest first.
    pub events: Vec<AuditEvent>,
    /// Where the next page starts, or `None` when this page holds the last
    /// of the matching events.
    pub next_cursor: Option<AuditCursor>,
}

/// Builds `<head> WHERE ...` for a filter. `page_limit` adds the keyset
/// order and `LIMIT`, and with it the cursor condition; without it (a
/// count) the cursor is ignored too.
///
/// The reach of the reader ([`AuditScope`]) is the first narrowing, so every
/// other filter is applied to the rows the caller may see.
///
/// Only constant fragments are formatted in. Every value, including the
/// `LIKE` pattern of an action prefix, goes through `push_bind`.
fn build_filtered(
    head: &str,
    filter: &AuditFilter,
    scope: &AuditScope,
    page_limit: Option<i64>,
) -> Result<QueryBuilder<Postgres>, StoreError> {
    let mut qb = QueryBuilder::new(head);
    qb.push(" WHERE true");
    push_scope_clause(&mut qb, scope);
    if let Some(kind) = &filter.resource_kind {
        qb.push(" AND resource_kind = ").push_bind(kind.clone());
    }
    if let Some(id) = &filter.resource_id {
        qb.push(" AND resource_id = ").push_bind(id.clone());
    }
    if let Some(principal) = &filter.principal_id {
        qb.push(" AND principal_id = ").push_bind(principal.clone());
    }
    if let Some(since) = filter.since {
        qb.push(" AND at >= ").push_bind(since);
    }
    if let Some(until) = filter.until {
        qb.push(" AND at < ").push_bind(until);
    }
    if let Some(action) = &filter.action {
        if action.ends_with('.') {
            // A prefix is a LIKE pattern with the caller's own `\`, `%` and
            // `_` escaped, so it can only ever match what was typed.
            let mut pattern = String::with_capacity(action.len() + 1);
            for ch in action.chars() {
                if matches!(ch, '\\' | '%' | '_') {
                    pattern.push('\\');
                }
                pattern.push(ch);
            }
            pattern.push('%');
            qb.push(" AND action LIKE ")
                .push_bind(pattern)
                .push(" ESCAPE '\\'");
        } else {
            qb.push(" AND action = ").push_bind(action.clone());
        }
    }
    if let Some(outcome) = &filter.outcome {
        qb.push(" AND outcome = ").push_bind(outcome.clone());
    }
    if let Some(limit) = page_limit {
        if let Some(cursor) = &filter.cursor {
            let at = micros_to_time(cursor.at_micros)
                .ok_or_else(|| StoreError::Validation("the cursor is not valid".to_owned()))?;
            qb.push(" AND (at, id) < (")
                .push_bind(at)
                .push(", ")
                .push_bind(cursor.id.clone())
                .push(")");
        }
        qb.push(" ORDER BY at DESC, id DESC LIMIT ")
            .push_bind(limit);
    }
    Ok(qb)
}

fn select_head() -> String {
    format!("SELECT {AUDIT_COLUMNS} FROM audit_event")
}

/// List audit events, newest first, narrowed by `filter` and by the reader's
/// reach `scope`. `filter.limit` is used as given; [`search`] is the bounded,
/// paged form.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list(
    pool: &PgPool,
    filter: AuditFilter,
    scope: &AuditScope,
) -> Result<Vec<AuditEvent>, StoreError> {
    let mut qb = build_filtered(&select_head(), &filter, scope, Some(filter.limit))?;
    let rows: Vec<AuditRow> = qb.build_query_as().fetch_all(pool).await?;
    Ok(rows.into_iter().map(AuditEvent::from).collect())
}

/// One page of events, newest first, narrowed by `filter`, by the reader's
/// reach `scope` and continuing after `filter.cursor`.
///
/// The order is `(at DESC, id DESC)`, so a page boundary that falls between
/// rows of one `at` loses and repeats nothing. `filter.limit` below 1 means
/// [`SEARCH_DEFAULT_LIMIT`]; above [`SEARCH_MAX_LIMIT`] it is cut to that.
/// One row more than the page is read to learn whether another page
/// exists.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails, and
/// [`StoreError::Validation`] if the cursor holds a time no timestamp can.
pub async fn search(
    pool: &PgPool,
    filter: &AuditFilter,
    scope: &AuditScope,
) -> Result<AuditPage, StoreError> {
    let page_size = if filter.limit < 1 {
        SEARCH_DEFAULT_LIMIT
    } else {
        filter.limit.min(SEARCH_MAX_LIMIT)
    };
    let mut qb = build_filtered(&select_head(), filter, scope, Some(page_size + 1))?;
    let mut rows: Vec<AuditRow> = qb.build_query_as().fetch_all(pool).await?;
    let page_len = usize::try_from(page_size).unwrap_or(usize::MAX);
    let has_more = rows.len() > page_len;
    rows.truncate(page_len);
    let next_cursor = match rows.last() {
        Some(last) if has_more => {
            let micros = time_to_micros(last.at).ok_or_else(|| {
                StoreError::Validation("an event time is out of range".to_owned())
            })?;
            Some(AuditCursor::from_parts(micros, last.id.clone()))
        }
        _ => None,
    };
    Ok(AuditPage {
        events: rows.into_iter().map(AuditEvent::from).collect(),
        next_cursor,
    })
}

/// How many events match `filter`, ignoring its `limit` and `cursor`. The
/// export uses it to say how many events match when the file holds fewer.
///
/// The count is of the rows the reader's reach `scope` allows, so it agrees
/// with the file the export writes.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn count_matching(
    pool: &PgPool,
    filter: &AuditFilter,
    scope: &AuditScope,
) -> Result<i64, StoreError> {
    let mut qb = build_filtered("SELECT count(*) FROM audit_event", filter, scope, None)?;
    let count: i64 = qb.build_query_scalar().fetch_one(pool).await?;
    Ok(count)
}

/// One event by id, or `None` when no event has that id or the reader's reach
/// `scope` keeps it out of the answer.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get(
    pool: &PgPool,
    id: &str,
    scope: &AuditScope,
) -> Result<Option<AuditEvent>, StoreError> {
    let mut qb = QueryBuilder::<Postgres>::new(format!(
        "SELECT {AUDIT_COLUMNS} FROM audit_event WHERE id = "
    ));
    qb.push_bind(id.to_owned());
    push_scope_clause(&mut qb, scope);
    let row: Option<AuditRow> = qb.build_query_as().fetch_optional(pool).await?;
    Ok(row.map(AuditEvent::from))
}

/// The database's plan for the statement [`search`] runs for `filter`, one
/// line per element. With `analyze` the statement is executed and the lines
/// carry real row counts and timings.
///
/// This exists so a test can pin which index serves each filter shape. It
/// shares [`search`]'s statement builder, so the plan it returns is the plan
/// of the real query — including the reach clause.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn explain_search(
    pool: &PgPool,
    filter: &AuditFilter,
    analyze: bool,
    scope: &AuditScope,
) -> Result<Vec<String>, StoreError> {
    let explain = if analyze {
        "EXPLAIN (ANALYZE, BUFFERS) "
    } else {
        "EXPLAIN "
    };
    let head = format!("{explain}{}", select_head());
    let mut qb = build_filtered(&head, filter, scope, Some(SEARCH_DEFAULT_LIMIT + 1))?;
    let lines: Vec<String> = qb.build_query_scalar().fetch_all(pool).await?;
    Ok(lines)
}

/// List the audit events about any of several resources of one kind,
/// newest first — one catalog asset is known by more than one id (its
/// catalog id, and the table keys rules are written against).
///
/// The reader's reach `scope` narrows the list the same way it narrows
/// every other read of the trail.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_for_resources(
    pool: &PgPool,
    resource_kind: &str,
    resource_ids: &[String],
    limit: i64,
    scope: &AuditScope,
) -> Result<Vec<AuditEvent>, StoreError> {
    if resource_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut qb = QueryBuilder::<Postgres>::new(format!(
        "SELECT {AUDIT_COLUMNS} FROM audit_event WHERE resource_kind = "
    ));
    qb.push_bind(resource_kind.to_owned())
        .push(" AND resource_id = ANY(")
        .push_bind(resource_ids.to_vec())
        .push(")");
    push_scope_clause(&mut qb, scope);
    qb.push(" ORDER BY at DESC LIMIT ").push_bind(limit);
    let rows: Vec<AuditRow> = qb.build_query_as().fetch_all(pool).await?;
    Ok(rows.into_iter().map(AuditEvent::from).collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn default_filter_has_a_bounded_limit_and_no_narrowing() {
        let filter = AuditFilter::default();
        assert_eq!(filter.limit, 200);
        assert!(filter.resource_kind.is_none());
        assert!(filter.resource_id.is_none());
        assert!(filter.principal_id.is_none());
        assert!(filter.since.is_none());
        assert!(filter.until.is_none());
        assert!(filter.action.is_none());
        assert!(filter.outcome.is_none());
        assert!(filter.cursor.is_none());
    }
}
