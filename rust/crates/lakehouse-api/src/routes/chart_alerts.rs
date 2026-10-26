//! Chart alerts (`BI-19` part C, decision D12 of
//! `docs/superpowers/plans/2026-10-18-bi-19-sharing.md`; feature page
//! `docs/core/features/sharing-and-delivery.md`, C1 to C4).
//!
//! A chart alert watches one number of one chart of a dashboard. Two things live
//! here because they need the API's dashboard code, which `lakehouse-alerts`
//! cannot depend on:
//!
//! - [`evaluate`]: the 15-minute pass reads the dashboard **as the person who
//!   set the alert** (C4) through [`crate::routes::dashboard::payload_for`], the
//!   same read their browser makes, so their roles, masking, row filters and
//!   dashboard rules apply, and the rule's filters ride in the same `filters`
//!   parameter the dashboard takes. The number is taken from the chart's tile
//!   (`lakehouse_alerts::chart_number`) and settled with
//!   [`lakehouse_alerts::settle`], so it fires once and recovers once (D11).
//! - [`authorise_save`] and [`may_delete`]: who may save or remove such a rule
//!   (C3), checked in the handler because the router's floor for
//!   `/api/alerts` is "signed in".
//!
//! # Planner decisions of 2026-10-26 (cited at the sites below)
//!
//! 1. Self-only target (C3) means the owner's **own email address** only; there
//!    is no console channel.
//! 2. C4 applies to chart rules in this part; mart and metric rules keep the
//!    fixed viewer role (`SqlGate` is unchanged).
//! 3. A chart alert's fired instance detail never carries the value
//!    (`alert_instance` is one list for everyone).
//!
//! # Restricted dashboards (`BI-19` Q-R12)
//!
//! An alert leaves the console by email or webhook, outside the dashboard's
//! rules, so on a dashboard marked "only the people listed" (itself or through a
//! folder above it) the only allowed target is the owner's own address. It is
//! refused when saved and skipped with a recorded reason when the restriction
//! is added later. A store that cannot answer counts as restricted.

use axum::http::StatusCode;
use lakehouse_alerts::{
    AlertChannel, AlertKind, AlertRule, AlertRuleInput, Reading, RunResult, SilenceSource,
    StateSource,
};
use lakehouse_auth::Principal;
use lakehouse_bi::access::Viewer;
use lakehouse_bi::store;
use lakehouse_core::ApiError;
use lakehouse_notify::{EmailSender, WebhookSender};
use serde_json::Value;

use crate::routes::dashboard_access::{board_is_restricted, viewer_for};
use crate::state::AppState;

const NO_OWNER: &str = "The alert has no owner, so it does not run.";
const OWNER_GONE: &str = "The person who set this alert is disabled or no longer exists.";
const LOST_ACCESS: &str = "The person who set this alert can no longer see this chart.";
const CHART_GONE: &str = "The chart was deleted.";
const RESTRICTED: &str = "The dashboard is restricted to the people listed, so its alert is not sent outside the console, except to the person who set it.";
const UNREADABLE: &str = "The chart could not be read.";
const NO_NUMBER_NOW: &str = "The chart no longer has a single number.";
/// The refusal when saving a chart alert for a restricted dashboard with another
/// target.
const RESTRICTED_SAVE: &str = "This dashboard is restricted to the people listed, so its alert can only be sent to your own email address.";
const OWN_EMAIL_ONLY: &str =
    "Without the alert permission an alert can only be sent to your own email address.";
const CHART_NOT_FOUND: &str = "chart not found.";

