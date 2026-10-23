//! Formatting tools (`BI-17` part A, with `AI-8`): `format_chart` and
//! `set_dashboard_theme`.
//!
//! # Posture
//!
//! `format_chart` changes only a chart's formatting and leaves everything else
//! of the chart as it is: it reads the stored definition, replaces the style
//! fields the call names (`format`, `valueFormats`, `dateFormat`, `colors`,
//! `axes`, `labels`, `annotations`, `tooltip`, `conditional`, `referenceLines`;
//! `null` clears one, a field not named is kept), and saves the whole chart
//! through the same builder and the same checks as `update_chart`
//! (`spec_from_input`, `lakehouse_bi::style::check_style`), so the assistant
//! cannot save what the console's builder would refuse and a refusal is the
//! console's own sentence. It does not relabel the chart's author.
//!
//! `set_dashboard_theme` saves a dashboard's theme and font through the console
//! route's own handler (`routes::dashboard::boards_update`), which checks the
//! theme, the font and the administrator's list of allowed fonts.
//!
//! Both are `WriteLow` with `dashboard:write` like `update_chart`: they delete,
//! share and publish nothing, so they need the chat's confirmation and no
//! approval.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use lakehouse_auth::Principal;
use lakehouse_bi::access::Viewer;
use lakehouse_bi::specs::{ChartKind, ChartY};
use lakehouse_bi::store::{self, ChartInput};
use lakehouse_bi::style::ChartColors;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashSet};

use super::{api_result_to_value, arg_str};
use crate::routes::dashboard;
use crate::routes::dashboard_organise::{require_editable_chart, snapshot_as, viewer_of};
use crate::state::AppState;
use crate::upstream_error;

/// Whether `name` is one of the formatting tools (one guard arm in `run_tool`).
pub(super) fn handles(name: &str) -> bool {
    matches!(name, "format_chart" | "set_dashboard_theme")
}

/// The fields `format_chart` may change: the style block of `ChartInput` and
/// the reference lines. Everything else of the chart is out of its reach.
pub(crate) const FORMAT_FIELDS: [&str; 10] = [
    "format",
    "valueFormats",
    "dateFormat",
    "colors",
    "axes",
    "labels",
    "annotations",
    "tooltip",
    "conditional",
    "referenceLines",
];

/// Dispatch one of the two tools by name.
pub(super) async fn run(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    match name {
        "format_chart" => Box::pin(format_chart(state, principal, args)).await,
        "set_dashboard_theme" => set_dashboard_theme(state, principal, args).await,
        other => json!({ "error": format!("unknown tool: {other}") }),
    }
}

/// The stored chart with the formatting `args` names, checked.
///
/// A field named replaces the stored one; `null` removes it; a field not named
/// is kept. Nothing outside [`FORMAT_FIELDS`] is read from `args`.
///
/// # Errors
///
/// The sentence for the model: no formatting field was given, the merged
/// definition is not a chart, or the style or the reference lines are refused
/// (the console's own sentence).
pub(crate) fn merge_format(
    stored: &ChartInput,
    args: &Map<String, Value>,
) -> Result<ChartInput, String> {
    if !FORMAT_FIELDS.iter().any(|f| args.contains_key(*f)) {
        return Err(format!(
            "format_chart changes formatting only: give at least one of {}.",
            FORMAT_FIELDS.join(", ")
        ));
    }
    // `PR #199 review M SHOULD-FIX BI-17 3`: a key that is not a formatting
    // field is refused, not dropped. The tool used to ignore it and answer
    // `formatted: true`, so a model that sent a title or a measure was told
    // the chart had changed when it had not.
    let unknown: Vec<String> = args
        .keys()
        .filter(|key| key.as_str() != "chart" && !FORMAT_FIELDS.contains(&key.as_str()))
        .map(|key| format!("\"{key}\""))
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "format_chart changes formatting only: {} {} not a field it changes. Use update_chart to change what the chart shows.",
            unknown.join(", "),
            if unknown.len() == 1 { "is" } else { "are" }
        ));
    }
    let Ok(mut def) = serde_json::to_value(stored) else {
        return Err("the chart's definition could not be read.".to_owned());
    };
    let Some(fields) = def.as_object_mut() else {
        return Err("the chart's definition could not be read.".to_owned());
    };
    for field in FORMAT_FIELDS {
        match args.get(field) {
            None => {}
            Some(Value::Null) => {
                fields.remove(field);
            }
            Some(value) => {
                fields.insert(field.to_owned(), value.clone());
            }
        }
    }
    let merged = match serde_json::from_value::<ChartInput>(def.clone()) {
        Ok(merged) => merged,
        Err(cause) => return Err(wrong_shape_sentence(&def, &cause.to_string())),
    };
    lakehouse_bi::style::check_style(&merged)?;
    lakehouse_bi::params::check_reference_lines(&merged)?;
    Ok(merged)
}

