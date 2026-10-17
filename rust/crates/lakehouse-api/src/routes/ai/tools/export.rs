//! Export tools (`AI-10`, export half; `BI-1` plan decision D9):
//! `export_dashboard` and `export_chart`.
//!
//! Both only *give a link*; neither makes a file. The link is the console's own
//! download route (`GET /api/dashboard/export/pdf`, `GET
//! /api/dashboard/chart-export`) with the call's arguments, and the file is
//! made when the person opens it, in their own browser session, so it carries
//! their permissions, their masking and their audit record (`dashboard.pdf_export`
//! / `dashboard.chart_export`). Nothing is generated or stored here, which is
//! why both are `Read` risk.
//!
//! A refusal is the console's sentence: downloads switched off for the person's
//! roles (`BI-1` D7), PDF export not set up on this deployment, a dashboard or
//! chart that does not exist for the person.

use lakehouse_auth::Principal;
use lakehouse_bi::store;
use serde_json::{Map, Value, json};

use super::arg_str;
use crate::export::{FileFormat, Values};
use crate::pdf::{Orientation, Paper};
use crate::routes::dashboard::{file_slug, pdf_supported};
use crate::routes::dashboard_organise::viewer_of;
use crate::state::AppState;

/// What the model is told when the call came without a signed-in person.
const SIGN_IN: &str = "Sign in to export.";

/// Whether `name` is one of the two export tools (one guard arm in `run_tool`).
pub(super) fn handles(name: &str) -> bool {
    matches!(name, "export_dashboard" | "export_chart")
}

/// Dispatch one of the two tools by name.
pub(super) async fn run(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    let Some(principal) = principal else {
        return json!({ "error": SIGN_IN });
    };
    // The same refusal the download routes give a person switched off from
    // downloading; a store failure is already a fixed sentence under a reference.
    if let Err(err) = crate::routes::settings::require_download(state, principal).await {
        return json!({ "error": downloads_off_sentence(&err) });
    }
    match name {
        "export_dashboard" => export_dashboard(state, principal, args).await,
        "export_chart" => export_chart(state, principal, args).await,
        other => json!({ "error": format!("unknown tool: {other}") }),
    }
}

/// The sentence for a refused download, without the `permission_denied:`
/// prefix the HTTP body carries (the model says it to a person).
fn downloads_off_sentence(err: &lakehouse_core::ApiError) -> String {
    match err {
        lakehouse_core::ApiError::PermissionDenied(sentence) => sentence.clone(),
        other => other.to_string(),
    }
}

/// `path?key=value&...` with every value percent-encoded.
fn link(path: &str, pairs: &[(&str, &str)]) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        query.append_pair(key, value);
    }
    format!("{path}?{}", query.finish())
}

async fn export_dashboard(
    state: &AppState,
    principal: &Principal,
    args: &Map<String, Value>,
) -> Value {
    let format = arg_str(args, "format");
    if !format.is_empty() && format != "pdf" {
        return json!({ "error": "A dashboard exports as a PDF: format must be pdf." });
    }
    let paper = match arg_str(args, "paper").as_str() {
        "" => Paper::A4,
        other => match Paper::parse(other) {
            Some(p) => p,
            None => return json!({ "error": "paper must be a4 or letter." }),
        },
    };
    let orientation = match arg_str(args, "orientation").as_str() {
        "" => Orientation::Portrait,
        other => match Orientation::parse(other) {
            Some(o) => o,
            None => return json!({ "error": "orientation must be portrait or landscape." }),
        },
    };
    if !pdf_supported(&state.config) {
        return json!({ "error": crate::routes::pdf_export::PDF_NOT_SET_UP });
    }
    let board_id = arg_str(args, "board");
    let board_id = board_id.trim();
    if board_id.is_empty() {
        return json!({ "error": "board is required (list_boards gives the ids)." });
    }
    let board = match store::get_board(&state.clickhouse, board_id, &viewer_of(principal)).await {
        Ok(board) => board,
        Err(err) => {
            return json!({
                "error": crate::upstream_error::report_ch(&crate::upstream_error::DATABASE, &err)
                    .to_string()
            });
        }
    };
    if board.is_none() && board_id != store::DEFAULT_BOARD_ID {
        return json!({ "error": "this dashboard does not exist" });
    }
    let name = board.as_ref().map_or("Main", |b| b.name.as_str());
    let date = chrono::Utc::now().format("%Y-%m-%d");
    json!({
        "url": link(
            "/api/dashboard/export/pdf",
            &[
                ("board", board_id),
                ("paper", paper.as_str()),
                ("orientation", orientation.as_str()),
            ],
        ),
        "fileName": format!("{}-{date}.pdf", file_slug(name, "dashboard")),
        "format": "pdf",
        "note": "Give the person this link; the PDF is made when they open it, with their own permissions.",
    })
}

