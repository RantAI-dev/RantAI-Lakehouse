//! Schema versions of Silver and Gold tables, recorded by the console
//! (ADR 0015, `docs/adr/0015-recorded-schema-versions.md`).
//!
//! A raw (Bronze) table's page shows when its columns were added, renamed
//! or retyped, because an `Iceberg` table carries every schema it has had in
//! its own metadata. A Silver or Gold table lives in `ClickHouse`, which
//! keeps one schema, the current one (`system.columns`): after an `ALTER`,
//! or a `CREATE OR REPLACE`, the definition before it is gone. So the
//! console records the history itself. Each time a pass sees that the
//! ordered list of `(column name, type)` of a table in the Silver database
//! (`silver`) or the Gold database (`serving`) differs from the last list it
//! recorded for that table, it appends a
//! version to `console.table_schema_version`, a `ClickHouse` table this
//! module owns and creates on first use — the arrangement
//! `routes::quality` has for `console.quality_run`. No `PostgreSQL`
//! migration: it is an observation about engine tables and needs no join
//! with console state.
//!
//! A table is keyed `<database>.<table>` (`silver.orders`,
//! `serving.mart_sales`), the key the catalog detail route itself uses. The
//! two databases are the pair that route serves ([`SILVER_DATABASE`],
//! [`GOLD_DATABASE`]), not [`crate::config::Config::gold_source_schema`],
//! which only the export routes read: a recorder that looked somewhere the
//! page never reads would make the page's "none recorded yet" untrue.
//!
//! # When the console looks
//!
//! One pass is two reads and at most one `INSERT` ([`observe`]). It is
//! started from three places, all through [`spawn_pass`]: when the API
//! starts (`main.rs`), on the alerts tick (`routes::alerts::run`, which the
//! orchestrator's `alerts_run_schedule` calls every fifteen minutes), and
//! when the orchestrator reports a run as finished
//! (`routes::pipelines::run_finished_event`).
//! One pass runs at a time; a start while another runs is refused, not
//! queued. A page view only ever reads ([`for_table`]): a read does not
//! write.
//!
//! # What a version is not
//!
//! - **It starts when it starts.** The first version of a table is dated
//!   when the console first saw it, not when the table was made.
//! - **It is an observation.** Two changes between two looks are recorded
//!   as one; a change undone before the next look is not recorded at all.
//! - **Columns are matched by name** ([`changes`]). A renamed column reads
//!   as one dropped and one added: the engine gives a column no identity
//!   that outlives its name.
//! - **It is the definition only.** The rows as they were are not kept.
//!
//! # Limits of the implementation
//!
//! - **One writer.** The single-flight flag ([`spawn_pass`]) is per
//!   process. Two API processes against one engine could each record the
//!   same number for a table; [`entries`] folds two equal consecutive lists
//!   into one entry, but one API per engine is assumed.
//! - **No cap on the read.** [`for_table`] reads every version of the
//!   table. A table whose columns differ at every look grows its list by one
//!   per look. There is deliberately no cap: a cut list would call its
//!   oldest shown entry "First recorded", which would be false.
//!
//! # Failure posture
//!
//! A failed pass is logged and changes nothing. The read path degrades
//! honestly: a store that does not exist yet reads as no versions, and so
//! does any other read failure after it is logged. An answer that is not a
//! result (see [`read`]) is a failed read, never "no rows". Upstream error
//! text never reaches a response, because nothing here puts it in one.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::SqlLiteral;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::routes::lakehouse::is_unknown_table_or_database_error;
use crate::routes::support::{nullable_u64_col, str_col};
use crate::state::AppState;

/// The Silver database, as `routes::catalog::clickhouse_asset_detail`
/// serves it (`silver.*`). A pass and that route must name the same pair of
/// databases, so whoever changes this or [`GOLD_DATABASE`] must change that
/// function's `db != "silver" && db != "serving"` check with it.
const SILVER_DATABASE: &str = "silver";

/// The Gold database, as `routes::catalog::clickhouse_asset_detail` serves
/// it (`serving.*`). Not [`crate::config::Config::gold_source_schema`]: the
/// catalog opens Gold tables as `serving.<table>` whatever that setting is,
/// and a version recorded under another name would never be read. See
/// [`SILVER_DATABASE`].
const GOLD_DATABASE: &str = "serving";

/// The databases one pass looks at: the pair the catalog detail route
/// serves.
const OBSERVED_DATABASES: [&str; 2] = [SILVER_DATABASE, GOLD_DATABASE];

/// The most tables one pass looks at. The rest, by name, wait for a
/// deployment that raises it: the pass logs how many it left out rather
/// than skipping them silently.
const MAX_TABLES: usize = 2_000;

/// Joins the sentences of one version's `change`.
const CHANGE_SEPARATOR: &str = " · ";

/// A table's columns as `(name, type)`, in the table's own order.
type Columns = Vec<(String, String)>;

// ── What changed between two column lists ─────────────────────────────

/// One sentence per change from `before` to `after`, matching columns by
/// name: `Added <name> (<type>)` and `Changed <name> from <type> to
/// <type>` in the table's current column order, then `Dropped <name>
/// (<type>)` in its previous order. When the same columns with the same
/// types only moved, the one sentence is `Reordered columns`.
///
/// Empty only when the two lists are equal: two different lists always
/// give at least one sentence.
fn changes(before: &[(String, String)], after: &[(String, String)]) -> Vec<String> {
    let was: HashMap<&str, &str> = before
        .iter()
        .map(|(name, ty)| (name.as_str(), ty.as_str()))
        .collect();
    let is: HashMap<&str, &str> = after
        .iter()
        .map(|(name, ty)| (name.as_str(), ty.as_str()))
        .collect();
    let mut out = Vec::new();
    for (name, ty) in after {
        match was.get(name.as_str()) {
            None => out.push(format!("Added {name} ({ty})")),
            Some(old) if old != ty => out.push(format!("Changed {name} from {old} to {ty}")),
            Some(_) => {}
        }
    }
    for (name, ty) in before {
        if !is.contains_key(name.as_str()) {
            out.push(format!("Dropped {name} ({ty})"));
        }
    }
    if out.is_empty() && before != after {
        out.push("Reordered columns".to_owned());
    }
    out
}

/// What the first version of a table says: how many columns it started
/// with, because nothing before it is known.
fn first_version_sentence(columns: usize) -> String {
    format!(
        "First recorded with {columns} column{}",
        if columns == 1 { "" } else { "s" }
    )
}

