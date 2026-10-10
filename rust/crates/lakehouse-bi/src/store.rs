//! Spec & board storage — dashboards live INSIDE the lakehouse (`console.bi_chart`
//! + `console.bi_board` tables in `ClickHouse`), not in a separate file/DB.
//!
//! Ports `src/services/clients/bi-store.ts`. Chart SQL never comes raw from an
//! LLM/user — the server assembles it from validated identifiers (mart &
//! columns that actually exist in `serving.*`), so there is no injection path
//! and only Gold is ever touched. The structured definition (`def`) is stored
//! so a chart can be EDITED and re-filtered (e.g. a year filter) without
//! parsing SQL.
//!
//! # Live schema
//!
//! `console.bi_chart` / `console.bi_board` hold real data today. The
//! `CREATE TABLE IF NOT EXISTS` + `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`
//! bootstrap in [`ensure_bi_table`] is kept byte-identical in spirit to the
//! TypeScript (same columns, same defaults, same migration order), verified
//! against `DESCRIBE console.bi_chart` / `DESCRIBE console.bi_board` on the
//! live cluster before this port was written.

use std::collections::HashMap;

use indexmap::IndexMap;
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::{Ident, SqlLiteral};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::builder::{QueryBuilder, Relation, build_kpi_sql};
use crate::embed_access::{self, EmbedAccess};
use crate::specs::{Aggregate, ChartKind, ChartSource, ChartY, NumFmt};

/// Errors produced while validating input or talking to `ClickHouse` through
/// this module. Ports the `Error` throws scattered through `bi-store.ts`.
#[derive(Debug, Error)]
pub enum BiError {
    /// A user-facing validation failure (bad input, unknown mart/column,
    /// ...). Carries the same Indonesian-language message the TS throws, so
    /// error bodies stay byte-identical for parity.
    #[error("{0}")]
    Validation(String),
    /// A `ClickHouse` failure while querying or writing.
    #[error(transparent)]
    Clickhouse(#[from] ChError),
}

const IDENT_ALLOWED: fn(&str) -> bool = |s| Ident::new(s).is_ok();

/// The `ChartKind`s `specFromInput` accepts, mirroring the TS `KINDS` array.
const KINDS: &[ChartKind] = &[
    ChartKind::Bar,
    ChartKind::Hbar,
    ChartKind::Line,
    ChartKind::Area,
    ChartKind::Stacked,
    ChartKind::Combo,
    ChartKind::Pie,
    ChartKind::Rose,
    ChartKind::Funnel,
    ChartKind::Treemap,
    ChartKind::Scatter,
    ChartKind::Bubble,
    ChartKind::Heatmap,
    ChartKind::Radar,
    ChartKind::Waterfall,
    ChartKind::Geomap,
    ChartKind::Pointmap,
    ChartKind::Geoheat,
    ChartKind::Sankey,
    ChartKind::Sunburst,
    ChartKind::Boxplot,
    ChartKind::Calendar,
    ChartKind::Kpi,
    ChartKind::Gauge,
    ChartKind::Table,
    ChartKind::Text,
];

/// Kinds allowed to carry a breakdown (2nd dimension). `heatmap`, `sankey`
/// and `sunburst` REQUIRE one ([`breakdown_required`]). Mirrors the TS
/// `BREAKDOWN_KINDS` set.
fn breakdown_allowed(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Bar
            | ChartKind::Hbar
            | ChartKind::Line
            | ChartKind::Area
            | ChartKind::Heatmap
            | ChartKind::Sankey
            | ChartKind::Sunburst
    )
}

/// Kinds that only make sense with a 2nd dimension.
fn breakdown_required(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Heatmap | ChartKind::Sankey | ChartKind::Sunburst
    )
}

/// Kinds drawn from one (`lat`, `lon`) pair per row instead of a category
/// column. The category (`dimension`) is then only an optional label.
fn is_point_kind(kind: ChartKind) -> bool {
    matches!(kind, ChartKind::Pointmap | ChartKind::Geoheat)
}

/// Kinds drawn over a bundled map outline, so they may carry a `map` id.
fn is_map_kind(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Geomap | ChartKind::Pointmap | ChartKind::Geoheat
    )
}

/// Longest map id the server accepts. The console owns the catalogue of
/// maps; the server only checks the shape, so a new bundled map needs no
/// API change.
const MAP_ID_MAX_LEN: usize = 40;

/// Shape of a map id: lowercase letters, digits and `-`, 1 to
/// [`MAP_ID_MAX_LEN`] bytes. Whether such an id is a map the console can draw
/// is the console's call (it shows "map not available" for an unknown one).
fn map_id_is_well_formed(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAP_ID_MAX_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Mirrors the TS `AGGS` set.
fn aggregate_allowed(agg: &str) -> bool {
    matches!(agg, "sum" | "avg" | "max" | "min" | "count")
}

/// Owned variant of the TypeScript `ChartSpec` shape — used for specs
/// assembled at runtime (stored charts), as opposed to the `&'static`
/// compile-time specs in [`crate::specs`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartSpec {
    /// Stable identifier.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Optional subtitle.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub subtitle: Option<String>,
    /// How to render the data.
    pub kind: ChartKind,
    /// Source mart (unqualified, e.g. `mart_wisman`), empty for `text` and
    /// for a chart built on a SQL source.
    pub mart: String,
    /// Dashboard SQL source id (`s_…`) when the chart reads a SQL source
    /// instead of a mart. `None` for every chart stored before SQL sources
    /// existed, which is why it defaults.
    #[serde(rename = "sqlSource", skip_serializing_if = "Option::is_none", default)]
    pub sql_source: Option<String>,
    /// `ClickHouse` SQL that returns the chart's rows, empty for `text`.
    pub sql: String,
    /// Column name for the X axis / category.
    pub x: String,
    /// Column name(s) for the Y axis / measure(s).
    pub y: ChartY,
    /// Optional 2nd-dimension breakdown column.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub series: Option<String>,
    /// Map id for the map kinds (`geomap`, `pointmap`, `geoheat`). Absent
    /// on a `geomap` stored before maps were selectable, which the console
    /// draws as `dki-jakarta`; that is why it defaults.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub map: Option<String>,
    /// Latitude column of a `pointmap`/`geoheat` (a column of the rows).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lat: Option<String>,
    /// Longitude column of a `pointmap`/`geoheat`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lon: Option<String>,
    /// Numeric display format.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub format: Option<NumFmt>,
    /// Grid span; `2` = full width.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub span: Option<u8>,
    /// Markdown content for `kind: "text"` tiles.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub text: Option<String>,
    /// Caption/unit for `kind: "kpi"` tiles.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub caption: Option<String>,
    /// Target/max value for `kind: "gauge"` tiles.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        serialize_with = "serialize_js_number"
    )]
    pub target: Option<f64>,
}

/// Serializes `Option<f64>` the way `JSON.stringify` renders a JS `number`:
/// a whole-valued float (e.g. `3_000_000.0`) becomes the bare integer
/// `3000000`, not `3000000.0`. `serde_json`'s default `f64` `Serialize`
/// always keeps the decimal point, which silently disagreed with the
/// TS-captured corpus (`target: 3000000` in both `dashboard-specs-list` and
/// `dashboard-export`) even though the underlying value was correct.
#[allow(
    clippy::ref_option,
    reason = "serde's `serialize_with` contract requires `&Option<f64>`, not `Option<&f64>`"
)]
fn serialize_js_number<S: serde::Serializer>(
    value: &Option<f64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        None => serializer.serialize_none(),
        #[allow(clippy::cast_possible_truncation)]
        Some(n) if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 => {
            serializer.serialize_i64(*n as i64)
        }
        Some(n) => serializer.serialize_f64(*n),
    }
}

/// A stored chart spec, as read back from `console.bi_chart`.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredChartSpec {
    /// The render/query spec.
    pub spec: ChartSpec,
    /// Where this spec originated.
    pub source: ChartSource,
    /// Owning board id.
    pub board: String,
    /// Structured input this spec was built from (for edit prefill / runtime
    /// re-filtering).
    pub def: ChartInput,
    /// Whether the source mart has a `tahun` (year) column.
    pub has_year: bool,
    /// `created_by` column value (`"ui"`/`"ai"`/...).
    pub created_by: Option<String>,
    /// `created_at` column value, `ClickHouse`-formatted.
    pub created_at: Option<String>,
}

/// High-level input (from the AI tool / UI builder) — the server assembles
/// its SQL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartInput {
    /// Display title.
    pub title: String,
    /// Optional subtitle.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub subtitle: Option<String>,
    /// Mart name, without the `serving.` prefix (e.g. `mart_wisman`).
    ///
    /// `#[serde(default)]`: the AI tool schema (`ai.rs`) only requires
    /// `title`/`kind` — `text`/`kpi` charts legitimately omit `mart` — so
    /// decoding must be lenient here and let [`spec_from_input`]'s
    /// downstream validation (which mirrors `specFromInput` in
    /// `bi-store.ts`) produce the proper Indonesian error message instead
    /// of failing at deserialization with "missing field `mart`".
    #[serde(default)]
    pub mart: String,
    /// Dashboard SQL source id, the alternative to `mart` (exactly one of
    /// the two for every kind but `text`). `camelCase` on the wire like the
    /// rest of the console's JSON.
    #[serde(rename = "sqlSource", skip_serializing_if = "Option::is_none", default)]
    pub sql_source: Option<String>,
    /// How to render the data.
    pub kind: ChartKind,
    /// X-axis / category column.
    #[serde(default)]
    pub dimension: String,
    /// Value column(s); more than one only for `stacked`.
    #[serde(default)]
    pub measures: Vec<String>,
    /// Optional 2nd dimension: splits into multiple series.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub breakdown: Option<String>,
    /// Map id (`geomap`, `pointmap`, `geoheat`); shape-checked only, the
    /// console owns the catalogue.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub map: Option<String>,
    /// Latitude column (`pointmap`, `geoheat`; required for those, refused
    /// for every other kind).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lat: Option<String>,
    /// Longitude column; see [`Self::lat`].
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lon: Option<String>,
    /// Aggregate function.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub aggregate: Option<String>,
    /// Row limit.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit: Option<u32>,
    /// Sort order.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub order: Option<String>,
    /// Grid span; `2` = full width.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub span: Option<u8>,
    /// Destination board (defaults to `"default"`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub board: Option<String>,
    /// Markdown content (`kind: "text"`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub text: Option<String>,
    /// Unit/caption (`kind: "kpi"`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub caption: Option<String>,
    /// Target/max (`kind: "gauge"`).
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        serialize_with = "serialize_js_number"
    )]
    pub target: Option<f64>,
}

/// A dashboard.
///
/// `#[serde(rename_all = "camelCase")]`: the TS `Board` type
/// (`bi-store.ts:44`) uses `createdAt`/`publicToken`/`embedEnabled` — the
/// same `camelCase`/`snake_case` mismatch class as `hasYear` (see
/// [`StoredEnvelope`]). This struct isn't wired to an HTTP response yet
/// (dashboard routes land in a later task), but it WILL be the JSON body
/// once they do, so the mismatch is fixed here before it can ship.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    /// Stable identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// One-line purpose, shown wherever a dashboard is listed rather than
    /// opened. Empty when never set.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    /// Display name of whoever created this board (`""` for boards made
    /// before the column existed, or by an unauthenticated caller).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_by: Option<String>,
    /// Tile layout, by chart id.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub layout: Option<LayoutMap>,
    /// Dashboard-wide dimension filters.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub filters: Option<Vec<FilterDef>>,
    /// `ClickHouse`-formatted creation timestamp.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_at: Option<String>,
    /// When this board was last written, `ClickHouse`-formatted.
    ///
    /// Same underlying column as [`Self::created_at`], deliberately: the
    /// table is a `ReplacingMergeTree(created_at)`, so `created_at` is the
    /// VERSION column and every upsert (rename, layout save, filter
    /// change) rewrites it with `now()`. It has therefore always been a
    /// last-modified value wearing a "created" name. Exposing it under
    /// both names lets a list view label it truthfully as "Updated"
    /// without breaking the existing `createdAt` field that the share
    /// dialog and the parity corpus already read.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub updated_at: Option<String>,
    /// Public read-only share token, when enabled.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub public_token: Option<String>,
    /// Whether signed (JWT) embedding is enabled.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub embed_enabled: Option<bool>,
    /// Dashboard folder id (`crate::folders`); empty = root. `None` only on
    /// a board value built in code before it is saved.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub folder_id: Option<String>,
    /// Signed-embed access state (`SEC-12`): what has been withdrawn and
    /// which sites may frame the embed. Never serialised: the list of
    /// withdrawn token ids is not for a dashboard listing, and it is read
    /// through `GET /api/dashboard/embed-info`.
    #[serde(skip)]
    pub embed_access: EmbedAccess,
}

/// A tile's position on the 12-column grid canvas.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TileBox {
    /// Grid column.
    pub x: i32,
    /// Grid row.
    pub y: i32,
    /// Width, in grid columns.
    pub w: i32,
    /// Height, in grid rows.
    pub h: i32,
}

