//! When a table's quality rules run (`DATA-14`, F3).
//!
//! No schedule means every hour, which is what every table did before.
//! `PUT /api/governance/quality/schedules/{asset}` (`governance:write`) sets
//! one table to run its rules **on every load** or **every N minutes**, from
//! one hour to one day. `GET /api/governance/quality/schedules` (any login,
//! like the rule list) is what the console reads to show the setting.
//!
//! An `on_load` table also keeps an interval, a day unless one is given:
//! loads that never reach the platform's run-finished event (change
//! capture, a connector run that failed after committing some tables) are
//! then still checked. The tick (`routes::quality`) applies the interval;
//! the load hook (`routes::quality_load`) applies `on_load`.
//!
//! `DELETE /api/governance/quality/schedules/{asset}` (`governance:write`,
//! `WEB-3` S3a-D3) removes a table's row, so the table runs at
//! [`DEFAULT_INTERVAL_MINUTES`] again — the schedule it had before anyone
//! set one.

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_core::ident::Ident;
use lakehouse_store::quality::{
    self, DEFAULT_INTERVAL_MINUTES, MAX_INTERVAL_MINUTES, MIN_INTERVAL_MINUTES, ScheduleMode,
};
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::quality::pool;
use crate::state::AppState;

/// The zones a quality rule can name a table in (`routes::quality::resolve`).
const ZONES: [&str; 3] = ["bronze", "silver", "serving"];

/// `zone.table` for a zone a rule can check, with a plain table name; the
/// lower-cased key a schedule is stored under.
pub(super) fn table_key(asset: &str) -> Option<String> {
    let (zone, table) = asset.trim().split_once('.')?;
    let zone = zone.to_lowercase();
    (ZONES.contains(&zone.as_str()) && Ident::new(table).is_ok())
        .then(|| format!("{zone}.{}", table.to_lowercase()))
}

/// `GET /api/governance/quality/schedules` — every table that has a
/// schedule. A table not listed runs every 60 minutes.
///
/// # Errors
///
/// `403` for a caller outside the tenant that owns the shared catalog;
/// `503`/`500` on a store or table-access failure.
pub async fn list_schedules(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Value>> {
    // GOV-16: the one GOV-15 gate — catalog tenant first, role table access
    // second — so the list cannot be used to learn which tables a role hides.
    let access =
        crate::routes::quality::quality_table_access(&state, Some(&principal), &headers).await?;
    let schedules = quality::list_schedules(pool(&state)?).await?;
    // `SEC-20` (plan ai-118): a schedule names the table it belongs to, and
    // the schedules of every tenant's upload are in the list, so the ones
    // outside the caller's reach are left out — the reach Query Studio keeps
    // (`crate::routes::query::resolve_reach` and
    // `crate::routes::query::reachable_names`).
    let reach = crate::routes::query::resolve_reach(&state, &principal, &headers).await?;
    let assets: Vec<String> = schedules.iter().map(|s| s.asset.clone()).collect();
    let readable = crate::routes::query::reachable_names(&state, reach, &assets).await?;
    // GOV-16: a schedule names the table it belongs to, so a narrowed caller
    // does not see the schedules of tables their role cannot read.
    let schedules: Vec<_> = schedules
        .into_iter()
        .filter(|s| readable.contains(&s.asset))
        .filter(|s| crate::routes::quality::asset_readable(&access, Some(&s.asset)))
        .collect();
    Ok(ApiJson(json!({
        "schedules": schedules,
        "defaultIntervalMinutes": DEFAULT_INTERVAL_MINUTES,
    })))
}

/// The `PUT …/schedules/{asset}` body.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleBody {
    mode: String,
    #[serde(default)]
    interval_minutes: Option<i32>,
}

