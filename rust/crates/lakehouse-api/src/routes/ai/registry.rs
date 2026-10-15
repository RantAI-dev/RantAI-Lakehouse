//! The AI Copilot tool registry: one static table of [`ToolSpec`]s that is
//! the single source of truth for a tool's JSON schema (fed to the LLM in
//! [`tool_schemas`]), its risk tier (consumed by the Ask-mode gate in
//! [`super::gate`]), and the console-route permission it stands in for.
//!
//! Before this module existed, the schema list and the dispatch `match` in
//! `ai.rs` were two hand-maintained lists that had to be kept in sync by
//! hand. [`tool_schemas`] is now derived from [`TOOLS`], and
//! [`super::tools::run_tool`] looks a name up here (via [`find`]) before
//! dispatching, so the two can no longer drift apart.

use serde_json::{Value, json};

/// How dangerous a tool call is, and therefore what gate it must pass
/// before it executes.
///
/// Only [`Risk::Read`] is enforced today — the Ask-mode gate in
/// [`super::gate`] refuses every non-`Read` tool in `mode: "ask"`.
/// `WriteLow`/`WriteHigh` are assigned now, ahead of the inline-confirm
/// and approvals-inbox flows that will actually branch on them (see the
/// copilot-operations-handover plan, Tier 0), so that later work is pure
/// gate logic rather than also having to invent a risk tier for every
/// tool from scratch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    /// Never mutates the lakehouse, a dashboard, or a pipeline. Available
    /// in every chat mode, including `ask`.
    Read,
    /// Mutates something, but is judged safe enough for an inline
    /// chat-side confirmation rather than a human-approval queue.
    /// Enforced by [`super::gate::decide`] (T0.4): a call without
    /// `"confirmed": true` in its args gets a `needs_confirmation` result
    /// instead of executing.
    WriteLow,
    /// Mutates something destructively or irreversibly enough that it
    /// should require routing through a human-approval queue rather than
    /// an inline confirmation. The real approvals-inbox flow is T0.5 of
    /// the copilot-operations-handover plan and does not exist yet — until
    /// then, [`super::gate::decide`] refuses every `WriteHigh` call with a
    /// clear "not implemented" reason rather than executing it or letting
    /// it through the `WriteLow` confirmation path.
    WriteHigh,
}

/// One entry in the AI Copilot's tool table: everything the chat loop
/// needs to advertise a tool to the LLM, gate its execution by chat mode,
/// and — in a later task — check the calling principal's permissions and
/// audit the call.
pub struct ToolSpec {
    /// The name the LLM calls this tool by; also the dispatch key
    /// [`super::tools::run_tool`] matches on.
    pub name: &'static str,
    /// Builds this tool's `OpenAI`-compatible JSON function schema. A
    /// plain `fn` pointer rather than a closure: none of these schemas
    /// capture anything, they are pure functions of no input.
    pub schema: fn() -> Value,
    /// This tool's risk tier — see [`Risk`].
    pub risk: Risk,
    /// The `resource:action` permission the equivalent console route
    /// requires, per `lakehouse-api::policy::POLICY_TABLE` — e.g.
    /// `create_chart`'s `"dashboard:write"` mirrors
    /// `POST /api/dashboard/specs`. An empty string means the closest
    /// equivalent route is `Policy::RequiresAuth` with no specific
    /// permission (authenticated only); there is nothing narrower to
    /// carry for that tool today.
    ///
    /// Enforced by [`super::gate::decide`] (T0.2 of the
    /// copilot-operations-handover plan): a principal whose merged
    /// `PermissionSet` lacks this permission has every call to this tool
    /// refused at dispatch, regardless of chat mode. Also used, as a pure
    /// optimisation (dispatch remains the real enforcement), to filter the
    /// tool list advertised to the model in [`tool_schemas_for`].
    pub permission: &'static str,
}

/// The thirteen values of a chart's `grain` (`BI-9`), shared by
/// [`create_chart_schema`] and [`update_chart_schema`]. Read from
/// [`lakehouse_bi::grain::ALL_GRAINS`] so the schema cannot drift from what
/// the server accepts.
fn grain_enum() -> Value {
    json!(
        lakehouse_bi::grain::ALL_GRAINS
            .iter()
            .map(|g| g.as_str())
            .collect::<Vec<_>>()
    )
}

/// The description of the `grain` property, the same on both chart tools.
const GRAIN_DESCRIPTION: &str = "how to group a date or timestamp `dimension` (the column's kind is in describe_mart / list_sql_sources `columns`): minute, hour, day, week, month, quarter, year; or a part of the date: hour_of_day, day_of_week, day_of_month, week_of_year, month_of_year, quarter_of_year. Not for number or text columns; minute, hour and hour_of_day need a datetime column; a calendar chart takes day only; only bar, hbar, line, area, stacked, combo, waterfall, heatmap, pie, rose, funnel, treemap, radar and calendar charts take one. Omit for no grouping.";

/// The `chart_kind_enum` JSON array shared by [`create_chart_schema`] and
/// [`update_chart_schema`] — factored out only because the two schemas
/// are otherwise identical lists; not shared with any other tool.
fn chart_kind_enum() -> Value {
    json!([
        "bar",
        "hbar",
        "line",
        "area",
        "stacked",
        "combo",
        "pie",
        "rose",
        "funnel",
        "treemap",
        "scatter",
        "bubble",
        "heatmap",
        "radar",
        "waterfall",
        "geomap",
        "pointmap",
        "geoheat",
        "sankey",
        "sunburst",
        "boxplot",
        "calendar",
        "kpi",
        "gauge",
        "table",
        "pivot",
        "text",
        "progress",
        "histogram",
        "bullet",
        "image",
        "web"
    ])
}

/// The properties `BI-16` part B adds to both chart tools (progress, histogram,
/// bullet, image and web cards), inserted into the schema built by
/// [`create_chart_schema`] and [`update_chart_schema`] so the two cannot drift.
/// The limits are the ones `lakehouse_bi::types_b` enforces, and an address the
/// administrator does not allow is refused with the console's own sentence.
fn with_type_properties(mut schema: Value) -> Value {
    let extra = json!({
        "goal": { "oneOf": [
            { "type": "number" },
            { "type": "object", "properties": { "measure": { "type": "string" } }, "required": ["measure"] } ],
            "description": "progress (required) and bullet (optional target): a number, or {\"measure\": <column>} for a second value of the same source, which must also be the second entry of `measures`. A progress bar takes one value in `measures` and no dimension" },
        "bands": { "type": "array", "minItems": 1, "maxItems": 3, "items": { "type": "number" },
            "description": "bullet only: 1 to 3 rising upper limits of the bands behind the bar (for example poor, fair, good)" },
        "histogram": { "type": "object", "properties": {
            "width": { "type": "number", "description": "the width of each range" },
            "count": { "type": "integer", "minimum": 1, "maximum": 100, "description": "at most this many ranges" } },
            "description": "histogram only: give a width or a count, not both; leave out for about 20 ranges chosen automatically. The number column is `dimension`; a histogram takes no measures, breakdown, limit or order" },
        "url": { "type": "string", "description": "image and web only (required): an https address on a site an administrator allows in Admin > Settings; any other address is refused" },
        "alt": { "type": "string", "maxLength": 200, "description": "image only (required): a short description of the picture for screen readers" }
    });
    if let Some(props) = schema
        .pointer_mut("/function/parameters/properties")
        .and_then(Value::as_object_mut)
        && let Value::Object(extra) = extra
    {
        props.extend(extra);
    }
    schema
}

/// The properties `BI-16` part A adds to both chart tools (raw tables, pivots,
/// KPI comparisons, column settings), inserted into the schema built by
/// [`create_chart_schema`] and [`update_chart_schema`] so the two cannot drift.
/// The limits are the ones `lakehouse_bi::tables` enforces.
fn with_table_properties(mut schema: Value) -> Value {
    let extra = json!({
        "tableMode": { "type": "string", "enum": ["grouped", "rows"], "description": "table only: grouped (default) summarises by `dimension`; rows lists the rows themselves, 50 a page, in the `columns` you pick (no dimension or measures)" },
        "columns": { "type": "array", "items": { "type": "string" }, "description": "table in rows mode: the columns to list, in order (1 to 30). pivot: the column fields (0 to 2)" },
        "rows": { "type": "array", "items": { "type": "string" }, "description": "pivot only (required): the row fields, outermost first (1 to 3)" },
        "values": { "type": "array", "items": { "type": "object", "properties": {
            "column": { "type": "string" },
            "aggregate": { "type": "string", "enum": ["sum", "avg", "max", "min", "count"] } },
            "required": ["column", "aggregate"] },
            "description": "pivot only (required): what each cell shows (1 to 5). A pivot takes no dimension, measures or breakdown" },
        "totals": { "type": "string", "enum": ["none", "grand", "all"], "description": "pivot only: none (default); grand adds the row, column and grand totals; all adds a subtotal for every outer group as well. Computed by the database, not by adding cells" },
        "sortColumn": { "type": "string", "description": "table in rows mode: the column the rows are sorted by" },
        "sortDir": { "type": "string", "enum": ["asc", "desc"] },
        "columnSettings": { "type": "object", "description": "table and pivot: per column name, {label, format (auto, number, percent, currency, date, link, image), decimals 0 to 6, width 60 to 800, wrap, hidden}. A link or image column must hold http(s) URLs" },
        "compare": { "type": "object", "description": "kpi only: {\"kind\":\"previous\",\"dateColumn\":<a date or timestamp column>,\"period\":day|week|month|quarter|year} compares the latest period that has data with the one before it and draws a trend line; or {\"kind\":\"goal\",\"value\":<number>} compares with a goal" },
        "goodDirection": { "type": "string", "enum": ["up", "down"], "description": "kpi only: which way is good, up (default) or down; colours the change" }
    });
    if let Some(props) = schema
        .pointer_mut("/function/parameters/properties")
        .and_then(Value::as_object_mut)
        && let Value::Object(extra) = extra
    {
        props.extend(extra);
    }
    schema
}

/// The `measures` of `create_chart` and `update_chart` (`BI-6`): a column name,
/// or a column with its own aggregate. A plain name takes the chart's
/// `aggregate`, as before.
fn measures_schema() -> Value {
    json!({ "type": "array", "items": { "oneOf": [
        { "type": "string" },
        { "type": "object", "properties": {
            "column": { "type": "string" },
            "aggregate": { "type": "string", "enum": ["sum", "avg", "max", "min", "count"] } },
            "required": ["column", "aggregate"],
            "description": "a measure with its own aggregate, e.g. the average of stay beside the sum of visitors; its result column is named <aggregate>_<column> (avg_stay). Not for kpi, gauge, boxplot, maps, pivot or a table in rows mode" },
        { "type": "object", "properties": {
            "column": param_ref_schema(),
            "aggregate": { "type": "string", "enum": ["sum", "avg", "max", "min", "count"] } },
            "required": ["column"],
            "description": "a measure whose column the viewer switches: {column: {param: <list parameter>}, aggregate?}" },
        { "type": "object", "properties": {
            "metric": { "type": "string", "description": "the name of a metric from list_metrics" } },
            "required": ["metric"],
            "description": "a saved metric by name (BI-2): the same number everywhere; it has its own aggregate, so no `aggregate` applies. The chart's table must be the metric's table. Prefer it to writing a measure yourself when list_metrics has one that matches" }
    ] },
        "description": "the columns to aggregate (numbers); each is a column name (the chart's `aggregate` applies) or {column, aggregate}" })
}

/// `{ "param": <name> }` (`BI-11`).
fn param_ref_schema() -> Value {
    json!({ "type": "object", "properties": { "param": { "type": "string", "description": "the name of a parameter from list_parameters" } },
        "required": ["param"] })
}

/// How a number is binned (`BI-10`): the three modes of `lakehouse_bi::bin`.
fn bin_schema(description: &str) -> Value {
    json!({ "type": "object", "properties": {
        "mode": { "type": "string", "enum": ["auto", "count", "width"], "description": "auto: about ten bins with round edges; count: at most that many bins; width: bins of exactly that width" },
        "count": { "type": ["number", "string"], "description": "mode count: 10, 50 or 100" },
        "width": { "type": ["number", "string"], "description": "mode width: a positive number; the only mode for a SQL source" } },
        "required": ["mode"], "description": description })
}

/// The filters a chart or a question carries (at most 20): the same filters a
/// dashboard takes, on columns of the data they belong to.
fn filters_schema(description: &str) -> Value {
    json!({ "type": "array", "items": { "oneOf": [
        { "type": "object", "properties": {
            "segment": { "type": "string", "description": "the name of a saved segment of the chart's table (BI-2): stands for its filters" } },
            "required": ["segment"] },
        { "type": "object", "properties": {
            "column": { "type": "string" },
            "op": { "type": "string", "enum": ["in", "not_in", "between", "relative", "contains", "starts_with", "ends_with", "not_contains"], "description": "default in" },
            "values": { "type": "array", "items": { "type": "string" }, "description": "in / not_in" },
            "min": { "type": "string", "description": "between: a number or a date YYYY-MM-DD" },
            "max": { "type": "string" },
            "minExclusive": { "type": "boolean" }, "maxExclusive": { "type": "boolean" },
            "text": { "type": "string", "description": "contains, starts_with, ends_with, not_contains" },
            "anchor": { "type": "string", "enum": ["last", "this", "previous", "next"], "description": "relative" },
            "unit": { "type": "string", "enum": ["day", "week", "month", "quarter", "year"] },
            "n": { "type": "number" } },
            "required": ["column"] }
    ] }, "description": description })
}

/// The joins of a chart or a question (`BI-7`): up to four more Gold marts.
fn joins_schema(description: &str) -> Value {
    json!({ "type": "array", "minItems": 1, "maxItems": 4, "items": { "type": "object", "properties": {
            "mart": { "type": "string", "description": "a Gold mart to join to the chart's `mart` (call suggest_join first)" },
            "type": { "type": "string", "enum": ["inner", "left", "right", "full"], "description": "left keeps every row of what comes before; inner only the matches; right and full keep the joined table's unmatched rows too" },
            "on": { "type": "array", "minItems": 1, "maxItems": 4, "items": { "type": "object", "properties": {
                "left": { "type": "string", "description": "a column of the chart's mart, or of an earlier join as <table>__<column>" },
                "op": { "type": "string", "enum": ["=", "<", "<=", ">", ">="] },
                "right": { "type": "string", "description": "a column of the table being joined" } },
                "required": ["left", "op", "right"] },
                "description": "the pairs to match, all of them; at least one uses =" } },
            "required": ["mart", "type", "on"] }, "description": description })
}

