//! Scheduled reports: the subscription routes and the one set of rules for
//! saving a subscription (`RPT-1` T4, plan decisions D7 and D8; feature
//! decisions 1 to 5 and 9).
//!
//! # One set of rules
//!
//! [`validate_input`] is what every way of saving a subscription goes through:
//! these routes and the assistant's tools (`AI-10`, `routes::ai::tools::reports`).
//! It checks, for the person the report will be made as (the owner): that they
//! can view and download, that the board is theirs to see, that the chosen
//! charts are data charts of that board, that the PDF service exists when a PDF
//! is asked for, that every console recipient is an active user who shares a
//! tenant with the owner when the owner belongs to tenants, that every
//! outside address is on a domain an administrator listed
//! (`reporting_settings.report_allowed_domains`, empty at first, so nothing
//! leaves the company until a domain is added), and that there are at most 50
//! recipients. A refusal is one of this module's sentences; it never echoes a
//! recipient's address or an upstream error.
//!
//! # Who may do what (feature decision 2)
//!
//! Anyone with `dashboard:read` who may download can subscribe. Only the owner
//! of a subscription, or an administrator (`settings:write`), can read its
//! recipients, change, pause, resume, delete or send it now. A person's list of
//! a dashboard's subscriptions holds their own; an administrator's holds all.
//!
//! # Email not set up (feature decision 9)
//!
//! A subscription can be saved without `SMTP_HOST`; the list says
//! `emailSetUp: false` so the console can say "Email is not set up on this
//! deployment", and a run records the same sentence (principle 2: no invented
//! success). A PDF attachment is refused while the deployment has no PDF
//! service.

use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, State};
use lakehouse_auth::Principal;
use lakehouse_bi::grain::Grain;
use lakehouse_bi::store;
use lakehouse_core::ApiError;
use lakehouse_store::report_schedule::{MonthDay, Schedule, zone_is_known};
use lakehouse_store::report_subscriptions::{
    self as subscriptions, Delivery, NewSubscription, Recipient, RecipientRow, Subscription,
};
use serde::Deserialize;
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use super::report_runner::{
    AttachmentSpec, MAX_RECIPIENTS, domain_allowed, normalise_address, run_subscription,
};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::dashboard::check_filters_text;
use crate::routes::dashboard_organise::viewer_of;
use crate::state::AppState;
use crate::upstream_error;

const NOT_YOURS: &str = "Only the person who set this report up, or an administrator, can do this.";
/// What an administrator who did not set a report up is told when they try more
/// than pausing, resuming, deleting or removing recipients (`RPT-1` review fix R5).
const NOT_THE_OWNER: &str = "An administrator who did not set this report up can pause, resume or delete it and remove recipients; only its owner can change anything else or send it now.";
const NO_SUCH_SUBSCRIPTION: &str = "this subscription does not exist";
const NO_SUCH_DASHBOARD: &str = "this dashboard does not exist";
const NO_RECIPIENTS: &str = "Choose at least one recipient.";
const TOO_MANY_RECIPIENTS: &str = "A report goes to at most 50 recipients.";
const NOT_AN_ACTIVE_USER: &str = "Every console recipient must be an active user.";
/// A console recipient outside the owner's tenants (`PR #204` review
/// (SHOULD-FIX) 1; feature decision 8).
const NOT_IN_YOUR_TENANTS: &str = "Every console recipient must be in one of your tenants.";
const BAD_ADDRESS: &str = "Every outside recipient must be an email address.";
const DOMAIN_NOT_ALLOWED: &str =
    "An outside address is on a domain an administrator has not allowed for reports.";
const NO_PDF_SERVICE: &str =
    "The PDF cannot be attached: the PDF service is not set up on this deployment.";
const CHART_NOT_ON_BOARD: &str = "Every chart file must be a data chart of this dashboard.";
const BAD_VALUES: &str = "filters must be a list, params an object, and grain one of minute, hour, day, week, month, quarter, year or own.";
/// Most deliveries one request lists.
const DELIVERIES_LIMIT: i64 = 50;
/// Most people the recipient picker lists.
const PICKER_LIMIT: i64 = 20;

