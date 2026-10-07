//! The copilot's DATA MAP: a compact, factual description of every table
//! the copilot can query, placed in its system prompt.
//!
//! # Why this replaced the old schema context
//!
//! The schema context the removed text-to-SQL endpoints used gave the model
//! table names and column types only: Gold marts with types, Silver as bare
//! names, and Bronze as catalog slugs. Measured on the local stack
//! (`ops/ai_eval/ai_eval.py`), that left three classes of wrong answer:
//!
//! - **Wrong literal.** Categorical values are stored in the source's own
//!   language (a country column holds `Jepang`, not `Japan`), so a query
//!   the model wrote from the English question matched nothing and it
//!   reported zero.
//! - **Invented coverage.** With no value ranges, a question about a year
//!   the data does not cover was answered from a guess instead of being
//!   declined.
//! - **Guessed Silver columns.** Silver tables were listed without columns,
//!   so every Silver query started with a failed guess.
//!
//! Published text-to-SQL work (column descriptions and sample values in the
//! prompt, value ranges for numeric columns) points the same way, and the
//! gain is largest for small models, which cannot recover from a missing
//! schema by exploring. So each Gold and Silver table is listed here with
//! its row count, every column with its type and catalog description,
//! numeric/date ranges, and the distinct values of low-cardinality text
//! columns.
//!
//! # Cost and freshness
//!
//! Building the map runs a handful of `system.*` queries plus one stats
//! query per table, so the result is cached for [`TTL`] and shared by every
//! chat. A table above [`STATS_ROW_CEILING`] rows is listed without stats
//! rather than scanned. Every section degrades on its own: a missing
//! catalog database drops only the dataset annotations, never the tables.
//!
//! # What this does not do
//!
//! It adds no permission beyond what the old schema context already
//! exposed (table and column names, now with sample values). The rows
//! themselves are still read only through `run_sql`, which goes through
//! `routes::query::run` and its masking/row-filter enforcement. Sample
//! values are read unmasked from `ClickHouse` here, so a column that a
//! masking policy covers is listed without samples
//! ([`masked_columns`]).

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use lakehouse_clickhouse::ChClient;
use lakehouse_core::ident::Ident;
use serde_json::{Map, Value};
use tokio::sync::Mutex;

use crate::routes::support::is_numeric_type;

/// How long one built map is reused. Two minutes keeps a newly loaded
/// table visible almost immediately while sparing every chat turn the
/// per-table stats queries.
const TTL: Duration = Duration::from_secs(120);
/// Upper bound on the rendered map, so a warehouse with hundreds of tables
/// cannot crowd the question out of a small model's context window. Tables
/// past the budget are named in one closing line instead of described.
const MAX_CHARS: usize = 14_000;
/// Distinct values listed per text column.
const SAMPLE_VALUES: usize = 12;
/// Longest single sample value, in characters.
const SAMPLE_VALUE_CHARS: usize = 40;
/// A table with more rows than this is described without stats.
const STATS_ROW_CEILING: u64 = 50_000_000;

static CACHE: LazyLock<Mutex<Option<(Instant, String)>>> = LazyLock::new(|| Mutex::new(None));

/// The rendered DATA MAP, from cache when it is younger than [`TTL`].
///
/// `masked` is the set [`masked_columns`] read from the authored policies,
/// or `None` when the policies could not be read: then no text samples are
/// listed at all, since which columns are masked is unknown. Only a map
/// built with no masked column is cached, so a map carrying samples is
/// never reused after a masking policy appears.
///
/// The lock is held while a stale map is rebuilt, so concurrent chats wait
/// for one rebuild instead of each running the stats queries. An empty
/// result (`ClickHouse` unreachable) is never cached.
pub(crate) async fn data_map(ch: &ChClient, masked: Option<&HashSet<(String, String)>>) -> String {
    let cacheable = masked.is_some_and(HashSet::is_empty);
    let mut guard = CACHE.lock().await;
    if cacheable
        && let Some((built, map)) = guard.as_ref()
        && built.elapsed() < TTL
    {
        return map.clone();
    }
    let map = build(ch, masked).await;
    if cacheable && !map.is_empty() {
        *guard = Some((Instant::now(), map.clone()));
    }
    map
}

