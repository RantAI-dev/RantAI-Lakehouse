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

/// What the page was reading with, from the arguments, as the texts
/// `GET /api/dashboard` takes: `filters` (a JSON list), `params` (a JSON
/// object) and `grain`. The model copies them from the page context, which lists
/// the active filters as raw JSON (`BI-1` review fix (SHOULD-FIX) R7); a value
/// the console would refuse is refused here with its sentence.
fn view_pairs(args: &Map<String, Value>) -> Result<Vec<(&'static str, String)>, String> {
    let mut out = Vec::new();
    if let Some(filters) = args
        .get("filters")
        .filter(|f| f.as_array().is_some_and(|a| !a.is_empty()))
    {
        let text = filters.to_string();
        crate::routes::dashboard::check_filters_text(Some(&text)).map_err(|_| {
            "filters must be the dashboard's filter list, copied from the page context.".to_owned()
        })?;
        out.push(("filters", text));
    }
    if let Some(params) = args
        .get("params")
        .filter(|p| p.as_object().is_some_and(|o| !o.is_empty()))
    {
        out.push(("params", params.to_string()));
    }
    let grain = arg_str(args, "grain");
    if !grain.is_empty() {
        let fits = lakehouse_bi::grain::Grain::parse(&grain)
            .is_some_and(lakehouse_bi::grain::Grain::is_truncation);
        if !fits {
            return Err(
                "grain must be one of minute, hour, day, week, month, quarter, year.".to_owned(),
            );
        }
        out.push(("grain", grain));
    }
    Ok(out)
}

/// Refuse a parameter name or a filter column the dashboard does not have, with
/// a sentence the model can fix (BI-1 QA fix A1: `ukuren_demo` for `ukuran_demo`
/// was passed into the link and failed only when the person opened it).
/// `params` are the defined parameters' names (`None`: not readable, left to the route); `columns` the columns the
/// dashboard's charts read, or `None` when they cannot all be known (a SQL
/// source, a join or a calculated field), in which case the columns are left to
/// the download route.
fn check_view_names(
    args: &Map<String, Value>,
    params: Option<&[String]>,
    columns: Option<&std::collections::BTreeSet<String>>,
) -> Result<(), String> {
    if let (Some(params), Some(given)) = (params, args.get("params").and_then(Value::as_object))
        && let Some(unknown) = given.keys().find(|k| !params.iter().any(|p| p == *k))
    {
        let known = if params.is_empty() {
            "none".to_owned()
        } else {
            params.join(", ")
        };
        return Err(format!(
            "There is no parameter named {unknown:?}. The parameters are: {known}. Use one of these names exactly, or leave params out."
        ));
    }
    let (Some(columns), Some(filters)) = (columns, args.get("filters").and_then(Value::as_array))
    else {
        return Ok(());
    };
    for filter in filters {
        let column = filter.get("column").and_then(Value::as_str).unwrap_or("");
        let is_segment = filter.get("segment").is_some_and(|s| !s.is_null());
        if !is_segment && !columns.contains(column) {
            let sample: Vec<&str> = columns.iter().take(20).map(String::as_str).collect();
            return Err(format!(
                "There is no column named {column:?} on this data. Columns include: {}. Copy the filters from the page context exactly, or leave filters out.",
                sample.join(", ")
            ));
        }
    }
    Ok(())
}

/// The defined parameters' names (`None` when they cannot be read: the route
/// then checks them when the link is opened).
async fn parameter_names(state: &AppState) -> Option<Vec<String>> {
    match lakehouse_bi::params::list_params(&state.clickhouse).await {
        Ok(defs) => Some(defs.into_iter().map(|d| d.name).collect()),
        Err(err) => {
            let _logged = crate::upstream_error::report_ch(&crate::upstream_error::DATABASE, &err);
            None
        }
    }
}

