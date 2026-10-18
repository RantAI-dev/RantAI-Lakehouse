//! Scheduled reports (`RPT-1`): the runner that sends them and the one function
//! that builds and sends a single report (plan decisions D3, D4, D5, D6;
//! feature page `docs/core/features/scheduled-reports.md`).
//!
//! # A report is made as the person who set it up (feature decision 1)
//!
//! [`run_subscription`] loads the owner by id with their roles *now*
//! ([`crate::routes::render::principal_of`], the loader the PDF's print view
//! uses), checks `dashboard:read` and the download right the way the print view
//! does ([`crate::routes::settings::may_download`]), and then reads everything
//! through the same code paths a download takes:
//!
//! - the PDF through [`crate::routes::pdf_export::make_pdf`] (the render token is
//!   signed only with `RENDER_TOKEN_SECRET`, `BI-1` review fix R1; the Gotenberg
//!   client and the two-slot semaphore are shared with the download route);
//! - each chart's file through [`crate::routes::dashboard::build_chart_file`]
//!   (the owner's masking and row filters through `rewrite_sql_for_roles`, the
//!   streamed writers, the one-million-row cap);
//! - the skip-when-empty check through the dashboard payload
//!   ([`crate::routes::dashboard::payload_for`]), so "empty" means what the
//!   screen shows.
//!
//! Every recipient, inside or outside the company, gets what the owner sees. A
//! disabled owner, or one who can no longer view or download, stops the run with
//! a recorded reason and nothing is sent. Each file that goes out is written to
//! the audit trail first (`BI-1` D8), and an audit store that cannot be written
//! stops the send.
//!
//! # What the delivery record says
//!
//! A delivery's `reason` is one of the sentences in this module or one of
//! `lakehouse_notify::DeliveryFailure`'s closed set; never text from the mail
//! server, Gotenberg or the engine (principle 4, SEC-11). Without email set up
//! the run records [`EMAIL_NOT_SET_UP`] and sends nothing (principle 2).
//!
//! # The runner
//!
//! [`tick`] claims at most five due subscriptions (`claim_due`: `FOR UPDATE SKIP
//! LOCKED`, `next_run_at` moved forward from the claim time in the same
//! transaction, so two replicas never take the same row and a missed run happens
//! once) and runs them one after the other. Running them one at a time keeps the
//! runner to a single PDF slot of the two a process has; "Send now" and the
//! downloads keep the other. A failed scheduled run adds one to the failure
//! count, tells the owner by email, and the fifth in a row pauses the
//! subscription. No per-user notification list exists in the console (the bell
//! lists open alerts and pending approvals for everyone), so the owner is told
//! by email, and the subscription's list shows the status and the reason.
//! "Send now" never counts toward the five.

use std::time::Duration;

use lakehouse_auth::Principal;
use lakehouse_bi::filters::FilterDef;
use lakehouse_bi::specs::ChartKind;
use lakehouse_bi::store;
use lakehouse_core::ApiError;
use lakehouse_notify::{Attachment, ReportMailer};
use lakehouse_store::StoreError;
use lakehouse_store::report_subscriptions::{
    self as subscriptions, Delivery, DeliveryStatus, NewDelivery, RecipientRow, Subscription,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio::io::AsyncReadExt;

use crate::pdf::{Orientation, Paper};
use crate::routes::dashboard::{
    ChartExportQuery, DashboardQuery, build_chart_file, export_file_name, payload_for,
    record_download_audit,
};
use crate::routes::dashboard_organise::viewer_of;
use crate::routes::pdf_export::{DashboardValues, PdfFailure, PdfJob, make_pdf};
use crate::state::AppState;

/// The attachments of one email, in all (feature decision 5).
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
/// Chart files in one email (feature decision 5).
pub const MAX_CHART_FILES: usize = 10;
/// People one subscription may address (feature decision 3).
pub const MAX_RECIPIENTS: usize = 50;
/// Subscriptions one tick takes.
const CLAIM_LIMIT: i64 = 5;
/// How often the runner looks for due subscriptions.
const TICK_EVERY: Duration = Duration::from_secs(60);
/// Deliveries older than this are removed once a day (feature decision 7).
pub const DELIVERY_RETENTION_DAYS: i64 = 90;
/// The subject of the email that tells an owner a scheduled send failed.
const OWNER_NOTICE_SUBJECT: &str = "A scheduled report could not be sent";

/// Email is not set up (`SMTP_HOST` empty): nothing is sent (feature decision 9).
pub const EMAIL_NOT_SET_UP: &str = "Email is not set up on this deployment.";
const OWNER_GONE: &str = "The person who set this report up is disabled or no longer exists.";
const OWNER_NO_ACCESS: &str =
    "The person who set this report up can no longer view or download dashboards.";
const BOARD_GONE: &str = "The dashboard no longer exists, or its owner can no longer see it.";
const NOBODY_LEFT: &str = "Nobody is left to send this report to.";
const EVERY_CHART_EMPTY: &str = "Every chart on the dashboard had no rows.";
const DATABASE_DOWN: &str = "The database is unavailable.";
const DASHBOARD_UNREADABLE: &str = "The dashboard could not be read.";
const CHART_FILE_FAILED: &str = "A chart's data file could not be made.";
const AUDIT_DOWN: &str = "The audit trail could not be written, so nothing was sent.";
const PDF_NOT_SET_UP: &str = "The PDF service is not set up on this deployment.";
const PDF_BUSY: &str = "The PDF service was busy; the next run will try again.";
const PDF_TIMEOUT: &str = "The PDF took too long to make.";
const PDF_UNREACHABLE: &str = "The PDF service is not reachable.";
const PDF_FAILED: &str = "The PDF could not be made.";
/// Paused after the fifth failure in a row (feature decision 7).
pub const PAUSED_AFTER_FAILURES: &str =
    "Paused after 5 failed sends in a row. Fix the cause, then resume it.";

// ── What a subscription attaches ─────────────────────────────────────────

/// The PDF of the dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PdfAttachment {
    /// `a4` or `letter`.
    pub paper: String,
    /// `portrait` or `landscape`.
    pub orientation: String,
}

/// One chart's data file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartAttachment {
    /// The tile's id.
    pub chart: String,
    /// `xlsx` or `csv`.
    pub format: String,
    /// `formatted` or `raw`.
    #[serde(default = "formatted")]
    pub values: String,
}

fn formatted() -> String {
    "formatted".to_owned()
}

