//! Report subscriptions, their recipients and the record of each send
//! (`RPT-1`, migration `0088`, plan decision D1; feature page
//! `docs/core/features/scheduled-reports.md`).
//!
//! This module stores and claims; it decides nothing about who may subscribe,
//! which outside domains are allowed or what a report contains. The routes and
//! the runner (`lakehouse-api`) own those rules, so they cannot differ between
//! "Send now", the schedule and the assistant.
//!
//! # Claiming without sending twice (feature decision 8)
//!
//! [`claim_due`] reads the due rows with `FOR UPDATE SKIP LOCKED`, moves each
//! one's `next_run_at` forward and commits, in one transaction. Two API
//! processes running it at once get disjoint rows: the second skips what the
//! first has locked, and after the commit the rows are no longer due. The next
//! time is computed from the claim time by the caller's closure (see
//! [`crate::report_schedule`]), so a send that was missed while the API was down
//! is claimed once, not once per missed time. A process that dies between the
//! claim and the send loses that one send; it is never sent twice. The runner
//! records nothing for a send it did not make.
//!
//! # What is never stored
//!
//! A delivery's `reason` is one of the product's own sentences. The runner maps
//! a mail-server or PDF-service failure to a fixed sentence before it reaches
//! [`record_delivery`] (SEC-11); this module cannot tell and does not need to.

use serde_json::Value;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::StoreError;
use crate::report_schedule::{Schedule, ScheduleError};

/// Failures in a row after which a subscription pauses itself (feature
/// decision 7).
pub const PAUSE_AFTER_FAILURES: i32 = 5;

/// The paused reason of a row whose schedule could not be worked out (a zone
/// removed from the database, a row edited by hand).
pub const UNSCHEDULABLE_REASON: &str = "The schedule could not be worked out.";

/// A stored subscription.
#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct Subscription {
    /// `report_subscription.id`.
    pub id: Uuid,
    /// The dashboard's id.
    pub board: String,
    /// The person the report is made as.
    pub owner_id: Uuid,
    /// `hourly`, `daily`, `weekly` or `monthly`.
    pub kind: String,
    /// Minute of the hour.
    pub minute: i16,
    /// Hour of the day; `None` for hourly.
    pub hour: Option<i16>,
    /// ISO weekday for weekly.
    pub weekday: Option<i16>,
    /// Day of the month for monthly; `-1` is the last day.
    pub month_day: Option<i16>,
    /// The IANA zone the schedule is read in.
    pub time_zone: String,
    /// What is attached: the PDF and the chosen charts (JSON, the runner's
    /// shape).
    pub attachments: Value,
    /// The dashboard filters (a JSON list), as `GET /api/dashboard` takes them.
    pub filters: Option<Value>,
    /// The parameter values (a JSON object).
    pub params: Option<Value>,
    /// The grain switch.
    pub grain: Option<String>,
    /// Send nothing when every data chart has no rows.
    pub skip_when_empty: bool,
    /// Whether the subscription is paused.
    pub paused: bool,
    /// Why, when paused.
    pub paused_reason: Option<String>,
    /// Failed sends in a row.
    pub consecutive_failures: i32,
    /// When the runner next takes it.
    pub next_run_at: OffsetDateTime,
    /// Creation time.
    pub created_at: OffsetDateTime,
    /// Last change.
    pub updated_at: OffsetDateTime,
}

impl Subscription {
    /// The schedule the columns hold.
    ///
    /// # Errors
    ///
    /// [`ScheduleError::Invalid`] for a row whose columns do not form a
    /// schedule (the table's `CHECK`s make that a hand edit).
    pub fn schedule(&self) -> Result<Schedule, ScheduleError> {
        Schedule::from_parts(
            &self.kind,
            self.minute,
            self.hour,
            self.weekday,
            self.month_day,
        )
    }
}

const SUBSCRIPTION_COLUMNS: &str = "id, board, owner_id, kind, minute, hour, weekday, month_day, \
     time_zone, attachments, filters, params, grain, skip_when_empty, paused, paused_reason, \
     consecutive_failures, next_run_at, created_at, updated_at";