/// Layout, keyed by chart id.
///
/// Order-preserving (`IndexMap`, not `HashMap`): `/api/dashboard/export`
/// renders this map directly into YAML by iterating it, and the corpus
/// (`dashboard-export`) captures a specific, non-alphabetical key order
/// straight from the `layout_json` column. A `HashMap` here would make that
/// order effectively random per run and fail parity nondeterministically.
pub type LayoutMap = IndexMap<String, TileBox>;

pub use crate::filters::FilterDef;

// ── id generation ───────────────────────────────────────────────────────
// Mirrors `randomUUID().slice(0, 8)` (first 8 hex chars of a v4 UUID's
// first group, which carries no version/variant bits and is therefore
// uniformly random) / `randomUUID().replace(/-/g, "")` (32 hex chars).

pub(crate) fn random_hex(n_bytes: usize) -> String {
    use std::fmt::Write as _;

    use rand::Rng;
    let mut bytes = vec![0_u8; n_bytes];
    rand::rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(n_bytes * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn new_board_id() -> String {
    format!("b_{}", random_hex(4))
}

fn new_chart_id() -> String {
    format!("u_{}", random_hex(4))
}

fn new_public_token() -> String {
    format!("p_{}", random_hex(16))
}

// ── DDL bootstrap ───────────────────────────────────────────────────────

/// Process-wide once-only guard for [`ensure_bi_table`]'s DDL bootstrap,
/// matching the TS module-level `let ensured = false` cache in
/// `ensureBiTable` (`bi-store.ts:57`).
///
/// Measured against the live cluster, the 8-statement DDL sequence costs
/// ~168ms per call; `ensure_bi_table` is invoked at the top of every public
/// function in this module (14 call sites), so a single `GET
/// /api/dashboard`-shaped request that touches boards and charts issued it
/// repeatedly — ~335ms of pure no-op DDL round-trips before any real work,
/// and `delete_board` on a 10-chart board issued ~96 DDL statements (~2s).
/// Caching the *outcome* here (not just documenting the option) makes
/// repeat calls free, matching the TS.
static BI_TABLE_ENSURED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// Create the `console` database and `bi_chart`/`bi_board` tables if they do
/// not already exist (idempotent). Ports `ensureBiTable` in `bi-store.ts`.
///
/// Runs the DDL bootstrap at most once per process (see
/// [`BI_TABLE_ENSURED`]), matching the TS's once-per-process behavior. A
/// failed attempt is not cached — the next call retries the DDL, since a
/// transient failure (e.g. `ClickHouse` briefly unreachable) shouldn't
/// permanently wedge every subsequent call into always failing.
///
/// # Errors
///
/// Returns [`ChError`] if any DDL statement fails.
pub async fn ensure_bi_table(ch: &ChClient) -> Result<(), ChError> {
    BI_TABLE_ENSURED
        .get_or_try_init(|| ensure_bi_table_uncached(ch))
        .await
        .map(drop)
}

/// The actual DDL bootstrap, run at most once per process by
/// [`ensure_bi_table`].
async fn ensure_bi_table_uncached(ch: &ChClient) -> Result<(), ChError> {
    ch.exec("CREATE DATABASE IF NOT EXISTS console", None)
        .await?;
    ch.exec(
        "CREATE TABLE IF NOT EXISTS console.bi_chart (\n\
           id String,\n\
           title String,\n\
           spec_json String,\n\
           board String DEFAULT 'default',\n\
           created_by String DEFAULT 'ui',\n\
           created_at DateTime DEFAULT now(),\n\
           is_deleted UInt8 DEFAULT 0\n\
         ) ENGINE = ReplacingMergeTree(created_at) ORDER BY id",
        None,
    )
    .await?;
    ch.exec(
        "ALTER TABLE console.bi_chart ADD COLUMN IF NOT EXISTS board String DEFAULT 'default'",
        None,
    )
    .await?;
    ch.exec(
        "CREATE TABLE IF NOT EXISTS console.bi_board (\n\
           id String, name String, layout_json String DEFAULT '{}', filters_json String DEFAULT '[]',\n\
           created_at DateTime DEFAULT now(), is_deleted UInt8 DEFAULT 0\n\
         ) ENGINE = ReplacingMergeTree(created_at) ORDER BY id",
        None,
    )
    .await?;
    ch.exec(
        "ALTER TABLE console.bi_board ADD COLUMN IF NOT EXISTS layout_json String DEFAULT '{}'",
        None,
    )
    .await?;
    ch.exec(
        "ALTER TABLE console.bi_board ADD COLUMN IF NOT EXISTS filters_json String DEFAULT '[]'",
        None,
    )
    .await?;
    ch.exec(
        "ALTER TABLE console.bi_board ADD COLUMN IF NOT EXISTS public_token String DEFAULT ''",
        None,
    )
    .await?;
    ch.exec(
        "ALTER TABLE console.bi_board ADD COLUMN IF NOT EXISTS embed_enabled UInt8 DEFAULT 0",
        None,
    )
    .await?;
    // Both `String DEFAULT ''`, not `DateTime DEFAULT now()`: a default is
    // evaluated at READ time for rows written before the column existed, so
    // a `now()` default would hand back a different value on every SELECT.
    // A constant empty string is stable for old rows.
    ch.exec(
        "ALTER TABLE console.bi_board ADD COLUMN IF NOT EXISTS description String DEFAULT ''",
        None,
    )
    .await?;
    // Dashboard folders (`crate::folders`): boards and SQL sources carry a
    // `folder_id`, `''` meaning the root.
    ch.exec(
        "ALTER TABLE console.bi_board ADD COLUMN IF NOT EXISTS folder_id String DEFAULT ''",
        None,
    )
    .await?;
    ch.exec(
        "ALTER TABLE console.bi_board ADD COLUMN IF NOT EXISTS created_by String DEFAULT ''",
        None,
    )
    .await?;
    // SEC-12: signed-embed access state kept with the board. `revoked_before`
    // is a Unix-seconds instant ("withdraw all"), the two JSON columns hold
    // the individually withdrawn tokens and the sites allowed to frame the
    // embed (`crate::embed_access`). Constant defaults, like the columns
    // above, so a row written before they existed reads as "nothing
    // withdrawn, no site may frame".
    ch.exec(
        "ALTER TABLE console.bi_board ADD COLUMN IF NOT EXISTS embed_revoked_before UInt64 DEFAULT 0",
        None,
    )
    .await?;
    ch.exec(
        "ALTER TABLE console.bi_board ADD COLUMN IF NOT EXISTS embed_revoked_jti_json String DEFAULT '[]'",
        None,
    )
    .await?;
    ch.exec(
        "ALTER TABLE console.bi_board ADD COLUMN IF NOT EXISTS embed_origins_json String DEFAULT '[]'",
        None,
    )
    .await?;
    ch.exec(
        "CREATE TABLE IF NOT EXISTS console.bi_folder (\n\
           id String, name String, parent_id String DEFAULT '',\n\
           created_by String DEFAULT '', created_at DateTime DEFAULT now(),\n\
           is_deleted UInt8 DEFAULT 0\n\
         ) ENGINE = ReplacingMergeTree(created_at) ORDER BY id",
        None,
    )
    .await?;
    // Dashboard SQL sources (`crate::sources`). Same versioned-insert shape
    // as the two tables above: every save is an INSERT, `created_at` is the
    // version, a delete is an `is_deleted = 1` tombstone.
    ch.exec(
        "CREATE TABLE IF NOT EXISTS console.bi_source (\n\
           id String, title String, sql String, columns_json String DEFAULT '[]',\n\
           folder_id String DEFAULT '', created_by String DEFAULT '',\n\
           created_at DateTime DEFAULT now(), is_deleted UInt8 DEFAULT 0\n\
         ) ENGINE = ReplacingMergeTree(created_at) ORDER BY id",
        None,
    )
    .await?;
    Ok(())
}

// ── Boards ──────────────────────────────────────────────────────────────

/// Id of the built-in "Main" dashboard.
///
/// Its tiles are assembled from code, not stored, so it has no `bi_board`
/// row until its layout is first saved — and that row exists only to carry
/// the layout: [`list_boards`] leaves it out, so it never shows up as a
/// second, user-made dashboard.
pub const DEFAULT_BOARD_ID: &str = "default";

const BOARD_COLS: &str = "id, name, description, created_by, layout_json, filters_json, public_token, embed_enabled, folder_id, embed_revoked_before, embed_revoked_jti_json, embed_origins_json, toString(created_at) AS created_at";

fn parse_layout(s: &str) -> LayoutMap {
    if s.is_empty() {
        return LayoutMap::new();
    }
    serde_json::from_str(s).unwrap_or_default()
}

fn parse_filters(s: &str) -> Vec<FilterDef> {
    if s.is_empty() {
        return Vec::new();
    }
    serde_json::from_str(s).unwrap_or_default()
}

fn row_str<'a>(row: &'a serde_json::Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

/// A `UInt64` column: `ClickHouse` quotes 64-bit integers in JSON output by
/// default, so it arrives as a string; a number is accepted too.
fn row_u64(row: &serde_json::Map<String, Value>, key: &str) -> u64 {
    row.get(key)
        .and_then(|v| {
            v.as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| v.as_u64())
        })
        .unwrap_or(0)
}

fn row_to_board(row: &serde_json::Map<String, Value>) -> Board {
    let layout_json = row_str(row, "layout_json");
    let filters_json = row_str(row, "filters_json");
    let public_token = row_str(row, "public_token");
    let embed_enabled = row
        .get("embed_enabled")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| row.get("embed_enabled").and_then(Value::as_i64))
        .unwrap_or(0);
    let stamp = row_str(row, "created_at").to_owned();
    Board {
        id: row_str(row, "id").to_owned(),
        name: row_str(row, "name").to_owned(),
        description: Some(row_str(row, "description").to_owned()),
        created_by: Some(row_str(row, "created_by").to_owned()),
        layout: Some(parse_layout(layout_json)),
        filters: Some(parse_filters(filters_json)),
        created_at: Some(stamp.clone()),
        updated_at: Some(stamp),
        public_token: Some(public_token.to_owned()),
        embed_enabled: Some(embed_enabled == 1),
        folder_id: Some(row_str(row, "folder_id").to_owned()),
        embed_access: EmbedAccess::from_stored(
            row_u64(row, "embed_revoked_before"),
            row_str(row, "embed_revoked_jti_json"),
            row_str(row, "embed_origins_json"),
        ),
    }
}

/// List every non-deleted user board, oldest first. Ports `listBoards`.
///
/// Excludes the [`DEFAULT_BOARD_ID`] layout row; read that with
/// [`get_board`].
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn list_boards(ch: &ChClient) -> Result<Vec<Board>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {BOARD_COLS} FROM console.bi_board FINAL WHERE is_deleted = 0 AND id != {} ORDER BY created_at",
                SqlLiteral::from(DEFAULT_BOARD_ID)
            ),
            None,
        )
        .await?;
    Ok(rows.iter().map(row_to_board).collect())
}

/// Fetch a board by id. Ports `getBoard`.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn get_board(ch: &ChClient, id: &str) -> Result<Option<Board>, ChError> {
    ensure_bi_table(ch).await?;
    let sql = format!(
        "SELECT {BOARD_COLS} FROM console.bi_board FINAL WHERE is_deleted = 0 AND id={} LIMIT 1",
        SqlLiteral::from(id)
    );
    let rows = ch.rows(&sql, None).await?;
    Ok(rows.first().map(row_to_board))
}

/// Create a new board. Ports `createBoard`.
///
/// # Errors
///
/// Returns [`BiError::Validation`] when `name` is blank after trimming, or
/// [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn create_board(
    ch: &ChClient,
    name: &str,
    description: &str,
    created_by: &str,
) -> Result<Board, BiError> {
    ensure_bi_table(ch).await?;
    let clean = name.trim();
    if clean.is_empty() {
        return Err(BiError::Validation(
            "dashboard name is required.".to_owned(),
        ));
    }
    let desc = description.trim();
    let author = created_by.trim();
    let id = new_board_id();
    let sql = format!(
        "INSERT INTO console.bi_board (id, name, description, created_by) VALUES ({}, {}, {}, {})",
        SqlLiteral::from(id.as_str()),
        SqlLiteral::from(clean),
        SqlLiteral::from(desc),
        SqlLiteral::from(author)
    );
    ch.exec(&sql, None).await?;
    Ok(Board {
        id,
        name: clean.to_owned(),
        description: Some(desc.to_owned()),
        created_by: Some(author.to_owned()),
        layout: Some(LayoutMap::new()),
        filters: Some(Vec::new()),
        created_at: None,
        updated_at: None,
        public_token: None,
        embed_enabled: None,
        folder_id: None,
        embed_access: EmbedAccess::default(),
    })
}