fn store_failed(err: &lakehouse_store::StoreError) -> ApiError {
    // `StoreError` already reads "database error"; the source goes to the log.
    upstream_error::internal_error(&upstream_error::STORE, err)
}

// ── Bodies ───────────────────────────────────────────────────────────────

/// The day a monthly schedule runs on: 1 to 28, or the word `last`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum MonthDayInput {
    Day(u8),
    Word(String),
}

/// The schedule as the console and the assistant send it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ScheduleInput {
    pub kind: String,
    #[serde(default)]
    pub minute: Option<u8>,
    #[serde(default)]
    pub hour: Option<u8>,
    #[serde(default)]
    pub weekday: Option<u8>,
    #[serde(default)]
    pub month_day: Option<MonthDayInput>,
}

/// One recipient: a console user by id, or an outside address.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecipientInput {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

/// The body of a create or an update.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SubscriptionInput {
    /// The dashboard (create only; an update may repeat it).
    #[serde(default)]
    pub board: Option<String>,
    pub schedule: ScheduleInput,
    /// The zone the schedule is read in; the report time zone when absent.
    #[serde(default)]
    pub time_zone: Option<String>,
    pub attachments: AttachmentSpec,
    #[serde(default)]
    pub filters: Option<Value>,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub grain: Option<String>,
    #[serde(default)]
    pub skip_when_empty: bool,
    pub recipients: Vec<RecipientInput>,
    /// An update may pause or resume at the same time.
    #[serde(default)]
    pub paused: Option<bool>,
}

fn schedule_of(input: &ScheduleInput) -> Result<Schedule, ApiError> {
    let bad = || ApiError::BadRequest("That schedule is not valid.".to_owned());
    let minute = input.minute.unwrap_or(0);
    let hour = || input.hour.ok_or_else(bad);
    let schedule = match input.kind.as_str() {
        "hourly" => Schedule::Hourly { minute },
        "daily" => Schedule::Daily {
            hour: hour()?,
            minute,
        },
        "weekly" => Schedule::Weekly {
            weekday: input.weekday.ok_or_else(bad)?,
            hour: hour()?,
            minute,
        },
        "monthly" => Schedule::Monthly {
            day: match input.month_day.as_ref().ok_or_else(bad)? {
                MonthDayInput::Day(d) => MonthDay::Day(*d),
                MonthDayInput::Word(w) if w == "last" => MonthDay::Last,
                MonthDayInput::Word(_) => return Err(bad()),
            },
            hour: hour()?,
            minute,
        },
        _ => return Err(bad()),
    };
    schedule.validate().map_err(|_| bad())?;
    Ok(schedule)
}

pub(crate) fn schedule_json(s: Schedule) -> Value {
    match s {
        Schedule::Hourly { minute } => json!({ "kind": "hourly", "minute": minute }),
        Schedule::Daily { hour, minute } => {
            json!({ "kind": "daily", "hour": hour, "minute": minute })
        }
        Schedule::Weekly {
            weekday,
            hour,
            minute,
        } => json!({ "kind": "weekly", "weekday": weekday, "hour": hour, "minute": minute }),
        Schedule::Monthly { day, hour, minute } => json!({
            "kind": "monthly",
            "monthDay": match day { MonthDay::Day(d) => json!(d), MonthDay::Last => json!("last") },
            "hour": hour,
            "minute": minute,
        }),
    }
}

// ── The rules ────────────────────────────────────────────────────────────

/// Whether a person may be sent a report made as the owner: an owner who
/// belongs to no tenant constrains nobody, otherwise the two must share one
/// tenant (feature decision 8; `RPT-1` review fix R8).
///
/// One copy, read by the recipient picker, the save-time refusal and the
/// runner's audience (`PR #204` review (SHOULD-FIX) 1; AGENTS.md rule 4: a
/// second copy of a rule is a finding).
pub(crate) fn shares_a_tenant(owner_tenants: &[Uuid], person_tenants: &[Uuid]) -> bool {
    owner_tenants.is_empty()
        || person_tenants
            .iter()
            .any(|tenant| owner_tenants.contains(tenant))
}

