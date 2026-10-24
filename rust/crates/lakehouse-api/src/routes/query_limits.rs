//! `GET`/`PUT /api/settings/limits` (`BI-5`, plan D1): the time and row limits
//! of charts and Query Studio, for the install and, for the time limit, per
//! role.
//!
//! Reading is `RequiresAuth`: anyone signed in sees the install's values and
//! the limits that apply to them (`effective`), which the console prints in
//! its sentences. Changing them is `settings:write`, like the other sections
//! of Admin > Settings. The numbers live in Postgres
//! (`lakehouse_store::query_limits`) and are read through the memo described
//! in `crate::query_limits`; a `PUT` clears the memo of this process, other
//! processes see the change within 30 seconds.
//!
//! The body is `{ install: { timeoutSeconds, chartRows, mapPoints,
//! studioRows }, roles: [{ roleId, timeoutSeconds }] }`; a role not listed has
//! no limit of its own. The ranges are the feature page's (decisions 1 and 2)
//! and are refused with a sentence that names the range; the table's `CHECK`s
//! are the backstop.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::StoreError;
use lakehouse_store::query_limits::{QueryLimits, RoleLimit};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::query_limits::{Snapshot, snapshot};
use crate::state::AppState;
use crate::upstream_error;

const BODY_SHAPE: &str = "body must be {\"install\": {\"timeoutSeconds\", \"chartRows\", \
    \"mapPoints\", \"studioRows\"}, \"roles\": [{\"roleId\", \"timeoutSeconds\"}]}.";

/// A limit's name, range and sentence (the feature page, decisions 1 and 2).
struct Range {
    label: &'static str,
    min: i32,
    max: i32,
    unit: &'static str,
}

const TIME: Range = Range {
    label: "Time limit",
    min: 10,
    max: 1800,
    unit: "seconds",
};
const CHART_ROWS: Range = Range {
    label: "Rows a chart reads",
    min: 100,
    max: 100_000,
    unit: "rows",
};
const MAP_POINTS: Range = Range {
    label: "Points a map shows",
    min: 100,
    max: 50_000,
    unit: "points",
};
const STUDIO_ROWS: Range = Range {
    label: "Rows Query Studio returns",
    min: 100,
    max: 100_000,
    unit: "rows",
};