/// The field whose shape is wrong, when one field alone is to blame.
///
/// A definition that does not read names no field: serde says what it found,
/// not where it found it. Taking each candidate field out of the definition
/// and reading what is left tells which one is to blame, for the definition
/// reads again once the wrong field is gone. Two wrong fields leave nothing
/// to single out, so no field is named.
///
/// The candidates are a parameter because the fields that can be to blame are
/// the ones the caller named: the formatting fields for `format_chart`, the
/// top-level keys of the call for a chart call. A candidate the definition
/// does not hold changes nothing when it is taken out, so it can never be
/// named.
pub(super) fn wrong_field<'a>(def: &Value, candidates: &[&'a str]) -> Option<&'a str> {
    let fields = def.as_object()?;
    let mut wrong: Option<&'a str> = None;
    for field in candidates {
        if !fields.contains_key(*field) {
            continue;
        }
        let mut without = fields.clone();
        without.remove(*field);
        if serde_json::from_value::<ChartInput>(Value::Object(without)).is_ok() {
            if wrong.is_some() {
                return None;
            }
            wrong = Some(*field);
        }
    }
    wrong
}

/// The sentence for a formatting field that does not read: what serde said,
/// and which field was to blame when one field alone is.
///
/// The field is named in front of serde's own words, because serde's words
/// say what was found and not where it was found: the model has to know
/// which of its arguments to fix.
fn wrong_shape_sentence(def: &Value, cause: &str) -> String {
    let named = wrong_field(def, &FORMAT_FIELDS).filter(|field| !cause.contains(*field));
    match named {
        Some(field) => format!("a formatting field has the wrong shape: \"{field}\": {cause}"),
        None => format!("a formatting field has the wrong shape: {cause}"),
    }
}

/// The kinds whose marks are coloured by item name and not by series: the
/// console colours the marks of a pie, a rose, a funnel and a treemap by the
/// name of the data value (`src/features/dashboards/style-option.ts:237`), so
/// a series key names a mark for these and says nothing about the chart's
/// measures.
const KINDS_NAMING_MARKS: [ChartKind; 4] = [
    ChartKind::Pie,
    ChartKind::Rose,
    ChartKind::Funnel,
    ChartKind::Treemap,
];

/// Check the series keys the call names against the names the chart draws.
///
/// The keys are the call's own: a key the stored chart holds is not the
/// call's doing, and a chart saved before this rule existed may hold one the
/// chart cannot draw.
///
/// # Errors
///
/// The sentence for the model: a key the chart does not draw, with the names
/// the chart does draw.
fn check_series_keys(
    def: &ChartInput,
    y: &ChartY,
    args: &Map<String, Value>,
) -> Result<(), String> {
    let Some(series) = args
        .get("colors")
        .and_then(Value::as_object)
        .and_then(|colors| colors.get("series"))
        .and_then(Value::as_object)
    else {
        return Ok(());
    };
    // A chart with a breakdown draws one series per data value, a chart that
    // reads a parameter draws names that are known only once the parameter is
    // resolved, and a chart that colours its marks by item name takes a key
    // that names a mark and not a series: none of them can be checked
    // against the chart's measures.
    if def.breakdown.as_deref().is_some_and(|b| !b.is_empty())
        || def.uses_params()
        || KINDS_NAMING_MARKS.contains(&def.kind)
    {
        return Ok(());
    }
    let drawn = store::drawn_names(y);
    let known: HashSet<&str> = drawn.iter().map(String::as_str).collect();
    for (key, colour) in series {
        if draws_key(def, &drawn, key, colour.as_str().unwrap_or_default()) {
            continue;
        }
        return Err(format!(
            "colors.series names \"{key}\", which the chart does not draw; it draws: {}.",
            lakehouse_bi::style::returns_list(&known)
        ));
    }
    Ok(())
}

