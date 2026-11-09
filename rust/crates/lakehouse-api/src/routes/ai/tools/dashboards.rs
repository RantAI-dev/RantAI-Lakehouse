//! Dashboard/chart tools: `create_chart`, `update_chart`, `delete_chart`,
//! `create_board`, `list_boards`, `list_charts`, `suggest_dashboard`.
//! Moved out of `ai.rs` unchanged (T0.1 registry refactor).

use lakehouse_auth::Principal;
use lakehouse_bi::access::Viewer;
use lakehouse_bi::layout::tab_of;
use lakehouse_bi::specs::{ChartKind, ChartSource};
use lakehouse_bi::store::{self, ChartInput};
use lakehouse_clickhouse::ChClient;
use serde_json::{Map, Value, json};

use super::arg_str;
use super::look::{FORMAT_FIELDS, check_layers_name_a_source, wrong_field};
use crate::routes::dashboard_organise::{
    Gate, drop_tile, place_new_chart, require_chart_board, require_editable_chart, snapshot_as,
};
use crate::routes::support::is_numeric_type;
use crate::upstream_error;

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

/// What a refused `compare` is followed by. Serde says which key is missing
/// but not where `compare` goes, and a model that sent `kind: "previous"` at
/// the top of the call, then `compare: {}`, was refused until its rounds ran
/// out. The names are those of `lakehouse_bi::tables::Compare`.
const COMPARE_EXAMPLE: &str = r#"`compare` sits inside the chart call, not at its top level, and a KPI that compares with its previous period reads: {"compare": {"kind": "previous", "dateColumn": "<date column>", "period": "month"}}"#;

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
                Some(field) => {
                    let sentence = format!("\"{field}\": {cause}");
                    if field == "compare" {
                        Err(format!("{sentence}. {COMPARE_EXAMPLE}"))
                    } else {
                        Err(sentence)
                    }
                }
                None => Err(cause),
            }
        }
    }
}

/// The longest digest, in characters.
const DIGEST_MAX: usize = 300;

/// What a chart holds, in one line the model can read back: the fields that
/// decide what the chart shows, and only those that are set. A model told
/// "saved" and nothing else describes the arguments it meant to send; this
/// line is what it must describe instead, so a top N, a bin or a filter that
/// is not listed was not made.
///
/// The fields are `dimension`, `breakdown`, `measures` (names), `sort`,
/// `limit`, `bin`, `grain`, and the number of `filters` and `joins`.
/// `limit` is the exception to "set only": beside a sort or a breakdown it
/// reads `none (default applies)` when the chart sets none, because a missing
/// limit there is the difference between a top N and the builder's own cap,
/// and the cap depends on the kind, so no number is named. A limit that is
/// set also says what it counts: the latest buckets of a truncation grain
/// ordered by bucket, otherwise the dimension's values, and that a breakdown
/// is not limited; a date part in its own order and a chart without a
/// dimension (a pivot) say less, because there the limit counts nothing the
/// digest can name. The line is cut to [`DIGEST_MAX`] characters, the last one
/// `…`.
///
/// It reads the definition the save is built from, not the one it stores: the
/// store fills every chart's `limit` with the one it built with, which would
/// hide whether the call set one. A create draft and its saved reply pass the
/// same `ChartInput`, so one chart gives one line. An update gets its line
/// only in its saved reply, where the input is the stored chart merged with
/// the call, read as the caller; its draft carries none.
pub(super) fn definition_digest(def: &ChartInput) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !def.dimension.is_empty() {
        parts.push(format!("dimension: {}", def.dimension));
    }
    let breakdown = def.breakdown.as_deref().filter(|b| !b.is_empty());
    if let Some(breakdown) = breakdown {
        parts.push(format!("breakdown: {breakdown}"));
    }
    if !def.measures.is_empty() {
        parts.push(format!("measures: {}", def.measures.join(", ")));
    }
    if let Some(sort) = &def.shape.sort {
        parts.push(format!("sort: {} {}", sort.by, sort.dir));
    }
    // What the limit counts is not what a reader assumes: the server applies
    // it to the dimension's values and never to a breakdown, and with a
    // truncation grain ordered by bucket it keeps the latest buckets. A bare
    // `limit: 10` beside a breakdown was read as "10 of the breakdown"
    // (BI-6-06 of the QA of 2026-11-07).
    //
    // The rules are `grain::cuts_to_latest` and `grain::effective_limit`, fed
    // the order the query builder feeds them. The builder also leaves a pivot
    // out of "the latest buckets" (it has no dimension, and its grain applies
    // to its first date field), so a chart without a dimension makes no claim
    // about what its limit counts. One gap stays: the builder resolves the
    // grain against the column type (`grain::resolve`), which a synchronous
    // digest cannot do, so this reads only the grain the chart names.
    let counted = match &def.limit {
        Some(lakehouse_bi::params::Limit::Fixed(n)) => Some((n.to_string(), Some(*n))),
        Some(lakehouse_bi::params::Limit::Param(p)) => {
            Some((format!("parameter {}", p.param), None))
        }
        None => None,
    };
    if let Some((limit, number)) = counted {
        use lakehouse_bi::grain::{Grain, cuts_to_latest, effective_limit};
        let order = def.effective_order();
        let grain = def
            .grain
            .as_deref()
            .and_then(Grain::parse)
            .filter(|_| !def.dimension.is_empty());
        let mut part = match grain {
            Some(grain) if cuts_to_latest(grain, &order) => format!(
                "limit: {limit} (the latest {}{} buckets of {})",
                number.map(|n| format!("{n} ")).unwrap_or_default(),
                grain.as_str(),
                def.dimension
            ),
            // A date part in its own order returns its whole range: the
            // builder swaps the limit for the range, so asking what it would
            // do with the largest limit tells whether this one is dropped.
            Some(grain) if effective_limit(grain, &order, u32::MAX) != u32::MAX => format!(
                "limit: {limit} (not applied: {} returns its whole range)",
                grain.as_str()
            ),
            _ if def.dimension.is_empty() => format!("limit: {limit}"),
            _ => format!("limit: {limit} {} values", def.dimension),
        };
        if breakdown.is_some() {
            part.push_str("; the breakdown is not limited");
        }
        parts.push(part);
    } else if def.shape.sort.is_some() || breakdown.is_some() {
        parts.push("limit: none (default applies)".to_owned());
    }
    if let Some(bin) = &def.shape.bin {
        parts.push(match (bin.mode.as_str(), bin.width, bin.count) {
            ("width", Some(width), _) => format!("bin: width {width}"),
            ("count", _, Some(count)) => format!("bin: {count} bins"),
            (mode, _, _) => format!("bin: {mode}"),
        });
    }
    if let Some(grain) = def.grain.as_deref().filter(|g| !g.is_empty()) {
        parts.push(format!("grain: {grain}"));
    }
    let filters = def.shape.filters.as_deref().map_or(0, <[_]>::len);
    if filters > 0 {
        parts.push(format!("filters: {filters}"));
    }
    let joins = def.shape.joins_list().len();
    if joins > 0 {
        parts.push(format!("joins: {joins}"));
    }
    let digest = parts.join("; ");
    if digest.chars().count() <= DIGEST_MAX {
        return digest;
    }
    let mut cut: String = digest.chars().take(DIGEST_MAX - 1).collect();
    cut.push('…');
    cut
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

/// The fields `args_with_kept_fields` leaves alone because each has its own
/// rule: `title` and `kind` are required in every call, `board` is kept by
/// `update_chart` itself, and `mart` and `sqlSource` go through
/// `args_with_kept_source`.
const NOT_CARRIED: [&str; 5] = ["title", "kind", "board", "mart", "sqlSource"];

/// The only fields a kind without data carries, and the kinds that take each.
///
/// A note, a heading, a divider, a section, an image and a web card read no
/// data, so the dimension, measures, filters and the rest are never carried
/// into one: the build refuses them. What the tile shows is its own content,
/// which a rename must keep: `text` for a note, a heading and a section (a
/// divider drops it), `url` for an image and a web card, `alt` for an image,
/// and `span` for all of them. Each kind lists only the fields it accepts, so
/// a carried field never makes the build refuse the rename.
const STATIC_CONTENT: [(&str, &[ChartKind]); 4] = [
    (
        "text",
        &[ChartKind::Text, ChartKind::Heading, ChartKind::Section],
    ),
    ("url", &[ChartKind::Image, ChartKind::Web]),
    ("alt", &[ChartKind::Image]),
    (
        "span",
        &[
            ChartKind::Text,
            ChartKind::Heading,
            ChartKind::Divider,
            ChartKind::Section,
            ChartKind::Image,
            ChartKind::Web,
        ],
    ),
];

/// Whether `field` is content the tile of this static `kind` carries.
fn is_static_content(field: &str, kind: ChartKind) -> bool {
    STATIC_CONTENT
        .iter()
        .any(|(name, kinds)| *name == field && kinds.contains(&kind))
}

/// The fields only a Gantt chart takes: the style and type checks refuse them
/// on every other kind, so they are carried only into a Gantt chart.
const GANTT_FIELDS: [&str; 4] = ["start", "end", "progress", "todayLine"];

/// The kinds that take a stored field, so a kind change leaves out a field
/// the new kind refuses.
///
/// A kind that is not listed refuses the field. The rules are the checks that
/// refuse the field, not rules of their own: a pivot and a table in rows mode
/// name their columns in their own fields, not in `dimension`, `measures` or
/// `breakdown` (`lakehouse-bi/src/store.rs`, `check_measures`), and the
/// `BI-16` fields are only for the kinds that read them
/// (`lakehouse-bi/src/tables.rs`, `only_for`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Takes {
    /// The kind takes the field.
    Kind(ChartKind),
    /// A table in rows mode takes the field.
    RowsTable,
    /// Every kind but a pivot takes the field. A table in rows mode takes
    /// none of it: it names its columns in its own fields.
    AllButPivot,
}