/// The columns the given charts read, when they all read plain marts.
async fn chart_columns(
    state: &AppState,
    charts: &[&store::StoredChartSpec],
) -> Option<std::collections::BTreeSet<String>> {
    let catalog = match lakehouse_bi::fields::list_catalog_fields(&state.clickhouse).await {
        Ok(found) => lakehouse_bi::fields::FieldCatalog::from_fields(found),
        Err(_) => return None,
    };
    if charts.iter().any(|c| {
        c.def.sql_source.is_some()
            || !c.def.shape.joins_list().is_empty()
            || catalog.chart_uses(&c.def)
    }) {
        return None;
    }
    let marts = crate::routes::support::mart_columns(&state.clickhouse)
        .await
        .ok()?;
    let mut out = std::collections::BTreeSet::new();
    for chart in charts.iter().filter(|c| !c.spec.kind.is_static()) {
        out.extend(marts.get(&chart.def.mart)?.keys().cloned());
    }
    Some(out)
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
    // BI-19 B1 review R1: the resolved viewer, not the baseline.
    let viewer = match super::viewer_for(state, Some(principal)).await {
        Ok(viewer) => viewer,
        Err(error) => return error,
    };
    let board = match store::get_board(&state.clickhouse, board_id, &viewer).await {
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
    let date = crate::routes::settings::report_date(state).await;
    // BI-1 QA fix A1: names the dashboard does not have are refused here.
    if args.contains_key("params") || args.contains_key("filters") {
        let params = parameter_names(state).await;
        let on_board: Vec<store::StoredChartSpec> =
            match store::list_stored_charts(&state.clickhouse, &viewer).await {
                Ok(all) => all
                    .into_iter()
                    .filter(|c| {
                        c.board == board_id
                            || (c.board.is_empty() && board_id == store::DEFAULT_BOARD_ID)
                    })
                    .collect(),
                Err(_) => Vec::new(),
            };
        let refs: Vec<&store::StoredChartSpec> = on_board.iter().collect();
        let columns = chart_columns(state, &refs).await;
        if let Err(sentence) = check_view_names(args, params.as_deref(), columns.as_ref()) {
            return json!({ "error": sentence });
        }
    }
    let view = match view_pairs(args) {
        Ok(view) => view,
        Err(sentence) => return json!({ "error": sentence }),
    };
    let mut pairs = vec![
        ("board", board_id),
        ("paper", paper.as_str()),
        ("orientation", orientation.as_str()),
    ];
    pairs.extend(view.iter().map(|(k, v)| (*k, v.as_str())));
    json!({
        "url": link("/api/dashboard/export/pdf", &pairs),
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
    // BI-19 B1 review R1: the resolved viewer, not the baseline.
    let viewer = match super::viewer_for(state, Some(principal)).await {
        Ok(viewer) => viewer,
        Err(error) => return error,
    };
    let charts = match store::list_stored_charts(&state.clickhouse, &viewer).await {
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
    // BI-1 QA fix A1: names the dashboard does not have are refused here.
    if args.contains_key("params") || args.contains_key("filters") {
        let params = parameter_names(state).await;
        let columns = chart_columns(state, &[chart]).await;
        if let Err(sentence) = check_view_names(args, params.as_deref(), columns.as_ref()) {
            return json!({ "error": sentence });
        }
    }
    let view = match view_pairs(args) {
        Ok(view) => view,
        Err(sentence) => return json!({ "error": sentence }),
    };
    let mut pairs = vec![
        ("chart", chart_id),
        ("format", format.extension()),
        ("values", values_word),
    ];
    pairs.extend(view.iter().map(|(k, v)| (*k, v.as_str())));
    json!({
        "url": link("/api/dashboard/chart-export", &pairs),
        "fileName": format!("{}.{}", file_slug(&chart.spec.title, "table"), format.extension()),
        "format": format.extension(),
        "note": "Give the person this link; the file is made when they open it, with their own permissions and the filters, parameters and grain in the link.",
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

    /// BI-1 QA fix A1: a garbled parameter name or an unknown filter column is
    /// refused with a sentence that names the right ones.
    #[test]
    fn an_unknown_parameter_or_filter_column_is_refused_with_a_sentence() {
        let params = vec!["ukuran_demo".to_owned(), "top".to_owned()];
        let columns: std::collections::BTreeSet<String> = ["region", "visitors"]
            .iter()
            .map(|c| (*c).to_owned())
            .collect();
        let args = |v: Value| v.as_object().unwrap().clone();

        let garbled = args(json!({ "params": { "ukuren_demo": 3 } }));
        let err = check_view_names(&garbled, Some(&params), Some(&columns)).unwrap_err();
        assert!(
            err.contains("\"ukuren_demo\"") && err.contains("ukuran_demo, top"),
            "{err}"
        );

        let right = args(
            json!({ "params": { "ukuran_demo": 3 }, "filters": [{ "column": "region", "values": ["x"] }] }),
        );
        assert!(check_view_names(&right, Some(&params), Some(&columns)).is_ok());

        let bad_column = args(json!({ "filters": [{ "column": "regon", "values": ["x"] }] }));
        let err = check_view_names(&bad_column, Some(&params), Some(&columns)).unwrap_err();
        assert!(
            err.contains("\"regon\"") && err.contains("region, visitors"),
            "{err}"
        );

        // A segment filter has no column; unknown columns and parameters are left
        // to the route when they cannot be known.
        let segment =
            args(json!({ "filters": [{ "column": "", "values": [], "segment": "vip" }] }));
        assert!(check_view_names(&segment, Some(&params), Some(&columns)).is_ok());
        assert!(check_view_names(&bad_column, Some(&params), None).is_ok());
        assert!(check_view_names(&garbled, None, None).is_ok());
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
