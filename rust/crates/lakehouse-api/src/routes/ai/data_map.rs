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
//! masking policy covers is listed without a range or a sample
//! ([`masked_columns`]). The stats queries also read every row, so a table
//! that a row-filter policy covers is listed with no stats at all
//! ([`row_filtered_tables`]).

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use lakehouse_clickhouse::ChClient;
use lakehouse_core::ident::Ident;
use lakehouse_store::annotation::AnnotationRow;
use lakehouse_store::chat_term::ChatTerm;
use lakehouse_store::semantic::SemanticEntry;
use serde_json::{Map, Value};
use tokio::sync::Mutex;

use super::prompt::{ENGLISH_WORDS, INDONESIAN_WORDS, tokens};
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

/// How many of the user's latest messages a question is read from: the
/// message that names a table is often the one before a short follow-up.
const QUESTION_MESSAGES: usize = 2;
/// A question word shorter than this is dropped: it is more often a stray
/// fragment than the name of a table.
const MIN_QUESTION_WORD_CHARS: usize = 3;
/// A question word must be at least this long to match a longer table word
/// by its beginning. Shorter, it must equal the table word: "ord" would
/// otherwise bring in every table that has a word starting so.
const MIN_PREFIX_CHARS: usize = 4;
/// Longest description on a table's one-line form, in characters.
const LINE_TEXT_CHARS: usize = 120;

/// The way to see a table's columns, which the budget line and the closing
/// sentence of a map with one-line tables both give.
const SEE_COLUMNS: &str = "Use describe_mart or `SELECT name, type FROM system.columns WHERE database = '…' AND table = '…'` to see their columns.";

const GOLD_HEADING: &str =
    "GOLD (database `serving`, aggregated marts; use these first for numbers):";
const SILVER_HEADING: &str = "SILVER (database `silver`, cleaned detail rows):";

/// One table as the cache holds it. Nothing here depends on the question or
/// on who asks: the cache is shared by every chat, so which tables a
/// question names is decided afterwards, per request, by [`assemble`].
struct TablePiece {
    /// `database.table`.
    qualified: String,
    /// The group heading the table is listed under.
    heading: &'static str,
    /// The table's whole entry: what [`render_table`] writes.
    full: String,
    /// The short form: the table, its row count and, when it has one, its
    /// description.
    line: String,
    /// The lower-cased words a question can find the table by: parts of its
    /// name and its column names, its sample values, its synonyms and the
    /// words of its descriptions.
    ///
    /// Built only from facts the full entry prints. A masked column's
    /// samples, and every sample of a row-filtered table, are not in the
    /// entry, so they are not in this set either: a word in a question must
    /// never bring in a table because of a value the policies keep out of
    /// the map.
    words: HashSet<String>,
}

/// Every table, in the order the map lists them, plus the closing line about
/// catalog datasets with no table. The cached part of the map.
#[derive(Default)]
pub(crate) struct Pieces {
    tables: Vec<TablePiece>,
    /// `"\nCatalog datasets with no Gold table: …\n"`, or `""`.
    unserved: String,
}

/// The cached pieces, with the row-filtered tables they were built without
/// stats for. They are reused only while that set is the current one, so a
/// row-filter policy added after the build is honoured on the next chat and
/// not after [`TTL`].
type CacheEntry = (Instant, HashSet<String>, Arc<Pieces>);

static CACHE: LazyLock<Mutex<Option<CacheEntry>>> = LazyLock::new(|| Mutex::new(None));

/// What the authored policies say the map must not read from the data: the
/// masked columns and the row-filtered tables. Both are read once per chat
/// from the same policies, so they travel together.
#[derive(Debug, Default)]
pub(crate) struct Withheld {
    masked: HashSet<(String, String)>,
    row_filtered: HashSet<String>,
}

impl Withheld {
    /// Read both sets from the policies' `conditions`.
    pub(crate) fn from_conditions(conditions: &[String]) -> Self {
        Self {
            masked: masked_columns(conditions),
            row_filtered: row_filtered_tables(conditions),
        }
    }
}

/// The pieces in `slot`, when they are younger than [`TTL`] and were built
/// without stats for exactly the tables in `row_filtered`.
fn reusable<'a>(
    slot: Option<&'a CacheEntry>,
    row_filtered: &HashSet<String>,
) -> Option<&'a Arc<Pieces>> {
    let (built, built_without, pieces) = slot?;
    (built.elapsed() < TTL && built_without == row_filtered).then_some(pieces)
}

/// The rendered DATA MAP, from cache when it is younger than [`TTL`].
///
/// The cache holds each table's pieces ([`Pieces`]) and never a rendered
/// map: [`assemble`] runs on every call, over the cached pieces or over
/// freshly built ones, with this request's `query_words` (see
/// [`question_words`]) and `relevant_enabled` (`AI_RELEVANT_TABLES`). The
/// question and the caller's words differ per chat, and the cache is one
/// for everybody.
///
/// `withheld` is what [`Withheld::from_conditions`] read from the authored
/// policies, or `None` when the policies could not be read: then no table
/// is listed with stats, since which columns are masked and which tables are
/// row-filtered is unknown. Only a map built with no masked column is
/// cached, so a map carrying samples is never reused after a masking policy
/// appears. The row-filtered tables are the same for every chat, so a map
/// built without their stats is shared like any other.
///
/// The lock is held while a stale map is rebuilt, so concurrent chats wait
/// for one rebuild instead of each running the stats queries. An empty
/// result (`ClickHouse` unreachable) is never cached.
pub(crate) async fn data_map(
    ch: &ChClient,
    withheld: Option<&Withheld>,
    notes: &Notes,
    query_words: &HashSet<String>,
    relevant_enabled: bool,
) -> String {
    let cacheable = withheld.is_some_and(|w| w.masked.is_empty());
    let mut guard = CACHE.lock().await;
    let pieces = if cacheable
        && let Some(w) = withheld
        && let Some(pieces) = reusable(guard.as_ref(), &w.row_filtered)
    {
        Arc::clone(pieces)
    } else {
        let built = Arc::new(collect(ch, withheld, notes, relevant_enabled).await);
        if cacheable
            && !built.tables.is_empty()
            && let Some(w) = withheld
        {
            *guard = Some((Instant::now(), w.row_filtered.clone(), Arc::clone(&built)));
        }
        built
    };
    drop(guard);
    assemble(&pieces, query_words, relevant_enabled, MAX_CHARS)
}

/// Drop the cached map, so the next chat rebuilds it. A person's
/// confirmation calls this to show their text at once and not after
/// [`TTL`].
pub(crate) async fn clear_cache() {
    *CACHE.lock().await = None;
}

/// Held by every test that reads or writes [`CACHE`], so two of them never
/// race on the one static.
#[cfg(test)]
pub(crate) static CACHE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

/// Put a built map into [`CACHE`], as a chat turn would.
#[cfg(test)]
pub(crate) async fn seed_cache_for_test() {
    let pieces = Pieces {
        tables: Vec::new(),
        unserved: "DATA MAP text".to_owned(),
    };
    *CACHE.lock().await = Some((Instant::now(), HashSet::new(), Arc::new(pieces)));
}

/// Put ready pieces into [`CACHE`] as a build with no row-filtered table.
#[cfg(test)]
async fn seed_pieces_for_test(pieces: Pieces) {
    *CACHE.lock().await = Some((Instant::now(), HashSet::new(), Arc::new(pieces)));
}

/// Whether [`CACHE`] holds no map.
#[cfg(test)]
pub(crate) async fn cache_is_empty_for_test() -> bool {
    CACHE.lock().await.is_none()
}

/// `(database.table, column)` pairs a masking policy covers for anyone,
/// read from the authored policies' structured `conditions`. A range or
/// a sample is withheld for these columns so the prompt never carries a
/// value the policy engine would have masked in a query result.
/// Deliberately broader than "masked for this principal": the map is
/// shared across principals.
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

/// The tables (lower-cased `database.table`) that any condition gives a
/// non-blank `rowFilter`. The stats queries read every row of a table, so
/// a row filter that hides rows from a query would not hide them from a
/// range or a sample; PR #79 review, SEC-16. Lower-cased because the policy
/// engine matches a condition's table case-insensitively
/// (`policy_engine.rs`, `eq_ignore_ascii_case`). Like [`masked_columns`],
/// not narrowed to a role: the map is shared across principals.
pub(crate) fn row_filtered_tables(conditions: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for raw in conditions {
        let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        let Some(table) = obj.get("table").and_then(Value::as_str) else {
            continue;
        };
        let filtered = obj
            .get("rowFilter")
            .and_then(Value::as_str)
            .is_some_and(|f| !f.trim().is_empty());
        if filtered {
            out.insert(table.to_ascii_lowercase());
        }
    }
    out
}

/// Longest table text, in characters: the `CHECK` on `semantic_entry` for a
/// table. Applied again here so a row written around the `CHECK` cannot
/// crowd the prompt.
pub(crate) const TABLE_TEXT_CHARS: usize = 400;
/// Longest column description, in characters (the `CHECK` for a column).
pub(crate) const COLUMN_TEXT_CHARS: usize = 200;
/// Most synonyms listed for one table or column.
pub(crate) const MAX_SYNONYMS: usize = 6;
/// Longest single synonym, in characters.
pub(crate) const SYNONYM_CHARS: usize = 40;
/// The roles a column may have (the `CHECK` on `semantic_entry.role`).
pub(crate) const ROLES: [&str; 6] = [
    "measure",
    "dimension",
    "time",
    "key",
    "flag",
    "non_additive",
];

/// What people and the drafting pass wrote about tables and columns, to be
/// rendered beside the facts the DATA MAP reads from `ClickHouse`: the
/// Catalog's annotation descriptions and the semantic layer's entries.
///
/// An empty `Notes` (the default) renders nothing, so the map is the text
/// it was before the layer existed.
#[derive(Default)]
pub(crate) struct Notes {
    /// `serving.<table>` or `silver.<table>` to its annotation description.
    annotations: HashMap<String, String>,
    /// `(asset, column)` to its entry; `column` is `""` for the table.
    entries: HashMap<(String, String), Note>,
}

struct Note {
    description: String,
    synonyms: Vec<String>,
    /// The entry's role, whatever its status: a draft's role counts until a
    /// person changes it.
    role: Option<String>,
    confirmed: bool,
}

/// One line of prompt text: control characters (line breaks above all) turn
/// into spaces so a stored description cannot start a line of the map, then
/// the text is cut to `max` characters, never inside one.
pub(crate) fn one_line(raw: &str, max: usize) -> String {
    let flat: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.trim().chars().take(max).collect()
}