/// INSERT (not `ALTER ... UPDATE`) — `ReplacingMergeTree`, instant & consistent.
#[allow(clippy::too_many_arguments)]
async fn upsert_board(
    ch: &ChClient,
    id: &str,
    name: &str,
    description: &str,
    created_by: &str,
    layout: &LayoutMap,
    filters: &[FilterDef],
    public_token: &str,
    embed_enabled: bool,
    folder_id: &str,
    embed_access: &EmbedAccess,
) -> Result<(), ChError> {
    let layout_json = serde_json::to_string(layout).unwrap_or_else(|_| "{}".to_owned());
    let filters_json = serde_json::to_string(filters).unwrap_or_else(|_| "[]".to_owned());
    // Every save is a full-row INSERT, so the embed access state is written
    // back on every save (carried forward from the board that was read), or
    // an unrelated edit such as a rename would reset a withdrawal (SEC-12).
    let withdrawn_json =
        serde_json::to_string(&embed_access.withdrawn).unwrap_or_else(|_| "[]".to_owned());
    let origins_json =
        serde_json::to_string(&embed_access.origins).unwrap_or_else(|_| "[]".to_owned());
    let sql = format!(
        "INSERT INTO console.bi_board (id, name, description, created_by, layout_json, filters_json, public_token, embed_enabled, folder_id, embed_revoked_before, embed_revoked_jti_json, embed_origins_json) VALUES \
         ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        SqlLiteral::from(id),
        SqlLiteral::from(name),
        SqlLiteral::from(description),
        SqlLiteral::from(created_by),
        SqlLiteral::from(layout_json),
        SqlLiteral::from(filters_json),
        SqlLiteral::from(public_token),
        i32::from(embed_enabled),
        SqlLiteral::from(folder_id),
        embed_access.revoked_before,
        SqlLiteral::from(withdrawn_json),
        SqlLiteral::from(origins_json),
    );
    ch.exec(&sql, None).await
}

async fn save_board_patch(
    ch: &ChClient,
    board: &Board,
    patch: BoardPatch<'_>,
) -> Result<(), ChError> {
    let name = patch.name.unwrap_or(board.name.as_str());
    let name = if name.is_empty() { "Dashboard" } else { name };
    // `created_by` is never patched: it records who made the board, which
    // does not change when somebody else edits it. It must still be
    // re-written on every upsert, or the ReplacingMergeTree row that wins
    // would carry an empty author.
    let description = patch
        .description
        .or(board.description.as_deref())
        .unwrap_or("");
    let created_by = patch
        .created_by
        .or(board.created_by.as_deref())
        .unwrap_or("");
    let empty_layout = LayoutMap::new();
    let layout = patch
        .layout
        .or(board.layout.as_ref())
        .unwrap_or(&empty_layout);
    let empty_filters = Vec::new();
    let filters = patch
        .filters
        .or(board.filters.as_deref())
        .unwrap_or(&empty_filters);
    let public_token = patch
        .public_token
        .or(board.public_token.as_deref())
        .unwrap_or("");
    let embed_enabled = patch
        .embed_enabled
        .unwrap_or_else(|| board.embed_enabled.unwrap_or(false));
    // Every save is a full-row INSERT, so a patch that does not touch the
    // folder must carry the current one forward or the board would fall
    // back to the root.
    let folder_id = patch.folder_id.or(board.folder_id.as_deref()).unwrap_or("");
    let embed_access = patch.embed_access.unwrap_or(&board.embed_access);
    upsert_board(
        ch,
        &board.id,
        name,
        description,
        created_by,
        layout,
        filters,
        public_token,
        embed_enabled,
        folder_id,
        embed_access,
    )
    .await
}

/// Fields that can be patched onto a [`Board`] before it is re-saved. Mirrors
/// the TS `saveBoardFrom(b, patch: Partial<Board>)` helper.
#[derive(Default)]
struct BoardPatch<'a> {
    name: Option<&'a str>,
    description: Option<&'a str>,
    created_by: Option<&'a str>,
    layout: Option<&'a LayoutMap>,
    filters: Option<&'a [FilterDef]>,
    public_token: Option<&'a str>,
    embed_enabled: Option<bool>,
    folder_id: Option<&'a str>,
    embed_access: Option<&'a EmbedAccess>,
}

/// Rename a board. No-op if the board does not exist (matches the TS `if
/// (b) await saveBoardFrom(...)`).
///
/// # Errors
///
/// Returns [`BiError::Validation`] when `name` is blank, or
/// [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn rename_board(ch: &ChClient, id: &str, name: &str) -> Result<(), BiError> {
    ensure_bi_table(ch).await?;
    let clean = name.trim();
    if clean.is_empty() {
        return Err(BiError::Validation("name is required.".to_owned()));
    }
    if let Some(board) = get_board(ch, id).await? {
        save_board_patch(
            ch,
            &board,
            BoardPatch {
                name: Some(clean),
                ..Default::default()
            },
        )
        .await?;
    }
    Ok(())
}

/// Set a board's one-line description. Blank clears it. No-op if the board
/// does not exist, matching [`rename_board`].
///
/// # Errors
///
/// Returns [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn describe_board(ch: &ChClient, id: &str, description: &str) -> Result<(), BiError> {
    ensure_bi_table(ch).await?;
    let clean = description.trim();
    if let Some(board) = get_board(ch, id).await? {
        save_board_patch(
            ch,
            &board,
            BoardPatch {
                description: Some(clean),
                ..Default::default()
            },
        )
        .await?;
    }
    Ok(())
}

/// Move a board into folder `folder_id` (`""` = root). The caller has
/// already checked that the folder exists.
///
/// # Errors
///
/// Returns [`BiError::Validation`] if the board does not exist, or
/// [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn move_board(ch: &ChClient, id: &str, folder_id: &str) -> Result<(), BiError> {
    ensure_bi_table(ch).await?;
    let board = get_board(ch, id)
        .await?
        .ok_or_else(|| BiError::Validation("dashboard not found.".to_owned()))?;
    save_board_patch(
        ch,
        &board,
        BoardPatch {
            folder_id: Some(folder_id),
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

/// Update a board's tile layout, creating a bare `Board { id, name:
/// "Dashboard" }` shell if it does not exist yet. Ports `updateBoardLayout`.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn update_board_layout(
    ch: &ChClient,
    id: &str,
    layout: &LayoutMap,
) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let board = get_board(ch, id).await?.unwrap_or_else(|| Board {
        id: id.to_owned(),
        name: "Dashboard".to_owned(),
        description: None,
        created_by: None,
        layout: None,
        filters: None,
        created_at: None,
        updated_at: None,
        public_token: None,
        embed_enabled: None,
        folder_id: None,
        embed_access: EmbedAccess::default(),
    });
    save_board_patch(
        ch,
        &board,
        BoardPatch {
            layout: Some(layout),
            ..Default::default()
        },
    )
    .await
}

/// Update a board's dashboard-wide filters, same fallback as
/// [`update_board_layout`]. Ports `updateBoardFilters`.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn update_board_filters(
    ch: &ChClient,
    id: &str,
    filters: &[FilterDef],
) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let board = get_board(ch, id).await?.unwrap_or_else(|| Board {
        id: id.to_owned(),
        name: "Dashboard".to_owned(),
        description: None,
        created_by: None,
        layout: None,
        filters: None,
        created_at: None,
        updated_at: None,
        public_token: None,
        embed_enabled: None,
        folder_id: None,
        embed_access: EmbedAccess::default(),
    });
    save_board_patch(
        ch,
        &board,
        BoardPatch {
            filters: Some(filters),
            ..Default::default()
        },
    )
    .await
}

/// Enable/disable the public read-only share link for a board. `enable =
/// true` mints a token if one doesn't already exist; `enable = false` clears
/// it (revoke). Returns the active token (`""` if revoked). Ports
/// `setBoardPublic`.
///
/// # Errors
///
/// Returns [`BiError::Validation`] if the board does not exist, or
/// [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn set_board_public(ch: &ChClient, id: &str, enable: bool) -> Result<String, BiError> {
    ensure_bi_table(ch).await?;
    let board = get_board(ch, id)
        .await?
        .ok_or_else(|| BiError::Validation("dashboard not found.".to_owned()))?;
    let token = if enable {
        let existing = board.public_token.clone().unwrap_or_default();
        if existing.is_empty() {
            new_public_token()
        } else {
            existing
        }
    } else {
        String::new()
    };
    save_board_patch(
        ch,
        &board,
        BoardPatch {
            public_token: Some(&token),
            ..Default::default()
        },
    )
    .await?;
    Ok(token)
}

/// Enable/disable signed (JWT) embedding for a board. Ports `setBoardEmbed`.
///
/// # Errors
///
/// Returns [`BiError::Validation`] if the board does not exist, or
/// [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn set_board_embed(ch: &ChClient, id: &str, enable: bool) -> Result<bool, BiError> {
    ensure_bi_table(ch).await?;
    let board = get_board(ch, id)
        .await?
        .ok_or_else(|| BiError::Validation("dashboard not found.".to_owned()))?;
    save_board_patch(
        ch,
        &board,
        BoardPatch {
            embed_enabled: Some(enable),
            ..Default::default()
        },
    )
    .await?;
    Ok(enable)
}

/// Replace the sites allowed to frame a board's embed pages (`SEC-12`).
/// Returns the normalised list that was stored.
///
/// # Errors
///
/// [`BiError::Validation`] if the board does not exist or an entry is not an
/// acceptable site (see [`embed_access::validate_origins`]);
/// [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn set_board_embed_origins(
    ch: &ChClient,
    id: &str,
    origins: &[String],
) -> Result<Vec<String>, BiError> {
    let origins = embed_access::validate_origins(origins).map_err(BiError::Validation)?;
    ensure_bi_table(ch).await?;
    let board = get_board(ch, id)
        .await?
        .ok_or_else(|| BiError::Validation("dashboard not found.".to_owned()))?;
    let access = EmbedAccess {
        origins: origins.clone(),
        ..board.embed_access.clone()
    };
    save_board_patch(
        ch,
        &board,
        BoardPatch {
            embed_access: Some(&access),
            ..Default::default()
        },
    )
    .await?;
    Ok(origins)
}

/// "Withdraw all embed tokens" for a board (`SEC-12`): every token whose
/// `iat` is at or before `now_unix_seconds` is refused from the next request.
/// Returns the stored instant.
///
/// # Errors
///
/// [`BiError::Validation`] if the board does not exist;
/// [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn revoke_all_embed_tokens(
    ch: &ChClient,
    id: &str,
    now_unix_seconds: u64,
) -> Result<u64, BiError> {
    ensure_bi_table(ch).await?;
    let board = get_board(ch, id)
        .await?
        .ok_or_else(|| BiError::Validation("dashboard not found.".to_owned()))?;
    let access = board.embed_access.clone().revoke_all(now_unix_seconds);
    let revoked_before = access.revoked_before;
    save_board_patch(
        ch,
        &board,
        BoardPatch {
            embed_access: Some(&access),
            ..Default::default()
        },
    )
    .await?;
    Ok(revoked_before)
}

/// "Withdraw one embed token" for a board (`SEC-12`): a token carrying
/// `jti` is refused for this board until it could no longer be accepted
/// anyway (`exp` plus `tolerance_secs`).
///
/// # Errors
///
/// [`BiError::Validation`] if the board does not exist or the list of
/// individually withdrawn tokens is full; [`BiError::Clickhouse`] on a
/// `ClickHouse` failure.
pub async fn withdraw_embed_token(
    ch: &ChClient,
    id: &str,
    jti: &str,
    exp: u64,
    now_unix_seconds: u64,
    tolerance_secs: u64,
) -> Result<(), BiError> {
    ensure_bi_table(ch).await?;
    let board = get_board(ch, id)
        .await?
        .ok_or_else(|| BiError::Validation("dashboard not found.".to_owned()))?;
    let access = board
        .embed_access
        .clone()
        .withdraw_token(jti, exp, now_unix_seconds, tolerance_secs)
        .map_err(BiError::Validation)?;
    save_board_patch(
        ch,
        &board,
        BoardPatch {
            embed_access: Some(&access),
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

/// Fetch a board by its public share token (read-only, no auth). `None` if
/// blank or not shared. Ports `getBoardByToken`.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn get_board_by_token(ch: &ChClient, token: &str) -> Result<Option<Board>, ChError> {
    ensure_bi_table(ch).await?;
    let t = token.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let sql = format!(
        "SELECT {BOARD_COLS} FROM console.bi_board FINAL WHERE is_deleted = 0 AND public_token={} LIMIT 1",
        SqlLiteral::from(t)
    );
    let rows = ch.rows(&sql, None).await?;
    Ok(rows.first().map(row_to_board))
}

/// Soft-delete a board (tombstone), then soft-delete every chart on it.
/// Ports `deleteBoard`.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn delete_board(ch: &ChClient, id: &str) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let sql = format!(
        "INSERT INTO console.bi_board (id, name, is_deleted) VALUES ({}, '', 1)",
        SqlLiteral::from(id)
    );
    ch.exec(&sql, None).await?;
    // Delete charts inside it via tombstone (consistent, not an async mutation).
    let charts = list_stored_charts(ch).await?;
    for c in charts.iter().filter(|c| c.board == id) {
        delete_chart(ch, &c.spec.id).await?;
    }
    Ok(())
}