/// Whether the chart draws the series key `key`: the key is one of the names
/// the chart draws, or the server's own rewrite (`key_series_by_drawn_name`)
/// turns it into one (a measure's column, `<aggregate>_<column>`).
///
/// The rewrite moves a key instead of saying which key it would move, so the
/// chart is copied with that one key in it and the copy is rewritten: what is
/// left in the copy names a series the chart draws. One key at a time, so
/// that the rewrite's "the drawn name already holds a colour" rule cannot
/// hide a second key of the call.
fn draws_key(def: &ChartInput, drawn: &[String], key: &str, colour: &str) -> bool {
    if drawn.iter().any(|name| name == key) {
        return true;
    }
    let mut copy = def.clone();
    copy.style.colors = Some(ChartColors {
        series: BTreeMap::from([(key.to_owned(), colour.to_owned())]),
        ..Default::default()
    });
    lakehouse_bi::style::key_series_by_drawn_name(&mut copy, drawn);
    copy.style.colors.as_ref().is_some_and(|colors| {
        colors
            .series
            .keys()
            .any(|name| drawn.iter().any(|drawn| drawn == name))
    })
}

async fn format_chart(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let id = arg_str(args, "chart");
    if id.is_empty() {
        return json!({ "error": "chart is required (an id from list_charts)." });
    }
    let ch = &state.clickhouse;
    let viewer: Viewer = principal.map_or_else(Viewer::anonymous, viewer_of);
    // A chart in the trash, one purged for good, or one on somebody else's
    // personal board cannot be edited, and all three answer like a chart
    // that never existed.
    match require_editable_chart(ch, &id, &viewer).await {
        Ok(_) => {}
        Err(crate::routes::dashboard_organise::Gate::Missing(sentence)) => {
            return json!({ "error": sentence });
        }
        Err(crate::routes::dashboard_organise::Gate::Store(cause)) => {
            return upstream_error::report_ch(&upstream_error::DATABASE, &cause).to_json();
        }
    }
    let stored = match store::list_stored_charts(ch, &viewer).await {
        Ok(all) => all.into_iter().find(|c| c.spec.id == id),
        Err(err) => return upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    };
    let Some(stored) = stored else {
        return json!({ "error": "chart not found." });
    };
    let merged = match merge_format(&stored.def, args) {
        Ok(m) => m,
        Err(sentence) => return json!({ "error": sentence }),
    };
    // A series key the chart cannot draw colours nothing, so the call is
    // refused here: nothing is saved yet.
    if let Err(sentence) = check_series_keys(&stored.def, &stored.spec.y, args) {
        return json!({ "error": sentence });
    }
    // BI-9: a chart's SQL is built with the report time zone (a deployment setting).
    let Ok(time) = crate::routes::settings::time_context(state).await else {
        return json!({ "error": "The reporting settings could not be read." });
    };
    let created_by = stored.created_by.clone().unwrap_or_else(|| "ai".to_owned());
    match store::spec_from_input(ch, &merged, stored.source, &created_by, Some(id), &time).await {
        Ok(spec) => match store::insert_chart(ch, &spec).await {
            Ok(()) => {
                snapshot_as(ch, &spec.board, principal).await;
                json!({
                    "formatted": true,
                    "id": spec.spec.id,
                    "title": spec.spec.title,
                    "kind": spec.spec.kind,
                })
            }
            Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
        },
        Err(err) => upstream_error::bi(&upstream_error::DATABASE, &err).to_json(),
    }
}

