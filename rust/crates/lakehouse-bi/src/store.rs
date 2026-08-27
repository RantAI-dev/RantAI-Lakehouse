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

use crate::builder::{QueryBuilder, build_kpi_sql};
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
    ChartKind::Kpi,
    ChartKind::Gauge,
    ChartKind::Table,
    ChartKind::Text,
];

/// Kinds allowed to carry a breakdown (2nd dimension). `heatmap` REQUIRES one.
/// Mirrors the TS `BREAKDOWN_KINDS` set.
fn breakdown_allowed(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Bar | ChartKind::Hbar | ChartKind::Line | ChartKind::Area | ChartKind::Heatmap
    )
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
    /// Source mart (unqualified, e.g. `mart_wisman`), empty for `text`.
    pub mart: String,
    /// `ClickHouse` SQL that returns the chart's rows, empty for `text`.
    pub sql: String,
    /// Column name for the X axis / category.
    pub x: String,
    /// Column name(s) for the Y axis / measure(s).
    pub y: ChartY,
    /// Optional 2nd-dimension breakdown column.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub series: Option<String>,
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
    pub mart: String,
    /// How to render the data.
    pub kind: ChartKind,
    /// X-axis / category column.
    pub dimension: String,
    /// Value column(s); more than one only for `stacked`.
    pub measures: Vec<String>,
    /// Optional 2nd dimension: splits into multiple series.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub breakdown: Option<String>,
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
    /// Tile layout, by chart id.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub layout: Option<LayoutMap>,
    /// Dashboard-wide dimension filters.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub filters: Option<Vec<FilterDef>>,
    /// `ClickHouse`-formatted creation timestamp.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_at: Option<String>,
    /// Public read-only share token, when enabled.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub public_token: Option<String>,
    /// Whether signed (JWT) embedding is enabled.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub embed_enabled: Option<bool>,
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

/// A dashboard filter: a column's allowed values, applied to every tile that
/// has that column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterDef {
    /// The column to filter on.
    pub column: String,
    /// Allowed values.
    pub values: Vec<String>,
}

// ── id generation ───────────────────────────────────────────────────────
// Mirrors `randomUUID().slice(0, 8)` (first 8 hex chars of a v4 UUID's
// first group, which carries no version/variant bits and is therefore
// uniformly random) / `randomUUID().replace(/-/g, "")` (32 hex chars).

fn random_hex(n_bytes: usize) -> String {
    use std::fmt::Write as _;

    use rand::RngCore;
    let mut bytes = vec![0_u8; n_bytes];
    rand::thread_rng().fill_bytes(&mut bytes);
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
    Ok(())
}

// ── Boards ──────────────────────────────────────────────────────────────

const BOARD_COLS: &str = "id, name, layout_json, filters_json, public_token, embed_enabled, toString(created_at) AS created_at";

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
    Board {
        id: row_str(row, "id").to_owned(),
        name: row_str(row, "name").to_owned(),
        layout: Some(parse_layout(layout_json)),
        filters: Some(parse_filters(filters_json)),
        created_at: Some(row_str(row, "created_at").to_owned()),
        public_token: Some(public_token.to_owned()),
        embed_enabled: Some(embed_enabled == 1),
    }
}

/// List every non-deleted board, oldest first. Ports `listBoards`.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn list_boards(ch: &ChClient) -> Result<Vec<Board>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!("SELECT {BOARD_COLS} FROM console.bi_board FINAL WHERE is_deleted = 0 ORDER BY created_at"),
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
pub async fn create_board(ch: &ChClient, name: &str) -> Result<Board, BiError> {
    ensure_bi_table(ch).await?;
    let clean = name.trim();
    if clean.is_empty() {
        return Err(BiError::Validation("nama dashboard wajib.".to_owned()));
    }
    let id = new_board_id();
    let sql = format!(
        "INSERT INTO console.bi_board (id, name) VALUES ({}, {})",
        SqlLiteral::from(id.as_str()),
        SqlLiteral::from(clean)
    );
    ch.exec(&sql, None).await?;
    Ok(Board {
        id,
        name: clean.to_owned(),
        layout: Some(LayoutMap::new()),
        filters: Some(Vec::new()),
        created_at: None,
        public_token: None,
        embed_enabled: None,
    })
}