// ── Versions as the page shows them ───────────────────────────────────

/// One recorded version, as read from the store.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredVersion {
    version: u32,
    /// `YYYY-MM-DDTHH:MM:SSZ`, UTC.
    at: String,
    columns: Columns,
}

/// One version of a table as the asset page shows it
/// (`AssetDetail.schemaVersions` in `contracts/assets.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Entry {
    /// The number the console gave it, from 1.
    pub(crate) version: u32,
    /// `YYYY-MM-DDTHH:MM:SSZ`, UTC: when the console *saw* the table in
    /// this shape, not when the table changed.
    pub(crate) at: String,
    /// What changed from the version before, [`changes`] joined with
    /// [`CHANGE_SEPARATOR`]; for the first version, how many columns it
    /// started with.
    pub(crate) change: String,
    /// True on the newest entry only.
    pub(crate) current: bool,
}

/// The entries of one table, newest first, from its stored versions.
///
/// Two consecutive stored versions with the same column list — two passes
/// that raced — are one entry, the earlier one: the later one recorded no
/// change.
fn entries(versions: &[StoredVersion]) -> Vec<Entry> {
    let mut ordered: Vec<&StoredVersion> = versions.iter().collect();
    ordered.sort_by_key(|v| v.version);
    let mut kept: Vec<&StoredVersion> = Vec::new();
    for v in ordered {
        if kept.last().is_some_and(|last| last.columns == v.columns) {
            continue;
        }
        kept.push(v);
    }
    let mut out: Vec<Entry> = kept
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let change = match i.checked_sub(1).and_then(|before| kept.get(before)) {
                None => first_version_sentence(v.columns.len()),
                Some(before) => changes(&before.columns, &v.columns).join(CHANGE_SEPARATOR),
            };
            Entry {
                version: v.version,
                at: v.at.clone(),
                change,
                current: false,
            }
        })
        .collect();
    out.reverse();
    if let Some(newest) = out.first_mut() {
        newest.current = true;
    }
    out
}

// ── console.table_schema_version ──────────────────────────────────────

static TABLE_ENSURED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// Creates the store on first use. A failed attempt is not cached, so a
/// `ClickHouse` that was down at the first pass does not wedge the later
/// ones.
async fn ensure_table(ch: &ChClient) -> Result<(), ChError> {
    ensure_table_once(&TABLE_ENSURED, ch).await
}

async fn ensure_table_once(
    ensured: &tokio::sync::OnceCell<()>,
    ch: &ChClient,
) -> Result<(), ChError> {
    ensured
        .get_or_try_init(|| async {
            ch.exec("CREATE DATABASE IF NOT EXISTS console", None)
                .await?;
            ch.exec(
                "CREATE TABLE IF NOT EXISTS console.table_schema_version (\n\
                   table_key String,\n\
                   version UInt32,\n\
                   columns String,\n\
                   observed_at DateTime64(3, 'UTC')\n\
                 ) ENGINE = MergeTree ORDER BY (table_key, version)",
                None,
            )
            .await
        })
        .await
        .map(drop)
}

/// Why a read of the engine did not give an answer to act on. Logged, never
/// put in a response.
#[derive(Debug, thiserror::Error)]
enum EngineError {
    /// The engine, or the way to it, failed a statement.
    #[error("the engine failed a statement")]
    Failed(#[from] ChError),
    /// The engine answered `2xx` with a body that was not a `FORMAT JSON`
    /// result.
    #[error("the engine's answer to a read was not a result")]
    NotAResult,
}

/// Runs a read and returns its rows, refusing an answer that is not a
/// result.
///
/// `ChClient::query` turns any `2xx` body that is not JSON into an empty
/// [`lakehouse_clickhouse::ChResult`] (`ClickHouse` does send `200` and then
/// fail mid-stream), which `rows` would hand back as zero rows. Here that
/// would read as "nothing recorded" and the pass would record version 1
/// again for every table. A real `FORMAT JSON` answer always carries `meta`,
/// even with no rows, so an empty `meta` is a failed read.
async fn read(ch: &ChClient, sql: &str) -> Result<Vec<Map<String, Value>>, EngineError> {
    let result = ch.query(sql, None).await?;
    if result.meta.is_empty() {
        return Err(EngineError::NotAResult);
    }
    Ok(result.data)
}

/// A column list as the store holds it: a JSON array of `[name, type]`
/// pairs, in the table's column order.
fn columns_json(columns: &[(String, String)]) -> String {
    json!(columns).to_string()
}

/// The inverse of [`columns_json`]; `None` for text that is not that.
fn parse_columns(text: &str) -> Option<Columns> {
    serde_json::from_str(text).ok()
}

/// The versions a table has in the store, oldest first, or `None` when a
/// row cannot be read as one.
fn stored_versions(rows: &[Map<String, Value>]) -> Option<Vec<StoredVersion>> {
    rows.iter()
        .map(|row| {
            Some(StoredVersion {
                version: u32::try_from(nullable_u64_col(row, "version")?).ok()?,
                at: str_col(row, "observed").to_owned(),
                columns: parse_columns(str_col(row, "columns"))?,
            })
        })
        .collect()
}

/// The versions of one table, newest first, for its asset detail. `table_key`
/// is `<database>.<table>`.
///
/// A store that does not exist yet means nothing was recorded, which is
/// the empty list it is; any other failure, an answer that is not a result,
/// or a row that is not a version, is logged and reads the same, so the page
/// still loads and an empty list already reads as "none recorded".
pub(crate) async fn for_table(ch: &ChClient, table_key: &str) -> Vec<Entry> {
    let sql = format!(
        "SELECT version, columns, \
                formatDateTime(observed_at, '%Y-%m-%dT%H:%i:%SZ', 'UTC') AS observed \
         FROM console.table_schema_version WHERE table_key = {} ORDER BY version",
        SqlLiteral::from(table_key)
    );
    match read(ch, &sql).await {
        Ok(rows) => stored_versions(&rows).map_or_else(
            || {
                tracing::warn!(
                    table = table_key,
                    "schema versions: a stored row is unreadable"
                );
                Vec::new()
            },
            |versions| entries(&versions),
        ),
        Err(EngineError::Failed(ChError::Server(ref body)))
            if is_unknown_table_or_database_error(body) =>
        {
            Vec::new()
        }
        Err(err) => {
            tracing::warn!(?err, table = table_key, "schema versions: read unavailable");
            Vec::new()
        }
    }
}

// ── One pass ──────────────────────────────────────────────────────────

/// The newest version the store holds for one table.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Newest {
    version: u32,
    /// `None` when the stored text is not a column list: it then differs
    /// from whatever the table has now, and the next version is recorded.
    columns: Option<Columns>,
}