/// `(database.table, column)` pairs a masking policy covers for anyone,
/// read from the authored policies' structured `conditions`. Samples are
/// withheld for these columns so the prompt never carries a value the
/// policy engine would have masked in a query result. Deliberately broader
/// than "masked for this principal": the map is shared across principals.
pub(crate) fn masked_columns(conditions: &[String]) -> HashSet<(String, String)> {
    let mut out = HashSet::new();
    for raw in conditions {
        let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        let Some(table) = obj.get("table").and_then(Value::as_str) else {
            continue;
        };
        if let Some(Value::Array(cols)) = obj.get("mask") {
            for col in cols.iter().filter_map(Value::as_str) {
                out.insert((table.to_owned(), col.to_owned()));
            }
        }
    }
    out
}

struct Column {
    name: String,
    ty: String,
}

struct Table {
    db: String,
    name: String,
    rows: Option<u64>,
    columns: Vec<Column>,
}

struct Dataset {
    slug: String,
    title: String,
    description: String,
    source_kind: &'static str,
    publisher: String,
    frequency: String,
    unit: String,
}

fn text(row: &Map<String, Value>, key: &str) -> String {
    match row.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// `primer`/`sekunder` is where a dataset comes from, not a lakehouse
/// layer; the model is told the plain meaning so it never reports it as
/// one.
fn source_kind(tier: &str) -> &'static str {
    match tier {
        "primer" => "primary source",
        "sekunder" => "secondary source",
        _ => "source kind not recorded",
    }
}

fn clean_value(raw: &str) -> String {
    let flat: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flat.chars().count() > SAMPLE_VALUE_CHARS {
        let cut: String = flat.chars().take(SAMPLE_VALUE_CHARS).collect();
        format!("{cut}…")
    } else {
        flat
    }
}

async fn load_tables(ch: &ChClient) -> Vec<Table> {
    let Ok(table_rows) = ch
        .rows(
            "SELECT database, name, toString(total_rows) AS total_rows FROM system.tables \
             WHERE database IN ('serving', 'silver') AND NOT is_temporary \
             AND name NOT LIKE '%\\_baru' ORDER BY database = 'silver', database, name",
            None,
        )
        .await
    else {
        return Vec::new();
    };
    let column_rows = ch
        .rows(
            "SELECT database, table, name, type FROM system.columns \
             WHERE database IN ('serving', 'silver') AND name NOT LIKE '\\_%' \
             ORDER BY database, table, position",
            None,
        )
        .await
        .unwrap_or_default();
    let mut by_table: HashMap<(String, String), Vec<Column>> = HashMap::new();
    for row in &column_rows {
        by_table
            .entry((text(row, "database"), text(row, "table")))
            .or_default()
            .push(Column {
                name: text(row, "name"),
                ty: text(row, "type"),
            });
    }
    table_rows
        .iter()
        .map(|row| {
            let db = text(row, "database");
            let name = text(row, "name");
            let columns = by_table
                .remove(&(db.clone(), name.clone()))
                .unwrap_or_default();
            Table {
                rows: text(row, "total_rows").parse().ok(),
                db,
                name,
                columns,
            }
        })
        .collect()
}