/// One recipient: a console user or an outside address, never both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recipient {
    /// A console user by id.
    User(Uuid),
    /// An outside address (stored lower-cased).
    Email(String),
}

/// A recipient as stored, with the console user's current name, address and
/// status when it is one.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct RecipientRow {
    /// The console user, for a user recipient.
    pub user_id: Option<Uuid>,
    /// The outside address, for an address recipient.
    pub email: Option<String>,
    /// The user's name now.
    pub user_name: Option<String>,
    /// The user's address now.
    pub user_email: Option<String>,
    /// The user's status now (`active` or `inactive`).
    pub user_status: Option<String>,
}

/// What a create or an update writes.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSubscription {
    /// The dashboard's id.
    pub board: String,
    /// The person the report is made as (set on create, never changed).
    pub owner_id: Uuid,
    /// When it runs.
    pub schedule: Schedule,
    /// The zone the schedule is read in.
    pub time_zone: String,
    /// What is attached (JSON).
    pub attachments: Value,
    /// Dashboard filters (JSON list).
    pub filters: Option<Value>,
    /// Parameter values (JSON object).
    pub params: Option<Value>,
    /// The grain switch.
    pub grain: Option<String>,
    /// Send nothing when empty.
    pub skip_when_empty: bool,
    /// The first (or next) due time, worked out by the caller.
    pub next_run_at: OffsetDateTime,
    /// Who receives it.
    pub recipients: Vec<Recipient>,
}

async fn replace_recipients(
    tx: &mut Transaction<'_, Postgres>,
    subscription: Uuid,
    recipients: &[Recipient],
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM report_recipient WHERE subscription_id = $1")
        .bind(subscription)
        .execute(&mut **tx)
        .await?;
    for recipient in recipients {
        let (user_id, email) = match recipient {
            Recipient::User(id) => (Some(*id), None),
            Recipient::Email(address) => (None, Some(address.trim().to_ascii_lowercase())),
        };
        sqlx::query(
            "INSERT INTO report_recipient (subscription_id, user_id, email) VALUES ($1, $2, $3)",
        )
        .bind(subscription)
        .bind(user_id)
        .bind(email)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Create a subscription and its recipients.
///
/// # Errors
///
/// [`StoreError::ForeignKeyViolation`] for an owner or a user recipient that
/// does not exist, [`StoreError::Conflict`] for the same recipient twice,
/// [`StoreError::Database`] otherwise (including a `CHECK` refusing a value).
pub async fn create(pool: &PgPool, new: &NewSubscription) -> Result<Subscription, StoreError> {
    let parts = new.schedule.to_parts();
    let mut tx = pool.begin().await?;
    let sql = format!(
        "INSERT INTO report_subscription (board, owner_id, kind, minute, hour, weekday, \
         month_day, time_zone, attachments, filters, params, grain, skip_when_empty, next_run_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
         RETURNING {SUBSCRIPTION_COLUMNS}"
    );
    let row: Subscription = sqlx::query_as(&sql)
        .bind(&new.board)
        .bind(new.owner_id)
        .bind(parts.kind)
        .bind(parts.minute)
        .bind(parts.hour)
        .bind(parts.weekday)
        .bind(parts.month_day)
        .bind(&new.time_zone)
        .bind(&new.attachments)
        .bind(&new.filters)
        .bind(&new.params)
        .bind(&new.grain)
        .bind(new.skip_when_empty)
        .bind(new.next_run_at)
        .fetch_one(&mut *tx)
        .await?;
    replace_recipients(&mut tx, row.id, &new.recipients).await?;
    tx.commit().await?;
    Ok(row)
}

/// Replace a subscription's schedule, attachments, values and recipients. The
/// owner and the board stay as they are (a report for another dashboard is a
/// new subscription). `None` when it does not exist.
///
/// # Errors
///
/// As [`create`].
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    new: &NewSubscription,
) -> Result<Option<Subscription>, StoreError> {
    let parts = new.schedule.to_parts();
    let mut tx = pool.begin().await?;
    let sql = format!(
        "UPDATE report_subscription SET kind = $2, minute = $3, hour = $4, weekday = $5, \
         month_day = $6, time_zone = $7, attachments = $8, filters = $9, params = $10, \
         grain = $11, skip_when_empty = $12, next_run_at = $13, updated_at = now() \
         WHERE id = $1 RETURNING {SUBSCRIPTION_COLUMNS}"
    );
    let row: Option<Subscription> = sqlx::query_as(&sql)
        .bind(id)
        .bind(parts.kind)
        .bind(parts.minute)
        .bind(parts.hour)
        .bind(parts.weekday)
        .bind(parts.month_day)
        .bind(&new.time_zone)
        .bind(&new.attachments)
        .bind(&new.filters)
        .bind(&new.params)
        .bind(&new.grain)
        .bind(new.skip_when_empty)
        .bind(new.next_run_at)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    replace_recipients(&mut tx, id, &new.recipients).await?;
    tx.commit().await?;
    Ok(Some(row))
}

/// One subscription by id.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Subscription>, StoreError> {
    let sql = format!("SELECT {SUBSCRIPTION_COLUMNS} FROM report_subscription WHERE id = $1");
    Ok(sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?)
}