/// Duplicate a board along with its charts and layout (new ids). Ports
/// `duplicateBoard`.
///
/// # Errors
///
/// Returns [`BiError::Validation`] if the source board does not exist, or
/// [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn duplicate_board(ch: &ChClient, id: &str, created_by: &str) -> Result<Board, BiError> {
    ensure_bi_table(ch).await?;
    let src = get_board(ch, id)
        .await?
        .ok_or_else(|| BiError::Validation("dashboard not found.".to_owned()))?;
    let charts = list_stored_charts(ch).await?;
    let charts: Vec<_> = charts.into_iter().filter(|c| c.board == id).collect();
    // The copy carries the original's description — it describes the same
    // thing — but is authored by whoever pressed Duplicate, not by the
    // person who made the original.
    let new_board = create_board(
        ch,
        &format!("{} (salinan)", src.name),
        src.description.as_deref().unwrap_or(""),
        created_by,
    )
    .await?;
    let mut id_map: HashMap<String, String> = HashMap::new();
    for c in &charts {
        let new_id = new_chart_id();
        id_map.insert(c.spec.id.clone(), new_id.clone());
        let mut clone = c.clone();
        clone.spec.id = new_id;
        clone.board = new_board.id.clone();
        insert_chart(ch, &clone).await?;
    }
    let mut new_layout = LayoutMap::new();
    if let Some(src_layout) = &src.layout {
        for (old_id, tile) in src_layout {
            if let Some(new_id) = id_map.get(old_id) {
                new_layout.insert(new_id.clone(), *tile);
            }
        }
    }
    update_board_layout(ch, &new_board.id, &new_layout).await?;
    // The copy lands next to the original, not at the root.
    let folder_id = src.folder_id.clone().unwrap_or_default();
    if !folder_id.is_empty() {
        move_board(ch, &new_board.id, &folder_id).await?;
    }
    Ok(Board {
        layout: Some(new_layout),
        folder_id: Some(folder_id),
        ..new_board
    })
}

// ── Charts ──────────────────────────────────────────────────────────────

/// Envelope stored in `spec_json`. Supports the new `{spec, def, hasYear}`
/// format as well as the old bare-`ChartSpec` format, mirroring the TS
/// `parsed.spec ?? (parsed as ChartSpec)` fallback.
///
/// A `#[derive(Deserialize)]` with `#[serde(flatten)]` on `spec` cannot
/// express this: `flatten` always tries to read the fields directly off the
/// top-level object, so it only ever matches the LEGACY bare-`ChartSpec`
/// shape and silently fails (or worse, partially matches) on the new
/// `{spec, def, hasYear}` envelope — which is exactly how every live row in
/// `console.bi_chart` was previously dropped. This manual impl inspects the
/// JSON shape first, exactly mirroring the TS `parsed.spec ?? (parsed as
/// ChartSpec)` fallback: prefer the nested `spec` key; otherwise treat the
/// whole object as a bare `ChartSpec`.
#[derive(Debug)]
struct StoredEnvelope {
    spec: ChartSpec,
    def: Option<ChartInput>,
    has_year: Option<bool>,
}

impl<'de> Deserialize<'de> for StoredEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        if let Some(spec_value) = value.get("spec").cloned() {
            let spec: ChartSpec =
                serde_json::from_value(spec_value).map_err(serde::de::Error::custom)?;
            let def: Option<ChartInput> = value
                .get("def")
                .cloned()
                .and_then(|d| serde_json::from_value(d).ok());
            let has_year = value.get("hasYear").and_then(Value::as_bool);
            Ok(Self {
                spec,
                def,
                has_year,
            })
        } else {
            let spec: ChartSpec =
                serde_json::from_value(value).map_err(serde::de::Error::custom)?;
            Ok(Self {
                spec,
                def: None,
                has_year: None,
            })
        }
    }
}

#[derive(Debug, Serialize)]
struct StoredPayload<'a> {
    spec: &'a ChartSpec,
    def: &'a ChartInput,
    #[serde(rename = "hasYear")]
    has_year: bool,
}

/// List stored specs (live, latest per id). Ports `listStoredCharts`.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure. A row whose `spec_json` is
/// corrupt is silently skipped, matching the TS `catch { /* skip */ }`.
pub async fn list_stored_charts(ch: &ChClient) -> Result<Vec<StoredChartSpec>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            "SELECT id, spec_json, board, created_by, toString(created_at) AS created_at\n\
               FROM console.bi_chart FINAL WHERE is_deleted = 0 ORDER BY created_at",
            None,
        )
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let spec_json = row_str(row, "spec_json");
        let parsed = match serde_json::from_str::<StoredEnvelope>(spec_json) {
            Ok(parsed) => parsed,
            Err(err) => {
                tracing::warn!(
                    id = row_str(row, "id"),
                    error = %err,
                    "skipping console.bi_chart row with unparseable spec_json"
                );
                continue;
            }
        };
        let created_by = row_str(row, "created_by").to_owned();
        let source = if created_by == "ai" {
            ChartSource::Ai
        } else {
            ChartSource::Ui
        };
        out.push(StoredChartSpec {
            spec: parsed.spec,
            source,
            board: {
                let b = row_str(row, "board");
                if b.is_empty() {
                    "default".to_owned()
                } else {
                    b.to_owned()
                }
            },
            def: parsed.def.unwrap_or_else(empty_chart_input),
            has_year: parsed.has_year.unwrap_or(false),
            created_by: Some(created_by),
            created_at: Some(row_str(row, "created_at").to_owned()),
        });
    }
    Ok(out)
}

fn empty_chart_input() -> ChartInput {
    ChartInput {
        title: String::new(),
        subtitle: None,
        mart: String::new(),
        sql_source: None,
        kind: ChartKind::Table,
        dimension: String::new(),
        measures: Vec::new(),
        breakdown: None,
        map: None,
        lat: None,
        lon: None,
        aggregate: None,
        limit: None,
        order: None,
        span: None,
        board: None,
        text: None,
        caption: None,
        target: None,
    }
}

/// Look up `mart`'s columns in `system.columns`, after first confirming the
/// mart itself exists in `serving.*` — split out of `spec_from_input` to
/// keep it under clippy's line-count limit. Ports the `system.tables` /
/// `system.columns` existence checks in `specFromInput`.
async fn validated_mart_columns(
    ch: &ChClient,
    mart: &str,
) -> Result<std::collections::HashSet<String>, BiError> {
    let exists_sql = format!(
        "SELECT toString(count()) AS n FROM system.tables WHERE database='serving' AND name={} AND name NOT LIKE '%\\_baru'",
        SqlLiteral::from(mart)
    );
    let exists_rows = ch
        .rows(&exists_sql, None)
        .await
        .map_err(BiError::Clickhouse)?;
    let n: i64 = exists_rows
        .first()
        .and_then(|r| r.get("n"))
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if n == 0 {
        return Err(BiError::Validation(format!(
            "Gold mart '{mart}' not found in serving."
        )));
    }
    let cols_sql = format!(
        "SELECT name FROM system.columns WHERE database='serving' AND table={}",
        SqlLiteral::from(mart)
    );
    let cols_rows = ch
        .rows(&cols_sql, None)
        .await
        .map_err(BiError::Clickhouse)?;
    Ok(cols_rows
        .iter()
        .filter_map(|r| r.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect())
}

/// Fields already validated by [`spec_from_input`], needed to assemble a
/// `kpi`/`gauge` [`StoredChartSpec`]. Split out purely to keep
/// `spec_from_input` under clippy's line-count limit.
struct KpiCtx<'a> {
    title: String,
    subtitle: Option<String>,
    kind: ChartKind,
    mart: String,
    sql_source: Option<String>,
    from: Relation,
    agg: String,
    measures: Vec<String>,
    span: u8,
    board: String,
    new_id: String,
    source: ChartSource,
    has_year: bool,
    created_by: &'a str,
}

/// Assemble the `kpi`/`gauge` branch of `specFromInput` (single number, no
/// dimension).
fn spec_from_kpi_input(input: &ChartInput, ctx: KpiCtx<'_>) -> Result<StoredChartSpec, BiError> {
    let KpiCtx {
        title,
        subtitle,
        kind,
        mart,
        sql_source,
        from,
        agg,
        measures,
        span,
        board,
        new_id,
        source,
        has_year,
        created_by,
    } = ctx;
    let m = measures[0].clone();
    let target = if kind == ChartKind::Gauge {
        input.target.filter(|t| *t > 0.0)
    } else {
        None
    };
    let caption = input
        .caption
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let def = ChartInput {
        title: title.clone(),
        subtitle: None,
        mart: mart.clone(),
        sql_source: sql_source.clone(),
        kind,
        dimension: String::new(),
        measures: vec![m.clone()],
        breakdown: None,
        map: None,
        lat: None,
        lon: None,
        aggregate: Some(agg.clone()),
        limit: None,
        order: None,
        span: Some(span),
        board: Some(board.clone()),
        text: None,
        caption: caption.clone(),
        target,
    };
    let measure_ident = Ident::new(m.as_str())
        .map_err(|_| BiError::Validation("invalid or missing measure column.".to_owned()))?;
    // `agg` was already checked against `aggregate_allowed` above, so this
    // conversion is exact (never hits the `Sum` fallback).
    let sql = build_kpi_sql(&from, &measure_ident, Aggregate::from_str_lossy(&agg), &[]);
    let spec = ChartSpec {
        id: new_id,
        title,
        subtitle,
        kind,
        mart,
        sql_source,
        sql,
        x: String::new(),
        y: ChartY::Single("v".to_owned()),
        series: None,
        map: None,
        lat: None,
        lon: None,
        format: Some(NumFmt::Int),
        span: Some(span),
        text: None,
        caption,
        target,
    };
    Ok(StoredChartSpec {
        spec,
        source,
        board,
        def,
        has_year,
        created_by: Some(created_by.to_owned()),
        created_at: None,
    })
}

/// Fields already validated by [`spec_from_input`], needed to assemble a
/// `text` [`StoredChartSpec`]. Split out purely to keep `spec_from_input`
/// under clippy's line-count limit.
struct TextCtx<'a> {
    title: String,
    subtitle: Option<String>,
    kind: ChartKind,
    new_id: String,
    span: u8,
    board: String,
    source: ChartSource,
    created_by: &'a str,
}

/// Assemble the `text` branch of `specFromInput` (markdown content, no
/// SQL/mart).
fn spec_from_text_input(input: &ChartInput, ctx: TextCtx<'_>) -> Result<StoredChartSpec, BiError> {
    let TextCtx {
        title,
        subtitle,
        kind,
        new_id,
        span,
        board,
        source,
        created_by,
    } = ctx;
    let text = input.text.as_deref().unwrap_or_default().trim().to_owned();
    if text.is_empty() {
        return Err(BiError::Validation("text content is required.".to_owned()));
    }
    let def = ChartInput {
        title: title.clone(),
        subtitle: None,
        mart: String::new(),
        sql_source: None,
        kind,
        dimension: String::new(),
        measures: Vec::new(),
        breakdown: None,
        map: None,
        lat: None,
        lon: None,
        aggregate: None,
        limit: None,
        order: None,
        span: Some(span),
        board: Some(board.clone()),
        text: Some(text.clone()),
        caption: None,
        target: None,
    };
    let spec = ChartSpec {
        id: new_id,
        title,
        subtitle,
        kind,
        mart: String::new(),
        sql_source: None,
        sql: String::new(),
        x: String::new(),
        y: ChartY::Single(String::new()),
        series: None,
        map: None,
        lat: None,
        lon: None,
        format: Some(NumFmt::Int),
        span: Some(span),
        text: Some(text),
        caption: None,
        target: None,
    };
    Ok(StoredChartSpec {
        spec,
        source,
        board,
        def,
        has_year: false,
        created_by: Some(created_by.to_owned()),
        created_at: None,
    })
}

/// Fields already validated by [`spec_from_input`], needed to assemble a
/// `table`/chart [`StoredChartSpec`] (needs a dimension). Split out purely
/// to keep `spec_from_input` under clippy's line-count limit.
struct ChartCtx<'a> {
    title: String,
    subtitle: Option<String>,
    kind: ChartKind,
    mart: String,
    sql_source: Option<String>,
    from: Relation,
    agg: String,
    measures: Vec<String>,
    span: u8,
    board: String,
    new_id: String,
    source: ChartSource,
    has_year: bool,
    created_by: &'a str,
    cols: std::collections::HashSet<String>,
    geo: GeoFields,
}

/// `map`/`lat`/`lon` of a chart input once their shape is checked: trimmed,
/// blank = absent, and present only on the kinds that use them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GeoFields {
    map: Option<String>,
    lat: Option<String>,
    lon: Option<String>,
}