async fn export_chart(state: &AppState, principal: &Principal, args: &Map<String, Value>) -> Value {
    let Some(format) = FileFormat::parse(&arg_str(args, "format")) else {
        return json!({ "error": "format must be xlsx or csv." });
    };
    let values = match arg_str(args, "values").as_str() {
        "" => Values::Formatted,
        other => match Values::parse(other) {
            Some(v) => v,
            None => return json!({ "error": "values must be formatted or raw." }),
        },
    };
    let chart_id = arg_str(args, "chart");
    let chart_id = chart_id.trim();
    if chart_id.is_empty() {
        return json!({ "error": "chart is required (list_charts gives the ids)." });
    }
    let charts = match store::list_stored_charts(&state.clickhouse, &viewer_of(principal)).await {
        Ok(charts) => charts,
        Err(err) => {
            return json!({
                "error": crate::upstream_error::report_ch(&crate::upstream_error::DATABASE, &err)
                    .to_string()
            });
        }
    };
    let Some(chart) = charts.iter().find(|c| c.spec.id == chart_id) else {
        return json!({ "error": "this tile does not exist" });
    };
    if chart.spec.kind.is_static() {
        return json!({ "error": "this tile shows no data, so there is nothing to download" });
    }
    let values_word = if values == Values::Raw {
        "raw"
    } else {
        "formatted"
    };
    json!({
        "url": link(
            "/api/dashboard/chart-export",
            &[
                ("chart", chart_id),
                ("format", format.extension()),
                ("values", values_word),
            ],
        ),
        "fileName": format!("{}.{}", file_slug(&chart.spec.title, "table"), format.extension()),
        "format": format.extension(),
        "note": "Give the person this link; the file is made when they open it, with their own permissions and the filters of the page they are on.",
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::config::Config;

    #[test]
    fn a_link_encodes_every_value() {
        assert_eq!(
            link("/api/x", &[("board", "a b&c=d"), ("paper", "a4")]),
            "/api/x?board=a+b%26c%3Dd&paper=a4"
        );
    }

    /// `AI-10`: both are read tools (they make nothing), need only the
    /// permission of seeing a dashboard, and name what they cannot do without.
    #[test]
    fn the_export_tools_are_read_tools_with_dashboard_read_and_their_required_fields() {
        use crate::routes::ai::registry::{self, Risk};
        for (name, required) in [
            ("export_dashboard", json!(["board"])),
            ("export_chart", json!(["chart", "format"])),
        ] {
            let spec = registry::find(name).expect("registered tool");
            assert_eq!(spec.risk, Risk::Read, "{name}");
            assert_eq!(spec.permission, "dashboard:read", "{name}");
            assert_eq!(
                (spec.schema)()["function"]["parameters"]["required"],
                required
            );
        }
    }

    #[test]
    fn only_the_two_export_tools_are_handled_here() {
        assert!(handles("export_dashboard"));
        assert!(handles("export_chart"));
        assert!(!handles("export_gold_mart"));
    }

    #[tokio::test]
    async fn without_a_person_nothing_is_offered() {
        let state = AppState::new(Config::from_map(&HashMap::new()).unwrap());
        let out = run(&state, None, "export_dashboard", &Map::new()).await;
        assert_eq!(out, json!({ "error": SIGN_IN }));
    }
}