/// The `BI-6` fields of `create_chart` and `update_chart` (the chart's own
/// filters, sort, "Other", groupings), inserted the way
/// [`with_table_properties`] inserts the table fields. The limits are the ones
/// `lakehouse_bi::shape` enforces.
fn with_shape_properties(mut schema: Value) -> Value {
    let extra = json!({
        "limit": { "oneOf": [ { "type": "number" }, param_ref_schema() ],
            "description": "the number of groups; or {param: <a number parameter>} to let the viewer choose it (a whole number from 1 to 100 when the chart is read)" },
        "dimension": { "oneOf": [ { "type": "string" }, param_ref_schema() ],
            "description": "the column to group by; or {param: <a list parameter whose values are columns of the data>} to let the viewer switch it" },
        "breakdown": { "oneOf": [ { "type": "string" }, param_ref_schema() ],
            "description": "a second column to split by; or {param: <a list parameter>} to let the viewer switch it" },
        "referenceLines": reference_lines_schema(),
        "filters": filters_schema("the chart's own filters (at most 20), always applied, whatever the dashboard's filters are; the same filters a dashboard takes, on columns of the chart's data. Use them for \"only one category\" or \"only 2026\""),
        "layers": { "type": "array", "minItems": 1, "maxItems": 4, "items": { "type": "object", "properties": {
            "mart": { "type": "string", "description": "the Gold mart this question reads (give this or sqlSource)" },
            "sqlSource": { "type": "string", "description": "the id of a saved SQL source (give this or mart)" },
            "dimension": { "type": "string", "description": "a column of this question's data to line up with the chart's dimension: both dates, both text or both numbers; dates are lined up by the chart's time grain" },
            "measures": measures_schema(),
            "aggregate": { "type": "string", "enum": ["sum", "avg", "max", "min", "count"], "description": "for the measures that have none of their own; default sum" },
            "filters": filters_schema("this question's own filters (at most 20), on its own data"),
            "joins": joins_schema("up to four more Gold marts joined to this question's mart; not for a SQL source"),
            "label": { "type": "string", "description": "the legend name, at most 60 characters" },
            "draw": { "type": "string", "enum": ["bar", "line"], "description": "how this question is drawn" },
            "axis": { "type": "string", "enum": ["left", "right"], "description": "which value axis; use right when its numbers are on another scale, like a rate beside visitors" } },
            "required": ["dimension", "measures", "draw", "axis"] },
            "description": "up to 4 more questions drawn on the same chart (bar, line, area or combo only), each from its own table or SQL source, lined up on the dimension's values: a value one lacks is a gap. For \"visitors next to hotel rooms by month\": the chart is visitors by month; one layer reads the rooms table by its month column. No breakdown, 'Other', steps or table calculation on a chart with layers. Unlike a join, layers never multiply rows" },
        "draw": { "type": "string", "enum": ["bar", "line"], "description": "with layers: how the chart's own series is drawn (default bar for a bar chart, line otherwise)" },
        "axis": { "type": "string", "enum": ["left", "right"], "description": "with layers: which value axis the chart's own series uses (default left)" },
        "sort": { "type": "object", "properties": {
            "by": { "type": "string", "description": "the dimension, a grouping of a table, or a measure's result name (the column name, or <aggregate>_<column> for a measure with its own aggregate)" },
            "dir": { "type": "string", "enum": ["asc", "desc"] } },
            "required": ["by", "dir"],
            "description": "the order of the groups, in place of the default; with `limit` it keeps the first N in that order (top N: desc by a measure; bottom N: asc). Not for kpi, gauge, boxplot, maps, pivot or a table in rows mode" },
        "other": { "type": "boolean", "description": "with `limit` and `sort`: gather every group past the limit into one group called Other, computed from the underlying rows (an average of Other is the average of those rows). bar, hbar, stacked, pie, rose, funnel, treemap and grouped table with one dimension only; not with a grain" },
        "steps": { "type": "array", "minItems": 1, "maxItems": 2, "items": { "type": "object", "properties": {
            "filter": { "type": "array", "minItems": 1, "maxItems": 10, "items": { "type": "object", "properties": {
                "column": { "type": "string", "description": "a grouping or a measure of the result before this step" },
                "op": { "type": "string", "enum": ["eq", "ne", "gt", "gte", "lt", "lte", "between"] },
                "value": { "type": ["string", "number"], "description": "all but between: a number, a date YYYY-MM-DD, or text (eq and ne only)" },
                "min": { "type": ["string", "number"] }, "max": { "type": ["string", "number"] } },
                "required": ["column", "op"] },
                "description": "keep the rows that meet every condition" },
            "summarise": { "type": "object", "properties": {
                "by": { "type": "array", "items": { "type": "string" }, "description": "the groupings to keep, a subset of the result before (at least one)" },
                "measures": { "type": "array", "minItems": 1, "maxItems": 6, "items": { "type": "object", "properties": {
                    "column": { "type": "string", "description": "a measure of the result before this step" },
                    "aggregate": { "type": "string", "enum": ["sum", "avg", "max", "min", "count"] } },
                    "required": ["column", "aggregate"] },
                    "description": "each is named <aggregate>_<column> in the result" } },
                "required": ["by", "measures"],
                "description": "group the result before this step again" } },
            "description": "give exactly one of filter and summarise" },
            "description": "up to two steps applied, in order, to the grouped result (for example: summarise by province, then keep totals over 1000). The chart's sort and limit then apply to the last step's result and may name only its columns (summarised measures are <aggregate>_<column>). Not for kpi, gauge, pivot, a raw table, maps, a time grain, a breakdown or 'Other'" },
        "joins": joins_schema("up to four more Gold marts joined to `mart`, read as one table: the chart's mart columns keep their names and a joined table's columns are <table>__<column> (a table joined twice: <table>2__<column>), usable as dimension, breakdown, measure, filter and in tables and pivots. Not for a SQL source. Joining on columns that are not unique in the joined table multiplies rows and so sums: prefer a max or avg for a joined table's measure. Calculated fields cannot use a joined column"),
        "groupings": { "type": "array", "minItems": 1, "maxItems": 5, "items": { "type": "object", "properties": {
            "column": { "type": "string" },
            "grain": { "type": "string", "enum": grain_enum(), "description": "for a date or timestamp column" },
            "bin": bin_schema("for a number column, instead of a grain") },
            "required": ["column"] },
            "description": "table only: the columns a grouped table is grouped by (1 to 5), instead of dimension" },
        "bin": bin_schema("group the number `dimension` (or a pivot's first number row or column field) in ranges, for example visitors in bins of 500; the result value is each bin's lower edge. bar, hbar, line, area, stacked, combo, pie, rose, funnel, treemap, radar, waterfall, grouped table, pivot; not with a grain"),
        "breakdownBin": bin_schema("the same for a number `breakdown`"),
        "grid": { "type": "object", "properties": {
            "size": { "type": ["string", "number"], "description": "auto, or the cell size in degrees: 0.01, 0.1, 1 or 10" } },
            "required": ["size"],
            "description": "pointmap and geoheat only: group the points into square cells of latitude and longitude and draw one mark per cell at its centre, with the cell's value and point count" }
    });
    if let Some(props) = schema
        .pointer_mut("/function/parameters/properties")
        .and_then(Value::as_object_mut)
        && let Value::Object(extra) = extra
    {
        props.extend(extra);
    }
    schema
}

fn run_sql_schema() -> Value {
    json!({ "type": "function", "function": { "name": "run_sql",
        "description": "Run a read-only ClickHouse SELECT and return its rows. Use it for every number you report. Query Gold marts (serving.mart_*) for totals and trends, Silver (silver.<table>) for detail rows. Compute totals, percentages, growth and rankings in the SQL itself. Tables and columns are in the DATA MAP.",
        "parameters": { "type": "object",
            "properties": { "sql": { "type": "string", "description": "A ClickHouse SELECT statement" } },
            "required": ["sql"] } } })
}

/// The tool that asks the person a question. Named once so the places that
/// must treat it apart (the chat's switch, a Digital Employee's run) cannot
/// misspell it; a test pins it to a registered tool.
pub const ASK_USER: &str = "ask_user";

fn ask_user_schema() -> Value {
    json!({ "type": "function", "function": { "name": "ask_user",
        "description": "Ask the user which reading of an unclear word they mean, with two to four options taken from the DATA MAP. Use it only when a word fits two or more tables, columns or values and nothing in the DATA MAP or THIS USER'S WORDS settles it. Ask once, run no query in that turn, and write nothing else after the call.",
        "parameters": { "type": "object", "properties": {
            "term": { "type": "string", "description": "the unclear word, as the user wrote it (1 to 60 characters)" },
            "question": { "type": "string", "description": "one sentence asking which they mean (at most 200 characters)" },
            "options": { "type": "array", "minItems": 2, "maxItems": 4, "items": { "type": "string" }, "description": "the readings to choose from: names from the DATA MAP, each 1 to 80 characters" } },
            "required": ["term", "question", "options"] } } })
}

/// The tool that proposes a corrected statement for a Query Studio query that
/// failed. Named once so the chat's switch and its offered list cannot
/// misspell it; a test pins it to a registered tool.
pub const PROPOSE_SQL: &str = "propose_sql";

fn propose_sql_schema() -> Value {
    json!({ "type": "function", "function": { "name": "propose_sql",
        "description": "Propose the whole corrected statement once, with one sentence saying what was wrong. The server checks that it would run for this user, and the person sees it as a diff and accepts or rejects it. It is never run by you or by the server. Use it only for a query the person asked you to fix.",
        "parameters": { "type": "object", "properties": {
            "sql": { "type": "string", "description": "the whole corrected statement, not a fragment (1 to 20000 characters)" },
            "reason": { "type": "string", "description": "one sentence saying what was wrong (1 to 300 characters)" },
            "engine": { "type": "string", "enum": ["clickhouse", "trino"], "description": "the engine the statement is for" } },
            "required": ["sql", "reason", "engine"] } } })
}

fn list_datasets_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_datasets",
        "description": "List the datasets registered in the lakehouse catalog: slug, title, whether it comes from a primary or secondary source, and the Gold table it is served from. Optional keyword or source filter.",
        "parameters": { "type": "object", "properties": {
            "search": { "type": "string", "description": "keyword to match in the title or slug" },
            "source": { "type": "string", "enum": ["primary", "secondary"], "description": "only datasets from primary or secondary sources" } } } } })
}

fn lakehouse_overview_schema() -> Value {
    json!({ "type": "function", "function": { "name": "lakehouse_overview",
        "description": "Everything in the lakehouse by layer: the registered Bronze datasets, and every Silver and Gold table with its row count. Use it for \"what data do we have?\" and other overview questions.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn describe_dataset_schema() -> Value {
    json!({ "type": "function", "function": { "name": "describe_dataset",
        "description": "Describe one dataset by slug: title, description, source kind, which layers (Bronze/Silver/Gold) it is present in with row counts, and its documented columns.",
        "parameters": { "type": "object", "properties": { "slug": { "type": "string" } },
            "required": ["slug"] } } })
}

fn get_lineage_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_lineage",
        "description": "Recorded lineage of a dataset or table: what feeds it and what it feeds (publisher, connector ingest, catalog registry, authored pipelines, views, Gold export). Every edge comes from a platform record; nothing is inferred from names.",
        "parameters": { "type": "object", "properties": { "slug": { "type": "string", "description": "a dataset slug, or a table name such as serving.<table> or bronze.<table>" } },
            "required": ["slug"] } } })
}

