//! The assistant's report and template tools (`BI-18` part D, T4; feature page
//! `docs/core/features/reports-and-templates.md`, decisions R5 and T5, with
//! `AI-9`'s templates half): `list_reports` (review round 1, SHOULD-FIX 2),
//! `create_report`, `update_report`,
//! `delete_report`, `list_templates`, `create_dashboard_from_template` and
//! `delete_template`.
//!
//! "Reports" here are the written pages of `BI-18` part D, not the scheduled
//! email reports of `RPT-1` (`tools::reports`), hence the module's name.
//!
//! # Assistant parity
//!
//! Every tool calls the function its console route calls
//! (`routes::dashboard_reports::{create_report, update_report, trash_report,
//! readable_report}`, `routes::dashboard_templates::{list_templates,
//! start_from_template, delete_template}`), so the checks are the console's
//! and cannot drift: `dashboard:write` (the tools' declared permission, which
//! the gate enforces), `edit` on the folder, the block rules with their fixed
//! sentences (the 31st chart names the limit), a chart the person cannot see
//! refused, and a deleted report in the trash. With nobody signed in nothing
//! is read or written.
//!
//! `update_report` adds to the console's replace-all edit the two moves a
//! conversation needs: `append` (blocks added at the end) and `chart` (one
//! chart block added at the end, "add this chart to the report"). Both read
//! the report as the person, add the blocks, and save through the same
//! function, so the limits apply to the result.

use lakehouse_auth::Principal;
use serde_json::{Map, Value, json};

use super::arg_str;
use crate::routes::dashboard_reports::{self as reports, ReportChange};
use crate::routes::dashboard_templates as templates;
use crate::state::AppState;

/// The tools this module answers.
pub(super) const TOOLS: [&str; 7] = [
    "list_reports",
    "create_report",
    "update_report",
    "delete_report",
    "list_templates",
    "create_dashboard_from_template",
    "delete_template",
];

/// Whether `name` is one of this module's tools (one guard arm in `run_tool`).
pub(super) fn handles(name: &str) -> bool {
    TOOLS.contains(&name)
}

fn error(err: &lakehouse_core::ApiError) -> Value {
    json!({ "error": err.to_string() })
}

fn signed_in(principal: Option<&Principal>) -> Result<&Principal, Value> {
    principal.ok_or_else(|| {
        json!({ "error": "Reports and templates are changed as the signed-in person, and there is none." })
    })
}

/// The console address of a report, for the chat to link.
fn report_url(id: &uuid::Uuid) -> String {
    format!("/dashboards/reports/{id}")
}

/// Run one of [`TOOLS`].
pub(super) async fn run(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    let principal = match signed_in(principal) {
        Ok(principal) => principal,
        Err(refusal) => return refusal,
    };
    let result = match name {
        "list_reports" => list_reports(state, principal).await,
        "create_report" => create_report(state, principal, args).await,
        "update_report" => update_report(state, principal, args).await,
        "delete_report" => delete_report(state, principal, args).await,
        "list_templates" => templates::list_templates(state)
            .await
            .map_err(|e| error(&e)),
        "create_dashboard_from_template" => start(state, principal, args).await,
        "delete_template" => delete_template(state, args).await,
        other => Err(json!({ "error": format!("unknown tool: {other}") })),
    };
    result.unwrap_or_else(|refusal| refusal)
}

/// The report as the chat is told it: what was made, with its address.
fn made(report: &lakehouse_store::dashboard_reports::Report) -> Value {
    let blocks = report.blocks.as_array().map_or(0, Vec::len);
    let charts = report
        .blocks
        .as_array()
        .map_or(0, |b| b.iter().filter(|b| b["type"] == "chart").count());
    json!({
        "ok": true,
        "report": {
            "id": report.id,
            "title": report.title,
            "folderId": report.folder_id,
            "blocks": blocks,
            "charts": charts,
        },
        "url": report_url(&report.id),
    })
}

/// The most reports `list_reports` gives the model.
const MAX_LISTED: usize = 100;

/// `list_reports` (review round 1, SHOULD-FIX 2): the reports the person may
/// read, through the list route's own function, so `update_report` has an id
/// to name. A digest: id, title, folder, last saved and chart count, newest
/// first, at most [`MAX_LISTED`], saying when there were more.
async fn list_reports(state: &AppState, principal: &Principal) -> Result<Value, Value> {
    let (reports, truncated) = reports::readable_reports(state, principal)
        .await
        .map_err(|e| error(&e))?;
    let cut = truncated || reports.len() > MAX_LISTED;
    let listed: Vec<Value> = reports
        .iter()
        .take(MAX_LISTED)
        .map(|r| {
            json!({
                "id": r.id,
                "title": r.title,
                "folderId": r.folder_id,
                "updatedAt": r.updated_at.format(&time::format_description::well_known::Rfc3339).unwrap_or_default(),
                "charts": r.chart_count,
                "url": report_url(&r.id),
            })
        })
        .collect();
    Ok(json!({ "reports": listed, "truncated": cut }))
}