impl Notes {
    /// Index the rows [`lakehouse_store`] returned. A row's table is its
    /// `asset`; an annotation's `asset_id` is the same qualified name for
    /// Silver and Gold tables.
    pub(crate) fn from_rows(annotations: Vec<AnnotationRow>, entries: Vec<SemanticEntry>) -> Self {
        Self {
            annotations: annotations
                .into_iter()
                .filter_map(|a| Some((a.asset_id, a.description?)))
                .collect(),
            entries: entries
                .into_iter()
                .map(|e| {
                    (
                        (e.asset, e.column_name),
                        Note {
                            description: e.description,
                            synonyms: e.synonyms,
                            role: e.role,
                            confirmed: e.status == "confirmed",
                        },
                    )
                })
                .collect(),
        }
    }

    /// Whether no row was indexed.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.annotations.is_empty() && self.entries.is_empty()
    }

    /// The text after a table's line: the annotation's, else a confirmed
    /// entry's, else a draft's. A draft is skipped when the catalog dataset
    /// already described the table (`dataset_described`): only a person's
    /// text is added to a description that exists.
    fn table_text(&self, asset: &str, dataset_described: bool) -> Option<String> {
        let annotation = self.annotations.get(asset).map(String::as_str);
        let entry = self.entries.get(&(asset.to_owned(), String::new()));
        let confirmed = entry
            .filter(|n| n.confirmed)
            .map(|n| n.description.as_str());
        let draft = entry
            .filter(|n| !n.confirmed && !dataset_described)
            .map(|n| n.description.as_str());
        [annotation, confirmed, draft]
            .into_iter()
            .flatten()
            .map(|t| one_line(t, TABLE_TEXT_CHARS))
            .find(|t| !t.is_empty())
    }

    /// A column's description: a confirmed entry, else the source
    /// registry's (`source`, today's text, left uncut), else a draft.
    fn column_text(&self, asset: &str, column: &str, source: Option<&str>) -> Option<String> {
        let entry = self.entries.get(&(asset.to_owned(), column.to_owned()));
        let confirmed = entry
            .filter(|n| n.confirmed)
            .map(|n| one_line(&n.description, COLUMN_TEXT_CHARS))
            .filter(|t| !t.is_empty());
        let source = source.filter(|t| !t.is_empty()).map(str::to_owned);
        let draft = entry
            .filter(|n| !n.confirmed)
            .map(|n| one_line(&n.description, COLUMN_TEXT_CHARS))
            .filter(|t| !t.is_empty());
        confirmed.or(source).or(draft)
    }

    /// A column's role, whatever the entry's status.
    fn role(&self, asset: &str, column: &str) -> Option<&str> {
        self.entries
            .get(&(asset.to_owned(), column.to_owned()))?
            .role
            .as_deref()
    }

    /// The synonyms of a table (`column` is `""`) or a column that
    /// [`Notes::synonyms`] prints, whatever the entry's status.
    fn synonym_names(&self, asset: &str, column: &str) -> Vec<String> {
        let Some(entry) = self.entries.get(&(asset.to_owned(), column.to_owned())) else {
            return Vec::new();
        };
        entry
            .synonyms
            .iter()
            .map(|s| one_line(s, SYNONYM_CHARS))
            .filter(|s| !s.is_empty())
            .take(MAX_SYNONYMS)
            .collect()
    }

    /// ` (also called: a, b)` for a table (`column` is `""`) or a column,
    /// whatever the entry's status, or `None` when it has none.
    fn synonyms(&self, asset: &str, column: &str) -> Option<String> {
        let names = self.synonym_names(asset, column);
        (!names.is_empty()).then(|| format!(" (also called: {})", names.join(", ")))
    }
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

