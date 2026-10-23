//! Dashboard/chart tools: `create_chart`, `update_chart`, `delete_chart`,
//! `create_board`, `list_boards`, `list_charts`, `suggest_dashboard`.
//! Moved out of `ai.rs` unchanged (T0.1 registry refactor).

use lakehouse_auth::Principal;
use lakehouse_bi::access::Viewer;
use lakehouse_bi::layout::tab_of;
use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store::{self, ChartInput};
use lakehouse_clickhouse::ChClient;
use serde_json::{Map, Value, json};

use super::arg_str;
use super::look::{FORMAT_FIELDS, wrong_field};
use crate::routes::dashboard_organise::{
    Gate, drop_tile, place_new_chart, require_chart_board, require_editable_chart, snapshot_as,
    viewer_of,
};
use crate::routes::support::is_numeric_type;
use crate::upstream_error;

/// Who the tools read as: the signed-in person, or nobody in particular when
/// none is attached (fail closed: no personal space, nothing in the trash).
fn viewer_for(principal: Option<&Principal>) -> Viewer {
    principal.map_or_else(Viewer::anonymous, viewer_of)
}

/// The tool's error for a refused dashboard or chart: the route's own
/// sentence, or, for a store that could not be read, the fixed message with
/// its reference id (never the database's text).
pub(super) fn refused(gate: Gate) -> Value {
    match gate {
        Gate::Missing(sentence) => json!({ "error": sentence }),
        Gate::Store(cause) => {
            upstream_error::report_ch(&upstream_error::DATABASE, &cause).to_json()
        }
    }
}

/// Parse a tool call's raw args `Value` into a [`ChartInput`], the same way
/// `args as unknown as ChartInput` casts in `ai-tools.ts` (no validation at
/// this boundary — `spec_from_input` validates for real).
///
/// A call that does not read names the field to blame when one field alone is
/// to blame, as `format_chart` does for its own fields: serde's words say
/// what was found and not where it was found, so the model has to know which
/// of its arguments to fix. Two fields of the wrong shape name none, and the
/// error is serde's own words.
fn parse_chart_input(args: &Map<String, Value>) -> Result<ChartInput, String> {
    let def = Value::Object(args.clone());
    match serde_json::from_value::<ChartInput>(def.clone()) {
        Ok(input) => Ok(input),
        Err(cause) => {
            let cause = cause.to_string();
            // The candidates are the call's own top-level keys: a chart's
            // arguments are flat on the wire.
            let candidates: Vec<&str> = args.keys().map(String::as_str).collect();
            match wrong_field(&def, &candidates).filter(|field| !cause.contains(*field)) {
                Some(field) => Err(format!("\"{field}\": {cause}")),
                None => Err(cause),
            }
        }
    }
}

/// The call's args with the formatting the call does not name filled in from
/// the stored chart: a field the call names replaces the stored one (`null`
/// clears it), a field the call leaves out is kept.
///
/// The merge is done on the raw args, key by key, because the `merge_format`
/// of `look.rs` refuses every key that is not a formatting one and this
/// merges into a call that names a title and measures too.
fn args_with_kept_formatting(args: &Map<String, Value>, stored: &ChartInput) -> Map<String, Value> {
    let mut merged = args.clone();
    let Ok(stored) = serde_json::to_value(stored) else {
        return merged;
    };
    let Some(fields) = stored.as_object() else {
        return merged;
    };
    for key in FORMAT_FIELDS {
        if merged.contains_key(key) {
            continue;
        }
        if let Some(value) = fields.get(key) {
            merged.insert(key.to_owned(), value.clone());
        }
    }
    merged
}

/// The call's args with the data source the call does not name filled in
/// from the stored chart: the stored mart, or the stored SQL source.
///
/// The call's own source wins: a call that names a mart or a SQL source
/// keeps it, so the two are never named at once. A kind that reads no data
/// at all (a note, a heading, an image) takes no source, so nothing is
/// carried over.
///
/// The second value says whether a source was carried over.
fn args_with_kept_source(
    args: &Map<String, Value>,
    call: &ChartInput,
    stored: &ChartInput,
) -> (Map<String, Value>, bool) {
    let mut merged = args.clone();
    if call.kind.is_static() {
        return (merged, false);
    }
    if !call.mart.trim().is_empty()
        || call
            .sql_source
            .as_deref()
            .is_some_and(|source| !source.trim().is_empty())
    {
        return (merged, false);
    }
    let mart = stored.mart.trim();
    if !mart.is_empty() {
        merged.insert("mart".to_owned(), json!(mart));
        return (merged, true);
    }
    if let Some(source) = stored
        .sql_source
        .as_deref()
        .map(str::trim)
        .filter(|source| !source.is_empty())
    {
        merged.insert("sqlSource".to_owned(), json!(source));
        return (merged, true);
    }
    (merged, false)
}

/// What a call carries over from the chart it changes: the call's own
/// arguments with the formatting the call does not name filled in, and
/// whether the stored formatting was kept.
struct KeptFormatting {
    /// The stored formatting was kept: the call does not change the kind.
    kept: bool,
    /// The call's arguments, with the kept formatting filled in.
    args: Map<String, Value>,
    /// The stored data source was kept: the call names no source of its own.
    source_kept: bool,
}

/// The formatting a call does not name, carried over from the stored chart.
///
/// The stored chart is read as `viewer`, so a chart the caller may not see
/// is not carried over. An `id` of `None` names no chart, so there is
/// nothing to carry over from and the call is used as it came.
///
/// # Errors
///
/// The error is the reply the caller sends: the fixed upstream error when
/// the store cannot be read, or the call's own parse error when the args
/// are not a chart.
async fn kept_formatting(
    ch: &ChClient,
    viewer: &Viewer,
    id: Option<&str>,
    args: &Map<String, Value>,
) -> Result<KeptFormatting, Value> {
    let stored = match id {
        Some(id) => match store::list_stored_charts(ch, viewer).await {
            Ok(all) => all.into_iter().find(|c| c.spec.id == id),
            Err(err) => {
                return Err(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json());
            }
        },
        None => None,
    };
    let call = match parse_chart_input(args) {
        Ok(i) => i,
        Err(e) => return Err(json!({ "error": e })),
    };
    let Some(stored) = stored else {
        return Ok(KeptFormatting {
            kept: true,
            args: args.clone(),
            source_kept: false,
        });
    };
    // The stored formatting is kept when the call does not change the kind:
    // a kind the call changes takes other formatting (colours are for
    // charts, a second axis for a bar), so the stored formatting is dropped
    // rather than carried into a kind that would refuse it.
    let (kept, args) = if stored.def.kind == call.kind {
        (true, args_with_kept_formatting(args, &stored.def))
    } else {
        (false, args.clone())
    };
    // The data source is carried over whatever the kind becomes: a chart
    // that names no source of its own reads the one the stored chart has.
    let (args, source_kept) = args_with_kept_source(&args, &call, &stored.def);
    Ok(KeptFormatting {
        kept,
        args,
        source_kept,
    })
}