/// A trimmed copy of `value`, `None` when it is absent or blank.
fn non_blank(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// The JSON name of `kind` (`"pointmap"`), for error messages.
fn kind_name(kind: ChartKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Check the shape of `map`/`lat`/`lon` against `kind`, before any schema
/// lookup, so a stray field is refused on every kind (`text` and `kpi`
/// included) instead of being stored and ignored. Column EXISTENCE is checked
/// later, against the relation's columns ([`validate_point_columns`]).
fn geo_fields(kind: ChartKind, input: &ChartInput) -> Result<GeoFields, BiError> {
    let geo = GeoFields {
        map: non_blank(input.map.as_deref()),
        lat: non_blank(input.lat.as_deref()),
        lon: non_blank(input.lon.as_deref()),
    };
    if let Some(map) = &geo.map {
        if !is_map_kind(kind) {
            return Err(BiError::Validation(
                "map is only for geomap/pointmap/geoheat.".to_owned(),
            ));
        }
        if !map_id_is_well_formed(map) {
            return Err(BiError::Validation(format!(
                "invalid map id: use 1-{MAP_ID_MAX_LEN} lowercase letters, digits or '-'."
            )));
        }
    }
    if is_point_kind(kind) {
        let (Some(lat), Some(lon)) = (&geo.lat, &geo.lon) else {
            return Err(BiError::Validation(format!(
                "a '{}' chart needs a latitude and a longitude column.",
                kind_name(kind)
            )));
        };
        if lat == lon {
            return Err(BiError::Validation(
                "latitude and longitude must be different columns.".to_owned(),
            ));
        }
    } else if geo.lat.is_some() || geo.lon.is_some() {
        return Err(BiError::Validation(
            "latitude/longitude columns are only for pointmap/geoheat.".to_owned(),
        ));
    }
    Ok(geo)
}

/// The column rules of a point map: `lat`/`lon` are real, valid columns,
/// there is exactly one measure, and no two roles share a column (the SQL
/// would select the same name twice). The label (`dimension`) is optional and
/// already checked for existence by [`validate_chart_shape`].
fn validate_point_columns(
    geo: &GeoFields,
    dimension: &str,
    measures: &[String],
    cols: &std::collections::HashSet<String>,
) -> Result<(), BiError> {
    let (Some(lat), Some(lon)) = (&geo.lat, &geo.lon) else {
        return Ok(());
    };
    for (what, col) in [("latitude", lat), ("longitude", lon)] {
        if !IDENT_ALLOWED(col) || !cols.contains(col) {
            return Err(BiError::Validation(format!(
                "invalid or missing {what} column '{col}'."
            )));
        }
    }
    let [measure] = measures else {
        return Err(BiError::Validation(
            "a map of points needs exactly one measure.".to_owned(),
        ));
    };
    let label_clashes = !dimension.is_empty()
        && [lat, lon, measure]
            .iter()
            .any(|col| col.as_str() == dimension);
    if measure == lat || measure == lon || label_clashes {
        return Err(BiError::Validation(
            "the label, measure, latitude and longitude must be different columns.".to_owned(),
        ));
    }
    Ok(())
}

/// Validate a `table`/chart shape (dimension existence, per-kind measure
/// counts, and the optional breakdown column) — split out of
/// `spec_from_chart_input` to keep it under clippy's line-count limit. Ports
/// the dimension/breakdown validation block in `specFromInput`.
fn validate_chart_shape(
    kind: ChartKind,
    dimension: &str,
    measures: &[String],
    breakdown: &str,
    cols: &std::collections::HashSet<String>,
) -> Result<(), BiError> {
    // A point map's dimension is only an optional tooltip label.
    let optional_label = is_point_kind(kind) && dimension.is_empty();
    if !optional_label && (!IDENT_ALLOWED(dimension) || !cols.contains(dimension)) {
        return Err(BiError::Validation(format!(
            "invalid or missing dimension column '{dimension}'."
        )));
    }
    if kind == ChartKind::Stacked && measures.len() < 2 {
        return Err(BiError::Validation(
            "a 'stacked' chart needs >=2 measures.".to_owned(),
        ));
    }
    if (kind == ChartKind::Scatter || kind == ChartKind::Combo) && measures.len() < 2 {
        let label = serde_json::to_value(kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        return Err(BiError::Validation(format!(
            "a '{label}' chart needs 2 measures (X & Y)."
        )));
    }
    if kind == ChartKind::Bubble && measures.len() < 3 {
        return Err(BiError::Validation(
            "a 'bubble' chart needs 3 measures (X, Y, size).".to_owned(),
        ));
    }
    if !breakdown.is_empty() {
        if !IDENT_ALLOWED(breakdown) || !cols.contains(breakdown) {
            return Err(BiError::Validation(format!(
                "invalid or missing breakdown column '{breakdown}'."
            )));
        }
        if breakdown == dimension {
            return Err(BiError::Validation(
                "breakdown must differ from dimension.".to_owned(),
            ));
        }
        if !breakdown_allowed(kind) {
            return Err(BiError::Validation(
                "breakdown is only for bar/hbar/line/area/heatmap/sankey/sunburst.".to_owned(),
            ));
        }
        if measures.len() > 1 {
            return Err(BiError::Validation(
                "with a breakdown, use exactly one measure.".to_owned(),
            ));
        }
    }
    if breakdown_required(kind) && breakdown.is_empty() {
        let label = serde_json::to_value(kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        return Err(BiError::Validation(format!(
            "{label} needs a breakdown (2nd dimension)."
        )));
    }
    if kind == ChartKind::Boxplot && measures.len() != 1 {
        return Err(BiError::Validation(
            "a 'boxplot' chart needs exactly one measure.".to_owned(),
        ));
    }
    Ok(())
}

/// Re-validate `mart`/`dimension`/`measures`/`breakdown` as [`Ident`]s (they
/// were already checked against `system.columns` in
/// [`validated_mart_columns`]/[`validate_chart_shape`]) and build the SQL via
/// [`QueryBuilder`]. Split out of `spec_from_chart_input` to keep it under
/// clippy's line-count limit.
fn build_chart_sql(
    kind: ChartKind,
    from: &Relation,
    dimension: &str,
    measures: &[String],
    agg: &str,
    order: &str,
    limit: u32,
    breakdown: Option<&str>,
) -> Result<String, BiError> {
    let dimension_ident = Ident::new(dimension).map_err(|_| {
        BiError::Validation(format!(
            "invalid or missing dimension column '{dimension}'."
        ))
    })?;
    let mut measure_idents = Vec::with_capacity(measures.len());
    for m in measures {
        measure_idents.push(
            Ident::new(m.as_str()).map_err(|_| {
                BiError::Validation("invalid or missing measure column.".to_owned())
            })?,
        );
    }
    let breakdown_ident = match breakdown {
        Some(b) => Some(Ident::new(b).map_err(|_| {
            BiError::Validation(format!("invalid or missing breakdown column '{b}'."))
        })?),
        None => None,
    };
    if kind == ChartKind::Boxplot {
        // One measure, checked by `validate_chart_shape`; no aggregate.
        let measure = measure_idents
            .first()
            .ok_or_else(|| BiError::Validation("invalid or missing measure column.".to_owned()))?;
        return Ok(crate::builder::build_boxplot_sql(
            from,
            &dimension_ident,
            measure,
            &[],
            limit,
        ));
    }
    // `agg` was already checked against `aggregate_allowed` by the caller
    // (`spec_from_chart_input`), so this conversion is exact.
    Ok(QueryBuilder::over(from.clone())
        .dimension(dimension_ident)
        .aggregate(Aggregate::from_str_lossy(agg))
        .order(order)
        .limit(limit)
        .breakdown(breakdown_ident)
        .measures(measure_idents)
        .build())
}

/// Re-validate the columns of a point map as [`Ident`]s and build its SQL
/// ([`crate::builder::build_points_sql`]). Split out of `build_chart_sql`,
/// which already takes as many arguments as clippy allows.
fn build_point_chart_sql(
    from: &Relation,
    geo: &GeoFields,
    dimension: &str,
    measures: &[String],
    agg: &str,
) -> Result<String, BiError> {
    let ident = |name: Option<&str>, what: &str| {
        name.and_then(|n| Ident::new(n).ok())
            .ok_or_else(|| BiError::Validation(format!("invalid or missing {what} column.")))
    };
    let lat = ident(geo.lat.as_deref(), "latitude")?;
    let lon = ident(geo.lon.as_deref(), "longitude")?;
    let measure = ident(measures.first().map(String::as_str), "measure")?;
    let label = if dimension.is_empty() {
        None
    } else {
        Some(ident(Some(dimension), "label")?)
    };
    // `agg` was already checked against `aggregate_allowed` by the caller.
    Ok(crate::builder::build_points_sql(
        from,
        (&lat, &lon),
        label.as_ref(),
        &measure,
        Aggregate::from_str_lossy(agg),
        &[],
    ))
}

/// The row limit and sort order a chart is built with. A calendar shows one
/// cell per day, so up to a year of rows; a point map has its own fixed cap
/// ([`crate::builder::point_limit`]); every other kind keeps the Top-N range
/// the builder offers (1-100).
fn limit_and_order(kind: ChartKind, input: &ChartInput, from: &Relation) -> (u32, String) {
    let limit = if kind == ChartKind::Calendar {
        input.limit.unwrap_or(366).clamp(1, 366)
    } else if is_point_kind(kind) {
        crate::builder::point_limit(from)
    } else {
        input.limit.unwrap_or(20).clamp(1, 100)
    };
    let order = input.order.clone().unwrap_or_else(|| {
        if matches!(
            kind,
            ChartKind::Line | ChartKind::Area | ChartKind::Calendar
        ) {
            "none".to_owned()
        } else {
            "desc".to_owned()
        }
    });
    (limit, order)
}

/// Assemble the `table`/chart branch of `specFromInput` (grouped, needs a
/// dimension; validates `stacked`/`scatter`/`combo`/`bubble` measure-count
/// rules and the optional breakdown column).
fn spec_from_chart_input(
    input: &ChartInput,
    ctx: ChartCtx<'_>,
) -> Result<StoredChartSpec, BiError> {
    let ChartCtx {
        title,
        subtitle,
        kind,
        mart,
        sql_source,
        from,
        agg,
        measures,
        span,
        board,
        new_id,
        source,
        has_year,
        created_by,
        cols,
        geo,
    } = ctx;

    let dimension = input.dimension.clone();
    let (limit, order) = limit_and_order(kind, input, &from);
    let breakdown = input.breakdown.clone().unwrap_or_default();
    validate_chart_shape(kind, &dimension, &measures, &breakdown, &cols)?;
    validate_point_columns(&geo, &dimension, &measures, &cols)?;

    let breakdown_opt = if breakdown.is_empty() {
        None
    } else {
        Some(breakdown)
    };
    let def = ChartInput {
        title: title.clone(),
        subtitle: subtitle.clone(),
        mart: mart.clone(),
        sql_source: sql_source.clone(),
        kind,
        dimension: dimension.clone(),
        measures: measures.clone(),
        breakdown: breakdown_opt.clone(),
        map: geo.map.clone(),
        lat: geo.lat.clone(),
        lon: geo.lon.clone(),
        aggregate: Some(agg.clone()),
        limit: Some(limit),
        order: Some(order.clone()),
        span: Some(span),
        board: Some(board.clone()),
        text: None,
        caption: None,
        target: None,
    };

    let sql = if is_point_kind(kind) {
        build_point_chart_sql(&from, &geo, &dimension, &measures, &agg)?
    } else {
        build_chart_sql(
            kind,
            &from,
            &dimension,
            &measures,
            &agg,
            &order,
            limit,
            breakdown_opt.as_deref(),
        )?
    };

    let y = if measures.len() == 1 {
        ChartY::Single(measures.into_iter().next().unwrap_or_default())
    } else {
        ChartY::Multi(measures)
    };
    let spec = ChartSpec {
        id: new_id,
        title,
        subtitle,
        kind,
        mart,
        sql_source,
        sql,
        x: dimension,
        y,
        series: breakdown_opt,
        map: geo.map,
        lat: geo.lat,
        lon: geo.lon,
        format: Some(NumFmt::Int),
        span: Some(span),
        text: None,
        caption: None,
        target: None,
    };
    Ok(StoredChartSpec {
        spec,
        source,
        board,
        def,
        has_year,
        created_by: Some(created_by.to_owned()),
        created_at: None,
    })
}

/// Fields common to every branch of `specFromInput`, derived once and
/// validated up front. Split out to keep `spec_from_input` under clippy's
/// line-count limit.
struct CommonFields {
    title: String,
    kind: ChartKind,
    new_id: String,
    board: String,
    span: u8,
    subtitle: Option<String>,
}

/// Validate/normalize the fields shared by every `specFromInput` branch:
/// `title` (required), `kind` (must be one of [`KINDS`]), `id` (generated if
/// absent), `board` (defaults to `"default"`), `span` (`1` or `2`), and
/// `subtitle` (blank becomes `None`).
fn derive_common_fields(input: &ChartInput, id: Option<String>) -> Result<CommonFields, BiError> {
    let title = input.title.trim().to_owned();
    if title.is_empty() {
        return Err(BiError::Validation("title is required.".to_owned()));
    }
    let kind = input.kind;
    if !KINDS.contains(&kind) {
        return Err(BiError::Validation(format!(
            "invalid kind: {}",
            serde_json::to_value(kind)
                .map_or_else(|_| "?".to_owned(), |v| v.as_str().unwrap_or("?").to_owned())
        )));
    }
    let new_id = id.unwrap_or_else(new_chart_id);
    let board = input
        .board
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default")
        .to_owned();
    let span = if input.span == Some(2) { 2 } else { 1 };
    let subtitle = input
        .subtitle
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    Ok(CommonFields {
        title,
        kind,
        new_id,
        board,
        span,
        subtitle,
    })
}

/// Where a chart's rows come from, resolved and validated against the real
/// schema: a `serving` mart's columns from `system.columns`, or a SQL
/// source's columns as probed when it was saved.
struct Resolved {
    mart: String,
    sql_source: Option<String>,
    from: Relation,
    cols: std::collections::HashSet<String>,
}

/// The `sqlSource` id a spec built over [`InlineSql`] carries. It marks the
/// spec as drawn from SQL, so everything keyed on "is a SQL source" (the
/// point-map row cap, the console's source badge) behaves as for a stored
/// source. No such id exists in `console.bi_source`, and only the preview
/// path builds these specs, so a chart can never be stored with it.
pub const UNSAVED_SOURCE_ID: &str = "unsaved";

/// SQL that is not (yet) a stored source, with the columns the caller probed
/// from it: what the chart builder previews a chart over before the source
/// is saved. The caller owns the guard — `check_sql_source` and the role
/// rewrite — exactly as it does before it stores a source; this crate only
/// wraps the text as a derived table.
#[derive(Debug, Clone, Copy)]
pub struct InlineSql<'a> {
    /// The statement, already accepted by the guard.
    pub sql: &'a str,
    /// Its columns, from the caller's probe.
    pub columns: &'a [crate::sources::SourceColumn],
}

/// A stored SQL source as a relation: its CURRENT text and the columns probed
/// when it was saved.
fn resolved_from_stored(source: crate::sources::SqlSource) -> Resolved {
    let cols = source.column_names();
    Resolved {
        mart: String::new(),
        sql_source: Some(source.id),
        from: Relation::Sql(source.sql),
        cols,
    }
}

/// Unsaved SQL as a relation, the same shape [`resolved_from_stored`] gives a
/// stored one. The chart names its columns only: a mart or a source id next
/// to inline SQL would say two things about where the rows come from.
fn resolved_from_inline(input: &ChartInput, inline: InlineSql<'_>) -> Result<Resolved, BiError> {
    if !input.mart.trim().is_empty()
        || input
            .sql_source
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
    {
        return Err(BiError::Validation(
            "a chart over unsaved SQL cannot also name a mart or a SQL source.".to_owned(),
        ));
    }
    Ok(Resolved {
        mart: String::new(),
        sql_source: Some(UNSAVED_SOURCE_ID.to_owned()),
        from: Relation::Sql(inline.sql.to_owned()),
        cols: inline.columns.iter().map(|c| c.name.clone()).collect(),
    })
}

/// Resolve `input.mart` / `input.sql_source` (exactly one) into a
/// [`Resolved`] relation.
async fn resolve_relation(ch: &ChClient, input: &ChartInput) -> Result<Resolved, BiError> {
    let source_id = input
        .sql_source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(id) = source_id {
        if !input.mart.trim().is_empty() {
            return Err(BiError::Validation(
                "choose either a mart or a SQL source, not both.".to_owned(),
            ));
        }
        let source = crate::sources::get_source(ch, id)
            .await?
            .ok_or_else(|| BiError::Validation(format!("SQL source '{id}' not found.")))?;
        return Ok(resolved_from_stored(source));
    }
    let mart = input
        .mart
        .strip_prefix("serving.")
        .unwrap_or(&input.mart)
        .to_owned();
    let mart_ident = Ident::new(mart.as_str())
        .map_err(|_| BiError::Validation(format!("invalid mart name: {}", input.mart)))?;
    let cols = validated_mart_columns(ch, &mart).await?;
    Ok(Resolved {
        mart,
        sql_source: None,
        from: Relation::Mart(mart_ident),
        cols,
    })
}

/// What [`spec_before_relation`] leaves to do.
enum Stage {
    /// A `text` chart: complete, it has no relation.
    Done(Box<StoredChartSpec>),
    /// Every other kind: the relation still has to be resolved.
    NeedsRelation(CommonFields, GeoFields),
}

/// The checks that need neither a mart nor a source: common fields, the
/// shape of `map`/`lat`/`lon`, and the whole `text` kind.
fn spec_before_relation(
    input: &ChartInput,
    source: ChartSource,
    created_by: &str,
    id: Option<String>,
) -> Result<Stage, BiError> {
    let common = derive_common_fields(input, id)?;
    let geo = geo_fields(common.kind, input)?;
    // ── TEXT — no SQL/mart ────────────────────────────────────────────
    if common.kind == ChartKind::Text {
        let CommonFields {
            title,
            kind,
            new_id,
            board,
            span,
            subtitle,
        } = common;
        return spec_from_text_input(
            input,
            TextCtx {
                title,
                subtitle,
                kind,
                new_id,
                span,
                board,
                source,
                created_by,
            },
        )
        .map(|spec| Stage::Done(Box::new(spec)));
    }
    Ok(Stage::NeedsRelation(common, geo))
}

/// Validate `input` against the REAL `ClickHouse` schema, then assemble a
/// [`StoredChartSpec`]. Throws a friendly error when the mart/columns are
/// invalid. `id` is optional — supplied for EDIT. Branches per kind: `text`
/// (no SQL) / `kpi`/`gauge` (single number) / table & chart (grouped). Ports
/// `specFromInput`.
///
/// # Errors
///
/// Returns [`BiError::Validation`] for any input/schema validation failure,
/// or [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn spec_from_input(
    ch: &ChClient,
    input: &ChartInput,
    source: ChartSource,
    created_by: &str,
    id: Option<String>,
) -> Result<StoredChartSpec, BiError> {
    let (common, geo) = match spec_before_relation(input, source, created_by, id)? {
        Stage::Done(spec) => return Ok(*spec),
        Stage::NeedsRelation(common, geo) => (common, geo),
    };
    // ── kpi/table/chart need a mart OR a SQL source ─────────────────
    let resolved = resolve_relation(ch, input).await?;
    spec_from_resolved(input, source, created_by, common, geo, resolved)
}

/// [`spec_from_input`] over SQL that is not a stored source (see
/// [`InlineSql`]): the same validation and the same SQL building, with the
/// relation and its columns taken from `inline` instead of read from
/// `console.bi_source`. Never touches `ClickHouse`.
///
/// # Errors
///
/// Returns [`BiError::Validation`] for any input/schema validation failure,
/// including a column the SQL does not return.
pub fn spec_from_inline_sql(
    input: &ChartInput,
    inline: InlineSql<'_>,
    source: ChartSource,
    created_by: &str,
) -> Result<StoredChartSpec, BiError> {
    let (common, geo) = match spec_before_relation(input, source, created_by, None)? {
        Stage::Done(spec) => return Ok(*spec),
        Stage::NeedsRelation(common, geo) => (common, geo),
    };
    let resolved = resolved_from_inline(input, inline)?;
    spec_from_resolved(input, source, created_by, common, geo, resolved)
}

/// The part of [`spec_from_input`] that follows relation resolution, shared
/// by the stored and the inline path so a chart is validated and its SQL
/// built by one piece of code whichever way its rows are named.
fn spec_from_resolved(
    input: &ChartInput,
    source: ChartSource,
    created_by: &str,
    common: CommonFields,
    geo: GeoFields,
    resolved: Resolved,
) -> Result<StoredChartSpec, BiError> {
    let CommonFields {
        title,
        kind,
        new_id,
        board,
        span,
        subtitle,
    } = common;
    let Resolved {
        mart,
        sql_source,
        from,
        cols,
    } = resolved;
    let has_year = cols.contains("tahun");
    let agg = input
        .aggregate
        .clone()
        .unwrap_or_else(|| "sum".to_owned())
        .to_lowercase();
    if !aggregate_allowed(&agg) {
        return Err(BiError::Validation(format!("invalid aggregate: {agg}")));
    }
    let measures = input.measures.clone();
    if measures.is_empty() {
        return Err(BiError::Validation(
            "at least one measure column is required.".to_owned(),
        ));
    }
    if measures
        .iter()
        .any(|m| !IDENT_ALLOWED(m) || !cols.contains(m))
    {
        return Err(BiError::Validation(
            "invalid or missing measure column.".to_owned(),
        ));
    }

    // ── KPI / GAUGE — single number (no dimension) ──────────────────
    if kind == ChartKind::Kpi || kind == ChartKind::Gauge {
        return spec_from_kpi_input(
            input,
            KpiCtx {
                title,
                subtitle,
                kind,
                mart,
                sql_source,
                from,
                agg,
                measures,
                span,
                board,
                new_id,
                source,
                has_year,
                created_by,
            },
        );
    }

    // ── TABLE / CHART — needs a dimension ────────────────────────────
    spec_from_chart_input(
        input,
        ChartCtx {
            title,
            subtitle,
            kind,
            mart,
            sql_source,
            from,
            agg,
            measures,
            span,
            board,
            new_id,
            source,
            has_year,
            created_by,
            cols,
            geo,
        },
    )
}

/// Save/replace a spec (smoke-tests the SQL first, so a broken spec never
/// gets stored). Ports `insertChart`.
///
/// # Errors
///
/// Returns [`ChError`] if the SQL smoke test or the `INSERT` fails.
pub async fn insert_chart(ch: &ChClient, spec: &StoredChartSpec) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    if !spec.spec.sql.is_empty() {
        // Smoke test — throws if the SQL fails to execute (skipped for text).
        ch.query(&spec.spec.sql, None).await?;
    }
    let payload = StoredPayload {
        spec: &spec.spec,
        def: &spec.def,
        has_year: spec.has_year,
    };
    let payload_json = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_owned());
    let created_by = spec
        .created_by
        .clone()
        .unwrap_or_else(|| match spec.source {
            ChartSource::Ai => "ai".to_owned(),
            ChartSource::Ui => "ui".to_owned(),
            ChartSource::Builtin => "builtin".to_owned(),
        });
    let sql = format!(
        "INSERT INTO console.bi_chart (id, title, spec_json, board, created_by) VALUES \
         ({}, {}, {}, {}, {})",
        SqlLiteral::from(spec.spec.id.as_str()),
        SqlLiteral::from(spec.spec.title.as_str()),
        SqlLiteral::from(payload_json),
        SqlLiteral::from(spec.board.as_str()),
        SqlLiteral::from(created_by),
    );
    ch.exec(&sql, None).await
}