/// `PUT /api/governance/quality/schedules/{asset}` — run a table's rules on
/// every load, or every `intervalMinutes` (60 to 1440). `intervalMinutes` is
/// required for `interval`; for `on_load` it is the backstop and defaults to
/// a day. Audited as `quality.schedule_update` on the table.
///
/// # Errors
///
/// `403` when the caller is outside the tenant that owns the shared catalog
/// (that answer is the same for every table); `400` when the table is not
/// `bronze.`/`silver.`/`serving.` plus a plain name, when the caller's table
/// access hides it (the sentence a rule on that table gets, `GOV-15`), or when
/// the table is outside the caller's reach (`SEC-80`, the same sentence), when the
/// mode is not `on_load` or `interval`, the interval is missing for `interval`
/// or outside 60 to 1440, or the body is malformed; `400` with the same
/// sentence when no table has that name (`SEC-80`); `503` with the text
/// "The table could not be checked. Try again." when the presence check cannot
/// run (`SEC-80`), for a claimed name and an unknown one alike; `503`/`500` on a
/// store failure.
pub async fn put_schedule(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Path(asset): Path<String>,
    body: axum::body::Bytes,
) -> ApiResult<ApiJson<Value>> {
    // GOV-15 (GOV-12 plan, section 4): the tenant gate comes first, before the
    // body is read and before anything is looked up (D4).
    let access =
        crate::routes::quality::quality_table_access(&state, Some(&principal), &headers).await?;
    let body: ScheduleBody = serde_json::from_slice(&body)
        // SEC-11 (CI run on DATA-14): a fixed sentence, not serde's text.
        .map_err(|_| {
            ApiError::BadRequest("Body must be JSON {mode, intervalMinutes}.".to_owned())
        })?;
    let key = table_key(&asset).ok_or_else(|| {
        ApiError::BadRequest(
            "Name the table as bronze.<table>, silver.<table> or serving.<table>.".to_owned(),
        )
    })?;
    // GOV-15 (GOV-12 plan, section 4): a schedule names a table, not a rule, so
    // a table the role does not read is refused with the sentence a rule on it
    // gets (D5).
    if !access.allows_key(&key) {
        return Err(crate::routes::quality::unreadable_table(asset.trim()).into());
    }
    // SEC-80: presence and reach are asked for every name before anything is
    // answered, presence first. An outage then answers a name another tenant
    // claimed and a name no table has alike, and a refusal of either is the
    // same 400.
    let present = crate::routes::quality::quality_table_present(&state, &key).await?;
    let in_reach =
        crate::routes::quality::quality_key_in_reach(&state, Some(&principal), &headers, &key)
            .await?;
    if !present || !in_reach {
        return Err(crate::routes::quality::unreadable_table(asset.trim()).into());
    }
    let mode = ScheduleMode::parse(&body.mode)
        .ok_or_else(|| ApiError::BadRequest("mode must be on_load or interval.".to_owned()))?;
    let interval = match (mode, body.interval_minutes) {
        (ScheduleMode::Interval, None) => {
            return Err(ApiError::BadRequest(format!(
                "intervalMinutes is required, from {MIN_INTERVAL_MINUTES} to {MAX_INTERVAL_MINUTES}."
            ))
            .into());
        }
        (ScheduleMode::Interval, Some(minutes)) => minutes,
        (ScheduleMode::OnLoad, minutes) => minutes.unwrap_or(MAX_INTERVAL_MINUTES),
    };
    let schedule =
        quality::put_schedule(pool(&state)?, &key, mode, interval, principal.id.uuid()).await?;
    crate::routes::governance::audit_table_change(
        &state,
        Some(&principal),
        &schedule.asset,
        "quality.schedule_update",
        json!({ "mode": schedule.mode, "intervalMinutes": schedule.interval_minutes }),
    )
    .await;
    Ok(ApiJson(json!(schedule)))
}

/// `DELETE /api/governance/quality/schedules/{asset}` — put a table back on
/// the default schedule, every [`DEFAULT_INTERVAL_MINUTES`] (60). Returns
/// 204; `GET …/schedules` then lists no row for the table, which is how the
/// console reads "the default". Audited as `quality.schedule_reset` on the
/// table, like [`put_schedule`] is on the setting it changes.
///
/// # Errors
///
/// `403` when the caller is outside the tenant that owns the shared catalog;
/// `400` when the table is not `bronze.`/`silver.`/`serving.` plus a plain
/// name, or when the caller's table access hides it (the sentence a rule on
/// that table gets, `GOV-15`), or when the table is outside the caller's reach
/// (`SEC-80`), or when no table has that name and the schedule it would reset
/// does not exist (`SEC-80`: a reset of a dropped table still works); an existing
/// schedule row in the caller's reach gives 204 even when the table is gone;
/// `503` with the text "The table could not be checked. Try again." when the
/// presence check cannot run (`SEC-80`), for a claimed name and an unknown one
/// alike; `503`/`500` on a store failure.
pub async fn delete_schedule(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Path(asset): Path<String>,
) -> ApiResult<StatusCode> {
    // GOV-15 (GOV-12 plan, section 4): the tenant gate comes first, before
    // anything is looked up (D4).
    let access =
        crate::routes::quality::quality_table_access(&state, Some(&principal), &headers).await?;
    let key = table_key(&asset).ok_or_else(|| {
        ApiError::BadRequest(
            "Name the table as bronze.<table>, silver.<table> or serving.<table>.".to_owned(),
        )
    })?;
    // GOV-15 (GOV-12 plan, section 4): refusing first, so a reset changes
    // nothing for a table the role does not read (D8).
    if !access.allows_key(&key) {
        return Err(crate::routes::quality::unreadable_table(asset.trim()).into());
    }
    // SEC-80: presence and reach are asked for every name before anything is
    // answered, presence first, so an outage answers a claimed name and an
    // unknown name alike.
    let present = crate::routes::quality::quality_table_present(&state, &key).await?;
    let in_reach =
        crate::routes::quality::quality_key_in_reach(&state, Some(&principal), &headers, &key)
            .await?;
    // A reset of an existing in-reach schedule answers 204 when its table was dropped.
    // An outage that stops the presence check answers 503 for it too (SEC-80).
    let scheduled = quality::list_schedules(pool(&state)?)
        .await?
        .iter()
        .any(|schedule| schedule.asset == key);
    if !in_reach || !(present || scheduled) {
        return Err(crate::routes::quality::unreadable_table(asset.trim()).into());
    }
    quality::delete_schedule(pool(&state)?, &key).await?;
    crate::routes::governance::audit_table_change(
        &state,
        Some(&principal),
        &key,
        "quality.schedule_reset",
        json!({ "intervalMinutes": DEFAULT_INTERVAL_MINUTES }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_key_is_a_readable_zone_and_a_plain_name() {
        assert_eq!(table_key("Silver.Orders"), Some("silver.orders".to_owned()));
        assert_eq!(
            table_key(" serving.mart_x "),
            Some("serving.mart_x".to_owned())
        );
        assert_eq!(table_key("bronze.raw"), Some("bronze.raw".to_owned()));
        for bad in [
            "orders",
            "gold.orders",
            "silver.",
            "silver.a b",
            "silver.a;b",
            "",
            "silver.`x`",
        ] {
            assert_eq!(table_key(bad), None, "{bad}");
        }
    }
}