/// What a subscription attaches: the shape of `report_subscription.attachments`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentSpec {
    /// The dashboard as a PDF, or none.
    #[serde(default)]
    pub pdf: Option<PdfAttachment>,
    /// Up to [`MAX_CHART_FILES`] chart files, in the order they are attached.
    #[serde(default)]
    pub charts: Vec<ChartAttachment>,
}

impl AttachmentSpec {
    /// Check the closed lists and the count.
    ///
    /// # Errors
    ///
    /// The sentence the person is told.
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(pdf) = &self.pdf {
            if Paper::parse(&pdf.paper).is_none() {
                return Err("paper must be a4 or letter.");
            }
            if Orientation::parse(&pdf.orientation).is_none() {
                return Err("orientation must be portrait or landscape.");
            }
        }
        if self.pdf.is_none() && self.charts.is_empty() {
            return Err("Choose at least one thing to attach: the PDF or a chart's data.");
        }
        if self.charts.len() > MAX_CHART_FILES {
            return Err("A report attaches at most 10 chart files.");
        }
        for c in &self.charts {
            if c.chart.trim().is_empty() || c.chart.len() > 200 {
                return Err("Every chart file needs a chart.");
            }
            if !matches!(c.format.as_str(), "xlsx" | "csv") {
                return Err("A chart file is xlsx or csv.");
            }
            if !matches!(c.values.as_str(), "formatted" | "raw") {
                return Err("values must be formatted or raw.");
            }
        }
        Ok(())
    }
}

// ── Addresses ────────────────────────────────────────────────────────────

/// A plain address, lower-cased: one `@`, a short local part without spaces or
/// the characters that make an address a list or a display name, and a domain
/// of dot-separated labels (letters, digits, hyphens). `None` for anything
/// else. Deliberately narrower than RFC 5322: a report goes to an address a
/// person typed, and the narrow shape is the one `lettre` and every mail
/// server accept.
#[must_use]
pub fn normalise_address(raw: &str) -> Option<String> {
    let address = raw.trim().to_ascii_lowercase();
    let (local, domain) = address.split_once('@')?;
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && local.bytes().all(|b| {
            b.is_ascii_graphic()
                && !matches!(
                    b,
                    b'<' | b'>'
                        | b'('
                        | b')'
                        | b','
                        | b';'
                        | b':'
                        | b'"'
                        | b'\\'
                        | b'['
                        | b']'
                        | b'@'
                )
        });
    (local_ok && domain_is_valid(domain) && address.len() <= 254).then_some(address)
}

/// A domain name for the allow-list or an address: at least two dot-separated
/// labels of letters, digits and hyphens, none empty or edged by a hyphen, at
/// most 253 characters. No wildcard.
#[must_use]
pub fn domain_is_valid(domain: &str) -> bool {
    let labels: Vec<&str> = domain.split('.').collect();
    domain.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// Whether `address` (already lower-cased) is at one of the `allowed` domains
/// (exact match; `a.example.com` is not covered by `example.com`).
#[must_use]
pub fn domain_allowed(address: &str, allowed: &[String]) -> bool {
    address
        .rsplit_once('@')
        .is_some_and(|(_, domain)| allowed.iter().any(|a| a == domain))
}

// ── The outcome of one run ───────────────────────────────────────────────

/// How one run ended, before it is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The email went out; `note` lists what was left out, in a sentence.
    Sent {
        /// Files or recipients left out, if any.
        note: Option<String>,
    },
    /// Nothing was sent on purpose.
    Skipped(String),
    /// Nothing was sent, and why.
    Failed(String),
}

impl Outcome {
    fn status(&self) -> DeliveryStatus {
        match self {
            Self::Sent { .. } => DeliveryStatus::Sent,
            Self::Skipped(_) => DeliveryStatus::Skipped,
            Self::Failed(_) => DeliveryStatus::Failed,
        }
    }

    fn reason(&self) -> Option<String> {
        match self {
            Self::Sent { note } => note.clone(),
            Self::Skipped(reason) | Self::Failed(reason) => Some(reason.clone()),
        }
    }
}

/// A run that stopped before anything was sent: the reason, in a sentence.
type Stop = String;

fn stop(reason: &str) -> Stop {
    reason.to_owned()
}

/// The longest reason a delivery record holds (the table's `CHECK` is 500
/// characters).
const MAX_REASON_CHARS: usize = 500;

/// `reason` cut to fit the record, on a character boundary, with an ellipsis
/// when it was cut. `RPT-1` review fix (SHOULD-FIX) R3: ten left-out charts made
/// a note longer than the `CHECK`, and the insert failed after the email had
/// gone. The email carries the whole list; the record keeps the start of it.
fn cap_reason(reason: &str) -> String {
    if reason.chars().count() <= MAX_REASON_CHARS {
        return reason.to_owned();
    }
    let kept: String = reason.chars().take(MAX_REASON_CHARS - 1).collect();
    format!("{kept}…")
}

/// Run one subscription now and record what happened. The scheduled runner
/// calls it for each claimed row (`manual: false`); "Send now" calls it with
/// `manual: true`, which records the delivery and sends the failure notice but
/// never counts toward the five failures in a row and never moves the schedule.
///
/// # Errors
///
/// [`StoreError`] when the delivery record or the failure count cannot be
/// written. The email itself, if one went out, is already sent.
pub async fn run_subscription(
    state: &AppState,
    mailer: &dyn ReportMailer,
    sub: &Subscription,
    manual: bool,
) -> Result<Delivery, StoreError> {
    let Some(pool) = state.pg.as_deref() else {
        return Err(StoreError::Unavailable);
    };
    let started_at = OffsetDateTime::now_utc();
    let (outcome, recipient_count) = match build_and_send(state, mailer, sub, manual).await {
        Ok(sent) => sent,
        Err(reason) => (Outcome::Failed(reason), 0),
    };
    let delivery = subscriptions::record_delivery(
        pool,
        &NewDelivery {
            subscription_id: sub.id,
            started_at,
            finished_at: OffsetDateTime::now_utc(),
            status: outcome.status(),
            manual,
            recipient_count: i32::try_from(recipient_count).unwrap_or(i32::MAX),
            // RPT-1 review fix (SHOULD-FIX) R3.
            reason: outcome.reason().map(|r| cap_reason(&r)),
        },
    )
    .await?;
    if let Outcome::Failed(reason) = &outcome {
        let paused_now = if manual {
            false
        } else {
            subscriptions::note_outcome(pool, sub.id, DeliveryStatus::Failed, PAUSED_AFTER_FAILURES)
                .await?
                .is_some_and(|s| s.paused_now)
        };
        tell_owner(state, mailer, sub, reason, paused_now).await;
    } else if !manual {
        subscriptions::note_outcome(pool, sub.id, outcome.status(), PAUSED_AFTER_FAILURES).await?;
    }
    Ok(delivery)
}

