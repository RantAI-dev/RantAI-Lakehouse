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
        "text"
    ])
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
        "description": "Gold marts (serving.*) that can be charted. With no argument: every mart with its row count. With `mart`: its columns split into dimensions (categories, time) and measures (numbers). Call this before create_chart.",
        "parameters": { "type": "object", "properties": {
            "mart": { "type": "string", "description": "mart name, e.g. mart_sales" } } } } })
}

fn create_chart_schema() -> Value {
    json!({ "type": "function", "function": { "name": "create_chart",
        "description": "Create a chart card on a dashboard (/dashboards) from a Gold mart OR a saved SQL source (sqlSource, for data that combines several marts). The server writes the SQL from the columns you pick; you do not write SQL. Call describe_mart (or list_sql_sources) first to use columns that exist.",
        "parameters": { "type": "object", "properties": {
            "title": { "type": "string" }, "subtitle": { "type": "string" },
            "mart": { "type": "string" }, "kind": { "type": "string", "enum": chart_kind_enum() },
            "sqlSource": { "type": "string", "description": "SQL source id (s_…) from list_sql_sources, instead of mart for a chart that combines several marts; set mart OR sqlSource, not both" },
            "text": { "type": "string" }, "caption": { "type": "string" },
            "target": { "type": "number" }, "dimension": { "type": "string" },
            "measures": { "type": "array", "items": { "type": "string" } },
            "breakdown": { "type": "string" },
            "map": { "type": "string", "description": "map kinds only (geomap, pointmap, geoheat): id of a bundled map, one of dki-jakarta (Jakarta cities), id-provinces (Indonesian provinces), id-regencies (Indonesian kabupaten/kota)" },
            "lat": { "type": "string", "description": "pointmap and geoheat only (required): the latitude column" },
            "lon": { "type": "string", "description": "pointmap and geoheat only (required): the longitude column" },
            "aggregate": { "type": "string", "enum": ["sum", "avg", "max", "min", "count"] },
            "limit": { "type": "number" }, "span": { "type": "number", "enum": [1, 2] },
            "board": { "type": "string" } },
            "required": ["title", "kind"] } } })
}

fn update_chart_schema() -> Value {
    json!({ "type": "function", "function": { "name": "update_chart",
        "description": "Change a saved chart (by id), keeping its id. Send every field, as for create_chart, with the new values. list_charts gives the ids.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string" }, "title": { "type": "string" }, "subtitle": { "type": "string" },
            "mart": { "type": "string" }, "kind": { "type": "string", "enum": chart_kind_enum() },
            "sqlSource": { "type": "string", "description": "SQL source id (s_…) from list_sql_sources, instead of mart for a chart that combines several marts; set mart OR sqlSource, not both" },
            "dimension": { "type": "string" },
            "measures": { "type": "array", "items": { "type": "string" } },
            "breakdown": { "type": "string" }, "caption": { "type": "string" },
            "map": { "type": "string", "description": "map kinds only (geomap, pointmap, geoheat): id of a bundled map, one of dki-jakarta, id-provinces, id-regencies" },
            "lat": { "type": "string", "description": "pointmap and geoheat only (required): the latitude column" },
            "lon": { "type": "string", "description": "pointmap and geoheat only (required): the longitude column" },
            "target": { "type": "number" },
            "aggregate": { "type": "string", "enum": ["sum", "avg", "max", "min", "count"] },
            "limit": { "type": "number" }, "span": { "type": "number", "enum": [1, 2] },
            "board": { "type": "string" } },
            "required": ["id", "title", "kind"] } } })
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
        "description": "Create an alert or digest rule. Alert (type=alert): mart, measure, agg, op and threshold; it watches that aggregate. Digest (type=digest): board, the dashboard to summarise. Delivered by webhook (target=URL) or email (target=address).",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string" },
            "type": { "type": "string", "enum": ["alert", "digest"] },
            "mart": { "type": "string", "description": "Gold mart name, for type=alert" },
            "measure": { "type": "string", "description": "measure column to watch, for type=alert" },
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
        "description": "Set what a connector ingests into Bronze. Call get_ingest_spec and discover_source first. adapter and dial must match the connector's type; each source object names a source table (or endpoint, file) and the Bronze table it lands in. Example for a Postgres table: adapter \"sql\", ingestMode \"batch\", dial {\"driver\":\"postgres\",\"host\":\"db\",\"port\":5432,\"database\":\"shop\",\"user\":\"reader\"}, sourceObjects [{\"name\":\"public.orders\",\"target\":\"orders\"}], scheduleCron \"0 * * * *\". The server validates the dial and refuses internal addresses.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string", "description": "connector id" },
            "adapter": { "type": "string", "enum": ["sql", "cdc", "files", "rest", "sheets", "mongodb", "kafka", "sftp"] },
            "ingestMode": { "type": "string", "enum": ["batch", "cdc", "stream"] },
            "dial": { "type": "object", "description": "how to reach the source; fields depend on the adapter" },
            "sourceObjects": { "type": "array", "items": { "type": "object", "properties": {
                "name": { "type": "string", "description": "source table (schema.table), endpoint or file" },
                "incrementalKey": { "type": "string", "description": "optional column for incremental reads" },
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
        "description": "Author a new pipeline that reads a source table, applies transforms and writes a target table. It is saved as a draft; mark_pipeline_ready makes it runnable. Zones: bronze, silver, gold. Transforms use a fixed vocabulary, one per entry: dedupe(col), filter(col = 'value') with = != < > <= >=, rename(old,new), cast(col,Int64) with String Int32 Int64 Float64 Boolean Date DateTime UUID, select(col1,col2).",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string" },
            "description": { "type": "string" },
            "sourceZone": { "type": "string", "enum": ["bronze", "silver", "gold"] },
            "sourceTable": { "type": "string" },
            "targetZone": { "type": "string", "enum": ["bronze", "silver", "gold"] },
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
    ToolSpec {
        name: "delete_chart",
        schema: delete_chart_schema,
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
    // (policy.rs:163); `POST /api/governance/{kind}` (quality,
    // classification) is `RequiresAuth` only (policy.rs:165, C1).
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
        permission: "",
    },
    ToolSpec {
        name: "draft_quality_rule",
        schema: draft_quality_rule_schema,
        risk: Risk::WriteLow,
        permission: "",
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
    fn tool_schemas_has_sixty_four_entries() {
        // 15 pre-T1 tools + 19 Tier 1 operations tools (5 alerts + 4
        // connectors + 7 pipelines + 3 saved queries) + 13 Tier 2 tools
        // (5 governance reads + 1 maintenance + 2 workloads + 2 gold
        // export + 3 governance drafts) + `lakehouse_overview` + 14
        // lakehouse-operation tools (6 ingest, 3 pipeline authoring, 4
        // Iceberg table, 1 capacity) + `list_sql_sources` (dashboard SQL
        // sources) + `ask_user`.
        assert_eq!(tool_schemas().len(), 64);
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
        // tools, the two rule-level draft tools (`draft_policy` needs
        // `policy:write`, so it is NOT in this list), and `ask_user`, which
        // stores and reads nothing.
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
                "draft_classification_rule".to_owned(),
                "draft_quality_rule".to_owned(),
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
            ("draft_classification_rule", Risk::WriteLow, ""),
            ("draft_quality_rule", Risk::WriteLow, ""),
        ];
        for (name, risk, permission) in expected {
            let spec = find(name).unwrap_or_else(|| panic!("{name} must be registered"));
            assert_eq!(spec.risk, *risk, "{name} risk");
            assert_eq!(spec.permission, *permission, "{name} permission");
        }
    }
}