fn check(range: &Range, value: i64) -> Result<i32, ApiError> {
    i32::try_from(value)
        .ok()
        .filter(|v| (range.min..=range.max).contains(v))
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "{}: a whole number from {} to {} {}.",
                range.label, range.min, range.max, range.unit
            ))
        })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstallBody {
    timeout_seconds: i64,
    chart_rows: i64,
    map_points: i64,
    studio_rows: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoleBody {
    role_id: Uuid,
    timeout_seconds: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitsBody {
    install: InstallBody,
    roles: Vec<RoleBody>,
}

/// Check a body against the ranges and return what to store.
fn checked(body: &LimitsBody) -> Result<(QueryLimits, Vec<(Uuid, i32)>), ApiError> {
    let install = QueryLimits {
        timeout_seconds: check(&TIME, body.install.timeout_seconds)?,
        chart_rows: check(&CHART_ROWS, body.install.chart_rows)?,
        map_points: check(&MAP_POINTS, body.install.map_points)?,
        studio_rows: check(&STUDIO_ROWS, body.install.studio_rows)?,
    };
    let mut roles: Vec<(Uuid, i32)> = Vec::with_capacity(body.roles.len());
    for role in &body.roles {
        if roles.iter().any(|(id, _)| *id == role.role_id) {
            return Err(ApiError::BadRequest(
                "A role may appear only once in the role limits.".to_owned(),
            ));
        }
        roles.push((role.role_id, check(&TIME, role.timeout_seconds)?));
    }
    Ok((install, roles))
}

fn install_json(l: &QueryLimits) -> Value {
    json!({
        "timeoutSeconds": l.timeout_seconds,
        "chartRows": l.chart_rows,
        "mapPoints": l.map_points,
        "studioRows": l.studio_rows,
    })
}

fn role_json(r: &RoleLimit) -> Value {
    json!({ "roleId": r.role_id, "timeoutSeconds": r.timeout_seconds })
}

fn body_json(snap: &Snapshot, principal: &Principal) -> Value {
    json!({
        "install": install_json(&snap.install),
        "roles": snap.roles.iter().map(role_json).collect::<Vec<_>>(),
        "effective": snap.effective_for(principal).to_json(),
    })
}

/// `GET /api/settings/limits` — the install's limits, the roles that have a
/// time limit of their own, and the limits that apply to the caller.
///
/// # Errors
///
/// 5xx through `upstream_error` when the settings cannot be read.
pub async fn get_limits(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    let snap = snapshot(&state).await?;
    Ok(ApiJson(body_json(&snap, &principal)))
}

/// `PUT /api/settings/limits` — save `{ install, roles }`.
///
/// # Errors
///
/// 400 with a sentence that names the range when a value is outside it, a
/// role is repeated or does not exist, or the body is not that shape; 503
/// without a Postgres pool; 5xx through `upstream_error` when the database
/// fails.
pub async fn put_limits(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    // Parsed by hand so a malformed body gets this module's message.
    let parsed: LimitsBody =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest(BODY_SHAPE.to_owned()))?;
    let (install, roles) = checked(&parsed)?;
    let Some(pool) = state.pg.as_deref() else {
        return Err(ApiError::Unavailable(
            "Settings need a database, and none is configured.".to_owned(),
        )
        .into());
    };
    match lakehouse_store::query_limits::save(
        pool,
        &install,
        &roles,
        &principal.id.uuid().to_string(),
    )
    .await
    {
        Ok(()) => {}
        Err(StoreError::ForeignKeyViolation) => {
            return Err(ApiError::BadRequest(
                "The role limits must name roles that exist.".to_owned(),
            )
            .into());
        }
        Err(err) => {
            return Err(upstream_error::internal_error(&upstream_error::STORE, &err).into());
        }
    }
    // Cleared after the write so the read below, and the next tile of this
    // process, see the new numbers (plan D2).
    state.query_limits.clear();
    let snap = snapshot(&state).await?;
    Ok(ApiJson(body_json(&snap, &principal)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn parse(text: &str) -> Result<(QueryLimits, Vec<(Uuid, i32)>), ApiError> {
        let body: LimitsBody = serde_json::from_str(text).expect("shape");
        checked(&body)
    }

    const OK: &str = r#"{"install":{"timeoutSeconds":60,"chartRows":2000,"mapPoints":5000,"studioRows":2000},"roles":[]}"#;

    #[test]
    fn a_body_inside_every_range_is_accepted() {
        let (install, roles) = parse(OK).expect("ok");
        assert_eq!(install, QueryLimits::default());
        assert_eq!(roles, Vec::<(Uuid, i32)>::new());
    }

    #[test]
    fn each_value_outside_its_range_is_refused_with_a_sentence_naming_the_range() {
        for (field, bad, sentence) in [
            (
                "timeoutSeconds",
                9,
                "Time limit: a whole number from 10 to 1800 seconds.",
            ),
            (
                "timeoutSeconds",
                1801,
                "Time limit: a whole number from 10 to 1800 seconds.",
            ),
            (
                "chartRows",
                99,
                "Rows a chart reads: a whole number from 100 to 100000 rows.",
            ),
            (
                "mapPoints",
                50_001,
                "Points a map shows: a whole number from 100 to 50000 points.",
            ),
            (
                "studioRows",
                100_001,
                "Rows Query Studio returns: a whole number from 100 to 100000 rows.",
            ),
        ] {
            let text = OK.replace(
                &format!(
                    "\"{field}\":{}",
                    match field {
                        "timeoutSeconds" => 60,
                        "chartRows" | "studioRows" => 2000,
                        _ => 5000,
                    }
                ),
                &format!("\"{field}\":{bad}"),
            );
            let err = parse(&text).expect_err("refused");
            assert_eq!(err.to_string(), sentence, "{field} = {bad}");
        }
    }

    #[test]
    fn a_role_time_limit_is_held_to_the_same_range_and_a_role_may_not_repeat() {
        let id = Uuid::new_v4();
        let short = OK.replace(
            "\"roles\":[]",
            &format!("\"roles\":[{{\"roleId\":\"{id}\",\"timeoutSeconds\":5}}]"),
        );
        assert!(parse(&short).is_err());
        let twice = OK.replace(
            "\"roles\":[]",
            &format!(
                "\"roles\":[{{\"roleId\":\"{id}\",\"timeoutSeconds\":60}},{{\"roleId\":\"{id}\",\"timeoutSeconds\":90}}]"
            ),
        );
        assert!(parse(&twice).is_err());
    }

    #[test]
    fn an_unknown_field_is_not_part_of_the_body() {
        let text = OK.replace("\"roles\":[]", "\"roles\":[],\"extra\":1");
        assert!(serde_json::from_str::<LimitsBody>(&text).is_err());
    }
}