/// Everything between "this subscription is due" and "the email is handed to
/// the mail server". `Ok` is a send or a skip with the number of people it was
/// addressed to; `Err` is the reason nothing was sent.
async fn build_and_send(
    state: &AppState,
    mailer: &dyn ReportMailer,
    sub: &Subscription,
    manual: bool,
) -> Result<(Outcome, usize), Stop> {
    let pool = state.pg.as_deref().ok_or_else(|| stop(DATABASE_DOWN))?;
    // Checked first: no point making a PDF nobody can send.
    if !mailer.is_configured() {
        return Err(stop(EMAIL_NOT_SET_UP));
    }
    let owner = load_owner(state, sub).await?;
    let settings = crate::routes::settings::format_settings(state)
        .await
        .map_err(|_| stop(DATABASE_DOWN))?;
    if !owner.has("dashboard:read")
        || !crate::routes::settings::may_download(&owner, &settings.download_denied_roles)
    {
        return Err(stop(OWNER_NO_ACCESS));
    }
    let board = store::get_board(&state.clickhouse, &sub.board, &viewer_of(&owner))
        .await
        .map_err(|_| stop(DASHBOARD_UNREADABLE))?;
    if board.is_none() && sub.board != store::DEFAULT_BOARD_ID {
        return Err(stop(BOARD_GONE));
    }
    let board_name = board
        .as_ref()
        .map_or("Main", |b| b.name.as_str())
        .to_owned();

    let rows = subscriptions::recipients(pool, sub.id)
        .await
        .map_err(|_| stop(DATABASE_DOWN))?;
    let allowed_users =
        users_who_may_receive(state, &rows, &owner, &settings.download_denied_roles).await;
    let audience = audience_of(&rows, &settings.report_allowed_domains, &allowed_users);
    if audience.addresses.is_empty() {
        return Err(stop(NOBODY_LEFT));
    }

    let texts = ValueTexts::of(sub);
    let values = texts.as_values();
    if sub.skip_when_empty && is_empty_report(state, &owner, sub, &values).await? {
        return Ok((
            Outcome::Skipped(EVERY_CHART_EMPTY.to_owned()),
            audience.addresses.len(),
        ));
    }

    let spec: AttachmentSpec =
        serde_json::from_value(sub.attachments.clone()).map_err(|_| stop(PDF_FAILED))?;
    let who = AuditWho {
        count: audience.addresses.len(),
        outside: audience.outside.clone(),
        origin: if manual { "manual" } else { "schedule" },
    };
    let files = attach(state, &owner, sub, (&spec, &values), (&board_name, who)).await?;

    let now = OffsetDateTime::now_utc();
    let date = lakehouse_store::report_schedule::local_date(&sub.time_zone, now)
        .unwrap_or_else(|| now.date().to_string());
    let mail = MailParts {
        board_name: &board_name,
        owner_name: &owner.display_name,
        filters: sub
            .filters
            .as_ref()
            .and_then(|f| serde_json::from_value::<Vec<FilterDef>>(f.clone()).ok())
            .unwrap_or_default(),
        link: dashboard_link(state, &sub.board),
        sent: files.sent_names(),
        left_out: &files.left_out,
        recipients_left_out: audience.left_out,
    };
    let result = mailer
        .send_with_attachments(
            &audience.addresses,
            &format!("{board_name} report, {date}"),
            &report_html(&mail),
            &files.attachments,
        )
        .await;
    if !result.ok {
        return Err(stop(email_failure_reason(result.error.as_deref())));
    }
    let mut notes = files.left_out.clone();
    if audience.left_out > 0 {
        notes.push(left_out_recipients(audience.left_out));
    }
    Ok((
        Outcome::Sent {
            note: (!notes.is_empty()).then(|| notes.join(" ")),
        },
        audience.addresses.len(),
    ))
}

/// The sentence for a failed hand-over to the mail server. `error` is one of
/// `lakehouse_notify::DeliveryFailure`'s closed set of sentences; each is matched
/// to a sentence written here, and anything else (a future reason) becomes the
/// plain "was not sent", so no text from the other side can pass through
/// (principle 4, SEC-11).
fn email_failure_reason(error: Option<&str>) -> &'static str {
    use lakehouse_notify::DeliveryFailure as F;
    match error {
        Some(e) if e == F::EmailNotConfigured.message() => EMAIL_NOT_SET_UP,
        Some(e) if e == F::EmailAddressRefused.message() => {
            "The email was not sent: an address was refused."
        }
        Some(e) if e == F::MailServerUnreachable.message() => {
            "The email was not sent: the mail server could not be reached."
        }
        Some(e) if e == F::MailServerRefused.message() => {
            "The email was not sent: the mail server refused the message."
        }
        _ => "The email was not sent.",
    }
}

/// The owner as they are now, or the sentence for why they cannot be used.
async fn load_owner(state: &AppState, sub: &Subscription) -> Result<Principal, Stop> {
    match crate::routes::render::principal_of(state, &sub.owner_id.to_string()).await {
        Ok(Some(principal)) => Ok(principal),
        Ok(None) => Err(stop(OWNER_GONE)),
        Err(_) => Err(stop(DATABASE_DOWN)),
    }
}

// ── Recipients ───────────────────────────────────────────────────────────

/// Who an email is addressed to, and how many of the subscription's recipients
/// were left out.
#[derive(Debug, PartialEq, Eq)]
struct Audience {
    addresses: Vec<String>,
    /// The addresses among them that are outside addresses (not console users).
    outside: Vec<String>,
    left_out: usize,
}

/// Who a report's files went to, written into each file's audit entry (`RPT-1`
/// review fix (SHOULD-FIX) R6): the trail says what left and for whom.
struct AuditWho {
    count: usize,
    outside: Vec<String>,
    origin: &'static str,
}

/// The addresses to send to: a console user who is active (their address as it
/// is now), an outside address whose domain the administrator still lists. The
/// rules are checked again at send time because both can change after the
/// subscription was saved; whoever no longer qualifies is left out and counted.
/// Duplicates (the same address as a user and as an outside address) go once.
fn audience_of(
    rows: &[RecipientRow],
    allowed_domains: &[String],
    allowed_users: &std::collections::HashSet<uuid::Uuid>,
) -> Audience {
    let mut addresses: Vec<String> = Vec::new();
    let mut outside: Vec<String> = Vec::new();
    let mut left_out = 0;
    for row in rows {
        let address = match (&row.user_id, &row.email) {
            (Some(id), _) => (row.user_status.as_deref() == Some("active")
                && allowed_users.contains(id))
            .then(|| row.user_email.as_deref().and_then(normalise_address))
            .flatten(),
            (None, Some(email)) => {
                normalise_address(email).filter(|a| domain_allowed(a, allowed_domains))
            }
            (None, None) => None,
        };
        match address {
            Some(a) if addresses.contains(&a) => {}
            Some(a) => {
                if row.user_id.is_none() {
                    outside.push(a.clone());
                }
                addresses.push(a);
            }
            None => left_out += 1,
        }
    }
    Audience {
        addresses,
        outside,
        left_out,
    }
}