/// Check `input` for a report made as `owner` and turn it into what the store
/// writes. `owner` is the person the report is made as, whoever is saving it.
///
/// # Errors
///
/// 400 with one of this module's sentences; 404 for a dashboard the owner cannot
/// see; 403 when the owner may not download; a store failure under a reference.
pub(crate) async fn validate_input(
    state: &AppState,
    owner: &Principal,
    board: &str,
    input: &SubscriptionInput,
) -> Result<NewSubscription, ApiError> {
    let pool = state
        .pg
        .as_deref()
        .ok_or_else(|| ApiError::Unavailable("the database is unavailable".to_owned()))?;
    crate::routes::settings::require_download(state, owner).await?;

    // The board is one the owner can see (a trashed board and another person's
    // personal board read as missing); the built-in board has no row.
    let found = store::get_board(&state.clickhouse, board, &viewer_of(owner))
        .await
        .map_err(|err| crate::routes::dashboard::classify_dashboard_ch_error(&err))?;
    if found.is_none() && board != store::DEFAULT_BOARD_ID {
        return Err(ApiError::NotFound(NO_SUCH_DASHBOARD.to_owned()));
    }

    let schedule = schedule_of(&input.schedule)?;
    let time_zone = match input
        .time_zone
        .as_deref()
        .map(str::trim)
        .filter(|z| !z.is_empty())
    {
        Some(zone) if zone_is_known(zone) => zone.to_owned(),
        Some(_) => return Err(ApiError::BadRequest("Unknown time zone.".to_owned())),
        None => crate::routes::settings::time_context(state)
            .await?
            .zone()
            .to_owned(),
    };

    input
        .attachments
        .validate()
        .map_err(|why| ApiError::BadRequest(why.to_owned()))?;
    if input.attachments.pdf.is_some() && !crate::routes::dashboard::pdf_supported(&state.config) {
        return Err(ApiError::BadRequest(NO_PDF_SERVICE.to_owned()));
    }
    check_charts(state, owner, board, &input.attachments).await?;

    // The dashboard values are checked as the dashboard route checks them, so a
    // malformed filter is refused here and not a blank report later.
    let values_ok = input.filters.as_ref().is_none_or(Value::is_array)
        && input.params.as_ref().is_none_or(Value::is_object)
        && input
            .grain
            .as_deref()
            .is_none_or(|g| g == "own" || Grain::parse(g).is_some_and(Grain::is_truncation));
    if !values_ok {
        return Err(ApiError::BadRequest(BAD_VALUES.to_owned()));
    }
    if let Some(filters) = input.filters.as_ref().filter(|f| !f.is_null()) {
        check_filters_text(Some(&filters.to_string()))?;
    }

    let recipients = check_recipients(state, pool, owner, &input.recipients).await?;
    let next_run_at = schedule
        .next_run_after(&time_zone, OffsetDateTime::now_utc())
        .map_err(|e| ApiError::BadRequest(e.message().to_owned()))?;
    Ok(NewSubscription {
        board: board.to_owned(),
        owner_id: owner.id.uuid(),
        schedule,
        time_zone,
        attachments: serde_json::to_value(&input.attachments)
            .map_err(|_| ApiError::Internal("The request could not be read.".to_owned()))?,
        filters: input.filters.clone().filter(|f| !f.is_null()),
        params: input.params.clone().filter(|p| !p.is_null()),
        grain: input.grain.clone().filter(|g| !g.is_empty()),
        skip_when_empty: input.skip_when_empty,
        next_run_at,
        recipients,
    })
}

/// Every chart file names a data chart of the board, as the owner sees it.
async fn check_charts(
    state: &AppState,
    owner: &Principal,
    board: &str,
    spec: &AttachmentSpec,
) -> Result<(), ApiError> {
    if spec.charts.is_empty() {
        return Ok(());
    }
    let charts = store::list_stored_charts(&state.clickhouse, &viewer_of(owner))
        .await
        .map_err(|err| crate::routes::dashboard::classify_dashboard_ch_error(&err))?;
    let on_board = |id: &str| {
        charts
            .iter()
            .any(|c| c.board == board && c.spec.id == id && !c.spec.kind.is_static())
    };
    if spec.charts.iter().all(|c| on_board(&c.chart)) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(CHART_NOT_ON_BOARD.to_owned()))
    }
}