/// The subscriptions of one dashboard, oldest first.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list_for_board(pool: &PgPool, board: &str) -> Result<Vec<Subscription>, StoreError> {
    let sql = format!(
        "SELECT {SUBSCRIPTION_COLUMNS} FROM report_subscription WHERE board = $1 \
         ORDER BY created_at, id"
    );
    Ok(sqlx::query_as(&sql).bind(board).fetch_all(pool).await?)
}

/// The subscriptions a person owns, oldest first (the assistant's list).
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list_for_owner(pool: &PgPool, owner: Uuid) -> Result<Vec<Subscription>, StoreError> {
    let sql = format!(
        "SELECT {SUBSCRIPTION_COLUMNS} FROM report_subscription WHERE owner_id = $1 \
         ORDER BY created_at, id"
    );
    Ok(sqlx::query_as(&sql).bind(owner).fetch_all(pool).await?)
}

/// A subscription's recipients, user recipients with their current name,
/// address and status. Console users first, then outside addresses, each by
/// address, so the order is the same every time.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn recipients(pool: &PgPool, id: Uuid) -> Result<Vec<RecipientRow>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT r.user_id, r.email, u.name AS user_name, u.email AS user_email, \
                u.status AS user_status \
         FROM report_recipient r LEFT JOIN app_user u ON u.id = r.user_id \
         WHERE r.subscription_id = $1 \
         ORDER BY r.user_id IS NULL, COALESCE(u.email, r.email)",
    )
    .bind(id)
    .fetch_all(pool)
    .await?)
}

/// Remove a subscription (its recipients and deliveries go with it). Whether it
/// existed.
///
/// # Errors
///
/// [`StoreError::Database`] if the delete fails.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM report_subscription WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Pause a subscription with the reason, or resume it. A resume clears the
/// reason and the failure count and sets the next due time (the caller works it
/// out from now: a row paused for a week must not fire for the week it missed).
/// `None` when it does not exist.
///
/// # Errors
///
/// [`StoreError::Database`] if the update fails.
pub async fn set_paused(
    pool: &PgPool,
    id: Uuid,
    paused: bool,
    reason: Option<&str>,
    resume_at: Option<OffsetDateTime>,
) -> Result<Option<Subscription>, StoreError> {
    let sql = if paused {
        format!(
            "UPDATE report_subscription SET paused = TRUE, paused_reason = $2, \
             updated_at = now() WHERE id = $1 RETURNING {SUBSCRIPTION_COLUMNS}"
        )
    } else {
        format!(
            "UPDATE report_subscription SET paused = FALSE, paused_reason = NULL, \
             consecutive_failures = 0, next_run_at = COALESCE($3, next_run_at), \
             updated_at = now() WHERE id = $1 RETURNING {SUBSCRIPTION_COLUMNS}"
        )
    };
    Ok(sqlx::query_as(&sql)
        .bind(id)
        .bind(reason)
        .bind(resume_at)
        .fetch_optional(pool)
        .await?)
}