/// The console recipients who may be sent this report today (`RPT-1` review fix
/// (SHOULD-FIX) R8; planner default, the owner confirms at the QA): they can
/// still view dashboards, their roles may download, and, when the owner belongs
/// to tenants, they share one with the owner. Someone who fails any of these is
/// left out and counted, as a disabled person is.
async fn users_who_may_receive(
    state: &AppState,
    rows: &[RecipientRow],
    owner: &Principal,
    denied_roles: &[String],
) -> std::collections::HashSet<uuid::Uuid> {
    let mut allowed = std::collections::HashSet::new();
    for id in rows.iter().filter_map(|r| r.user_id) {
        let Ok(Some(person)) = crate::routes::render::principal_of(state, &id.to_string()).await
        else {
            continue;
        };
        let shares_a_tenant = owner.tenant_ids.is_empty()
            || person
                .tenant_ids
                .iter()
                .any(|t| owner.tenant_ids.contains(t));
        if person.has("dashboard:read")
            && crate::routes::settings::may_download(&person, denied_roles)
            && shares_a_tenant
        {
            allowed.insert(id);
        }
    }
    allowed
}

fn left_out_recipients(count: usize) -> String {
    if count == 1 {
        "1 recipient was left out: they are disabled, may no longer view or download dashboards, are outside the owner's tenants, or their domain is no longer allowed."
            .to_owned()
    } else {
        format!(
            "{count} recipients were left out: they are disabled, may no longer view or download dashboards, are outside the owner's tenants, or their domain is no longer allowed."
        )
    }
}

// ── Reading the dashboard ────────────────────────────────────────────────

/// The filter, parameter and grain text of a subscription, as the dashboard
/// routes take them (a stored JSON value goes back to the text the page sends).
struct ValueTexts {
    filters: Option<String>,
    params: Option<String>,
    grain: Option<String>,
}

impl ValueTexts {
    fn of(sub: &Subscription) -> Self {
        let text = |v: &Option<Value>| {
            v.as_ref()
                .filter(|v| !v.is_null())
                .map(std::string::ToString::to_string)
        };
        Self {
            filters: text(&sub.filters),
            params: text(&sub.params),
            grain: sub.grain.clone().filter(|g| !g.is_empty()),
        }
    }

    fn as_values(&self) -> DashboardValues<'_> {
        DashboardValues {
            filters: self.filters.as_deref(),
            params: self.params.as_deref(),
            grain: self.grain.as_deref(),
        }
    }
}

/// Whether every data chart of the dashboard has no rows for the owner with the
/// subscription's values (feature decision 6). A dashboard with no data chart
/// is not empty (a text-only page still has something to say), and a chart that
/// failed to read is not "empty": we cannot say so, so the report is sent.
async fn is_empty_report(
    state: &AppState,
    owner: &Principal,
    sub: &Subscription,
    values: &DashboardValues<'_>,
) -> Result<bool, Stop> {
    let query = DashboardQuery::for_render(&sub.board, values.filters, values.params, values.grain);
    let payload = payload_for(state, owner, &query)
        .await
        .map_err(|_| stop(DASHBOARD_UNREADABLE))?;
    Ok(every_data_chart_is_empty(&payload))
}

/// The pure half of [`is_empty_report`].
fn every_data_chart_is_empty(payload: &Value) -> bool {
    let Some(charts) = payload.get("charts").and_then(Value::as_array) else {
        return false;
    };
    let mut data_charts = 0;
    for chart in charts {
        let is_static = chart
            .get("kind")
            .cloned()
            .and_then(|k| serde_json::from_value::<ChartKind>(k).ok())
            .is_some_and(ChartKind::is_static);
        if is_static {
            continue;
        }
        let Some(id) = chart.get("id").and_then(Value::as_str) else {
            return false;
        };
        let result = payload.get("results").and_then(|r| r.get(id));
        let Some(rows) = result.and_then(|r| r.get("rows")).and_then(Value::as_array) else {
            // An error tile or a missing result: not provably empty.
            return false;
        };
        data_charts += 1;
        if !rows.is_empty() {
            return false;
        }
    }
    data_charts > 0
}

// ── Files ────────────────────────────────────────────────────────────────

/// The files of one email and what was left out.
#[derive(Default)]
struct Files {
    attachments: Vec<Attachment>,
    left_out: Vec<String>,
}

impl Files {
    fn sent_names(&self) -> Vec<String> {
        self.attachments
            .iter()
            .map(|a| a.file_name.clone())
            .collect()
    }
}

/// The 20 MB budget of an email, counted on the files' bytes.
#[derive(Default)]
struct Budget {
    used: usize,
}

impl Budget {
    /// Take `size` bytes if they fit.
    fn take(&mut self, size: usize) -> bool {
        if self.used.saturating_add(size) > MAX_ATTACHMENT_BYTES {
            return false;
        }
        self.used += size;
        true
    }
}

/// A file name not used yet in this email: the second `Revenue.xlsx` becomes
/// `Revenue-2.xlsx`.
fn unique_name(taken: &[Attachment], wanted: &str) -> String {
    if !taken.iter().any(|a| a.file_name == wanted) {
        return wanted.to_owned();
    }
    let (stem, extension) = wanted.rsplit_once('.').unwrap_or((wanted, ""));
    for n in 2_u32..1000 {
        let candidate = if extension.is_empty() {
            format!("{stem}-{n}")
        } else {
            format!("{stem}-{n}.{extension}")
        };
        if !taken.iter().any(|a| a.file_name == candidate) {
            return candidate;
        }
    }
    // Not reachable with ten files at most; a name that repeats is still a name.
    wanted.to_owned()
}

/// One audit entry waiting for the files to be complete: action, resource kind,
/// resource id, arguments.
type AuditEntry = (&'static str, &'static str, String, Value);