/// The recipients as the store writes them, after the rules of feature decision
/// 3: at most 50, console users active, outside addresses on a listed domain.
/// A console recipient must also share a tenant with `owner` when the owner
/// belongs to tenants (feature decision 8; `PR #204` review (SHOULD-FIX) 1).
async fn check_recipients(
    state: &AppState,
    pool: &lakehouse_store::PgPool,
    owner: &Principal,
    wanted: &[RecipientInput],
) -> Result<Vec<Recipient>, ApiError> {
    if wanted.is_empty() {
        return Err(ApiError::BadRequest(NO_RECIPIENTS.to_owned()));
    }
    if wanted.len() > MAX_RECIPIENTS {
        return Err(ApiError::BadRequest(TOO_MANY_RECIPIENTS.to_owned()));
    }
    let allowed = crate::routes::settings::format_settings(state)
        .await?
        .report_allowed_domains;
    let mut out: Vec<Recipient> = Vec::new();
    for item in wanted {
        let recipient = match (&item.user_id, &item.email) {
            (Some(id), None) => {
                let uuid = id
                    .parse::<Uuid>()
                    .map_err(|_| ApiError::BadRequest(NOT_AN_ACTIVE_USER.to_owned()))?;
                // The tenant rule first: a person outside the owner's tenants
                // is refused the same way whether they are active or not, so
                // the refusal does not tell the caller what the state of a
                // person outside their tenants is.
                if !owner.tenant_ids.is_empty() {
                    let tenants = lakehouse_store::identity::user_tenant_ids(pool, uuid)
                        .await
                        .map_err(|e| store_failed(&e))?;
                    if !shares_a_tenant(&owner.tenant_ids, &tenants) {
                        return Err(ApiError::BadRequest(NOT_IN_YOUR_TENANTS.to_owned()));
                    }
                }
                let status = lakehouse_store::identity::user_status(pool, uuid)
                    .await
                    .map_err(|e| store_failed(&e))?;
                if status.as_deref() != Some("active") {
                    return Err(ApiError::BadRequest(NOT_AN_ACTIVE_USER.to_owned()));
                }
                Recipient::User(uuid)
            }
            (None, Some(email)) => {
                let address = normalise_address(email)
                    .ok_or_else(|| ApiError::BadRequest(BAD_ADDRESS.to_owned()))?;
                if !domain_allowed(&address, &allowed) {
                    return Err(ApiError::BadRequest(DOMAIN_NOT_ALLOWED.to_owned()));
                }
                Recipient::Email(address)
            }
            _ => {
                return Err(ApiError::BadRequest(
                    "Each recipient is either a user or an email address.".to_owned(),
                ));
            }
        };
        if !out.contains(&recipient) {
            out.push(recipient);
        }
    }
    Ok(out)
}

// ── JSON out ─────────────────────────────────────────────────────────────

fn recipient_json(r: &RecipientRow) -> Value {
    match (&r.user_id, &r.email) {
        (Some(id), _) => json!({
            "userId": id,
            "name": r.user_name,
            "email": r.user_email,
            "active": r.user_status.as_deref() == Some("active"),
        }),
        (None, email) => json!({ "email": email }),
    }
}

fn delivery_json(d: &Delivery) -> Value {
    json!({
        "id": d.id,
        "startedAt": d.started_at.to_string(),
        "finishedAt": d.finished_at.to_string(),
        "status": d.status,
        "origin": d.origin,
        "recipientCount": d.recipient_count,
        "reason": d.reason,
    })
}

async fn subscription_json(
    pool: &lakehouse_store::PgPool,
    sub: &Subscription,
    last: Option<&Delivery>,
) -> Result<Value, ApiError> {
    let recipients = subscriptions::recipients(pool, sub.id)
        .await
        .map_err(|e| store_failed(&e))?;
    let schedule = sub.schedule().map_or(Value::Null, schedule_json);
    Ok(json!({
        "id": sub.id,
        "board": sub.board,
        "ownerId": sub.owner_id,
        "schedule": schedule,
        "timeZone": sub.time_zone,
        "attachments": sub.attachments,
        "filters": sub.filters,
        "params": sub.params,
        "grain": sub.grain,
        "skipWhenEmpty": sub.skip_when_empty,
        "paused": sub.paused,
        "pausedReason": sub.paused_reason,
        "consecutiveFailures": sub.consecutive_failures,
        "nextRunAt": sub.next_run_at.to_string(),
        "createdAt": sub.created_at.to_string(),
        "recipients": recipients.iter().map(recipient_json).collect::<Vec<_>>(),
        "lastDelivery": last.map(delivery_json),
    }))
}

