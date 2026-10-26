//! A dashboard as a file, and the rules for reading one back (`BI-19` part D,
//! plan D14).
//!
//! The file is JSON: `{kind: "rantai-dashboard", version: 1, board, charts,
//! tables}`. It describes one dashboard: its settings, its charts' stored
//! definitions, and the tables and columns those charts read, so an importer
//! can map them to the tables of another deployment. It is a *description*,
//! never a grant:
//!
//! - The top level and the board refuse unknown fields (`deny_unknown_fields`),
//!   so a file cannot carry an owner, a folder, a public link, embed settings
//!   or a version history for the importer to inherit; the importer's own
//!   rights decide all of that (the new dashboard is theirs, in the folder they
//!   name).
//! - Ids in the file (`charts[].id`, the keys of the layout) are only names
//!   inside the file. The import gives every chart a fresh id and re-keys the
//!   layout, so a file cannot overwrite or point at an existing chart or board.
//! - A chart's definition is *input*: it goes through the same
//!   `store::spec_from_input` as a chart saved by hand, so its columns,
//!   calculated fields, metrics and parameters are checked against the real
//!   schema, and a SQL source is a reference to one that already exists and the
//!   importer may read. No SQL source or calculated field is created from a file.
//!
//! This module is the pure part (parse, limits, table mapping, re-keying); the
//! route owns the reads and writes.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::filters::FilterDef;
use crate::layout::{PhoneTile, Tab};
use crate::params::BoardParam;
use crate::store::{Board, ChartInput, LayoutMap, StoredChartSpec};

/// `kind` of every dashboard file.
pub const KIND: &str = "rantai-dashboard";

/// The only `version` this build reads and writes.
pub const VERSION: u32 = 1;

/// The largest file read, in bytes (plan D14).
pub const MAX_BYTES: usize = 1_000_000;

/// The most charts a file may hold (plan D14).
pub const MAX_CHARTS: usize = 200;

/// The longest file-local chart id; ids are names inside the file only.
const MAX_ID_CHARS: usize = 64;

/// A dashboard file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DashboardFile {
    /// Always [`KIND`].
    pub kind: String,
    /// Always [`VERSION`].
    pub version: u32,
    /// The dashboard's own settings.
    pub board: FileBoard,
    /// The charts, in the order they were on the dashboard.
    pub charts: Vec<FileChart>,
    /// Every table the charts read, with the columns it had when exported.
    pub tables: Vec<FileTable>,
}

/// What `duplicate_board` copies, and nothing it does not: the sharing state,
/// owner and folder are not here.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FileBoard {
    /// Display name.
    pub name: String,
    /// One-line purpose.
    #[serde(default)]
    pub description: String,
    /// Dashboard-wide filters, with their tab ties.
    #[serde(default)]
    pub filters: Vec<FilterDef>,
    /// Saved auto-refresh in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_seconds: Option<u32>,
    /// Saved time grain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<String>,
    /// Parameter controls with their saved values.
    #[serde(default)]
    pub params: Vec<BoardParam>,
    /// Tabs, in order.
    #[serde(default)]
    pub tabs: Vec<Tab>,
    /// Tile places, keyed by [`FileChart::id`].
    #[serde(default)]
    pub layout: LayoutMap,
    /// Phone order, by [`FileChart::id`].
    #[serde(default)]
    pub phone_layout: Vec<PhoneTile>,
    /// `light` or `dark`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Font id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
}

/// One chart: a name inside the file and the definition it was saved from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FileChart {
    /// A name inside the file; the import makes a new id.
    pub id: String,
    /// The chart's definition (`ChartInput`), as `spec_from_input` takes it.
    pub def: ChartInput,
}

/// A table the charts read and the columns it had.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FileTable {
    /// The table name, without the `serving.` prefix.
    pub name: String,
    /// Its column names when the file was made (empty when it could not be read).
    #[serde(default)]
    pub columns: Vec<String>,
}

/// Why a file was refused. Each has one fixed sentence ([`Self::message`]); the
/// parser's own text, which could quote the file, never reaches a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileError {
    /// Larger than [`MAX_BYTES`].
    TooBig,
    /// More than [`MAX_CHARTS`] charts.
    TooManyCharts,
    /// Not JSON, or not shaped like a dashboard file.
    NotADashboardFile,
    /// `kind` is not [`KIND`].
    WrongKind,
    /// `version` is not [`VERSION`].
    UnknownVersion,
    /// Two charts share an id, an id is empty or too long, or the layout names
    /// a chart that is not in the file.
    BadChartIds,
}