/// The stored fields and the kinds that take each.
///
/// AI-5 AC1 (2026-11-08): a call that changes the kind leaves out a stored
/// field the new kind refuses, so the model need not clear it with `null`.
const TAKEN_BY_KIND: [(&str, &[Takes]); 13] = [
    // A pivot and a table in rows mode name their columns in their own fields
    // (`lakehouse-bi/src/store.rs`, `check_measures`).
    ("dimension", &[Takes::AllButPivot]),
    ("measures", &[Takes::AllButPivot]),
    ("breakdown", &[Takes::AllButPivot]),
    // The pivot's own fields (`lakehouse-bi/src/tables.rs`, `only_for`).
    ("rows", &[Takes::Kind(ChartKind::Pivot)]),
    ("values", &[Takes::Kind(ChartKind::Pivot)]),
    ("totals", &[Takes::Kind(ChartKind::Pivot)]),
    // The columns of a table in rows mode and of a pivot (`only_for`).
    (
        "columns",
        &[Takes::Kind(ChartKind::Pivot), Takes::RowsTable],
    ),
    // The sort of a table in rows mode (`only_for`).
    ("sortColumn", &[Takes::RowsTable]),
    ("sortDir", &[Takes::RowsTable]),
    // The fields of a table (`only_for`).
    ("tableMode", &[Takes::Kind(ChartKind::Table)]),
    (
        "columnSettings",
        &[Takes::Kind(ChartKind::Table), Takes::Kind(ChartKind::Pivot)],
    ),
    // The comparison of a KPI (`only_for`).
    ("compare", &[Takes::Kind(ChartKind::Kpi)]),
    ("goodDirection", &[Takes::Kind(ChartKind::Kpi)]),
];

/// Whether the chart of this kind takes the stored field.
///
/// A field no rule names is taken by every kind: only the fields the checks
/// refuse are listed.
fn kind_takes(field: &str, kind: ChartKind, rows_table: bool) -> bool {
    let Some((_, takers)) = TAKEN_BY_KIND.iter().find(|(name, _)| *name == field) else {
        return true;
    };
    takers.iter().any(|taker| match taker {
        Takes::Kind(k) => *k == kind,
        Takes::RowsTable => rows_table,
        Takes::AllButPivot => kind != ChartKind::Pivot && !rows_table,
    })
}

/// Whether the chart the call describes is a table in rows mode.
///
/// The mode is the one the call names, or the stored one the call leaves out:
/// a table in rows mode names its columns in `columns`, and the checks treat
/// it as a table of its own.
fn is_rows_table(args: &Map<String, Value>, call: &ChartInput, stored: &ChartInput) -> bool {
    if call.kind != ChartKind::Table {
        return false;
    }
    let mode = match args.get("tableMode") {
        Some(Value::String(text)) => Some(text.as_str()),
        // A `null` clears the mode: the table is the grouped one.
        Some(_) => None,
        None => stored.tables.table_mode.as_deref(),
    };
    mode == Some("rows")
}

/// The args without the keys the call sent as `null`.
///
/// `null` means "clear this field", so the key must behave as if it were left
/// out. Only the fields that are an `Option` accept `null` when the args are
/// parsed; a map, a list or a column slot would refuse it.
fn without_nulls(args: &Map<String, Value>) -> Map<String, Value> {
    args.iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// Whether a serialised field holds nothing. A chart serialises an empty
/// dimension as `""` and empty measures as `[]`, which are the values of a
/// field the chart never had.
fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(fields) => fields.is_empty(),
        _ => false,
    }
}

/// The call's args with every other field of the stored chart that the call
/// does not name filled in: the dimension, measures, breakdown, filters, sort
/// and the rest, so a rename that sends only `id`, `title` and `kind` does not
/// empty the chart.
///
/// A key the call sent as `null` is named, so it is not carried over; the
/// caller removes it afterwards (see [`without_nulls`]). Left out because each
/// has its own rule: [`NOT_CARRIED`] and the formatting. A kind that reads no
/// data (a note, a heading, an image) carries only its own content
/// ([`STATIC_CONTENT`]), and the Gantt fields are carried only into a Gantt
/// chart. `text` is carried only into a kind that shows it: a data kind drops
/// it when it builds the chart, so naming it as kept would be wrong.
///
/// A call that changes the kind leaves out a stored field the new kind
/// refuses ([`TAKEN_BY_KIND`]): a bar turned into a pivot keeps no dimension,
/// no measures and no breakdown, so the build has nothing to refuse (AI-5
/// AC1, 2026-11-08). The fields the call names itself are sent as given, so a
/// call that asks for a refused field is still refused by the build.
///
/// The second value is the sorted names of the fields carried over.
fn args_with_kept_fields(
    args: &Map<String, Value>,
    call: &ChartInput,
    stored: &ChartInput,
) -> (Map<String, Value>, Vec<String>) {
    let mut merged = args.clone();
    // A call that changes the kind takes other fields: a stored field the new
    // kind refuses is left out (AI-5 AC1, 2026-11-08).
    let kind_changed = stored.kind != call.kind;
    let rows_table = is_rows_table(args, call, stored);
    let Ok(stored) = serde_json::to_value(stored) else {
        return (merged, Vec::new());
    };
    let Some(fields) = stored.as_object() else {
        return (merged, Vec::new());
    };
    let mut carried = Vec::new();
    for (key, value) in fields {
        if NOT_CARRIED.contains(&key.as_str())
            || FORMAT_FIELDS.contains(&key.as_str())
            || merged.contains_key(key)
            || is_blank(value)
            || (call.kind.is_static() && !is_static_content(key, call.kind))
            || (key == "text" && !call.kind.is_static())
            || (GANTT_FIELDS.contains(&key.as_str()) && call.kind != ChartKind::Gantt)
            || (kind_changed && !kind_takes(key, call.kind, rows_table))
        {
            continue;
        }
        merged.insert(key.clone(), value.clone());
        carried.push(key.clone());
    }
    carried.sort();
    (merged, carried)
}

/// What a call carries over from the chart it changes: the call's own
/// arguments with the formatting, the data source and the other fields the
/// call does not name filled in, and what was kept.
struct KeptFormatting {
    /// The stored formatting was kept: the call does not change the kind.
    kept: bool,
    /// The call's arguments, with everything kept filled in and the keys the
    /// call sent as `null` removed.
    args: Map<String, Value>,
    /// The stored data source was kept: the call names no source of its own.
    source_kept: bool,
    /// The names of the other stored fields the call did not name and that
    /// were carried over, sorted.
    fields_kept: Vec<String>,
}

/// The formatting, the data source and the other fields a call does not name,
/// carried over from the stored chart.
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
    // The call is read without its `null` keys: `null` clears a field, so
    // a field of any type may be sent as `null` (`title` and `kind` then
    // read as missing, and the call is refused).
    let call = match parse_chart_input(&without_nulls(args)) {
        Ok(i) => i,
        Err(e) => return Err(json!({ "error": e })),
    };
    let Some(stored) = stored else {
        // A call that names a chart is read as an update even when the chart
        // is gone, so its `null` keys are removed as for a chart found. A
        // call with no id is a draft of `create_chart` and stays as it came.
        return Ok(KeptFormatting {
            kept: true,
            args: if id.is_some() {
                without_nulls(args)
            } else {
                args.clone()
            },
            source_kept: false,
            fields_kept: Vec::new(),
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
    // Every other field the call leaves out is carried over too, also when the
    // kind changes, except the ones the new kind refuses (AI-5 AC1,
    // 2026-11-08). The `null` keys stay in `args` until here so that they
    // count as named and are not carried; they are removed last.
    let (args, fields_kept) = args_with_kept_fields(&args, &call, &stored.def);
    Ok(KeptFormatting {
        kept,
        args: without_nulls(&args),
        source_kept,
        fields_kept,
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
    viewer: &Viewer,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
    id: Option<String>,
    time: &lakehouse_bi::grain::TimeContext,
) -> Value {
    let PreparedChart { input, .. } = match prepare_chart(ch, viewer, None, args, time).await {
        Ok(prepared) => prepared,
        Err(reply) => return reply,
    };
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
                "definition": definition_digest(&input),
                "url": "/dashboards",
                "note": "Chart tersimpan & langsung tampil di halaman Dashboards.",
                })
            }
            Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
        },
        Err(err) => upstream_error::bi(&upstream_error::DATABASE, &err).to_json(),
    }
}

/// What `create_chart` and `update_chart` build before the chart is
/// stored. The fields after `input` describe the stored chart an update
/// changes. On a create they hold neutral values (an empty old board,
/// `formatting_kept` true, nothing kept) that the create path ignores.
struct PreparedChart {
    /// The definition the chart is built from and the digest describes.
    input: ChartInput,
    /// The board the chart is on now (an update's).
    old_board: String,
    /// The stored formatting was kept.
    formatting_kept: bool,
    /// The stored data source was kept.
    source_kept: bool,
    /// The names of the stored fields carried over, sorted.
    fields_kept: Vec<String>,
}

/// Every check the two chart tools run before `store::spec_from_input`, in
/// one place, so the save and the draft check (`check_chart_draft`) cannot
/// drift apart (`PR #391 review`, `PR #402 review`: the draft ran fewer checks
/// than the save and named the columns of a SQL source the caller cannot see).
/// `id` is `None` for a create and `Some` for an update. The order is the
/// save's: for an update the chart's own gate first, then the carry-over, then
/// the board, then the sources the caller may see.
///
/// `viewer` must be the caller's own (`scope_for_write`).
async fn prepare_chart(
    ch: &ChClient,
    viewer: &Viewer,
    id: Option<&str>,
    args: &Map<String, Value>,
    time: &lakehouse_bi::grain::TimeContext,
) -> Result<PreparedChart, Value> {
    let Some(id) = id else {
        let mut input = parse_chart_input(args).map_err(|e| json!({ "error": e }))?;
        check_layers_name_a_source(&input).map_err(|sentence| json!({ "error": sentence }))?;
        // BI-17 part A, decision 6 (`PR #199 review M SHOULD-FIX BI-17 4`): a new
        // chart in a deployment with a company palette draws with it. A palette
        // the model chose is kept, and a chart that already exists keeps what it
        // has (that is an update, which does not come here).
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
        require_chart_board(ch, input.board.as_deref(), viewer)
            .await
            .map_err(refused)?;
        // BI-19 B2 re-review S4-b: a source in a folder the caller cannot see is the
        // unknown-source answer, as for the console's create.
        store::ensure_sources_visible(ch, &input, viewer, &[])
            .await
            .map_err(|err| upstream_error::bi(&upstream_error::DATABASE, &err).to_json())?;
        return Ok(PreparedChart {
            input,
            old_board: String::new(),
            formatting_kept: true,
            source_kept: false,
            fields_kept: Vec::new(),
        });
    };
    if id.is_empty() {
        return Err(json!({ "error": "id is required" }));
    }
    // The gate comes before the args are parsed: the carry-over below needs
    // the chart's stored definition and the board it is on.
    // BI-18 part C, part 2: a chart in the trash or on somebody else's
    // personal board cannot be edited, nor moved onto such a board.
    let old_board = require_editable_chart(ch, id, viewer)
        .await
        .map_err(refused)?;
    // The stored chart, read as the caller: the gate above has refused a
    // chart the caller may not see, so this reads only what the caller may.
    // The read is boxed, so this future does not carry every chart on the
    // board.
    let kept = Box::pin(kept_formatting(ch, viewer, Some(id), args)).await?;
    let mut input = parse_chart_input(&kept.args).map_err(|e| json!({ "error": e }))?;
    check_layers_name_a_source(&input).map_err(|sentence| json!({ "error": sentence }))?;
    require_chart_board(ch, input.board.as_deref(), viewer)
        .await
        .map_err(refused)?;
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
    // BI-19 B2 re-review S4-b: as the console's update: a source the chart does
    // not already read must be one the caller may see.
    let already: Vec<String> = match store::list_stored_charts(ch, viewer).await {
        Ok(all) => all
            .into_iter()
            .find(|c| c.spec.id == id)
            .map(|c| store::sql_sources_named(&c.def))
            .unwrap_or_default(),
        Err(err) => {
            return Err(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json());
        }
    };
    store::ensure_sources_visible(ch, &input, viewer, &already)
        .await
        .map_err(|err| upstream_error::bi(&upstream_error::DATABASE, &err).to_json())?;
    Ok(PreparedChart {
        input,
        old_board,
        formatting_kept: kept.kept,
        source_kept: kept.source_kept,
        fields_kept: kept.fields_kept,
    })
}