/// Catalog datasets keyed by the Gold table they are served from, plus
/// column descriptions keyed by `(slug, column)`. Both are optional: a
/// deployment without the catalog registry still gets a full table map.
async fn load_catalog(
    ch: &ChClient,
) -> (HashMap<String, Dataset>, HashMap<(String, String), String>) {
    let catalog = super::tools::data::CATALOG_UNION;
    let sync = "(SELECT slug, author, frekuensi, satuan FROM lake.`bronze_meta.dataset_sync` \
                UNION ALL SELECT slug, author, frekuensi, satuan FROM lake.`bronze_meta_sec.dataset_sync`)";
    let with_sync = format!(
        "SELECT c.slug AS slug, c.title AS title, c.description AS description, c.tier AS tier, \
         c.table_name AS table_name, s.author AS author, s.frekuensi AS frequency, s.satuan AS unit \
         FROM {catalog} AS c LEFT JOIN {sync} AS s ON s.slug = c.slug"
    );
    let rows = match ch.rows(&with_sync, None).await {
        Ok(rows) => rows,
        Err(_) => ch
            .rows(
                &format!("SELECT slug, title, description, tier, table_name FROM {catalog}"),
                None,
            )
            .await
            .unwrap_or_default(),
    };
    let mut datasets = HashMap::new();
    for row in &rows {
        datasets.insert(
            text(row, "table_name"),
            Dataset {
                slug: text(row, "slug"),
                title: text(row, "title"),
                description: text(row, "description"),
                source_kind: source_kind(&text(row, "tier")),
                publisher: text(row, "author"),
                frequency: text(row, "frequency"),
                unit: text(row, "unit"),
            },
        );
    }
    let described = ch
        .rows(
            "SELECT slug, key_asli AS col, deskripsi AS description FROM lake.`bronze_meta.dataset_column` \
             UNION ALL SELECT slug, key_asli AS col, deskripsi AS description FROM lake.`bronze_meta_sec.dataset_column`",
            None,
        )
        .await
        .unwrap_or_default();
    let descriptions = described
        .iter()
        .map(|row| {
            (
                (text(row, "slug"), text(row, "col")),
                text(row, "description"),
            )
        })
        .collect();
    (datasets, descriptions)
}

/// Whether `ty` is a type [`column_stats`] summarises as a text column.
fn is_text_type(ty: &str) -> bool {
    ty.contains("String") || ty.starts_with("Enum") || ty.contains("(Enum")
}

/// Whether `ty` is summarised by its range (numbers and dates).
fn is_range_type(ty: &str) -> bool {
    is_numeric_type(ty) || ty.contains("Date")
}

/// One line of facts per column: `min..max` for numbers and dates, the
/// distinct count and up to [`SAMPLE_VALUES`] values for text. Empty for a
/// table that is too big, has no summarisable columns, or fails to answer.
async fn column_stats(
    ch: &ChClient,
    table: &Table,
    masked: Option<&HashSet<(String, String)>>,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if table.rows.is_some_and(|n| n > STATS_ROW_CEILING) {
        return out;
    }
    let (Ok(db), Ok(name)) = (Ident::new(table.db.clone()), Ident::new(table.name.clone())) else {
        return out;
    };
    let qualified = format!("{}.{}", table.db, table.name);
    let mut exprs = Vec::new();
    let mut plan = Vec::new();
    for (i, col) in table.columns.iter().enumerate() {
        let Ok(ident) = Ident::new(col.name.clone()) else {
            continue;
        };
        if is_range_type(&col.ty) {
            exprs.push(format!(
                "toString(min(`{ident}`)) AS lo{i}, toString(max(`{ident}`)) AS hi{i}"
            ));
            plan.push((i, false));
        } else if is_text_type(&col.ty) {
            if masked.is_none_or(|m| m.contains(&(qualified.clone(), col.name.clone()))) {
                continue;
            }
            exprs.push(format!(
                "toString(uniq(`{ident}`)) AS n{i}, \
                 arrayStringConcat(arraySlice(arraySort(groupUniqArray(200)(toString(`{ident}`))), 1, {SAMPLE_VALUES}), '\\u001f') AS v{i}"
            ));
            plan.push((i, true));
        }
    }
    if exprs.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT {} FROM `{db}`.`{name}` SETTINGS max_execution_time = 5",
        exprs.join(", ")
    );
    let Ok(rows) = ch.rows(&sql, None).await else {
        return out;
    };
    let Some(row) = rows.first() else {
        return out;
    };
    for (i, is_text) in plan {
        let Some(col) = table.columns.get(i) else {
            continue;
        };
        let fact = if is_text {
            let distinct: u64 = text(row, &format!("n{i}")).parse().unwrap_or(0);
            let values: Vec<String> = text(row, &format!("v{i}"))
                .split('\u{1f}')
                .filter(|v| !v.is_empty())
                .map(clean_value)
                .collect();
            if values.is_empty() {
                continue;
            }
            if distinct <= u64::try_from(values.len()).unwrap_or(u64::MAX) {
                format!("values: {}", values.join(" | "))
            } else {
                format!("{distinct} distinct, e.g. {}", values.join(" | "))
            }
        } else {
            let (lo, hi) = (text(row, &format!("lo{i}")), text(row, &format!("hi{i}")));
            if lo.is_empty() {
                continue;
            }
            format!("range {lo}..{hi}")
        };
        out.insert(col.name.clone(), fact);
    }
    out
}