fn settings_flags(state: &AppState) -> Value {
    json!({
        "emailSetUp": state.config.smtp_host.is_some(),
        "pdfSupported": crate::routes::dashboard::pdf_supported(&state.config),
    })
}

// ── Authorisation ────────────────────────────────────────────────────────

fn is_admin(principal: &Principal) -> bool {
    principal.has("settings:write")
}

fn pool_of(state: &AppState) -> Result<&lakehouse_store::PgPool, ApiError> {
    state
        .pg
        .as_deref()
        .ok_or_else(|| ApiError::Unavailable("the database is unavailable".to_owned()))
}

/// The subscription, if it exists and `principal` is its owner or an
/// administrator.
async fn own_or_admin(
    state: &AppState,
    principal: &Principal,
    id: &str,
) -> Result<Subscription, ApiError> {
    let uuid = id
        .parse::<Uuid>()
        .map_err(|_| ApiError::NotFound(NO_SUCH_SUBSCRIPTION.to_owned()))?;
    let sub = subscriptions::get(pool_of(state)?, uuid)
        .await
        .map_err(|e| store_failed(&e))?
        .ok_or_else(|| ApiError::NotFound(NO_SUCH_SUBSCRIPTION.to_owned()))?;
    if sub.owner_id == principal.id.uuid() || is_admin(principal) {
        Ok(sub)
    } else {
        Err(ApiError::PermissionDenied(NOT_YOURS.to_owned()))
    }
}

/// The person a report is made as: the caller when they own it, else the owner
/// loaded as they are now (an administrator editing someone else's report).
async fn owner_principal(
    state: &AppState,
    caller: &Principal,
    sub: &Subscription,
) -> Result<Principal, ApiError> {
    if sub.owner_id == caller.id.uuid() {
        return Ok(caller.clone());
    }
    match crate::routes::render::principal_of(state, &sub.owner_id.to_string()).await {
        Ok(Some(owner)) => Ok(owner),
        Ok(None) => Err(ApiError::BadRequest(
            "The person who set this report up is disabled or no longer exists.".to_owned(),
        )),
        Err(_) => Err(ApiError::Unavailable(
            "the database is unavailable".to_owned(),
        )),
    }
}

fn parse_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_| ApiError::BadRequest("The request body is not valid.".to_owned()))
}

// ── Routes ───────────────────────────────────────────────────────────────

/// Query of `GET /api/reports/subscriptions`.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    board: Option<String>,
}

/// `GET /api/reports/subscriptions?board=` — the subscriptions of one
/// dashboard that the caller owns (an administrator sees all), each with its
/// last delivery, and whether email and the PDF service are set up.
///
/// # Errors
///
/// 400 without `board`; 404 for a dashboard the caller cannot see; a store
/// failure under a reference.
pub async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<ListQuery>,
) -> ApiResult<ApiJson<Value>> {
    let board = q
        .board
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .ok_or_else(|| ApiError::BadRequest("board is required".to_owned()))?;
    let pool = pool_of(&state)?;
    let found = store::get_board(&state.clickhouse, board, &viewer_of(&principal))
        .await
        .map_err(|err| crate::routes::dashboard::classify_dashboard_ch_error(&err))?;
    if found.is_none() && board != store::DEFAULT_BOARD_ID {
        return Err(ApiError::NotFound(NO_SUCH_DASHBOARD.to_owned()).into());
    }
    let admin = is_admin(&principal);
    let rows: Vec<Subscription> = subscriptions::list_for_board(pool, board)
        .await
        .map_err(|e| store_failed(&e))?
        .into_iter()
        .filter(|s| admin || s.owner_id == principal.id.uuid())
        .collect();
    let ids: Vec<Uuid> = rows.iter().map(|s| s.id).collect();
    let last = subscriptions::last_deliveries(pool, &ids)
        .await
        .map_err(|e| store_failed(&e))?;
    let mut items = Vec::with_capacity(rows.len());
    for sub in &rows {
        let last = last.iter().find(|d| d.subscription_id == sub.id);
        let mut item = subscription_json(pool, sub, last).await?;
        item["canEdit"] = json!(admin || sub.owner_id == principal.id.uuid());
        items.push(item);
    }
    let mut body = settings_flags(&state);
    body["items"] = Value::Array(items);
    Ok(ApiJson(body))
}