/// Checks a chart draft before the gate offers it: a draft that cannot be
/// built must not be offered as a card the person can confirm. It runs the
/// checks of the save, the one [`prepare_chart`] and then
/// `store::spec_from_input`, and stops there, so it writes nothing and is
/// refused with the reply the save gives.
///
/// `id` is `None` for a create and `Some` for an update; an update with no
/// `id` is refused here, as the tool refuses it. `viewer` must be the
/// caller's own (`scope_for_write`, as the tool builds it): the draft reads
/// the stored chart and the sources as the caller, so it says nothing about
/// a chart, a dashboard or a SQL source the caller cannot see. The level
/// checks (`edit` on the dashboards) are the dispatcher's and are not part
/// of this function: `check_draft` runs them first.
///
/// A draft that builds returns the digest of the definition the save would
/// store, for a create the call's own and for an update the stored chart
/// merged with the call.
///
/// # Errors
///
/// The error the tool would return when the draft cannot be built.
pub(super) async fn check_chart_draft(
    ch: &ChClient,
    viewer: &Viewer,
    args: &Map<String, Value>,
    id: Option<String>,
    time: &lakehouse_bi::grain::TimeContext,
) -> Result<String, Value> {
    let PreparedChart { input, .. } = prepare_chart(ch, viewer, id.as_deref(), args, time).await?;
    match store::spec_from_input(ch, &input, ChartSource::Ai, "ai", id, time).await {
        Ok(_) => Ok(definition_digest(&input)),
        Err(err) => Err(upstream_error::bi(&upstream_error::DATABASE, &err).to_json()),
    }
}

pub(super) async fn update_chart(
    ch: &ChClient,
    viewer: &Viewer,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
    time: &lakehouse_bi::grain::TimeContext,
) -> Value {
    let id = arg_str(args, "id");
    let PreparedChart {
        input,
        old_board,
        formatting_kept,
        source_kept,
        fields_kept,
    } = match prepare_chart(ch, viewer, Some(id.as_str()), args, time).await {
        Ok(prepared) => prepared,
        Err(reply) => return reply,
    };
    match store::spec_from_input(ch, &input, ChartSource::Ai, "ai", Some(id), time).await {
        Ok(spec) => {
            // The new row takes a new version, which sorts the chart last on
            // a board where its tile has no stored box. When the chart stays
            // on its board, the board is boxed first so the tile stays where
            // it is, and a failure here stops the update before the chart row
            // is written. A chart that leaves needs no freeze: the order of
            // the tiles that stay does not change, and a box left behind
            // would be a hole the page never fills. A chart arriving on
            // another board is a new tile there and goes last.
            if store::canonical_board_id(&old_board) == store::canonical_board_id(&spec.board)
                && let Err(err) = store::freeze_board_layout(ch, &old_board).await
            {
                return upstream_error::bi(&upstream_error::DATABASE, &err).to_json();
            }
            match store::insert_chart(ch, &spec).await {
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
                        "definition": definition_digest(&input),
                        "formattingKept": formatting_kept,
                    });
                    if source_kept {
                        reply["sourceKept"] = json!(true);
                    }
                    if !fields_kept.is_empty() {
                        reply["fieldsKept"] = json!(fields_kept);
                    }
                    if !formatting_kept {
                        reply["note"] = json!(
                            "the chart's kind changed, so its formatting was reset; send the formatting again if the new kind takes it."
                        );
                    }
                    reply
                }
                Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
            }
        }
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