/// The tab a chart sits on, when the board it is on has tabs: the tab its
/// tile names, or the board's first one.
fn tab_of_chart<'a>(board: &'a store::Board, chart_id: &str) -> Option<&'a str> {
    let tabs = board.tabs.as_deref().unwrap_or_default();
    if tabs.is_empty() {
        return None;
    }
    // `tab_of` reads the layout to find the tile's own tab; a board with no
    // layout has no tile, so every chart sits on the first tab.
    let empty = store::LayoutMap::new();
    let layout = board.layout.as_ref().unwrap_or(&empty);
    tab_of(tabs, layout, chart_id)
}

pub(super) async fn create_chart(
    ch: &ChClient,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
    id: Option<String>,
    time: &lakehouse_bi::grain::TimeContext,
) -> Value {
    let mut input = match parse_chart_input(args) {
        Ok(i) => i,
        Err(e) => return json!({ "error": e }),
    };
    // BI-17 part A, decision 6 (`PR #199 review M SHOULD-FIX BI-17 4`): a new
    // chart in a deployment with a company palette draws with it. A palette
    // the model chose is kept, and a chart that already exists keeps what it
    // has (that is `update_chart`, which does not come here).
    //
    // `PR #199 review O BLOCKER 2`: only a kind that takes colours gets one.
    // The style check refuses colours on a table, a KPI or a static tile, so
    // setting the palette on every kind made the assistant unable to create
    // any of them in a deployment with a company palette.
    if time.company_palette().len() >= 2
        && lakehouse_bi::style::is_chart(input.kind)
        && input
            .style
            .colors
            .as_ref()
            .is_none_or(|colors| colors.palette.is_none())
    {
        input
            .style
            .colors
            .get_or_insert_with(Default::default)
            .palette = Some("company".to_owned());
    }
    // BI-18 part C, part 2: not onto a dashboard in the trash or in somebody
    // else's personal space.
    if let Err(err) = require_chart_board(ch, input.board.as_deref(), &viewer_for(principal)).await
    {
        return refused(err);
    }
    match store::spec_from_input(ch, &input, ChartSource::Ai, "ai", id, time).await {
        Ok(spec) => match store::insert_chart(ch, &spec).await {
            Ok(()) => {
                place_new_chart(ch, &spec.board, &spec.spec.id).await;
                snapshot_as(ch, &spec.board, principal).await;
                json!({
                "created": true,
                "id": spec.spec.id,
                "title": spec.spec.title,
                "kind": spec.spec.kind,
                "mart": spec.spec.mart,
                "board": spec.board,
                "url": "/dashboards",
                "note": "Chart tersimpan & langsung tampil di halaman Dashboards.",
                })
            }
            Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
        },
        Err(err) => upstream_error::bi(&upstream_error::DATABASE, &err).to_json(),
    }
}

/// Checks a chart draft before the gate offers it: a draft that cannot be
/// built must not be offered as a card the person can confirm. It runs the
/// same validation as the tools (`store::spec_from_input`) and stops there,
/// so it writes nothing.
///
/// `id` is `None` for a create and `Some` for an update; an update with no
/// `id` is refused here, as the tool refuses it. Returns `None` when the
/// draft can be built, and the error the tool would return otherwise.
pub(super) async fn check_chart_draft(
    ch: &ChClient,
    args: &Map<String, Value>,
    id: Option<String>,
    time: &lakehouse_bi::grain::TimeContext,
) -> Option<Value> {
    if id.as_deref().is_some_and(str::is_empty) {
        return Some(json!({ "error": "id is required" }));
    }
    // The stored chart is read as the server (`Viewer::system()`): this check
    // has no principal of its own. It only answers whether the draft builds;
    // `update_chart` reads the chart again as the caller and refuses a chart
    // the caller may not edit, and a chart id is never listed to a caller who
    // cannot see the chart.
    let viewer = Viewer::system();
    // The same carry-over `update_chart` does, so the card and the save
    // agree. A chart that is not found, or one in the trash, is checked as it
    // came.
    let kept = match Box::pin(kept_formatting(ch, &viewer, id.as_deref(), args)).await {
        Ok(kept) => kept,
        Err(reply) => return Some(reply),
    };
    let input = match parse_chart_input(&kept.args) {
        Ok(i) => i,
        Err(e) => return Some(json!({ "error": e })),
    };
    match store::spec_from_input(ch, &input, ChartSource::Ai, "ai", id, time).await {
        Ok(_) => None,
        Err(err) => Some(upstream_error::bi(&upstream_error::DATABASE, &err).to_json()),
    }
}