impl FileError {
    /// The sentence the caller is told.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::TooBig => "That file is larger than 1 MB.",
            Self::TooManyCharts => "That file holds more than 200 charts.",
            Self::NotADashboardFile => "That is not a dashboard file.",
            Self::WrongKind => "That is not a dashboard file from this product.",
            Self::UnknownVersion => "That dashboard file is from a version this one cannot read.",
            Self::BadChartIds => "That dashboard file names its charts inconsistently.",
        }
    }
}

/// Read a file posted as JSON. `size` is the length in bytes of the file as it
/// was posted (the caller measures it before parsing, so a refusal costs
/// nothing).
///
/// # Errors
///
/// A [`FileError`]: too big, too many charts, not a dashboard file, an unknown
/// `kind` or `version`, or chart ids that do not hold together. `kind` and
/// `version` are checked before the rest of the shape, so a file from a later
/// version is told so and not "not a dashboard file".
pub fn parse(raw: Value, size: usize) -> Result<DashboardFile, FileError> {
    if size > MAX_BYTES {
        return Err(FileError::TooBig);
    }
    let Some(map) = raw.as_object() else {
        return Err(FileError::NotADashboardFile);
    };
    if map.get("kind").and_then(Value::as_str) != Some(KIND) {
        return Err(FileError::WrongKind);
    }
    if map.get("version").and_then(Value::as_u64) != Some(u64::from(VERSION)) {
        return Err(FileError::UnknownVersion);
    }
    if map
        .get("charts")
        .and_then(Value::as_array)
        .is_some_and(|c| c.len() > MAX_CHARTS)
    {
        return Err(FileError::TooManyCharts);
    }
    let file: DashboardFile =
        serde_json::from_value(raw).map_err(|_| FileError::NotADashboardFile)?;
    let mut seen = BTreeSet::new();
    for chart in &file.charts {
        if chart.id.is_empty()
            || chart.id.chars().count() > MAX_ID_CHARS
            || !seen.insert(chart.id.as_str())
        {
            return Err(FileError::BadChartIds);
        }
    }
    let names_a_stranger = file
        .board
        .layout
        .keys()
        .chain(file.board.phone_layout.iter().map(|t| &t.id))
        .any(|id| !seen.contains(id.as_str()));
    if names_a_stranger {
        return Err(FileError::BadChartIds);
    }
    Ok(file)
}

/// The name of a table without the `serving.` prefix.
fn bare(name: &str) -> &str {
    name.trim().strip_prefix("serving.").unwrap_or(name.trim())
}

/// Every table a chart reads: its own, its joins', and each layer's with its
/// joins. A chart over a SQL source reads no table by name.
#[must_use]
pub fn chart_tables(def: &ChartInput) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut add = |name: &str| {
        let name = bare(name);
        if !name.is_empty() {
            out.insert(name.to_owned());
        }
    };
    add(&def.mart);
    for join in def.shape.joins_list() {
        add(&join.mart);
    }
    for layer in def.shape.layers_list() {
        add(&layer.mart);
        for join in layer.joins.iter().flatten() {
            add(&join.mart);
        }
    }
    out
}