/// `viewer` is the caller's resolved one (`super::viewer_for`, `BI-19` B1 review R1).
pub(super) async fn list_boards(ch: &ChClient, viewer: &Viewer) -> Value {
    match store::list_boards(ch, viewer).await {
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

/// `viewer` is the caller's resolved one (`super::viewer_for`, `BI-19` B1 review R1).
pub(super) async fn list_charts(ch: &ChClient, viewer: &Viewer) -> Value {
    let charts = match store::list_stored_charts(ch, viewer).await {
        Ok(charts) => charts,
        Err(err) => return upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    };
    // The boards, for the tab each chart sits on. A board the viewer may not
    // see is not in the list, so its charts are listed without a tab.
    let boards = match store::list_boards(ch, viewer).await {
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
        // value format are stored under. A chart with a breakdown draws one
        // series per data value, so the name it draws is the measure and
        // not a series.
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
pub(super) async fn chart_sources(
    ch: &ChClient,
    viewer: &Viewer,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    if name == "suggest_join" {
        suggest_join(ch, args).await
    } else {
        list_sql_sources(ch, viewer).await
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
///
/// `BI-19` B2 review S4 (owner decision): a source filed in a folder is listed
/// only to a caller who may see that folder, as in the console.
pub(super) async fn list_sql_sources(ch: &ChClient, viewer: &Viewer) -> Value {
    match lakehouse_bi::sources::list_sources(ch).await {
        Ok(sources) => {
            let out: Vec<Value> = sources
                .iter()
                .filter(|s| crate::routes::dashboard_sources::source_visible(viewer, s))
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
    viewer: &Viewer,
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
    let board = match require_editable_chart(ch, &id, viewer).await {
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

    /// The tests' stand-in for `super::viewer_for`: the baseline reading, as
    /// no rule exists in a unit test.
    fn as_viewer(principal: Option<&Principal>) -> Viewer {
        principal.map_or_else(
            Viewer::anonymous,
            crate::routes::dashboard_organise::viewer_of,
        )
    }

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
            list_boards(&ch, &as_viewer(None)).await,
            list_charts(&ch, &as_viewer(None)).await,
            suggest_dashboard(&ch).await,
            delete_chart(&ch, &as_viewer(None), None, &{
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
        let _ = list_boards(&ch, &as_viewer(Some(&me))).await;
        let _ = list_charts(&ch, &as_viewer(Some(&me))).await;
        let _ = list_boards(&ch, &as_viewer(None)).await;
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
        let reply = delete_chart(&ch, &as_viewer(Some(&person(7))), Some(&person(7)), &args).await;
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
        let reply = delete_chart(&ch, &as_viewer(Some(&person(7))), Some(&person(7)), &args).await;
        assert_eq!(reply["error"], json!("this tile does not exist"), "{reply}");
        assert!(reply.get("deleted").is_none(), "{reply}");
        assert_eq!(
            writes(&sent(&server).await),
            [] as [&std::string::String; 0]
        );
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
        let reply = delete_chart(&ch, &as_viewer(Some(&person(7))), Some(&person(7)), &args).await;
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
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &args,
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["error"], json!("dashboard not found."));
        assert_eq!(
            writes(&sent(&server).await),
            [] as [&std::string::String; 0]
        );
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

    /// A layer that names no source is refused before the assistant saves it,
    /// and the refusal names the arguments its model sends.
    #[tokio::test]
    async fn create_chart_refuses_a_layer_with_no_data_source() {
        let server = mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = create_chart(
            &ch,
            &as_viewer(None),
            None,
            &arguments(json!({
                "title": "Visitors", "kind": "bar", "mart": "mart_x",
                "dimension": "province", "measures": ["visitors"],
                "layers": [{
                    "dimension": "province", "measures": ["visitors"],
                    "draw": "line", "axis": "right",
                }],
            })),
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(
            reply,
            json!({
                "error": "question 1: the layer names no data source: send its mart or sqlSource."
            }),
            "{reply}"
        );
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
            &as_viewer(None),
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
            &as_viewer(None),
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
            let created =
                create_chart(&ch, &as_viewer(None), None, &arguments(body), None, &time).await;
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
            &as_viewer(None),
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
            &as_viewer(None),
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
            &as_viewer(None),
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
            &as_viewer(None),
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
            &as_viewer(None),
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
            let reply =
                create_chart(&ch, &as_viewer(None), None, &arguments(input), None, &time).await;
            assert!(
                reply["error"].as_str().unwrap().contains(sentence),
                "{reply}"
            );
        }
    }

    /// `BI-16` part C1 (`AI-5`): "Show the project plan as a Gantt chart" is a
    /// task column, a start and an end; a start that is not a date is refused
    /// in the console's words.
    #[tokio::test]
    async fn the_request_to_show_the_project_plan_creates_a_gantt_chart() {
        let server = MockServer::start().await;
        rows(&server, "FROM system.tables", vec![json!({ "n": "1" })]).await;
        rows(
            &server,
            "FROM system.columns",
            vec![
                json!({ "name": "task", "type": "String" }),
                json!({ "name": "team", "type": "String" }),
                json!({ "name": "begins", "type": "Date" }),
                json!({ "name": "ends", "type": "Date" }),
                json!({ "name": "done", "type": "Float64" }),
            ],
        )
        .await;
        rest(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let time = lakehouse_bi::grain::TimeContext::default();
        let plan = json!({
            "title": "Project plan", "kind": "gantt", "mart": "mart_x", "dimension": "task",
            "breakdown": "team", "start": "begins", "end": "ends", "progress": "done",
        });
        let made = create_chart(
            &ch,
            &as_viewer(None),
            None,
            &arguments(plan.clone()),
            None,
            &time,
        )
        .await;
        assert_eq!(made["created"], json!(true), "{made}");
        let all = sent(&server).await;
        let ran = statements(&all);
        assert!(ran[0].contains("WHERE __e >= __s"), "{}", ran[0]);
        for (extra, sentence) in [
            (json!({ "start": "team" }), "not a date or date-time column"),
            (json!({ "progress": "team" }), "not a number column"),
            (json!({ "end": null }), "start column and an end column"),
            (json!({ "measures": ["done"] }), "no measure"),
        ] {
            let mut input = plan.clone();
            input
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let reply =
                create_chart(&ch, &as_viewer(None), None, &arguments(input), None, &time).await;
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
            &as_viewer(None),
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
            &as_viewer(None),
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
            &as_viewer(None),
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
            &as_viewer(None),
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
            &as_viewer(None),
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
            let refused =
                create_chart(&ch, &as_viewer(None), None, &image(url), None, &allowed).await;
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
            &as_viewer(None),
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
        let refused = update_chart(&ch, &as_viewer(None), None, &update, &allowed).await;
        assert_eq!(
            refused["error"],
            json!(lakehouse_bi::types_b::CONTENT_SITE_REFUSED)
        );
        let web = create_chart(
            &ch,
            &as_viewer(None),
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
        stored_chart_server_on(mart_server().await, def).await
    }

    /// [`stored_chart_server`] over a server the test built, for a chart that
    /// needs columns `mart_server` does not have.
    async fn stored_chart_server_on(server: MockServer, def: Value) -> MockServer {
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
            &as_viewer(Some(&person(7))),
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
            &as_viewer(Some(&person(7))),
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
            &as_viewer(Some(&person(7))),
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
        let draft = check_chart_draft(
            &ch,
            &as_viewer(Some(&person(7))),
            &args,
            Some("c_1".to_owned()),
            &time,
        )
        .await;
        let saved = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &args,
            &time,
        )
        .await;
        let draft = draft.expect_err("the draft is refused");
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
            &as_viewer(Some(&person(7))),
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
            &as_viewer(Some(&person(7))),
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
            &as_viewer(Some(&person(7))),
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
            &as_viewer(Some(&person(7))),
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
            &as_viewer(Some(&person(7))),
            &args,
            Some("c_1".to_owned()),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert!(draft.is_ok(), "{draft:?}");
    }

    // ── A call that names only id, title and kind keeps the rest ───────────

    /// `mart_x` with a second text column, three date and number columns for a
    /// Gantt chart, and a console that takes every write.
    async fn wide_mart_server() -> MockServer {
        let server = MockServer::start().await;
        rows(&server, "FROM system.tables", vec![json!({ "n": "1" })]).await;
        rows(
            &server,
            "FROM system.columns",
            vec![
                json!({ "name": "province", "type": "String" }),
                json!({ "name": "region", "type": "String" }),
                json!({ "name": "visitors", "type": "UInt64" }),
                json!({ "name": "begins", "type": "Date" }),
                json!({ "name": "ends", "type": "Date" }),
                json!({ "name": "done", "type": "Float64" }),
            ],
        )
        .await;
        rest(&server).await;
        server
    }

    /// A stored bar with a dimension, a measure, a breakdown and a chart
    /// filter, and no formatting.
    fn stored_bar_with_data() -> Value {
        json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
            "breakdown": "region",
            "filters": [{ "column": "region", "values": ["north"] }],
            "board": "b_board",
        })
    }

    /// The same stored bar, with a subtitle the update has to clear.
    fn stored_bar_with_subtitle() -> Value {
        let mut def = stored_bar_with_data();
        def["subtitle"] = json!("Visitors by province");
        def
    }

    /// The call that renames a chart and sends nothing but `id`, `title` and
    /// `kind`: the shape the tool's description asks for.
    fn bare_rename_call(extra: Value) -> Map<String, Value> {
        let mut args = arguments(json!({
            "id": "c_1",
            "title": "Visitors 2024",
            "kind": "bar",
        }));
        for (name, value) in arguments(extra) {
            args.insert(name, value);
        }
        args
    }

    /// A rename that names only `id`, `title` and `kind` keeps the chart's
    /// dimension, measures, breakdown and filter, and the reply names them.
    #[tokio::test]
    async fn a_rename_that_names_no_data_fields_keeps_the_dimension_measures_breakdown_and_filter()
    {
        let server = stored_chart_server_on(wide_mart_server().await, stored_bar_with_data()).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &bare_rename_call(json!({})),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        assert_eq!(
            reply["fieldsKept"],
            json!(["breakdown", "dimension", "filters", "measures"]),
            "{reply}"
        );
        let row = chart_row(&sent(&server).await);
        assert!(row.contains("\"dimension\":\"province\""), "{row}");
        assert!(row.contains("\"measures\":[\"visitors\"]"), "{row}");
        assert!(row.contains("\"breakdown\":\"region\""), "{row}");
        assert!(row.contains("\"values\":[\"north\"]"), "{row}");
    }

    /// `null` clears a field the call would otherwise keep: the breakdown is
    /// gone and the measure stays.
    #[tokio::test]
    async fn a_rename_that_sends_a_null_breakdown_clears_it_and_keeps_the_measure() {
        let server = stored_chart_server_on(wide_mart_server().await, stored_bar_with_data()).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &bare_rename_call(json!({ "breakdown": null })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        assert_eq!(
            reply["fieldsKept"],
            json!(["dimension", "filters", "measures"]),
            "{reply}"
        );
        let row = chart_row(&sent(&server).await);
        assert!(!row.contains("\"breakdown\""), "{row}");
        assert!(row.contains("\"measures\":[\"visitors\"]"), "{row}");
    }

    /// A call that sends the word "null" as the subtitle stores no subtitle:
    /// the assistant's model writes that text when it follows the build
    /// prompt's "send `subtitle: null`" against a schema that types the
    /// field as a string (AI-3, 2026-11-08).
    ///
    /// The prepared chart's subtitle is read off the written row, not off the
    /// reply: the reply's definition digest carries no subtitle field, while
    /// the stored chart has a `subtitle` key only when it has a subtitle.
    #[tokio::test]
    async fn an_update_that_sends_the_word_null_as_the_subtitle_stores_no_subtitle() {
        let server =
            stored_chart_server_on(wide_mart_server().await, stored_bar_with_subtitle()).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &bare_rename_call(json!({ "subtitle": "null" })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        let row = chart_row(&sent(&server).await);
        assert!(!row.contains("\"subtitle\""), "{row}");
    }

    /// A call that clears the subtitle with a real `null` stores no subtitle
    /// either: the server drops the key, so the subtitle the chart had is
    /// not carried over. This is the "null clears it" contract the tool's
    /// description promises, and it held before the store guard of
    /// AI-3 (2026-11-08); that guard is what handles the *text* "null", while
    /// this test pins the contract around it rather than the guard itself.
    ///
    /// The prepared chart's subtitle is read off the written row, not off the
    /// reply: the reply's definition digest carries no subtitle field, while
    /// the stored chart has a `subtitle` key only when it has a subtitle.
    #[tokio::test]
    async fn an_update_that_clears_the_subtitle_with_null_stores_no_subtitle() {
        let server =
            stored_chart_server_on(wide_mart_server().await, stored_bar_with_subtitle()).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &bare_rename_call(json!({ "subtitle": null })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        let row = chart_row(&sent(&server).await);
        assert!(!row.contains("\"subtitle\""), "{row}");
    }

    /// A chart changed to another kind keeps its data fields, but not its
    /// formatting.
    #[tokio::test]
    async fn a_kind_change_that_names_no_data_fields_keeps_them_and_resets_the_formatting() {
        let mut def = stored_bar_with_data();
        def["colors"] = json!({ "palette": "muted" });
        let server = stored_chart_server_on(wide_mart_server().await, def).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &bare_rename_call(json!({ "kind": "line" })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        assert_eq!(reply["formattingKept"], json!(false), "{reply}");
        let row = chart_row(&sent(&server).await);
        assert!(row.contains("\"dimension\":\"province\""), "{row}");
        assert!(row.contains("\"measures\":[\"visitors\"]"), "{row}");
        assert!(!row.contains("muted"), "{row}");
    }

    /// A bar chart turned into a pivot keeps no field the pivot refuses: the
    /// dimension, the measures and the breakdown are left out, because the
    /// pivot names its columns in its own fields (AI-5 AC1, 2026-11-08).
    #[test]
    fn a_kind_change_leaves_out_the_fields_the_new_kind_refuses() {
        let stored = input(stored_bar_with_data());
        let args = arguments(json!({
            "id": "c_1",
            "title": "Visitors",
            "kind": "pivot",
            "rows": ["province"],
            "values": [{ "column": "visitors", "aggregate": "sum" }],
            "totals": "grand",
        }));
        let call = input(json!({
            "title": "Visitors",
            "kind": "pivot",
            "rows": ["province"],
            "values": [{ "column": "visitors", "aggregate": "sum" }],
            "totals": "grand",
        }));
        let (merged, carried) = args_with_kept_fields(&args, &call, &stored);
        assert_eq!(carried, vec!["filters".to_owned()]);
        assert!(!merged.contains_key("dimension"), "{merged:?}");
        assert!(!merged.contains_key("measures"), "{merged:?}");
        assert!(!merged.contains_key("breakdown"), "{merged:?}");
    }

    /// A pivot turned into a bar chart keeps no field the bar refuses: the
    /// pivot's own fields are left out, and the bar takes the dimension and
    /// the measures the call names.
    #[test]
    fn a_kind_change_to_a_bar_leaves_out_the_pivot_fields() {
        let stored = input(json!({
            "title": "Visitors",
            "kind": "pivot",
            "mart": "mart_x",
            "rows": ["province"],
            "columns": ["region"],
            "values": [{ "column": "visitors", "aggregate": "sum" }],
            "totals": "grand",
        }));
        let args = arguments(json!({
            "id": "c_1",
            "title": "Visitors",
            "kind": "bar",
            "dimension": "province",
            "measures": ["visitors"],
        }));
        let call = input(json!({
            "title": "Visitors",
            "kind": "bar",
            "dimension": "province",
            "measures": ["visitors"],
        }));
        let (merged, carried) = args_with_kept_fields(&args, &call, &stored);
        assert!(carried.is_empty(), "{carried:?}");
        assert!(!merged.contains_key("rows"), "{merged:?}");
        assert!(!merged.contains_key("values"), "{merged:?}");
        assert!(!merged.contains_key("totals"), "{merged:?}");
        assert!(!merged.contains_key("columns"), "{merged:?}");
    }

    /// A bar chart turned into a pivot table is built without a refusal: the
    /// pivot names its columns in its own fields, so the fields of the bar
    /// are left out and the build has nothing to refuse (AI-5 AC1,
    /// 2026-11-08).
    #[tokio::test]
    async fn a_chart_turned_into_a_pivot_is_built_without_a_refusal() {
        let server = stored_chart_server_on(wide_mart_server().await, stored_bar_with_data()).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &bare_rename_call(json!({
                "kind": "pivot",
                "rows": ["province"],
                "values": [{ "column": "visitors", "aggregate": "sum" }],
                "totals": "grand",
            })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        assert_eq!(reply["kind"], json!("pivot"), "{reply}");
        assert_eq!(reply["fieldsKept"], json!(["filters"]), "{reply}");
        let row = chart_row(&sent(&server).await);
        // The definition holds the empty dimension and the empty measures of
        // a pivot, and no breakdown at all.
        assert!(row.contains("\"dimension\":\"\""), "{row}");
        assert!(row.contains("\"measures\":[]"), "{row}");
        assert!(!row.contains("\"breakdown\""), "{row}");
        assert!(row.contains("\"rows\":[\"province\"]"), "{row}");
        assert!(row.contains("\"totals\":\"grand\""), "{row}");
    }

    /// A call that names a field the new kind refuses is refused by the
    /// build: the model asked for it on purpose, so the tool says why.
    #[tokio::test]
    async fn a_pivot_call_that_names_the_measures_is_refused_by_the_build() {
        let server = stored_chart_server_on(wide_mart_server().await, stored_bar_with_data()).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &bare_rename_call(json!({
                "kind": "pivot",
                "rows": ["province"],
                "values": [{ "column": "visitors", "aggregate": "sum" }],
                "totals": "grand",
                "measures": ["visitors"],
            })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(
            reply["error"],
            json!(
                "a pivot names its columns in its own fields, not in dimension, measures or breakdown."
            ),
            "{reply}"
        );
    }

    /// `null` clears a field of any type, not only an optional one: a call
    /// that sends the value formats as `null` loses them.
    #[tokio::test]
    async fn a_rename_that_sends_null_value_formats_clears_them() {
        let mut def = stored_bar_with_data();
        def["valueFormats"] = json!({ "visitors": { "style": "number" } });
        let server = stored_chart_server_on(wide_mart_server().await, def).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &bare_rename_call(json!({ "valueFormats": null })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        let row = chart_row(&sent(&server).await);
        assert!(!row.contains("valueFormats"), "{row}");
    }

    /// A heading keeps its text through a rename that names only `id`,
    /// `title` and `kind`: the text is the tile's content, and the build
    /// refuses a heading without one.
    #[tokio::test]
    async fn a_heading_renamed_with_only_id_title_and_kind_keeps_its_text() {
        let server = stored_chart_server_on(
            wide_mart_server().await,
            json!({
                "title": "Sales",
                "kind": "heading",
                "span": 2,
                "text": "Sales in the north",
                "board": "b_board",
            }),
        )
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &arguments(json!({ "id": "c_1", "title": "Sales 2024", "kind": "heading" })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        assert_eq!(reply["fieldsKept"], json!(["span", "text"]), "{reply}");
        let row = chart_row(&sent(&server).await);
        assert!(row.contains("Sales in the north"), "{row}");
    }

    /// An image keeps its address and its description through a rename, and a
    /// tile without data never takes a data field: the stored image has none,
    /// and nothing but its own content is named.
    #[tokio::test]
    async fn an_image_renamed_with_only_id_title_and_kind_keeps_its_address_and_description() {
        let server = stored_chart_server_on(
            wide_mart_server().await,
            json!({
                "title": "Logo",
                "kind": "image",
                "span": 1,
                "url": "https://img.example/logo.png",
                "alt": "The company logo",
                "board": "b_board",
            }),
        )
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &arguments(json!({ "id": "c_1", "title": "Logo 2024", "kind": "image" })),
            &sites(&["https://img.example"]),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        assert_eq!(
            reply["fieldsKept"],
            json!(["alt", "span", "url"]),
            "{reply}"
        );
        let row = chart_row(&sent(&server).await);
        assert!(row.contains("https://img.example/logo.png"), "{row}");
        assert!(row.contains("The company logo"), "{row}");
    }

    /// A bar chart turned into a heading takes no data field along: the
    /// build refuses a filter on a tile without data, so carrying one would
    /// make the change fail. The call names the heading's text.
    #[tokio::test]
    async fn a_chart_turned_into_a_heading_carries_no_data_field() {
        let server = stored_chart_server_on(wide_mart_server().await, stored_bar_with_data()).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &arguments(
                json!({ "id": "c_1", "title": "Visitors", "kind": "heading", "text": "Visitors" }),
            ),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        let row = chart_row(&sent(&server).await);
        for gone in ["\"breakdown\"", "\"filters\"", "\"measures\":[\"visitors"] {
            assert!(!row.contains(gone), "{gone} in {row}");
        }
    }

    /// A heading changed to a bar chart does not report its text as kept: a
    /// data kind drops `text` when it is built, so naming it in `fieldsKept`
    /// would say something was saved that was not.
    #[tokio::test]
    async fn a_heading_changed_to_a_bar_chart_does_not_report_its_text_as_kept() {
        let server = stored_chart_server_on(
            wide_mart_server().await,
            json!({
                "title": "Sales",
                "kind": "heading",
                "span": 1,
                "text": "Section A",
                "board": "b_board",
            }),
        )
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &arguments(json!({
                "id": "c_1", "title": "Sales", "kind": "bar", "mart": "mart_x",
                "dimension": "province", "measures": ["visitors"],
            })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        let kept = reply["fieldsKept"].to_string();
        assert!(!kept.contains("text"), "{reply}");
        let row = chart_row(&sent(&server).await);
        assert!(!row.contains("Section A"), "{row}");
    }

    /// The draft check of an update reads the call as the tool does: `null`
    /// clears a field of any type, so a `null` list or map does not make the
    /// draft unreadable.
    #[tokio::test]
    async fn the_draft_check_of_an_update_reads_null_as_a_cleared_field() {
        let server = stored_chart_server_on(wide_mart_server().await, stored_bar_with_data()).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let draft = check_chart_draft(
            &ch,
            &as_viewer(Some(&person(7))),
            &arguments(json!({
                "title": "Visitors",
                "kind": "bar",
                "mart": "mart_x",
                "dimension": "province",
                "measures": ["visitors"],
                "valueFormats": null,
            })),
            Some("c_1".to_owned()),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert!(draft.is_ok(), "{draft:?}");
    }

    /// The draft of an update of a chart that is not there, one in the trash
    /// and one purged for good all answer the sentence `update_chart` answers,
    /// so the card is never offered for a save that cannot happen and the
    /// draft says nothing about a chart the caller cannot edit.
    #[tokio::test]
    async fn the_draft_check_of_an_update_of_a_chart_that_cannot_be_edited_answers_that_the_tile_does_not_exist()
     {
        let call = arguments(json!({
            "title": "Visitors", "kind": "bar", "mart": "mart_x",
            "dimension": "province", "measures": ["visitors"],
        }));
        let time = lakehouse_bi::grain::TimeContext::default();
        // No row at all, a chart in the trash (2) and one purged for good (1).
        for row in [None, Some(2), Some(1)] {
            let server = wide_mart_server().await;
            if let Some(state) = row {
                rows_first(
                    &server,
                    "SELECT is_deleted, board FROM console.bi_chart",
                    vec![json!({ "is_deleted": state, "board": "b_board" })],
                )
                .await;
            }
            let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
            let viewer = as_viewer(Some(&person(7)));
            let draft =
                check_chart_draft(&ch, &viewer, &call, Some("c_gone".to_owned()), &time).await;
            assert_eq!(
                draft,
                Err(json!({ "error": "this tile does not exist" })),
                "chart row {row:?}"
            );
        }
    }

    /// The card and the save agree: the draft check of a rename that names
    /// only `id`, `title` and `kind` builds the chart the save builds.
    #[tokio::test]
    async fn the_draft_check_of_a_rename_that_names_no_data_fields_passes() {
        let server = stored_chart_server_on(wide_mart_server().await, stored_bar_with_data()).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let draft = check_chart_draft(
            &ch,
            &as_viewer(Some(&person(7))),
            &bare_rename_call(json!({})),
            Some("c_1".to_owned()),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert!(draft.is_ok(), "{draft:?}");
    }

    /// A title is required, and `null` is not a title: the call is refused and
    /// nothing is written.
    #[tokio::test]
    async fn an_update_that_sends_a_null_title_is_refused() {
        let server = stored_chart_server_on(wide_mart_server().await, stored_bar_with_data()).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &bare_rename_call(json!({ "title": null })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        let error = reply["error"].as_str().unwrap_or_default();
        assert!(error.contains("title"), "{reply}");
        assert!(reply.get("updated").is_none(), "{reply}");
        assert_eq!(
            writes(&sent(&server).await),
            [] as [&std::string::String; 0]
        );
    }

    /// The Gantt fields are carried only when the new kind is Gantt: a Gantt
    /// chart changed to a bar chart does not take its columns or its today
    /// line along, because a bar chart refuses them.
    #[tokio::test]
    async fn a_gantt_chart_changed_to_a_bar_chart_does_not_carry_the_gantt_fields() {
        let server = stored_chart_server_on(
            wide_mart_server().await,
            json!({
                "title": "Plan",
                "kind": "gantt",
                "mart": "mart_x",
                "dimension": "province",
                "start": "begins",
                "end": "ends",
                "progress": "done",
                "todayLine": false,
                "board": "b_board",
            }),
        )
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let reply = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &bare_rename_call(json!({ "dimension": "province", "measures": ["visitors"] })),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        assert!(reply.get("fieldsKept").is_none(), "{reply}");
        let row = chart_row(&sent(&server).await);
        for gone in ["begins", "ends", "\"progress\"", "\"todayLine\""] {
            assert!(!row.contains(gone), "{gone} in {row}");
        }
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
        let reply = list_charts(&ch, &as_viewer(Some(&person(7)))).await;
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
        let reply = list_charts(&ch, &as_viewer(Some(&person(7)))).await;
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
        let reply = list_charts(&ch, &as_viewer(Some(&person(7)))).await;
        let chart = &reply["charts"][0];
        assert_eq!(chart["style"], json!({}), "{reply}");
        assert!(chart.get("referenceLines").is_none(), "{reply}");
        assert_eq!(chart["seriesNames"], json!(["visitors"]), "{reply}");
    }

    // ── What a chart holds, read back ──────────────────────────────────────

    fn input(def: Value) -> ChartInput {
        serde_json::from_value(def).expect("the definition reads")
    }

    /// A field the chart does not hold is left out, so a model that reads the
    /// digest cannot take it for something that was made.
    #[test]
    fn a_definition_digest_leaves_out_what_the_chart_does_not_hold() {
        let digest = definition_digest(&input(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
        })));
        assert_eq!(digest, "dimension: province; measures: visitors");
    }

    #[test]
    fn a_definition_digest_lists_every_field_it_covers_when_the_chart_holds_them_all() {
        let digest = definition_digest(&input(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "arrival",
            "breakdown": "province",
            "measures": ["visitors", { "column": "spend", "aggregate": "avg" }],
            "sort": { "by": "visitors", "dir": "desc" },
            "limit": 10,
            "grain": "month",
            "filters": [
                { "column": "province", "values": ["a"] },
                { "column": "region", "values": ["b"] },
            ],
        })));
        assert_eq!(
            digest,
            "dimension: arrival; breakdown: province; measures: visitors, spend; \
             sort: visitors desc; limit: 10 arrival values; \
             the breakdown is not limited; grain: month; filters: 2"
        );
    }

    /// A limit counts the values of the dimension, never the breakdown: with
    /// a time grain ordered by bucket it keeps the latest buckets. The digest
    /// says which, so a model cannot read "limit: 10" as "10 provinces"
    /// (BI-6-06 of the QA of 2026-11-07).
    #[test]
    fn a_definition_digest_says_a_limit_beside_a_grain_keeps_the_latest_buckets() {
        let by_bucket = definition_digest(&input(json!({
            "title": "T", "kind": "line", "mart": "mart_x", "dimension": "tanggal",
            "breakdown": "provinsi", "measures": ["visitors"],
            "grain": "month", "limit": 10,
        })));
        assert!(
            by_bucket.contains(
                "limit: 10 (the latest 10 month buckets of tanggal); the breakdown is not limited"
            ),
            "{by_bucket}"
        );
        // A sort by the dimension itself is the bucket order too.
        let by_dimension = definition_digest(&input(json!({
            "title": "T", "kind": "line", "mart": "mart_x", "dimension": "tanggal",
            "breakdown": "provinsi", "measures": ["visitors"],
            "sort": { "by": "tanggal", "dir": "asc" },
            "grain": "month", "limit": 10,
        })));
        assert!(
            by_dimension.contains("limit: 10 (the latest 10 month buckets of tanggal)"),
            "{by_dimension}"
        );
    }

    #[test]
    fn a_definition_digest_says_a_limit_beside_a_sort_counts_dimension_values() {
        let top_n = definition_digest(&input(json!({
            "title": "T", "kind": "bar", "mart": "mart_x", "dimension": "provinsi",
            "measures": ["visitors"],
            "sort": { "by": "visitors", "dir": "desc" }, "limit": 10,
        })));
        assert!(top_n.contains("limit: 10 provinsi values"), "{top_n}");
        assert!(!top_n.contains("latest"), "{top_n}");
        // No breakdown, so nothing to say about one.
        assert!(!top_n.contains("breakdown"), "{top_n}");
    }

    /// A part of the date (`day_of_week`) ordered by value is a top N of its
    /// values, and a truncation ordered by value is a top N of buckets: both
    /// count dimension values, neither keeps "the latest".
    #[test]
    fn a_definition_digest_reads_a_limit_on_a_date_part_or_a_value_order_as_dimension_values() {
        let part = definition_digest(&input(json!({
            "title": "T", "kind": "bar", "mart": "mart_x", "dimension": "arrival",
            "measures": ["visitors"],
            "sort": { "by": "visitors", "dir": "desc" },
            "grain": "day_of_week", "limit": 3,
        })));
        assert!(part.contains("limit: 3 arrival values"), "{part}");
        assert!(!part.contains("latest"), "{part}");
        let by_value = definition_digest(&input(json!({
            "title": "T", "kind": "bar", "mart": "mart_x", "dimension": "arrival",
            "measures": ["visitors"],
            "sort": { "by": "visitors", "dir": "desc" },
            "grain": "month", "limit": 3,
        })));
        assert!(by_value.contains("limit: 3 arrival values"), "{by_value}");
        assert!(!by_value.contains("latest"), "{by_value}");
    }

    /// A date part ordered by its own value (no sort, or a sort by the
    /// dimension) always returns its whole range: the builder replaces the
    /// limit with the part's range (`grain::effective_limit`). The digest must
    /// not claim a count of values the chart does not cut to.
    #[test]
    fn a_definition_digest_says_a_limit_on_a_date_part_in_its_own_order_cuts_nothing() {
        let digest = definition_digest(&input(json!({
            "title": "T", "kind": "bar", "mart": "mart_x", "dimension": "arrival",
            "measures": ["visitors"], "grain": "day_of_week", "limit": 3,
        })));
        assert!(
            digest.contains("limit: 3 (not applied: day_of_week returns its whole range)"),
            "{digest}"
        );
        assert!(!digest.contains("arrival values"), "{digest}");
        assert!(!digest.contains("latest"), "{digest}");
    }

    /// A pivot has no dimension (its grain applies to its first date field)
    /// and the builder never cuts it to its latest buckets, so the digest
    /// makes no claim about what the limit counts and names no empty column.
    #[test]
    fn a_definition_digest_makes_no_latest_claim_for_a_pivot_with_a_grain_and_a_limit() {
        let digest = definition_digest(&input(json!({
            "title": "T", "kind": "pivot", "mart": "mart_x",
            "rows": ["place"], "columns": ["day"],
            "values": [{ "column": "visitors", "aggregate": "sum" }],
            "grain": "month", "limit": 10,
        })));
        assert!(digest.contains("limit: 10; grain: month"), "{digest}");
        assert!(!digest.contains("latest"), "{digest}");
        assert!(!digest.contains("buckets"), "{digest}");
        assert!(!digest.contains("values"), "{digest}");
    }

    /// A parameter limit has no number to print, so the text names the
    /// parameter in the number's place and stays true either way.
    #[test]
    fn a_definition_digest_names_a_limit_parameter_in_place_of_the_number() {
        let latest = definition_digest(&input(json!({
            "title": "T", "kind": "line", "mart": "mart_x", "dimension": "tanggal",
            "breakdown": "provinsi", "measures": ["visitors"],
            "grain": "month", "limit": { "param": "top_n" },
        })));
        assert!(
            latest.contains(
                "limit: parameter top_n (the latest month buckets of tanggal); the breakdown is not limited"
            ),
            "{latest}"
        );
        let values = definition_digest(&input(json!({
            "title": "T", "kind": "bar", "mart": "mart_x", "dimension": "provinsi",
            "measures": ["visitors"],
            "sort": { "by": "visitors", "dir": "desc" }, "limit": { "param": "top_n" },
        })));
        assert!(
            values.contains("limit: parameter top_n provinsi values"),
            "{values}"
        );
    }

    /// The longer limit text must not push the line past the cap, nor drop
    /// the cut mark.
    #[test]
    fn a_definition_digest_with_a_limit_text_stays_within_three_hundred_characters() {
        let digest = definition_digest(&input(json!({
            "title": "T", "kind": "line", "mart": "mart_x",
            "dimension": "a_dimension_with_a_rather_long_column_name_that_keeps_going_on",
            "breakdown": "a_breakdown_with_a_rather_long_column_name_that_keeps_going_on",
            "measures": ["visitors", { "column": "spend", "aggregate": "avg" }],
            "sort": { "by": "visitors", "dir": "desc" },
            "limit": 10, "grain": "month",
            "filters": [
                { "column": "province", "values": ["a"] },
                { "column": "region", "values": ["b"] },
            ],
        })));
        assert!(digest.chars().count() <= 300, "{digest}");
        assert!(digest.ends_with('…'), "{digest}");
    }

    #[test]
    fn a_definition_digest_names_a_bin_by_its_width_or_count() {
        let wide = definition_digest(&input(json!({
            "title": "T", "kind": "bar", "mart": "mart_x", "dimension": "visitors",
            "measures": ["spend"],
            "bin": { "mode": "width", "width": 500 },
        })));
        assert!(wide.contains("bin: width 500"), "{wide}");
        let counted = definition_digest(&input(json!({
            "title": "T", "kind": "bar", "mart": "mart_x", "dimension": "visitors",
            "measures": ["spend"],
            "bin": { "mode": "count", "count": 10 },
        })));
        assert!(counted.contains("bin: 10 bins"), "{counted}");
    }

    /// A breakdown with no limit keeps every value of the dimension: the
    /// digest says so, because that is what "top N" is not.
    #[test]
    fn a_definition_digest_reads_limit_none_beside_a_breakdown_or_a_sort() {
        let with_breakdown = definition_digest(&input(json!({
            "title": "T", "kind": "bar", "mart": "mart_x", "dimension": "arrival",
            "breakdown": "province", "measures": ["visitors"],
        })));
        assert!(with_breakdown.contains("limit: none"), "{with_breakdown}");
        let with_sort = definition_digest(&input(json!({
            "title": "T", "kind": "bar", "mart": "mart_x", "dimension": "province",
            "measures": ["visitors"], "sort": { "by": "visitors", "dir": "asc" },
        })));
        assert!(with_sort.contains("limit: none"), "{with_sort}");
    }

    #[test]
    fn a_definition_digest_over_three_hundred_characters_is_cut_with_an_ellipsis() {
        let names: Vec<String> = (0..12)
            .map(|i| format!("{i:02}_{}", "x".repeat(37)))
            .collect();
        let digest = definition_digest(&input(json!({
            "title": "T", "kind": "stacked", "mart": "mart_x", "dimension": "province",
            "measures": names,
        })));
        assert_eq!(digest.chars().count(), 300, "{digest}");
        assert!(digest.ends_with('…'), "{digest}");
        assert!(
            digest.starts_with("dimension: province; measures: 00_x"),
            "{digest}"
        );
    }

    /// An update draft carries the digest of the merged chart, as a create
    /// draft carries its own, and it is the digest the saved reply carries:
    /// the stored chart merged with the call. The draft reads the stored chart
    /// as the caller and runs the save's checks (`PR #391 review`, `PR #402
    /// review`), so the digest says nothing a caller may not read.
    #[tokio::test]
    async fn the_draft_of_an_update_digests_the_merged_chart_as_the_saved_reply_does() {
        let server = stored_chart_server_on(wide_mart_server().await, stored_bar_with_data()).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let time = lakehouse_bi::grain::TimeContext::default();
        let draft = check_chart_draft(
            &ch,
            &as_viewer(Some(&person(7))),
            &bare_rename_call(json!({})),
            Some("c_1".to_owned()),
            &time,
        )
        .await
        .expect("the draft builds");
        let saved = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &bare_rename_call(json!({})),
            &time,
        )
        .await;
        // The call names neither a breakdown nor a filter: the stored chart's
        // are what the saved chart holds, so a digest of the call alone would
        // have left them out.
        assert_eq!(
            saved["definition"],
            json!(
                "dimension: province; breakdown: region; measures: visitors; limit: none (default applies); filters: 1"
            ),
            "{saved}"
        );
        assert_eq!(saved["definition"].as_str(), Some(draft.as_str()));
    }

    /// A stored bar whose formatting names a column the chart no longer
    /// returns, over a mart no other test uses: the draft of a rename that
    /// merges it is refused with a sentence that names that column, which is
    /// how a draft read as the server would show a caller a chart they may not
    /// see.
    fn stored_bar_with_stale_format() -> Value {
        let mut stored = stored_bar_with_data();
        stored["valueFormats"] = json!({ "hidden_revenue": { "style": "number" } });
        stored["mart"] = json!("mart_hidden_source");
        stored
    }

    /// The draft check reads the stored chart as the caller, so a chart on
    /// somebody else's private board is refused with the sentence the update
    /// tool answers, and the answer names nothing of the chart: not its
    /// columns, its formatting keys or its source.
    #[tokio::test]
    async fn the_draft_of_an_update_of_a_chart_on_somebody_elses_private_board_exposes_no_stored_field()
     {
        let server = wide_mart_server().await;
        rows_first(
            &server,
            "SELECT is_deleted, owner_id FROM console.bi_board",
            vec![json!({ "is_deleted": 0, "owner_id": "someone-else" })],
        )
        .await;
        let server = stored_chart_server_on(server, stored_bar_with_stale_format()).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let time = lakehouse_bi::grain::TimeContext::default();
        let draft = check_chart_draft(
            &ch,
            &as_viewer(Some(&person(7))),
            &bare_rename_call(json!({})),
            Some("c_1".to_owned()),
            &time,
        )
        .await;
        let saved = update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &bare_rename_call(json!({})),
            &time,
        )
        .await;
        assert_eq!(saved["error"], json!("this tile does not exist"), "{saved}");
        assert!(saved.get("definition").is_none(), "{saved}");
        // The draft answers what the save answers, and the answer is the whole
        // reply: it carries none of the hidden chart.
        let text = format!("{draft:?}");
        for hidden in ["hidden_revenue", "mart_hidden_source", "region", "province"] {
            assert!(!text.contains(hidden), "{hidden} leaked: {text}");
        }
        assert_eq!(draft, Err(saved));
    }

    /// The counterpart for a chart the caller sees: the same stored chart on
    /// a shared board is merged into the draft as before, so the caller
    /// gets the refusal about their own chart's formatting. The draft read the
    /// chart as the caller, not as nobody.
    #[tokio::test]
    async fn the_draft_of_an_update_of_a_chart_the_caller_sees_still_merges_the_stored_chart() {
        let server =
            stored_chart_server_on(wide_mart_server().await, stored_bar_with_stale_format()).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let draft = check_chart_draft(
            &ch,
            &as_viewer(Some(&person(7))),
            &bare_rename_call(json!({})),
            Some("c_1".to_owned()),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await;
        let error = draft.expect_err("the stale format is refused");
        assert!(
            error["error"]
                .as_str()
                .is_some_and(|e| e.contains("hidden_revenue")),
            "{error}"
        );
    }

    /// A create draft carries the digest of the call's own definition: there
    /// is no stored chart to read. It reads the dashboards as the caller, as
    /// the save does.
    #[tokio::test]
    async fn the_draft_of_a_create_carries_the_digest_of_the_call() {
        let server = wide_mart_server().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let draft = check_chart_draft(
            &ch,
            &as_viewer(Some(&person(7))),
            &arguments(json!({
                "title": "Visitors", "kind": "bar", "mart": "mart_x",
                "dimension": "province", "breakdown": "region", "measures": ["visitors"],
            })),
            None,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await
        .expect("the draft builds");
        assert_eq!(
            draft,
            "dimension: province; breakdown: region; measures: visitors; limit: none (default applies)"
        );
    }

    // ── The draft runs the checks of the save (`PR #391 review`, `PR #402 review`) ──

    /// A caller who sees every dashboard and every folder except `folder`,
    /// the way a deployment with one folder marked "only the people listed"
    /// reads a person with no rule on it.
    fn viewer_without_folder(folder: &str) -> Viewer {
        use lakehouse_bi::access::{AccessMap, ItemKind, Level};
        let tree = vec![lakehouse_bi::folders::Folder {
            id: folder.to_owned(),
            name: "Restricted".to_owned(),
            parent_id: String::new(),
            created_by: String::new(),
            updated_at: None,
            owner_id: String::new(),
        }];
        AccessMap::new(
            "u7",
            false,
            Some(Level::View),
            &tree,
            &[],
            &[(ItemKind::Folder, folder.to_owned())],
        )
        .viewer(&[])
        .expect("one folder fits")
    }

    /// The source `s_hidden`, filed in the folder `f_hidden`, with a text
    /// column and two number columns, added to `server`.
    async fn with_hidden_source(server: MockServer) -> MockServer {
        rows_first(
            &server,
            "FROM console.bi_source FINAL",
            vec![json!({
                "id": "s_hidden",
                "title": "Payroll",
                "sql": "SELECT city, salary, bonus FROM payroll",
                "columns_json": r#"[{"name":"city","type":"String"},{"name":"salary","type":"UInt64"},{"name":"bonus","type":"UInt64"}]"#,
                "folder_id": "f_hidden",
                "created_by": "",
                "updated_at": "2026-10-22 00:00:00",
            })],
        )
        .await;
        server
    }

    /// A chart over the hidden source that bins its text column: the refusal
    /// of that bin names the source's number columns (`number_columns_sentence`).
    fn binned_text_column_of_the_hidden_source() -> Value {
        json!({
            "title": "Spread", "kind": "bar", "sqlSource": "s_hidden",
            "dimension": "city", "measures": ["salary"],
            "bin": { "mode": "width", "width": 10 },
        })
    }

    fn assert_names_nothing_of_the_hidden_source(reply: &impl std::fmt::Debug) {
        let text = format!("{reply:?}");
        for hidden in ["salary", "bonus", "city", "number column"] {
            assert!(!text.contains(hidden), "{hidden} leaked: {text}");
        }
    }

    /// A create draft naming a SQL source in a folder the caller cannot see is
    /// answered as the save answers it, and the answer names none of the
    /// source's columns. Before, the draft got as far as the bin and listed
    /// the source's number columns.
    #[tokio::test]
    async fn the_draft_of_a_create_on_a_hidden_sql_source_is_refused_as_the_save_refuses() {
        let server = with_hidden_source(mart_server().await).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let viewer = viewer_without_folder("f_hidden");
        let args = arguments(binned_text_column_of_the_hidden_source());
        let time = lakehouse_bi::grain::TimeContext::default();
        let draft = check_chart_draft(&ch, &viewer, &args, None, &time).await;
        let saved = create_chart(&ch, &viewer, Some(&person(7)), &args, None, &time).await;
        assert_eq!(
            saved["error"],
            json!("SQL source 's_hidden' not found."),
            "{saved}"
        );
        assert_names_nothing_of_the_hidden_source(&draft);
        assert_eq!(draft, Err(saved));
    }

    /// The same for an update that adds such a source to a chart that reads a
    /// mart.
    #[tokio::test]
    async fn the_draft_of_an_update_that_adds_a_hidden_sql_source_is_refused_as_the_save_refuses() {
        let server = stored_chart_server_on(
            with_hidden_source(wide_mart_server().await).await,
            stored_bar_with_data(),
        )
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let viewer = viewer_without_folder("f_hidden");
        let mut args = arguments(binned_text_column_of_the_hidden_source());
        args.insert("id".to_owned(), json!("c_1"));
        args.insert("board".to_owned(), json!("b_board"));
        let time = lakehouse_bi::grain::TimeContext::default();
        let draft = check_chart_draft(&ch, &viewer, &args, Some("c_1".to_owned()), &time).await;
        let saved = update_chart(&ch, &viewer, Some(&person(7)), &args, &time).await;
        assert_eq!(
            saved["error"],
            json!("SQL source 's_hidden' not found."),
            "{saved}"
        );
        assert_names_nothing_of_the_hidden_source(&draft);
        assert_eq!(draft, Err(saved));
    }

    /// A chart that already reads a source keeps reading it when its folder is
    /// hidden from the caller: the dashboard governs, not the folder of the
    /// source. The draft is accepted, and so is the save.
    #[tokio::test]
    async fn the_draft_of_an_update_of_a_chart_that_already_reads_a_hidden_source_is_accepted() {
        let server = stored_chart_server_on(
            with_hidden_source(wide_mart_server().await).await,
            json!({
                "title": "Payroll", "kind": "bar", "sqlSource": "s_hidden",
                "dimension": "city", "measures": ["salary"], "board": "b_board",
            }),
        )
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let viewer = viewer_without_folder("f_hidden");
        let args = arguments(json!({
            "id": "c_1", "title": "Payroll 2024", "kind": "bar", "sqlSource": "s_hidden",
            "dimension": "city", "measures": ["salary"], "board": "b_board",
        }));
        let time = lakehouse_bi::grain::TimeContext::default();
        let draft = check_chart_draft(&ch, &viewer, &args, Some("c_1".to_owned()), &time).await;
        let saved = update_chart(&ch, &viewer, Some(&person(7)), &args, &time).await;
        assert_eq!(saved["updated"], json!(true), "{saved}");
        assert!(draft.is_ok(), "{draft:?}");
    }

    /// A create draft onto a dashboard in the trash is refused as the save
    /// refuses it.
    #[tokio::test]
    async fn the_draft_of_a_create_onto_a_dashboard_in_the_trash_is_refused_as_the_save_refuses() {
        let server = mart_server().await;
        rows_first(
            &server,
            "SELECT is_deleted, owner_id FROM console.bi_board",
            vec![json!({ "is_deleted": 2, "owner_id": "" })],
        )
        .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let args = arguments(json!({
            "title": "Visitors", "kind": "bar", "mart": "mart_x",
            "dimension": "province", "measures": ["visitors"], "board": "b_trash",
        }));
        let time = lakehouse_bi::grain::TimeContext::default();
        let viewer = as_viewer(Some(&person(7)));
        let draft = check_chart_draft(&ch, &viewer, &args, None, &time).await;
        let saved = create_chart(&ch, &viewer, Some(&person(7)), &args, None, &time).await;
        assert!(saved["error"].is_string(), "{saved}");
        assert!(saved.get("created").is_none(), "{saved}");
        assert_eq!(draft, Err(saved));
    }

    // ── A call whose `compare` does not read ────────────────────────────────

    #[test]
    fn a_compare_that_does_not_read_is_refused_with_a_valid_example() {
        let reply = parse_chart_input(&arguments(json!({
            "title": "Visitors", "kind": "kpi", "mart": "mart_x", "measures": ["visitors"],
            "compare": {},
        })))
        .expect_err("an empty compare does not read");
        assert!(reply.contains("\"compare\""), "{reply}");
        assert!(
            reply.contains(
                r#"{"compare": {"kind": "previous", "dateColumn": "<date column>", "period": "month"}}"#
            ),
            "{reply}"
        );
        assert!(reply.contains("inside the chart call"), "{reply}");
    }

    /// The example is for `compare` alone: another field's error is not
    /// followed by it.
    #[test]
    fn a_field_other_than_compare_that_does_not_read_gets_no_compare_example() {
        let reply = parse_chart_input(&arguments(json!({
            "title": "Visitors", "kind": "bar", "mart": "mart_x", "measures": ["visitors"],
            "limit": "ten",
        })))
        .expect_err("a text limit does not read");
        assert!(reply.contains("\"limit\""), "{reply}");
        assert!(!reply.contains("dateColumn"), "{reply}");
    }

    // ── An updated chart keeps its place on the dashboard ───────────────────

    /// The definition the charts of these tests were built from, on `board`.
    fn kept_place_def(board: &str) -> Value {
        json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "province",
            "measures": ["visitors"],
            "board": board,
        })
    }

    /// The stored document of chart `id`.
    fn stored_document_of(id: &str, board: &str) -> String {
        let mut document: Value =
            serde_json::from_str(&stored_document(kept_place_def(board))).unwrap();
        document["spec"]["id"] = json!(id);
        document.to_string()
    }

    /// A server where chart `c_1` is the middle one of three live charts of
    /// `board`, listed `c_a`, `c_1`, `c_z`, and the board has the row
    /// `board_row` (none: the board has no row). The board's writes fail when
    /// `board_write_fails`.
    async fn three_charts_server(
        board: &str,
        board_row: Option<Value>,
        board_write_fails: bool,
    ) -> MockServer {
        let server = mart_server().await;
        if board_write_fails {
            Mock::given(method("POST"))
                .and(body_string_contains("INSERT INTO console.bi_board "))
                .respond_with(
                    ResponseTemplate::new(500).set_body_string("planted-marker upstream text"),
                )
                .with_priority(1)
                .mount(&server)
                .await;
        }
        rows_first(
            &server,
            "SELECT is_deleted, board FROM console.bi_chart",
            vec![json!({ "is_deleted": 0, "board": board })],
        )
        .await;
        rows_first(
            &server,
            "FROM console.bi_chart FINAL WHERE is_deleted = 0",
            ["c_a", "c_1", "c_z"]
                .iter()
                .map(|id| {
                    json!({
                        "id": id,
                        "spec_json": stored_document_of(id, board),
                        "board": board,
                        "created_by": "ai",
                        "created_at": "2026-10-22 00:00:00",
                    })
                })
                .collect(),
        )
        .await;
        rows_first(
            &server,
            "SELECT is_deleted, owner_id FROM console.bi_board",
            vec![json!({ "is_deleted": 0, "owner_id": "" })],
        )
        .await;
        rows_first(
            &server,
            "FROM console.bi_board FINAL WHERE is_deleted = 0 AND id=",
            board_row.into_iter().collect(),
        )
        .await;
        server
    }

    /// A row of board `b_board` that nobody has arranged: no tile has a box.
    fn unarranged_board() -> Value {
        json!({
            "id": "b_board", "name": "Sales", "description": "", "created_by": "Ann",
            "layout_json": "{}", "filters_json": "[]", "public_token": "",
            "embed_enabled": 0, "folder_id": "", "refresh_seconds": 0, "grain": "",
            "params_json": "[]", "tabs_json": "[]", "phone_layout_json": "[]",
            "theme": "", "font": "", "cache_json": "", "owner_id": "",
            "created_at": "2026-10-22 00:00:00",
        })
    }

    async fn update_middle_chart(server: &MockServer) -> Value {
        update_middle_chart_with(server, json!({})).await
    }

    /// [`update_middle_chart`] with the fields of `extra` named in the call.
    async fn update_middle_chart_with(server: &MockServer, extra: Value) -> Value {
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        update_chart(
            &ch,
            &as_viewer(Some(&person(7))),
            Some(&person(7)),
            &rename_call(extra),
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await
    }

    /// The position of the first write that goes into `table`, among `writes`.
    fn write_position(writes: &[&String], table: &str) -> usize {
        writes
            .iter()
            .position(|s| s.contains(table))
            .unwrap_or_else(|| panic!("nothing was written into {table}: {writes:#?}"))
    }

    /// Rewriting a chart gives its row a new version, which sends a tile with
    /// no stored box to the end of the list. The board is therefore saved
    /// with a box for every tile, in the order they are drawn now, before the
    /// chart row is written; the updated chart keeps the place it had.
    #[tokio::test]
    async fn an_update_boxes_the_tiles_of_an_unarranged_board_before_it_rewrites_the_chart() {
        let server = three_charts_server("b_board", Some(unarranged_board()), false).await;
        let reply = update_middle_chart(&server).await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        let all = sent(&server).await;
        let w = writes(&all);
        let board_at = write_position(&w, "INTO console.bi_board ");
        let chart_at = write_position(&w, "INTO console.bi_chart");
        assert!(board_at < chart_at, "{w:#?}");
        assert_eq!(
            w.iter()
                .filter(|s| s.contains("INTO console.bi_board "))
                .count(),
            1,
            "{w:#?}"
        );
        assert!(
            w[board_at].contains(
                r#"'{"c_a":{"x":0,"y":0,"w":6,"h":6},"c_1":{"x":6,"y":0,"w":6,"h":6},"c_z":{"x":0,"y":6,"w":6,"h":6}}'"#
            ),
            "{}",
            w[board_at]
        );
    }

    /// A board whose tiles all have a box is not written by an update.
    #[tokio::test]
    async fn an_update_does_not_write_a_board_whose_tiles_all_have_a_box() {
        let mut board = unarranged_board();
        board["layout_json"] = json!(
            r#"{"c_a":{"x":0,"y":0,"w":6,"h":6},"c_1":{"x":6,"y":0,"w":6,"h":6},"c_z":{"x":0,"y":6,"w":6,"h":6}}"#
        );
        let server = three_charts_server("b_board", Some(board), false).await;
        let reply = update_middle_chart(&server).await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        let all = sent(&server).await;
        let w = writes(&all);
        assert!(
            w.iter().all(|s| !s.contains("INTO console.bi_board ")),
            "{w:#?}"
        );
        write_position(&w, "INTO console.bi_chart");
    }

    /// The update never half-applies: when the board cannot be saved, the
    /// chart row is not written and the reply is the fixed message with a
    /// reference, not the store's text.
    #[tokio::test]
    async fn an_update_whose_board_write_fails_leaves_the_chart_alone_and_leaks_nothing() {
        let server = three_charts_server("b_board", Some(unarranged_board()), true).await;
        let reply = update_middle_chart(&server).await;
        let text = reply.to_string();
        assert!(reply["errorId"].is_string(), "{text}");
        assert!(reply.get("updated").is_none(), "{text}");
        assert!(!text.contains("planted-marker"), "leaked: {text}");
        let all = sent(&server).await;
        assert!(
            writes(&all)
                .iter()
                .all(|s| !s.contains("INTO console.bi_chart")),
            "{all:#?}"
        );
    }

    /// A chart saved with no board is on the built-in board, which has no row
    /// until something about it is saved: its tiles are boxed on that row.
    #[tokio::test]
    async fn an_update_of_a_chart_with_no_board_boxes_the_built_in_boards_tiles() {
        let server = three_charts_server("", None, false).await;
        let reply = update_middle_chart(&server).await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        let all = sent(&server).await;
        let w = writes(&all);
        let board_at = write_position(&w, "INTO console.bi_board ");
        assert!(
            board_at < write_position(&w, "INTO console.bi_chart"),
            "{w:#?}"
        );
        assert!(
            w[board_at].contains("VALUES ('default', 'Dashboard',"),
            "{}",
            w[board_at]
        );
        assert!(
            w[board_at].contains(
                r#"'{"c_a":{"x":0,"y":0,"w":6,"h":6},"c_1":{"x":6,"y":0,"w":6,"h":6},"c_z":{"x":0,"y":6,"w":6,"h":6}}'"#
            ),
            "{}",
            w[board_at]
        );
    }

    /// A board with no live row does not stop the update, and the update does
    /// not make a row for it.
    #[tokio::test]
    async fn an_update_on_a_board_with_no_row_still_saves_the_chart() {
        let server = three_charts_server("b_board", None, false).await;
        let reply = update_middle_chart(&server).await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        let all = sent(&server).await;
        let w = writes(&all);
        assert!(
            w.iter().all(|s| !s.contains("INTO console.bi_board ")),
            "{w:#?}"
        );
        write_position(&w, "INTO console.bi_chart");
    }

    /// Naming the board the chart is already on is the same as naming none:
    /// the chart stays, so the board is boxed first.
    #[tokio::test]
    async fn an_update_that_names_the_boards_own_id_boxes_its_tiles_first() {
        let server = three_charts_server("b_board", Some(unarranged_board()), false).await;
        let reply = update_middle_chart_with(&server, json!({ "board": "b_board" })).await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        let all = sent(&server).await;
        let w = writes(&all);
        assert!(
            write_position(&w, "INTO console.bi_board ")
                < write_position(&w, "INTO console.bi_chart"),
            "{w:#?}"
        );
    }

    /// A chart that moves to another board leaves the tiles that stay in
    /// their order, so the old board is not written: a box saved for the
    /// leaving chart would stay as a hole the page never closes.
    #[tokio::test]
    async fn an_update_that_moves_the_chart_to_another_board_writes_no_board() {
        let server = three_charts_server("b_board", Some(unarranged_board()), false).await;
        let reply = update_middle_chart_with(&server, json!({ "board": "b_other" })).await;
        assert_eq!(reply["updated"], json!(true), "{reply}");
        assert_eq!(reply["board"], json!("b_other"), "{reply}");
        let all = sent(&server).await;
        let w = writes(&all);
        assert!(
            w.iter().all(|s| !s.contains("INTO console.bi_board ")),
            "{w:#?}"
        );
        write_position(&w, "INTO console.bi_chart");
    }
}