/// INSERT (not `ALTER ... UPDATE`) — `ReplacingMergeTree`, instant & consistent.
async fn upsert_board(
    ch: &ChClient,
    id: &str,
    name: &str,
    layout: &LayoutMap,
    filters: &[FilterDef],
    public_token: &str,
    embed_enabled: bool,
) -> Result<(), ChError> {
    let layout_json = serde_json::to_string(layout).unwrap_or_else(|_| "{}".to_owned());
    let filters_json = serde_json::to_string(filters).unwrap_or_else(|_| "[]".to_owned());
    let sql = format!(
        "INSERT INTO console.bi_board (id, name, layout_json, filters_json, public_token, embed_enabled) VALUES \
         ({}, {}, {}, {}, {}, {})",
        SqlLiteral::from(id),
        SqlLiteral::from(name),
        SqlLiteral::from(layout_json),
        SqlLiteral::from(filters_json),
        SqlLiteral::from(public_token),
        i32::from(embed_enabled),
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
    upsert_board(
        ch,
        &board.id,
        name,
        layout,
        filters,
        public_token,
        embed_enabled,
    )
    .await
}

/// Fields that can be patched onto a [`Board`] before it is re-saved. Mirrors
/// the TS `saveBoardFrom(b, patch: Partial<Board>)` helper.
#[derive(Default)]
struct BoardPatch<'a> {
    name: Option<&'a str>,
    layout: Option<&'a LayoutMap>,
    filters: Option<&'a [FilterDef]>,
    public_token: Option<&'a str>,
    embed_enabled: Option<bool>,
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
        return Err(BiError::Validation("nama wajib.".to_owned()));
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
        layout: None,
        filters: None,
        created_at: None,
        public_token: None,
        embed_enabled: None,
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
        layout: None,
        filters: None,
        created_at: None,
        public_token: None,
        embed_enabled: None,
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
        .ok_or_else(|| BiError::Validation("dashboard tidak ditemukan.".to_owned()))?;
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
        .ok_or_else(|| BiError::Validation("dashboard tidak ditemukan.".to_owned()))?;
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
pub async fn duplicate_board(ch: &ChClient, id: &str) -> Result<Board, BiError> {
    ensure_bi_table(ch).await?;
    let src = get_board(ch, id)
        .await?
        .ok_or_else(|| BiError::Validation("dashboard tidak ditemukan.".to_owned()))?;
    let charts = list_stored_charts(ch).await?;
    let charts: Vec<_> = charts.into_iter().filter(|c| c.board == id).collect();
    let new_board = create_board(ch, &format!("{} (salinan)", src.name)).await?;
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
    Ok(Board {
        layout: Some(new_layout),
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
        kind: ChartKind::Table,
        dimension: String::new(),
        measures: Vec::new(),
        breakdown: None,
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
            "mart Gold '{mart}' tidak ditemukan di serving."
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
        kind,
        dimension: String::new(),
        measures: vec![m.clone()],
        breakdown: None,
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
        .map_err(|_| BiError::Validation("kolom measure tidak valid / tak ada.".to_owned()))?;
    let mart_ident = Ident::new(mart.as_str())
        .map_err(|_| BiError::Validation(format!("nama mart tidak valid: {mart}")))?;
    // `agg` was already checked against `aggregate_allowed` above, so this
    // conversion is exact (never hits the `Sum` fallback).
    let sql = build_kpi_sql(
        &mart_ident,
        &measure_ident,
        Aggregate::from_str_lossy(&agg),
        &[],
    );
    let spec = ChartSpec {
        id: new_id,
        title,
        subtitle,
        kind,
        mart,
        sql,
        x: String::new(),
        y: ChartY::Single("v".to_owned()),
        series: None,
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
        return Err(BiError::Validation("konten teks wajib.".to_owned()));
    }
    let def = ChartInput {
        title: title.clone(),
        subtitle: None,
        mart: String::new(),
        kind,
        dimension: String::new(),
        measures: Vec::new(),
        breakdown: None,
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
        sql: String::new(),
        x: String::new(),
        y: ChartY::Single(String::new()),
        series: None,
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
    agg: String,
    measures: Vec<String>,
    span: u8,
    board: String,
    new_id: String,
    source: ChartSource,
    has_year: bool,
    created_by: &'a str,
    cols: std::collections::HashSet<String>,
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
    if !IDENT_ALLOWED(dimension) || !cols.contains(dimension) {
        return Err(BiError::Validation(format!(
            "kolom dimensi '{dimension}' tak valid / tak ada."
        )));
    }
    if kind == ChartKind::Stacked && measures.len() < 2 {
        return Err(BiError::Validation(
            "chart 'stacked' butuh ≥2 measure.".to_owned(),
        ));
    }
    if (kind == ChartKind::Scatter || kind == ChartKind::Combo) && measures.len() < 2 {
        let label = serde_json::to_value(kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        return Err(BiError::Validation(format!(
            "chart '{label}' butuh 2 measure (X & Y)."
        )));
    }
    if kind == ChartKind::Bubble && measures.len() < 3 {
        return Err(BiError::Validation(
            "chart 'bubble' butuh 3 measure (X, Y, ukuran).".to_owned(),
        ));
    }
    if !breakdown.is_empty() {
        if !IDENT_ALLOWED(breakdown) || !cols.contains(breakdown) {
            return Err(BiError::Validation(format!(
                "kolom breakdown '{breakdown}' tak valid / tak ada."
            )));
        }
        if breakdown == dimension {
            return Err(BiError::Validation(
                "breakdown harus beda dari dimensi.".to_owned(),
            ));
        }
        if !breakdown_allowed(kind) {
            return Err(BiError::Validation(
                "breakdown hanya untuk bar/hbar/line/area/heatmap.".to_owned(),
            ));
        }
        if measures.len() > 1 {
            return Err(BiError::Validation(
                "dengan breakdown, pakai tepat satu measure.".to_owned(),
            ));
        }
    }
    if kind == ChartKind::Heatmap && breakdown.is_empty() {
        return Err(BiError::Validation(
            "heatmap butuh breakdown (dimensi ke-2).".to_owned(),
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
    mart: &str,
    dimension: &str,
    measures: &[String],
    agg: &str,
    order: &str,
    limit: u32,
    breakdown: Option<&str>,
) -> Result<String, BiError> {
    let mart_ident = Ident::new(mart)
        .map_err(|_| BiError::Validation(format!("nama mart tidak valid: {mart}")))?;
    let dimension_ident = Ident::new(dimension).map_err(|_| {
        BiError::Validation(format!("kolom dimensi '{dimension}' tak valid / tak ada."))
    })?;
    let mut measure_idents = Vec::with_capacity(measures.len());
    for m in measures {
        measure_idents.push(
            Ident::new(m.as_str()).map_err(|_| {
                BiError::Validation("kolom measure tidak valid / tak ada.".to_owned())
            })?,
        );
    }
    let breakdown_ident = match breakdown {
        Some(b) => Some(Ident::new(b).map_err(|_| {
            BiError::Validation(format!("kolom breakdown '{b}' tak valid / tak ada."))
        })?),
        None => None,
    };
    // `agg` was already checked against `aggregate_allowed` by the caller
    // (`spec_from_chart_input`), so this conversion is exact.
    Ok(QueryBuilder::new(mart_ident)
        .dimension(dimension_ident)
        .aggregate(Aggregate::from_str_lossy(agg))
        .order(order)
        .limit(limit)
        .breakdown(breakdown_ident)
        .measures(measure_idents)
        .build())
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
        agg,
        measures,
        span,
        board,
        new_id,
        source,
        has_year,
        created_by,
        cols,
    } = ctx;

    let dimension = input.dimension.clone();
    let limit = input.limit.unwrap_or(20).clamp(1, 100);
    let order = input.order.clone().unwrap_or_else(|| {
        if matches!(kind, ChartKind::Line | ChartKind::Area) {
            "none".to_owned()
        } else {
            "desc".to_owned()
        }
    });
    let breakdown = input.breakdown.clone().unwrap_or_default();
    validate_chart_shape(kind, &dimension, &measures, &breakdown, &cols)?;

    let breakdown_opt = if breakdown.is_empty() {
        None
    } else {
        Some(breakdown)
    };
    let def = ChartInput {
        title: title.clone(),
        subtitle: subtitle.clone(),
        mart: mart.clone(),
        kind,
        dimension: dimension.clone(),
        measures: measures.clone(),
        breakdown: breakdown_opt.clone(),
        aggregate: Some(agg.clone()),
        limit: Some(limit),
        order: Some(order.clone()),
        span: Some(span),
        board: Some(board.clone()),
        text: None,
        caption: None,
        target: None,
    };

    let sql = build_chart_sql(
        &mart,
        &dimension,
        &measures,
        &agg,
        &order,
        limit,
        breakdown_opt.as_deref(),
    )?;

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
        sql,
        x: dimension,
        y,
        series: breakdown_opt,
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
        return Err(BiError::Validation("title wajib diisi.".to_owned()));
    }
    let kind = input.kind;
    if !KINDS.contains(&kind) {
        return Err(BiError::Validation(format!(
            "kind tidak valid: {}",
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
    let CommonFields {
        title,
        kind,
        new_id,
        board,
        span,
        subtitle,
    } = derive_common_fields(input, id)?;

    // ── TEXT — no SQL/mart ────────────────────────────────────────────
    if kind == ChartKind::Text {
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
        );
    }

    // ── kpi/table/chart need a mart ──────────────────────────────────
    let mart = input
        .mart
        .strip_prefix("serving.")
        .unwrap_or(&input.mart)
        .to_owned();
    if !IDENT_ALLOWED(&mart) {
        return Err(BiError::Validation(format!(
            "nama mart tidak valid: {}",
            input.mart
        )));
    }
    let cols = validated_mart_columns(ch, &mart).await?;
    let has_year = cols.contains("tahun");
    let agg = input
        .aggregate
        .clone()
        .unwrap_or_else(|| "sum".to_owned())
        .to_lowercase();
    if !aggregate_allowed(&agg) {
        return Err(BiError::Validation(format!("aggregate tidak valid: {agg}")));
    }
    let measures = input.measures.clone();
    if measures.is_empty() {
        return Err(BiError::Validation(
            "minimal satu kolom measure.".to_owned(),
        ));
    }
    if measures
        .iter()
        .any(|m| !IDENT_ALLOWED(m) || !cols.contains(m))
    {
        return Err(BiError::Validation(
            "kolom measure tidak valid / tak ada.".to_owned(),
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
            agg,
            measures,
            span,
            board,
            new_id,
            source,
            has_year,
            created_by,
            cols,
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
                sql,
                x: dimension.to_owned(),
                y,
                series: None,
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
                kind,
                dimension: dimension.to_owned(),
                measures,
                breakdown: None,
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

    /// Regression test for H1 (`ensure_bi_table` re-issuing all 8 DDL
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
        assert_eq!(
            first_call_requests, 8,
            "first call should issue all 8 DDL statements"
        );

        ensure_bi_table(&ch).await.unwrap();
        let after_second_call = server.received_requests().await.unwrap().len();
        assert_eq!(
            after_second_call, first_call_requests,
            "second call must be a no-op (cached), matching the TS's once-per-process guard"
        );
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
            vec![FilterDef {
                column: "kawasan".to_owned(),
                values: vec!["Asia".to_owned()]
            }]
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
            layout: None,
            filters: None,
            created_at: Some("2026-01-01 00:00:00".to_owned()),
            public_token: Some("p_abc".to_owned()),
            embed_enabled: Some(true),
        };
        let json = serde_json::to_value(&board).unwrap();
        assert_eq!(json.get("createdAt").unwrap(), "2026-01-01 00:00:00");
        assert_eq!(json.get("publicToken").unwrap(), "p_abc");
        assert_eq!(json.get("embedEnabled").unwrap(), true);
        assert!(json.get("created_at").is_none());
        assert!(json.get("public_token").is_none());
        assert!(json.get("embed_enabled").is_none());
    }
}