async fn set_dashboard_theme(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    // The save below is the console route's own handler, which acts as the
    // caller; with nobody attached there is nobody to act as.
    let Some(principal) = principal else {
        return json!({ "error": "set_dashboard_theme changes a dashboard as the signed-in person, and there is none." });
    };
    let board = arg_str(args, "board");
    if board.is_empty() {
        return json!({ "error": "board is required (an id from list_boards)." });
    }
    if board == store::DEFAULT_BOARD_ID {
        return json!({
            "error": "the built-in dashboard has no theme of its own; duplicate it first, or use another dashboard."
        });
    }
    let mut body = Map::new();
    body.insert("id".to_owned(), json!(board));
    for key in ["theme", "font"] {
        if let Some(value) = args.get(key).filter(|v| v.is_string()) {
            body.insert(key.to_owned(), value.clone());
        }
    }
    if body.len() == 1 {
        return json!({ "error": "give a theme (auto, light or dark), a font, or both." });
    }
    let bytes = Bytes::from(serde_json::to_vec(&Value::Object(body)).unwrap_or_default());
    let result = api_result_to_value(
        dashboard::boards_update(State(state.clone()), Extension(principal.clone()), bytes).await,
    )
    .await;
    if result.get("error").is_some() {
        return result;
    }
    json!({
        "updated": true,
        "board": board,
        "theme": args.get("theme"),
        "font": args.get("font"),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    use super::*;
    use crate::config::Config;

    /// The state the tool reads through: no Postgres, so the reporting
    /// settings read as their defaults.
    fn state_for(server: &MockServer) -> AppState {
        let env = HashMap::from([
            ("CH_URL".to_owned(), server.uri()),
            (
                "DATABASE_URL".to_owned(),
                "not a connection string".to_owned(),
            ),
        ]);
        AppState::new(Config::from_map(&env).expect("a valid test config"))
    }

    /// A `ClickHouse` that answers the chart's own state with these rows and
    /// every other statement with none.
    async fn engine(rows: Vec<Value>) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(move |request: &Request| {
                let sql = String::from_utf8_lossy(&request.body);
                let rows = if sql.contains("FROM console.bi_chart") {
                    rows.clone()
                } else {
                    Vec::new()
                };
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": rows, "rows": rows.len() }))
            })
            .mount(&server)
            .await;
        server
    }

    fn stored() -> ChartInput {
        stored_with(&json!({}))
    }

    /// The stored chart the tests use, with the fields `extra` adds or
    /// replaces.
    fn stored_with(extra: &Value) -> ChartInput {
        let mut base = json!({
            "title": "Visitors", "mart": "mart_wisman", "kind": "bar",
            "dimension": "province", "measures": ["visitors"], "limit": 20, "order": "desc",
            "board": "b1", "click": { "kind": "url", "url": "https://example.com/{value}" },
            "format": { "style": "number", "decimals": 1 },
            "referenceLines": [{ "value": 1000.0, "label": "Goal" }],
            "colors": { "palette": "muted" },
        });
        for (key, value) in extra.as_object().unwrap() {
            base[key] = value.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    fn args(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(map) => map,
            other => panic!("not an object: {other}"),
        }
    }

    /// Plan T8: only `colors` is given, so the existing format and reference lines stay.
    #[test]
    fn a_call_naming_only_colors_keeps_the_format_and_the_reference_lines() {
        let before = stored();
        let after = merge_format(
            &before,
            &args(json!({ "chart": "c1", "colors": { "series": { "visitors": "#00aa00" } } })),
        )
        .unwrap();
        assert_eq!(after.style.format, before.style.format);
        assert_eq!(after.shape.reference_lines, before.shape.reference_lines);
        let colors = after.style.colors.unwrap();
        assert_eq!(colors.series["visitors"], "#00aa00");
        assert_eq!(
            colors.palette, None,
            "a field named replaces the stored one whole"
        );
    }

    /// `PR #199 review M SHOULD-FIX BI-17 3`: a key that is not a formatting
    /// field is refused, not dropped. The test before this one asserted the
    /// dropping: it named a title, a mart and a kind, and expected the chart
    /// to be unchanged and the call to succeed. The tool answered
    /// `formatted: true`, so a model that sent a title was told the chart had
    /// changed when it had not.
    #[test]
    fn a_key_that_is_not_a_formatting_field_is_refused() {
        for (key, value) in [
            ("title", json!("Hijacked")),
            ("mart", json!("mart_other")),
            ("kind", json!("pie")),
            ("dimension", json!("x")),
            ("measures", json!(["y"])),
            ("limit", json!(1)),
            ("click", json!(null)),
            ("board", json!("b2")),
        ] {
            let mut call = json!({ "chart": "c1", "labels": { "show": true } });
            call[key] = value;
            let e = merge_format(&stored(), &args(call)).unwrap_err();
            assert!(e.contains(key), "{key}: {e}");
        }
        // The formatting fields themselves still go through.
        let after = merge_format(
            &stored(),
            &args(json!({ "chart": "c1", "labels": { "show": true } })),
        )
        .unwrap();
        assert!(after.style.labels.unwrap().show);
    }

    #[test]
    fn null_clears_a_field_and_a_field_not_named_is_kept() {
        let after = merge_format(
            &stored(),
            &args(json!({ "chart": "c1", "format": null, "referenceLines": null })),
        )
        .unwrap();
        assert!(after.style.format.is_none());
        assert!(after.shape.reference_lines.is_none());
        assert_eq!(
            after.style.colors.unwrap().palette.as_deref(),
            Some("muted")
        );
    }

    #[test]
    fn the_request_of_ai_8_ac1_applies_all_three_changes() {
        // "Show values in rupiah, make the bars green, add a target line at 1,000."
        let after = merge_format(
            &stored(),
            &args(json!({
                "chart": "c1",
                "format": { "style": "currency", "currency": "IDR", "decimals": 0 },
                "colors": { "series": { "visitors": "#16a34a" } },
                "referenceLines": [{ "value": 1000.0, "label": "Target" }],
            })),
        )
        .unwrap();
        assert_eq!(after.style.format.unwrap().currency.as_deref(), Some("IDR"));
        assert_eq!(after.style.colors.unwrap().series["visitors"], "#16a34a");
        assert_eq!(
            after.shape.reference_lines.unwrap()[0].label.as_deref(),
            Some("Target")
        );
    }

    #[test]
    fn a_currency_that_is_not_one_is_refused_with_the_consoles_sentence() {
        let e = merge_format(
            &stored(),
            &args(
                json!({ "chart": "c1", "format": { "style": "currency", "currency": "RUPIAH" } }),
            ),
        )
        .unwrap_err();
        assert!(e.contains("not a currency code"), "{e}");
        let e = merge_format(
            &stored(),
            &args(json!({ "chart": "c1", "format": { "decimals": 11 } })),
        )
        .unwrap_err();
        assert!(e.contains("0 to 10"), "{e}");
    }

    #[test]
    fn a_field_the_kind_does_not_take_is_refused_like_the_builder_does() {
        let e = merge_format(
            &stored(),
            &args(json!({ "chart": "c1", "conditional": [{ "column": "visitors", "color": "#ff0000", "rule": { "type": "contains", "text": "x" } }] })),
        )
        .unwrap_err();
        assert!(e.contains("conditional colours is only for"), "{e}");
        let e = merge_format(
            &stored(),
            &args(
                json!({ "chart": "c1", "referenceLines": [{ "kind": "average", "value": 3.0 }] }),
            ),
        )
        .unwrap_err();
        assert!(e.contains("is computed"), "{e}");
    }

    #[test]
    fn a_field_of_the_wrong_shape_is_refused_with_a_fixed_sentence() {
        let e =
            merge_format(&stored(), &args(json!({ "chart": "c1", "labels": "yes" }))).unwrap_err();
        assert!(e.contains("wrong shape"), "{e}");
        let e = merge_format(
            &stored(),
            &args(json!({ "chart": "c1", "format": { "style": "rupiah" } })),
        )
        .unwrap_err();
        assert!(e.contains("wrong shape"), "{e}");
    }

    /// A refusal that says "a formatting field" leaves the model guessing
    /// which of its arguments is wrong. The sentence names the field, so the
    /// model knows what to fix.
    #[test]
    fn a_field_of_the_wrong_shape_names_the_field() {
        let e = merge_format(
            &stored(),
            &args(json!({ "chart": "c1", "format": { "style": "rupiah" } })),
        )
        .unwrap_err();
        assert_eq!(
            e,
            "a formatting field has the wrong shape: \"format\": unknown variant `rupiah`, expected one of `auto`, `number`, `percent`, `currency`, `scientific`, `compact`"
        );
    }

    #[test]
    fn a_call_with_no_formatting_field_is_refused_and_says_which_are_allowed() {
        let e = merge_format(&stored(), &args(json!({ "chart": "c1", "title": "x" }))).unwrap_err();
        assert!(e.starts_with("format_chart changes formatting only"), "{e}");
        assert!(e.contains("colors") && e.contains("referenceLines"), "{e}");
    }

    #[test]
    fn the_fields_the_tool_may_change_are_exactly_the_style_block_and_the_reference_lines() {
        let all = ChartInput {
            style: serde_json::from_value(json!({
                "format": {}, "valueFormats": {"v": {}}, "dateFormat": {}, "colors": {}, "axes": {},
                "labels": {"show": true}, "annotations": [{"x": "a", "text": "t"}], "tooltip": {"fields": ["v"]},
                "conditional": [{"column": "v", "rule": {"type": "contains", "text": "x"}, "color": "#000000"}],
            }))
            .unwrap(),
            ..stored()
        };
        let json = serde_json::to_value(&all).unwrap();
        for field in FORMAT_FIELDS.iter().filter(|f| **f != "referenceLines") {
            assert!(json.get(*field).is_some(), "{field} is not a style field");
        }
    }

    /// `PR #199 review O SHOULD-FIX 2`: a chart that was purged for good
    /// (state 1) answers like one that never existed, so the answer does not
    /// tell the two apart.
    #[tokio::test]
    async fn a_purged_chart_answers_like_a_chart_that_never_existed() {
        let purged = engine(vec![json!({ "is_deleted": 1, "board": "b1" })]).await;
        let unknown = engine(Vec::new()).await;
        let call = args(json!({ "chart": "c1", "colors": { "palette": "muted" } }));
        let purged_reply = Box::pin(format_chart(&state_for(&purged), None, &call)).await;
        let unknown_reply = Box::pin(format_chart(&state_for(&unknown), None, &call)).await;
        assert_eq!(
            purged_reply, unknown_reply,
            "a purged chart and a chart that never existed answer the same"
        );
        assert_eq!(purged_reply["error"], json!("this tile does not exist"));
    }

    /// The chart the series tests use: two measures, each with an aggregate
    /// of its own, so the names it draws are `sum_jumlah` and `sum_wisman`.
    fn two_measures() -> ChartInput {
        stored_with(&json!({
            "measures": [
                { "column": "jumlah", "aggregate": "sum" },
                { "column": "wisman", "aggregate": "sum" },
            ],
        }))
    }

    /// The names that chart draws.
    fn two_drawn() -> ChartY {
        ChartY::Multi(vec!["sum_jumlah".to_owned(), "sum_wisman".to_owned()])
    }

    /// A series key the chart does not draw colours nothing, so the call is
    /// refused: the refusal names the key and the names the chart draws, so
    /// the model can pick one of them.
    #[test]
    fn a_series_key_the_chart_does_not_draw_is_refused_with_the_names_it_draws() {
        let e = check_series_keys(
            &two_measures(),
            &two_drawn(),
            &args(json!({ "chart": "c1", "colors": { "series": { "Total": "#00aa00" } } })),
        )
        .unwrap_err();
        assert_eq!(
            e,
            "colors.series names \"Total\", which the chart does not draw; it draws: sum_jumlah, sum_wisman."
        );
    }

    /// A key the server's own rewrite turns into a drawn name is accepted:
    /// the column a measure reads and the name its result column is called.
    #[test]
    fn a_measure_column_key_is_accepted() {
        for key in ["jumlah", "sum_jumlah", "wisman", "sum_wisman"] {
            check_series_keys(
                &two_measures(),
                &two_drawn(),
                &args(json!({ "chart": "c1", "colors": { "series": { (key): "#00aa00" } } })),
            )
            .unwrap_or_else(|e| panic!("{key} is refused: {e}"));
        }
    }

    /// A chart that draws one series per data value takes any key: its
    /// series are unknown until the data is read.
    #[test]
    fn a_breakdown_chart_accepts_any_key() {
        check_series_keys(
            &stored_with(&json!({ "breakdown": "gender" })),
            &ChartY::Single("visitors".to_owned()),
            &args(json!({ "chart": "c1", "colors": { "series": { "Total": "#00aa00" } } })),
        )
        .unwrap_or_else(|e| panic!("a key is refused: {e}"));
    }

    /// A chart that colours its marks by item name takes any key: the key
    /// names a mark and not a series.
    #[test]
    fn a_pie_accepts_any_key() {
        check_series_keys(
            &stored_with(&json!({ "kind": "pie" })),
            &ChartY::Single("visitors".to_owned()),
            &args(json!({ "chart": "c1", "colors": { "series": { "Total": "#00aa00" } } })),
        )
        .unwrap_or_else(|e| panic!("a key is refused: {e}"));
    }

    /// A key the stored chart holds is not the call's doing: a call that
    /// names another field is not refused for it.
    #[test]
    fn a_stored_series_key_is_not_refused_when_the_call_names_another_field() {
        let def = stored_with(&json!({ "colors": { "series": { "Total": "#00aa00" } } }));
        check_series_keys(
            &def,
            &ChartY::Single("visitors".to_owned()),
            &args(json!({ "chart": "c1", "colors": { "palette": "muted" } })),
        )
        .unwrap_or_else(|e| panic!("a stored key is refused: {e}"));
    }

    /// The row the store answers with: the chart's own row and the document
    /// it holds.
    fn stored_chart_row(def: &Value, y: &str) -> Value {
        json!({
            "id": "c1",
            "is_deleted": 0,
            "board": "default",
            "created_by": "ai",
            "created_at": "2026-10-22 00:00:00",
            "spec_json": json!({
                "spec": {
                    "id": "c1",
                    "title": "Visitors",
                    "kind": "bar",
                    "mart": "mart_wisman",
                    "sql": "SELECT province, sum(visitors) AS visitors FROM serving.mart_wisman GROUP BY province",
                    "x": "province",
                    "y": y,
                },
                "def": def,
            })
            .to_string(),
        })
    }

    /// The tool answers a series key the chart cannot draw with the sentence
    /// that names the key and the names the chart draws, and saves nothing.
    #[tokio::test]
    async fn a_series_key_the_chart_cannot_draw_is_refused_by_the_tool() {
        let server = engine(vec![stored_chart_row(
            &serde_json::to_value(stored()).unwrap(),
            "visitors",
        )])
        .await;
        let call = args(json!({ "chart": "c1", "colors": { "series": { "Total": "#00aa00" } } }));
        let reply = Box::pin(format_chart(&state_for(&server), None, &call)).await;
        assert_eq!(
            reply["error"],
            json!(
                "colors.series names \"Total\", which the chart does not draw; it draws: visitors."
            ),
            "{reply}"
        );
    }
}