/// One version a pass is about to record.
#[derive(Debug, PartialEq, Eq)]
struct NewVersion<'a> {
    key: &'a str,
    version: u32,
    columns: &'a [(String, String)],
}

/// Every table of `databases` with its ordered columns, keyed
/// `<database>.<table>`, in one read. A table with no columns is left out.
async fn current_tables(
    ch: &ChClient,
    databases: &[&str],
) -> Result<BTreeMap<String, Columns>, EngineError> {
    let list = databases
        .iter()
        .map(|db| SqlLiteral::from(*db).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT database, table, name, type FROM system.columns \
         WHERE database IN ({list}) ORDER BY database, table, position"
    );
    let mut tables: BTreeMap<String, Columns> = BTreeMap::new();
    for row in read(ch, &sql).await? {
        let key = format!("{}.{}", str_col(&row, "database"), str_col(&row, "table"));
        tables.entry(key).or_default().push((
            str_col(&row, "name").to_owned(),
            str_col(&row, "type").to_owned(),
        ));
    }
    tables.retain(|_, columns| !columns.is_empty());
    Ok(tables)
}

/// The first [`MAX_TABLES`] tables by name, and how many were left out.
fn limit_tables(tables: BTreeMap<String, Columns>) -> (BTreeMap<String, Columns>, usize) {
    let left_out = tables.len().saturating_sub(MAX_TABLES);
    (tables.into_iter().take(MAX_TABLES).collect(), left_out)
}

/// The newest recorded version of every table, in one read.
///
/// The aggregates are aliased `newest_version` and `newest_columns`, not
/// `version` and `columns`: `ClickHouse` resolves an alias inside the same
/// `SELECT`, so `argMax(columns, version) … AS version` would read
/// `version` as the alias `max(version)` and the engine refuses the
/// statement (`Code: 184`, `ILLEGAL_AGGREGATION`, "found inside another
/// aggregate function"), which no pass could then get past. A mock cannot
/// see this; do not rename the aliases back.
const NEWEST_RECORDED_SQL: &str = "SELECT table_key, max(version) AS newest_version, \
     argMax(columns, version) AS newest_columns \
     FROM console.table_schema_version GROUP BY table_key";

async fn newest_recorded(ch: &ChClient) -> Result<HashMap<String, Newest>, EngineError> {
    let rows = read(ch, NEWEST_RECORDED_SQL).await?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let version = u32::try_from(nullable_u64_col(row, "newest_version")?).ok()?;
            Some((
                str_col(row, "table_key").to_owned(),
                Newest {
                    version,
                    columns: parse_columns(str_col(row, "newest_columns")),
                },
            ))
        })
        .collect())
}

/// The versions to record: a table with none recorded gets version 1, one
/// whose columns differ from its newest recorded list gets the number
/// after it, and one that has not changed gets nothing.
fn plan<'a>(
    tables: &'a BTreeMap<String, Columns>,
    recorded: &HashMap<String, Newest>,
) -> Vec<NewVersion<'a>> {
    tables
        .iter()
        .filter_map(|(key, columns)| {
            let version = match recorded.get(key) {
                None => 1,
                Some(newest) if newest.columns.as_ref() == Some(columns) => return None,
                Some(newest) => newest.version.saturating_add(1),
            };
            Some(NewVersion {
                key,
                version,
                columns,
            })
        })
        .collect()
}