/// Take the subscriptions that are due at `now`, at most `limit`, oldest first,
/// and move each one's next due time forward in the same transaction (see the
/// module doc). `next_after` works the new time out from the claim; `None`
/// means the schedule cannot be worked out, and that row is paused with
/// [`UNSCHEDULABLE_REASON`] and not returned.
///
/// The returned rows still hold the time they were due.
///
/// # Errors
///
/// [`StoreError::Database`] if the transaction fails; nothing is claimed then.
pub async fn claim_due<F>(
    pool: &PgPool,
    now: OffsetDateTime,
    limit: i64,
    next_after: F,
) -> Result<Vec<Subscription>, StoreError>
where
    F: Fn(&Subscription) -> Option<OffsetDateTime>,
{
    let mut tx = pool.begin().await?;
    let sql = format!(
        "SELECT {SUBSCRIPTION_COLUMNS} FROM report_subscription \
         WHERE NOT paused AND next_run_at <= $1 ORDER BY next_run_at, id LIMIT $2 \
         FOR UPDATE SKIP LOCKED"
    );
    let due: Vec<Subscription> = sqlx::query_as(&sql)
        .bind(now)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
    let mut claimed = Vec::with_capacity(due.len());
    for row in due {
        if let Some(next) = next_after(&row) {
            sqlx::query("UPDATE report_subscription SET next_run_at = $2 WHERE id = $1")
                .bind(row.id)
                .bind(next)
                .execute(&mut *tx)
                .await?;
            claimed.push(row);
        } else {
            sqlx::query(
                "UPDATE report_subscription SET paused = TRUE, paused_reason = $2 WHERE id = $1",
            )
            .bind(row.id)
            .bind(UNSCHEDULABLE_REASON)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(claimed)
}

/// How a send ended, as the table stores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    /// The email went out.
    Sent,
    /// It did not, and the reason says why.
    Failed,
    /// Nothing was sent on purpose (skip when empty).
    Skipped,
}

impl DeliveryStatus {
    /// The stored word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

/// What a send did, to be recorded.
#[derive(Debug, Clone, PartialEq)]
pub struct NewDelivery {
    /// The subscription.
    pub subscription_id: Uuid,
    /// When the send began.
    pub started_at: OffsetDateTime,
    /// When it ended.
    pub finished_at: OffsetDateTime,
    /// How it ended.
    pub status: DeliveryStatus,
    /// `true` when "Send now" made it, `false` when the schedule did.
    pub manual: bool,
    /// How many people the email was addressed to.
    pub recipient_count: i32,
    /// A plain sentence of the product's own; never upstream text.
    pub reason: Option<String>,
}

/// A recorded send.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct Delivery {
    /// `report_delivery.id`.
    pub id: Uuid,
    /// The subscription.
    pub subscription_id: Uuid,
    /// When the send began.
    pub started_at: OffsetDateTime,
    /// When it ended.
    pub finished_at: OffsetDateTime,
    /// `sent`, `failed` or `skipped`.
    pub status: String,
    /// `schedule` or `manual`.
    pub origin: String,
    /// People addressed.
    pub recipient_count: i32,
    /// The plain reason.
    pub reason: Option<String>,
}

const DELIVERY_COLUMNS: &str = "id, subscription_id, started_at, finished_at, status, origin, \
     recipient_count, reason";

/// Record a send.
///
/// # Errors
///
/// [`StoreError::ForeignKeyViolation`] when the subscription was deleted while
/// it ran, [`StoreError::Database`] otherwise.
pub async fn record_delivery(pool: &PgPool, new: &NewDelivery) -> Result<Delivery, StoreError> {
    let sql = format!(
        "INSERT INTO report_delivery (subscription_id, started_at, finished_at, status, origin, \
         recipient_count, reason) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING {DELIVERY_COLUMNS}"
    );
    Ok(sqlx::query_as(&sql)
        .bind(new.subscription_id)
        .bind(new.started_at)
        .bind(new.finished_at)
        .bind(new.status.as_str())
        .bind(if new.manual { "manual" } else { "schedule" })
        .bind(new.recipient_count)
        .bind(&new.reason)
        .fetch_one(pool)
        .await?)
}

/// A subscription's recorded sends, newest first.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list_deliveries(
    pool: &PgPool,
    subscription: Uuid,
    limit: i64,
) -> Result<Vec<Delivery>, StoreError> {
    let sql = format!(
        "SELECT {DELIVERY_COLUMNS} FROM report_delivery WHERE subscription_id = $1 \
         ORDER BY started_at DESC, id LIMIT $2"
    );
    Ok(sqlx::query_as(&sql)
        .bind(subscription)
        .bind(limit)
        .fetch_all(pool)
        .await?)
}

