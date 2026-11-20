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
use crate::routes::settings::{
    audit_settings_event, audit_settings_refusal, changed_setting_paths, handler_refusal_status,
};
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

/// The body of a read. `BI-5` review S6 (feature decision 6): the install's
/// numbers and the roles that have a limit of their own are the administrator's
/// to read; everyone else is told the limits that apply to them (`effective`)
/// and nothing about other roles. `all` is whether the caller may see the rest.
fn body_json(snap: &Snapshot, principal: &Principal, all: bool) -> Value {
    let effective = snap.effective_for(principal).to_json();
    if !all {
        return json!({ "effective": effective });
    }
    json!({
        "install": install_json(&snap.install),
        "roles": snap.roles.iter().map(role_json).collect::<Vec<_>>(),
        "effective": effective,
    })
}

/// `GET /api/settings/limits` — the limits that apply to the caller, and, to a
/// holder of `settings:write` only, the install's limits and the roles that have
/// a time limit of their own (`BI-5` review S6).
///
/// # Errors
///
/// 5xx through `upstream_error` when the settings cannot be read.
pub async fn get_limits(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    let snap = snapshot(&state).await?;
    Ok(ApiJson(body_json(
        &snap,
        &principal,
        principal.permissions.has("settings:write"),
    )))
}

/// `PUT /api/settings/limits` — save `{ install, roles }`. The route's audit
/// rows are the handler's (plan D3): the save writes one naming the settings
/// that changed, and a save this handler refuses writes its own `refused`
/// row.
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
    let reply = save_limits(&state, &principal, body).await;
    match reply {
        Ok(reply) => Ok(reply),
        Err(err) => {
            // Plan D3: the route's refusal row is now the handler's to write;
            // the gate's 403 row is separate and already written there.
            if let Some(status) = handler_refusal_status(&err) {
                audit_settings_refusal(
                    &state,
                    &principal,
                    "settings.limits.update",
                    "limits",
                    status,
                )
                .await;
            }
            Err(err.into())
        }
    }
}

/// The limits as the console names them, for the snapshot a save's row diffs
/// against (plan D3): `install.*` and one entry per role keyed by the role's
/// NAME, so the paths read `roles.Analyst.timeoutSeconds` the way the console
/// names a role limit.
fn limits_snapshot_json(install: &QueryLimits, roles: &[RoleLimit]) -> Value {
    let roles: serde_json::Map<String, Value> = roles
        .iter()
        .map(|role| {
            (
                role.role_name.clone(),
                json!({ "timeoutSeconds": role.timeout_seconds }),
            )
        })
        .collect();
    json!({ "install": install_json(install), "roles": roles })
}

/// The save behind [`put_limits`], whose errors the wrapper turns into the
/// refusal row.
async fn save_limits(
    state: &AppState,
    principal: &Principal,
    body: Bytes,
) -> Result<ApiJson<Value>, ApiError> {
    // Parsed by hand so a malformed body gets this module's message.
    let parsed: LimitsBody =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest(BODY_SHAPE.to_owned()))?;
    let (install, roles) = checked(&parsed)?;
    let Some(pool) = state.pg.as_deref() else {
        return Err(ApiError::Unavailable(
            "Settings need a database, and none is configured.".to_owned(),
        ));
    };
    let (replaced, stored) = match lakehouse_store::query_limits::save(
        pool,
        &install,
        &roles,
        &principal.id.uuid().to_string(),
    )
    .await
    {
        Ok(saved) => saved,
        Err(StoreError::ForeignKeyViolation) => {
            return Err(ApiError::BadRequest(
                "The role limits must name roles that exist.".to_owned(),
            ));
        }
        Err(err) => {
            return Err(upstream_error::internal_error(&upstream_error::STORE, &err));
        }
    };
    // Cleared after the write so the next tile of this process sees the new
    // numbers (plan D2).
    state.query_limits.clear();
    // Plan D3: the row names the settings that changed — the dotted paths
    // between the values this save replaced (plan D3b) and the values it
    // stored, with the roles' names, never a value. Both sides come out of
    // the save's own transaction, so no read sits between the write and the
    // row: a read after the commit could fail, which would leave a committed
    // save with no record of it, or name another save's values as this one's.
    // The store seeds the install row, so no replaced row reads as the
    // defaults every reader falls back to.
    let paths = changed_setting_paths(
        &limits_snapshot_json(&replaced.install.unwrap_or_default(), &replaced.roles),
        &limits_snapshot_json(&stored.install, &stored.roles),
    );
    let detail = (!paths.is_empty()).then(|| paths.join(", "));
    audit_settings_event(
        state,
        principal,
        "settings.limits.update",
        "limits",
        json!({ "fields": paths }),
        detail.as_deref(),
    )
    .await;
    // The answer carries the values the save stored, so the caller sees what
    // was written and not what another save wrote in between.
    let snap = Snapshot {
        install: stored.install,
        roles: stored.roles,
    };
    Ok(ApiJson(body_json(&snap, principal, true)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn parse(text: &str) -> Result<(QueryLimits, Vec<(Uuid, i32)>), ApiError> {
        let body: LimitsBody = serde_json::from_str(text).expect("shape");
        checked(&body)
    }

    const OK: &str = r#"{"install":{"timeoutSeconds":60,"chartRows":2000,"mapPoints":5000,"studioRows":64000},"roles":[]}"#;

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
                        "chartRows" => 2000,
                        "studioRows" => 64000,
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

    /// The save's answer and its audit row come from the values the store
    /// returned (plan D3b), so the route reads the limits nowhere between the
    /// write and the row. A read after the commit could fail, which would
    /// leave a committed save with no record of it, or name another save's
    /// values as this one's — the race the store's own transaction closed.
    ///
    /// The property is about the shape of the code, so it is pinned by
    /// reading it: the save holds no read of the limits at all.
    #[test]
    fn the_save_reads_the_limits_nowhere_after_the_write() {
        let source = include_str!("query_limits.rs");
        let start = source
            .find("async fn save_limits(")
            .expect("the save is in this file");
        let rest = &source[start..];
        let end = rest.find("\n}\n").expect("the save ends with a brace");
        let body = &rest[..end];
        for read in [
            "snapshot(",
            "query_limits::get(",
            "query_limits::role_limits(",
        ] {
            assert!(
                !body.contains(read),
                "the save must answer from the values the store returned, so it reads the \
                 limits nowhere after the write; it reads them with `{read}`: {body}"
            );
        }
    }
}