/// What [`attach`] collects: the files, what they have used of the budget, and
/// the audit entries to write before any of it is sent.
struct Collected<'a> {
    state: &'a AppState,
    owner: &'a Principal,
    sub: &'a Subscription,
    values: &'a DashboardValues<'a>,
    who: AuditWho,
    files: Files,
    budget: Budget,
    audit: Vec<AuditEntry>,
}

impl Collected<'_> {
    /// The PDF, or a line saying it did not fit.
    async fn add_pdf(&mut self, pdf: &PdfAttachment, board_name: &str) -> Result<(), Stop> {
        let (Some(paper), Some(orientation)) = (
            Paper::parse(&pdf.paper),
            Orientation::parse(&pdf.orientation),
        ) else {
            return Err(stop(PDF_FAILED));
        };
        let bytes = make_pdf(
            self.state,
            &PdfJob {
                board: &self.sub.board,
                person: self.owner.id.uuid(),
                values: *self.values,
                paper,
                orientation,
            },
        )
        .await
        .map_err(|failure| {
            // RPT-1 review fix (SHOULD-FIX) R7: the stored sentence is fixed;
            // the cause goes to the log under a reference, as the download
            // route does it.
            if let PdfFailure::Conversion(err) = &failure {
                let _logged = crate::routes::pdf_export::pdf_failure(err);
            } else {
                tracing::warn!(subscription = %self.sub.id, "scheduled report: no PDF was made");
            }
            stop(pdf_reason(&failure))
        })?;
        if !self.budget.take(bytes.len()) {
            self.files.left_out.push(format!(
                "The PDF was left out: with it the email would pass {} MB.",
                MAX_ATTACHMENT_BYTES / (1024 * 1024)
            ));
            return Ok(());
        }
        self.audit.push((
            "report.pdf_attachment",
            "dashboard",
            self.sub.board.clone(),
            json!({
                "subscription": self.sub.id,
                "board": self.sub.board,
                "paper": paper.as_str(),
                "orientation": orientation.as_str(),
                "bytes": bytes.len(),
                "recipientCount": self.who.count,
                "outsideAddresses": self.who.outside,
                "origin": self.who.origin,
            }),
        ));
        self.files.attachments.push(Attachment {
            file_name: format!(
                "{}.pdf",
                crate::routes::dashboard::file_slug(board_name, "dashboard")
            ),
            content_type: "application/pdf".to_owned(),
            bytes,
        });
        Ok(())
    }

    /// One chart's file, or a line saying why it is not in the email.
    async fn add_chart(&mut self, wanted: &ChartAttachment) -> Result<(), Stop> {
        let query = ChartExportQuery::for_report(
            &wanted.chart,
            (&wanted.format, &wanted.values),
            self.values,
        );
        let file = match build_chart_file(self.state, self.owner, &query, false).await {
            Ok(file) => file,
            Err(ApiError::NotFound(_) | ApiError::BadRequest(_)) => {
                self.files.left_out.push(
                    "A chart's data was left out: the chart no longer exists, shows no data, or does not fit this report's filters."
                        .to_owned(),
                );
                return Ok(());
            }
            Err(other) => {
                // RPT-1 review fix (SHOULD-FIX) R7: the engine's cause was
                // logged under a reference when the error was made; this says
                // which report and chart it belongs to.
                tracing::warn!(
                    subscription = %self.sub.id,
                    chart = %wanted.chart,
                    error = %other,
                    "scheduled report: a chart's file could not be made"
                );
                return Err(stop(CHART_FILE_FAILED));
            }
        };
        let title = file.chart.spec.title.clone();
        let cap = crate::export::EXPORT_ROW_CAP;
        let cut = file.written.cut;
        let name = unique_name(
            &self.files.attachments,
            &export_file_name(&title, file.format.extension(), cut.then_some(cap)),
        );
        let len = usize::try_from(file.written.len).unwrap_or(usize::MAX);
        if !self.budget.take(len) {
            self.files.left_out.push(format!(
                "\"{title}\" was left out: with it the email would pass {} MB.",
                MAX_ATTACHMENT_BYTES / (1024 * 1024)
            ));
            return Ok(());
        }
        let mut bytes = Vec::with_capacity(len);
        let mut handle = file.written.file;
        handle.read_to_end(&mut bytes).await.map_err(|err| {
            tracing::warn!(%err, subscription = %self.sub.id, "scheduled report: a chart's file could not be read back");
            stop(CHART_FILE_FAILED)
        })?;
        if cut {
            self.files
                .left_out
                .push(format!("\"{title}\" holds only its first {cap} rows."));
        }
        self.audit.push((
            "report.chart_attachment",
            "dashboard_tile",
            file.chart.spec.id.clone(),
            json!({
                "subscription": self.sub.id,
                "board": self.sub.board,
                "tile": file.chart.spec.id,
                "rows": file.written.rows,
                "cut": cut,
                "format": file.format.extension(),
                "bytes": bytes.len(),
                "recipientCount": self.who.count,
                "outsideAddresses": self.who.outside,
                "origin": self.who.origin,
            }),
        ));
        self.files.attachments.push(Attachment {
            file_name: name,
            content_type: file.format.content_type().to_owned(),
            bytes,
        });
        Ok(())
    }
}

/// Make the PDF and the chart files, in that order, within the budget. A file
/// that does not fit, or a chart that is gone, is left out and said so (feature
/// decision 5); a file that fails for any other reason stops the run (a report
/// without what it promised is not sent as if it were whole). Every file that
/// goes out is written to the audit trail before this returns.
async fn attach(
    state: &AppState,
    owner: &Principal,
    sub: &Subscription,
    (spec, values): (&AttachmentSpec, &DashboardValues<'_>),
    (board_name, who): (&str, AuditWho),
) -> Result<Files, Stop> {
    let mut collected = Collected {
        state,
        owner,
        sub,
        values,
        who,
        files: Files::default(),
        budget: Budget::default(),
        audit: Vec::new(),
    };
    if let Some(pdf) = &spec.pdf {
        collected.add_pdf(pdf, board_name).await?;
    }
    for wanted in &spec.charts {
        collected.add_chart(wanted).await?;
    }
    // BI-1 D8: the record comes before the file leaves. A trail that cannot be
    // written stops the send.
    for (action, kind, id, args) in collected.audit {
        record_download_audit(state, owner, (action, kind, &id), args)
            .await
            .map_err(|_| stop(AUDIT_DOWN))?;
    }
    Ok(collected.files)
}

fn pdf_reason(failure: &PdfFailure) -> &'static str {
    match failure {
        PdfFailure::NotSetUp => PDF_NOT_SET_UP,
        PdfFailure::Busy => PDF_BUSY,
        PdfFailure::Conversion(crate::pdf::PdfError::Timeout) => PDF_TIMEOUT,
        PdfFailure::Conversion(crate::pdf::PdfError::Unreachable(_)) => PDF_UNREACHABLE,
        PdfFailure::Conversion(_) => PDF_FAILED,
    }
}