pub(super) async fn update_chart(
    ch: &ChClient,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
    time: &lakehouse_bi::grain::TimeContext,
) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    // The gate comes before the args are parsed: the carry-over below needs
    // the chart's stored definition and the board it is on.
    // BI-18 part C, part 2: a chart in the trash or on somebody else's
    // personal board cannot be edited, nor moved onto such a board.
    let viewer = viewer_for(principal);
    let old_board = match require_editable_chart(ch, &id, &viewer).await {
        Ok(board) => board,
        Err(gate) => return refused(gate),
    };
    // The stored chart, read as the caller: the gate above has refused a
    // chart the caller may not see, so this reads only what the caller may.
    // The read is boxed, so this future does not carry every chart on the
    // board.
    let kept = match Box::pin(kept_formatting(ch, &viewer, Some(id.as_str()), args)).await {
        Ok(kept) => kept,
        Err(reply) => return reply,
    };
    let formatting_kept = kept.kept;
    let source_kept = kept.source_kept;
    let mut input = match parse_chart_input(&kept.args) {
        Ok(i) => i,
        Err(e) => return json!({ "error": e }),
    };
    if let Err(err) = require_chart_board(ch, input.board.as_deref(), &viewer).await {
        return refused(err);
    }
    // The chart stays on the board it is on when the call does not name one:
    // without this it would move to the built-in dashboard.
    if input
        .board
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .is_empty()
    {
        input.board = Some(old_board.clone());
    }
    match store::spec_from_input(ch, &input, ChartSource::Ai, "ai", Some(id), time).await {
        Ok(spec) => match store::insert_chart(ch, &spec).await {
            Ok(()) => {
                if old_board != spec.board {
                    snapshot_as(ch, &old_board, principal).await;
                }
                snapshot_as(ch, &spec.board, principal).await;
                let mut reply = json!({
                    "updated": true,
                    "id": spec.spec.id,
                    "title": spec.spec.title,
                    "kind": spec.spec.kind,
                    "mart": spec.spec.mart,
                    "board": spec.board,
                    "formattingKept": formatting_kept,
                });
                if source_kept {
                    reply["sourceKept"] = json!(true);
                }
                if !formatting_kept {
                    reply["note"] = json!(
                        "the chart's kind changed, so its formatting was reset; send the formatting again if the new kind takes it."
                    );
                }
                reply
            }
            Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
        },
        Err(err) => upstream_error::bi(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn create_board(
    ch: &ChClient,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    // `created_by` is left empty: the assistant is not a person, and naming
    // it as the author would put a non-account into a column the UI shows
    // as an owner.
    match store::create_board(
        ch,
        &arg_str(args, "name"),
        &arg_str(args, "description"),
        "",
    )
    .await
    {
        Ok(board) => {
            snapshot_as(ch, &board.id, principal).await;
            json!({
                "created": true, "id": board.id, "name": board.name,
                "note": "Pakai id ini di create_chart.board.",
            })
        }
        Err(err) => upstream_error::bi(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn list_boards(ch: &ChClient, principal: Option<&Principal>) -> Value {
    match store::list_boards(ch, &viewer_for(principal)).await {
        Ok(boards) => {
            let mut out = vec![json!({ "id": "default", "name": "Main" })];
            // BI-18 part C: a board's tabs, when it has any, so the assistant can name them.
            out.extend(boards.iter().map(|b| {
                let mut entry = json!({ "id": b.id, "name": b.name });
                if let Some(tabs) = b.tabs.as_ref().filter(|t| !t.is_empty()) {
                    entry["tabs"] = json!(tabs);
                }
                entry
            }));
            json!({ "boards": out })
        }
        Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn suggest_dashboard(ch: &ChClient) -> Value {
    let marts = match ch
        .rows(
            "SELECT name, toString(total_rows) AS total_rows FROM system.tables \
             WHERE database='serving' AND name NOT LIKE '%\\_baru' ORDER BY name",
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    };
    let mut out = Vec::with_capacity(marts.len());
    for m in &marts {
        let name = m.get("name").and_then(Value::as_str).unwrap_or("");
        let cols = match ch
            .rows(
                &format!("SELECT name, type FROM system.columns WHERE database='serving' AND table='{name}' ORDER BY position"),
                None,
            )
            .await
        {
            Ok(r) => r,
            Err(err) => return upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
        };
        let dimensions: Vec<&str> = cols
            .iter()
            .filter(|c| !is_numeric_type(c.get("type").and_then(Value::as_str).unwrap_or("")))
            .filter_map(|c| c.get("name").and_then(Value::as_str))
            .collect();
        let measures: Vec<&str> = cols
            .iter()
            .filter(|c| is_numeric_type(c.get("type").and_then(Value::as_str).unwrap_or("")))
            .filter_map(|c| c.get("name").and_then(Value::as_str))
            .collect();
        let rows_n: i64 = m
            .get("total_rows")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        out.push(
            json!({ "mart": name, "rows": rows_n, "dimensions": dimensions, "measures": measures }),
        );
    }
    json!({ "marts": out })
}

pub(super) async fn list_charts(ch: &ChClient, principal: Option<&Principal>) -> Value {
    let viewer = viewer_for(principal);
    let charts = match store::list_stored_charts(ch, &viewer).await {
        Ok(charts) => charts,
        Err(err) => return upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    };
    // The boards, for the tab each chart sits on. A board the viewer may not
    // see is not in the list, so its charts are listed without a tab.
    let boards = match store::list_boards(ch, &viewer).await {
        Ok(boards) => boards,
        Err(err) => return upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    };
    let mut out = Vec::with_capacity(charts.len());
    for c in &charts {
        let mut entry = json!({
            "id": c.spec.id, "title": c.spec.title, "kind": c.spec.kind,
            "mart": c.spec.mart, "sqlSource": c.spec.sql_source,
            "source": c.source,
            "board": c.board,
        });
        // The formatting the chart holds, as it is stored: the fields it
        // does not hold stay out, so a chart with no formatting lists an
        // empty style. A `StyleFields` holds no map without a string key
        // and no float JSON cannot hold, so the serialisation cannot fail;
        // the fallback says the style is unknown, not that it is empty.
        entry["style"] = serde_json::to_value(&c.def.style).unwrap_or(Value::Null);
        // The reference lines it holds, when it has any.
        if let Some(lines) = c
            .def
            .shape
            .reference_lines
            .as_ref()
            .filter(|lines| !lines.is_empty())
        {
            entry["referenceLines"] = serde_json::to_value(lines).unwrap_or(Value::Null);
        }
        // The names its series can take: the keys a series colour and a
        // value format are stored under.
        entry["seriesNames"] = json!(store::drawn_names(&c.spec.y));
        let tab = match boards.iter().find(|b| b.id == c.board) {
            Some(board) => tab_of_chart(board, &c.spec.id),
            None => None,
        };
        if let Some(tab) = tab {
            entry["tab"] = json!(tab);
        }
        out.push(entry);
    }
    json!({ "total": out.len(), "charts": out })
}

/// The two read tools that say what a chart can be built on beyond one mart:
/// `list_sql_sources` and `suggest_join` (one dispatch arm, `run_tool` is at
/// clippy's line limit).
pub(super) async fn chart_sources(ch: &ChClient, name: &str, args: &Map<String, Value>) -> Value {
    if name == "suggest_join" {
        suggest_join(ch, args).await
    } else {
        list_sql_sources(ch).await
    }
}

/// `suggest_join` (`BI-7`): the route's answer for two marts. A refusal that is
/// ours (unknown mart, bad name) is told in its words; a database failure is
/// the fixed message the route would give, never the database's text.
pub(super) async fn suggest_join(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let (left, right) = (arg_str(args, "left"), arg_str(args, "right"));
    match crate::routes::dashboard::join_suggestions_value(ch, Some(&left), Some(&right)).await {
        Ok(body) => body,
        Err(
            lakehouse_core::ApiError::BadRequest(sentence)
            | lakehouse_core::ApiError::NotFound(sentence),
        ) => json!({ "error": sentence }),
        Err(_) => json!({ "error": "the tables' columns could not be read right now" }),
    }
}

/// `list_sql_sources`: every dashboard SQL source with its columns split
/// into dimensions and measures, the same split `describe_mart` uses. A
/// `ClickHouse` failure is a fixed message, never its text.
pub(super) async fn list_sql_sources(ch: &ChClient) -> Value {
    match lakehouse_bi::sources::list_sources(ch).await {
        Ok(sources) => {
            let out: Vec<Value> = sources
                .iter()
                .map(|s| {
                    let (measures, dimensions): (Vec<_>, Vec<_>) = s
                        .columns
                        .iter()
                        .partition(|c| crate::routes::support::is_numeric_type(&c.ty));
                    // BI-9: each column's kind, so the assistant can tell which
                    // dimensions take a `grain` (date, datetime).
                    let columns: Vec<Value> = s
                        .columns
                        .iter()
                        .map(|c| {
                            json!({
                                "name": c.name,
                                "kind": lakehouse_bi::filters::ColumnKind::from_clickhouse_type(&c.ty),
                            })
                        })
                        .collect();
                    json!({
                        "id": s.id, "title": s.title,
                        "dimensions": dimensions.iter().map(|c| &c.name).collect::<Vec<_>>(),
                        "measures": measures.iter().map(|c| &c.name).collect::<Vec<_>>(),
                        "columns": columns,
                    })
                })
                .collect();
            json!({ "total": out.len(), "sources": out })
        }
        Err(err) => {
            tracing::warn!(%err, "list_sql_sources failed");
            json!({ "error": "SQL sources tidak bisa dibaca saat ini." })
        }
    }
}

pub(super) async fn delete_chart(
    ch: &ChClient,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    // BI-18 part C, part 2: a chart the person cannot see is answered like one
    // that does not exist, and nothing is deleted. A deleted chart goes to the
    // trash, restorable for 30 days.
    let board = match require_editable_chart(ch, &id, &viewer_for(principal)).await {
        Ok(board) => board,
        Err(gate) => return refused(gate),
    };
    let by = principal.map_or("Assistant", |p| p.display_name.as_str());
    match store::delete_chart(ch, &id, by).await {
        Ok(()) => {
            drop_tile(ch, &board, &id).await;
            snapshot_as(ch, &board, principal).await;
            json!({ "deleted": true, "id": id })
        }
        Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// SEC-11: the assistant's dashboard tools never relay `ClickHouse`'s
    /// text; they return a fixed message and a reference id.
    #[tokio::test]
    async fn a_failing_dashboard_tool_reports_a_fixed_message_and_a_reference() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(
                "Code: 60. DB::Exception: Table planted-marker-table-x doesn't exist (version 0.0.0)",
            ))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        for result in [
            list_boards(&ch, None).await,
            list_charts(&ch, None).await,
            suggest_dashboard(&ch).await,
            delete_chart(&ch, None, &{
                let mut a = Map::new();
                a.insert("id".to_owned(), json!("c1"));
                a
            })
            .await,
        ] {
            let text = result.to_string();
            assert!(!text.contains("planted-marker"), "leaked: {text}");
            assert!(!text.contains("version 0.0.0"), "{text}");
            assert!(result["errorId"].is_string(), "{text}");
        }
    }

    // ── BI-18 part C, part 2: the personal space and the trash ──────────────

    use lakehouse_auth::{PermissionSet, PrincipalId};
    use wiremock::matchers::body_string_contains;

    fn person(n: u128) -> Principal {
        Principal {
            id: PrincipalId::User(uuid::Uuid::from_u128(n)),
            tenant_ids: Vec::new(),
            display_name: "Rina".to_owned(),
            permissions: PermissionSet::parse("dashboard:read, dashboard:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    async fn rows(server: &MockServer, needle: &str, data: Vec<Value>) {
        Mock::given(method("POST"))
            .and(body_string_contains(needle))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": data, "rows": data.len() })),
            )
            .mount(server)
            .await;
    }

    /// A rule that answers before the catch-all: the catch-all is mounted
    /// first (in [`mart_server`]) and a rule of the same priority is decided
    /// by insertion order, so a later rule needs the higher priority to be
    /// the one that answers.
    async fn rows_first(server: &MockServer, needle: &str, data: Vec<Value>) {
        Mock::given(method("POST"))
            .and(body_string_contains(needle))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": data, "rows": data.len() })),
            )
            .with_priority(1)
            .mount(server)
            .await;
    }

    async fn rest(server: &MockServer) {
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(server)
            .await;
    }

    async fn sent(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }

    fn writes(sent: &[String]) -> Vec<&String> {
        sent.iter()
            .filter(|s| s.trim_start().starts_with("INSERT"))
            .collect()
    }

    /// The assistant reads as the person it acts for; with nobody attached it
    /// reads as nobody in particular, which sees no personal space.
    #[tokio::test]
    async fn the_assistant_lists_boards_and_charts_as_the_person_it_acts_for() {
        let server = MockServer::start().await;
        rest(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let me = person(7);
        let id = uuid::Uuid::from_u128(7).to_string();
        let _ = list_boards(&ch, Some(&me)).await;
        let _ = list_charts(&ch, Some(&me)).await;
        let _ = list_boards(&ch, None).await;
        let all = sent(&server).await;
        let boards: Vec<&String> = all
            .iter()
            .filter(|s| s.contains("FROM console.bi_board FINAL WHERE is_deleted = 0"))
            .collect();
        // `list_charts` reads the boards too, for the tabs of the charts, so
        // the person's read comes twice and the anonymous one last.
        assert_eq!(boards.len(), 3, "{boards:#?}");
        assert!(
            boards[0].contains(&format!("(owner_id = '' OR owner_id = '{id}')")),
            "{}",
            boards[0]
        );
        assert!(
            boards[1].contains(&format!("(owner_id = '' OR owner_id = '{id}')")),
            "{}",
            boards[1]
        );
        assert!(
            boards[2].contains("(owner_id = '' OR owner_id = '')"),
            "{}",
            boards[2]
        );
        let charts: Vec<&String> = all
            .iter()
            .filter(|s| s.contains("FROM console.bi_chart FINAL"))
            .collect();
        assert!(
            charts[0].contains(&format!("owner_id != '' AND owner_id != '{id}'")),
            "{}",
            charts[0]
        );
        assert!(charts[0].contains("is_deleted = 2"), "{}", charts[0]);
    }

    /// A chart the person cannot see is answered like one that does not exist,
    /// and nothing is deleted.
    #[tokio::test]
    async fn the_assistant_cannot_delete_a_chart_on_somebody_elses_private_board() {
        let server = MockServer::start().await;
        rows(
            &server,
            "SELECT is_deleted, board FROM console.bi_chart",
            vec![json!({ "is_deleted": 0, "board": "b_draft" })],
        )
        .await;
        rows(
            &server,
            "SELECT is_deleted, owner_id FROM console.bi_board",
            vec![json!({ "is_deleted": 0, "owner_id": "someone-else" })],
        )
        .await;
        rest(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let mut args = Map::new();
        args.insert("id".to_owned(), json!("c_private"));
        let reply = delete_chart(&ch, Some(&person(7)), &args).await;
        assert_eq!(reply["error"], json!("this tile does not exist"), "{reply}");
        assert!(reply.get("deleted").is_none(), "{reply}");
        let all = sent(&server).await;
        assert!(writes(&all).is_empty(), "{:?}", writes(&all));
    }

    /// A chart that is not there at all is refused like one the person may not
    /// see: the same sentence, and no "deleted" that says the chart is gone.
    #[tokio::test]
    async fn the_assistant_refuses_to_delete_a_chart_that_is_not_there() {
        let server = MockServer::start().await;
        rows(
            &server,
            "SELECT is_deleted, board FROM console.bi_chart",
            Vec::new(),
        )
        .await;
        rest(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let mut args = Map::new();
        args.insert("id".to_owned(), json!("c_missing"));
        let reply = delete_chart(&ch, Some(&person(7)), &args).await;
        assert_eq!(reply["error"], json!("this tile does not exist"), "{reply}");
        assert!(reply.get("deleted").is_none(), "{reply}");
        assert!(writes(&sent(&server).await).is_empty());
    }

    #[tokio::test]
    async fn the_assistant_moves_a_chart_to_the_trash_under_the_persons_name() {
        let server = MockServer::start().await;
        rows(
            &server,
            "SELECT is_deleted, board FROM console.bi_chart",
            vec![json!({ "is_deleted": 0, "board": "default" })],
        )
        .await;
        rest(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let mut args = Map::new();
        args.insert("id".to_owned(), json!("c_mine"));
        let reply = delete_chart(&ch, Some(&person(7)), &args).await;
        assert_eq!(reply["deleted"], json!(true));
        let all = sent(&server).await;
        let copy = writes(&all)
            .into_iter()
            .find(|s| s.contains("INTO console.bi_chart"))
            .unwrap();
        assert!(copy.contains(", 2, now(), 'Rina', ''"), "{copy}");
    }

    #[tokio::test]
    async fn the_assistant_cannot_put_a_chart_on_a_dashboard_in_the_trash() {
        let server = MockServer::start().await;
        rows(
            &server,
            "SELECT is_deleted, owner_id FROM console.bi_board",
            vec![json!({ "is_deleted": 2, "owner_id": "" })],
        )
        .await;
        rest(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let mut args = Map::new();
        for (k, v) in [
            ("title", json!("t")),
            ("kind", json!("bar")),
            ("mart", json!("m")),
            ("dimension", json!("d")),
            ("measures", json!(["x"])),
            ("board", json!("b_gone")),
        ] {
            args.insert(k.to_owned(), v);
        }
        let reply = create_chart(
            &ch,
            Some(&person(7)),
            &args,
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["error"], json!("dashboard not found."));
        assert!(writes(&sent(&server).await).is_empty());
    }

    // ── BI-16 part B (AI-5): progress, histogram, bullet, image and web ─────

    /// A mart with a text and three number columns, and a console that takes
    /// every write.
    async fn mart_server() -> MockServer {
        let server = MockServer::start().await;
        rows(&server, "FROM system.tables", vec![json!({ "n": "1" })]).await;
        rows(
            &server,
            "FROM system.columns",
            vec![
                json!({ "name": "province", "type": "String" }),
                json!({ "name": "visitors", "type": "UInt64" }),
                json!({ "name": "target", "type": "UInt64" }),
                json!({ "name": "price", "type": "Float64" }),
            ],
        )
        .await;
        rest(&server).await;
        server
    }

    fn arguments(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(map) => map,
            other => panic!("not an object: {other}"),
        }
    }

    fn sites(list: &[&str]) -> lakehouse_bi::grain::TimeContext {
        lakehouse_bi::grain::TimeContext::default()
            .with_content_sites(list.iter().map(|s| (*s).to_owned()).collect())
    }

    /// The statement the tool ran to check the chart it saved.
    fn statements(all: &[String]) -> Vec<&String> {
        all.iter()
            .filter(|s| s.starts_with("SELECT") && s.contains("serving.mart_x"))
            .collect()
    }

    /// `PR #199 review M SHOULD-FIX BI-17 4` (decision M5): a new chart in a
    /// deployment with a company palette draws with it. A palette the model
    /// chose is kept.
    #[tokio::test]
    async fn a_new_chart_draws_with_the_company_palette_when_the_model_chooses_none() {
        let time = lakehouse_bi::grain::TimeContext::default()
            .with_company_palette(vec!["#112233".to_owned(), "#445566".to_owned()]);
        let server = mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let created = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "Visitors", "kind": "bar", "mart": "mart_x",
                "dimension": "province", "measures": ["visitors"],
            })),
            None,
            &time,
        )
        .await;
        assert_eq!(created["created"], json!(true), "{created}");
        let all = sent(&server).await;
        let stored = writes(&all);
        assert!(
            stored.iter().any(|w| w.contains("\"palette\":\"company\"")),
            "{stored:#?}"
        );
        // A palette the model chose is kept.
        let server = mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let created = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "Visitors", "kind": "bar", "mart": "mart_x",
                "dimension": "province", "measures": ["visitors"],
                "colors": { "palette": "muted" },
            })),
            None,
            &time,
        )
        .await;
        assert_eq!(created["created"], json!(true), "{created}");
        let all = sent(&server).await;
        let stored = writes(&all);
        assert!(
            stored.iter().any(|w| w.contains("\"palette\":\"muted\"")),
            "{stored:#?}"
        );
    }

    /// `PR #199 review O BLOCKER 2`: a kind that takes no colours must not be
    /// given the company palette — the style check refuses colours on a
    /// table, a KPI or a static tile, so the assistant could create none of
    /// them in a deployment with a company palette.
    #[tokio::test]
    async fn a_kind_that_takes_no_colours_is_created_even_with_a_company_palette() {
        let time = lakehouse_bi::grain::TimeContext::default()
            .with_company_palette(vec!["#112233".to_owned(), "#445566".to_owned()]);
        for (kind, extra) in [
            (
                "table",
                json!({ "mart": "mart_x", "dimension": "province", "measures": ["visitors"] }),
            ),
            (
                "kpi",
                json!({ "mart": "mart_x", "measures": ["visitors"], "aggregate": "sum" }),
            ),
            ("text", json!({ "text": "Hello" })),
        ] {
            let mut body = json!({ "title": "Tile", "kind": kind });
            let fields = body.as_object_mut().expect("an object");
            for (name, value) in extra.as_object().expect("an object") {
                fields.insert(name.clone(), value.clone());
            }
            let server = mart_server().await;
            let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
            let created = create_chart(&ch, None, &arguments(body), None, &time).await;
            assert_eq!(created["created"], json!(true), "{kind}: {created}");
            let all = sent(&server).await;
            let stored = writes(&all);
            assert!(
                !stored.iter().any(|w| w.contains("\"palette\":\"company\"")),
                "{kind}: a kind that takes no colours is given no palette: {stored:#?}"
            );
        }
    }

    /// AI-5: "Show a progress bar of visitors against a goal of 1,000,000".
    #[tokio::test]
    async fn the_request_for_a_progress_bar_against_a_goal_of_a_million_creates_one() {
        let server = mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "Visitors against 1,000,000", "kind": "progress", "mart": "mart_x",
                "measures": ["visitors"], "aggregate": "sum", "goal": 1_000_000,
            })),
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["created"], json!(true), "{reply}");
        assert_eq!(reply["kind"], json!("progress"));
        let all = sent(&server).await;
        assert_eq!(
            statements(&all)
                .first()
                .map(|s| s.trim_end_matches("\nFORMAT JSON")),
            Some("SELECT round(sum(visitors)) AS v FROM serving.mart_x")
        );
        let stored = writes(&all);
        assert!(
            stored.iter().any(|w| w.contains("goal")),
            "the goal is stored"
        );
        // A goal that is a second value, and a goal that is missing.
        let valued = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "Against target", "kind": "progress", "mart": "mart_x",
                "measures": ["visitors", "target"], "goal": { "measure": "target" },
            })),
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(valued["created"], json!(true), "{valued}");
        let none = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "No goal", "kind": "progress", "mart": "mart_x", "measures": ["visitors"],
            })),
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert!(
            none["error"].as_str().unwrap().contains("needs a goal"),
            "{none}"
        );
    }

    /// AI-5: "Show how hotel prices are spread" is a histogram of the price
    /// column; the same call with `histogram: {count: 10}` cuts it in ten.
    #[tokio::test]
    async fn the_request_to_show_how_prices_are_spread_creates_a_histogram() {
        let server = mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let time = lakehouse_bi::grain::TimeContext::default();
        let auto = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "How hotel prices are spread", "kind": "histogram", "mart": "mart_x",
                "dimension": "price",
            })),
            None,
            &time,
        )
        .await;
        assert_eq!(auto["created"], json!(true), "{auto}");
        let ten = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "In ten ranges", "kind": "histogram", "mart": "mart_x",
                "dimension": "price", "histogram": { "count": 10 },
            })),
            None,
            &time,
        )
        .await;
        assert_eq!(ten["created"], json!(true), "{ten}");
        let all = sent(&server).await;
        let ran = statements(&all);
        // BI-18 part C review fix (SHOULD-FIX) R21: automatic aims at the nice width
        // nearest 20 ranges, and a count of 10 is ten ranges, not "at most 10".
        assert!(ran[0].contains("log10((mx - mn) / 20)"), "{}", ran[0]);
        assert!(ran[1].contains("(mx - mn) / 10 AS raw"), "{}", ran[1]);
        assert!(ran.iter().all(|s| s.contains("count() AS __count")));
        // The same refusals as the console, in the same words.
        for (extra, sentence) in [
            (json!({ "histogram": { "count": 101 } }), "1 to 100 ranges"),
            (
                json!({ "histogram": { "count": 5, "width": 2 } }),
                "not both",
            ),
            (json!({ "dimension": "province" }), "not a number column"),
        ] {
            let mut input = json!({
                "title": "t", "kind": "histogram", "mart": "mart_x", "dimension": "price",
            });
            input
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let reply = create_chart(&ch, None, &arguments(input), None, &time).await;
            assert!(
                reply["error"].as_str().unwrap().contains(sentence),
                "{reply}"
            );
        }
    }

    #[tokio::test]
    async fn a_bullet_graph_per_province_with_bands_is_created_and_a_goal_on_a_bar_chart_is_refused()
     {
        let server = mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let time = lakehouse_bi::grain::TimeContext::default();
        let bullet = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "Visitors against target", "kind": "bullet", "mart": "mart_x",
                "dimension": "province", "measures": ["visitors", "target"],
                "goal": { "measure": "target" }, "bands": [50, 80, 100],
            })),
            None,
            &time,
        )
        .await;
        assert_eq!(bullet["created"], json!(true), "{bullet}");
        let bar = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "t", "kind": "bar", "mart": "mart_x", "dimension": "province",
                "measures": ["visitors"], "goal": 5,
            })),
            None,
            &time,
        )
        .await;
        assert_eq!(
            bar["error"],
            json!("goal is only for a progress bar and a bullet graph.")
        );
    }

    /// A call whose args do not read names the field to blame: serde's words
    /// say what was found and not where it was found, so the model has to
    /// know which of its arguments to fix. The bands of a bullet graph are
    /// numbers, and the model sent objects.
    #[tokio::test]
    async fn a_chart_call_with_bands_of_objects_names_the_field_to_fix() {
        let server = mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = create_chart(
            &ch,
            None,
            &arguments(json!({
                "title": "Visitors against target", "kind": "bullet", "mart": "mart_x",
                "dimension": "province", "measures": ["visitors", "target"],
                "goal": { "measure": "target" },
                "bands": [{ "value": 1000, "label": "Target" }],
            })),
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(
            reply["error"],
            json!("\"bands\": invalid type: map, expected f64"),
            "{reply}"
        );
    }

    /// Two fields of the wrong shape name none: there is nothing to single
    /// out, so the error is serde's own words, as it was before.
    #[tokio::test]
    async fn a_chart_call_with_two_fields_of_the_wrong_shape_names_none() {
        let server = mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let args = arguments(json!({
            "title": "Visitors against target", "kind": "bullet", "mart": "mart_x",
            "dimension": "province", "measures": ["visitors", "target"],
            "goal": { "measure": 5 },
            "bands": [{ "value": 1000, "label": "Target" }],
        }));
        let reply = create_chart(
            &ch,
            None,
            &args,
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        // The error is serde's own words, with no field's name in front.
        let serde_own = serde_json::from_value::<ChartInput>(Value::Object(args.clone()))
            .unwrap_err()
            .to_string();
        assert_eq!(reply["error"], json!(serde_own), "{reply}");
        assert!(
            !reply["error"].as_str().unwrap().contains("\"bands\""),
            "{reply}"
        );
    }

    /// The address of an image or a web page is checked against the
    /// administrator's list on the assistant's path as on the console's, with
    /// the console's sentence, and the address is not repeated.
    #[tokio::test]
    async fn an_image_on_a_site_that_is_not_allowed_is_refused_with_the_consoles_sentence() {
        let server = mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let image = |url: &str| {
            arguments(json!({ "title": "Logo", "kind": "image", "url": url, "alt": "The logo" }))
        };
        let allowed = sites(&["https://img.example"]);
        let created = create_chart(
            &ch,
            None,
            &image("https://img.example/logo.png"),
            None,
            &allowed,
        )
        .await;
        assert_eq!(created["created"], json!(true), "{created}");
        for url in [
            "https://other.example/logo.png",
            "http://img.example/logo.png",
            "https://img.example@evil.example/logo.png",
        ] {
            let refused = create_chart(&ch, None, &image(url), None, &allowed).await;
            assert_eq!(
                refused["error"],
                json!(lakehouse_bi::types_b::CONTENT_SITE_REFUSED),
                "{url}"
            );
            assert!(!refused.to_string().contains(url));
        }
        // With no site listed nothing can be created; update_chart is checked too.
        let none = create_chart(
            &ch,
            None,
            &image("https://img.example/logo.png"),
            None,
            &sites(&[]),
        )
        .await;
        assert_eq!(
            none["error"],
            json!(lakehouse_bi::types_b::CONTENT_SITE_REFUSED)
        );
        rows_first(
            &server,
            "SELECT is_deleted, board FROM console.bi_chart",
            vec![json!({ "is_deleted": 0, "board": "default" })],
        )
        .await;
        let mut update = image("https://other.example/logo.png");
        update.insert("id".to_owned(), json!("c_1"));
        let refused = update_chart(&ch, None, &update, &allowed).await;
        assert_eq!(
            refused["error"],
            json!(lakehouse_bi::types_b::CONTENT_SITE_REFUSED)
        );
        let web = create_chart(
            &ch,
            None,
            &arguments(
                json!({ "title": "Status", "kind": "web", "url": "https://img.example/status" }),
            ),
            None,
            &allowed,
        )
        .await;
        assert_eq!(web["created"], json!(true), "{web}");
    }

    // ── The chart tools say what they did ──────────────────────────────

    /// The document the store keeps for a chart: its spec and the definition
    /// it was built from.
    fn stored_document(def: Value) -> String {
        let mut document = json!({
            "spec": {
                "id": "c_1",
                "title": "Visitors",
                "kind": "bar",
                "mart": "mart_x",
                "sql": "SELECT province, sum(visitors) AS visitors FROM serving.mart_x GROUP BY province",
                "x": "province",
                "y": "visitors",
            },
            "hasYear": false,
        });
        document["def"] = def;
        document.to_string()
    }

    /// A server with one stored chart on the board `b_board`, over the mart
    /// `mart_x`: the chart's own row, the stored document and the board it is
    /// on.
    async fn stored_chart_server(def: Value) -> MockServer {
        let server = mart_server().await;
        rows_first(
            &server,
            "SELECT is_deleted, board FROM console.bi_chart",
            vec![json!({ "is_deleted": 0, "board": "b_board" })],
        )
        .await;
        rows_first(
            &server,
            "FROM console.bi_chart FINAL WHERE is_deleted = 0",
            vec![json!({
                "id": "c_1",
                "spec_json": stored_document(def),
                "board": "b_board",
                "created_by": "ai",
                "created_at": "2026-10-22 00:00:00",
            })],
        )
        .await;
        rows_first(
            &server,
            "SELECT is_deleted, owner_id FROM console.bi_board",
            vec![json!({ "is_deleted": 0, "owner_id": "" })],
        )
        .await;
        server
    }

    /// The call that renames a chart and names nothing else; `extra` adds the
    /// fields the call names besides the title.
    fn rename_call(extra: Value) -> Map<String, Value> {
        let mut args = arguments(json!({
            "id": "c_1",
            "title": "Visitors 2024",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
        }));
        for (name, value) in arguments(extra) {
            args.insert(name, value);
        }
        args
    }

    /// The row the store wrote for the chart.
    fn chart_row(all: &[String]) -> String {
        writes(all)
            .into_iter()
            .find(|s| s.contains("INTO console.bi_chart"))
            .unwrap_or_else(|| panic!("no chart row was written: {all:#?}"))
            .clone()
    }

    /// A stored chart with a number format and a palette, updated with a new
    /// title and no formatting and no board, keeps both and keeps its board.
    #[tokio::test]
    async fn an_update_that_names_no_formatting_keeps_the_formatting_and_the_board() {
        let server = stored_chart_server(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
            "board": "b_board",
            "format": { "style": "number", "decimals": 0 },
            "colors": { "palette": "muted" },
        }))
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            Some(&person(7)),
            &rename_call(json!({})),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        assert_eq!(reply["formattingKept"], json!(true), "{reply}");
        assert_eq!(reply["board"], json!("b_board"), "{reply}");
        let row = chart_row(&sent(&server).await);
        assert!(row.contains("\"palette\":\"muted\""), "{row}");
        assert!(row.contains("\"decimals\":0"), "{row}");
    }

    /// A call that clears a colour clears it and keeps the rest of the
    /// formatting.
    #[tokio::test]
    async fn an_update_that_clears_a_colour_keeps_the_number_format() {
        let server = stored_chart_server(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
            "board": "b_board",
            "format": { "style": "number", "decimals": 0 },
            "colors": { "palette": "muted" },
        }))
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            Some(&person(7)),
            &rename_call(json!({ "colors": null })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        let row = chart_row(&sent(&server).await);
        assert!(!row.contains("muted"), "{row}");
        assert!(row.contains("\"decimals\":0"), "{row}");
    }

    /// A call that changes the kind drops the formatting: a colour is for a
    /// chart, and the reply says the formatting was reset.
    #[tokio::test]
    async fn an_update_that_changes_the_kind_resets_the_formatting() {
        let server = stored_chart_server(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
            "board": "b_board",
            "format": { "style": "number", "decimals": 0 },
            "colors": { "palette": "muted" },
        }))
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            Some(&person(7)),
            &rename_call(json!({ "kind": "table" })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        assert_eq!(reply["formattingKept"], json!(false), "{reply}");
        assert!(reply["note"].is_string(), "{reply}");
        let row = chart_row(&sent(&server).await);
        assert!(!row.contains("muted"), "{row}");
        assert!(!row.contains("\"decimals\":0"), "{row}");
    }

    /// The card and the save agree: a partial update whose kept formatting no
    /// longer fits the call is refused by both, with the same sentence.
    #[tokio::test]
    async fn the_draft_check_and_the_update_refuse_a_partial_update_whose_kept_formatting_no_longer_fits()
     {
        let server = stored_chart_server(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
            "board": "b_board",
            "valueFormats": { "revenue": { "style": "number" } },
        }))
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let args = rename_call(json!({}));
        let time = lakehouse_bi::grain::TimeContext::default();
        let draft = check_chart_draft(&ch, &args, Some("c_1".to_owned()), &time).await;
        let saved = update_chart(&ch, Some(&person(7)), &args, &time).await;
        let draft = draft.expect("the draft is refused");
        assert_eq!(
            draft["error"],
            json!(
                "a value format names 'revenue', which the chart does not return; it returns: price, province, target, visitors."
            ),
            "{draft}"
        );
        assert_eq!(saved["error"], draft["error"], "{saved}");
    }

    /// A rename that names no mart keeps the chart's own mart: the assistant
    /// does not have to repeat it, and the reply says it was kept.
    #[tokio::test]
    async fn an_update_that_names_no_mart_keeps_the_stored_mart() {
        let server = stored_chart_server(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
            "board": "b_board",
        }))
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let mut args = rename_call(json!({}));
        args.remove("mart");
        let reply = update_chart(
            &ch,
            Some(&person(7)),
            &args,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        assert_eq!(reply["mart"], json!("mart_x"), "{reply}");
        assert_eq!(reply["sourceKept"], json!(true), "{reply}");
        let row = chart_row(&sent(&server).await);
        assert!(row.contains("mart_x"), "{row}");
    }

    /// The assistant's model sends `"mart": null` for "no mart". It reads as
    /// no mart, so the chart's own mart is kept and the reply says so.
    #[tokio::test]
    async fn an_update_that_sends_a_null_mart_keeps_the_stored_mart() {
        let server = stored_chart_server(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
            "board": "b_board",
        }))
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            Some(&person(7)),
            &rename_call(json!({ "mart": null })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        assert_eq!(reply["mart"], json!("mart_x"), "{reply}");
        assert_eq!(reply["sourceKept"], json!(true), "{reply}");
        let row = chart_row(&sent(&server).await);
        assert!(row.contains("mart_x"), "{row}");
    }

    /// A static kind takes no source: turning a mart chart into a text tile
    /// carries nothing over, and the reply does not say a source was kept.
    #[tokio::test]
    async fn an_update_that_turns_a_chart_into_text_carries_no_source() {
        let server = stored_chart_server(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
            "board": "b_board",
        }))
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let mut args = rename_call(json!({ "kind": "text", "text": "A note" }));
        args.remove("mart");
        let reply = update_chart(
            &ch,
            Some(&person(7)),
            &args,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        assert_eq!(reply["kind"], json!("text"), "{reply}");
        assert!(reply.get("sourceKept").is_none(), "{reply}");
        let row = chart_row(&sent(&server).await);
        assert!(row.contains("A note"), "{row}");
        assert!(!row.contains("mart_x"), "{row}");
    }

    /// A call that names a SQL source keeps it: the chart's own mart is not
    /// carried over, so the two are never named at once.
    #[tokio::test]
    async fn an_update_that_names_a_sql_source_does_not_carry_the_mart() {
        let server = stored_chart_server(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
            "board": "b_board",
        }))
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let mut args = rename_call(json!({ "sqlSource": "s_1" }));
        args.remove("mart");
        let reply = update_chart(
            &ch,
            Some(&person(7)),
            &args,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        let error = reply["error"].as_str().unwrap_or_default();
        assert!(
            !error.contains("choose either a mart or a SQL source, not both."),
            "{reply}"
        );
        assert!(error.contains("SQL source 's_1' not found."), "{reply}");
    }

    /// The card and the save agree: a rename that names no mart is checked
    /// with the mart the chart has, so it is offered for confirmation.
    #[tokio::test]
    async fn the_draft_check_of_a_rename_that_names_no_mart_passes() {
        let server = stored_chart_server(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
            "board": "b_board",
        }))
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let mut args = rename_call(json!({}));
        args.remove("mart");
        let draft = check_chart_draft(
            &ch,
            &args,
            Some("c_1".to_owned()),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert!(draft.is_none(), "{draft:?}");
    }

    /// A chart is listed with the board it is on and the tab it sits on: a
    /// tile that names no tab sits on the board's first one.
    #[tokio::test]
    async fn the_assistant_lists_a_chart_with_the_board_and_the_tab_it_sits_on() {
        let server = MockServer::start().await;
        rows_first(
            &server,
            "FROM console.bi_chart FINAL WHERE is_deleted = 0",
            vec![json!({
                "id": "c_1",
                "spec_json": stored_document(json!({
                    "title": "Visitors",
                    "kind": "bar",
                    "mart": "mart_x",
                    "dimension": "province",
                    "measures": ["visitors"],
                    "board": "b_board",
                })),
                "board": "b_board",
                "created_by": "ai",
                "created_at": "2026-10-22 00:00:00",
            })],
        )
        .await;
        rows_first(
            &server,
            "FROM console.bi_board FINAL WHERE is_deleted = 0",
            vec![json!({
                "id": "b_board",
                "name": "Sales",
                "owner_id": "",
                "tabs_json": "[{\"id\":\"tab_a\",\"title\":\"A\"},{\"id\":\"tab_b\",\"title\":\"B\"}]",
                "layout_json": "{\"c_1\":{\"x\":0,\"y\":0,\"w\":2,\"h\":2}}",
            })],
        )
        .await;
        rest(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = list_charts(&ch, Some(&person(7))).await;
        let chart = &reply["charts"][0];
        assert_eq!(chart["board"], json!("b_board"), "{reply}");
        assert_eq!(chart["tab"], json!("tab_a"), "{reply}");
    }

    /// A chart is listed with the formatting it holds and the names its
    /// series can take: the number format, the series colour and the
    /// reference line a call can name, and the name a series colour is
    /// keyed by.
    #[tokio::test]
    async fn the_assistant_lists_the_formatting_and_the_series_names_of_a_chart() {
        let server = stored_chart_server(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
            "board": "b_board",
            "format": { "style": "number", "decimals": 0 },
            "colors": { "series": { "visitors": "#112233" } },
            "referenceLines": [{ "value": 1000.0, "label": "Target" }],
        }))
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = list_charts(&ch, Some(&person(7))).await;
        let chart = &reply["charts"][0];
        assert_eq!(
            chart["style"]["format"],
            json!({ "style": "number", "decimals": 0 }),
            "{reply}"
        );
        assert_eq!(
            chart["style"]["colors"]["series"]["visitors"],
            json!("#112233"),
            "{reply}"
        );
        assert_eq!(
            chart["referenceLines"][0]["value"],
            json!(1000.0),
            "{reply}"
        );
        assert_eq!(
            chart["referenceLines"][0]["label"],
            json!("Target"),
            "{reply}"
        );
        assert_eq!(chart["seriesNames"], json!(["visitors"]), "{reply}");
    }

    /// A chart with no formatting lists an empty style and no reference
    /// lines: the fields a chart does not hold stay out of the entry, and
    /// the names its series can take are still listed.
    #[tokio::test]
    async fn the_assistant_lists_a_chart_with_no_formatting_as_an_empty_style() {
        let server = stored_chart_server(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
            "board": "b_board",
        }))
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = list_charts(&ch, Some(&person(7))).await;
        let chart = &reply["charts"][0];
        assert_eq!(chart["style"], json!({}), "{reply}");
        assert!(chart.get("referenceLines").is_none(), "{reply}");
        assert_eq!(chart["seriesNames"], json!(["visitors"]), "{reply}");
    }
}