/// One `INSERT` for every version of the pass. `version` is a `u32`, so it
/// renders as digits and nothing else; every text goes through
/// [`SqlLiteral`].
fn insert_sql(versions: &[NewVersion<'_>]) -> String {
    let values = versions
        .iter()
        .map(|v| {
            format!(
                "({}, {}, {}, now64(3))",
                SqlLiteral::from(v.key),
                v.version,
                SqlLiteral::from(columns_json(v.columns))
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO console.table_schema_version (table_key, version, columns, observed_at) \
         VALUES {values}"
    )
}

/// One pass over `silver` and `serving` ([`OBSERVED_DATABASES`]): records a
/// version for each table whose columns differ from the last list recorded
/// for it, and returns how many it recorded.
///
/// # Errors
///
/// Returns [`EngineError::Failed`] with the [`ChError`] of the first read
/// or write that failed, or [`EngineError::NotAResult`] when a read was
/// answered with something that is not a result. Nothing is recorded then:
/// the one `INSERT` is the last step, so a failed read leaves the store as
/// it was.
async fn observe(state: &AppState) -> Result<usize, EngineError> {
    observe_databases(&state.clickhouse, &OBSERVED_DATABASES).await
}

async fn observe_databases(ch: &ChClient, databases: &[&str]) -> Result<usize, EngineError> {
    let (tables, left_out) = limit_tables(current_tables(ch, databases).await?);
    if left_out > 0 {
        tracing::warn!(
            looked_at = MAX_TABLES,
            left_out,
            "schema versions: more tables than a pass looks at; the last ones by name were not looked at"
        );
    }
    if tables.is_empty() {
        return Ok(0);
    }
    ensure_table(ch).await?;
    let recorded = newest_recorded(ch).await?;
    let versions = plan(&tables, &recorded);
    if versions.is_empty() {
        return Ok(0);
    }
    ch.exec(&insert_sql(&versions), None).await?;
    Ok(versions.len())
}

// ── The background pass ───────────────────────────────────────────────

/// Set while a pass is running, so a slow pass is not joined by another.
static PASS_RUNNING: AtomicBool = AtomicBool::new(false);

/// Holds a "pass running" flag and clears it when dropped, however the pass
/// ends — a pass that panicked must not stop every later one from
/// starting.
struct PassGuard(&'static AtomicBool);

impl PassGuard {
    /// Takes `flag`; `None` while someone else holds it.
    fn claim(flag: &'static AtomicBool) -> Option<Self> {
        // The guard is built only on the claimed path: one built and dropped
        // on the refused path would clear the flag the running pass holds.
        if flag.swap(true, Ordering::SeqCst) {
            None
        } else {
            Some(Self(flag))
        }
    }
}

impl Drop for PassGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Starts one pass in the background and returns at once, because the
/// callers — the API's start-up, the alerts tick, a run-finished report —
/// must not wait on a scan of the engine's tables. Returns `false`,
/// starting nothing, while another pass runs. A failed pass is logged and
/// changes nothing.
pub(crate) fn spawn_pass(state: &AppState) -> bool {
    spawn_pass_on(&PASS_RUNNING, state)
}

fn spawn_pass_on(flag: &'static AtomicBool, state: &AppState) -> bool {
    let Some(running) = PassGuard::claim(flag) else {
        return false;
    };
    let state = state.clone();
    tokio::spawn(async move {
        let _running = running;
        match observe(&state).await {
            Ok(recorded) => tracing::info!(recorded, "schema versions: pass finished"),
            Err(err) => tracing::warn!(?err, "schema versions: pass failed"),
        }
    });
    true
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::time::Duration;

    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Config;

    fn cols(pairs: &[(&str, &str)]) -> Columns {
        pairs
            .iter()
            .map(|(name, ty)| ((*name).to_owned(), (*ty).to_owned()))
            .collect()
    }

    fn stored(version: u32, at: &str, pairs: &[(&str, &str)]) -> StoredVersion {
        StoredVersion {
            version,
            at: at.to_owned(),
            columns: cols(pairs),
        }
    }

    // ── changes ──

    #[test]
    fn an_added_column_is_named_with_its_type() {
        let before = cols(&[("id", "UInt64")]);
        let after = cols(&[("id", "UInt64"), ("email", "String")]);
        assert_eq!(changes(&before, &after), vec!["Added email (String)"]);
    }

    #[test]
    fn a_dropped_column_is_named_with_its_type() {
        let before = cols(&[("id", "UInt64"), ("email", "String")]);
        let after = cols(&[("id", "UInt64")]);
        assert_eq!(changes(&before, &after), vec!["Dropped email (String)"]);
    }

    #[test]
    fn a_retyped_column_names_both_types() {
        let before = cols(&[("id", "UInt32")]);
        let after = cols(&[("id", "UInt64")]);
        assert_eq!(
            changes(&before, &after),
            vec!["Changed id from UInt32 to UInt64"]
        );
    }

    #[test]
    fn several_changes_come_as_added_and_changed_in_table_order_then_dropped() {
        let before = cols(&[("id", "UInt32"), ("legacy", "String"), ("note", "String")]);
        let after = cols(&[
            ("id", "UInt64"),
            ("note", "String"),
            ("created_at", "DateTime"),
        ]);
        assert_eq!(
            changes(&before, &after),
            vec![
                "Changed id from UInt32 to UInt64",
                "Added created_at (DateTime)",
                "Dropped legacy (String)",
            ]
        );
    }

    #[test]
    fn a_renamed_column_reads_as_one_dropped_and_one_added() {
        let before = cols(&[("id", "UInt64"), ("mail", "String")]);
        let after = cols(&[("id", "UInt64"), ("email", "String")]);
        assert_eq!(
            changes(&before, &after),
            vec!["Added email (String)", "Dropped mail (String)"]
        );
    }

    #[test]
    fn the_same_columns_in_another_order_are_a_reorder() {
        let before = cols(&[("id", "UInt64"), ("email", "String")]);
        let after = cols(&[("email", "String"), ("id", "UInt64")]);
        assert_eq!(changes(&before, &after), vec!["Reordered columns"]);
    }

    #[test]
    fn equal_lists_have_no_changes_and_different_lists_always_have_one() {
        let a = cols(&[("id", "UInt64"), ("email", "String")]);
        assert!(changes(&a, &a).is_empty());
        let pairs = [
            (cols(&[]), cols(&[("id", "UInt64")])),
            (cols(&[("id", "UInt64")]), cols(&[])),
            (a.clone(), cols(&[("email", "String"), ("id", "UInt64")])),
            (a, cols(&[("id", "UInt64"), ("email", "Nullable(String)")])),
            // A repeated name cannot happen in the engine; the rule holds anyway.
            (
                cols(&[("id", "UInt64")]),
                cols(&[("id", "UInt64"), ("id", "UInt64")]),
            ),
        ];
        for (before, after) in &pairs {
            assert!(
                !changes(before, after).is_empty(),
                "{before:?} -> {after:?} must say what changed"
            );
        }
    }

    // ── entries ──

    #[test]
    fn the_first_version_says_how_many_columns_it_started_with() {
        assert_eq!(first_version_sentence(1), "First recorded with 1 column");
        assert_eq!(first_version_sentence(2), "First recorded with 2 columns");
        assert_eq!(first_version_sentence(0), "First recorded with 0 columns");
    }

    #[test]
    fn entries_are_newest_first_and_only_the_newest_is_current() {
        let versions = [
            stored(1, "2026-10-01T08:00:00Z", &[("id", "UInt64")]),
            stored(
                2,
                "2026-10-03T09:30:00Z",
                &[("id", "UInt64"), ("email", "String")],
            ),
            stored(3, "2026-10-04T10:00:00Z", &[("id", "UInt64")]),
        ];
        assert_eq!(
            entries(&versions),
            vec![
                Entry {
                    version: 3,
                    at: "2026-10-04T10:00:00Z".to_owned(),
                    change: "Dropped email (String)".to_owned(),
                    current: true,
                },
                Entry {
                    version: 2,
                    at: "2026-10-03T09:30:00Z".to_owned(),
                    change: "Added email (String)".to_owned(),
                    current: false,
                },
                Entry {
                    version: 1,
                    at: "2026-10-01T08:00:00Z".to_owned(),
                    change: "First recorded with 1 column".to_owned(),
                    current: false,
                },
            ]
        );
    }

    #[test]
    fn several_sentences_of_one_version_are_joined() {
        let versions = [
            stored(1, "2026-10-01T08:00:00Z", &[("id", "UInt32")]),
            stored(
                2,
                "2026-10-02T08:00:00Z",
                &[("id", "UInt64"), ("email", "String")],
            ),
        ];
        assert_eq!(
            entries(&versions)[0].change,
            "Changed id from UInt32 to UInt64 · Added email (String)"
        );
    }

    #[test]
    fn a_stored_order_that_is_not_by_version_is_sorted() {
        let versions = [
            stored(
                2,
                "2026-10-02T08:00:00Z",
                &[("id", "UInt64"), ("a", "String")],
            ),
            stored(1, "2026-10-01T08:00:00Z", &[("id", "UInt64")]),
        ];
        let out = entries(&versions);
        assert_eq!(out.iter().map(|e| e.version).collect::<Vec<_>>(), [2, 1]);
        assert!(out[0].current && !out[1].current);
    }

    #[test]
    fn two_consecutive_versions_with_the_same_columns_are_one_entry() {
        // Two passes that raced both recorded version 2.
        let versions = [
            stored(1, "2026-10-01T08:00:00Z", &[("id", "UInt64")]),
            stored(
                2,
                "2026-10-02T08:00:00Z",
                &[("id", "UInt64"), ("email", "String")],
            ),
            stored(
                3,
                "2026-10-02T08:00:01Z",
                &[("id", "UInt64"), ("email", "String")],
            ),
        ];
        let out = entries(&versions);
        assert_eq!(out.iter().map(|e| e.version).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(out[0].at, "2026-10-02T08:00:00Z", "the earlier one stands");
        assert_eq!(out[0].change, "Added email (String)");
        assert!(out[0].current);
    }

    #[test]
    fn a_table_that_went_back_to_an_earlier_shape_keeps_both_entries() {
        let versions = [
            stored(1, "2026-10-01T08:00:00Z", &[("id", "UInt64")]),
            stored(
                2,
                "2026-10-02T08:00:00Z",
                &[("id", "UInt64"), ("email", "String")],
            ),
            stored(3, "2026-10-03T08:00:00Z", &[("id", "UInt64")]),
        ];
        assert_eq!(entries(&versions).len(), 3);
    }

    #[test]
    fn no_stored_versions_is_no_entries() {
        assert!(entries(&[]).is_empty());
    }

    #[test]
    fn an_entry_serializes_in_the_shape_the_console_reads() {
        let entry = Entry {
            version: 2,
            at: "2026-10-03T09:30:00Z".to_owned(),
            change: "Added email (String)".to_owned(),
            current: true,
        };
        assert_eq!(
            json!(entry),
            json!({
                "version": 2,
                "at": "2026-10-03T09:30:00Z",
                "change": "Added email (String)",
                "current": true,
            })
        );
    }

    // ── what a pass records ──

    fn tables(entries: &[(&str, &[(&str, &str)])]) -> BTreeMap<String, Columns> {
        entries
            .iter()
            .map(|(key, pairs)| ((*key).to_owned(), cols(pairs)))
            .collect()
    }

    fn newest(version: u32, pairs: &[(&str, &str)]) -> Newest {
        Newest {
            version,
            columns: Some(cols(pairs)),
        }
    }

    #[test]
    fn a_pass_records_a_new_table_as_one_a_changed_one_as_the_next_and_leaves_the_rest() {
        let seen = tables(&[
            ("serving.sales", &[("id", "UInt64")]),
            ("silver.new_table", &[("id", "UInt64")]),
            ("silver.orders", &[("id", "UInt64"), ("email", "String")]),
            ("silver.stable", &[("id", "UInt64")]),
        ]);
        let recorded = HashMap::from([
            ("serving.sales".to_owned(), newest(4, &[("id", "UInt64")])),
            ("silver.orders".to_owned(), newest(2, &[("id", "UInt64")])),
            ("silver.stable".to_owned(), newest(1, &[("id", "UInt64")])),
            // Recorded, but not seen now: a table that went away records nothing.
            ("silver.gone".to_owned(), newest(3, &[("id", "UInt64")])),
        ]);
        let versions = plan(&seen, &recorded);
        assert_eq!(
            versions
                .iter()
                .map(|v| (v.key, v.version))
                .collect::<Vec<_>>(),
            [("silver.new_table", 1), ("silver.orders", 3)]
        );
        assert_eq!(
            versions[1].columns,
            cols(&[("id", "UInt64"), ("email", "String")])
        );
    }

    #[test]
    fn a_stored_list_that_cannot_be_read_is_recorded_over_with_the_next_number() {
        let seen = tables(&[("silver.orders", &[("id", "UInt64")])]);
        let recorded = HashMap::from([(
            "silver.orders".to_owned(),
            Newest {
                version: 5,
                columns: None,
            },
        )]);
        let versions = plan(&seen, &recorded);
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, 6);
    }

    #[test]
    fn a_pass_looks_at_the_first_two_thousand_tables_by_name_and_counts_the_rest() {
        let many: BTreeMap<String, Columns> = (0..2_003)
            .map(|n| (format!("silver.t{n:05}"), cols(&[("id", "UInt64")])))
            .collect();
        let (kept, left_out) = limit_tables(many);
        assert_eq!(kept.len(), 2_000);
        assert_eq!(left_out, 3);
        assert!(kept.contains_key("silver.t00000"));
        assert!(kept.contains_key("silver.t01999"));
        assert!(!kept.contains_key("silver.t02000"));

        let (kept, left_out) = limit_tables(tables(&[("silver.a", &[("id", "UInt64")])]));
        assert_eq!((kept.len(), left_out), (1, 0));
    }

    #[test]
    fn one_insert_carries_every_version_and_escapes_what_it_quotes() {
        let seen = tables(&[
            ("silver.a", &[("id", "UInt64")]),
            ("silver.b", &[("it's", "String"), ("back\\slash", "String")]),
        ]);
        let sql = insert_sql(&plan(&seen, &HashMap::new()));
        assert_eq!(sql.matches("INSERT INTO").count(), 1);
        assert!(sql.starts_with(
            "INSERT INTO console.table_schema_version \
             (table_key, version, columns, observed_at) VALUES "
        ));
        assert!(sql.contains(r#"('silver.a', 1, '[["id","UInt64"]]', now64(3))"#));
        // The quote is doubled and the backslash is doubled, so neither can
        // end the literal; the JSON's own escape of the backslash is one more.
        assert!(
            sql.contains(
                r#"('silver.b', 1, '[["it''s","String"],["back\\\\slash","String"]]', now64(3))"#
            ),
            "{sql}"
        );
    }

    #[test]
    fn a_column_list_round_trips_through_the_store_text() {
        let columns = cols(&[("id", "UInt64"), ("it's \"odd\"", "Nullable(String)")]);
        assert_eq!(parse_columns(&columns_json(&columns)), Some(columns));
        assert_eq!(parse_columns("not json"), None);
        assert_eq!(parse_columns(r#"[["only one"]]"#), None);
    }

    // ── against a fake engine ──

    /// A real `FORMAT JSON` answer: `meta` names the selected columns even
    /// when there are no rows, which is what [`read`] relies on to tell a
    /// result from a body that is not one.
    fn ch_rows(names: &[&str], data: &Value) -> ResponseTemplate {
        let meta: Vec<Value> = names
            .iter()
            .map(|name| json!({ "name": name, "type": "String" }))
            .collect();
        ResponseTemplate::new(200).set_body_json(json!({
            "meta": meta,
            "data": data,
            "rows": data.as_array().map_or(0, Vec::len),
        }))
    }

    fn column_row(database: &str, table: &str, name: &str, ty: &str) -> Value {
        json!({ "database": database, "table": table, "name": name, "type": ty })
    }

    /// One row of the pass's read of the newest version per table.
    fn store_row(key: &str, version: u32, columns: &str) -> Value {
        json!({ "table_key": key, "newest_version": version, "newest_columns": columns })
    }

    async fn mount_columns(server: &MockServer, rows: &Value) {
        Mock::given(method("POST"))
            .and(body_string_contains("FROM system.columns"))
            .respond_with(ch_rows(&["database", "table", "name", "type"], rows))
            .mount(server)
            .await;
    }

    /// The pass's read of the newest version per table.
    async fn mount_newest(server: &MockServer, rows: &Value) {
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.table_schema_version"))
            .respond_with(ch_rows(
                &["table_key", "newest_version", "newest_columns"],
                rows,
            ))
            .mount(server)
            .await;
    }

    /// The page's read of one table's versions.
    async fn mount_versions(server: &MockServer, rows: &Value) {
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.table_schema_version"))
            .respond_with(ch_rows(&["version", "columns", "observed"], rows))
            .mount(server)
            .await;
    }

    /// The `CREATE` statements and the `INSERT`: a bare `200`, as the engine
    /// answers a statement that returns no rows. Mounted last, after the
    /// reads, so it never answers one of them.
    async fn mount_writes(server: &MockServer) {
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(server)
            .await;
    }

    async fn bodies(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .expect("the server records requests")
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }

    fn inserts(bodies: &[String]) -> Vec<&String> {
        bodies
            .iter()
            .filter(|b| b.contains("INSERT INTO console.table_schema_version"))
            .collect()
    }

    fn client(server: &MockServer) -> ChClient {
        ChClient::new(server.uri(), "default".to_owned(), String::new())
    }

    #[tokio::test]
    async fn the_store_is_created_once_with_the_columns_the_pass_writes_and_reads() {
        let server = MockServer::start().await;
        mount_writes(&server).await;
        let ch = client(&server);
        let ensured = tokio::sync::OnceCell::const_new();

        ensure_table_once(&ensured, &ch).await.unwrap();
        ensure_table_once(&ensured, &ch).await.unwrap();

        let seen = bodies(&server).await;
        assert_eq!(
            seen.len(),
            2,
            "one CREATE DATABASE and one CREATE TABLE, not four"
        );
        assert_eq!(seen[0], "CREATE DATABASE IF NOT EXISTS console");
        assert!(seen[1].contains("CREATE TABLE IF NOT EXISTS console.table_schema_version"));
        for column in ["table_key String", "version UInt32", "columns String"] {
            assert!(seen[1].contains(column), "{column}");
        }
        assert!(seen[1].contains("observed_at DateTime64(3, 'UTC')"));
        assert!(seen[1].contains("ENGINE = MergeTree ORDER BY (table_key, version)"));
    }

    #[tokio::test]
    async fn a_failed_create_is_not_remembered_so_the_next_pass_tries_again() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Code: 999. DB::Exception"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        mount_writes(&server).await;
        let ch = client(&server);
        let ensured = tokio::sync::OnceCell::const_new();

        assert!(ensure_table_once(&ensured, &ch).await.is_err());
        ensure_table_once(&ensured, &ch).await.unwrap();
    }

    #[tokio::test]
    async fn a_new_table_is_recorded_as_version_one() {
        let server = MockServer::start().await;
        mount_columns(
            &server,
            &json!([
                column_row("silver", "orders", "id", "UInt64"),
                column_row("silver", "orders", "email", "String"),
            ]),
        )
        .await;
        mount_newest(&server, &json!([])).await;
        mount_writes(&server).await;

        let recorded = observe_databases(&client(&server), &["silver", "serving"])
            .await
            .unwrap();

        assert_eq!(recorded, 1);
        let seen = bodies(&server).await;
        let writes = inserts(&seen);
        assert_eq!(writes.len(), 1);
        assert!(
            writes[0].contains(
                r#"('silver.orders', 1, '[["id","UInt64"],["email","String"]]', now64(3))"#
            ),
            "{}",
            writes[0]
        );
    }

    #[tokio::test]
    async fn the_two_databases_are_asked_for_as_literals_in_one_read() {
        let server = MockServer::start().await;
        mount_columns(&server, &json!([])).await;
        mount_writes(&server).await;

        observe_databases(&client(&server), &["silver", "mar'ts"])
            .await
            .unwrap();

        let seen = bodies(&server).await;
        let reads: Vec<&String> = seen
            .iter()
            .filter(|b| b.contains("system.columns"))
            .collect();
        assert_eq!(reads.len(), 1);
        assert!(reads[0].contains("WHERE database IN ('silver', 'mar''ts')"));
        assert!(reads[0].contains("ORDER BY database, table, position"));
        assert!(
            inserts(&seen).is_empty() && seen.len() == 1,
            "an engine with no such tables is read once and neither creates nor writes anything"
        );
    }

    #[tokio::test]
    async fn an_unchanged_table_records_nothing() {
        let server = MockServer::start().await;
        mount_columns(
            &server,
            &json!([
                column_row("silver", "orders", "id", "UInt64"),
                column_row("silver", "orders", "email", "String"),
            ]),
        )
        .await;
        mount_newest(
            &server,
            &json!([store_row(
                "silver.orders",
                3,
                r#"[["id","UInt64"],["email","String"]]"#
            )]),
        )
        .await;
        mount_writes(&server).await;

        let recorded = observe_databases(&client(&server), &["silver"])
            .await
            .unwrap();

        assert_eq!(recorded, 0);
        assert!(inserts(&bodies(&server).await).is_empty());
    }

    #[tokio::test]
    async fn a_changed_table_gets_the_next_number() {
        let server = MockServer::start().await;
        mount_columns(
            &server,
            &json!([
                column_row("serving", "sales", "id", "UInt64"),
                column_row("serving", "sales", "total", "Decimal(18, 2)"),
            ]),
        )
        .await;
        mount_newest(
            &server,
            &json!([store_row("serving.sales", 4, r#"[["id","UInt64"]]"#)]),
        )
        .await;
        mount_writes(&server).await;

        let recorded = observe_databases(&client(&server), &["silver", "serving"])
            .await
            .unwrap();

        assert_eq!(recorded, 1);
        let seen = bodies(&server).await;
        let writes = inserts(&seen);
        assert_eq!(writes.len(), 1);
        assert!(
            writes[0].contains(
                r#"('serving.sales', 5, '[["id","UInt64"],["total","Decimal(18, 2)"]]', now64(3))"#
            ),
            "{}",
            writes[0]
        );
    }

    #[tokio::test]
    async fn several_changed_tables_go_in_one_insert() {
        let server = MockServer::start().await;
        mount_columns(
            &server,
            &json!([
                column_row("serving", "sales", "id", "UInt64"),
                column_row("silver", "customers", "id", "UInt64"),
                column_row("silver", "orders", "id", "UInt64"),
                column_row("silver", "stable", "id", "UInt64"),
            ]),
        )
        .await;
        mount_newest(
            &server,
            &json!([store_row("silver.stable", 1, r#"[["id","UInt64"]]"#)]),
        )
        .await;
        mount_writes(&server).await;

        let recorded = observe_databases(&client(&server), &["silver", "serving"])
            .await
            .unwrap();

        assert_eq!(recorded, 3);
        let seen = bodies(&server).await;
        let writes = inserts(&seen);
        assert_eq!(writes.len(), 1, "one INSERT for the whole pass");
        for key in ["serving.sales", "silver.customers", "silver.orders"] {
            assert!(writes[0].contains(&format!("('{key}', 1,")), "{key}");
        }
        assert!(!writes[0].contains("silver.stable"));
    }

    #[tokio::test]
    async fn a_failed_read_of_the_columns_records_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM system.columns"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Code: 999. DB::Exception"))
            .mount(&server)
            .await;
        mount_newest(&server, &json!([])).await;
        mount_writes(&server).await;

        let result = observe_databases(&client(&server), &["silver"]).await;

        assert!(matches!(
            result,
            Err(EngineError::Failed(ChError::Server(_)))
        ));
        assert!(inserts(&bodies(&server).await).is_empty());
    }

    #[tokio::test]
    async fn a_failed_read_of_the_store_records_nothing() {
        let server = MockServer::start().await;
        mount_columns(
            &server,
            &json!([column_row("silver", "orders", "id", "UInt64")]),
        )
        .await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.table_schema_version"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Code: 999. DB::Exception"))
            .mount(&server)
            .await;
        mount_writes(&server).await;

        let result = observe_databases(&client(&server), &["silver"]).await;

        assert!(matches!(
            result,
            Err(EngineError::Failed(ChError::Server(_)))
        ));
        assert!(
            inserts(&bodies(&server).await).is_empty(),
            "a table that might already be recorded is not recorded again on a guess"
        );
    }

    /// `ClickHouse` can answer `200` and then fail mid-stream; `ChClient`
    /// turns such a body into an empty result, which `rows` would call "no
    /// rows". On the store read that would read as "nothing recorded" and
    /// every table would get version 1 again.
    #[tokio::test]
    async fn a_200_with_an_empty_body_on_the_store_read_records_nothing() {
        let server = MockServer::start().await;
        mount_columns(
            &server,
            &json!([
                column_row("silver", "orders", "id", "UInt64"),
                column_row("silver", "customers", "id", "UInt64"),
            ]),
        )
        .await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.table_schema_version"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        mount_writes(&server).await;

        let result = observe_databases(&client(&server), &["silver"]).await;

        assert!(matches!(result, Err(EngineError::NotAResult)), "{result:?}");
        assert!(
            inserts(&bodies(&server).await).is_empty(),
            "an answer that is not a result must not be taken for an empty store"
        );
    }

    #[tokio::test]
    async fn a_200_with_an_empty_body_on_the_columns_read_records_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM system.columns"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        mount_newest(&server, &json!([])).await;
        mount_writes(&server).await;

        let result = observe_databases(&client(&server), &["silver"]).await;

        assert!(matches!(result, Err(EngineError::NotAResult)), "{result:?}");
        let seen = bodies(&server).await;
        assert!(inserts(&seen).is_empty());
        assert_eq!(
            seen.len(),
            1,
            "the pass stops at the read it could not take for an answer"
        );
    }

    /// Engine error `Code: 184` (`ILLEGAL_AGGREGATION`), seen on the dev
    /// stack's `ClickHouse` 26.8: `max(version) AS version` makes the
    /// `version` in `argMax(columns, version)` the alias, so the statement
    /// is refused and no pass could ever record anything. A mock cannot
    /// refuse it, so this pins the sent text.
    #[tokio::test]
    async fn the_read_of_the_newest_versions_aliases_no_aggregate_to_a_column_name() {
        let server = MockServer::start().await;
        mount_columns(
            &server,
            &json!([column_row("silver", "orders", "id", "UInt64")]),
        )
        .await;
        mount_newest(&server, &json!([])).await;
        mount_writes(&server).await;

        observe_databases(&client(&server), &["silver"])
            .await
            .unwrap();

        let seen = bodies(&server).await;
        let read = seen
            .iter()
            .find(|b| b.contains("GROUP BY table_key"))
            .expect("the pass reads the newest version of every table");
        assert!(read.contains(" AS newest_version"), "{read}");
        assert!(read.contains(" AS newest_columns"), "{read}");
        for shadowing in [" AS version", " AS columns", " AS table_key"] {
            assert!(
                !read.contains(shadowing),
                "`{shadowing}` would shadow a column of console.table_schema_version: {read}"
            );
        }
    }

    // ── what the page reads ──

    #[tokio::test]
    async fn a_table_reads_back_its_versions_newest_first_by_its_key() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            &json!([
                {
                    "version": 1, "columns": r#"[["id","UInt64"]]"#,
                    "observed": "2026-10-01T08:00:00Z",
                },
                {
                    "version": "2", "columns": r#"[["id","UInt64"],["email","String"]]"#,
                    "observed": "2026-10-03T09:30:00Z",
                },
            ]),
        )
        .await;

        let out = for_table(&client(&server), "silver.o'rders").await;

        assert_eq!(
            out,
            vec![
                Entry {
                    version: 2,
                    at: "2026-10-03T09:30:00Z".to_owned(),
                    change: "Added email (String)".to_owned(),
                    current: true,
                },
                Entry {
                    version: 1,
                    at: "2026-10-01T08:00:00Z".to_owned(),
                    change: "First recorded with 1 column".to_owned(),
                    current: false,
                },
            ]
        );
        let seen = bodies(&server).await;
        assert!(
            seen[0].contains("WHERE table_key = 'silver.o''rders' ORDER BY version"),
            "{}",
            seen[0]
        );
        assert!(seen[0].contains("formatDateTime(observed_at, '%Y-%m-%dT%H:%i:%SZ', 'UTC')"));
    }

    #[tokio::test]
    async fn a_store_that_does_not_exist_yet_reads_as_no_versions() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404).set_body_string(
                "Code: 60. DB::Exception: Table console.table_schema_version does not exist. (UNKNOWN_TABLE)",
            ))
            .mount(&server)
            .await;
        assert!(
            for_table(&client(&server), "silver.orders")
                .await
                .is_empty()
        );

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404).set_body_string(
                "Code: 81. DB::Exception: Database console does not exist. (UNKNOWN_DATABASE)",
            ))
            .mount(&server)
            .await;
        assert!(
            for_table(&client(&server), "silver.orders")
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn any_other_failed_read_also_reads_as_no_versions() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Code: 999. DB::Exception"))
            .mount(&server)
            .await;
        assert!(
            for_table(&client(&server), "silver.orders")
                .await
                .is_empty()
        );

        // An engine nobody answers for.
        let ch = ChClient::new(
            "http://127.0.0.1:1".to_owned(),
            "default".to_owned(),
            String::new(),
        );
        assert!(for_table(&ch, "silver.orders").await.is_empty());
    }

    #[tokio::test]
    async fn a_stored_row_that_is_not_a_version_reads_as_no_versions_rather_than_a_wrong_history() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            &json!([
                { "version": 1, "columns": r#"[["id","UInt64"]]"#, "observed": "2026-10-01T08:00:00Z" },
                { "version": 2, "columns": "not json", "observed": "2026-10-02T08:00:00Z" },
            ]),
        )
        .await;
        assert!(
            for_table(&client(&server), "silver.orders")
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn an_answer_that_is_not_a_result_reads_as_no_versions() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        assert!(
            for_table(&client(&server), "silver.orders")
                .await
                .is_empty()
        );
    }

    // ── the background pass ──

    fn state_for(ch_url: &str) -> AppState {
        let mut env = HashMap::new();
        // A malformed `DATABASE_URL` boots the state with no `PostgreSQL`
        // pool, which a pass never needs.
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        env.insert("CH_URL".to_owned(), ch_url.to_owned());
        AppState::new(Config::from_map(&env).expect("a valid test Config"))
    }

    async fn until_clear(flag: &AtomicBool) {
        for _ in 0..200 {
            if !flag.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the pass never finished");
    }

    #[test]
    fn a_flag_that_is_held_cannot_be_claimed_and_stays_held() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let first = PassGuard::claim(&FLAG).expect("a free flag is claimed");
        assert!(PassGuard::claim(&FLAG).is_none());
        assert!(
            FLAG.load(Ordering::SeqCst),
            "a refused claim must not clear the flag the running pass holds"
        );
        drop(first);
        assert!(!FLAG.load(Ordering::SeqCst));
        assert!(PassGuard::claim(&FLAG).is_some());
    }

    #[test]
    fn the_flag_clears_when_the_pass_panics() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let outcome = std::panic::catch_unwind(|| {
            let _running = PassGuard::claim(&FLAG).expect("a free flag is claimed");
            panic!("the pass died");
        });
        assert!(outcome.is_err());
        assert!(!FLAG.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_pass_does_not_start_while_one_runs_and_starts_again_after_it() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let server = MockServer::start().await;
        mount_columns(&server, &json!([])).await;
        mount_writes(&server).await;
        let state = state_for(&server.uri());

        assert!(spawn_pass_on(&FLAG, &state));
        // The test runtime is single-threaded: the pass cannot have run yet.
        assert!(!spawn_pass_on(&FLAG, &state), "one pass at a time");
        until_clear(&FLAG).await;
        assert_eq!(
            bodies(&server)
                .await
                .iter()
                .filter(|b| b.contains("system.columns"))
                .count(),
            1,
            "the refused start did not run a pass of its own"
        );

        assert!(spawn_pass_on(&FLAG, &state), "the next one may start");
        until_clear(&FLAG).await;
    }

    #[tokio::test]
    async fn a_failed_pass_is_logged_changes_nothing_and_frees_the_flag() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Code: 999. DB::Exception"))
            .mount(&server)
            .await;
        let state = state_for(&server.uri());

        assert!(spawn_pass_on(&FLAG, &state));
        until_clear(&FLAG).await;

        assert!(inserts(&bodies(&server).await).is_empty());
        assert!(
            spawn_pass_on(&FLAG, &state),
            "a failed pass does not wedge the next"
        );
        until_clear(&FLAG).await;
    }

    #[tokio::test]
    async fn the_pass_asks_for_the_two_databases_the_catalog_serves() {
        let server = MockServer::start().await;
        mount_columns(&server, &json!([])).await;
        mount_writes(&server).await;
        let state = state_for(&server.uri());

        assert_eq!(observe(&state).await.unwrap(), 0);

        let seen = bodies(&server).await;
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].contains("WHERE database IN ('silver', 'serving')"),
            "{}",
            seen[0]
        );
    }

    /// `GOLD_SOURCE_SCHEMA` is the export routes' setting. The catalog
    /// detail route serves `serving.*` whatever it is, so the pass must not
    /// follow it to a database the page never reads.
    #[tokio::test]
    async fn a_non_default_gold_source_schema_does_not_change_the_databases() {
        let server = MockServer::start().await;
        mount_columns(&server, &json!([])).await;
        mount_writes(&server).await;
        for setting in ["marts", "silver"] {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
            env.insert("CH_URL".to_owned(), server.uri());
            env.insert("GOLD_SOURCE_SCHEMA".to_owned(), setting.to_owned());
            let state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
            assert_eq!(state.config.gold_source_schema, setting);

            assert_eq!(observe(&state).await.unwrap(), 0);
        }

        let seen = bodies(&server).await;
        assert_eq!(seen.len(), 2);
        for body in &seen {
            assert!(
                body.contains("WHERE database IN ('silver', 'serving')"),
                "{body}"
            );
        }
    }
}