/// Soft-delete (tombstone); `ReplacingMergeTree` picks up the latest
/// version. Ports `deleteChart`.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn delete_chart(ch: &ChClient, id: &str) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let sql = format!(
        "INSERT INTO console.bi_chart (id, title, spec_json, created_by, is_deleted) VALUES \
         ({}, '', '{{}}', 'system', 1)",
        SqlLiteral::from(id)
    );
    ch.exec(&sql, None).await
}

#[cfg(test)]
impl StoredChartSpec {
    /// Test-only constructor for a minimal [`StoredChartSpec`], used by
    /// `crate::builder`'s unit tests to avoid depending on a live
    /// `ClickHouse` connection.
    pub(crate) fn for_test(
        kind: ChartKind,
        mart: &str,
        dimension: &str,
        measures: &[&str],
        sql: String,
    ) -> Self {
        let measures: Vec<String> = measures.iter().map(|m| (*m).to_owned()).collect();
        let y = if measures.len() == 1 {
            ChartY::Single(measures[0].clone())
        } else {
            ChartY::Multi(measures.clone())
        };
        Self {
            spec: ChartSpec {
                id: "test".to_owned(),
                title: "Test".to_owned(),
                subtitle: None,
                kind,
                mart: mart.to_owned(),
                sql_source: None,
                sql,
                x: dimension.to_owned(),
                y,
                series: None,
                map: None,
                lat: None,
                lon: None,
                format: Some(NumFmt::Int),
                span: Some(1),
                text: None,
                caption: None,
                target: None,
            },
            source: ChartSource::Ui,
            board: "default".to_owned(),
            def: ChartInput {
                title: "Test".to_owned(),
                subtitle: None,
                mart: mart.to_owned(),
                sql_source: None,
                kind,
                dimension: dimension.to_owned(),
                measures,
                breakdown: None,
                map: None,
                lat: None,
                lon: None,
                aggregate: Some("sum".to_owned()),
                limit: Some(20),
                order: Some("none".to_owned()),
                span: Some(1),
                board: Some("default".to_owned()),
                text: None,
                caption: None,
                target: None,
            },
            has_year: false,
            created_by: Some("ui".to_owned()),
            created_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn client(url: &str) -> ChClient {
        ChClient::new(url.to_owned(), "default".to_owned(), String::new())
    }

    /// Regression test for H1 (`ensure_bi_table` re-issuing all 13 DDL
    /// statements on every call): the first call to any public function
    /// must run the DDL bootstrap, but every call after that — across the
    /// whole process, matching the TS's module-level `ensured` flag — must
    /// be free. `BI_TABLE_ENSURED` is a crate-wide static, so this is the
    /// only test in the crate allowed to exercise `ensure_bi_table`
    /// end-to-end (a second such test would observe an already-warm cache
    /// and could pass for the wrong reason, or race depending on test
    /// execution order).
    #[tokio::test]
    async fn ensure_bi_table_runs_ddl_at_most_once_per_process() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;

        let ch = client(&server.uri());
        ensure_bi_table(&ch).await.unwrap();
        let first_call_requests = server.received_requests().await.unwrap().len();
        // 13 = the original 8, plus `description` and `created_by`
        // `ADD COLUMN` statements added for the dashboard list page, plus
        // dashboard SQL sources (`bi_source`) and folders (`bi_folder` +
        // `bi_board.folder_id`) joining the bootstrap; the property under
        // test — every later call is free — is unchanged. 16 = those 13 plus
        // the three SEC-12 `bi_board` columns (`embed_revoked_before`,
        // `embed_revoked_jti_json`, `embed_origins_json`).
        assert_eq!(
            first_call_requests, 16,
            "first call should issue all 16 DDL statements"
        );

        ensure_bi_table(&ch).await.unwrap();
        let after_second_call = server.received_requests().await.unwrap().len();
        assert_eq!(
            after_second_call, first_call_requests,
            "second call must be a no-op (cached), matching the TS's once-per-process guard"
        );
    }

    #[test]
    fn sankey_and_sunburst_need_a_breakdown_and_a_boxplot_one_measure() {
        let cols: std::collections::HashSet<String> = ["region", "channel", "amount", "qty"]
            .iter()
            .map(|c| (*c).to_owned())
            .collect();
        let m1 = vec!["amount".to_owned()];
        let m2 = vec!["amount".to_owned(), "qty".to_owned()];
        for kind in [ChartKind::Sankey, ChartKind::Sunburst] {
            assert!(validate_chart_shape(kind, "region", &m1, "", &cols).is_err());
            assert!(validate_chart_shape(kind, "region", &m1, "channel", &cols).is_ok());
        }
        assert!(validate_chart_shape(ChartKind::Boxplot, "region", &m1, "", &cols).is_ok());
        assert!(validate_chart_shape(ChartKind::Boxplot, "region", &m2, "", &cols).is_err());
        assert!(
            validate_chart_shape(ChartKind::Calendar, "region", &m1, "channel", &cols).is_err(),
            "a calendar takes no breakdown"
        );
    }

    fn geo_input(kind: ChartKind) -> ChartInput {
        let mut input = empty_chart_input();
        input.title = "Map".to_owned();
        input.kind = kind;
        input
    }

    fn column_set(names: &[&str]) -> std::collections::HashSet<String> {
        names.iter().map(|c| (*c).to_owned()).collect()
    }

    /// A `geomap` stored before maps were selectable carries none of the new
    /// fields; it must read back unchanged (no migration) and write back
    /// without them, so an old chart's stored JSON does not grow keys.
    #[test]
    fn an_old_geomap_row_without_map_lat_lon_reads_and_writes_back_unchanged() {
        let old_row = r#"{"spec":{"id":"u_e491bf76","title":"Culinary by region (map)","kind":"geomap","mart":"mart_kuliner","sql":"SELECT wilayah, round(sum(jumlah_usaha)) AS jumlah_usaha FROM serving.mart_kuliner GROUP BY wilayah ORDER BY jumlah_usaha DESC LIMIT 20","x":"wilayah","y":"jumlah_usaha","format":"int","span":2},"def":{"title":"Culinary by region (map)","mart":"mart_kuliner","kind":"geomap","dimension":"wilayah","measures":["jumlah_usaha"],"aggregate":"sum","limit":20,"order":"desc","span":2,"board":"b_5cbfb279"},"hasYear":false}"#;
        let parsed: StoredEnvelope = serde_json::from_str(old_row).unwrap();
        assert_eq!(parsed.spec.kind, ChartKind::Geomap);
        assert_eq!(
            (&parsed.spec.map, &parsed.spec.lat, &parsed.spec.lon),
            (&None, &None, &None)
        );
        let def = parsed.def.expect("def is in the envelope");
        assert_eq!((&def.map, &def.lat, &def.lon), (&None, &None, &None));
        let spec_json = serde_json::to_value(&parsed.spec).unwrap();
        let def_json = serde_json::to_value(&def).unwrap();
        for key in ["map", "lat", "lon"] {
            assert!(spec_json.get(key).is_none(), "spec grew `{key}`");
            assert!(def_json.get(key).is_none(), "def grew `{key}`");
        }
    }

    #[test]
    fn a_point_map_row_keeps_its_map_and_coordinate_columns() {
        let row = r#"{"spec":{"id":"u_1","title":"T","kind":"pointmap","mart":"mart_x","sql":"SELECT 1","x":"place","y":"visitors","map":"id-provinces","lat":"lat","lon":"lon"},"def":{"title":"T","mart":"mart_x","kind":"pointmap","dimension":"place","measures":["visitors"],"map":"id-provinces","lat":"lat","lon":"lon"}}"#;
        let parsed: StoredEnvelope = serde_json::from_str(row).unwrap();
        assert_eq!(parsed.spec.kind, ChartKind::Pointmap);
        assert_eq!(parsed.spec.map.as_deref(), Some("id-provinces"));
        assert_eq!(parsed.spec.lat.as_deref(), Some("lat"));
        let def = parsed.def.unwrap();
        assert_eq!(def.lon.as_deref(), Some("lon"));
    }

    #[test]
    fn a_map_id_is_checked_for_shape_only() {
        for ok in ["dki-jakarta", "id-regencies", "a", "kab-2024"] {
            assert!(map_id_is_well_formed(ok), "{ok}");
        }
        for bad in [
            "",
            "Dki-Jakarta",
            "id_regencies",
            "id regencies",
            "../etc",
            "peta/ok",
            "é",
        ] {
            assert!(!map_id_is_well_formed(bad), "{bad}");
        }
        assert!(map_id_is_well_formed(&"a".repeat(MAP_ID_MAX_LEN)));
        assert!(!map_id_is_well_formed(&"a".repeat(MAP_ID_MAX_LEN + 1)));
    }

    #[test]
    fn map_lat_and_lon_are_accepted_only_on_the_kinds_that_use_them() {
        // geomap: a map id, no coordinates.
        let mut geomap = geo_input(ChartKind::Geomap);
        geomap.map = Some("id-provinces".to_owned());
        assert_eq!(
            geo_fields(ChartKind::Geomap, &geomap)
                .unwrap()
                .map
                .as_deref(),
            Some("id-provinces")
        );
        geomap.lat = Some("lat".to_owned());
        assert!(geo_fields(ChartKind::Geomap, &geomap).is_err());

        // A blank map id is the same as none (an old geomap).
        let mut blank = geo_input(ChartKind::Geomap);
        blank.map = Some("  ".to_owned());
        assert_eq!(
            geo_fields(ChartKind::Geomap, &blank).unwrap(),
            GeoFields::default()
        );

        // pointmap / geoheat: both coordinates, different columns.
        for kind in [ChartKind::Pointmap, ChartKind::Geoheat] {
            let mut input = geo_input(kind);
            assert!(geo_fields(kind, &input).is_err(), "no coordinates");
            input.lat = Some("lat".to_owned());
            assert!(geo_fields(kind, &input).is_err(), "no longitude");
            input.lon = Some("lat".to_owned());
            assert!(geo_fields(kind, &input).is_err(), "same column twice");
            input.lon = Some("lon".to_owned());
            let geo = geo_fields(kind, &input).unwrap();
            assert_eq!(
                (geo.lat.as_deref(), geo.lon.as_deref()),
                (Some("lat"), Some("lon"))
            );
            input.map = Some("Not A Map".to_owned());
            assert!(geo_fields(kind, &input).is_err(), "malformed map id");
        }

        // Every other kind refuses all three.
        for kind in [ChartKind::Bar, ChartKind::Kpi, ChartKind::Text] {
            let mut with_map = geo_input(kind);
            with_map.map = Some("id-provinces".to_owned());
            assert!(geo_fields(kind, &with_map).is_err(), "{kind:?} map");
            let mut with_lon = geo_input(kind);
            with_lon.lon = Some("lon".to_owned());
            assert!(geo_fields(kind, &with_lon).is_err(), "{kind:?} lon");
            assert!(geo_fields(kind, &geo_input(kind)).is_ok());
        }
    }

    #[test]
    fn point_columns_must_exist_be_distinct_and_have_one_measure() {
        let cols = column_set(&["lat", "lon", "visitors", "place", "bad col"]);
        let geo = GeoFields {
            map: None,
            lat: Some("lat".to_owned()),
            lon: Some("lon".to_owned()),
        };
        let one = vec!["visitors".to_owned()];
        assert!(validate_point_columns(&geo, "place", &one, &cols).is_ok());
        assert!(
            validate_point_columns(&geo, "", &one, &cols).is_ok(),
            "the label is optional"
        );

        let missing = GeoFields {
            lat: Some("latitude".to_owned()),
            ..geo.clone()
        };
        assert!(validate_point_columns(&missing, "", &one, &cols).is_err());
        let not_an_ident = GeoFields {
            lon: Some("bad col".to_owned()),
            ..geo.clone()
        };
        assert!(validate_point_columns(&not_an_ident, "", &one, &cols).is_err());
        let two = vec!["visitors".to_owned(), "lat".to_owned()];
        assert!(
            validate_point_columns(&geo, "", &two, &cols).is_err(),
            "one measure only"
        );
        assert!(
            validate_point_columns(&geo, "", &["lat".to_owned()], &cols).is_err(),
            "measure is a coordinate"
        );
        assert!(
            validate_point_columns(&geo, "lon", &one, &cols).is_err(),
            "label is a coordinate"
        );
        assert!(
            validate_point_columns(&geo, "visitors", &one, &cols).is_err(),
            "label is the measure"
        );
    }

    #[test]
    fn a_point_map_needs_no_dimension_but_other_charts_still_do() {
        let cols = column_set(&["lat", "lon", "visitors", "place"]);
        let m1 = vec!["visitors".to_owned()];
        for kind in [ChartKind::Pointmap, ChartKind::Geoheat] {
            assert!(validate_chart_shape(kind, "", &m1, "", &cols).is_ok());
            assert!(validate_chart_shape(kind, "place", &m1, "", &cols).is_ok());
            assert!(validate_chart_shape(kind, "nope", &m1, "", &cols).is_err());
            assert!(
                validate_chart_shape(kind, "place", &m1, "lat", &cols).is_err(),
                "a point map takes no breakdown"
            );
        }
        assert!(validate_chart_shape(ChartKind::Geomap, "", &m1, "", &cols).is_err());
    }

    #[test]
    fn a_point_map_has_a_fixed_limit_and_its_input_limit_is_ignored() {
        let mart = Relation::Mart(Ident::new("mart_x").unwrap());
        assert_eq!(crate::builder::point_limit(&mart), 5000);
        let sql = build_point_chart_sql(
            &mart,
            &GeoFields {
                map: None,
                lat: Some("lat".to_owned()),
                lon: Some("lon".to_owned()),
            },
            "",
            &["visitors".to_owned()],
            "avg",
        )
        .unwrap();
        assert!(sql.contains("round(avg(visitors)) AS visitors"), "{sql}");
        assert!(sql.ends_with("LIMIT 5000"), "{sql}");
    }

    #[test]
    fn parse_layout_handles_empty_and_malformed_json() {
        assert_eq!(parse_layout(""), LayoutMap::new());
        assert_eq!(parse_layout("not json"), LayoutMap::new());
        let mut want = LayoutMap::new();
        want.insert(
            "c1".to_owned(),
            TileBox {
                x: 0,
                y: 0,
                w: 4,
                h: 2,
            },
        );
        assert_eq!(parse_layout(r#"{"c1":{"x":0,"y":0,"w":4,"h":2}}"#), want);
    }

    #[test]
    fn parse_filters_handles_empty_and_malformed_json() {
        assert_eq!(parse_filters(""), Vec::<FilterDef>::new());
        assert_eq!(parse_filters("nope"), Vec::<FilterDef>::new());
        assert_eq!(
            parse_filters(r#"[{"column":"kawasan","values":["Asia"]}]"#),
            vec![FilterDef::in_values("kawasan", vec!["Asia".to_owned()])]
        );
    }

    #[test]
    fn random_hex_produces_expected_length() {
        assert_eq!(random_hex(4).len(), 8);
        assert_eq!(random_hex(16).len(), 32);
    }

    #[test]
    fn new_ids_have_expected_prefixes() {
        assert!(new_board_id().starts_with("b_"));
        assert!(new_chart_id().starts_with("u_"));
        assert!(new_public_token().starts_with("p_"));
    }

    #[test]
    fn stored_envelope_supports_legacy_bare_spec_format() {
        let legacy = serde_json::json!({
            "id": "u_1", "title": "T", "kind": "bar", "mart": "mart_x",
            "sql": "SELECT 1", "x": "a", "y": "b"
        });
        let parsed: StoredEnvelope = serde_json::from_value(legacy).unwrap();
        assert_eq!(parsed.spec.id, "u_1");
        assert!(parsed.def.is_none());
    }

    /// Regression test for B1 (`lakehouse-bi` was dropping 100% of live
    /// stored charts): the NEW `{spec, def, hasYear}` envelope, captured
    /// verbatim from a real row in `console.bi_chart` on the live cluster
    /// (`id = "u_f1b0fd25"`) via a read-only `SELECT`. Before the fix, this
    /// literal failed to deserialize under the old `#[serde(flatten)]`
    /// struct (it expects `id`/`title`/... at the top level, not nested
    /// under `spec`), so every live row like this one was silently
    /// skipped — a total data loss, not a partial one, since all 15 live
    /// rows use this shape.
    #[test]
    fn stored_envelope_supports_new_spec_def_has_year_format() {
        let live_row = r#"{"spec":{"id":"u_f1b0fd25","title":"Combo · dtw","kind":"combo","mart":"mart_kunjungan_dtw","sql":"SELECT destinasi, round(sum(wisnus)) AS wisnus, round(sum(wisman)) AS wisman FROM serving.mart_kunjungan_dtw GROUP BY destinasi ORDER BY wisnus DESC LIMIT 8","x":"destinasi","y":["wisnus","wisman"],"format":"int","span":1},"def":{"title":"Combo · dtw","mart":"mart_kunjungan_dtw","kind":"combo","dimension":"destinasi","measures":["wisnus","wisman"],"aggregate":"sum","limit":8,"order":"desc","span":1,"board":"b_5cbfb279"},"hasYear":false}"#;
        let parsed: StoredEnvelope = serde_json::from_str(live_row).unwrap();
        assert_eq!(parsed.spec.id, "u_f1b0fd25");
        assert_eq!(parsed.spec.kind, ChartKind::Combo);
        assert_eq!(parsed.spec.mart, "mart_kunjungan_dtw");
        assert_eq!(
            parsed.spec.y,
            ChartY::Multi(vec!["wisnus".to_owned(), "wisman".to_owned()])
        );
        let def = parsed.def.expect("def must be present in the new envelope");
        assert_eq!(def.dimension, "destinasi");
        assert_eq!(def.measures, vec!["wisnus".to_owned(), "wisman".to_owned()]);
        assert_eq!(parsed.has_year, Some(false));
    }

    /// `hasYear: true` must round-trip as `Some(true)` — this is the
    /// specific field the bug report called out as silently defaulting to
    /// `false` even after fixing the envelope shape (missing `#[serde(rename
    /// = "hasYear")]`, now handled by the manual `Deserialize` impl reading
    /// the `"hasYear"` key directly).
    #[test]
    fn stored_envelope_reads_has_year_true() {
        let with_year = serde_json::json!({
            "spec": {"id": "u_2", "title": "T", "kind": "bar", "mart": "mart_x",
                      "sql": "SELECT 1", "x": "a", "y": "b"},
            "def": {"title": "T", "mart": "mart_x", "kind": "bar", "dimension": "a", "measures": ["b"]},
            "hasYear": true
        });
        let parsed: StoredEnvelope = serde_json::from_value(with_year).unwrap();
        assert_eq!(parsed.has_year, Some(true));
    }

    /// SEC-12: the three embed columns come back as the board's access state
    /// (a `UInt64` arrives quoted), a row from before they existed reads as
    /// "nothing withdrawn, no site may frame it", and none of it is
    /// serialised into a dashboard listing.
    #[test]
    fn row_to_board_reads_the_embed_access_columns_and_does_not_serialise_them() {
        let row = serde_json::json!({
            "id": "b_1", "name": "Dash", "embed_enabled": "1",
            "embed_revoked_before": "1700000000",
            "embed_revoked_jti_json": "[{\"jti\":\"j1\",\"exp\":1700003600}]",
            "embed_origins_json": "[\"https://app.customer.example\"]",
        });
        let board = row_to_board(row.as_object().unwrap());
        assert_eq!(board.embed_access.revoked_before, 1_700_000_000);
        assert_eq!(board.embed_access.withdrawn.len(), 1);
        assert_eq!(
            board.embed_access.origins,
            vec!["https://app.customer.example"]
        );
        let json = serde_json::to_string(&board).unwrap();
        assert!(
            !json.contains("j1") && !json.contains("customer.example"),
            "{json}"
        );

        let old_row = serde_json::json!({ "id": "b_old", "name": "Old" });
        let old = row_to_board(old_row.as_object().unwrap());
        assert_eq!(old.embed_access, EmbedAccess::default());
    }

    /// `Board`'s JSON wire shape must match the TS `Board` type
    /// (`createdAt`/`publicToken`/`embedEnabled`, not `snake_case`) — this
    /// struct isn't wired to a route yet, but when it is, the mismatch
    /// found in the B1 audit would otherwise silently drop these fields
    /// from every dashboard API response.
    #[test]
    fn board_serializes_camel_case() {
        let board = Board {
            id: "b_1".to_owned(),
            name: "Dash".to_owned(),
            description: Some("Ringkasan kunjungan".to_owned()),
            created_by: Some("Bootstrap Admin".to_owned()),
            layout: None,
            filters: None,
            created_at: Some("2026-01-01 00:00:00".to_owned()),
            updated_at: Some("2026-01-02 00:00:00".to_owned()),
            public_token: Some("p_abc".to_owned()),
            embed_enabled: Some(true),
            folder_id: Some("f_1".to_owned()),
            embed_access: EmbedAccess::default(),
        };
        let json = serde_json::to_value(&board).unwrap();
        assert_eq!(json.get("createdAt").unwrap(), "2026-01-01 00:00:00");
        assert_eq!(json.get("updatedAt").unwrap(), "2026-01-02 00:00:00");
        assert_eq!(json.get("createdBy").unwrap(), "Bootstrap Admin");
        assert_eq!(json.get("description").unwrap(), "Ringkasan kunjungan");
        assert_eq!(json.get("publicToken").unwrap(), "p_abc");
        assert_eq!(json.get("embedEnabled").unwrap(), true);
        assert_eq!(json.get("folderId").unwrap(), "f_1");
        assert!(json.get("created_at").is_none());
        assert!(json.get("updated_at").is_none());
        assert!(json.get("created_by").is_none());
        assert!(json.get("public_token").is_none());
        assert!(json.get("embed_enabled").is_none());
    }

    /// D1 regression: the AI tool schema for `create_chart`/`update_chart`
    /// (`ai.rs`) only advertises `required: ["title", "kind"]` — a
    /// schema-valid call for a `text` chart genuinely omits `mart`,
    /// `dimension`, and `measures`. Before `#[serde(default)]` was added to
    /// those fields, this failed at deserialization with `missing field
    /// 'mart'` rather than reaching [`spec_from_input`]'s validation, making
    /// `kind: "text"` charts unreachable via AI chat (a functional
    /// regression vs. the lenient `specFromInput` in `bi-store.ts`).
    #[test]
    fn chart_input_decodes_minimal_text_tool_args() {
        let input: ChartInput = serde_json::from_value(
            serde_json::json!({ "title": "Catatan", "kind": "text" }),
        )
        .expect("minimal text tool args must decode, not fail on missing mart/dimension/measures");
        assert_eq!(input.kind, ChartKind::Text);
        assert_eq!(input.mart, "");
        assert_eq!(input.dimension, "");
        assert!(input.measures.is_empty());
    }

    /// Same D1 regression, for `kind: "kpi"`: the tool schema doesn't
    /// require `dimension` (KPIs are dimensionless), so a schema-valid call
    /// omits it — decoding must not fail on that either.
    #[test]
    fn chart_input_decodes_minimal_kpi_tool_args() {
        let input: ChartInput = serde_json::from_value(serde_json::json!({
            "title": "Total Kunjungan",
            "kind": "kpi",
            "mart": "mart_wisman",
            "measures": ["total"],
        }))
        .expect("minimal kpi tool args (no dimension) must decode");
        assert_eq!(input.kind, ChartKind::Kpi);
        assert_eq!(input.mart, "mart_wisman");
        assert_eq!(input.dimension, "");
        assert_eq!(input.measures, vec!["total".to_owned()]);
    }

    // ── charts over SQL that is not stored (`spec_from_inline_sql`) ──────

    fn source_columns(pairs: &[(&str, &str)]) -> Vec<crate::sources::SourceColumn> {
        pairs
            .iter()
            .map(|(name, ty)| crate::sources::SourceColumn {
                name: (*name).to_owned(),
                ty: (*ty).to_owned(),
            })
            .collect()
    }

    fn inline_chart(extra: &serde_json::Value) -> ChartInput {
        let mut base = serde_json::json!({
            "title": "Visitors", "kind": "hbar", "dimension": "place", "measures": ["visitors"],
        });
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    const INLINE_SQL: &str = "SELECT place, lat, lon, visitors FROM serving.mart_x";

    fn inline_cols() -> Vec<crate::sources::SourceColumn> {
        source_columns(&[
            ("place", "String"),
            ("lat", "Float64"),
            ("lon", "Float64"),
            ("visitors", "UInt64"),
        ])
    }

    /// The reason the inline path exists: a chart over unsaved SQL must be
    /// the chart the same input yields once the SQL is saved, so the preview
    /// shows what will be stored. Both go through `spec_from_resolved`; this
    /// pins that the two relations feeding it agree.
    #[test]
    fn a_chart_over_inline_sql_is_built_exactly_like_one_over_the_same_stored_source() {
        let cols = inline_cols();
        let input = inline_chart(&serde_json::json!({}));
        let inline = spec_from_inline_sql(
            &input,
            InlineSql {
                sql: INLINE_SQL,
                columns: &cols,
            },
            ChartSource::Ui,
            "ui",
        )
        .unwrap();

        let stored_source = crate::sources::SqlSource {
            id: "s_1234abcd".to_owned(),
            title: "t".to_owned(),
            sql: INLINE_SQL.to_owned(),
            columns: cols,
            folder_id: String::new(),
            created_by: String::new(),
            updated_at: None,
        };
        let mut stored_input = input;
        stored_input.sql_source = Some("s_1234abcd".to_owned());
        let stored = spec_from_resolved(
            &stored_input,
            ChartSource::Ui,
            "ui",
            derive_common_fields(&stored_input, None).unwrap(),
            geo_fields(ChartKind::Hbar, &stored_input).unwrap(),
            resolved_from_stored(stored_source),
        )
        .unwrap();

        assert_eq!(inline.spec.sql, stored.spec.sql);
        assert!(inline.spec.sql.contains("FROM (\n"), "{}", inline.spec.sql);
        assert!(inline.spec.sql.contains("SETTINGS"), "{}", inline.spec.sql);
        assert_eq!(inline.spec.x, stored.spec.x);
        assert_eq!(inline.spec.y, stored.spec.y);
        assert_eq!(inline.spec.mart, stored.spec.mart);
        assert_eq!(inline.spec.sql_source.as_deref(), Some(UNSAVED_SOURCE_ID));
    }

    #[test]
    fn a_chart_over_inline_sql_naming_a_column_the_sql_does_not_return_is_refused() {
        let cols = inline_cols();
        for extra in [
            serde_json::json!({ "measures": ["not_returned"] }),
            serde_json::json!({ "dimension": "not_returned" }),
        ] {
            let err = spec_from_inline_sql(
                &inline_chart(&extra),
                InlineSql {
                    sql: INLINE_SQL,
                    columns: &cols,
                },
                ChartSource::Ui,
                "ui",
            )
            .unwrap_err();
            assert!(matches!(err, BiError::Validation(_)), "{err:?}");
        }
        let err = spec_from_inline_sql(
            &inline_chart(&serde_json::json!({ "measures": ["not_returned"] })),
            InlineSql {
                sql: INLINE_SQL,
                columns: &cols,
            },
            ChartSource::Ui,
            "ui",
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "invalid or missing measure column.");
    }

    #[test]
    fn a_point_map_over_inline_sql_is_capped_like_a_stored_source() {
        let cols = inline_cols();
        let input = inline_chart(&serde_json::json!({
            "kind": "pointmap", "dimension": "place", "lat": "lat", "lon": "lon",
        }));
        let spec = spec_from_inline_sql(
            &input,
            InlineSql {
                sql: INLINE_SQL,
                columns: &cols,
            },
            ChartSource::Ui,
            "ui",
        )
        .unwrap();
        assert!(spec.spec.sql.contains("LIMIT 2000"), "{}", spec.spec.sql);
        assert_eq!(spec.spec.lat.as_deref(), Some("lat"));
        assert_eq!(spec.spec.sql_source.as_deref(), Some(UNSAVED_SOURCE_ID));

        let missing_lon = inline_chart(&serde_json::json!({
            "kind": "pointmap", "dimension": "place", "lat": "lat", "lon": "nope",
        }));
        assert!(
            spec_from_inline_sql(
                &missing_lon,
                InlineSql {
                    sql: INLINE_SQL,
                    columns: &cols
                },
                ChartSource::Ui,
                "ui",
            )
            .is_err()
        );
    }

    #[test]
    fn a_chart_over_inline_sql_cannot_also_name_a_mart_or_a_source() {
        let cols = inline_cols();
        for extra in [
            serde_json::json!({ "mart": "mart_x" }),
            serde_json::json!({ "sqlSource": "s_1234abcd" }),
        ] {
            let err = spec_from_inline_sql(
                &inline_chart(&extra),
                InlineSql {
                    sql: INLINE_SQL,
                    columns: &cols,
                },
                ChartSource::Ui,
                "ui",
            )
            .unwrap_err();
            assert!(err.to_string().contains("unsaved SQL"), "{err}");
        }
    }
}