/// Make the file of a dashboard. `columns` is what is known of each table's
/// columns (a table missing from it is listed with none).
#[must_use]
pub fn export(
    board: &Board,
    charts: &[StoredChartSpec],
    columns: &BTreeMap<String, Vec<String>>,
) -> DashboardFile {
    let mut tables = BTreeSet::new();
    let charts: Vec<FileChart> = charts
        .iter()
        .map(|c| {
            let mut def = c.def.clone();
            // The file is about the dashboard it is in; the id of the one it
            // came from is not part of it.
            def.board = None;
            tables.extend(chart_tables(&def));
            FileChart {
                id: c.spec.id.clone(),
                def,
            }
        })
        .collect();
    let ids: BTreeSet<&str> = charts.iter().map(|c| c.id.as_str()).collect();
    DashboardFile {
        kind: KIND.to_owned(),
        version: VERSION,
        board: FileBoard {
            name: board.name.clone(),
            description: board.description.clone().unwrap_or_default(),
            filters: board.filters.clone().unwrap_or_default(),
            refresh_seconds: board.refresh_seconds,
            grain: board.grain.clone(),
            params: board.params.clone().unwrap_or_default(),
            tabs: board.tabs.clone().unwrap_or_default(),
            layout: board
                .layout
                .iter()
                .flatten()
                .filter(|(id, _)| ids.contains(id.as_str()))
                .map(|(id, tile)| (id.clone(), tile.clone()))
                .collect(),
            phone_layout: board
                .phone_layout
                .iter()
                .flatten()
                .filter(|t| ids.contains(t.id.as_str()))
                .cloned()
                .collect(),
            theme: board.theme.clone(),
            font: board.font.clone(),
        },
        charts,
        tables: tables
            .into_iter()
            .map(|name| FileTable {
                columns: columns.get(&name).cloned().unwrap_or_default(),
                name,
            })
            .collect(),
    }
}

/// Point a chart's definition at the importer's tables and the new dashboard:
/// every table named in it is replaced by its entry in `mapping` (keys are file
/// table names without the prefix).
///
/// Everything else in `def` is left as the file has it: it is validated by
/// `spec_from_input` afterwards, exactly as a chart posted by hand.
///
/// # Errors
///
/// The name of the first table `mapping` has no entry for.
pub fn retarget(
    def: &mut ChartInput,
    mapping: &BTreeMap<String, String>,
    board: &str,
) -> Result<(), String> {
    let map = |name: &str| -> Result<String, String> {
        let key = bare(name);
        if key.is_empty() {
            return Ok(name.to_owned());
        }
        mapping.get(key).cloned().ok_or_else(|| key.to_owned())
    };
    def.mart = map(&def.mart)?;
    if let Some(joins) = def.shape.joins.as_mut() {
        for join in joins {
            join.mart = map(&join.mart)?;
        }
    }
    if let Some(layers) = def.shape.layers.as_mut() {
        for layer in layers {
            layer.mart = map(&layer.mart)?;
            for join in layer.joins.iter_mut().flatten() {
                join.mart = map(&join.mart)?;
            }
        }
    }
    def.board = Some(board.to_owned());
    Ok(())
}

