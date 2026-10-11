//! `GET`/`PUT /api/settings/reporting` — the deployment's report time zone
//! and first day of the week (`BI-9`).
//!
//! # Why one deployment-wide setting
//!
//! Grouping a timestamp by day or month, and the relative date filters
//! ("this month", "last 7 days"), both need to know which zone's midnight
//! counts. One zone for everybody keeps a chart and a filter from disagreeing
//! (feature page `docs/core/features/time-grain.md`, "Limits to tell a
//! customer"). Stored in Postgres (`reporting_settings`, migration `0066`);
//! with no row saved the defaults apply (`Asia/Jakarta`, Monday).
//!
//! # Who may do what
//!
//! Reading needs only a sign-in: the console reads the zone and the first day
//! to label axes. Changing needs `settings:write`, a new permission string
//! that follows the rules at the top of `policy.rs`: no seed grant, only a
//! `*:*` holder (Platform Admin) has it until an operator grants it to a role.
//!
//! # The zone is checked against the engine
//!
//! A name is saved only if it has the shape of a zone name
//! ([`lakehouse_bi::grain::TimeZoneName`]) and `system.time_zones` lists it,
//! so a typo cannot reach every dashboard's SQL. That table is readable by a
//! user with `SELECT` on one database only (measured on `ClickHouse` 26.8), so
//! the check needs no privilege the API's user lacks. A failure of the engine
//! or the database goes through `upstream_error`.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use lakehouse_auth::Principal;
use lakehouse_bi::grain::{TimeContext, WeekStart};
use lakehouse_core::ApiError;
use lakehouse_core::ident::SqlLiteral;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::upstream_error;

/// What the person is told when the body is not the two fields.
const BODY_SHAPE: &str =
    "body must be {\"timeZone\": string, \"weekStart\": \"monday\"|\"sunday\"}";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsBody {
    time_zone: String,
    week_start: String,
}

/// The time context every dashboard read uses, loaded once per request.
///
/// No Postgres pool or no saved row means the defaults, not an error: a
/// deployment that never opened Settings behaves as it was told it would.
///
/// # Errors
///
/// A database failure is reported through `upstream_error` (no fabricated
/// default, which would shift every date silently); a stored row that no
/// longer validates (edited by hand around the CHECKs) is a fixed `500`.
pub(crate) async fn time_context(state: &AppState) -> Result<TimeContext, ApiError> {
    let Some(pool) = state.pg.as_deref() else {
        return Ok(TimeContext::default());
    };
    let saved = lakehouse_store::reporting_settings::get(pool)
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    let Some(saved) = saved else {
        return Ok(TimeContext::default());
    };
    let week_start = WeekStart::parse(&saved.week_start);
    week_start
        .and_then(|w| TimeContext::new(&saved.time_zone, w).ok())
        .ok_or_else(|| ApiError::Internal("The reporting settings are invalid.".to_owned()))
}

fn body_json(ctx: &TimeContext, saved: bool) -> Value {
    json!({
        "timeZone": ctx.zone(),
        "weekStart": ctx.week_start().as_str(),
        "saved": saved,
    })
}

/// `GET /api/settings/reporting` — the zone and first day in use, and whether
/// they were ever saved (`saved: false` means these are the defaults).
///
/// # Errors
///
/// 5xx from [`time_context`] when the settings cannot be read.
pub async fn get_reporting(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let ctx = time_context(&state).await?;
    let saved = match state.pg.as_deref() {
        Some(pool) => lakehouse_store::reporting_settings::get(pool)
            .await
            .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?
            .is_some(),
        None => false,
    };
    Ok(ApiJson(body_json(&ctx, saved)))
}

/// `PUT /api/settings/reporting` — save `{timeZone, weekStart}`.
///
/// # Errors
///
/// 400 with a fixed message when the body is not those two fields, the first
/// day is not `monday` or `sunday`, or the engine does not know the zone;
/// 503 without a Postgres pool; 5xx through `upstream_error` when the engine
/// or the database fails.
pub async fn put_reporting(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    // Parsed by hand so a malformed body gets this module's message.
    let parsed: SettingsBody =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest(BODY_SHAPE.to_owned()))?;
    let week_start = WeekStart::parse(&parsed.week_start)
        .ok_or_else(|| ApiError::BadRequest("weekStart must be monday or sunday.".to_owned()))?;
    let ctx = TimeContext::new(&parsed.time_zone, week_start)
        .map_err(|_| ApiError::BadRequest("Unknown time zone.".to_owned()))?;
    let Some(pool) = state.pg.as_deref() else {
        return Err(ApiError::Unavailable(
            "Settings need a database, and none is configured.".to_owned(),
        )
        .into());
    };
    if !engine_knows_zone(&state, ctx.zone()).await? {
        return Err(ApiError::BadRequest("Unknown time zone.".to_owned()).into());
    }
    let stored = lakehouse_store::reporting_settings::upsert(
        pool,
        ctx.zone(),
        week_start.as_str(),
        &principal.id.uuid().to_string(),
    )
    .await
    .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    // Echo what was stored, through the same constructor reads use.
    let echoed = WeekStart::parse(&stored.week_start)
        .and_then(|w| TimeContext::new(&stored.time_zone, w).ok())
        .unwrap_or(ctx);
    Ok(ApiJson(body_json(&echoed, true)))
}

/// Whether `system.time_zones` lists `zone`. `zone` already has the shape of
/// a zone name and is printed as a literal.
async fn engine_knows_zone(state: &AppState, zone: &str) -> Result<bool, ApiError> {
    let sql = format!(
        "SELECT count() AS n FROM system.time_zones WHERE time_zone = {}",
        SqlLiteral::from(zone)
    );
    let rows = state
        .clickhouse
        .rows(&sql, None)
        .await
        .map_err(|err| upstream_error::ch_error(&upstream_error::DATABASE, &err))?;
    let n = rows
        .first()
        .and_then(|r| r.get("n"))
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0);
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_body_requires_both_fields_and_refuses_extra_ones() {
        assert!(
            serde_json::from_str::<SettingsBody>(r#"{"timeZone":"UTC","weekStart":"monday"}"#)
                .is_ok()
        );
        for bad in [
            r#"{"timeZone":"UTC"}"#,
            r#"{"weekStart":"monday"}"#,
            r#"{"timeZone":"UTC","weekStart":"monday","x":1}"#,
            r#"{"timeZone":1,"weekStart":"monday"}"#,
            "[]",
        ] {
            assert!(serde_json::from_str::<SettingsBody>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_response_names_the_zone_the_first_day_and_whether_it_was_saved() {
        let ctx = TimeContext::new("Europe/Berlin", WeekStart::Sunday).unwrap();
        assert_eq!(
            body_json(&ctx, true),
            json!({"timeZone": "Europe/Berlin", "weekStart": "sunday", "saved": true})
        );
        assert_eq!(
            body_json(&TimeContext::default(), false),
            json!({"timeZone": "Asia/Jakarta", "weekStart": "monday", "saved": false})
        );
    }
}