fn get_quality_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_quality",
        "description": "Summary of the latest data quality check results (how many checks passed, warned or failed). Says so when no checks have run yet.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn trigger_lakehouse_build_schema() -> Value {
    json!({ "type": "function", "function": { "name": "trigger_lakehouse_build",
        "description": "Rebuild the lakehouse layer by layer from what this deployment has: ingest every connector with an ingest spec into Bronze, run every authored pipeline, then export Gold to Iceberg (or the deployment's own build job when it has one). The result lists what was launched and what was skipped and why. Use only when the user asks to rebuild or refresh everything.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn get_build_status_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_build_status",
        "description": "A log of the 10 most recent Dagster runs across all jobs, with status and start time. It does NOT give each pipeline's current state: to find failing pipelines use list_pipelines.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn describe_mart_schema() -> Value {
    json!({ "type": "function", "function": { "name": "describe_mart",
        "description": "Gold marts (serving.*) that can be charted. With no argument: every mart with its row count. With `mart`: its columns split into dimensions (categories, time) and measures (numbers). Each column also has a kind (number, date, datetime, text); a date or datetime dimension can take a grain. Call this before create_chart.",
        "parameters": { "type": "object", "properties": {
            "mart": { "type": "string", "description": "mart name, e.g. mart_sales" } } } } })
}

/// The `referenceLines` of the chart tools (`BI-11`, `BI-17`): a fixed or
/// parameter value, a line computed from the rows on screen, or a band.
fn reference_lines_schema() -> Value {
    json!({ "type": "array", "maxItems": 5, "items": { "type": "object", "properties": {
        "kind": { "type": "string", "enum": ["value", "average", "median", "min", "max", "trend", "band"],
            "description": "value (default): a line at `value`; average, median, min, max: computed from the rows the chart shows; trend: a linear trend over them; band: a shaded band from `value` to `to`" },
        "value": { "oneOf": [ { "type": "number" }, param_ref_schema() ], "description": "where on the value axis: a number, or {param: <a number parameter>}. Required for kind value and band; leave out for a computed line" },
        "to": { "oneOf": [ { "type": "number" }, param_ref_schema() ], "description": "band only: the other edge" },
        "label": { "type": "string", "description": "at most 60 characters" },
        "color": { "type": "string", "description": "#rrggbb" },
        "measure": { "type": "string", "description": "a computed line: which value of the chart it describes (a result name); default the first" } } },
        "description": "up to 5 marks on the value axis (bar, hbar, line, area, stacked, combo), for example a target at 1,000, the average, a trend or a band. Use a parameter for a line the viewer can move. A computed line describes the rows on screen" })
}

/// The `BI-17` part A formatting fields of `create_chart`, `update_chart` and
/// `format_chart` (the style block of `lakehouse_bi::style`), written once so
/// the three cannot drift. The limits are the ones `style::check_style` enforces.
fn style_properties() -> Value {
    let value_format = json!({ "type": "object", "properties": {
        "style": { "type": "string", "enum": ["auto", "number", "percent", "currency", "scientific", "compact"], "description": "percent multiplies by 100 (0.125 is 12.5%); compact is 1.2 K, 3.4 M, 5 B, 2 T; auto is how it looked before" },
        "decimals": { "type": "integer", "minimum": 0, "maximum": 10 },
        "thousands": { "type": "boolean", "description": "group thousands (default true)" },
        "currency": { "type": "string", "description": "an ISO 4217 code such as IDR or USD, upper case; with the currency or compact style. Rupiah is IDR" },
        "currencyDisplay": { "type": "string", "enum": ["symbol", "code"] },
        "prefix": { "type": "string", "description": "at most 16 characters" },
        "suffix": { "type": "string", "description": "at most 16 characters" } } });
    let axis = json!({ "type": "object", "properties": {
        "title": { "type": "string", "description": "at most 60 characters" },
        "show": { "type": "boolean" },
        "min": { "type": "number" }, "max": { "type": "number" },
        "log": { "type": "boolean", "description": "logarithmic scale; every value must be above zero" },
        "rotate": { "type": "integer", "enum": [0, 45, 90], "description": "label rotation, category axis" } } });
    json!({
        "format": { "oneOf": [value_format, { "type": "null" }], "description": "how every number of the chart is written (the number format). null removes it. Changing it with format_chart keeps everything else of the chart" },
        "valueFormats": { "type": ["object", "null"], "additionalProperties": value_format, "description": "a number format per value, keyed by the value's result name; replaces `format` for that value" },
        "dateFormat": { "type": ["object", "null"], "properties": {
            "style": { "type": "string", "enum": ["auto", "short", "medium", "long", "iso", "monthYear"] },
            "hour12": { "type": "boolean" }, "abbreviate": { "type": "boolean", "description": "short month and day names" } },
            "description": "how dates and timestamps are written" },
        "colors": { "type": ["object", "null"], "properties": {
            "palette": { "type": "string", "enum": ["default", "bright", "muted", "sequential-blue", "diverging-red-blue", "company"] },
            "series": { "type": "object", "additionalProperties": { "type": "string" }, "description": "series name (or the value's result name) to #rrggbb; \"make the bars green\" is the chart's value name with a green" },
            "byValue": { "type": "array", "minItems": 2, "maxItems": 3, "items": { "type": "string" }, "description": "colour by value: 2 or 3 #rrggbb stops (one value, no breakdown; bar, hbar, pie, treemap, geomap)" } },
            "description": "colours; a field given here replaces the whole colors setting" },
        "axes": { "type": ["object", "null"], "properties": {
            "x": axis, "y": axis, "y2": axis,
            "rightValues": { "type": "array", "items": { "type": "string" }, "description": "result names drawn against a second Y axis (bar, line, area, combo with at least two values)" } },
            "description": "axis titles, limits, log scale, label rotation, hiding; bar, hbar, line, area, stacked, combo, scatter, bubble, waterfall, boxplot" },
        "labels": { "type": ["object", "null"], "properties": { "show": { "type": "boolean" } }, "description": "write each value on its bar or point, in its format" },
        "annotations": { "type": ["array", "null"], "maxItems": 20, "items": { "type": "object", "properties": {
            "x": { "type": "string", "description": "the category or date the note is pinned to, as the chart shows it" },
            "series": { "type": "string" },
            "text": { "type": "string", "description": "plain text, at most 140 characters" } }, "required": ["x", "text"] },
            "description": "short notes pinned to points (bar, hbar, line, area, stacked, combo, waterfall)" },
        "tooltip": { "type": ["object", "null"], "properties": { "fields": { "type": "array", "items": { "type": "string" } } },
            "description": "show only these of the chart's values in the tooltip (result names)" },
        "conditional": { "type": ["array", "null"], "maxItems": 20, "items": { "type": "object", "properties": {
            "column": { "type": "string" },
            "rule": { "type": "object", "description": "{type:\"compare\",op:lt|le|gt|ge|eq|ne,value}, {type:\"between\",from,to}, {type:\"contains\",text} or {type:\"range\",colors:[2 or 3 #rrggbb]}" },
            "color": { "type": "string", "description": "#rrggbb; not for a range" },
            "target": { "type": "string", "enum": ["cell", "text"] } }, "required": ["column", "rule"] },
            "description": "table, pivot and kpi only: colour a cell or the value by a rule, for example below 1,000 in red" }
    })
}

/// Insert [`style_properties`] into a chart tool's schema, the way
/// [`with_shape_properties`] inserts the `BI-6` fields.
fn with_style_properties(mut schema: Value) -> Value {
    if let Some(props) = schema
        .pointer_mut("/function/parameters/properties")
        .and_then(Value::as_object_mut)
        && let Value::Object(extra) = style_properties()
    {
        props.extend(extra);
    }
    schema
}

fn format_chart_schema() -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "chart".to_owned(),
        json!({ "type": "string", "description": "the id of the chart (list_charts)" }),
    );
    if let Value::Object(style) = style_properties() {
        properties.extend(style);
    }
    properties.insert("referenceLines".to_owned(), reference_lines_schema());
    json!({ "type": "function", "function": { "name": "format_chart",
        "description": "Change only how a saved chart looks: number and date formats (rupiah, percent, compact, decimals), colours (a palette, a colour per series, colour by value), axes, data labels, reference lines (a target at 1,000, an average, a trend, a band), notes, tooltip fields and, for a table or KPI, conditional colours. Send ONLY the fields that change: a field you give replaces that field, null removes it, and everything you leave out stays as it is (the data, the title, the filters and the other formatting). list_charts gives the ids. To change what the chart shows, use update_chart.",
        "parameters": { "type": "object", "properties": properties, "required": ["chart"] } } })
}

fn set_dashboard_theme_schema() -> Value {
    json!({ "type": "function", "function": { "name": "set_dashboard_theme",
        "description": "Set a dashboard's theme and font for its tiles, wherever it is shown (the console, public links, embeds). theme: auto follows the console, light or dark fixes it. font: one of the fonts an administrator allows (inter, ibm-plex-sans, source-serif, jetbrains-mono, system), or an empty string for the console's own. Not for the built-in dashboard.",
        "parameters": { "type": "object", "properties": {
            "board": { "type": "string", "description": "the dashboard id (list_boards)" },
            "theme": { "type": "string", "enum": ["auto", "light", "dark"] },
            "font": { "type": "string", "enum": ["", "inter", "ibm-plex-sans", "source-serif", "jetbrains-mono", "system"] } },
            "required": ["board"] } } })
}

fn create_chart_schema() -> Value {
    with_type_properties(with_style_properties(with_shape_properties(
        with_table_properties(
            json!({ "type": "function", "function": { "name": "create_chart",
        "description": "Create a chart card on a dashboard (/dashboards) from a Gold mart OR a saved SQL source (sqlSource, for data that combines several marts). The server writes the SQL from the columns you pick; you do not write SQL. Call describe_mart (or list_sql_sources) first to use columns that exist.",
        "parameters": { "type": "object", "properties": {
            "title": { "type": "string" }, "subtitle": { "type": "string" },
            "mart": { "type": "string" }, "kind": { "type": "string", "enum": chart_kind_enum() },
            "sqlSource": { "type": "string", "description": "SQL source id (s_…) from list_sql_sources, instead of mart for a chart that combines several marts; set mart OR sqlSource, not both" },
            "text": { "type": "string" }, "caption": { "type": "string" },
            "target": { "type": "number" }, "dimension": { "type": "string" },
            "measures": measures_schema(),
            "breakdown": { "type": "string" },
            "map": { "type": "string", "description": "map kinds only (geomap, pointmap, geoheat): id of a bundled map, one of dki-jakarta (Jakarta cities), id-provinces (Indonesian provinces), id-regencies (Indonesian kabupaten/kota)" },
            "lat": { "type": "string", "description": "pointmap and geoheat only (required): the latitude column" },
            "lon": { "type": "string", "description": "pointmap and geoheat only (required): the longitude column" },
            "aggregate": { "type": "string", "enum": ["sum", "avg", "max", "min", "count"] },
            "limit": { "type": "number" }, "span": { "type": "number", "enum": [1, 2] },
            "click": { "type": "object", "description": "what a click on the chart does instead of the drill menu (not for text): {\"kind\":\"dashboard\",\"board\":<dashboard id>,\"column\":<column of that dashboard the clicked value filters>}, {\"kind\":\"query\",\"id\":<saved query id>}, or {\"kind\":\"url\",\"url\":<https://… or a /path; {value} is replaced by the clicked value>}" },
            "grain": { "type": "string", "enum": grain_enum(), "description": GRAIN_DESCRIPTION },
            "board": { "type": "string" } },
            "required": ["title", "kind"] } } }),
        ),
    )))
}

fn update_chart_schema() -> Value {
    with_type_properties(with_style_properties(with_shape_properties(
        with_table_properties(
            json!({ "type": "function", "function": { "name": "update_chart",
        "description": "Change a saved chart (by id), keeping its id. Send every field, as for create_chart, with the new values. list_charts gives the ids.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string" }, "title": { "type": "string" }, "subtitle": { "type": "string" },
            "mart": { "type": "string" }, "kind": { "type": "string", "enum": chart_kind_enum() },
            "sqlSource": { "type": "string", "description": "SQL source id (s_…) from list_sql_sources, instead of mart for a chart that combines several marts; set mart OR sqlSource, not both" },
            "dimension": { "type": "string" },
            "measures": measures_schema(),
            "breakdown": { "type": "string" }, "caption": { "type": "string" },
            "map": { "type": "string", "description": "map kinds only (geomap, pointmap, geoheat): id of a bundled map, one of dki-jakarta, id-provinces, id-regencies" },
            "lat": { "type": "string", "description": "pointmap and geoheat only (required): the latitude column" },
            "lon": { "type": "string", "description": "pointmap and geoheat only (required): the longitude column" },
            "target": { "type": "number" },
            "aggregate": { "type": "string", "enum": ["sum", "avg", "max", "min", "count"] },
            "limit": { "type": "number" }, "span": { "type": "number", "enum": [1, 2] },
            "click": { "type": "object", "description": "what a click on the chart does instead of the drill menu (not for text): {\"kind\":\"dashboard\",\"board\":<dashboard id>,\"column\":<column of that dashboard the clicked value filters>}, {\"kind\":\"query\",\"id\":<saved query id>}, or {\"kind\":\"url\",\"url\":<https://… or a /path; {value} is replaced by the clicked value>}" },
            "grain": { "type": "string", "enum": grain_enum(), "description": GRAIN_DESCRIPTION },
            "board": { "type": "string" } },
            "required": ["id", "title", "kind"] } } }),
        ),
    )))
}

fn create_board_schema() -> Value {
    json!({ "type": "function", "function": { "name": "create_board",
        "description": "Create a new named dashboard (board). Returns its id for create_chart.",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string" },
            "description": { "type": "string", "description": "One-line purpose of the board, shown on the dashboard list page." } },
            "required": ["name"] } } })
}

fn list_boards_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_boards",
        "description": "List the dashboards (boards) that exist.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn suggest_dashboard_schema() -> Value {
    json!({ "type": "function", "function": { "name": "suggest_dashboard",
        "description": "Every Gold mart with its dimensions and measures at once, for proposing a set of dashboard charts.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn list_charts_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_charts",
        "description": "List the chart cards saved on dashboards.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn suggest_join_schema() -> Value {
    json!({ "type": "function", "function": { "name": "suggest_join",
        "description": "Candidate column pairs for joining two Gold marts in a chart (`joins` of create_chart), best first: both columns in their tables' sorting keys with the same name and kind, then the same name and kind, then the same name. `suggested` marks the pairs to use; `coversRightKey` false means a row of the first mart may match several rows of the second, so sums of the second mart's columns multiply. Reads column metadata only.",
        "parameters": { "type": "object", "properties": {
            "left": { "type": "string", "description": "the chart's mart, e.g. mart_visits" },
            "right": { "type": "string", "description": "the mart to join to it" } },
            "required": ["left", "right"] } } })
}

fn list_sql_sources_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_sql_sources",
        "description": "List the dashboard SQL sources (saved SQL, usually combining several Gold marts) with their dimensions and measures. Use the id as sqlSource in create_chart. Users create SQL sources in Query Studio, not in chat.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn delete_chart_schema() -> Value {
    json!({ "type": "function", "function": { "name": "delete_chart",
        "description": "Delete one saved chart card (by id). Built-in charts cannot be deleted. Needs human approval before it runs.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

// ── BI-18 part C tabs (with AI-9) ─────────────────────────────────────────

fn create_tab_schema() -> Value {
    json!({ "type": "function", "function": { "name": "create_tab",
        "description": "Add a tab to a dashboard (board). On a dashboard with no tabs the first tab holds every chart already on it, so to separate some charts create a second tab as well and then move_charts_to_tab. The built-in dashboard has no tabs.",
        "parameters": { "type": "object", "properties": {
            "board": { "type": "string", "description": "dashboard id from list_boards" },
            "title": { "type": "string", "description": "1 to 60 characters" } },
            "required": ["board", "title"] } } })
}

fn rename_tab_schema() -> Value {
    json!({ "type": "function", "function": { "name": "rename_tab",
        "description": "Rename a tab of a dashboard. The tab keeps its id, so charts and filters stay on it.",
        "parameters": { "type": "object", "properties": {
            "board": { "type": "string" },
            "tab": { "type": "string", "description": "the tab's id or its title, from list_boards" },
            "title": { "type": "string", "description": "the new title, 1 to 60 characters" } },
            "required": ["board", "tab", "title"] } } })
}

fn move_charts_to_tab_schema() -> Value {
    json!({ "type": "function", "function": { "name": "move_charts_to_tab",
        "description": "Put charts of a dashboard on one of its tabs, below what is already there. Charts are picked by id (list_charts); every one must be on this dashboard.",
        "parameters": { "type": "object", "properties": {
            "board": { "type": "string" },
            "tab": { "type": "string", "description": "the tab's id or its title" },
            "chartIds": { "type": "array", "items": { "type": "string" }, "description": "ids of the charts to move" } },
            "required": ["board", "tab", "chartIds"] } } })
}

fn move_tab_schema() -> Value {
    json!({ "type": "function", "function": { "name": "move_tab",
        "description": "Change the order of a dashboard's tabs. Give `tabs` (every tab, in the new order) or `tab` with `position` (1 is first). Tab ids, titles and the charts on each stay as they are.",
        "parameters": { "type": "object", "properties": {
            "board": { "type": "string" },
            "tabs": { "type": "array", "items": { "type": "string" }, "description": "every tab's id or title, in the new order" },
            "tab": { "type": "string", "description": "the tab's id or its title, to move to `position`" },
            "position": { "type": "integer", "minimum": 1, "description": "where the tab goes, 1 for first" } },
            "required": ["board"] } } })
}

fn delete_tab_schema() -> Value {
    json!({ "type": "function", "function": { "name": "delete_tab",
        "description": "Delete a tab of a dashboard. Its charts move to the first remaining tab (none are deleted); a filter tied to it is tied to that tab instead. Needs human approval before it runs.",
        "parameters": { "type": "object", "properties": {
            "board": { "type": "string" },
            "tab": { "type": "string", "description": "the tab's id or its title" } },
            "required": ["board", "tab"] } } })
}

// ── BI-8 calculated fields (with AI-4) ───────────────────────────────────

/// The properties that name the source of a calculated field, the same on
/// every tool that takes one.
fn calc_source_properties() -> Value {
    json!({
        "mart": { "type": "string", "description": "Gold mart name (from describe_mart); set mart OR sqlSource, not both" },
        "sqlSource": { "type": "string", "description": "SQL source id (s_…) from list_sql_sources, instead of mart" }
    })
}

fn list_formula_functions_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_formula_functions",
        "description": "The formula language of calculated fields and every function it has (name, signature, one line of help, an example). Call it before writing a formula. You write formulas, never SQL.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn list_metrics_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_metrics",
        "description": "The saved metrics (name, label, description, the table or SQL source they are computed from, their date column). A metric is one agreed definition of a number such as revenue. Call it BEFORE writing a query or a chart for a number the user names: if one matches, use it and name it in your answer; if none matches, say so before you write SQL.",
        "parameters": { "type": "object", "properties": calc_source_properties() } } })
}

fn describe_metric_schema() -> Value {
    json!({ "type": "function", "function": { "name": "describe_metric",
        "description": "One metric in full: its formula, filters, date column and default grain, and the charts and dashboards that use it.",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string", "description": "the metric's name from list_metrics" } },
            "required": ["name"] } } })
}