/// Whether a numeric column identifies a row (a year, a month number, an
/// id or code) rather than being a measure to add up.
fn is_key_like(name: &str) -> bool {
    let n = name.to_lowercase();
    [
        "year", "tahun", "month", "bulan", "day", "hari", "week", "minggu", "quarter", "kuartal",
        "_no", "_id", "kode", "code",
    ]
    .iter()
    .any(|k| n == k.trim_start_matches('_') || n.contains(k))
        || n == "id"
}

/// The table's grain and its measures, for the small-model mistake this
/// was added for: `qwen3:4b` answered "the peak month's visits" with one
/// country's row (`ORDER BY jumlah DESC LIMIT 1`) instead of the month's
/// total, because nothing said a row is one combination of several
/// columns. `None` when the table has no numeric measure.
fn grain_line(table: &Table) -> Option<String> {
    let (measures, keys): (Vec<&Column>, Vec<&Column>) = table
        .columns
        .iter()
        .partition(|c| is_numeric_type(&c.ty) && !is_key_like(&c.name));
    if measures.is_empty() || keys.is_empty() {
        return None;
    }
    let keys: Vec<&str> = keys.iter().map(|c| c.name.as_str()).collect();
    let measures: Vec<&str> = measures.iter().map(|c| c.name.as_str()).collect();
    Some(format!(
        "    grain: one row per {}; measures: {} (SUM them over rows for any total)\n",
        keys.join(" x "),
        measures.join(", ")
    ))
}

/// One table's entry: its line (row count, and the catalog dataset it
/// serves, if any) and one line per column with its description and
/// stats.
fn render_table(
    out: &mut String,
    table: &Table,
    dataset: Option<&Dataset>,
    descriptions: &HashMap<(String, String), String>,
    stats: &HashMap<String, String>,
) {
    let rows = table
        .rows
        .map_or_else(|| "row count unknown".to_owned(), |n| format!("{n} rows"));
    let _ = write!(out, "- {}.{} ({rows})", table.db, table.name);
    if let Some(d) = dataset {
        let _ = write!(
            out,
            " — dataset \"{}\" (slug {}, {}",
            d.title, d.slug, d.source_kind
        );
        for (label, value) in [
            ("publisher", &d.publisher),
            ("frequency", &d.frequency),
            ("unit", &d.unit),
        ] {
            if !value.is_empty() {
                let _ = write!(out, ", {label}: {value}");
            }
        }
        out.push(')');
        if !d.description.is_empty() {
            let _ = write!(out, ": {}", d.description);
        }
    }
    out.push('\n');
    if let Some(grain) = grain_line(table) {
        out.push_str(&grain);
    }
    for col in &table.columns {
        let _ = write!(out, "    {} {}", col.name, col.ty);
        if let Some(d) = dataset
            && let Some(desc) = descriptions.get(&(d.slug.clone(), col.name.clone()))
            && !desc.is_empty()
        {
            let _ = write!(out, " — {desc}");
        }
        if let Some(fact) = stats.get(&col.name) {
            let _ = write!(out, "; {fact}");
        }
        out.push('\n');
    }
}