// ── The email ────────────────────────────────────────────────────────────

/// What the body of a report says.
struct MailParts<'a> {
    board_name: &'a str,
    owner_name: &'a str,
    filters: Vec<FilterDef>,
    link: Option<String>,
    sent: Vec<String>,
    left_out: &'a [String],
    recipients_left_out: usize,
}

/// The link to the dashboard in the console, from `CONSOLE_PUBLIC_URL` (never
/// from a request). `None` when the deployment did not say where the console is.
fn dashboard_link(state: &AppState, board: &str) -> Option<String> {
    let base = state.config.console_public_url.as_deref()?;
    let id: String = board
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    Some(format!("{base}/dashboards/{id}"))
}

/// Text for an HTML body: the five characters that matter.
fn html_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// A filter in words, for the body: `region: Bali, Jawa`.
fn describe_filter(f: &FilterDef) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !f.values.is_empty() {
        parts.push(f.values.join(", "));
    }
    match (&f.min, &f.max) {
        (Some(min), Some(max)) => parts.push(format!("{min} to {max}")),
        (Some(min), None) => parts.push(format!("from {min}")),
        (None, Some(max)) => parts.push(format!("up to {max}")),
        (None, None) => {}
    }
    if let Some(text) = &f.text {
        parts.push(format!("text \"{text}\""));
    }
    if parts.is_empty() {
        f.column.clone()
    } else {
        format!("{}: {}", f.column, parts.join("; "))
    }
}

/// A heading and a bulleted list of already-escaped items.
fn html_list(heading: &str, items: &[String]) -> String {
    let list: Vec<String> = items.iter().map(|i| format!("<li>{i}</li>")).collect();
    format!("<p>{heading}</p><ul>{}</ul>", list.concat())
}

/// The report email's body. Every value that came from a person or the
/// dashboard is escaped.
fn report_html(mail: &MailParts<'_>) -> String {
    let mut blocks: Vec<String> = vec![
        format!(
            "<h3 style=\"margin:0 0 8px\">{}</h3>",
            html_text(mail.board_name)
        ),
        format!(
            "<p>This report was set up by {}.</p>",
            html_text(mail.owner_name)
        ),
    ];
    if mail.filters.is_empty() {
        blocks.push("<p>Filters: none.</p>".to_owned());
    } else {
        let items: Vec<String> = mail
            .filters
            .iter()
            .map(|f| html_text(&describe_filter(f)))
            .collect();
        blocks.push(html_list("Filters:", &items));
    }
    blocks.push(format!(
        "<p>The data in this report is what {} is allowed to see. Everyone it is sent to sees the same data.</p>",
        html_text(mail.owner_name)
    ));
    if !mail.sent.is_empty() {
        let items: Vec<String> = mail.sent.iter().map(|n| html_text(n)).collect();
        blocks.push(html_list("Attached:", &items));
    }
    let mut notes: Vec<String> = mail.left_out.iter().map(|n| html_text(n)).collect();
    if mail.recipients_left_out > 0 {
        notes.push(html_text(&left_out_recipients(mail.recipients_left_out)));
    }
    if !notes.is_empty() {
        blocks.push(html_list("Left out:", &notes));
    }
    if let Some(link) = &mail.link {
        blocks.push(format!(
            "<p><a href=\"{}\">Open the dashboard</a></p>",
            html_text(link)
        ));
    }
    blocks.push(
        "<p style=\"color:#888;font-size:12px;margin-top:16px\">Rantai Lake, scheduled report</p>"
            .to_owned(),
    );
    format!(
        "<div style=\"font-family:system-ui,sans-serif;font-size:14px\">{}</div>",
        blocks.concat()
    )
}

/// Tell the owner a scheduled send failed (feature decision 7). The console has
/// no per-person notification list, so this is an email to the owner's own
/// address; a failure to send it is logged and nothing more (the delivery
/// record and the subscription's list already say what happened).
async fn tell_owner(
    state: &AppState,
    mailer: &dyn ReportMailer,
    sub: &Subscription,
    reason: &str,
    paused_now: bool,
) {
    if !mailer.is_configured() {
        return;
    }
    let Some(pool) = state.pg.as_deref() else {
        return;
    };
    let address = match lakehouse_store::identity::get_user(pool, &sub.owner_id.to_string()).await {
        Ok(user) if user.status == "active" => normalise_address(&user.email),
        _ => None,
    };
    let Some(address) = address else {
        return;
    };
    let mut blocks = vec![
        "<p>A scheduled report could not be sent.</p>".to_owned(),
        format!("<p>{}</p>", html_text(reason)),
    ];
    if paused_now {
        blocks.push(format!("<p>{}</p>", html_text(PAUSED_AFTER_FAILURES)));
    }
    if let Some(link) = dashboard_link(state, &sub.board) {
        blocks.push(format!(
            "<p><a href=\"{}\">Open the dashboard</a></p>",
            html_text(&link)
        ));
    }
    let html = format!(
        "<div style=\"font-family:system-ui,sans-serif;font-size:14px\">{}</div>",
        blocks.concat()
    );
    let result = mailer
        .send_with_attachments(&[address], OWNER_NOTICE_SUBJECT, &html, &[])
        .await;
    if !result.ok {
        tracing::warn!(
            subscription = %sub.id,
            reason = result.error.as_deref().unwrap_or(""),
            "could not tell the owner a scheduled report failed"
        );
    }
}

// ── The runner ───────────────────────────────────────────────────────────

/// Take the subscriptions that are due now and run each. Returns how many were
/// taken. A failure to claim is logged and the next tick tries again; a
/// failure to record one run is logged and the rest still run.
pub async fn tick(state: &AppState, mailer: &dyn ReportMailer) -> usize {
    let Some(pool) = state.pg.as_deref() else {
        return 0;
    };
    let now = OffsetDateTime::now_utc();
    let claimed = match subscriptions::claim_due(pool, now, CLAIM_LIMIT, |sub| {
        sub.schedule()
            .ok()
            .and_then(|s| s.next_run_after(&sub.time_zone, now).ok())
    })
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(%err, "report runner could not claim due subscriptions");
            return 0;
        }
    };
    for sub in &claimed {
        if let Err(err) = run_subscription(state, mailer, sub, false).await {
            tracing::warn!(%err, subscription = %sub.id, "report run could not be recorded");
        }
    }
    claimed.len()
}