fn query_metric_schema() -> Value {
    json!({ "type": "function", "function": { "name": "query_metric",
        "description": "The value of a metric, read as the person you are helping (their access, masks and row filters apply). With no other argument it is one number. groupBy splits it by one column; grain (day, week, month, quarter, year) splits it by that period of the metric's date column; filters narrow it further. Say which metric you used.",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string", "description": "the metric's name from list_metrics" },
            "groupBy": { "type": "string", "description": "a column of the metric's table; with grain, a date column" },
            "grain": { "type": "string", "enum": ["day", "week", "month", "quarter", "year"] },
            "filters": filters_schema("filters on columns of the metric's table, or a saved segment {segment}") },
            "required": ["name"] } } })
}

fn metric_properties() -> Value {
    let mut properties = calc_source_properties();
    properties["label"] =
        json!({ "type": "string", "description": "what people read, for example Total visitors" });
    properties["description"] = json!({ "type": "string", "description": "what it counts and how, at most 500 characters" });
    properties["formula"] = json!({ "type": "string", "description": "an AGGREGATE in the formula language, for example Sum([visitors]); never SQL (see list_formula_functions)" });
    properties["filters"] = filters_schema(
        "the metric's own filters, applied to this metric's number only (or a saved segment {segment})",
    );
    properties["timeColumn"] = json!({ "type": "string", "description": "a date or timestamp column its period comparisons run on" });
    properties["grain"] = json!({ "type": "string", "enum": ["day", "week", "month", "quarter", "year"], "description": "its default period" });
    properties
}

fn create_metric_schema() -> Value {
    let mut properties = metric_properties();
    properties["name"] = json!({ "type": "string", "description": "letters, digits and _ (for example total_visitors), unique, not a column; charts pick the metric by it and it cannot change later" });
    json!({ "type": "function", "function": { "name": "create_metric",
        "description": "Save a reusable metric on a mart or SQL source. Only when the user asks for one; check list_metrics first so there is one definition per number. Needs the metric:write permission.",
        "parameters": { "type": "object", "properties": properties, "required": ["name", "label", "formula"] } } })
}

fn update_metric_schema() -> Value {
    let mut properties = metric_properties();
    properties["id"] = json!({ "type": "string", "description": "from list_metrics" });
    json!({ "type": "function", "function": { "name": "update_metric",
        "description": "Change a metric (by id). The name and the source stay; every chart that uses it follows the change at once, so say so. Send the whole definition.",
        "parameters": { "type": "object", "properties": properties, "required": ["id", "label", "formula"] } } })
}

fn delete_metric_schema() -> Value {
    json!({ "type": "function", "function": { "name": "delete_metric",
        "description": "Delete one metric (by id). Refused while a chart uses it. Needs human approval before it runs.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

fn list_calculated_fields_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_calculated_fields",
        "description": "The calculated fields of one mart or SQL source (name, formula, whether it is one value per row or an aggregate, and its type). A chart uses one by naming it in dimension, breakdown or measures like a column.",
        "parameters": { "type": "object", "properties": calc_source_properties() } } })
}

fn validate_formula_schema() -> Value {
    let mut properties = calc_source_properties();
    properties["formula"] = json!({ "type": "string", "description": "the formula to check, in the formula language (see list_formula_functions)" });
    properties["name"] = json!({ "type": "string", "description": "the field's name, if it already has one (so a field that uses itself is caught)" });
    json!({ "type": "function", "function": { "name": "validate_formula",
        "description": "Check a formula against a mart or SQL source without saving anything: returns ok with its level and type, or the message and the character position of the first mistake. Use it to fix a formula before create_calculated_field.",
        "parameters": { "type": "object", "properties": properties, "required": ["formula"] } } })
}

fn create_calculated_field_schema() -> Value {
    let mut properties = calc_source_properties();
    properties["name"] = json!({ "type": "string", "description": "a name of letters, digits and _ (for example profit), not a column of the source; charts pick the field by it" });
    properties["formula"] = json!({ "type": "string", "description": "for example [revenue] - [cost], or Sum([revenue]) / CountDistinct([customer]); never SQL" });
    json!({ "type": "function", "function": { "name": "create_calculated_field",
        "description": "Save a calculated field on a mart or SQL source from a formula in the formula language (see list_formula_functions); the server checks and compiles it. Then use its name in create_chart like a column: a per-row field can be a dimension, breakdown or measure; an aggregate field only a measure.",
        "parameters": { "type": "object", "properties": properties, "required": ["name", "formula"] } } })
}

fn update_calculated_field_schema() -> Value {
    json!({ "type": "function", "function": { "name": "update_calculated_field",
        "description": "Change the formula of a calculated field (by id from list_calculated_fields). The name stays; every chart using the field follows the new formula.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string" },
            "formula": { "type": "string", "description": "the new formula" } },
            "required": ["id", "formula"] } } })
}

fn delete_calculated_field_schema() -> Value {
    json!({ "type": "function", "function": { "name": "delete_calculated_field",
        "description": "Delete one calculated field (by id). Refused while a chart or another field uses it. Needs human approval before it runs.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

// ── BI-10 value groups and hierarchies (with AI-6) ───────────────────────

fn create_value_group_schema() -> Value {
    let mut properties = calc_source_properties();
    properties["name"] = json!({ "type": "string", "description": "a name of letters, digits and _ (for example region_group), not a column of the source; charts pick the group by it like a column" });
    properties["group"] = json!({ "type": "object", "properties": {
        "column": { "type": "string", "description": "a text column of the source" },
        "groups": { "type": "array", "minItems": 1, "maxItems": 50, "items": { "type": "object", "properties": {
            "name": { "type": "string", "description": "what the values are shown as, for example Java" },
            "values": { "type": "array", "items": { "type": "string" }, "description": "values of the column exactly as spelled; a value belongs to one group only; at most 2000 in all" } },
            "required": ["name", "values"] } },
        "rest": { "type": "string", "enum": ["keep", "other"], "description": "keep: values in no group keep their own name; other: they are shown as Other" } },
        "required": ["column", "groups", "rest"] });
    json!({ "type": "function", "function": { "name": "create_value_group",
        "description": "Gather chosen values of a text column under names of your own (for example six provinces as Java) and save it as a field of the source. Then use its name in create_chart as dimension or breakdown, like a column. It is a text column, so not a measure. Take the values from run_sql or describe_mart, never invent them.",
        "parameters": { "type": "object", "properties": properties, "required": ["name", "group"] } } })
}

fn list_hierarchies_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_hierarchies",
        "description": "The hierarchies of one mart or SQL source: named, ordered lists of columns (for example province, city, place) that a chart can be drilled down through by clicking a value.",
        "parameters": { "type": "object", "properties": calc_source_properties() } } })
}

fn create_hierarchy_schema() -> Value {
    let mut properties = calc_source_properties();
    properties["name"] =
        json!({ "type": "string", "description": "for example Geography (1 to 64 characters)" });
    properties["levels"] = json!({ "type": "array", "minItems": 2, "maxItems": 6, "items": { "type": "string" }, "description": "2 to 6 columns of the source, outermost first (province, city, place)" });
    json!({ "type": "function", "function": { "name": "create_hierarchy",
        "description": "Save a hierarchy on a mart or SQL source. A viewer can then drill down a chart whose dimension is one of its levels. Charts do not store it, so it never breaks a chart.",
        "parameters": { "type": "object", "properties": properties, "required": ["name", "levels"] } } })
}