async fn create_report(
    state: &AppState,
    principal: &Principal,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let blocks = args.get("blocks").cloned().unwrap_or_else(|| json!([]));
    let report = reports::create_report(
        state,
        principal,
        &arg_str(args, "title"),
        &arg_str(args, "folderId"),
        blocks,
    )
    .await
    .map_err(|e| error(&e))?;
    Ok(made(&report))
}

fn report_id(args: &Map<String, Value>) -> Result<uuid::Uuid, Value> {
    let raw = arg_str(args, "id");
    if raw.trim().is_empty() {
        return Err(json!({ "error": "id is required: the report's id, from create_report." }));
    }
    reports::report_id(&raw).map_err(|e| error(&e))
}

async fn update_report(
    state: &AppState,
    principal: &Principal,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let id = report_id(args)?;
    let mut blocks = args.get("blocks").cloned();
    let append = args.get("append").cloned();
    let chart = arg_str(args, "chart");
    if append.is_some() || !chart.trim().is_empty() {
        // Read as the person, then add: the save below checks the whole.
        let scope = crate::routes::dashboard_access::scope_for_write(state, principal)
            .await
            .map_err(|e| error(&e))?;
        let current = match &blocks {
            Some(given) => given.clone(),
            None => {
                reports::readable_report(state, &scope.viewer, id)
                    .await
                    .map_err(|e| error(&e))?
                    .blocks
            }
        };
        let mut list = current.as_array().cloned().unwrap_or_default();
        match append {
            Some(Value::Array(more)) => list.extend(more),
            Some(_) => return Err(json!({ "error": "append must be a list of blocks." })),
            None => {}
        }
        if !chart.trim().is_empty() {
            list.push(json!({ "type": "chart", "chart": chart.trim() }));
        }
        blocks = Some(Value::Array(list));
    }
    let report = reports::update_report(
        state,
        principal,
        id,
        ReportChange {
            title: args.get("title").and_then(Value::as_str).map(str::to_owned),
            folder_id: args
                .get("folderId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            blocks,
        },
    )
    .await
    .map_err(|e| error(&e))?;
    Ok(made(&report))
}

async fn delete_report(
    state: &AppState,
    principal: &Principal,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let id = report_id(args)?;
    let report = reports::trash_report(state, principal, id)
        .await
        .map_err(|e| error(&e))?;
    Ok(json!({
        "ok": true,
        "deleted": { "id": report.id, "title": report.title },
        "note": "The report is in the trash and can be restored for 30 days.",
    }))
}

async fn start(
    state: &AppState,
    principal: &Principal,
    args: &Map<String, Value>,
) -> Result<Value, Value> {
    let raw = arg_str(args, "template");
    let id = raw
        .trim()
        .parse::<uuid::Uuid>()
        .map_err(|_| json!({ "error": templates::TEMPLATE_NOT_FOUND }))?;
    let started = templates::start_from_template(
        state,
        principal,
        id,
        &arg_str(args, "name"),
        &arg_str(args, "folderId"),
    )
    .await
    .map_err(|e| error(&e))?;
    Ok(json!({
        "ok": true,
        "board": { "id": started.board.id, "name": started.board.name },
        "leftOut": started.left_out,
        "url": format!("/dashboards/{}", started.board.id),
    }))
}

async fn delete_template(state: &AppState, args: &Map<String, Value>) -> Result<Value, Value> {
    let raw = arg_str(args, "id");
    let id = raw
        .trim()
        .parse::<uuid::Uuid>()
        .map_err(|_| json!({ "error": templates::TEMPLATE_NOT_FOUND }))?;
    templates::delete_template(state, id)
        .await
        .map_err(|e| error(&e))?;
    Ok(json!({ "ok": true, "deleted": id }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_module_answers_its_seven_tools_and_no_scheduled_report_tool() {
        for name in TOOLS {
            assert!(handles(name), "{name}");
        }
        assert!(!handles("create_report_subscription"));
        assert!(!handles("list_report_subscriptions"));
    }

    #[test]
    fn a_report_address_is_the_reader_page() {
        let id = uuid::Uuid::nil();
        assert_eq!(
            report_url(&id),
            "/dashboards/reports/00000000-0000-0000-0000-000000000000"
        );
    }
}