/// Remove deliveries older than [`DELIVERY_RETENTION_DAYS`]. How many went.
pub async fn prune(state: &AppState) -> u64 {
    let Some(pool) = state.pg.as_deref() else {
        return 0;
    };
    let cutoff = OffsetDateTime::now_utc() - time::Duration::days(DELIVERY_RETENTION_DAYS);
    match subscriptions::prune_deliveries(pool, cutoff).await {
        Ok(removed) => removed,
        Err(err) => {
            tracing::warn!(%err, "report delivery cleanup failed");
            0
        }
    }
}

/// Start the report runner beside the other background tasks (plan D3): a tick
/// every minute with missed ticks skipped, and the 90-day cleanup once a day.
/// Without a Postgres pool it logs once and spawns nothing.
pub fn spawn(state: &AppState) {
    if state.pg.is_none() {
        tracing::warn!("no Postgres pool configured; the scheduled-report runner will not run");
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        let mailer =
            lakehouse_notify::EmailSender::new(crate::routes::alerts::smtp_config(&state.config));
        let mut interval = tokio::time::interval(TICK_EVERY);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_cleanup: Option<OffsetDateTime> = None;
        loop {
            interval.tick().await;
            let taken = tick(&state, &mailer).await;
            if taken > 0 {
                tracing::info!(taken, "scheduled reports run");
            }
            let now = OffsetDateTime::now_utc();
            if last_cleanup.is_none_or(|at| now - at >= time::Duration::days(1)) {
                let removed = prune(&state).await;
                if removed > 0 {
                    tracing::info!(removed, "scheduled-report delivery cleanup complete");
                }
                last_cleanup = Some(now);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use uuid::Uuid;

    use super::*;

    fn user_row(status: &str, email: &str) -> RecipientRow {
        RecipientRow {
            user_id: Some(Uuid::new_v4()),
            email: None,
            user_name: Some("U".to_owned()),
            user_email: Some(email.to_owned()),
            user_status: Some(status.to_owned()),
        }
    }

    fn outside_row(email: &str) -> RecipientRow {
        RecipientRow {
            user_id: None,
            email: Some(email.to_owned()),
            user_name: None,
            user_email: None,
            user_status: None,
        }
    }

    #[test]
    fn addresses_are_lower_cased_and_must_have_one_at_and_a_dotted_domain() {
        assert_eq!(
            normalise_address("  Ann.Lee+x@Example.COM ").as_deref(),
            Some("ann.lee+x@example.com")
        );
        for bad in [
            "",
            "nobody",
            "a@b",
            "@example.com",
            "a@@example.com",
            "a b@example.com",
            "a@exa mple.com",
            "Ann <a@example.com>",
            "a,b@example.com",
            "a@-example.com",
            "a@example..com",
            "a@example.com.",
            "a@exa_mple.com",
        ] {
            assert_eq!(normalise_address(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_domain_is_valid_without_wildcards_and_an_address_matches_it_exactly() {
        assert!(domain_is_valid("example.com"));
        assert!(domain_is_valid("sub.partner.co.id"));
        for bad in [
            "*.example.com",
            "example",
            "",
            "a..b",
            "-a.com",
            "a.com-",
            "exa mple.com",
        ] {
            assert!(!domain_is_valid(bad), "{bad:?}");
        }
        let allowed = vec!["example.com".to_owned()];
        assert!(domain_allowed("a@example.com", &allowed));
        assert!(
            !domain_allowed("a@sub.example.com", &allowed),
            "no subdomains"
        );
        assert!(!domain_allowed("a@evil-example.com", &allowed));
        assert!(!domain_allowed("a@example.com", &[]));
    }

    #[test]
    fn the_audience_leaves_out_disabled_people_and_domains_no_longer_listed() {
        let rows = vec![
            user_row("active", "ann@company.example"),
            user_row("inactive", "bob@company.example"),
            outside_row("press@partner.example"),
            outside_row("x@unlisted.example"),
            // The same person as a user and as an address goes once.
            outside_row("ANN@company.example"),
        ];
        let everyone: std::collections::HashSet<uuid::Uuid> =
            rows.iter().filter_map(|r| r.user_id).collect();
        let got = audience_of(
            &rows,
            &["partner.example".to_owned(), "company.example".to_owned()],
            &everyone,
        );
        assert_eq!(
            got.addresses,
            ["ann@company.example", "press@partner.example"]
        );
        assert_eq!(got.left_out, 2, "the disabled user and the unlisted domain");
        // With no domain listed, no outside address is addressed at all.
        let none = audience_of(&rows, &[], &everyone);
        assert_eq!(none.addresses, ["ann@company.example"]);
        // RPT-1 review fix (SHOULD-FIX) R8: an active user who may no longer
        // receive it (not in the allowed set) is left out too.
        let nobody = std::collections::HashSet::new();
        let none = audience_of(&rows, &["partner.example".to_owned()], &nobody);
        assert_eq!(none.addresses, ["press@partner.example"]);
        assert_eq!(none.outside, ["press@partner.example"]);
        assert_eq!(none.left_out, 4);
    }

    #[test]
    fn the_attachment_spec_is_checked_against_the_closed_lists() {
        let pdf = |paper: &str| {
            Some(PdfAttachment {
                paper: paper.to_owned(),
                orientation: "portrait".to_owned(),
            })
        };
        let chart = |format: &str, values: &str| ChartAttachment {
            chart: "c1".to_owned(),
            format: format.to_owned(),
            values: values.to_owned(),
        };
        let spec = |pdf, charts| AttachmentSpec { pdf, charts };
        assert_eq!(spec(pdf("a4"), vec![]).validate(), Ok(()));
        assert!(spec(None, vec![]).validate().is_err(), "nothing attached");
        assert!(spec(pdf("a3"), vec![]).validate().is_err());
        assert_eq!(spec(None, vec![chart("xlsx", "raw")]).validate(), Ok(()));
        assert!(spec(None, vec![chart("pdf", "raw")]).validate().is_err());
        assert!(spec(None, vec![chart("csv", "weird")]).validate().is_err());
        let ten: Vec<_> = (0..MAX_CHART_FILES).map(|_| chart("csv", "raw")).collect();
        assert_eq!(
            spec(None, ten.clone()).validate(),
            Ok(()),
            "ten is the limit"
        );
        let mut eleven = ten;
        eleven.push(chart("csv", "raw"));
        assert!(spec(None, eleven).validate().is_err());
    }

    #[test]
    fn the_stored_attachment_spec_refuses_unknown_keys_and_defaults_to_formatted_values() {
        assert!(
            serde_json::from_value::<AttachmentSpec>(json!({ "pdf": null, "zip": true })).is_err()
        );
        let parsed: AttachmentSpec =
            serde_json::from_value(json!({ "charts": [{ "chart": "c1", "format": "csv" }] }))
                .unwrap();
        assert_eq!(
            parsed.charts[0].values, "formatted",
            "formatted is the default"
        );
        assert_eq!(parsed.pdf, None);
    }

    // RPT-1 review fix (SHOULD-FIX) R3.
    #[test]
    fn a_long_reason_is_cut_to_fit_the_record_on_a_character_boundary() {
        assert_eq!(cap_reason("short"), "short");
        let exact = "x".repeat(MAX_REASON_CHARS);
        assert_eq!(cap_reason(&exact), exact);
        // Multi-byte characters: counted as characters, never split.
        let long = "é".repeat(MAX_REASON_CHARS + 50);
        let cut = cap_reason(&long);
        assert_eq!(cut.chars().count(), MAX_REASON_CHARS);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn the_budget_takes_files_until_the_next_one_would_pass_twenty_megabytes() {
        let mut budget = Budget::default();
        assert!(budget.take(MAX_ATTACHMENT_BYTES - 10));
        assert!(!budget.take(11), "one byte over is left out");
        assert!(budget.take(10), "a smaller file still fits after a refusal");
        assert!(!budget.take(1));
    }

    #[test]
    fn a_second_file_with_the_same_name_gets_a_number() {
        let file = |n: &str| Attachment {
            file_name: n.to_owned(),
            content_type: "text/csv".to_owned(),
            bytes: vec![],
        };
        assert_eq!(unique_name(&[], "Revenue.xlsx"), "Revenue.xlsx");
        let taken = vec![file("Revenue.xlsx"), file("Revenue-2.xlsx")];
        assert_eq!(unique_name(&taken, "Revenue.xlsx"), "Revenue-3.xlsx");
        assert_eq!(unique_name(&taken, "Other.csv"), "Other.csv");
    }

    #[test]
    fn a_dashboard_is_empty_only_when_every_data_chart_has_no_rows() {
        let payload = |results: Value| {
            json!({
                "charts": [
                    { "id": "c1", "kind": "bar" },
                    { "id": "t1", "kind": "text" },
                    { "id": "c2", "kind": "table" },
                ],
                "results": results,
            })
        };
        let empty = || json!({ "columns": [], "rows": [] });
        assert!(every_data_chart_is_empty(&payload(
            json!({ "c1": empty(), "t1": empty(), "c2": empty() })
        )));
        assert!(!every_data_chart_is_empty(&payload(
            json!({ "c1": empty(), "c2": { "columns": ["a"], "rows": [{ "a": 1 }] } })
        )));
        // A tile that failed is not proof of emptiness.
        assert!(!every_data_chart_is_empty(&payload(
            json!({ "c1": empty(), "c2": { "error": "x" } })
        )));
        // A missing result neither.
        assert!(!every_data_chart_is_empty(&payload(
            json!({ "c1": empty() })
        )));
        // No data chart at all: a text-only page is not empty.
        assert!(!every_data_chart_is_empty(&json!({
            "charts": [{ "id": "t1", "kind": "text" }], "results": {}
        })));
        assert!(!every_data_chart_is_empty(&json!({})));
    }

    #[test]
    fn the_body_escapes_what_people_typed_and_says_whose_data_it_is() {
        let filters = vec![FilterDef {
            column: "region".to_owned(),
            values: vec!["Bali".to_owned(), "<b>x</b>".to_owned()],
            ..serde_json::from_value::<FilterDef>(json!({ "column": "region" })).unwrap()
        }];
        let html = report_html(&MailParts {
            board_name: "Sales <script>",
            owner_name: "Ann \"A\" & Co",
            filters,
            link: Some("https://c.example/dashboards/x?a=1&b=2".to_owned()),
            sent: vec!["Sales.pdf".to_owned()],
            left_out: &["\"Visits\" was left out: with it the email would pass 20 MB.".to_owned()],
            recipients_left_out: 1,
        });
        assert!(!html.contains("<script>"));
        assert!(html.contains("Sales &lt;script&gt;"));
        assert!(html.contains("Ann &quot;A&quot; &amp; Co"));
        assert!(html.contains("region: Bali, &lt;b&gt;x&lt;/b&gt;"));
        assert!(html.contains("is allowed to see. Everyone it is sent to sees the same data."));
        assert!(html.contains("Sales.pdf"));
        assert!(html.contains("was left out: with it the email would pass 20 MB."));
        assert!(html.contains("1 recipient was left out"));
        assert!(html.contains("href=\"https://c.example/dashboards/x?a=1&amp;b=2\""));
    }

    #[test]
    fn a_body_without_a_link_or_filters_says_so_and_has_no_anchor() {
        let html = report_html(&MailParts {
            board_name: "Main",
            owner_name: "Ann",
            filters: vec![],
            link: None,
            sent: vec![],
            left_out: &[],
            recipients_left_out: 0,
        });
        assert!(html.contains("Filters: none."));
        assert!(!html.contains("<a "));
        assert!(!html.contains("Left out"));
    }

    #[test]
    fn a_mail_failure_becomes_one_of_our_sentences_and_unknown_text_is_dropped() {
        use lakehouse_notify::DeliveryFailure as F;
        assert_eq!(
            email_failure_reason(Some(F::MailServerUnreachable.message())),
            "The email was not sent: the mail server could not be reached."
        );
        assert_eq!(
            email_failure_reason(Some(F::MailServerRefused.message())),
            "The email was not sent: the mail server refused the message."
        );
        assert_eq!(
            email_failure_reason(Some(F::EmailNotConfigured.message())),
            EMAIL_NOT_SET_UP
        );
        assert_eq!(
            email_failure_reason(Some("550 5.1.1 user@internal.example unknown")),
            "The email was not sent."
        );
        assert_eq!(email_failure_reason(None), "The email was not sent.");
    }

    #[test]
    fn the_pdf_reasons_are_the_fixed_sentences() {
        assert_eq!(pdf_reason(&PdfFailure::NotSetUp), PDF_NOT_SET_UP);
        assert_eq!(pdf_reason(&PdfFailure::Busy), PDF_BUSY);
        assert_eq!(
            pdf_reason(&PdfFailure::Conversion(crate::pdf::PdfError::Timeout)),
            PDF_TIMEOUT
        );
        assert_eq!(
            pdf_reason(&PdfFailure::Conversion(crate::pdf::PdfError::NotAPdf)),
            PDF_FAILED
        );
    }
}