fn delete_hierarchy_schema() -> Value {
    json!({ "type": "function", "function": { "name": "delete_hierarchy",
        "description": "Delete one hierarchy (by id from list_hierarchies). No chart depends on it. Needs human approval before it runs.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

// ── BI-11 parameters ──────────────────────────────────────────────────────

fn list_parameters_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_parameters",
        "description": "The parameters of the deployment: named values (a number, text, a date, or a choice from a list) that a viewer changes on a dashboard and that a formula ({name}), a chart's limit, a reference line or a column switch can read. Use before create_chart to reuse one.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn parameter_properties() -> Value {
    json!({
        "name": { "type": "string", "description": "letters, digits and _, at most 64; a formula writes it as {name}. Cannot be changed later" },
        "label": { "type": "string", "description": "what the control shows" },
        "type": { "type": "string", "enum": ["number", "text", "date", "list"], "description": "cannot be changed later" },
        "default": { "type": ["number", "string"], "description": "the value when nobody chose one; a date is YYYY-MM-DD; a list default is one of the values" },
        "min": { "type": ["number", "string"], "description": "number or date" },
        "max": { "type": ["number", "string"], "description": "number or date" },
        "maxLength": { "type": "number", "description": "text: at most 200" },
        "values": { "type": "array", "minItems": 1, "maxItems": 200, "items": { "type": "object", "properties": {
            "value": { "type": ["number", "string"] }, "label": { "type": "string" } },
            "required": ["value"] },
            "description": "list: the choices, all text or all numbers. For a parameter that switches a chart's column, the values are column names of the chart's data (take them from describe_mart)" }
    })
}

fn create_parameter_schema() -> Value {
    json!({ "type": "function", "function": { "name": "create_parameter",
        "description": "Define a parameter. Then use it in a formula as {name}, as a chart's limit, as a reference line (referenceLines) or, for a list, as the dimension, breakdown or a measure's column.",
        "parameters": { "type": "object", "properties": parameter_properties(),
            "required": ["name", "type", "default"] } } })
}

fn update_parameter_schema() -> Value {
    let mut properties = parameter_properties();
    properties["id"] = json!({ "type": "string", "description": "from list_parameters" });
    json!({ "type": "function", "function": { "name": "update_parameter",
        "description": "Change a parameter (by id): its label, default, bounds or values. Send every field. Its name and type stay.",
        "parameters": { "type": "object", "properties": properties,
            "required": ["id", "name", "type", "default"] } } })
}

fn delete_parameter_schema() -> Value {
    json!({ "type": "function", "function": { "name": "delete_parameter",
        "description": "Delete one parameter (by id from list_parameters). Refused while a formula, a chart or a dashboard uses it. Needs human approval before it runs.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

// ── T1.1 Alerts ──────────────────────────────────────────────────────────

/// The five comparison operators an alert rule may use, mirroring
/// `lakehouse_alerts::AlertOp` — shared between [`create_alert_rule_schema`]
/// and [`update_alert_rule_schema`] so the two lists cannot drift apart.
fn alert_op_enum() -> Value {
    json!([">", ">=", "<", "<=", "=="])
}

/// The five aggregate functions an alert rule may watch, mirroring
/// `lakehouse_alerts::AGGS`.
fn alert_agg_enum() -> Value {
    json!(["sum", "avg", "max", "min", "count"])
}

fn list_alert_rules_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_alert_rules",
        "description": "List every alert rule (threshold alerts) and digest (scheduled summaries), with whether each is enabled.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn create_alert_rule_schema() -> Value {
    json!({ "type": "function", "function": { "name": "create_alert_rule",
        "description": "Create an alert or digest rule. Alert (type=alert): mart, measure, agg, op and threshold; it watches that aggregate. Or give metric (a name from list_metrics) with op and threshold, and the alert watches that metric instead (it is read with the fixed viewer role every alert uses, not with the user's access). Digest (type=digest): board, the dashboard to summarise. Delivered by webhook (target=URL) or email (target=address).",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string" },
            "type": { "type": "string", "enum": ["alert", "digest"] },
            "mart": { "type": "string", "description": "Gold mart name, for type=alert" },
            "measure": { "type": "string", "description": "measure column to watch, for type=alert" },
            "metric": { "type": "string", "description": "the name of a metric to watch, for type=alert, instead of mart/measure/agg" },
            "agg": { "type": "string", "enum": alert_agg_enum() },
            "op": { "type": "string", "enum": alert_op_enum() },
            "threshold": { "type": "number" },
            "board": { "type": "string", "description": "dashboard board id, for type=digest" },
            "channel": { "type": "string", "enum": ["webhook", "email"] },
            "target": { "type": "string", "description": "webhook URL or email address to deliver to" },
            "enabled": { "type": "boolean" } },
            "required": ["name", "type", "channel", "target"] } } })
}

fn update_alert_rule_schema() -> Value {
    json!({ "type": "function", "function": { "name": "update_alert_rule",
        "description": "Change a saved alert or digest rule (by id). Send every field, as for create_alert_rule, with the new values. list_alert_rules gives the ids.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string" },
            "name": { "type": "string" },
            "type": { "type": "string", "enum": ["alert", "digest"] },
            "mart": { "type": "string" },
            "measure": { "type": "string" },
            "metric": { "type": "string", "description": "the name of a metric to watch instead of mart/measure/agg" },
            "agg": { "type": "string", "enum": alert_agg_enum() },
            "op": { "type": "string", "enum": alert_op_enum() },
            "threshold": { "type": "number" },
            "board": { "type": "string" },
            "channel": { "type": "string", "enum": ["webhook", "email"] },
            "target": { "type": "string" },
            "enabled": { "type": "boolean" } },
            "required": ["id"] } } })
}

fn delete_alert_rule_schema() -> Value {
    json!({ "type": "function", "function": { "name": "delete_alert_rule",
        "description": "Delete an alert or digest rule permanently (by id). Needs human approval before it runs.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

fn run_alert_rule_schema() -> Value {
    json!({ "type": "function", "function": { "name": "run_alert_rule",
        "description": "Evaluate one alert or digest rule now (by id). If its condition holds, the webhook or email is really sent.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

// ── T1.2 Connectors ──────────────────────────────────────────────────────

fn connector_direction_enum() -> Value {
    json!(["source", "sink", "bidirectional"])
}

fn list_connectors_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_connectors",
        "description": "List the registered data connectors (sources and targets) and their health.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn create_connector_schema() -> Value {
    json!({ "type": "function", "function": { "name": "create_connector",
        "description": "Register a new connector. The server creates the connector id and derives the credential reference names from it: choose a source (env/file) and kind per slot, never send a secret or secretRef. The names the operator must provide are returned once.",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string" },
            "type": { "type": "string", "description": "e.g. PostgreSQL, Object storage, Kafka" },
            "direction": { "type": "string", "enum": connector_direction_enum() },
            "host": { "type": "string", "description": "connection target (host:port or endpoint)" },
            "credential": { "type": "object", "description": "credential spec; the server derives the reference names from the connector id", "properties": {
                "source": { "type": "string", "enum": ["env", "file"] },
                "primary": { "type": "string", "enum": ["password", "secret_key", "access_key", "api_key", "token", "private_key"] },
                "secondary": { "type": "string", "enum": ["password", "secret_key", "access_key", "api_key", "token", "private_key"], "description": "second slot, e.g. an S3 secret key; optional" } },
                "required": ["source", "primary"] },
            "environment": { "type": "string" },
            "tenant": { "type": "string" },
            "residency": { "type": "string" },
            "capabilities": { "type": "array", "items": { "type": "string" } },
            "owner": { "type": "string" } },
            "required": ["name", "type", "direction", "host", "credential", "environment", "tenant"] } } })
}

fn test_connector_schema() -> Value {
    json!({ "type": "function", "function": { "name": "test_connector",
        "description": "Run a real connection test on a connector (by id). PostgreSQL, S3-compatible object storage, MySQL/MariaDB, SQL Server and REST sources can be dialled; any other type returns supported:false instead of a made-up result.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

fn delete_connector_schema() -> Value {
    json!({ "type": "function", "function": { "name": "delete_connector",
        "description": "Delete a connector registration permanently (by id). Needs human approval before it runs.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

// ── T1.3 Pipelines (additions) ───────────────────────────────────────────

fn list_pipelines_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_pipelines",
        "description": "List every pipeline (Dagster jobs and pipelines authored in the console) with its latest status (e.g. failed, completed), schedule, and last and next run. Use it to answer which pipelines are failing or healthy.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn list_pipeline_runs_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_pipeline_runs",
        "description": "The latest runs (up to 30) of one pipeline (by id), with status and times.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

fn trigger_pipeline_schema() -> Value {
    // R4 plan 2c: `runConfig` is an OPTIONAL JSON object of run-time
    // config the job accepts (the same shape `POST /api/pipelines/{id}/
    // trigger` takes when supplied). The schema lists it as
    // `additionalProperties: true` — Dagster's per-job schema is the
    // source of truth for what's allowed; this is a tool definition,
    // not a validator.
    json!({ "type": "function", "function": { "name": "trigger_pipeline",
        "description": "Run one pipeline now (by id). Different from trigger_lakehouse_build, which always runs the main Bronze -> Silver -> Gold rebuild. Optional runConfig (object) overrides the job's defaults — validated against the job's config schema before launch; an invalid config fails with a structured 400.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string" },
            "runConfig": { "type": "object", "additionalProperties": true }
        },
        "required": ["id"] } } })
}

fn retry_pipeline_run_schema() -> Value {
    // Plan 1c (R2, day-1): `stepKeys` lets the model ask for a specific
    // subset of steps rather than "everything" or "failed only". Each
    // key is verified against the parent run's `run_steps` by the
    // route layer (a typo here surfaces as 400, not a silent no-op).
    // `fromFailure` and `stepKeys` are mutually exclusive in practice
    // — when both arrive the tool uses `stepKeys` (the more specific
    // intent); the schema lists both as the model has to see them
    // both to make the choice.
    json!({ "type": "function", "function": { "name": "retry_pipeline_run",
        "description": "Re-run a finished pipeline run (by runId): every step, only the failed steps (with fromFailure, failed runs only), or a named subset of steps (with stepKeys).",
        "parameters": { "type": "object", "properties": {
            "runId": { "type": "string" },
            "fromFailure": { "type": "boolean" },
            "stepKeys": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["runId"] } } })
}

fn pause_pipeline_schema() -> Value {
    json!({ "type": "function", "function": { "name": "pause_pipeline",
        "description": "Pause a pipeline's schedule (by id). Needs human approval before it runs.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

fn resume_pipeline_schema() -> Value {
    json!({ "type": "function", "function": { "name": "resume_pipeline",
        "description": "Resume a paused pipeline's schedule (by id).",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

fn cancel_pipeline_run_schema() -> Value {
    json!({ "type": "function", "function": { "name": "cancel_pipeline_run",
        "description": "Stop a running pipeline run (by runId). Needs human approval before it runs.",
        "parameters": { "type": "object", "properties": { "runId": { "type": "string" } },
            "required": ["runId"] } } })
}

// ── T1.4 Saved queries ───────────────────────────────────────────────────

fn save_query_schema() -> Value {
    json!({ "type": "function", "function": { "name": "save_query",
        "description": "Save a SQL query under a name, to run again later with run_saved_query.",
        "parameters": { "type": "object", "properties": {
            "title": { "type": "string" },
            "sql": { "type": "string", "description": "A ClickHouse SELECT statement" },
            "tags": { "type": "array", "items": { "type": "string" } },
            "owner": { "type": "string" } },
            "required": ["title", "sql"] } } })
}

fn list_saved_queries_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_saved_queries",
        "description": "List the saved queries.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn run_saved_query_schema() -> Value {
    json!({ "type": "function", "function": { "name": "run_saved_query",
        "description": "Run a saved query (by id) and return its rows. Only read-only statements run, as in Query Studio.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

// ── T2.1 Governance reads ────────────────────────────────────────────────

fn get_audit_history_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_audit_history",
        "description": "Audit history: Dagster pipeline runs plus copilot and console actions (who did what, when, and the outcome).",
        "parameters": { "type": "object", "properties": {} } } })
}

fn list_classification_rules_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_classification_rules",
        "description": "Data classification per asset and column (public/internal/confidential/restricted): what was observed plus the rules that were written.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn list_quality_rules_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_quality_rules",
        "description": "Data quality rules (completeness, uniqueness, ...) and their last results: what was observed plus the rules that were written.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn get_cdc_health_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_cdc_health",
        "description": "Health of CDC replication slots per connector: status, lag and WAL retained, to catch a stuck slot before it fills the source database's disk.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn get_maintenance_metrics_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_maintenance_metrics",
        "description": "History of Bronze Iceberg maintenance runs (remove_orphan_files): orphan data and manifest files removed per table. expire_snapshots is refused by ClickHouse for catalog-managed Iceberg tables and is recorded as skipped. Read-only.",
        "parameters": { "type": "object", "properties": {} } } })
}

// ── T2.2 Maintenance ─────────────────────────────────────────────────────
//
// See the copilot-operations-handover plan, section 3.7 correction C2:
// there is no dry-run-only trigger. `bronze_maintenance_job`
// (`dagster/dispar_orchestrate/maintenance.py:297-299`) always runs a dry
// pass AND THEN the applied pass in the same job, and
// `DgClient::launch_run` takes a job name only, with no run-config
// override to split the two. So there is exactly ONE maintenance tool,
// `WriteHigh` (it genuinely deletes orphan Iceberg data/manifest files),
// never advertised as a "dry run".

fn run_bronze_maintenance_schema() -> Value {
    json!({ "type": "function", "function": { "name": "run_bronze_maintenance",
        "description": "Run Bronze maintenance now (Dagster bronze_maintenance_job). This APPLIES changes: orphan Iceberg data and manifest files are deleted. It is not a dry run. Needs human approval before it runs.",
        "parameters": { "type": "object", "properties": {} } } })
}

// ── T2.3 Workloads ───────────────────────────────────────────────────────

fn list_workloads_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_workloads",
        "description": "ClickHouse queries running right now, with the ids (\"w-<n>\") kill_query uses.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn kill_query_schema() -> Value {
    json!({ "type": "function", "function": { "name": "kill_query",
        "description": "Stop one running ClickHouse query (a real KILL QUERY, by id from list_workloads, e.g. \"w-0\"). Needs human approval before it runs.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

// ── T2.4 Gold export ─────────────────────────────────────────────────────

fn export_gold_mart_schema() -> Value {
    json!({ "type": "function", "function": { "name": "export_gold_mart",
        "description": "Export one Gold mart (serving.<mart>) to its Iceberg Gold table through Lakekeeper. Append-only: running it again adds the rows again with a new _exported_at; it does not replace them.",
        "parameters": { "type": "object", "properties": {
            "mart": { "type": "string", "description": "Gold mart name, e.g. mart_sales" } },
            "required": ["mart"] } } })
}

fn get_gold_export_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_gold_export",
        "description": "Read a Gold Iceberg table back (by mart) through Lakekeeper: its row count and format version, independent of ClickHouse.",
        "parameters": { "type": "object", "properties": {
            "mart": { "type": "string", "description": "Gold mart name, e.g. mart_sales" } },
            "required": ["mart"] } } })
}

// ── T2.5 Governance draft tools ──────────────────────────────────────────
//
// All three create a record in the least-active state the underlying
// store can express, never anything a human would recognise as "already
// active": `draft_policy` always forces `activate: false` (status
// `"draft"` — `policy.status_check` also allows `"ready"`, which this
// tool never produces). `quality_rule`/`classification_rule` have NO
// activation concept in the schema at all (`0003_governance.sql`: no
// `enabled`/`active` column, no `POST .../activate` route anywhere in
// `routes::governance`) — every row `create_classification_rule` inserts
// starts `review_status = 'needs-review'` (an authored-but-unevaluated
// fact, per `lakehouse_store::governance`'s module doc comment), and
// every row `create_quality_rule` inserts leaves the NOT NULL
// `last_status`/`last_run_at` columns at their `'warning'`/`now()`
// placeholder defaults, which the API never surfaces — no evaluator
// exists anywhere in the workspace, so `QualityRule` always reports both
// as `null` (WS1 finding J18). Either way there is no API path in this
// codebase that ever promotes one further. So these two tools cannot
// accidentally create something "more active" than a human clicking the
// same console form would — draft-only is already the only state
// reachable.

fn draft_policy_schema() -> Value {
    json!({ "type": "function", "function": { "name": "draft_policy",
        "description": "Draft a new governance policy. It is always saved as a draft, never active; activating it stays a human action in the console (Governance -> Policies).",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string" },
            "kind": { "type": "string", "description": "e.g. \"Row filter\", \"Agent autonomy\"" },
            "subjects": { "type": "string", "description": "who or what the policy applies to" },
            "resources": { "type": "string", "description": "which data the policy covers" },
            "effect": { "type": "string", "description": "e.g. \"Permit with obligation\", \"Require approval\"" },
            "conditions": { "type": "string" },
            "owner": { "type": "string" } },
            "required": ["name", "kind", "subjects", "resources", "effect"] } } })
}

fn draft_classification_rule_schema() -> Value {
    json!({ "type": "function", "function": { "name": "draft_classification_rule",
        "description": "Write a new classification or masking rule for an asset (optionally one column). It is saved as needs-review; review stays a human action in the console.",
        "parameters": { "type": "object", "properties": {
            "asset": { "type": "string" },
            "column": { "type": "string" },
            "classification": { "type": "string", "enum": ["public", "internal", "confidential", "restricted"] },
            "maskingRule": { "type": "string" } },
            "required": ["asset", "classification"] } } })
}

fn draft_quality_rule_schema() -> Value {
    json!({ "type": "function", "function": { "name": "draft_quality_rule",
        "description": "Write a new data quality rule for an asset. A new rule is saved as a warning (not yet evaluated).",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string" },
            "asset": { "type": "string" },
            "dimension": { "type": "string", "description": "e.g. completeness, uniqueness, accuracy" },
            "threshold": { "type": "string", "description": "e.g. \">= 95%\"" },
            "severity": { "type": "string", "enum": ["critical", "high", "medium", "low", "info"] } },
            "required": ["name", "asset", "dimension", "threshold", "severity"] } } })
}

fn get_ingest_spec_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_ingest_spec",
        "description": "Read a connector's ingest spec: its adapter (sql, cdc, files, rest, sheets, mongodb, kafka, sftp), ingest mode, dial (how to reach the source), the source objects it ingests and the Bronze table each lands in, and its schedule. Null fields mean ingest was never set up.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string", "description": "connector id, from list_connectors" } },
            "required": ["id"] } } })
}

fn set_ingest_spec_schema() -> Value {
    json!({ "type": "function", "function": { "name": "set_ingest_spec",
        "description": "Set what a connector ingests into Bronze. Call get_ingest_spec and discover_source first. adapter and dial must match the connector's type; each source object names a source table (or endpoint, file) and the Bronze table it lands in. When you send a connector's source objects again (for example to add a table), keep each object's loadMode and primaryKey exactly as get_ingest_spec returned them: an object sent without a loadMode goes back to replace. Example for a Postgres table: adapter \"sql\", ingestMode \"batch\", dial {\"driver\":\"postgres\",\"host\":\"db\",\"port\":5432,\"database\":\"shop\",\"user\":\"reader\"}, sourceObjects [{\"name\":\"public.orders\",\"target\":\"orders\"}], scheduleCron \"0 * * * *\". The server validates the dial and refuses internal addresses. This tool never carries a credential, so it cannot change where a connector points (adapter, host, port, database, endpoint, bucket, base URL or brokers): such a change is refused with a conflict and is made in the console, where the credentials are entered again.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string", "description": "connector id" },
            "adapter": { "type": "string", "enum": ["sql", "cdc", "files", "rest", "sheets", "mongodb", "kafka", "sftp"] },
            "ingestMode": { "type": "string", "enum": ["batch", "cdc", "stream"] },
            "dial": { "type": "object", "description": "how to reach the source; fields depend on the adapter" },
            "sourceObjects": { "type": "array", "items": { "type": "object", "properties": {
                "name": { "type": "string", "description": "source table (schema.table), endpoint or file" },
                "incrementalKey": { "type": "string", "description": "optional column for incremental reads" },
                "loadMode": { "type": "string",
                    "enum": ["replace", "append", "incremental", "replace_dedup", "incremental_dedup", "history"],
                    "description": "how each run writes the table; omitted means replace" },
                "primaryKey": { "type": "array", "items": { "type": "string" },
                    "description": "columns that identify one row; needed by replace_dedup, incremental_dedup and history" },
                "target": { "type": "string", "description": "Bronze table name it lands in" } },
                "required": ["name", "target"] } },
            "scheduleCron": { "type": "string", "description": "cron schedule for batch ingest, optional" } },
            "required": ["id", "adapter", "ingestMode", "dial", "sourceObjects"] } } })
}

fn discover_source_schema() -> Value {
    json!({ "type": "function", "function": { "name": "discover_source",
        "description": "Connect to a connector's source and list the tables (and their columns) that could be ingested. Read-only on the source. Returns supported:false for source types this build cannot list.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string", "description": "connector id" },
            "schema": { "type": "string", "description": "optional schema to list, e.g. public" } },
            "required": ["id"] } } })
}

fn run_ingest_schema() -> Value {
    json!({ "type": "function", "function": { "name": "run_ingest",
        "description": "Run one connector's ingest now: reads its source objects into Bronze (the same as Run now in the console). The connector needs an ingest spec first. A CDC connector returns supported:false: Debezium streams it continuously.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string", "description": "connector id" } },
            "required": ["id"] } } })
}