async fn build(ch: &ChClient, masked: Option<&HashSet<(String, String)>>) -> String {
    let tables = load_tables(ch).await;
    if tables.is_empty() {
        return String::new();
    }
    let (datasets, descriptions) = load_catalog(ch).await;

    let mut out = String::new();
    let mut section = "";
    let mut skipped: Vec<String> = Vec::new();
    for table in &tables {
        let qualified = format!("{}.{}", table.db, table.name);
        if out.len() > MAX_CHARS {
            skipped.push(qualified);
            continue;
        }
        let heading = if table.db == "serving" {
            "GOLD (database `serving`, aggregated marts; use these first for numbers):"
        } else {
            "SILVER (database `silver`, cleaned detail rows):"
        };
        if heading != section {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(heading);
            out.push('\n');
            section = heading;
        }
        let dataset = if table.db == "serving" {
            datasets.get(&table.name)
        } else {
            None
        };
        let stats = column_stats(ch, table, masked).await;
        render_table(&mut out, table, dataset, &descriptions, &stats);
    }
    if !skipped.is_empty() {
        let _ = write!(
            out,
            "\n{} more tables not described here (budget): {}. Use describe_mart or \
             `SELECT name, type FROM system.columns WHERE database = '…' AND table = '…'` to see their columns.\n",
            skipped.len(),
            skipped
                .iter()
                .take(60)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let unserved: Vec<String> = datasets
        .iter()
        .filter(|(table, _)| {
            !tables
                .iter()
                .any(|t| t.db == "serving" && &t.name == *table)
        })
        .map(|(table, d)| {
            format!(
                "{} (\"{}\", expected at serving.{table}, not found)",
                d.slug, d.title
            )
        })
        .collect();
    if !unserved.is_empty() {
        let _ = write!(
            out,
            "\nCatalog datasets with no Gold table: {}.\n",
            unserved.join(", ")
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_kind_names_the_origin_never_a_layer() {
        assert_eq!(source_kind("primer"), "primary source");
        assert_eq!(source_kind("sekunder"), "secondary source");
        assert_eq!(source_kind(""), "source kind not recorded");
    }

    #[test]
    fn masked_columns_reads_structured_conditions_and_skips_prose() {
        let masked = masked_columns(&[
            r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email","phone"]}"#.to_owned(),
            "Analysts may not see personal data".to_owned(),
            r#"{"table":"serving.mart_y","rowFilter":"region = 'x'"}"#.to_owned(),
        ]);
        assert!(masked.contains(&("serving.mart_x".to_owned(), "email".to_owned())));
        assert!(masked.contains(&("serving.mart_x".to_owned(), "phone".to_owned())));
        assert_eq!(masked.len(), 2);
    }

    #[test]
    fn clean_value_flattens_control_characters_and_truncates() {
        assert_eq!(clean_value("a\nb"), "a b");
        let long = "x".repeat(SAMPLE_VALUE_CHARS + 5);
        assert_eq!(clean_value(&long).chars().count(), SAMPLE_VALUE_CHARS + 1);
    }

    #[test]
    fn range_and_text_types_are_told_apart() {
        assert!(is_range_type("UInt16"));
        assert!(is_range_type("Date32"));
        assert!(is_range_type("Nullable(DateTime64(3))"));
        assert!(is_text_type("LowCardinality(String)"));
        assert!(is_text_type("Enum8('a' = 1)"));
        assert!(!is_text_type("UInt8"));
    }

    #[test]
    fn a_table_states_its_grain_and_which_columns_to_sum() {
        let col = |name: &str, ty: &str| Column {
            name: name.to_owned(),
            ty: ty.to_owned(),
        };
        let table = Table {
            db: "serving".to_owned(),
            name: "mart_visits".to_owned(),
            rows: Some(720),
            columns: vec![
                col("tahun", "UInt16"),
                col("bulan_no", "UInt8"),
                col("negara", "String"),
                col("jumlah", "UInt32"),
            ],
        };
        assert_eq!(
            grain_line(&table).as_deref(),
            Some(
                "    grain: one row per tahun x bulan_no x negara; measures: jumlah (SUM them over rows for any total)\n"
            )
        );
        let only_labels = Table {
            columns: vec![col("indikator", "String")],
            ..table
        };
        assert_eq!(grain_line(&only_labels), None);
    }
}