/// `POST /api/reports/subscriptions` — subscribe the caller to a dashboard.
///
/// # Errors
///
/// As [`validate_input`]; 400 without `board`.
pub async fn create(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let input: SubscriptionInput = parse_body(&body)?;
    let board = input
        .board
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .ok_or_else(|| ApiError::BadRequest("board is required".to_owned()))?
        .to_owned();
    let new = validate_input(&state, &principal, &board, &input).await?;
    let pool = pool_of(&state)?;
    let made = subscriptions::create(pool, &new)
        .await
        .map_err(|e| store_failed(&e))?;
    Ok(ApiJson(subscription_json(pool, &made, None).await?))
}

/// `GET /api/reports/subscriptions/{id}`.
///
/// # Errors
///
/// 404 for an unknown id; 403 for someone who is neither the owner nor an
/// administrator.
pub async fn get(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let sub = own_or_admin(&state, &principal, &id).await?;
    let pool = pool_of(&state)?;
    let last = subscriptions::last_deliveries(pool, &[sub.id])
        .await
        .map_err(|e| store_failed(&e))?;
    Ok(ApiJson(subscription_json(pool, &sub, last.first()).await?))
}

/// Write an update and apply a pause or resume that came with it. The one copy:
/// the route and the assistant's `update_report_subscription` both call it.
///
/// # Errors
///
/// 404 when the subscription is gone; a store failure under a reference.
pub(crate) async fn save_update(
    pool: &lakehouse_store::PgPool,
    sub: &Subscription,
    new: &NewSubscription,
    paused: Option<bool>,
) -> Result<Subscription, ApiError> {
    let mut saved = subscriptions::update(pool, sub.id, new)
        .await
        .map_err(|e| store_failed(&e))?
        .ok_or_else(|| ApiError::NotFound(NO_SUCH_SUBSCRIPTION.to_owned()))?;
    match paused {
        Some(true) if !saved.paused => {
            saved = subscriptions::set_paused(pool, sub.id, true, Some("Paused by hand."), None)
                .await
                .map_err(|e| store_failed(&e))?
                .unwrap_or(saved);
        }
        Some(false) if saved.paused => {
            saved = subscriptions::set_paused(pool, sub.id, false, None, Some(new.next_run_at))
                .await
                .map_err(|e| store_failed(&e))?
                .unwrap_or(saved);
        }
        _ => {}
    }
    Ok(saved)
}

/// `PUT /api/reports/subscriptions/{id}` — replace the schedule, attachments,
/// values and recipients; `paused` pauses or resumes. The dashboard and the
/// owner never change.
///
/// # Errors
///
/// As [`validate_input`], plus 403 for someone who is neither the owner nor an
/// administrator and 400 when `board` names another dashboard.
pub async fn update(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let sub = own_or_admin(&state, &principal, &id).await?;
    let pool = pool_of(&state)?;
    // RPT-1 review fix (SHOULD-FIX) R4: pausing or resuming goes straight to the
    // store. It must not re-check recipients, domains and rights, or a report
    // the person no longer wants could not be stopped once any of them changed.
    if let Some(paused) = pause_only(&body) {
        let saved = apply_pause(pool, &sub, paused).await?;
        return Ok(ApiJson(subscription_json(pool, &saved, None).await?));
    }
    let input: SubscriptionInput = parse_body(&body)?;
    if input.board.as_deref().is_some_and(|b| b != sub.board) {
        return Err(ApiError::BadRequest(
            "A report cannot move to another dashboard; make a new one.".to_owned(),
        )
        .into());
    }
    let owner = owner_principal(&state, &principal, &sub).await?;
    let new = validate_input(&state, &owner, &sub.board, &input).await?;
    // RPT-1 review fix (SHOULD-FIX) R5: an administrator who is not the owner
    // may pause, resume, delete and remove recipients, nothing else.
    if sub.owner_id != principal.id.uuid() {
        let stored = subscriptions::recipients(pool, sub.id)
            .await
            .map_err(|e| store_failed(&e))?;
        if !only_removes_recipients(&sub, &new, &stored) {
            return Err(ApiError::PermissionDenied(NOT_THE_OWNER.to_owned()).into());
        }
    }
    let saved = save_update(pool, &sub, &new, input.paused).await?;
    Ok(ApiJson(subscription_json(pool, &saved, None).await?))
}