fn list_ingest_runs_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_ingest_runs",
        "description": "Recent ingest runs of one connector: when each ran, which object, rows written, and success or the reason it failed.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string", "description": "connector id" } },
            "required": ["id"] } } })
}

fn rotate_connector_credential_schema() -> Value {
    json!({ "type": "function", "function": { "name": "rotate_connector_credential",
        "description": "Point a connector's credential slot at a new credential. The server derives the credential name from the connector id; you choose only the slot, where it is read from (env or file) and its kind. The new credential is tested before it is saved. Needs human approval before it runs.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string", "description": "connector id" },
            "slot": { "type": "string", "enum": ["primary", "secondary"] },
            "source": { "type": "string", "enum": ["env", "file"] },
            "kind": { "type": "string", "enum": ["password", "secret_key", "access_key", "api_key", "token", "private_key"] } },
            "required": ["id", "slot", "source", "kind"] } } })
}

fn create_pipeline_schema() -> Value {
    json!({ "type": "function", "function": { "name": "create_pipeline",
        "description": "Author a new pipeline that reads a source table, applies transforms and writes a target table. It is saved as a draft; mark_pipeline_ready makes it runnable. Source zones: bronze, silver, gold. Target zones: silver, gold -- a pipeline never writes bronze, which is written by ingestion. Transforms use a fixed vocabulary, one per entry: dedupe(col), filter(col = 'value') with = != < > <= >=, rename(old,new), cast(col,Int64) with String Int32 Int64 Float64 Boolean Date DateTime UUID, select(col1,col2).",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string" },
            "description": { "type": "string" },
            "sourceZone": { "type": "string", "enum": ["bronze", "silver", "gold"] },
            "sourceTable": { "type": "string" },
            "targetZone": { "type": "string", "enum": ["silver", "gold"] },
            "targetTable": { "type": "string" },
            "transforms": { "type": "array", "items": { "type": "string" } },
            "incrementalColumn": { "type": "string", "description": "optional column for incremental loads" },
            "schedule": { "type": "string", "description": "\"manual\" or a cron expression" } },
            "required": ["name", "sourceZone", "sourceTable", "targetZone", "targetTable"] } } })
}

fn get_pipeline_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_pipeline",
        "description": "One pipeline's detail: its definition (source, transforms, target), op graph, schedule and recent runs.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string", "description": "pipeline id from list_pipelines (pl-… for an authored pipeline, or a Dagster job name)" } },
            "required": ["id"] } } })
}

fn mark_pipeline_ready_schema() -> Value {
    json!({ "type": "function", "function": { "name": "mark_pipeline_ready",
        "description": "Move an authored pipeline (pl-…) from draft to ready, so the orchestrator builds a job for it and trigger_pipeline can run it.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string", "description": "authored pipeline id (pl-…)" } },
            "required": ["id"] } } })
}

fn list_iceberg_tables_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_iceberg_tables",
        "description": "The Iceberg tables in the lakehouse warehouse (Lakekeeper): with namespace, that namespace's tables; without, every namespace with its tables, including Bronze and exported Gold. Each table has its format version, current snapshot, last update, files, records and size.",
        "parameters": { "type": "object", "properties": {
            "namespace": { "type": "string", "description": "optional, e.g. bronze or gold" } } } } })
}

fn describe_iceberg_table_schema() -> Value {
    json!({ "type": "function", "function": { "name": "describe_iceberg_table",
        "description": "One Iceberg table: schema (columns and types), partition spec, and recent snapshots with their operation, time and record counts.",
        "parameters": { "type": "object", "properties": {
            "namespace": { "type": "string" },
            "table": { "type": "string" } },
            "required": ["namespace", "table"] } } })
}

fn get_table_maintenance_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_table_maintenance",
        "description": "One Iceberg table's maintenance policy (snapshots to keep, orphan-file age, small-file compaction, schedule) and its last maintenance status.",
        "parameters": { "type": "object", "properties": {
            "namespace": { "type": "string" },
            "table": { "type": "string" } },
            "required": ["namespace", "table"] } } })
}

fn set_table_maintenance_schema() -> Value {
    json!({ "type": "function", "function": { "name": "set_table_maintenance",
        "description": "Set one Iceberg table's maintenance policy, read by the nightly maintenance job. Send the whole policy.",
        "parameters": { "type": "object", "properties": {
            "namespace": { "type": "string" },
            "table": { "type": "string" },
            "snapshotsToKeep": { "type": "integer", "description": "newest snapshots to keep, optional" },
            "orphanAgeHours": { "type": "integer", "description": "remove orphan files older than this, optional" },
            "compactSmallFiles": { "type": "boolean" },
            "schedule": { "type": "string", "description": "cron schedule, optional" } },
            "required": ["namespace", "table", "compactSmallFiles"] } } })
}

fn get_capacity_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_capacity",
        "description": "Storage capacity: each object-storage bucket's size and growth from the daily capacity snapshot, and ClickHouse's bytes on disk.",
        "parameters": { "type": "object", "properties": {} } } })
}

/// The AI Copilot's full tool table, in the exact order the LLM sees them
/// in — [`tool_schemas`] preserves this order verbatim, and it is
/// load-bearing for the committed snapshot in
/// `tests/fixtures/tool_schemas.json`.
///
/// Risk assignment for this task (T0.1 of the copilot-operations-handover
/// plan) is deliberately coarse: every tool that was in the old
/// `WRITE_TOOLS` array is non-`Read` (`WriteLow`, except `delete_chart`
/// which is `WriteHigh`); everything else is `Read`. Refining
/// `WriteLow` vs `WriteHigh` for tools added in later tasks, and actually
/// branching gate behaviour on the distinction, happens in T0.4/T0.5.
pub static TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "run_sql",
        schema: run_sql_schema,
        risk: Risk::Read,
        permission: "query:read",
    },
    ToolSpec {
        name: "list_datasets",
        schema: list_datasets_schema,
        risk: Risk::Read,
        permission: "catalog:read",
    },
    ToolSpec {
        name: "lakehouse_overview",
        schema: lakehouse_overview_schema,
        risk: Risk::Read,
        permission: "catalog:read",
    },
    ToolSpec {
        name: "describe_dataset",
        schema: describe_dataset_schema,
        risk: Risk::Read,
        permission: "catalog:read",
    },
    ToolSpec {
        name: "get_lineage",
        schema: get_lineage_schema,
        risk: Risk::Read,
        permission: "lineage:read",
    },
    ToolSpec {
        name: "get_quality",
        schema: get_quality_schema,
        risk: Risk::Read,
        // No route with a narrower permission than `RequiresAuth` covers
        // quality directly (`GET /api/governance/{kind}` is
        // `RequiresAuth`) — empty means "authenticated only", see the
        // `permission` field doc comment.
        permission: "",
    },
    ToolSpec {
        name: "trigger_lakehouse_build",
        schema: trigger_lakehouse_build_schema,
        risk: Risk::WriteLow,
        permission: "pipeline:write",
    },
    ToolSpec {
        name: "get_build_status",
        schema: get_build_status_schema,
        risk: Risk::Read,
        permission: "pipeline:read",
    },
    ToolSpec {
        name: "describe_mart",
        schema: describe_mart_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "create_chart",
        schema: create_chart_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "update_chart",
        schema: update_chart_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "format_chart",
        schema: format_chart_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "set_dashboard_theme",
        schema: set_dashboard_theme_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "create_board",
        schema: create_board_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "list_boards",
        schema: list_boards_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "suggest_dashboard",
        schema: suggest_dashboard_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "list_charts",
        schema: list_charts_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    // Read-only: authoring SQL sources needs `dashboard:sql` and happens in
    // Query Studio; chat can only list them and build charts on them.
    ToolSpec {
        name: "list_sql_sources",
        schema: list_sql_sources_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    // Read-only, like `describe_mart`: column metadata of two marts.
    ToolSpec {
        name: "suggest_join",
        schema: suggest_join_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "delete_chart",
        schema: delete_chart_schema,
        risk: Risk::WriteHigh,
        permission: "dashboard:write",
    },
    // BI-18 part C / AI-9: tabs. They save through `PUT /api/dashboard/boards`
    // (`dashboard:write`); deleting a tab asks for approval like `delete_chart`.
    ToolSpec {
        name: "create_tab",
        schema: create_tab_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "rename_tab",
        schema: rename_tab_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "move_charts_to_tab",
        schema: move_charts_to_tab_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "move_tab",
        schema: move_tab_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "delete_tab",
        schema: delete_tab_schema,
        risk: Risk::WriteHigh,
        permission: "dashboard:write",
    },
    // BI-8 / AI-4: calculated fields. The permissions mirror
    // `/api/dashboard/calc-fields` in `POLICY_TABLE`: reading, checking and
    // the function list need `dashboard:read`; saving and changing need
    // `dashboard:write`; deleting is high-risk like `delete_chart`.
    ToolSpec {
        name: "list_formula_functions",
        schema: list_formula_functions_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "list_calculated_fields",
        schema: list_calculated_fields_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "validate_formula",
        schema: validate_formula_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "create_calculated_field",
        schema: create_calculated_field_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "update_calculated_field",
        schema: update_calculated_field_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "delete_calculated_field",
        schema: delete_calculated_field_schema,
        risk: Risk::WriteHigh,
        permission: "dashboard:write",
    },
    // BI-2 / AI-7: reusable metrics. Reading, describing and querying need
    // `dashboard:read` (query_metric runs as the caller); the writes mirror
    // `/api/dashboard/metrics` and need `metric:write`; deleting asks for
    // approval like `delete_calculated_field`.
    ToolSpec {
        name: "list_metrics",
        schema: list_metrics_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "describe_metric",
        schema: describe_metric_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "query_metric",
        schema: query_metric_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "create_metric",
        schema: create_metric_schema,
        risk: Risk::WriteLow,
        permission: "metric:write",
    },
    ToolSpec {
        name: "update_metric",
        schema: update_metric_schema,
        risk: Risk::WriteLow,
        permission: "metric:write",
    },
    ToolSpec {
        name: "delete_metric",
        schema: delete_metric_schema,
        risk: Risk::WriteHigh,
        permission: "metric:write",
    },
    // BI-10 / AI-6: value groups and hierarchies. The permissions mirror
    // `/api/dashboard/calc-fields` and `/api/dashboard/hierarchies` in
    // `POLICY_TABLE`: reading needs `dashboard:read`, saving `dashboard:write`;
    // deleting a hierarchy asks for approval like `delete_calculated_field`.
    ToolSpec {
        name: "create_value_group",
        schema: create_value_group_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "list_hierarchies",
        schema: list_hierarchies_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "create_hierarchy",
        schema: create_hierarchy_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "delete_hierarchy",
        schema: delete_hierarchy_schema,
        risk: Risk::WriteHigh,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "list_parameters",
        schema: list_parameters_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "create_parameter",
        schema: create_parameter_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "update_parameter",
        schema: update_parameter_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "delete_parameter",
        schema: delete_parameter_schema,
        risk: Risk::WriteHigh,
        permission: "dashboard:write",
    },
    // ── T1.1 Alerts (Tier 1 of the copilot-operations-handover plan) ────
    // Permission strings verified against `policy.rs::POLICY_TABLE`
    // (plan section 3.7 C1): `GET /api/alerts` is `RequiresAuth` (no
    // narrower permission — empty string, same convention as
    // `get_quality`), `POST`/`PUT`/`DELETE /api/alerts` and the live
    // `/api/alerts/run` evaluation all require `alert:write`.
    ToolSpec {
        name: "list_alert_rules",
        schema: list_alert_rules_schema,
        risk: Risk::Read,
        permission: "",
    },
    ToolSpec {
        name: "create_alert_rule",
        schema: create_alert_rule_schema,
        risk: Risk::WriteLow,
        permission: "alert:write",
    },
    ToolSpec {
        name: "update_alert_rule",
        schema: update_alert_rule_schema,
        risk: Risk::WriteLow,
        permission: "alert:write",
    },
    ToolSpec {
        name: "delete_alert_rule",
        schema: delete_alert_rule_schema,
        risk: Risk::WriteHigh,
        permission: "alert:write",
    },
    ToolSpec {
        name: "run_alert_rule",
        schema: run_alert_rule_schema,
        risk: Risk::WriteLow,
        permission: "alert:write",
    },
    // ── T1.2 Connectors ───────────────────────────────────────────────
    // Every `/api/connectors*` route — including `GET` and `/test` —
    // requires `connector:manage` (policy.rs:266-275); there is no
    // narrower read permission to carry here.
    ToolSpec {
        name: "list_connectors",
        schema: list_connectors_schema,
        risk: Risk::Read,
        permission: "connector:manage",
    },
    ToolSpec {
        name: "create_connector",
        schema: create_connector_schema,
        risk: Risk::WriteLow,
        permission: "connector:manage",
    },
    ToolSpec {
        name: "test_connector",
        schema: test_connector_schema,
        risk: Risk::WriteLow,
        permission: "connector:manage",
    },
    ToolSpec {
        name: "delete_connector",
        schema: delete_connector_schema,
        risk: Risk::WriteHigh,
        permission: "connector:manage",
    },
    // ── T1.3 Pipelines (additions) ────────────────────────────────────
    // `pipeline:read` for the two list tools, `pipeline:write` for every
    // mutation (policy.rs:210-218) — same split `trigger_lakehouse_build`/
    // `get_build_status` already use above.
    ToolSpec {
        name: "list_pipelines",
        schema: list_pipelines_schema,
        risk: Risk::Read,
        permission: "pipeline:read",
    },
    ToolSpec {
        name: "list_pipeline_runs",
        schema: list_pipeline_runs_schema,
        risk: Risk::Read,
        permission: "pipeline:read",
    },
    ToolSpec {
        name: "trigger_pipeline",
        schema: trigger_pipeline_schema,
        risk: Risk::WriteLow,
        permission: "pipeline:write",
    },
    ToolSpec {
        name: "retry_pipeline_run",
        schema: retry_pipeline_run_schema,
        risk: Risk::WriteLow,
        permission: "pipeline:write",
    },
    ToolSpec {
        name: "pause_pipeline",
        schema: pause_pipeline_schema,
        risk: Risk::WriteHigh,
        permission: "pipeline:write",
    },
    ToolSpec {
        name: "resume_pipeline",
        schema: resume_pipeline_schema,
        risk: Risk::WriteLow,
        permission: "pipeline:write",
    },
    ToolSpec {
        name: "cancel_pipeline_run",
        schema: cancel_pipeline_run_schema,
        risk: Risk::WriteHigh,
        permission: "pipeline:write",
    },
    // ── T1.4 Saved queries ────────────────────────────────────────────
    // `POST /api/query/run`, `GET /api/query/saved` and `/history` all
    // require the SAME `query:read` (policy.rs:202-205) — there is no
    // separate write permission for saved queries in `POLICY_TABLE`.
    ToolSpec {
        name: "save_query",
        schema: save_query_schema,
        risk: Risk::WriteLow,
        permission: "query:read",
    },
    ToolSpec {
        name: "list_saved_queries",
        schema: list_saved_queries_schema,
        risk: Risk::Read,
        permission: "query:read",
    },
    ToolSpec {
        name: "run_saved_query",
        schema: run_saved_query_schema,
        risk: Risk::Read,
        permission: "query:read",
    },
    // ── T2.1 Governance reads ─────────────────────────────────────────
    // `GET /api/governance/{kind}` is `RequiresAuth` for every kind
    // (policy.rs:164, C1) — no narrower permission to carry.
    ToolSpec {
        name: "get_audit_history",
        schema: get_audit_history_schema,
        risk: Risk::Read,
        permission: "",
    },
    ToolSpec {
        name: "list_classification_rules",
        schema: list_classification_rules_schema,
        risk: Risk::Read,
        permission: "",
    },
    ToolSpec {
        name: "list_quality_rules",
        schema: list_quality_rules_schema,
        risk: Risk::Read,
        permission: "",
    },
    ToolSpec {
        name: "get_cdc_health",
        schema: get_cdc_health_schema,
        risk: Risk::Read,
        permission: "",
    },
    ToolSpec {
        name: "get_maintenance_metrics",
        schema: get_maintenance_metrics_schema,
        risk: Risk::Read,
        permission: "",
    },
    // ── T2.2 Maintenance (C2: exactly one tool, see its schema doc) ────
    ToolSpec {
        name: "run_bronze_maintenance",
        schema: run_bronze_maintenance_schema,
        risk: Risk::WriteHigh,
        permission: "",
    },
    // ── T2.3 Workloads ──────────────────────────────────────────────
    // `GET /api/ops/workloads` is `RequiresAuth`; cancelling one requires
    // `workload:cancel` (policy.rs:156, C1).
    ToolSpec {
        name: "list_workloads",
        schema: list_workloads_schema,
        risk: Risk::Read,
        permission: "",
    },
    ToolSpec {
        name: "kill_query",
        schema: kill_query_schema,
        risk: Risk::WriteHigh,
        permission: "workload:cancel",
    },
    // ── T2.4 Gold export ────────────────────────────────────────────
    // `GET`/`POST /api/gold/export/{mart}` are both `RequiresAuth`
    // (policy.rs:197-198, C1) — the route's OWN `x-run-token`/service-
    // identity guard is a separate, stricter door for the Dagster
    // scheduler (ADR 0011), not something the copilot goes through; see
    // `tools::gold`'s module doc comment for why bypassing it here is the
    // same precedent as `run_alert_rule` bypassing `check_run_token`.
    ToolSpec {
        name: "export_gold_mart",
        schema: export_gold_mart_schema,
        risk: Risk::WriteLow,
        permission: "",
    },
    ToolSpec {
        name: "get_gold_export",
        schema: get_gold_export_schema,
        risk: Risk::Read,
        permission: "",
    },
    // ── T2.5 Governance draft tools ─────────────────────────────────
    // `POST /api/governance/policies` requires `policy:write`
    // (policy.rs:163). SEC-23 (F3): `POST /api/governance/{kind}` (quality,
    // classification) now requires `governance:write` too, so the two rule
    // tools carry it — [`super::gate::decide`] refuses them without it. Before,
    // the tools were an unguarded path around the route policy.
    ToolSpec {
        name: "draft_policy",
        schema: draft_policy_schema,
        risk: Risk::WriteLow,
        permission: "policy:write",
    },
    ToolSpec {
        name: "draft_classification_rule",
        schema: draft_classification_rule_schema,
        risk: Risk::WriteLow,
        permission: "governance:write",
    },
    ToolSpec {
        name: "draft_quality_rule",
        schema: draft_quality_rule_schema,
        risk: Risk::WriteLow,
        permission: "governance:write",
    },
    ToolSpec {
        name: "get_ingest_spec",
        schema: get_ingest_spec_schema,
        risk: Risk::Read,
        permission: "ingest:read",
    },
    ToolSpec {
        name: "set_ingest_spec",
        schema: set_ingest_spec_schema,
        risk: Risk::WriteLow,
        permission: "connector:manage",
    },
    ToolSpec {
        name: "discover_source",
        schema: discover_source_schema,
        risk: Risk::Read,
        permission: "connector:manage",
    },
    ToolSpec {
        name: "run_ingest",
        schema: run_ingest_schema,
        risk: Risk::WriteLow,
        permission: "connector:manage",
    },
    ToolSpec {
        name: "list_ingest_runs",
        schema: list_ingest_runs_schema,
        risk: Risk::Read,
        permission: "connector:manage",
    },
    ToolSpec {
        name: "rotate_connector_credential",
        schema: rotate_connector_credential_schema,
        risk: Risk::WriteHigh,
        permission: "connector:manage",
    },
    ToolSpec {
        name: "create_pipeline",
        schema: create_pipeline_schema,
        risk: Risk::WriteLow,
        permission: "pipeline:write",
    },
    ToolSpec {
        name: "get_pipeline",
        schema: get_pipeline_schema,
        risk: Risk::Read,
        permission: "pipeline:read",
    },
    ToolSpec {
        name: "mark_pipeline_ready",
        schema: mark_pipeline_ready_schema,
        risk: Risk::WriteLow,
        permission: "pipeline:write",
    },
    ToolSpec {
        name: "list_iceberg_tables",
        schema: list_iceberg_tables_schema,
        risk: Risk::Read,
        permission: "catalog:read",
    },
    ToolSpec {
        name: "describe_iceberg_table",
        schema: describe_iceberg_table_schema,
        risk: Risk::Read,
        permission: "catalog:read",
    },
    ToolSpec {
        name: "get_table_maintenance",
        schema: get_table_maintenance_schema,
        risk: Risk::Read,
        permission: "catalog:read",
    },
    ToolSpec {
        name: "set_table_maintenance",
        schema: set_table_maintenance_schema,
        risk: Risk::WriteLow,
        permission: "governance:write",
    },
    ToolSpec {
        name: "get_capacity",
        schema: get_capacity_schema,
        risk: Risk::Read,
        permission: "catalog:read",
    },
    // Asks the person a question and stores nothing, so it needs no
    // permission beyond being signed in. The console's click is what saves
    // the answer.
    ToolSpec {
        name: ASK_USER,
        schema: ask_user_schema,
        risk: Risk::Read,
        permission: "",
    },
    // Checks a proposed statement as the caller, with the caller's own
    // permission, and runs nothing. The Query Studio's own permission is
    // `query:read`, so the same person may propose what they may run.
    ToolSpec {
        name: PROPOSE_SQL,
        schema: propose_sql_schema,
        risk: Risk::Read,
        permission: "query:read",
    },
];

/// The `OpenAI`-compatible `tools` schema array, matching
/// `TOOL_SCHEMAS = Object.values(TOOLS).map((t) => t.schema)` in
/// `ai-tools.ts` — now derived from [`TOOLS`] rather than declared
/// separately. Byte-identical to the pre-refactor output; see
/// `tests/fixtures/tool_schemas.json`.
#[must_use]
pub fn tool_schemas() -> Vec<Value> {
    TOOLS.iter().map(|t| (t.schema)()).collect()
}

/// Looks up a tool by name. `None` means the name is not a registered
/// tool at all (the "unknown tool" case in
/// [`super::tools::run_tool`]), as distinct from a registered tool this
/// mode/principal is not allowed to call.
#[must_use]
pub fn find(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|t| t.name == name)
}