/// The layout and phone layout re-keyed to the new chart ids; an entry whose
/// chart was left out is dropped.
#[must_use]
pub fn rekey(
    layout: &LayoutMap,
    phone: &[PhoneTile],
    ids: &BTreeMap<String, String>,
) -> (LayoutMap, Vec<PhoneTile>) {
    let layout = layout
        .iter()
        .filter_map(|(old, tile)| ids.get(old).map(|new| (new.clone(), tile.clone())))
        .collect();
    let phone = phone
        .iter()
        .filter_map(|t| {
            ids.get(&t.id).map(|new| PhoneTile {
                id: new.clone(),
                h: t.h,
            })
        })
        .collect();
    (layout, phone)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    fn file_json() -> Value {
        json!({
            "kind": KIND, "version": 1,
            "board": {
                "name": "Sales", "description": "d", "filters": [],
                "tabs": [{"id": "t1", "title": "First"}],
                "layout": {"a": {"x": 0, "y": 0, "w": 6, "h": 3, "tab": "t1"}, "b": {"x": 6, "y": 0, "w": 6, "h": 3, "tab": "t1"}},
                "phoneLayout": [{"id": "b", "h": 4}]
            },
            "charts": [
                {"id": "a", "def": {"title": "A", "mart": "serving.mart_a", "kind": "bar", "dimension": "d", "measures": ["m"]}},
                {"id": "b", "def": {"title": "B", "mart": "mart_b", "kind": "bar", "dimension": "d", "measures": ["m"],
                    "joins": [{"mart": "mart_c", "type": "left", "on": [{"left": "k", "op": "=", "right": "k"}]}]}}
            ],
            "tables": [{"name": "mart_a", "columns": ["d", "m"]}]
        })
    }

    #[test]
    fn a_file_reads_and_names_the_tables_its_charts_read() {
        let file = parse(file_json(), 1000).unwrap();
        assert_eq!(file.charts.len(), 2);
        assert_eq!(
            chart_tables(&file.charts[0].def),
            BTreeSet::from(["mart_a".to_owned()])
        );
        assert_eq!(
            chart_tables(&file.charts[1].def),
            BTreeSet::from(["mart_b".to_owned(), "mart_c".to_owned()])
        );
    }

    #[test]
    fn a_file_over_1_mb_over_200_charts_or_of_another_version_is_refused() {
        assert_eq!(
            parse(file_json(), MAX_BYTES + 1).unwrap_err(),
            FileError::TooBig
        );
        let mut many = file_json();
        many["charts"] = json!(
            (0..=MAX_CHARTS)
                .map(|i| json!({"id": format!("c{i}"), "def": {"title": "x", "kind": "text"}}))
                .collect::<Vec<_>>()
        );
        assert_eq!(parse(many, 1000).unwrap_err(), FileError::TooManyCharts);
        let mut later = file_json();
        later["version"] = json!(2);
        assert_eq!(parse(later, 1000).unwrap_err(), FileError::UnknownVersion);
        let mut other = file_json();
        other["kind"] = json!("something-else");
        assert_eq!(parse(other, 1000).unwrap_err(), FileError::WrongKind);
        assert_eq!(
            parse(json!([1]), 10).unwrap_err(),
            FileError::NotADashboardFile
        );
    }

    /// The file cannot carry what only the importer may decide: an owner, a
    /// folder, a public link, embed settings, a chart id of an existing board.
    #[test]
    fn a_file_cannot_carry_an_owner_a_folder_a_public_link_or_embed_settings() {
        for key in [
            "ownerId",
            "folderId",
            "publicToken",
            "embedEnabled",
            "embedOrigins",
            "public",
        ] {
            let mut top = file_json();
            top[key] = json!("x");
            assert_eq!(
                parse(top, 1000).unwrap_err(),
                FileError::NotADashboardFile,
                "{key} at the top"
            );
            let mut board = file_json();
            board["board"][key] = json!("x");
            assert_eq!(
                parse(board, 1000).unwrap_err(),
                FileError::NotADashboardFile,
                "{key} on the board"
            );
        }
    }

    #[test]
    fn chart_ids_must_be_unique_and_the_layout_may_name_only_charts_of_the_file() {
        let mut dup = file_json();
        dup["charts"][1]["id"] = json!("a");
        assert_eq!(parse(dup, 1000).unwrap_err(), FileError::BadChartIds);
        let mut stranger = file_json();
        stranger["board"]["layout"]["u_existing"] = json!({"x": 0, "y": 0, "w": 3, "h": 3});
        assert_eq!(parse(stranger, 1000).unwrap_err(), FileError::BadChartIds);
    }

    #[test]
    fn retargeting_replaces_every_table_and_points_the_chart_at_the_new_board() {
        let file = parse(file_json(), 1000).unwrap();
        let mut def = file.charts[1].def.clone();
        let mapping = BTreeMap::from([
            ("mart_b".to_owned(), "gold_b".to_owned()),
            ("mart_c".to_owned(), "gold_c".to_owned()),
        ]);
        retarget(&mut def, &mapping, "b_new").unwrap();
        assert_eq!(def.mart, "gold_b");
        assert_eq!(def.shape.joins_list()[0].mart, "gold_c");
        assert_eq!(def.board.as_deref(), Some("b_new"));
        let partial = BTreeMap::from([("mart_b".to_owned(), "gold_b".to_owned())]);
        let mut def = file.charts[1].def.clone();
        assert_eq!(retarget(&mut def, &partial, "b_new").unwrap_err(), "mart_c");
    }

    #[test]
    fn the_layout_follows_the_new_ids_and_drops_a_chart_that_was_left_out() {
        let file = parse(file_json(), 1000).unwrap();
        let ids = BTreeMap::from([("b".to_owned(), "u_new".to_owned())]);
        let (layout, phone) = rekey(&file.board.layout, &file.board.phone_layout, &ids);
        assert_eq!(layout.keys().collect::<Vec<_>>(), ["u_new"]);
        assert_eq!(layout["u_new"].tab.as_deref(), Some("t1"));
        assert_eq!(phone.len(), 1);
        assert_eq!(phone[0].id, "u_new");
    }
}