/// Some(paused) when the body is exactly `{"paused": <bool>}`.
pub(crate) fn pause_only(body: &Bytes) -> Option<bool> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let map = v.as_object()?;
    (map.len() == 1).then(|| map.get("paused").and_then(Value::as_bool))?
}

/// Pause a subscription, or resume it with its next time worked out from now.
/// The one copy for the route and the assistant's tool (`RPT-1` review fix R4).
pub(crate) async fn apply_pause(
    pool: &lakehouse_store::PgPool,
    sub: &Subscription,
    paused: bool,
) -> Result<Subscription, ApiError> {
    let resume_at = sub.schedule().ok().and_then(|s| {
        s.next_run_after(&sub.time_zone, OffsetDateTime::now_utc())
            .ok()
    });
    let reason = paused.then_some("Paused by hand.");
    subscriptions::set_paused(pool, sub.id, paused, reason, resume_at)
        .await
        .map_err(|e| store_failed(&e))?
        .ok_or_else(|| ApiError::NotFound(NO_SUCH_SUBSCRIPTION.to_owned()))
}

/// Whether `new` differs from the stored subscription only by dropping
/// recipients (`RPT-1` review fix R5).
fn only_removes_recipients(
    sub: &Subscription,
    new: &NewSubscription,
    stored: &[RecipientRow],
) -> bool {
    let kept: Vec<Recipient> = stored
        .iter()
        .filter_map(|r| match (&r.user_id, &r.email) {
            (Some(id), _) => Some(Recipient::User(*id)),
            (None, Some(email)) => Some(Recipient::Email(email.clone())),
            (None, None) => None,
        })
        .collect();
    sub.schedule().ok() == Some(new.schedule)
        && sub.time_zone == new.time_zone
        && sub.attachments == new.attachments
        && sub.filters == new.filters
        && sub.params == new.params
        && sub.grain == new.grain
        && sub.skip_when_empty == new.skip_when_empty
        && new.recipients.iter().all(|r| kept.contains(r))
}

/// `DELETE /api/reports/subscriptions/{id}`.
///
/// # Errors
///
/// 404 / 403 as [`get`].
pub async fn delete(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let sub = own_or_admin(&state, &principal, &id).await?;
    subscriptions::delete(pool_of(&state)?, sub.id)
        .await
        .map_err(|e| store_failed(&e))?;
    Ok(ApiJson(json!({ "deleted": true })))
}

/// `POST /api/reports/subscriptions/{id}/send` — send it now, as a test: the
/// same report the schedule makes, recorded as a manual delivery, never counted
/// toward the five failures and never moving the schedule.
///
/// # Errors
///
/// 404 / 403 as [`get`]; a store failure under a reference. A report that could
/// not be made is a 200 whose delivery says `failed` and why.
pub async fn send_now(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let sub = own_or_admin(&state, &principal, &id).await?;
    // RPT-1 review fix (SHOULD-FIX) R5: sending the owner's data to the owner's
    // recipients on someone else's say-so is more than an administrator may.
    if sub.owner_id != principal.id.uuid() {
        return Err(ApiError::PermissionDenied(NOT_THE_OWNER.to_owned()).into());
    }
    let mailer =
        lakehouse_notify::EmailSender::new(crate::routes::alerts::smtp_config(&state.config));
    let delivery = run_subscription(&state, &mailer, &sub, true)
        .await
        .map_err(|e| store_failed(&e))?;
    Ok(ApiJson(delivery_json(&delivery)))
}