/// The tool schemas a principal with `perms` should be OFFERED, given
/// `perms` — an optimisation only, not enforcement: a tool this filters
/// out is still refused by [`super::gate::decide`] at dispatch if it
/// somehow reaches `run_tool` anyway (a hallucinated `tool_calls` entry, or
/// `MiniMax` XML extracted from free text). Unlike [`tool_schemas`], which
/// is pinned byte-identical to `tests/fixtures/tool_schemas.json` and must
/// never change, this is a fresh accessor so that snapshot stays untouched.
///
/// `perms: None` matches [`super::gate::decide`]'s absent-principal
/// contract: treated as "authenticated, no grants" — every tool with a
/// non-empty `permission` is filtered out, every tool with an empty one
/// (`""`, "authenticated only") is kept.
#[must_use]
pub fn tool_schemas_for(perms: Option<&lakehouse_auth::PermissionSet>) -> Vec<Value> {
    TOOLS
        .iter()
        .zip(tool_schemas())
        .filter(|(t, _)| t.permission.is_empty() || perms.is_some_and(|p| p.has(t.permission)))
        .map(|(_, schema)| schema)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn tool_schemas_has_eighty_five_entries() {
        // 15 pre-T1 tools + 19 Tier 1 operations tools (5 alerts + 4
        // connectors + 7 pipelines + 3 saved queries) + 13 Tier 2 tools
        // (5 governance reads + 1 maintenance + 2 workloads + 2 gold
        // export + 3 governance drafts) + `lakehouse_overview` + 14
        // lakehouse-operation tools (6 ingest, 3 pipeline authoring, 4
        // Iceberg table, 1 capacity) + `list_sql_sources` (dashboard SQL
        // sources) + `ask_user` + 6 calculated-field tools (`BI-8`) +
        // `suggest_join` (`BI-7`) + `create_value_group`, `list_hierarchies`,
        // `create_hierarchy` and `delete_hierarchy` (`BI-10`) + `list_parameters`,
        // `create_parameter`, `update_parameter` and `delete_parameter` (`BI-11`)
        // + `list_metrics`, `describe_metric`, `query_metric`, `create_metric`,
        // `update_metric` and `delete_metric` (`BI-2`, `AI-7`) + `propose_sql`
        // (`AI-12`, plan 15; merged from main) + `create_tab`, `rename_tab`,
        // `move_charts_to_tab`, `move_tab` and `delete_tab` (`BI-18` part C, `AI-9`).
        assert_eq!(tool_schemas().len(), 93);
    }

    /// `BI-2` (with `AI-7`): reading a metric needs `dashboard:read`; defining,
    /// changing and deleting one needs `metric:write`, the permission of the
    /// routes they call, and a delete asks for approval.
    #[test]
    fn the_metric_tools_have_the_risk_and_permission_of_the_routes_they_call() {
        for (name, risk, permission) in [
            ("list_metrics", Risk::Read, "dashboard:read"),
            ("describe_metric", Risk::Read, "dashboard:read"),
            ("query_metric", Risk::Read, "dashboard:read"),
            ("create_metric", Risk::WriteLow, "metric:write"),
            ("update_metric", Risk::WriteLow, "metric:write"),
            ("delete_metric", Risk::WriteHigh, "metric:write"),
        ] {
            let spec = find(name).expect("registered tool");
            assert_eq!(spec.risk, risk, "{name}");
            assert_eq!(spec.permission, permission, "{name}");
        }
        // A measure may be a metric, and a filter a segment, on both chart tools.
        for tool in ["create_chart", "update_chart"] {
            let schema = (find(tool).expect("registered tool").schema)();
            let properties = &schema["function"]["parameters"]["properties"];
            assert_eq!(
                properties["measures"]["items"]["oneOf"][3]["required"],
                json!(["metric"]),
                "{tool}"
            );
            assert_eq!(
                properties["filters"]["items"]["oneOf"][0]["required"],
                json!(["segment"]),
                "{tool}"
            );
        }
    }

    /// `BI-18` part C (with `AI-9`): the tab tools need `dashboard:write`, the
    /// permission of the route they save through, and deleting a tab asks for
    /// approval.
    #[test]
    fn the_tab_tools_have_the_risk_and_permission_of_the_route_they_call() {
        for (name, risk) in [
            ("create_tab", Risk::WriteLow),
            ("rename_tab", Risk::WriteLow),
            ("move_charts_to_tab", Risk::WriteLow),
            ("move_tab", Risk::WriteLow),
            ("delete_tab", Risk::WriteHigh),
        ] {
            let spec = find(name).expect("registered tool");
            assert_eq!(
                (spec.risk, spec.permission),
                (risk, "dashboard:write"),
                "{name}"
            );
        }
    }

    #[test]
    fn the_bi_10_tools_have_the_risk_and_permission_of_the_routes_they_call() {
        for (name, risk, permission) in [
            ("create_value_group", Risk::WriteLow, "dashboard:write"),
            ("list_hierarchies", Risk::Read, "dashboard:read"),
            ("create_hierarchy", Risk::WriteLow, "dashboard:write"),
            ("delete_hierarchy", Risk::WriteHigh, "dashboard:write"),
        ] {
            let spec = find(name).expect("registered tool");
            assert_eq!((spec.risk, spec.permission), (risk, permission), "{name}");
        }
    }

    #[test]
    fn the_bi_11_tools_have_the_risk_and_permission_of_the_routes_they_call() {
        for (name, risk, permission) in [
            ("list_parameters", Risk::Read, "dashboard:read"),
            ("create_parameter", Risk::WriteLow, "dashboard:write"),
            ("update_parameter", Risk::WriteLow, "dashboard:write"),
            ("delete_parameter", Risk::WriteHigh, "dashboard:write"),
        ] {
            let spec = find(name).expect("registered tool");
            assert_eq!((spec.risk, spec.permission), (risk, permission), "{name}");
        }
    }

    /// `BI-11` (with `AI-6`): the chart tools take a parameter for the limit
    /// and the column slots, and reference lines.
    #[test]
    fn the_formatting_fields_are_the_same_on_the_three_chart_tools() {
        // BI-17 part A / AI-8: written once (`style_properties`), so create_chart, update_chart
        // and format_chart cannot drift, and every field is a field of `lakehouse_bi::style`.
        let style = style_properties();
        let style = style.as_object().expect("an object");
        assert_eq!(style.len(), 9);
        for tool in ["create_chart", "update_chart", "format_chart"] {
            let schema = (find(tool).expect("registered tool").schema)();
            let properties = &schema["function"]["parameters"]["properties"];
            for (field, definition) in style {
                assert_eq!(&properties[field], definition, "{tool} {field}");
            }
            assert_eq!(
                properties["referenceLines"],
                reference_lines_schema(),
                "{tool}"
            );
        }
    }

    #[test]
    fn format_chart_names_the_chart_and_nothing_else_is_required() {
        let schema = (find("format_chart").expect("registered tool").schema)();
        assert_eq!(
            schema["function"]["parameters"]["required"],
            json!(["chart"])
        );
        let spec = find("format_chart").expect("registered tool");
        assert_eq!(
            (spec.risk, spec.permission),
            (Risk::WriteLow, "dashboard:write")
        );
        let spec = find("set_dashboard_theme").expect("registered tool");
        assert_eq!(
            (spec.risk, spec.permission),
            (Risk::WriteLow, "dashboard:write")
        );
        // Its fields are exactly the ones the tool may change.
        let schema = (find("format_chart").expect("registered tool").schema)();
        let mut names: Vec<&str> = schema["function"]["parameters"]["properties"]
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .filter(|k| *k != "chart")
            .collect();
        names.sort_unstable();
        let mut allowed = crate::routes::ai::tools::FORMAT_FIELDS.to_vec();
        allowed.sort_unstable();
        assert_eq!(names, allowed);
    }

    #[test]
    fn the_chart_tools_take_parameters_and_reference_lines() {
        for tool in ["create_chart", "update_chart"] {
            let schema = (find(tool).expect("registered tool").schema)();
            let properties = &schema["function"]["parameters"]["properties"];
            assert!(properties.get("referenceLines").is_some(), "{tool}");
            for slot in ["limit", "dimension", "breakdown"] {
                assert_eq!(
                    properties[slot]["oneOf"][1]["required"],
                    json!(["param"]),
                    "{tool} {slot}"
                );
            }
            assert_eq!(
                properties["measures"]["items"]["oneOf"][2]["required"],
                json!(["column"]),
                "{tool}"
            );
        }
    }

    /// `BI-13` (with `AI-6`): the chart tools take layers, and how the chart's
    /// own series is drawn.
    #[test]
    fn the_chart_tools_take_layers_draw_and_axis() {
        for tool in ["create_chart", "update_chart"] {
            let schema = (find(tool).expect("registered tool").schema)();
            let properties = &schema["function"]["parameters"]["properties"];
            assert_eq!(properties["layers"]["maxItems"], 4, "{tool}");
            assert_eq!(
                properties["layers"]["items"]["required"],
                json!(["dimension", "measures", "draw", "axis"]),
                "{tool}"
            );
            assert_eq!(properties["draw"]["enum"], json!(["bar", "line"]), "{tool}");
            assert_eq!(
                properties["axis"]["enum"],
                json!(["left", "right"]),
                "{tool}"
            );
            // A layer takes the same filters and joins a chart takes.
            for field in ["filters", "joins"] {
                assert_eq!(
                    properties["layers"]["items"]["properties"][field]["items"],
                    properties[field]["items"],
                    "{tool} {field}"
                );
            }
        }
    }

    #[test]
    fn ask_user_is_a_read_tool_that_needs_no_permission() {
        let spec = find(ASK_USER).expect("registered tool");
        assert_eq!(ASK_USER, "ask_user");
        assert_eq!(spec.risk, Risk::Read);
        assert_eq!(spec.permission, "");
        let schema = (spec.schema)();
        let required = &schema["function"]["parameters"]["required"];
        assert_eq!(required, &json!(["term", "question", "options"]));
    }

    /// One-off fixture writer, run by hand only after an intentional schema
    /// change: `cargo test -p lakehouse-api --lib write_tool_schema_fixture
    /// -- --ignored`.
    #[test]
    #[ignore = "writes tests/fixtures/tool_schemas.json; run only for a reviewed schema change"]
    fn write_tool_schema_fixture() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/tool_schemas.json"
        );
        std::fs::write(path, serde_json::to_string_pretty(&tool_schemas()).unwrap()).unwrap();
    }

    /// Characterization snapshot (T0.1): `tool_schemas()`, now derived
    /// from [`TOOLS`], must still reproduce the exact JSON captured from
    /// the pre-refactor `ai.rs` byte-for-byte.
    #[test]
    fn tool_schemas_snapshot_is_byte_identical() {
        let expected = include_str!("../../../tests/fixtures/tool_schemas.json");
        let actual = serde_json::to_string_pretty(&tool_schemas()).unwrap();
        assert_eq!(
            actual, expected,
            "tool_schemas() output drifted from the committed snapshot \
             (rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json) — \
             regenerate the fixture ONLY for an intentional, reviewed \
             schema change, never to make this test pass during a refactor"
        );
    }

    /// `BI-6` (with `AI-6`): the chart tools take the chart builder's fields, and
    /// `measures` accepts a column with its own aggregate beside a plain name.
    #[test]
    fn create_chart_and_update_chart_take_the_chart_builder_fields() {
        for tool in ["create_chart", "update_chart"] {
            let schema = (find(tool).expect("registered tool").schema)();
            let properties = &schema["function"]["parameters"]["properties"];
            for field in [
                "filters",
                "sort",
                "other",
                "groupings",
                "steps",
                "joins",
                "bin",
                "breakdownBin",
                "grid",
            ] {
                assert!(properties.get(field).is_some(), "{tool} lacks {field}");
            }
            // BI-10: a bin is {mode, count?, width?}; a grouping may carry one.
            assert_eq!(properties["bin"]["required"], json!(["mode"]), "{tool}");
            assert!(
                properties["groupings"]["items"]["properties"]
                    .get("bin")
                    .is_some(),
                "{tool}"
            );
            let item = &properties["measures"]["items"]["oneOf"];
            assert_eq!(item[0]["type"], "string", "{tool}");
            assert_eq!(
                item[1]["required"],
                json!(["column", "aggregate"]),
                "{tool}"
            );
            assert_eq!(
                properties["sort"]["required"],
                json!(["by", "dir"]),
                "{tool}"
            );
        }
    }

    /// `BI-16` part B (`AI-5`): both chart tools take the five new kinds and their
    /// fields, and the tool count does not change (no new tool).
    #[test]
    fn the_chart_tools_take_the_progress_histogram_bullet_image_and_web_fields() {
        for tool in ["create_chart", "update_chart"] {
            let schema = (find(tool).expect("registered tool").schema)();
            let properties = &schema["function"]["parameters"]["properties"];
            let kinds = properties["kind"]["enum"].as_array().unwrap();
            for kind in ["progress", "histogram", "bullet", "image", "web"] {
                assert!(kinds.iter().any(|k| k == kind), "{tool} lacks kind {kind}");
            }
            for field in ["goal", "bands", "histogram", "url", "alt"] {
                assert!(properties.get(field).is_some(), "{tool} lacks {field}");
            }
            assert_eq!(
                properties["histogram"]["properties"]["count"]["maximum"],
                json!(100)
            );
            assert_eq!(properties["bands"]["maxItems"], json!(3));
            assert_eq!(properties["alt"]["maxLength"], json!(200));
            // The tool says where an address may come from, as the console does.
            assert!(
                properties["url"]["description"]
                    .as_str()
                    .unwrap()
                    .contains("Admin > Settings")
            );
        }
        // `format_chart` changes only the look: it takes none of them.
        let format = (find("format_chart").expect("registered tool").schema)();
        let props = &format["function"]["parameters"]["properties"];
        for field in ["goal", "bands", "histogram", "url", "alt"] {
            assert!(props.get(field).is_none(), "format_chart took {field}");
        }
    }

    /// `SEC-14`: the assistant must never carry a secret, so
    /// `set_ingest_spec` has no credential property, and the handler it
    /// calls only ever receives the five spec keys the tool forwards.
    #[test]
    fn set_ingest_spec_has_no_credential_property() {
        let schema = (find("set_ingest_spec").expect("registered tool").schema)();
        let properties = schema["function"]["parameters"]["properties"]
            .as_object()
            .expect("an object of properties");
        let mut names: Vec<&str> = properties.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "adapter",
                "dial",
                "id",
                "ingestMode",
                "scheduleCron",
                "sourceObjects"
            ]
        );
        let description = schema["function"]["description"]
            .as_str()
            .unwrap_or_default();
        assert!(description.contains("never carries a credential"));
    }

    #[test]
    fn tools_len_matches_schema_len() {
        assert_eq!(TOOLS.len(), tool_schemas().len());
    }

    /// The five tools that made up the old `WRITE_TOOLS` array must all
    /// still be non-`Read`; two representative read tools must not be.
    #[test]
    fn former_write_tools_are_non_read() {
        for name in [
            "trigger_lakehouse_build",
            "create_chart",
            "update_chart",
            "delete_chart",
            "create_board",
        ] {
            let spec = find(name).expect("registered tool");
            assert_ne!(spec.risk, Risk::Read, "{name} must be a non-Read risk");
        }
        assert_eq!(find("run_sql").expect("registered tool").risk, Risk::Read);
        assert_eq!(
            find("list_datasets").expect("registered tool").risk,
            Risk::Read
        );
    }

    /// T0.1's assigned risk split: `delete_chart` is `WriteHigh`, the
    /// other four former `WRITE_TOOLS` are `WriteLow`.
    #[test]
    fn delete_chart_is_write_high_others_are_write_low() {
        assert_eq!(
            find("delete_chart").expect("registered tool").risk,
            Risk::WriteHigh
        );
        for name in [
            "trigger_lakehouse_build",
            "create_chart",
            "update_chart",
            "create_board",
        ] {
            assert_eq!(
                find(name).expect("registered tool").risk,
                Risk::WriteLow,
                "{name} must be WriteLow"
            );
        }
    }

    #[test]
    fn find_returns_none_for_unknown_name() {
        assert!(find("not_a_real_tool").is_none());
    }

    /// `tool_schemas_for` is an optimisation over the same [`TOOLS`] table:
    /// a Platform Admin (`*:*`) is offered every tool.
    #[test]
    fn tool_schemas_for_admin_offers_every_tool() {
        let perms = lakehouse_auth::PermissionSet::parse("*:*");
        assert_eq!(tool_schemas_for(Some(&perms)).len(), TOOLS.len());
    }

    /// An Analyst (`query:read, catalog:read, lineage:read`) is offered
    /// only the tools with an empty `permission` or one of those three —
    /// no `dashboard:*` tool.
    #[test]
    fn tool_schemas_for_analyst_excludes_dashboard_tools() {
        let perms = lakehouse_auth::PermissionSet::parse("query:read, catalog:read, lineage:read");
        let offered = tool_schemas_for(Some(&perms));
        let offered_names: Vec<&str> = offered
            .iter()
            .map(|v| v["function"]["name"].as_str().expect("name"))
            .collect();
        assert!(offered_names.contains(&"run_sql"));
        assert!(offered_names.contains(&"list_datasets"));
        assert!(offered_names.contains(&"get_lineage"));
        assert!(!offered_names.contains(&"describe_mart"));
        assert!(!offered_names.contains(&"create_chart"));
        assert!(!offered_names.contains(&"list_boards"));
    }

    /// With no principal at all, only empty-`permission` tools are offered.
    #[test]
    fn tool_schemas_for_none_offers_only_empty_permission_tools() {
        let offered_names: Vec<String> = tool_schemas_for(None)
            .iter()
            .map(|v| v["function"]["name"].as_str().expect("name").to_owned())
            .collect();
        // Every tool whose `permission` is `""` ("authenticated only"), in
        // `TOOLS` order: `get_quality`/`list_alert_rules` (T1.1) plus the
        // Tier 2 tools that carry no narrower permission than
        // `RequiresAuth` (C1) — every T2.1 governance read,
        // `run_bronze_maintenance`, `list_workloads`, both gold export
        // tools, and `ask_user`, which stores and reads nothing. The three
        // draft tools are NOT in this list: `draft_policy` needs
        // `policy:write`, and since SEC-23 the two rule drafts need
        // `governance:write`.
        assert_eq!(
            offered_names,
            vec![
                "get_quality".to_owned(),
                "list_alert_rules".to_owned(),
                "get_audit_history".to_owned(),
                "list_classification_rules".to_owned(),
                "list_quality_rules".to_owned(),
                "get_cdc_health".to_owned(),
                "get_maintenance_metrics".to_owned(),
                "run_bronze_maintenance".to_owned(),
                "list_workloads".to_owned(),
                "export_gold_mart".to_owned(),
                "get_gold_export".to_owned(),
                "ask_user".to_owned(),
            ]
        );
    }

    /// T1.1-T1.4: every new operations tool has exactly the risk and
    /// permission specified in the copilot-operations-handover plan's
    /// section 3.7 C1 table (verified against `policy.rs::POLICY_TABLE`).
    #[test]
    fn tier1_tools_have_the_documented_risk_and_permission() {
        let expected: &[(&str, Risk, &str)] = &[
            ("list_alert_rules", Risk::Read, ""),
            ("create_alert_rule", Risk::WriteLow, "alert:write"),
            ("update_alert_rule", Risk::WriteLow, "alert:write"),
            ("delete_alert_rule", Risk::WriteHigh, "alert:write"),
            ("run_alert_rule", Risk::WriteLow, "alert:write"),
            ("list_connectors", Risk::Read, "connector:manage"),
            ("create_connector", Risk::WriteLow, "connector:manage"),
            ("test_connector", Risk::WriteLow, "connector:manage"),
            ("delete_connector", Risk::WriteHigh, "connector:manage"),
            ("list_pipelines", Risk::Read, "pipeline:read"),
            ("list_pipeline_runs", Risk::Read, "pipeline:read"),
            ("trigger_pipeline", Risk::WriteLow, "pipeline:write"),
            ("retry_pipeline_run", Risk::WriteLow, "pipeline:write"),
            ("pause_pipeline", Risk::WriteHigh, "pipeline:write"),
            ("resume_pipeline", Risk::WriteLow, "pipeline:write"),
            ("cancel_pipeline_run", Risk::WriteHigh, "pipeline:write"),
            ("save_query", Risk::WriteLow, "query:read"),
            ("list_saved_queries", Risk::Read, "query:read"),
            ("run_saved_query", Risk::Read, "query:read"),
        ];
        for (name, risk, permission) in expected {
            let spec = find(name).unwrap_or_else(|| panic!("{name} must be registered"));
            assert_eq!(spec.risk, *risk, "{name} risk");
            assert_eq!(spec.permission, *permission, "{name} permission");
        }
    }

    /// T2.1-T2.5: every Tier 2 tool has exactly the risk and permission
    /// specified in the copilot-operations-handover plan's section 3.7 C1
    /// table (verified against `policy.rs::POLICY_TABLE`), with
    /// `run_bronze_maintenance` and `kill_query` per C2/C1 as `WriteHigh`.
    #[test]
    fn tier2_tools_have_the_documented_risk_and_permission() {
        let expected: &[(&str, Risk, &str)] = &[
            ("get_audit_history", Risk::Read, ""),
            ("list_classification_rules", Risk::Read, ""),
            ("list_quality_rules", Risk::Read, ""),
            ("get_cdc_health", Risk::Read, ""),
            ("get_maintenance_metrics", Risk::Read, ""),
            ("run_bronze_maintenance", Risk::WriteHigh, ""),
            ("list_workloads", Risk::Read, ""),
            ("kill_query", Risk::WriteHigh, "workload:cancel"),
            ("export_gold_mart", Risk::WriteLow, ""),
            ("get_gold_export", Risk::Read, ""),
            ("draft_policy", Risk::WriteLow, "policy:write"),
            (
                "draft_classification_rule",
                Risk::WriteLow,
                "governance:write",
            ),
            ("draft_quality_rule", Risk::WriteLow, "governance:write"),
        ];
        for (name, risk, permission) in expected {
            let spec = find(name).unwrap_or_else(|| panic!("{name} must be registered"));
            assert_eq!(spec.risk, *risk, "{name} risk");
            assert_eq!(spec.permission, *permission, "{name} permission");
        }
    }
}