/// The newest recorded send of each of the given subscriptions (those with
/// none are absent), for the list's "last delivery" column.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn last_deliveries(
    pool: &PgPool,
    subscriptions: &[Uuid],
) -> Result<Vec<Delivery>, StoreError> {
    let sql = format!(
        "SELECT DISTINCT ON (subscription_id) {DELIVERY_COLUMNS} FROM report_delivery \
         WHERE subscription_id = ANY($1) ORDER BY subscription_id, started_at DESC, id"
    );
    Ok(sqlx::query_as(&sql)
        .bind(subscriptions)
        .fetch_all(pool)
        .await?)
}

/// What a send's end did to the subscription's failure count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureState {
    /// Failures in a row now.
    pub consecutive_failures: i32,
    /// Whether this outcome paused the subscription.
    pub paused_now: bool,
}

/// Note how a send ended. A success resets the count. A failure adds one, and at
/// [`PAUSE_AFTER_FAILURES`] the subscription is paused with `pause_reason`
/// (feature decision 7). A skip changes nothing. `None` when the subscription
/// no longer exists.
///
/// # Errors
///
/// [`StoreError::Database`] if the update fails.
pub async fn note_outcome(
    pool: &PgPool,
    id: Uuid,
    status: DeliveryStatus,
    pause_reason: &str,
) -> Result<Option<FailureState>, StoreError> {
    let row: Option<(i32, bool)> = match status {
        DeliveryStatus::Sent => {
            sqlx::query_as(
                "UPDATE report_subscription SET consecutive_failures = 0 WHERE id = $1 \
                 RETURNING consecutive_failures, FALSE",
            )
            .bind(id)
            .fetch_optional(pool)
            .await?
        }
        DeliveryStatus::Skipped => {
            sqlx::query_as(
                "SELECT consecutive_failures, FALSE FROM report_subscription WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(pool)
            .await?
        }
        DeliveryStatus::Failed => {
            // `paused_now` is true only for the update that crosses the line
            // (the row was not paused before), so the owner is told once. The
            // row is locked for the read, so two failures at once count twice.
            let mut tx = pool.begin().await?;
            let was: Option<(bool,)> =
                sqlx::query_as("SELECT paused FROM report_subscription WHERE id = $1 FOR UPDATE")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let Some((was_paused,)) = was else {
                return Ok(None);
            };
            let (failures, paused): (i32, bool) = sqlx::query_as(
                "UPDATE report_subscription SET consecutive_failures = consecutive_failures + 1, \
                 paused = paused OR consecutive_failures + 1 >= $2, \
                 paused_reason = CASE WHEN NOT paused AND consecutive_failures + 1 >= $2 \
                                      THEN $3 ELSE paused_reason END \
                 WHERE id = $1 RETURNING consecutive_failures, paused",
            )
            .bind(id)
            .bind(PAUSE_AFTER_FAILURES)
            .bind(pause_reason)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            Some((failures, paused && !was_paused))
        }
    };
    Ok(row.map(|(consecutive_failures, paused_now)| FailureState {
        consecutive_failures,
        paused_now,
    }))
}

/// Remove deliveries that started before `cutoff` (the runner calls it once a
/// day with 90 days, feature decision 7). How many went.
///
/// # Errors
///
/// [`StoreError::Database`] if the delete fails.
pub async fn prune_deliveries(pool: &PgPool, cutoff: OffsetDateTime) -> Result<u64, StoreError> {
    let done = sqlx::query("DELETE FROM report_delivery WHERE started_at < $1")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(done.rows_affected())
}