/// How a check that did not produce a [`Reading`] ends.
enum Unreadable {
    /// The rule cannot run for now and says why (C4): state `stopped`.
    Stop(&'static str),
    /// This check could not run; the state stays (C2).
    Failed(String),
}

fn failed(reason: &str) -> Unreadable {
    Unreadable::Failed(reason.to_owned())
}

/// The address of `owner`, lower-cased; `None` when it cannot be read.
async fn address_of(state: &AppState, owner: &str) -> Option<String> {
    let pg = state.pg.as_deref()?;
    lakehouse_store::identity::get_user(pg, owner)
        .await
        .ok()
        .map(|u| u.email.trim().to_ascii_lowercase())
}

/// Whether the rule's only target is the owner's own address (decision 1).
async fn targets_only_the_owner(state: &AppState, owner: &str, rule: &AlertRule) -> bool {
    rule.channel == AlertChannel::Email
        && address_of(state, owner)
            .await
            .is_some_and(|own| own == rule.target.trim().to_ascii_lowercase())
}

/// Read one chart rule's number as its owner.
async fn read(state: &AppState, rule: &AlertRule) -> Result<Reading, Unreadable> {
    let ch = &state.clickhouse;
    let Some(owner) = rule.owner_id.as_deref() else {
        return Err(Unreadable::Stop(NO_OWNER));
    };
    let chart_id = rule.chart.as_deref().unwrap_or_default();
    let principal = match crate::routes::render::principal_of(state, owner).await {
        Ok(Some(principal)) => principal,
        Ok(None) => return Err(Unreadable::Stop(OWNER_GONE)),
        Err(_) => return Err(failed("The person who set this alert could not be read.")),
    };
    let viewer = match viewer_for(state, &principal).await {
        Ok(viewer) => viewer,
        Err(ApiError::PermissionDenied(_)) => return Err(Unreadable::Stop(LOST_ACCESS)),
        Err(_) => return Err(failed(UNREADABLE)),
    };
    let visible = store::list_stored_charts(ch, &viewer)
        .await
        .map_err(|_| failed(UNREADABLE))?;
    let Some(chart) = visible.iter().find(|c| c.spec.id == chart_id) else {
        // Told apart for the person reading the reason: a chart that is gone, or
        // one the owner's rules no longer reach.
        let all = store::list_stored_charts(ch, &Viewer::system())
            .await
            .map_err(|_| failed(UNREADABLE))?;
        return Err(Unreadable::Stop(
            if all.iter().any(|c| c.spec.id == chart_id) {
                LOST_ACCESS
            } else {
                CHART_GONE
            },
        ));
    };
    // Q-R12: fail closed, a store that cannot answer counts as restricted.
    let restricted = board_is_restricted(state.pg.as_deref(), ch, &chart.board)
        .await
        .unwrap_or(true);
    if restricted && !targets_only_the_owner(state, owner, rule).await {
        return Err(failed(RESTRICTED));
    }
    let filters = if rule.filters.is_empty() {
        None
    } else {
        serde_json::to_string(&rule.filters).ok()
    };
    let query = crate::routes::dashboard::DashboardQuery::for_render(
        &chart.board,
        filters.as_deref(),
        None,
        None,
    )
    // BI-19 C (PR #250 CI): a number that decides a send is never a remembered
    // one (the recovery would arrive a cache age late).
    .fresh_chart(chart_id);
    let payload = match crate::routes::dashboard::payload_for(state, &principal, &query).await {
        Ok(payload) => payload,
        Err(response) => {
            return Err(match response.status() {
                StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => Unreadable::Stop(LOST_ACCESS),
                _ => failed(UNREADABLE),
            });
        }
    };
    let Some(tile) = payload.get("results").and_then(|r| r.get(chart_id)) else {
        return Err(failed(UNREADABLE));
    };
    if tile.get("error").is_some() {
        return Err(failed(UNREADABLE));
    }
    let columns: Vec<String> = tile
        .get("columns")
        .and_then(Value::as_array)
        .map(|c| {
            c.iter()
                .filter_map(|n| n.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let rows: &[Value] = tile
        .get("rows")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let watched = format!("chart \"{}\"", chart.spec.title);
    if rule.condition.as_deref() == Some("any_rows") {
        return Ok(Reading::any_rows(&watched, rows.len()));
    }
    let shape = lakehouse_alerts::number_shape(chart.def.kind);
    if shape == lakehouse_alerts::NumberShape::NoNumber {
        return Err(failed(NO_NUMBER_NOW));
    }
    let value = lakehouse_alerts::chart_number(shape, rule.series.as_deref(), &columns, rows)
        .map_err(Unreadable::Failed)?;
    Ok(Reading::against_threshold(rule, &watched, value))
}

/// The 15-minute pass over every enabled chart rule (`only` narrows it to one
/// id, as `?id=` does). Each rule is read as its owner and settled, so it
/// sends once on a crossing and once on the recovery.
pub(crate) async fn evaluate(
    state: &AppState,
    senders: (&WebhookSender, &EmailSender),
    silence: Option<&dyn SilenceSource>,
    states: Option<&dyn StateSource>,
    only: Option<&str>,
) -> Vec<RunResult> {
    let rules = match lakehouse_alerts::list_rules(&state.clickhouse).await {
        Ok(rules) => rules,
        Err(err) => {
            tracing::warn!(%err, "chart alerts: the rules could not be listed");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for rule in rules.iter().filter(|r| {
        r.enabled
            && r.kind == AlertKind::Alert
            && r.chart.is_some()
            && only.is_none_or(|id| id == r.id)
    }) {
        out.push(match read(state, rule).await {
            Ok(reading) => {
                lakehouse_alerts::settle(senders, silence, states, rule, Ok(reading)).await
            }
            Err(Unreadable::Stop(reason)) => lakehouse_alerts::stop(states, rule, reason).await,
            Err(Unreadable::Failed(reason)) => {
                lakehouse_alerts::settle(senders, silence, states, rule, Err(reason)).await
            }
        });
    }
    out
}

/// The stored rule `id`, for the save and delete guards.
///
/// # Errors
///
/// 503 when the rules cannot be read.
pub(crate) async fn stored_rule(state: &AppState, id: &str) -> Result<Option<AlertRule>, ApiError> {
    lakehouse_alerts::get_rule(&state.clickhouse, id)
        .await
        .map_err(|err| {
            tracing::warn!(%err, "an alert rule could not be read before a change");
            ApiError::Unavailable("the alert rules could not be read".to_owned())
        })
}

/// Keep, of `rules`, the chart rules `principal` may see (`BI-19` C review
/// SF-5): their own, and, with the alert permission, those on charts they can
/// see themselves. A chart rule names a chart id, filter values and an owner, so
/// the alert permission does not widen the list to charts the person cannot
/// open. Every other rule passes (this is only about the new kind of rule,
/// which any signed-in person can now save). A store that cannot answer leaves
/// only their own rules (principle 3).
pub(crate) async fn visible_to(
    state: &AppState,
    rules: Vec<AlertRule>,
    principal: Option<&Principal>,
) -> Vec<AlertRule> {
    if rules.iter().all(|r| r.chart.is_none()) {
        return rules;
    }
    let me = principal.map(|p| p.id.uuid().to_string());
    let seen: Option<Vec<String>> = match principal.filter(|p| p.has("alert:write")) {
        Some(p) => match viewer_for(state, p).await {
            Ok(viewer) => store::list_stored_charts(&state.clickhouse, &viewer)
                .await
                .ok()
                .map(|charts| charts.into_iter().map(|c| c.spec.id).collect()),
            Err(_) => None,
        },
        None => None,
    };
    rules
        .into_iter()
        .filter(|rule| {
            let Some(chart) = rule.chart.as_deref() else {
                return true;
            };
            let own = rule.owner_id.is_some() && rule.owner_id == me;
            own || seen
                .as_ref()
                .is_some_and(|ids| ids.iter().any(|id| id == chart))
        })
        .collect()
}

/// Whether `principal` may delete `rule` (C3): the alert permission, or their
/// own chart rule. With no principal it is `false` (`BI-19` C review SF-4,
/// principle 3: the guard never opens for an unknown caller).
pub(crate) fn may_delete(principal: Option<&Principal>, rule: Option<&AlertRule>) -> bool {
    let Some(principal) = principal else {
        return false;
    };
    principal.has("alert:write")
        || rule.is_some_and(|r| {
            r.chart.is_some()
                && r.owner_id.as_deref() == Some(principal.id.uuid().to_string().as_str())
        })
}

/// The save guard of `POST` and `PUT /api/alerts` (`BI-19` part C, C3), run
/// after the scope checks: the router's floor is "signed in", so the permission
/// is judged here. `stored` is the rule `id` names, read once by the caller
/// ([`stored_rule`]), which also uses it to decide whether the rule's state is
/// kept.
///
/// - Without `alert:write`, only a chart alert whose target is the person's own
///   address (planner decision 1), on a chart they can view; and a change needs
///   an existing **chart** rule that they own: any other id, an unknown one
///   included, is 403 (`BI-19` C review C-R1).
/// - A chart alert needs the chart visible to the caller (404 as for an unknown
///   one) and, on a restricted dashboard, an own-address target (Q-R12).
/// - Sets `input.owner_id`: the caller for every chart rule that is saved, a new
///   one or a changed one (`BI-19` C review C-R2: a rule is read as whoever
///   ends up owning it, and its chart, filters and target were judged as the
///   caller, so it is not kept as the previous owner); the stored owner for a
///   changed rule that is not a chart rule; nothing for a new one (decision 2).
///
/// # Errors
///
/// 401 for a chart alert with no principal; 403 `alert:write`; 404 for a chart
/// the caller cannot see; 400 for a restricted dashboard.
pub(crate) async fn authorise_save(
    state: &AppState,
    principal: Option<&Principal>,
    id: Option<&str>,
    stored: Option<&AlertRule>,
    input: &mut AlertRuleInput,
) -> Result<(), ApiError> {
    let chart = input
        .chart
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_owned);
    let Some(principal) = principal else {
        return if chart.is_some() {
            Err(ApiError::unauthorized())
        } else {
            Ok(())
        };
    };
    let me = principal.id.uuid().to_string();
    let has_write = principal.has("alert:write");
    let kind_is_alert = matches!(input.kind.as_deref(), None | Some("alert"));
    // BI-19 C review C-R1: without the permission a change is to one's own
    // chart rule or nothing, whatever kind the stored rule is.
    if !has_write
        && id.is_some()
        && !stored.is_some_and(|r| r.chart.is_some() && r.owner_id.as_deref() == Some(me.as_str()))
    {
        return Err(ApiError::PermissionDenied("alert:write".to_owned()));
    }
    if chart.is_none() || !kind_is_alert {
        if !has_write {
            return Err(ApiError::PermissionDenied("alert:write".to_owned()));
        }
        input.owner_id = stored.and_then(|r| r.owner_id.clone());
        return Ok(());
    }
    let Some(chart) = chart else {
        return Ok(());
    };
    let own_target = input.channel.as_deref() == Some("email")
        && address_of(state, &me).await.is_some_and(|own| {
            own == input
                .target
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        });
    if !has_write && !own_target {
        return Err(ApiError::PermissionDenied(OWN_EMAIL_ONLY.to_owned()));
    }
    let viewer = viewer_for(state, principal).await?;
    let visible = store::list_stored_charts(&state.clickhouse, &viewer)
        .await
        .map_err(|err| crate::routes::dashboard_folders::classify_ch_error(&err))?;
    let Some(found) = visible.iter().find(|c| c.spec.id == chart) else {
        return Err(ApiError::NotFound(CHART_NOT_FOUND.to_owned()));
    };
    let restricted = board_is_restricted(state.pg.as_deref(), &state.clickhouse, &found.board)
        .await
        .unwrap_or(true);
    if restricted && !own_target {
        return Err(ApiError::BadRequest(RESTRICTED_SAVE.to_owned()));
    }
    // BI-19 C review C-R2: the caller owns the rule from this save on.
    input.owner_id = Some(me);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::may_delete;

    /// `BI-19` C review SF-4: with no principal the guard is closed.
    #[test]
    fn a_delete_with_no_principal_is_refused() {
        assert!(!may_delete(None, None));
    }
}