/// The stats query for one table and which of its columns it summarises
/// (`true` for a text column). `None` when the table gets no stats: it is
/// too big, a row-filter policy covers it, the policies are unreadable
/// (`withheld` is `None`, so which tables are row-filtered is unknown), or
/// it has no summarisable column.
///
/// A row-filtered table gets no query at all, range or sample: both read
/// every row, including the ones the filter hides from a query.
fn stats_plan(table: &Table, withheld: Option<&Withheld>) -> Option<(String, Vec<(usize, bool)>)> {
    let withheld = withheld?;
    if table.rows.is_some_and(|n| n > STATS_ROW_CEILING) {
        return None;
    }
    let (Ok(db), Ok(name)) = (Ident::new(table.db.clone()), Ident::new(table.name.clone())) else {
        return None;
    };
    let qualified = format!("{}.{}", table.db, table.name);
    if withheld
        .row_filtered
        .contains(&qualified.to_ascii_lowercase())
    {
        return None;
    }
    let mut exprs = Vec::new();
    let mut plan = Vec::new();
    for (i, col) in table.columns.iter().enumerate() {
        let Ok(ident) = Ident::new(col.name.clone()) else {
            continue;
        };
        if is_range_type(&col.ty) {
            // A masked column's smallest and largest value are as private as
            // its samples: they reach the chat's prompt and the drafting pass.
            // PR #81 review.
            if withheld
                .masked
                .contains(&(qualified.clone(), col.name.clone()))
            {
                continue;
            }
            exprs.push(format!(
                "toString(min(`{ident}`)) AS lo{i}, toString(max(`{ident}`)) AS hi{i}"
            ));
            plan.push((i, false));
        } else if is_text_type(&col.ty) {
            if withheld
                .masked
                .contains(&(qualified.clone(), col.name.clone()))
            {
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
        return None;
    }
    let sql = format!(
        "SELECT {} FROM `{db}`.`{name}` SETTINGS max_execution_time = 5",
        exprs.join(", ")
    );
    Some((sql, plan))
}

/// What the stats query told about one table's columns.
#[derive(Default)]
struct Stats {
    /// One line of facts per column, as [`render_table`] prints it.
    facts: HashMap<String, String>,
    /// The sample values behind the text columns' facts, for the words a
    /// question can find the table by.
    samples: HashMap<String, Vec<String>>,
}

/// One line of facts per column: `min..max` for numbers and dates, the
/// distinct count and up to [`SAMPLE_VALUES`] values for text. Empty for a
/// table [`stats_plan`] gives no query, or that fails to answer.
async fn column_stats(ch: &ChClient, table: &Table, withheld: Option<&Withheld>) -> Stats {
    let mut out = Stats::default();
    let Some((sql, plan)) = stats_plan(table, withheld) else {
        return out;
    };
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
            let fact = if distinct <= u64::try_from(values.len()).unwrap_or(u64::MAX) {
                format!("values: {}", values.join(" | "))
            } else {
                format!("{distinct} distinct, e.g. {}", values.join(" | "))
            };
            out.samples.insert(col.name.clone(), values);
            fact
        } else {
            let (lo, hi) = (text(row, &format!("lo{i}")), text(row, &format!("hi{i}")));
            if lo.is_empty() {
                continue;
            }
            format!("range {lo}..{hi}")
        };
        out.facts.insert(col.name.clone(), fact);
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
/// columns.
///
/// A column's role in `notes` overrides the name and type guess: a numeric
/// column with the role `flag`, `non_additive`, `key`, `dimension` or `time`
/// is not a measure. A numeric, non-key-like `flag` or `non_additive` column
/// (one that would otherwise be a measure) is not part of the grain either,
/// because it describes a row instead of naming it. A text or boolean `flag`
/// or `non_additive` column stays among the grain keys, because rows still
/// differ by it. The `non_additive` columns are listed after the measures,
/// with the warning not to total them. `None` when no column names the
/// grain, or when there is neither a measure nor a `non_additive` column to
/// say anything about.
fn grain_line(table: &Table, notes: &Notes) -> Option<String> {
    let asset = format!("{}.{}", table.db, table.name);
    let role = |c: &Column| notes.role(&asset, &c.name);
    let is_candidate = |c: &Column| is_numeric_type(&c.ty) && !is_key_like(&c.name);
    let (measures, others): (Vec<&Column>, Vec<&Column>) = table.columns.iter().partition(|c| {
        is_candidate(c)
            && !matches!(
                role(c),
                Some("flag" | "non_additive" | "key" | "dimension" | "time")
            )
    });
    let keys: Vec<&str> = others
        .iter()
        .filter(|c| !(is_candidate(c) && matches!(role(c), Some("flag" | "non_additive"))))
        .map(|c| c.name.as_str())
        .collect();
    let non_additive: Vec<&str> = table
        .columns
        .iter()
        .filter(|c| role(c) == Some("non_additive"))
        .map(|c| c.name.as_str())
        .collect();
    if keys.is_empty() || (measures.is_empty() && non_additive.is_empty()) {
        return None;
    }
    let mut line = format!("    grain: one row per {}", keys.join(" x "));
    if !measures.is_empty() {
        let measures: Vec<&str> = measures.iter().map(|c| c.name.as_str()).collect();
        let _ = write!(
            line,
            "; measures: {} (SUM them over rows for any total)",
            measures.join(", ")
        );
    }
    if !non_additive.is_empty() {
        let _ = write!(
            line,
            "; not additive: {} (never SUM these across rows: compute them from a detail table, or say the total cannot be read from this table)",
            non_additive.join(", ")
        );
    }
    line.push('\n');
    Some(line)
}

/// `720 rows`, or `row count unknown`.
fn rows_text(table: &Table) -> String {
    table
        .rows
        .map_or_else(|| "row count unknown".to_owned(), |n| format!("{n} rows"))
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
    notes: &Notes,
) {
    let rows = rows_text(table);
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
    let asset = format!("{}.{}", table.db, table.name);
    let dataset_described = dataset.is_some_and(|d| !d.description.is_empty());
    if let Some(text) = notes.table_text(&asset, dataset_described) {
        let _ = write!(out, ": {text}");
    }
    if let Some(synonyms) = notes.synonyms(&asset, "") {
        out.push_str(&synonyms);
    }
    out.push('\n');
    if let Some(grain) = grain_line(table, notes) {
        out.push_str(&grain);
    }
    for col in &table.columns {
        let _ = write!(out, "    {} {}", col.name, col.ty);
        let source = dataset
            .and_then(|d| descriptions.get(&(d.slug.clone(), col.name.clone())))
            .map(String::as_str);
        if let Some(desc) = notes.column_text(&asset, &col.name, source) {
            let _ = write!(out, " — {desc}");
        }
        if let Some(synonyms) = notes.synonyms(&asset, &col.name) {
            out.push_str(&synonyms);
        }
        match notes.role(&asset, &col.name) {
            Some("non_additive") => out.push_str(" [never SUM across rows]"),
            Some("flag") => out.push_str(" [0/1 flag]"),
            _ => {}
        }
        if let Some(fact) = stats.get(&col.name) {
            let _ = write!(out, "; {fact}");
        }
        out.push('\n');
    }
}

/// The live tables and the catalog facts the DATA MAP renders from, read
/// from `ClickHouse` once. [`build`] renders all of them; the semantic
/// layer's drafting pass asks for one table's facts through [`Live::facts`],
/// which runs the same stats query and the same [`render_table`], so what
/// the model reads about a table is what the chat reads about it, masked
/// columns included.
pub(crate) struct Live {
    tables: Vec<Table>,
    datasets: HashMap<String, Dataset>,
    descriptions: HashMap<(String, String), String>,
}

/// One live table, as the drafting pass needs to know it.
pub(crate) struct LiveTable {
    /// `serving.<table>` or `silver.<table>`.
    pub(crate) asset: String,
    /// Whether the table is in `serving` (Gold), which the pass drafts first.
    pub(crate) serving: bool,
    /// The table's column names, in table order.
    pub(crate) columns: Vec<String>,
}

impl Live {
    /// Read the tables and, when there are any, the catalog. Empty when
    /// `ClickHouse` did not answer.
    pub(crate) async fn load(ch: &ChClient) -> Self {
        let tables = load_tables(ch).await;
        let (datasets, descriptions) = if tables.is_empty() {
            (HashMap::new(), HashMap::new())
        } else {
            load_catalog(ch).await
        };
        Self {
            tables,
            datasets,
            descriptions,
        }
    }

    /// The catalog dataset a Gold table is served from, if any.
    fn dataset(&self, table: &Table) -> Option<&Dataset> {
        if table.db == "serving" {
            self.datasets.get(&table.name)
        } else {
            None
        }
    }

    /// Every live table, in the order the map lists them.
    pub(crate) fn tables(&self) -> Vec<LiveTable> {
        self.tables
            .iter()
            .map(|t| LiveTable {
                asset: format!("{}.{}", t.db, t.name),
                serving: t.db == "serving",
                columns: t.columns.iter().map(|c| c.name.clone()).collect(),
            })
            .collect()
    }

    /// The text the map prints for one table, without any entry or
    /// annotation: name, row count, source description, every column with
    /// its type and stats. `withheld` is read as [`data_map`] reads it, so a
    /// masked column and a row-filtered table, or any table while `withheld`
    /// is `None`, carry no stats at all. `None` when `asset` is not a live
    /// table.
    pub(crate) async fn facts(
        &self,
        ch: &ChClient,
        asset: &str,
        withheld: Option<&Withheld>,
    ) -> Option<String> {
        let table = self
            .tables
            .iter()
            .find(|t| format!("{}.{}", t.db, t.name) == asset)?;
        let stats = column_stats(ch, table, withheld).await;
        let mut out = String::new();
        render_table(
            &mut out,
            table,
            self.dataset(table),
            &self.descriptions,
            &stats.facts,
            &Notes::default(),
        );
        Some(out)
    }
}

/// Read every table's facts from `ClickHouse`: the expensive part of the
/// map, and the part the cache keeps.
///
/// With `relevant_enabled` every table gets its stats, as every table may be
/// the one a question names and a named table past the budget is written in
/// full; [`assemble`] decides how much of each is written. With it off the
/// map is the one that was before tables were matched to a question
/// ([`in_order`]), which never writes a table past the budget, so only the
/// tables inside the budget are queried: the cost this build had then.
///
/// PR review fix (SHOULD-FIX): the stats of every table were read whatever
/// the switch said. The switch is static config, so it does not change
/// during the life of the cache and the pieces built under one value are
/// never read under the other.
async fn collect(
    ch: &ChClient,
    withheld: Option<&Withheld>,
    notes: &Notes,
    relevant_enabled: bool,
) -> Pieces {
    let live = Live::load(ch).await;
    collect_with(
        &live,
        withheld,
        notes,
        relevant_enabled,
        MAX_CHARS,
        |table| Box::pin(column_stats(ch, table, withheld)),
    )
    .await
}

/// What reads one table's stats. Boxed because a closure that returns a
/// future borrowing its argument cannot be written for a generic `Fut`, and
/// an `async` closure here makes the chat handler's future not `Send`.
/// `'a` is the borrow of the loaded tables, which the stats reader's own
/// borrows (the `ClickHouse` client, the policies) must outlive.
type StatsFuture<'a> = std::pin::Pin<Box<dyn Future<Output = Stats> + Send + 'a>>;

/// [`collect`] over tables already loaded, with `stats_of` as the one call
/// that reads a table's stats. `budget` is the length [`in_order`] stops at.
async fn collect_with<'a>(
    live: &'a Live,
    withheld: Option<&Withheld>,
    notes: &Notes,
    relevant_enabled: bool,
    budget: usize,
    mut stats_of: impl FnMut(&'a Table) -> StatsFuture<'a>,
) -> Pieces {
    let mut stats = Vec::with_capacity(live.tables.len());
    let mut past_budget = false;
    for table in &live.tables {
        if !relevant_enabled && !past_budget {
            // The same walk `assemble` makes over the tables done so far
            // (`pieces_from` pairs only as many tables as there are stats),
            // so the tables skipped here are the ones it skips.
            let done = pieces_from(live, &stats, notes, withheld);
            past_budget = in_order(&done, budget).0.len() > budget;
        }
        stats.push(if past_budget {
            Stats::default()
        } else {
            stats_of(table).await
        });
    }
    pieces_from(live, &stats, notes, withheld)
}

/// The group heading of a table.
fn heading_of(table: &Table) -> &'static str {
    if table.db == "serving" {
        GOLD_HEADING
    } else {
        SILVER_HEADING
    }
}

/// Add the words of `text` to `into`.
fn add_words(into: &mut HashSet<String>, text: &str) {
    into.extend(tokens(text));
}

/// The words a question can find `table` by, from the facts its entry
/// prints and no others (see [`TablePiece::words`]).
///
/// `stats` holds no sample of a masked column or of a row-filtered table to
/// begin with ([`stats_plan`]). The check on `withheld` below repeats that
/// rule at the point where the words are made, so a change to the stats path
/// cannot put a withheld value into a word that every user's question can
/// match.
fn table_words(
    table: &Table,
    dataset: Option<&Dataset>,
    descriptions: &HashMap<(String, String), String>,
    stats: &Stats,
    notes: &Notes,
    withheld: Option<&Withheld>,
) -> HashSet<String> {
    let asset = format!("{}.{}", table.db, table.name);
    let mut words = HashSet::new();
    add_words(&mut words, &table.name);
    let dataset_described = dataset.is_some_and(|d| !d.description.is_empty());
    if let Some(d) = dataset {
        add_words(&mut words, &d.description);
    }
    if let Some(text) = notes.table_text(&asset, dataset_described) {
        add_words(&mut words, &text);
    }
    for synonym in notes.synonym_names(&asset, "") {
        add_words(&mut words, &synonym);
    }
    let samples_allowed =
        withheld.is_some_and(|w| !w.row_filtered.contains(&asset.to_ascii_lowercase()));
    for col in &table.columns {
        add_words(&mut words, &col.name);
        let source = dataset
            .and_then(|d| descriptions.get(&(d.slug.clone(), col.name.clone())))
            .map(String::as_str);
        if let Some(text) = notes.column_text(&asset, &col.name, source) {
            add_words(&mut words, &text);
        }
        for synonym in notes.synonym_names(&asset, &col.name) {
            add_words(&mut words, &synonym);
        }
        let masked =
            withheld.is_some_and(|w| w.masked.contains(&(asset.clone(), col.name.clone())));
        if samples_allowed
            && !masked
            && let Some(values) = stats.samples.get(&col.name)
        {
            for value in values {
                add_words(&mut words, value);
            }
        }
    }
    words
}

/// A table's short form: `- db.table (N rows)` and, when the table has a
/// description, `: <description>` cut to [`LINE_TEXT_CHARS`].
fn table_line(table: &Table, dataset: Option<&Dataset>, notes: &Notes) -> String {
    let mut line = format!("- {}.{} ({})", table.db, table.name, rows_text(table));
    let asset = format!("{}.{}", table.db, table.name);
    let dataset_description = dataset
        .map(|d| one_line(&d.description, LINE_TEXT_CHARS))
        .filter(|t| !t.is_empty());
    let dataset_described = dataset.is_some_and(|d| !d.description.is_empty());
    let described = dataset_description.or_else(|| {
        notes
            .table_text(&asset, dataset_described)
            .map(|t| one_line(&t, LINE_TEXT_CHARS))
    });
    if let Some(text) = described {
        let _ = write!(line, ": {text}");
    }
    line
}

/// Turn the live tables and their stats into pieces, with no `ClickHouse`
/// call: the same inputs always give the same pieces.
fn pieces_from(live: &Live, stats: &[Stats], notes: &Notes, withheld: Option<&Withheld>) -> Pieces {
    let mut tables = Vec::with_capacity(live.tables.len());
    for (table, stats) in live.tables.iter().zip(stats) {
        let dataset = live.dataset(table);
        let mut full = String::new();
        render_table(
            &mut full,
            table,
            dataset,
            &live.descriptions,
            &stats.facts,
            notes,
        );
        tables.push(TablePiece {
            qualified: format!("{}.{}", table.db, table.name),
            heading: heading_of(table),
            full,
            line: table_line(table, dataset, notes),
            words: table_words(table, dataset, &live.descriptions, stats, notes, withheld),
        });
    }
    let unserved: Vec<String> = live
        .datasets
        .iter()
        .filter(|(table, _)| {
            !live
                .tables
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
    Pieces {
        tables,
        unserved: if unserved.is_empty() {
            String::new()
        } else {
            format!(
                "\nCatalog datasets with no Gold table: {}.\n",
                unserved.join(", ")
            )
        },
    }
}

/// The words of a question: the user's last [`QUESTION_MESSAGES`] messages
/// (`user_messages` holds every user message, oldest first), lower-cased, without
/// words shorter than [`MIN_QUESTION_WORD_CHARS`] and without the function
/// words of [`super::prompt::reply_language`]'s two lists. A stored term of
/// the caller's `terms` that the question uses (see [`term_is_used`]) also
/// brings the words of its meaning, with the same filters.
///
/// The terms are the caller's own, read for this request. This runs per
/// request and its result is never stored in the shared cache.
pub(crate) fn question_words(user_messages: &[&str], terms: &[ChatTerm]) -> HashSet<String> {
    let keep = |word: &String| {
        word.chars().count() >= MIN_QUESTION_WORD_CHARS
            && !INDONESIAN_WORDS.contains(&word.as_str())
            && !ENGLISH_WORDS.contains(&word.as_str())
    };
    let recent: Vec<&str> = user_messages
        .iter()
        .rev()
        .take(QUESTION_MESSAGES)
        .copied()
        .collect();
    let mut words: HashSet<String> = recent
        .iter()
        .flat_map(|message| tokens(message))
        .filter(keep)
        .collect();
    let from_terms: Vec<String> = terms
        .iter()
        .filter(|t| term_is_used(&t.term, &recent, &words, keep))
        .flat_map(|t| tokens(&t.meaning))
        .filter(keep)
        .collect();
    words.extend(from_terms);
    words
}

/// Whether the question uses the stored `term`, by either rule.
///
/// 1. Every word of the term (same tokeniser and filters as the question's
///    words) is among `question`, in any order. A term left with no word
///    after the filters never matches by this rule, so a term of only
///    function words cannot bring its meaning into every question.
/// 2. The lower-cased term stands as a phrase in one of the `messages`,
///    with no letter, digit or `_` next to it. This is for the terms the
///    tokeniser cannot hold whole: `q3` or `vip_user`, whose digits and `_`
///    it splits or drops.
///
/// PR review fix (SHOULD-FIX): a term was compared to one question word, so
/// a term of several words, or with `_` or a digit, never matched.
fn term_is_used(
    term: &str,
    messages: &[&str],
    question: &HashSet<String>,
    keep: impl Fn(&String) -> bool,
) -> bool {
    let term_words: Vec<String> = tokens(term).filter(|w| keep(w)).collect();
    if !term_words.is_empty() && term_words.iter().all(|w| question.contains(w)) {
        return true;
    }
    let term = term.trim().to_lowercase();
    let part_of_word = |c: char| c.is_alphanumeric() || c == '_';
    !term.is_empty()
        && messages.iter().any(|message| {
            let message = message.to_lowercase();
            message.match_indices(&term).any(|(start, found)| {
                let before = message[..start].chars().next_back();
                let after = message[start + found.len()..].chars().next();
                !before.is_some_and(part_of_word) && !after.is_some_and(part_of_word)
            })
        })
}

/// Whether a question word finds `piece`: it equals one of the table's
/// words, or it is at least [`MIN_PREFIX_CHARS`] long and begins one.
fn is_named(piece: &TablePiece, query: &HashSet<String>) -> bool {
    query.iter().any(|word| {
        piece.words.contains(word)
            || (word.chars().count() >= MIN_PREFIX_CHARS
                && piece.words.iter().any(|w| w.starts_with(word.as_str())))
    })
}

/// The map's text from the cached pieces and this request's question.
///
/// With `relevant_enabled` off, or when no table is named by `query_words`,
/// every table is written in full, in order: the map this was before tables
/// were matched to the question. Otherwise, under each heading, the tables
/// the question names come first in full and the others follow on one line
/// each, then one closing sentence says so.
///
/// `budget` is the length after which no more tables are started (the map
/// lists the rest in one line): [`MAX_CHARS`] in a chat.
fn assemble(
    pieces: &Pieces,
    query_words: &HashSet<String>,
    relevant_enabled: bool,
    budget: usize,
) -> String {
    if pieces.tables.is_empty() {
        return String::new();
    }
    let named: Vec<bool> = pieces
        .tables
        .iter()
        .map(|t| relevant_enabled && is_named(t, query_words))
        .collect();
    let (mut out, skipped) = if named.contains(&true) {
        named_first(pieces, &named, budget)
    } else {
        in_order(pieces, budget)
    };
    if !skipped.is_empty() {
        let _ = write!(
            out,
            "\n{} more tables not described here (budget): {}. {SEE_COLUMNS}\n",
            skipped.len(),
            skipped
                .iter()
                .take(60)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    out.push_str(&pieces.unserved);
    out
}

/// Every table in full, in order, until the map is past `budget`; the tables
/// left are returned by name.
fn in_order(pieces: &Pieces, budget: usize) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut section = "";
    let mut skipped: Vec<String> = Vec::new();
    for table in &pieces.tables {
        if out.len() > budget {
            skipped.push(table.qualified.clone());
            continue;
        }
        if table.heading != section {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(table.heading);
            out.push('\n');
            section = table.heading;
        }
        out.push_str(&table.full);
    }
    (out, skipped)
}

/// The tables `named` marks in full and the others on one line each, under
/// their headings. The named tables are placed first, so when `budget` runs
/// out it is the one-line tables that are left out.
fn named_first(pieces: &Pieces, named: &[bool], budget: usize) -> (String, Vec<String>) {
    struct Group {
        heading: &'static str,
        full: String,
        lines: String,
    }
    let mut groups: Vec<Group> = Vec::new();
    for table in &pieces.tables {
        if !groups.iter().any(|g| g.heading == table.heading) {
            groups.push(Group {
                heading: table.heading,
                full: String::new(),
                lines: String::new(),
            });
        }
    }
    let mut used = 0;
    let mut skipped: Vec<String> = Vec::new();
    let mut any_line = false;
    for want_named in [true, false] {
        for (table, table_named) in pieces.tables.iter().zip(named) {
            if *table_named != want_named {
                continue;
            }
            if used > budget {
                skipped.push(table.qualified.clone());
                continue;
            }
            let Some(group) = groups.iter_mut().find(|g| g.heading == table.heading) else {
                continue;
            };
            if group.full.is_empty() && group.lines.is_empty() {
                used += group.heading.len() + 2;
            }
            if want_named {
                group.full.push_str(&table.full);
                used += table.full.len();
            } else {
                group.lines.push_str(&table.line);
                group.lines.push('\n');
                used += table.line.len() + 1;
                any_line = true;
            }
        }
    }
    let mut out = String::new();
    for group in groups
        .iter()
        .filter(|g| !g.full.is_empty() || !g.lines.is_empty())
    {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(group.heading);
        out.push('\n');
        out.push_str(&group.full);
        out.push_str(&group.lines);
    }
    if any_line {
        let _ = write!(
            out,
            "\nA table shown on one line is described in full when a question names it. {SEE_COLUMNS}\n"
        );
    }
    (out, skipped)
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
    fn row_filtered_tables_lists_a_table_whose_only_obligation_is_a_row_filter() {
        let tables = row_filtered_tables(&[
            r#"{"roles":["Analyst"],"table":"serving.mart_y","rowFilter":"region = 'x'"}"#
                .to_owned(),
        ]);
        assert_eq!(tables, HashSet::from(["serving.mart_y".to_owned()]));
    }

    #[test]
    fn row_filtered_tables_skips_a_blank_or_missing_filter_and_prose() {
        let tables = row_filtered_tables(&[
            r#"{"roles":["Analyst"],"table":"serving.mask_only","mask":["email"]}"#.to_owned(),
            r#"{"roles":["Analyst"],"table":"serving.empty","rowFilter":""}"#.to_owned(),
            r#"{"roles":["Analyst"],"table":"serving.blank","rowFilter":"  \n "}"#.to_owned(),
            r#"{"roles":["Analyst"],"table":"serving.null","rowFilter":null}"#.to_owned(),
            "Analysts may not see rows of another region".to_owned(),
        ]);
        assert!(tables.is_empty(), "{tables:?}");
    }

    #[test]
    fn row_filtered_tables_lists_a_table_that_has_both_a_mask_and_a_filter_in_lower_case() {
        let tables = row_filtered_tables(&[
            r#"{"roles":["Analyst"],"table":"Serving.Mart_Z","mask":["email"],"rowFilter":"a = 1"}"#
                .to_owned(),
        ]);
        assert_eq!(tables, HashSet::from(["serving.mart_z".to_owned()]));
    }

    fn visits_table() -> Table {
        let col = |name: &str, ty: &str| Column {
            name: name.to_owned(),
            ty: ty.to_owned(),
        };
        Table {
            db: "serving".to_owned(),
            name: "mart_visits".to_owned(),
            rows: Some(720),
            columns: vec![col("tahun", "UInt16"), col("negara", "String")],
        }
    }

    fn withheld_for(conditions: &[&str]) -> Withheld {
        let owned: Vec<String> = conditions.iter().map(|c| (*c).to_owned()).collect();
        Withheld::from_conditions(&owned)
    }

    #[test]
    fn a_table_with_no_policy_gets_a_stats_query_over_its_columns() {
        let withheld = withheld_for(&[]);
        let Some((sql, plan)) = stats_plan(&visits_table(), Some(&withheld)) else {
            panic!("a table with no row filter must get a stats query");
        };
        assert!(sql.contains("FROM `serving`.`mart_visits`"), "{sql}");
        assert_eq!(plan, vec![(0, false), (1, true)]);
    }

    #[test]
    fn a_row_filtered_table_gets_no_stats_query_at_all() {
        let withheld = withheld_for(&[
            r#"{"roles":["Analyst"],"table":"serving.mart_visits","rowFilter":"negara = 'x'"}"#,
        ]);
        assert!(stats_plan(&visits_table(), Some(&withheld)).is_none());
    }

    #[test]
    fn unreadable_policies_get_no_stats_query_for_any_table() {
        assert!(stats_plan(&visits_table(), None).is_none());
    }

    #[test]
    fn a_masked_text_column_stays_out_of_the_stats_query_and_the_other_columns_stay_in() {
        let withheld = withheld_for(&[
            r#"{"roles":["Analyst"],"table":"serving.mart_visits","mask":["negara"]}"#,
        ]);
        let Some((sql, plan)) = stats_plan(&visits_table(), Some(&withheld)) else {
            panic!("a table with no row filter must get a stats query");
        };
        assert_eq!(plan, vec![(0, false)]);
        assert!(!sql.contains("negara"), "{sql}");
    }

    /// `mart_visits` with a numeric year, a date and a text column: the three
    /// kinds of column the stats query summarises.
    fn visits_with_date_table() -> Table {
        Table {
            db: "serving".to_owned(),
            name: "mart_visits".to_owned(),
            rows: Some(720),
            columns: vec![
                col("tahun", "UInt16"),
                col("tanggal", "Date"),
                col("negara", "String"),
            ],
        }
    }

    #[test]
    fn a_masked_numeric_column_stays_out_of_the_range_query_and_the_other_columns_stay_in() {
        let withheld = withheld_for(&[
            r#"{"roles":["Analyst"],"table":"serving.mart_visits","mask":["tahun"]}"#,
        ]);
        let Some((sql, plan)) = stats_plan(&visits_with_date_table(), Some(&withheld)) else {
            panic!("a table with a column left to summarise must get a stats query");
        };
        assert!(!sql.contains("min(`tahun`)"), "{sql}");
        assert!(!sql.contains("max(`tahun`)"), "{sql}");
        assert!(sql.contains("min(`tanggal`)"), "{sql}");
        assert!(sql.contains("uniq(`negara`)"), "{sql}");
        assert_eq!(plan, vec![(1, false), (2, true)]);
    }

    #[test]
    fn a_masked_date_column_stays_out_of_the_range_query_and_the_other_columns_stay_in() {
        let withheld = withheld_for(&[
            r#"{"roles":["Analyst"],"table":"serving.mart_visits","mask":["tanggal"]}"#,
        ]);
        let Some((sql, plan)) = stats_plan(&visits_with_date_table(), Some(&withheld)) else {
            panic!("a table with a column left to summarise must get a stats query");
        };
        assert!(!sql.contains("min(`tanggal`)"), "{sql}");
        assert!(!sql.contains("max(`tanggal`)"), "{sql}");
        assert!(sql.contains("min(`tahun`)"), "{sql}");
        assert!(sql.contains("uniq(`negara`)"), "{sql}");
        assert_eq!(plan, vec![(0, false), (2, true)]);
    }

    #[test]
    fn a_table_with_no_masked_column_gets_the_same_stats_query_as_before() {
        let withheld = withheld_for(&[]);
        let Some((sql, _)) = stats_plan(&visits_with_date_table(), Some(&withheld)) else {
            panic!("a table with no row filter must get a stats query");
        };
        assert_eq!(
            sql,
            "SELECT toString(min(`tahun`)) AS lo0, toString(max(`tahun`)) AS hi0, toString(min(`tanggal`)) AS lo1, toString(max(`tanggal`)) AS hi1, toString(uniq(`negara`)) AS n2, arrayStringConcat(arraySlice(arraySort(groupUniqArray(200)(toString(`negara`))), 1, 12), '\\u001f') AS v2 FROM `serving`.`mart_visits` SETTINGS max_execution_time = 5"
        );
    }

    #[test]
    fn a_table_whose_only_summarised_column_is_masked_gets_no_stats_query() {
        let withheld = withheld_for(&[
            r#"{"roles":["Analyst"],"table":"serving.mart_visits","mask":["tahun"]}"#,
        ]);
        let only_year = Table {
            columns: vec![col("tahun", "UInt16")],
            ..visits_with_date_table()
        };
        assert!(stats_plan(&only_year, Some(&withheld)).is_none());
    }

    #[test]
    fn a_column_with_no_stats_fact_is_written_with_its_name_and_type_and_no_range() {
        let table = Table {
            columns: vec![col("tahun", "UInt16")],
            ..visits_with_date_table()
        };
        let mut out = String::new();
        render_table(
            &mut out,
            &table,
            None,
            &HashMap::new(),
            &HashMap::new(),
            &Notes::default(),
        );
        assert!(out.contains("\n    tahun UInt16\n"), "{out}");
        assert!(!out.contains("range "), "{out}");
    }

    fn cached_without(without: &[&str], age: Duration) -> CacheEntry {
        let Some(built) = Instant::now().checked_sub(age) else {
            panic!("the clock must be older than the test's cache age");
        };
        let pieces = Pieces {
            tables: Vec::new(),
            unserved: "map".to_owned(),
        };
        (
            built,
            without.iter().map(|t| (*t).to_owned()).collect(),
            Arc::new(pieces),
        )
    }

    fn reused_marker(slot: Option<&CacheEntry>, row_filtered: &HashSet<String>) -> Option<String> {
        reusable(slot, row_filtered).map(|p| p.unserved.clone())
    }

    #[test]
    fn a_cached_map_is_reused_only_for_the_same_row_filtered_tables_and_within_the_ttl() {
        let fresh = cached_without(&["serving.a"], Duration::ZERO);
        let same = HashSet::from(["serving.a".to_owned()]);
        assert_eq!(reused_marker(Some(&fresh), &same).as_deref(), Some("map"));
        assert_eq!(
            reused_marker(Some(&fresh), &HashSet::new()),
            None,
            "a policy dropped since the build: rebuild"
        );
        let built_before_the_policy = cached_without(&[], Duration::ZERO);
        assert_eq!(
            reused_marker(Some(&built_before_the_policy), &same),
            None,
            "a row filter added since the build must not be served the old map"
        );
        let stale = cached_without(&["serving.a"], TTL + Duration::from_secs(1));
        assert_eq!(reused_marker(Some(&stale), &same), None);
        assert_eq!(reused_marker(None, &same), None);
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
            grain_line(&table, &Notes::default()).as_deref(),
            Some(
                "    grain: one row per tahun x bulan_no x negara; measures: jumlah (SUM them over rows for any total)\n"
            )
        );
        let only_labels = Table {
            columns: vec![col("indikator", "String")],
            ..table
        };
        assert_eq!(grain_line(&only_labels, &Notes::default()), None);
    }

    fn col(name: &str, ty: &str) -> Column {
        Column {
            name: name.to_owned(),
            ty: ty.to_owned(),
        }
    }

    /// A Gold table that a catalog dataset serves, with a source description
    /// on `tahun` only.
    fn served_table() -> Table {
        Table {
            db: "serving".to_owned(),
            name: "visits".to_owned(),
            rows: Some(720),
            columns: vec![
                col("tahun", "UInt16"),
                col("negara", "String"),
                col("jumlah", "UInt32"),
            ],
        }
    }

    fn served_dataset() -> Dataset {
        Dataset {
            slug: "visits-by-country".to_owned(),
            title: "Visits by country".to_owned(),
            description: "Monthly visits.".to_owned(),
            source_kind: "primary source",
            publisher: "Agency".to_owned(),
            frequency: String::new(),
            unit: "visits".to_owned(),
        }
    }

    fn served_descriptions() -> HashMap<(String, String), String> {
        HashMap::from([(
            ("visits-by-country".to_owned(), "tahun".to_owned()),
            "Calendar year".to_owned(),
        )])
    }

    fn served_stats() -> HashMap<String, String> {
        HashMap::from([
            ("tahun".to_owned(), "range 2019..2024".to_owned()),
            ("negara".to_owned(), "values: Japan | Korea".to_owned()),
        ])
    }

    /// A Silver table: no dataset, so no description and no source text.
    fn silver_table() -> Table {
        Table {
            db: "silver".to_owned(),
            name: "raw_visits".to_owned(),
            rows: None,
            columns: vec![col("origin", "String"), col("count", "UInt32")],
        }
    }

    fn render_served(notes: &Notes) -> String {
        let mut out = String::new();
        render_table(
            &mut out,
            &served_table(),
            Some(&served_dataset()),
            &served_descriptions(),
            &served_stats(),
            notes,
        );
        out
    }

    fn render_silver(notes: &Notes) -> String {
        let mut out = String::new();
        render_table(
            &mut out,
            &silver_table(),
            None,
            &HashMap::new(),
            &HashMap::new(),
            notes,
        );
        out
    }

    // The two strings below are what the renderer wrote before the semantic
    // layer existed, copied before `render_table` was changed.
    const SERVED_TODAY: &str = "\
- serving.visits (720 rows) — dataset \"Visits by country\" (slug visits-by-country, primary source, publisher: Agency, unit: visits): Monthly visits.
    grain: one row per tahun x negara; measures: jumlah (SUM them over rows for any total)
    tahun UInt16 — Calendar year; range 2019..2024
    negara String; values: Japan | Korea
    jumlah UInt32
";
    const SILVER_TODAY: &str = "\
- silver.raw_visits (row count unknown)
    grain: one row per origin; measures: count (SUM them over rows for any total)
    origin String
    count UInt32
";

    #[test]
    fn with_no_notes_a_table_with_a_dataset_renders_as_it_did_before() {
        assert_eq!(render_served(&Notes::default()), SERVED_TODAY);
    }

    #[test]
    fn with_no_notes_a_table_without_a_dataset_renders_as_it_did_before() {
        assert_eq!(render_silver(&Notes::default()), SILVER_TODAY);
    }

    fn entry(asset: &str, column: &str, description: &str, confirmed: bool) -> SemanticEntry {
        SemanticEntry {
            asset: asset.to_owned(),
            column_name: column.to_owned(),
            description: description.to_owned(),
            synonyms: Vec::new(),
            role: None,
            status: if confirmed { "confirmed" } else { "draft" }.to_owned(),
            written_by: None,
            model: None,
            updated_at: "2026-10-07T00:00:00.000Z".to_owned(),
        }
    }

    fn with_synonyms(mut e: SemanticEntry, synonyms: &[&str]) -> SemanticEntry {
        e.synonyms = synonyms.iter().map(|s| (*s).to_owned()).collect();
        e
    }

    fn annotation(asset: &str, description: &str) -> AnnotationRow {
        AnnotationRow {
            asset_id: asset.to_owned(),
            owner: None,
            steward: None,
            tags: Vec::new(),
            description: Some(description.to_owned()),
        }
    }

    fn notes(annotations: Vec<AnnotationRow>, entries: Vec<SemanticEntry>) -> Notes {
        Notes::from_rows(annotations, entries)
    }

    fn with_role(mut e: SemanticEntry, role: &str) -> SemanticEntry {
        e.role = Some(role.to_owned());
        e
    }

    /// `serving.sales` with the given `(name, type)` columns.
    fn sales_with(columns: &[(&str, &str)]) -> Table {
        Table {
            db: "serving".to_owned(),
            name: "sales".to_owned(),
            rows: Some(48),
            columns: columns.iter().map(|(n, t)| col(n, t)).collect(),
        }
    }

    /// A role-only entry for a column of `serving.sales`.
    fn role_of(column: &str, role: &str, confirmed: bool) -> SemanticEntry {
        with_role(entry("serving.sales", column, "", confirmed), role)
    }

    const NOT_ADDITIVE_ADVICE: &str = "(never SUM these across rows: compute them from a detail table, or say the total cannot be read from this table)";

    fn render_sales(table: &Table, notes: &Notes) -> String {
        let mut out = String::new();
        render_table(
            &mut out,
            table,
            None,
            &HashMap::new(),
            &HashMap::new(),
            notes,
        );
        out
    }

    #[test]
    fn with_no_roles_every_numeric_non_key_column_is_a_measure_and_no_line_is_tagged() {
        let table = sales_with(&[
            ("year", "UInt16"),
            ("region", "String"),
            ("orders", "UInt32"),
            ("net_amt", "Float64"),
            ("active", "UInt8"),
        ]);
        let notes = notes(vec![], vec![]);
        assert_eq!(
            grain_line(&table, &notes).as_deref(),
            Some(
                "    grain: one row per year x region; measures: orders, net_amt, active (SUM them over rows for any total)\n"
            )
        );
        let rendered = render_sales(&table, &notes);
        assert!(!rendered.contains('['), "{rendered}");
    }

    #[test]
    fn a_non_additive_column_leaves_the_measures_and_is_named_with_the_warning() {
        let table = sales_with(&[
            ("year", "UInt16"),
            ("region", "String"),
            ("orders", "UInt32"),
            ("net_amt", "Float64"),
        ]);
        let notes = notes(
            vec![],
            vec![
                role_of("orders", "non_additive", true),
                role_of("net_amt", "measure", true),
            ],
        );
        assert_eq!(
            grain_line(&table, &notes),
            Some(format!(
                "    grain: one row per year x region; measures: net_amt (SUM them over rows for any total); not additive: orders {NOT_ADDITIVE_ADVICE}\n"
            ))
        );
        let rendered = render_sales(&table, &notes);
        assert!(
            rendered.contains("    orders UInt32 [never SUM across rows]\n"),
            "{rendered}"
        );
        assert!(rendered.contains("    net_amt Float64\n"), "{rendered}");
    }

    #[test]
    fn a_flag_column_leaves_the_measures_and_its_line_says_it_is_a_flag() {
        let table = sales_with(&[
            ("year", "UInt16"),
            ("region", "String"),
            ("net_amt", "Float64"),
            ("active", "UInt8"),
        ]);
        let notes = notes(vec![], vec![role_of("active", "flag", true)]);
        assert_eq!(
            grain_line(&table, &notes).as_deref(),
            Some(
                "    grain: one row per year x region; measures: net_amt (SUM them over rows for any total)\n"
            )
        );
        let rendered = render_sales(&table, &notes);
        assert!(
            rendered.contains("    active UInt8 [0/1 flag]\n"),
            "{rendered}"
        );
    }

    #[test]
    fn with_every_numeric_column_non_additive_the_line_does_not_tell_the_model_to_sum() {
        let table = sales_with(&[
            ("year", "UInt16"),
            ("region", "String"),
            ("orders", "UInt32"),
            ("visitors", "UInt32"),
        ]);
        let notes = notes(
            vec![],
            vec![
                role_of("orders", "non_additive", true),
                role_of("visitors", "non_additive", true),
            ],
        );
        let line = grain_line(&table, &notes).unwrap_or_default();
        assert!(!line.contains("SUM them"), "{line}");
        assert!(!line.contains("measures:"), "{line}");
        assert_eq!(
            line,
            format!(
                "    grain: one row per year x region; not additive: orders, visitors {NOT_ADDITIVE_ADVICE}\n"
            )
        );
    }

    #[test]
    fn a_draft_role_counts_the_same_as_a_confirmed_one() {
        let table = sales_with(&[
            ("year", "UInt16"),
            ("region", "String"),
            ("orders", "UInt32"),
            ("net_amt", "Float64"),
            ("active", "UInt8"),
        ]);
        let notes = notes(
            vec![],
            vec![
                role_of("orders", "non_additive", false),
                role_of("active", "flag", true),
            ],
        );
        assert_eq!(
            grain_line(&table, &notes),
            Some(format!(
                "    grain: one row per year x region; measures: net_amt (SUM them over rows for any total); not additive: orders {NOT_ADDITIVE_ADVICE}\n"
            ))
        );
        let rendered = render_sales(&table, &notes);
        assert!(
            rendered.contains("    orders UInt32 [never SUM across rows]\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("    active UInt8 [0/1 flag]\n"),
            "{rendered}"
        );
    }

    #[test]
    fn a_dimension_time_or_key_role_takes_a_numeric_column_out_of_the_measures() {
        for role in ["dimension", "time", "key"] {
            let table = sales_with(&[
                ("region", "String"),
                ("wave", "UInt8"),
                ("net_amt", "Float64"),
            ]);
            let notes = notes(vec![], vec![role_of("wave", role, true)]);
            assert_eq!(
                grain_line(&table, &notes).as_deref(),
                Some(
                    "    grain: one row per region x wave; measures: net_amt (SUM them over rows for any total)\n"
                ),
                "role {role}"
            );
        }
    }

    #[test]
    fn a_text_flag_column_stays_in_the_grain_and_its_line_says_it_is_a_flag() {
        let table = sales_with(&[
            ("year", "UInt16"),
            ("region", "String"),
            ("active", "String"),
            ("net_amt", "Float64"),
        ]);
        let notes = notes(vec![], vec![role_of("active", "flag", true)]);
        assert_eq!(
            grain_line(&table, &notes).as_deref(),
            Some(
                "    grain: one row per year x region x active; measures: net_amt (SUM them over rows for any total)\n"
            )
        );
        let rendered = render_sales(&table, &notes);
        assert!(
            rendered.contains("    active String [0/1 flag]\n"),
            "{rendered}"
        );
    }

    #[test]
    fn a_text_non_additive_column_stays_in_the_grain_and_is_not_listed_as_a_measure() {
        let table = sales_with(&[
            ("year", "UInt16"),
            ("region", "String"),
            ("band", "String"),
            ("net_amt", "Float64"),
        ]);
        let notes = notes(vec![], vec![role_of("band", "non_additive", true)]);
        assert_eq!(
            grain_line(&table, &notes),
            Some(format!(
                "    grain: one row per year x region x band; measures: net_amt (SUM them over rows for any total); not additive: band {NOT_ADDITIVE_ADVICE}\n"
            ))
        );
    }

    #[test]
    fn a_table_left_with_no_measure_and_no_non_additive_column_has_no_grain_line() {
        let table = sales_with(&[("region", "String"), ("active", "UInt8")]);
        let notes = notes(vec![], vec![role_of("active", "flag", true)]);
        assert_eq!(grain_line(&table, &notes), None);
    }

    #[test]
    fn the_role_tag_follows_the_description_and_synonyms_and_precedes_the_stats() {
        let table = sales_with(&[("region", "String"), ("orders", "UInt32")]);
        let notes = notes(
            vec![],
            vec![with_role(
                with_synonyms(
                    entry("serving.sales", "orders", "Orders placed", true),
                    &["purchases"],
                ),
                "non_additive",
            )],
        );
        let mut out = String::new();
        render_table(
            &mut out,
            &table,
            None,
            &HashMap::new(),
            &HashMap::from([("orders".to_owned(), "range 1..9".to_owned())]),
            &notes,
        );
        assert!(
            out.contains(
                "    orders UInt32 — Orders placed (also called: purchases) [never SUM across rows]; range 1..9\n"
            ),
            "{out}"
        );
    }

    #[test]
    fn a_draft_fills_a_column_the_source_does_not_describe() {
        let n = notes(
            vec![],
            vec![entry(
                "serving.visits",
                "negara",
                "Country of origin",
                false,
            )],
        );
        assert!(
            render_served(&n)
                .contains("\n    negara String — Country of origin; values: Japan | Korea\n")
        );
    }

    #[test]
    fn the_source_description_wins_over_a_draft() {
        let n = notes(
            vec![],
            vec![entry("serving.visits", "tahun", "Model guess", false)],
        );
        assert_eq!(render_served(&n), SERVED_TODAY);
    }

    #[test]
    fn a_confirmed_entry_wins_over_the_source_description() {
        let n = notes(
            vec![],
            vec![entry("serving.visits", "tahun", "Fiscal year", true)],
        );
        let text = render_served(&n);
        assert!(text.contains("\n    tahun UInt16 — Fiscal year; range 2019..2024\n"));
        assert!(!text.contains("Calendar year"));
    }

    #[test]
    fn column_synonyms_follow_the_description_and_precede_the_stats() {
        let n = notes(
            vec![],
            vec![with_synonyms(
                entry("serving.visits", "negara", "Country of origin", true),
                &["nation", "origin country"],
            )],
        );
        assert!(render_served(&n).contains(
            "\n    negara String — Country of origin (also called: nation, origin country); values: Japan | Korea\n"
        ));
    }

    #[test]
    fn table_synonyms_follow_the_table_text() {
        let n = notes(
            vec![annotation("silver.raw_visits", "Raw visit sheet")],
            vec![with_synonyms(
                entry("silver.raw_visits", "", "Model text", false),
                &["arrivals", "tourists"],
            )],
        );
        assert!(render_silver(&n).starts_with(
            "- silver.raw_visits (row count unknown): Raw visit sheet (also called: arrivals, tourists)\n"
        ));
    }

    #[test]
    fn an_annotation_wins_over_a_draft_for_the_table() {
        let n = notes(
            vec![annotation("silver.raw_visits", "Raw visit sheet")],
            vec![entry("silver.raw_visits", "", "Model text", false)],
        );
        let text = render_silver(&n);
        assert!(text.starts_with("- silver.raw_visits (row count unknown): Raw visit sheet\n"));
        assert!(!text.contains("Model text"));
    }

    #[test]
    fn a_draft_describes_a_table_that_has_no_other_text() {
        let n = notes(
            vec![],
            vec![entry("silver.raw_visits", "", "Model text", false)],
        );
        assert!(
            render_silver(&n).starts_with("- silver.raw_visits (row count unknown): Model text\n")
        );
    }

    #[test]
    fn a_confirmed_entry_wins_over_a_draft_for_the_table() {
        let n = notes(
            vec![],
            vec![entry("silver.raw_visits", "", "Person text", true)],
        );
        assert!(
            render_silver(&n).starts_with("- silver.raw_visits (row count unknown): Person text\n")
        );
    }

    #[test]
    fn a_draft_is_not_added_to_a_table_whose_dataset_has_a_description() {
        let n = notes(
            vec![],
            vec![entry("serving.visits", "", "Model text", false)],
        );
        assert_eq!(render_served(&n), SERVED_TODAY);
    }

    #[test]
    fn a_confirmed_entry_is_added_to_a_table_whose_dataset_has_a_description() {
        let n = notes(
            vec![],
            vec![entry("serving.visits", "", "Person text", true)],
        );
        assert!(render_served(&n).contains("unit: visits): Monthly visits.: Person text\n"));
    }

    #[test]
    fn an_entry_for_a_column_the_table_no_longer_has_is_not_rendered() {
        let n = notes(
            vec![],
            vec![entry("silver.raw_visits", "dropped_col", "Gone", true)],
        );
        assert_eq!(render_silver(&n), SILVER_TODAY);
    }

    #[test]
    fn an_entry_for_another_table_is_not_rendered() {
        let n = notes(
            vec![annotation("silver.other", "Other table")],
            vec![entry("silver.other", "origin", "Other column", true)],
        );
        assert_eq!(render_silver(&n), SILVER_TODAY);
    }

    #[test]
    fn text_written_around_the_check_is_cut_to_the_rule() {
        let n = notes(
            vec![],
            vec![
                entry("silver.raw_visits", "", &"t".repeat(500), true),
                entry("silver.raw_visits", "origin", &"c".repeat(500), true),
                with_synonyms(
                    entry("silver.raw_visits", "count", "Amount", true),
                    &[&"s".repeat(60), "a", "b", "c", "d", "e", "seventh"],
                ),
            ],
        );
        let text = render_silver(&n);
        let expected = format!(
            "- silver.raw_visits (row count unknown): {}\n",
            "t".repeat(400)
        );
        assert!(text.starts_with(&expected));
        assert!(text.contains(&format!("\n    origin String — {}\n", "c".repeat(200))));
        assert!(text.contains(&format!(
            "\n    count UInt32 — Amount (also called: {}, a, b, c, d, e)\n",
            "s".repeat(40)
        )));
    }

    #[test]
    fn a_cut_lands_on_a_character_boundary() {
        let n = notes(
            vec![],
            vec![entry("silver.raw_visits", "origin", &"é".repeat(300), true)],
        );
        assert!(
            render_silver(&n).contains(&format!("\n    origin String — {}\n", "é".repeat(200)))
        );
    }

    #[test]
    fn a_line_break_in_a_description_cannot_start_a_new_line_of_the_map() {
        let n = notes(
            vec![],
            vec![entry("silver.raw_visits", "origin", "first\nsecond", true)],
        );
        assert!(render_silver(&n).contains("\n    origin String — first second\n"));
    }

    // ── the whole map, as it was before only the named tables were written in full ──

    /// An orders table in `serving` with no catalog dataset.
    fn orders_table() -> Table {
        Table {
            db: "serving".to_owned(),
            name: "orders".to_owned(),
            rows: Some(10),
            columns: vec![
                col("order_id", "UInt32"),
                col("status", "String"),
                col("total", "UInt32"),
            ],
        }
    }

    fn orders_stats() -> HashMap<String, String> {
        HashMap::from([("status".to_owned(), "values: open | paid".to_owned())])
    }

    /// Three live tables (two Gold, one Silver) and one catalog dataset
    /// whose Gold table does not exist.
    fn golden_live() -> Live {
        let ghost = Dataset {
            slug: "ghost-data".to_owned(),
            title: "Ghost data".to_owned(),
            description: String::new(),
            source_kind: "secondary source",
            publisher: String::new(),
            frequency: String::new(),
            unit: String::new(),
        };
        Live {
            tables: vec![served_table(), orders_table(), silver_table()],
            datasets: HashMap::from([
                ("visits".to_owned(), served_dataset()),
                ("ghost".to_owned(), ghost),
            ]),
            descriptions: served_descriptions(),
        }
    }

    fn facts_only(facts: HashMap<String, String>) -> Stats {
        Stats {
            facts,
            samples: HashMap::new(),
        }
    }

    fn golden_stats() -> Vec<Stats> {
        vec![
            facts_only(served_stats()),
            facts_only(orders_stats()),
            Stats::default(),
        ]
    }

    /// Pieces for the fixed tables, with policies that withhold nothing.
    fn golden_pieces() -> Pieces {
        pieces_from(
            &golden_live(),
            &golden_stats(),
            &Notes::default(),
            Some(&Withheld::default()),
        )
    }

    const ORDERS_TODAY: &str = "\
- serving.orders (10 rows)
    grain: one row per order_id x status; measures: total (SUM them over rows for any total)
    order_id UInt32
    status String; values: open | paid
    total UInt32
";
    const GHOST_LINE: &str = "\nCatalog datasets with no Gold table: ghost-data (\"Ghost data\", expected at serving.ghost, not found).\n";
    const SEE_COLUMNS_TODAY: &str = "Use describe_mart or `SELECT name, type FROM system.columns WHERE database = '…' AND table = '…'` to see their columns.";

    // Written by hand from the renderer's format before `build` was split
    // into pieces and `assemble`, and run against the unsplit code first.
    fn map_today() -> String {
        format!(
            "GOLD (database `serving`, aggregated marts; use these first for numbers):\n\
             {SERVED_TODAY}{ORDERS_TODAY}\nSILVER (database `silver`, cleaned detail rows):\n{SILVER_TODAY}{GHOST_LINE}"
        )
    }

    fn map_today_over_budget() -> String {
        format!(
            "GOLD (database `serving`, aggregated marts; use these first for numbers):\n\
             {SERVED_TODAY}\n2 more tables not described here (budget): serving.orders, silver.raw_visits. {SEE_COLUMNS_TODAY}\n{GHOST_LINE}"
        )
    }

    #[test]
    fn the_whole_map_is_the_text_it_was() {
        assert_eq!(
            assemble(&golden_pieces(), &HashSet::new(), true, MAX_CHARS),
            map_today()
        );
    }

    #[test]
    fn tables_past_the_budget_are_named_in_one_closing_line() {
        assert_eq!(
            assemble(&golden_pieces(), &HashSet::new(), true, 0),
            map_today_over_budget()
        );
    }

    // ── only the tables a question names are written in full ──

    /// A text column's stats: the fact line and the samples behind it.
    fn text_stats(column: &str, values: &[&str]) -> Stats {
        Stats {
            facts: HashMap::from([(column.to_owned(), format!("values: {}", values.join(" | ")))]),
            samples: HashMap::from([(
                column.to_owned(),
                values.iter().map(|v| (*v).to_owned()).collect(),
            )]),
        }
    }

    fn table_of(db: &str, name: &str, rows: Option<u64>, columns: Vec<Column>) -> Table {
        Table {
            db: db.to_owned(),
            name: name.to_owned(),
            rows,
            columns,
        }
    }

    /// Four invented tables: three in Gold, one in Silver. The words that
    /// find each one are its own: `swiftpost` (a sample value) only
    /// `shipments`, `purchases` (a synonym) only `orders`, `sales` (a
    /// description) only `customers`.
    fn four_tables() -> Live {
        Live {
            tables: vec![
                table_of(
                    "serving",
                    "orders",
                    Some(10),
                    vec![
                        col("order_id", "UInt32"),
                        col("status", "String"),
                        col("total", "UInt32"),
                    ],
                ),
                table_of(
                    "serving",
                    "customers",
                    Some(5),
                    vec![col("customer_id", "UInt32"), col("region", "String")],
                ),
                table_of(
                    "serving",
                    "shipments",
                    Some(7),
                    vec![
                        col("shipment_id", "UInt32"),
                        col("carrier", "String"),
                        col("weight", "UInt32"),
                    ],
                ),
                table_of(
                    "silver",
                    "raw_events",
                    None,
                    vec![col("event_name", "String"), col("count", "UInt32")],
                ),
            ],
            datasets: HashMap::new(),
            descriptions: HashMap::new(),
        }
    }

    fn four_tables_stats() -> Vec<Stats> {
        vec![
            text_stats("status", &["open", "paid"]),
            text_stats("region", &["northern", "southern"]),
            text_stats("carrier", &["parcelco", "swiftpost"]),
            text_stats("event_name", &["login", "logout"]),
        ]
    }

    /// The stats of `four_tables` where the shipments' samples are known but
    /// their fact line is not printed: what a masked column, or a
    /// row-filtered table, must look like to the entry. The samples are the
    /// leak a word must never be made from.
    fn four_tables_stats_with_unprinted_carrier_samples() -> Vec<Stats> {
        let mut stats = four_tables_stats();
        stats[2] = Stats {
            facts: HashMap::new(),
            samples: HashMap::from([(
                "carrier".to_owned(),
                vec!["parcelco".to_owned(), "swiftpost".to_owned()],
            )]),
        };
        stats
    }

    fn four_tables_notes() -> Notes {
        notes(
            vec![annotation(
                "serving.orders",
                "Customer orders placed online",
            )],
            vec![
                with_synonyms(entry("serving.orders", "", "unused", false), &["purchases"]),
                entry(
                    "serving.customers",
                    "region",
                    "Sales area of the customer",
                    true,
                ),
            ],
        )
    }

    fn four_tables_pieces_with(withheld: &Withheld, stats: &[Stats]) -> Pieces {
        pieces_from(&four_tables(), stats, &four_tables_notes(), Some(withheld))
    }

    fn four_tables_pieces() -> Pieces {
        four_tables_pieces_with(&Withheld::default(), &four_tables_stats())
    }

    fn query(words: &[&str]) -> HashSet<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    const SHIPMENTS_FULL: &str = "\
- serving.shipments (7 rows)
    grain: one row per shipment_id x carrier; measures: weight (SUM them over rows for any total)
    shipment_id UInt32
    carrier String; values: parcelco | swiftpost
    weight UInt32
";

    /// The text with every table in full, as it is when nothing is named.
    fn all_in_full() -> String {
        assemble(&four_tables_pieces(), &query(&[]), true, MAX_CHARS)
    }

    #[test]
    fn a_question_that_names_no_table_gets_the_map_it_always_got() {
        let text = assemble(&four_tables_pieces(), &query(&["zebra"]), true, MAX_CHARS);
        assert_eq!(text, all_in_full());
        assert!(text.contains("    order_id UInt32\n"), "{text}");
        assert!(!text.contains("shown on one line"), "{text}");
    }

    #[test]
    fn a_sample_value_puts_its_table_first_and_in_full_and_the_others_on_one_line() {
        let text = assemble(
            &four_tables_pieces(),
            &query(&["swiftpost"]),
            true,
            MAX_CHARS,
        );
        let expected = format!(
            "{GOLD_HEADING}\n{SHIPMENTS_FULL}\
             - serving.orders (10 rows): Customer orders placed online\n\
             - serving.customers (5 rows)\n\
             \n{SILVER_HEADING}\n\
             - silver.raw_events (row count unknown)\n\
             \nA table shown on one line is described in full when a question names it. {SEE_COLUMNS_TODAY}\n"
        );
        assert_eq!(text, expected);
    }

    #[test]
    fn a_synonym_names_its_table() {
        let text = assemble(
            &four_tables_pieces(),
            &query(&["purchases"]),
            true,
            MAX_CHARS,
        );
        assert!(text.contains("    order_id UInt32\n"), "{text}");
        assert!(!text.contains("    customer_id UInt32\n"), "{text}");
        assert!(text.contains("- serving.customers (5 rows)\n"), "{text}");
    }

    #[test]
    fn a_description_names_its_table() {
        let text = assemble(&four_tables_pieces(), &query(&["sales"]), true, MAX_CHARS);
        assert!(text.contains("    customer_id UInt32\n"), "{text}");
        assert!(!text.contains("    order_id UInt32\n"), "{text}");
    }

    #[test]
    fn a_word_of_three_characters_must_equal_a_table_word_and_four_may_begin_one() {
        let pieces = four_tables_pieces();
        let shown = |words: &[&str]| assemble(&pieces, &query(words), true, MAX_CHARS);
        // "ord" begins "order" and "orders" but is too short to count.
        assert_eq!(shown(&["ord"]), all_in_full());
        // "orde" is four characters and begins "order".
        assert!(
            shown(&["orde"]).contains("    order_id UInt32\n"),
            "a four-character beginning must name the table"
        );
        // "ders" is inside "orders" and begins nothing.
        assert_eq!(shown(&["ders"]), all_in_full());
        // A three-character word that equals a table word does name it.
        assert!(
            shown(&["raw"]).contains("    event_name String; values: login | logout\n"),
            "an equal three-character word must name the table"
        );
    }

    #[test]
    fn a_stored_term_brings_the_words_of_its_meaning_and_other_words_do_not() {
        let terms = [ChatTerm {
            term: "buyers".to_owned(),
            meaning: "customers by region".to_owned(),
            question: String::new(),
            updated_at: String::new(),
        }];
        let with_term = question_words(&["How many buyers are there?"], &terms);
        assert_eq!(with_term, query(&["buyers", "customers", "region"]));
        let text = assemble(&four_tables_pieces(), &with_term, true, MAX_CHARS);
        assert!(text.contains("    customer_id UInt32\n"), "{text}");
        assert!(!text.contains("    order_id UInt32\n"), "{text}");
        let without = question_words(&["How many clients are there?"], &terms);
        assert_eq!(without, query(&["clients"]));
    }

    fn term_meaning(term: &str, meaning: &str) -> ChatTerm {
        ChatTerm {
            term: term.to_owned(),
            meaning: meaning.to_owned(),
            question: String::new(),
            updated_at: String::new(),
        }
    }

    // PR review fix (SHOULD-FIX): a term was compared to one question word, so a
    // term of several words never matched.
    #[test]
    fn a_term_of_two_words_brings_its_meaning_only_when_both_are_in_the_question() {
        let terms = [term_meaning("active customer", "ordered in the past month")];
        let both = question_words(&["Show me every customer that is active"], &terms);
        assert!(both.contains("month"), "{both:?}");
        let one = question_words(&["Show me every customer"], &terms);
        assert!(!one.contains("month"), "{one:?}");
        // The two words may be in two of the last messages.
        let split = question_words(&["Who is active?", "And each customer?"], &terms);
        assert!(split.contains("month"), "{split:?}");
    }

    #[test]
    fn a_term_with_an_underscore_or_a_digit_is_found_as_the_phrase_in_the_question() {
        let terms = [
            term_meaning("vip_user", "paying clients"),
            term_meaning("q3", "july to september"),
        ];
        let words = question_words(&["List the vip_user rows for q3"], &terms);
        assert!(
            words.contains("paying") && words.contains("september"),
            "{words:?}"
        );
        // Inside a longer word the phrase is not the term.
        let other = question_words(&["List the vip_users for q30"], &terms);
        assert!(
            !other.contains("paying") && !other.contains("september"),
            "{other:?}"
        );
    }

    #[test]
    fn a_term_of_only_function_words_does_not_match_a_question_without_the_phrase() {
        let terms = [term_meaning("the of", "everything at all")];
        let words = question_words(&["How many of the orders were there?"], &terms);
        assert_eq!(words, query(&["orders"]));
    }

    #[test]
    fn a_term_of_only_function_words_matches_when_it_stands_as_a_phrase() {
        let terms = [term_meaning("of the", "everything at all")];
        let words = question_words(&["How many of the orders"], &terms);
        assert!(
            words.contains("everything") && words.contains("orders"),
            "{words:?}"
        );
    }

    #[test]
    fn function_words_and_short_words_are_not_part_of_a_question() {
        assert_eq!(
            question_words(&["How many of the orders were there in 2024?"], &[]),
            query(&["orders"])
        );
        assert_eq!(
            question_words(&["Berapa jumlah pesanan dari ke di?"], &[]),
            query(&["pesanan"])
        );
        assert_eq!(question_words(&["a bc def"], &[]), query(&["def"]));
    }

    #[test]
    fn a_question_is_read_from_the_last_two_user_messages() {
        let words = question_words(&["oldest", "middle", "newest"], &[]);
        assert_eq!(words, query(&["middle", "newest"]));
    }

    #[test]
    fn with_the_switch_off_the_map_is_the_text_it_was() {
        assert_eq!(
            assemble(
                &four_tables_pieces(),
                &query(&["swiftpost"]),
                false,
                MAX_CHARS
            ),
            all_in_full()
        );
    }

    #[test]
    fn with_a_budget_for_two_tables_the_named_one_is_kept() {
        let pieces = four_tables_pieces();
        let first = pieces.tables[0].full.len();
        let budget = GOLD_HEADING.len() + 1 + first;
        let in_order = assemble(&pieces, &query(&[]), true, budget);
        assert!(!in_order.contains("    shipment_id UInt32\n"), "{in_order}");
        assert!(in_order.contains("serving.shipments"), "{in_order}");
        let named = assemble(&pieces, &query(&["swiftpost"]), true, budget);
        assert!(named.contains(SHIPMENTS_FULL), "{named}");
        assert!(!named.contains("    order_id UInt32\n"), "{named}");
    }

    #[test]
    fn a_masked_columns_sample_does_not_name_its_table() {
        let masked = withheld_for(&[
            r#"{"roles":["Analyst"],"table":"serving.shipments","mask":["carrier"]}"#,
        ]);
        let stats = four_tables_stats_with_unprinted_carrier_samples();
        let leaked = four_tables_pieces_with(&masked, &stats);
        let text = assemble(&leaked, &query(&["swiftpost"]), true, MAX_CHARS);
        assert_eq!(text, assemble(&leaked, &query(&[]), true, MAX_CHARS));
        assert!(!text.contains("shown on one line"), "{text}");
        // The same samples with no policy do name the table, so the check
        // above is about the policy and not about the fixture.
        let open = four_tables_pieces_with(&Withheld::default(), &stats);
        assert!(
            assemble(&open, &query(&["swiftpost"]), true, MAX_CHARS).contains("shown on one line")
        );
    }

    #[test]
    fn a_row_filtered_tables_samples_and_unreadable_policies_name_nothing() {
        let filtered = withheld_for(&[
            r#"{"roles":["Analyst"],"table":"serving.shipments","rowFilter":"carrier = 'x'"}"#,
        ]);
        let stats = four_tables_stats_with_unprinted_carrier_samples();
        let pieces = four_tables_pieces_with(&filtered, &stats);
        assert!(
            !assemble(&pieces, &query(&["swiftpost"]), true, MAX_CHARS)
                .contains("shown on one line")
        );
        let unknown = pieces_from(&four_tables(), &stats, &four_tables_notes(), None);
        assert!(
            !assemble(&unknown, &query(&["swiftpost"]), true, MAX_CHARS)
                .contains("shown on one line")
        );
    }

    /// Build the four tables' pieces through [`collect_with`] with a stats
    /// reader that never touches `ClickHouse`, and return the names of the
    /// tables it was asked about.
    async fn collect_four(relevant_enabled: bool, budget: usize) -> (Pieces, Vec<String>) {
        let asked = std::sync::Mutex::new(Vec::new());
        let live = four_tables();
        let pieces = collect_with(
            &live,
            Some(&Withheld::default()),
            &four_tables_notes(),
            relevant_enabled,
            budget,
            |table| {
                Box::pin(async {
                    if let Ok(mut asked) = asked.lock() {
                        asked.push(table.name.clone());
                    }
                    let at = live.tables.iter().position(|t| t.name == table.name);
                    at.and_then(|i| four_tables_stats().into_iter().nth(i))
                        .unwrap_or_default()
                })
            },
        )
        .await;
        (pieces, asked.into_inner().unwrap_or_default())
    }

    // PR review fix (SHOULD-FIX): the build read the stats of every table even
    // with `AI_RELEVANT_TABLES` off, where origin/main read only the tables
    // inside the budget.
    #[tokio::test]
    async fn with_the_switch_off_a_table_past_the_budget_is_not_queried() {
        // A budget of one character is spent by the first table.
        let (off, asked_off) = collect_four(false, 1).await;
        assert_eq!(asked_off, ["orders"]);
        let (on, asked_on) = collect_four(true, 1).await;
        assert_eq!(asked_on, ["orders", "customers", "shipments", "raw_events"]);
        // The map the switch off writes is the same text either way.
        assert_eq!(
            assemble(&off, &query(&[]), false, 1),
            assemble(&on, &query(&[]), false, 1)
        );
    }

    #[tokio::test]
    async fn with_the_switch_off_and_room_in_the_budget_every_table_is_queried() {
        let (_, asked) = collect_four(false, MAX_CHARS).await;
        assert_eq!(asked, ["orders", "customers", "shipments", "raw_events"]);
    }

    /// The cache is one for every chat: two questions over the same cached
    /// pieces each get their own text, and a third that names nothing gets
    /// the whole map, whatever the first two named.
    #[tokio::test]
    async fn the_question_is_applied_per_request_and_never_stored_in_the_cache() {
        let _serial = CACHE_TEST_LOCK.lock().await;
        seed_pieces_for_test(four_tables_pieces()).await;
        // Never asked: every call below finds the pieces in the cache.
        let ch = ChClient::new(
            "http://127.0.0.1:1".to_owned(),
            "user".to_owned(),
            "password".to_owned(),
        );
        let withheld = Withheld::default();
        let ask = async |words: &[&str]| {
            data_map(&ch, Some(&withheld), &Notes::default(), &query(words), true).await
        };
        let shipments = ask(&["swiftpost"]).await;
        let customers = ask(&["northern"]).await;
        let nothing = ask(&["zebra"]).await;
        clear_cache().await;
        assert!(shipments.contains(SHIPMENTS_FULL), "{shipments}");
        assert!(
            !shipments.contains("    customer_id UInt32\n"),
            "{shipments}"
        );
        assert!(
            customers.contains("    customer_id UInt32\n"),
            "{customers}"
        );
        assert!(
            !customers.contains("    shipment_id UInt32\n"),
            "{customers}"
        );
        assert!(nothing.contains("    shipment_id UInt32\n"), "{nothing}");
        assert!(nothing.contains("    customer_id UInt32\n"), "{nothing}");
        assert!(nothing.contains("    order_id UInt32\n"), "{nothing}");
    }

    /// No other test in this crate's lib binary builds the map, so nothing
    /// refills the cache between the call and the check.
    #[tokio::test]
    async fn clearing_the_cache_drops_the_built_map() {
        let _serial = CACHE_TEST_LOCK.lock().await;
        seed_cache_for_test().await;
        clear_cache().await;
        assert!(cache_is_empty_for_test().await);
    }
}