/// `GET /api/reports/subscriptions/{id}/deliveries` — the last 50 sends.
///
/// # Errors
///
/// 404 / 403 as [`get`].
pub async fn deliveries(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let sub = own_or_admin(&state, &principal, &id).await?;
    let rows = subscriptions::list_deliveries(pool_of(&state)?, sub.id, DELIVERIES_LIMIT)
        .await
        .map_err(|e| store_failed(&e))?;
    Ok(ApiJson(
        json!({ "items": rows.iter().map(delivery_json).collect::<Vec<_>>() }),
    ))
}

/// Query of `GET /api/reports/recipients`.
#[derive(Debug, Deserialize)]
pub struct PickerQuery {
    #[serde(default)]
    q: Option<String>,
}

/// `GET /api/reports/recipients?q=` — active users' names and addresses for the
/// recipient picker, at most 20, to anyone who may subscribe. Narrower than
/// `identity:read`'s full list on purpose: no roles, no tenants, no status.
///
/// The list is scoped to the caller's tenants (feature decision 8; `PR #204`
/// review (SHOULD-FIX) 1): a caller who belongs to tenants sees only the
/// people who share one of them, so the picker never offers a recipient the
/// save would refuse. A caller who belongs to no tenant sees everyone, which
/// is what a deployment that does not use tenants does.
///
/// # Errors
///
/// 403 when the caller may not download.
pub async fn recipients(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<PickerQuery>,
) -> ApiResult<ApiJson<Value>> {
    crate::routes::settings::require_download(&state, &principal).await?;
    let found = subscriptions::search_active_users(
        pool_of(&state)?,
        q.q.as_deref().unwrap_or("").trim(),
        PICKER_LIMIT,
        &principal.tenant_ids,
    )
    .await
    .map_err(|e| store_failed(&e))?;
    Ok(ApiJson(json!({
        "items": found.iter().map(|(id, name, email)| json!({
            "id": id, "name": name, "email": email,
        })).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn schedule(value: &Value) -> Result<Schedule, ApiError> {
        schedule_of(&serde_json::from_value(value.clone()).unwrap())
    }

    #[test]
    fn a_schedule_of_each_kind_is_read_and_a_missing_field_is_refused() {
        assert_eq!(
            schedule(&json!({ "kind": "hourly", "minute": 15 })).unwrap(),
            Schedule::Hourly { minute: 15 }
        );
        assert_eq!(
            schedule(&json!({ "kind": "weekly", "weekday": 1, "hour": 8 })).unwrap(),
            Schedule::Weekly {
                weekday: 1,
                hour: 8,
                minute: 0
            }
        );
        assert_eq!(
            schedule(&json!({ "kind": "monthly", "monthDay": "last", "hour": 23, "minute": 59 }))
                .unwrap(),
            Schedule::Monthly {
                day: MonthDay::Last,
                hour: 23,
                minute: 59
            }
        );
        for bad in [
            json!({ "kind": "daily" }),
            json!({ "kind": "weekly", "hour": 8 }),
            json!({ "kind": "weekly", "weekday": 8, "hour": 8 }),
            json!({ "kind": "monthly", "monthDay": 29, "hour": 8 }),
            json!({ "kind": "monthly", "monthDay": "first", "hour": 8 }),
            json!({ "kind": "daily", "hour": 24 }),
            json!({ "kind": "yearly", "hour": 1 }),
        ] {
            assert!(schedule(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_schedule_json_reads_back_the_same_schedule() {
        for s in [
            Schedule::Hourly { minute: 5 },
            Schedule::Daily {
                hour: 8,
                minute: 30,
            },
            Schedule::Weekly {
                weekday: 7,
                hour: 0,
                minute: 0,
            },
            Schedule::Monthly {
                day: MonthDay::Day(28),
                hour: 1,
                minute: 2,
            },
            Schedule::Monthly {
                day: MonthDay::Last,
                hour: 1,
                minute: 2,
            },
        ] {
            assert_eq!(schedule(&schedule_json(s)).unwrap(), s);
        }
    }

    #[test]
    fn an_unknown_field_in_a_body_is_refused() {
        let body = json!({
            "schedule": { "kind": "hourly" },
            "attachments": { "charts": [] },
            "recipients": [],
            "owner": "someone-else",
        });
        assert!(serde_json::from_value::<SubscriptionInput>(body).is_err());
    }
}
