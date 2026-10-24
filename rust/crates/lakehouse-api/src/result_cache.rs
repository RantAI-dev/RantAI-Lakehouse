//! Remembered dashboard results (`BI-4` part B; plan
//! `docs/superpowers/plans/2026-10-24-bi-4-caching-parallel-tiles.md`
//! decisions D3 to D6; feature page
//! `docs/core/features/result-caching-and-parallel-tiles.md`).
//!
//! # Where it sits
//!
//! `routes::support::try_run_spec_sql` calls [`CacheScope::run`] **after**
//! `policy_engine::rewrite_sql_for_roles` and the `BI-5` limit settings, and
//! before the engine call. Every tile query of every path (the tile itself, a
//! table's row count, a layer, a SQL source, a metric) reaches that one
//! function, so no caller can read around the cache (plan D3).
//!
//! The plan's `TileEngine` parameter is a request-scoped value instead: a
//! [`CacheScope`] set with [`scoped`] around the tiles of a request, read with
//! [`current`]. The reasons are the ones `BI-5` gave for its task-local run:
//! about twenty call sites would change for no gain, and the safe default is
//! better with a scope. A call made outside any scope (Query Studio, the
//! assistant's free SQL, the SQL-source preview, an export) has no scope, so it
//! is never remembered; only a path that opens a scope can be.
//!
//! # The key never crosses permissions (feature decision 4)
//!
//! The key is the SHA-256 of the **rewritten** statement, the engine name and
//! the limit settings sent with it. The rewrite has already put the person's
//! masks into the text (a masked column is a different expression) and has
//! already replaced `__principal_id__` and `__principal_tenant_ids__` with
//! the person's own values, so two people whose rules differ run different
//! statements and get different keys; two people with the same rules run the
//! same statement and may share a result, which shows them nothing they could
//! not see. There is no user id in the key because none is needed, and a key
//! made of one would stop two equal viewers sharing. The `BI-5` limits are in
//! the key (the text carries the row cap and the time cap, and the settings
//! carry the same numbers), so a result computed under one limit is never
//! served under another.
//!
//! # What is never remembered
//!
//! An error (including a time limit and a Stop), a result the row cap cut
//! (`limitHit`), a result over [`MAX_ENTRY_BYTES`], a statement whose tables
//! could not be named, and anything computed while the freshness check itself
//! failed. A run that is cancelled leaves nothing: the entry is inserted only
//! when the computation finishes, in one step.
//!
//! # Freshness (feature decision 1)
//!
//! A remembered result is used while it is younger than the rule's duration
//! (24 hours by default, or the minutes a person set) and while the
//! fingerprint of its tables is unchanged. The fingerprint is
//! `max(modification_time)` and `sum(rows)` of the active parts of each table
//! in `system.parts`, read once per request for every table the request has
//! asked about. A table that is not in `system.parts` (a view, an Iceberg
//! table engine) cannot be watched, so a result that reads one is remembered
//! for [`UNWATCHABLE_BOUND`] under the default rule, or for the person's
//! minutes when they set them. A fingerprint that cannot be read counts as
//! "changed": the result is computed fresh and not stored.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use lakehouse_bi::cache_rule::CacheRule;
use lakehouse_clickhouse::ChClient;
use lakehouse_core::ident::SqlLiteral;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::routes::support::SpecRunFailure;

/// The largest result one entry may hold (feature decision 7).
pub const MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;

/// The default memory budget in megabytes (`RESULT_CACHE_MAX_MB`).
pub const DEFAULT_MAX_MB: u64 = 256;

/// The most `RESULT_CACHE_MAX_MB` may be set to.
pub const MAX_MB_LIMIT: u64 = 4096;

/// The outer bound of every entry: no result is used after 24 hours, whatever
/// the fingerprint says.
pub const OUTER_TTL: Duration = Duration::from_hours(24);

/// How long a result is used when a table under it cannot be watched and the
/// person set no minutes (feature decision 1).
pub const UNWATCHABLE_BOUND: Duration = Duration::from_mins(10);

/// The engine name in the key. Trino tiles are not cached (plan section 4).
const ENGINE: &str = "clickhouse";

/// `RESULT_CACHE_MAX_MB`: whole megabytes, clamped to 0..=4096, where 0
/// switches remembering off. Unset, empty or unparseable gives
/// [`DEFAULT_MAX_MB`].
#[must_use]
pub fn max_mb_from(raw: Option<&str>) -> u64 {
    raw.map(str::trim)
        .and_then(|s| s.parse::<u64>().ok())
        .map_or(DEFAULT_MAX_MB, |n| n.min(MAX_MB_LIMIT))
}

/// The clock the validity check reads; tests replace it.
type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// What the rules of the three levels come to for one chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effective {
    /// "Always fresh": nothing is looked up or stored.
    Off,
    /// The signed rule: fingerprint unchanged, at most 24 hours.
    Default,
    /// The person's minutes, together with the fingerprint.
    Minutes(u32),
}

impl Effective {
    /// The narrowest rule that is set wins: chart, then dashboard, then
    /// install; none set is the default (feature decision 2).
    #[must_use]
    pub fn resolve(chart: CacheRule, board: CacheRule, install: CacheRule) -> Self {
        let narrowest = [chart, board, install]
            .into_iter()
            .find(|rule| *rule != CacheRule::Follow)
            .unwrap_or(CacheRule::Follow);
        match narrowest {
            CacheRule::Follow => Self::Default,
            CacheRule::Fresh => Self::Off,
            CacheRule::Minutes(n) => Self::Minutes(n),
        }
    }
}

/// A table named in a statement: `schema.table`, or a bare `table` of the
/// connection's default database (`sql_rewrite::canonicalize`).
type TableName = String;

/// What `system.parts` says about a table: when its newest part was written
/// and how many rows its active parts hold.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Print {
    modified: String,
    rows: String,
}

/// The fingerprint of a statement's tables, sorted by name. `None` is a table
/// `system.parts` does not list: it cannot be watched.
type Prints = Vec<(TableName, Option<Print>)>;

/// One remembered result.
#[derive(Debug)]
struct CachedResult {
    value: Value,
    computed_at: DateTime<Utc>,
    prints: Prints,
    bytes: usize,
}

impl CachedResult {
    fn unwatchable(&self) -> bool {
        self.prints.iter().any(|(_, print)| print.is_none())
    }
}

/// What a tile query answered and where the answer came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    /// `{"columns", "rows"}`.
    pub value: Value,
    /// When the engine computed it.
    pub computed_at: DateTime<Utc>,
    /// Whether it came from memory.
    pub remembered: bool,
}

/// What the engine run produced, before the cache decides whether to keep it.
#[derive(Debug)]
pub struct Computed {
    /// `{"columns", "rows"}`, plus `limitHit` when the row cap cut it.
    pub value: Value,
    /// `false` for a result the row cap cut: it is answered but not kept.
    pub complete: bool,
}

/// The store: a weighted, bounded, in-process map from key to result.
pub struct ResultCache {
    entries: Option<moka::future::Cache<[u8; 32], Arc<CachedResult>>>,
    clock: Clock,
}

impl std::fmt::Debug for ResultCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultCache")
            .field("enabled", &self.entries.is_some())
            .finish_non_exhaustive()
    }
}

impl ResultCache {
    /// A store of at most `max_mb` megabytes; 0 remembers nothing.
    #[must_use]
    pub fn new(max_mb: u64) -> Self {
        Self::with_clock(max_mb, Arc::new(Utc::now))
    }

    /// As [`Self::new`], reading time from `clock` (tests move it).
    #[must_use]
    pub fn with_clock(max_mb: u64, clock: Clock) -> Self {
        // Weights are whole kibibytes, so the 4,096 MB ceiling fits the
        // weigher's `u32`.
        let entries = (max_mb > 0).then(|| {
            moka::future::Cache::builder()
                .max_capacity(max_mb.saturating_mul(1024))
                .weigher(|_key: &[u8; 32], value: &Arc<CachedResult>| {
                    u32::try_from(value.bytes / 1024 + 1).unwrap_or(u32::MAX)
                })
                .time_to_live(OUTER_TTL)
                .build()
        });
        Self { entries, clock }
    }

    /// Whether this store keeps anything (`RESULT_CACHE_MAX_MB` above 0).
    #[cfg(test)]
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.entries.is_some()
    }

    /// Entries held, after pending maintenance. Tests and `/health` style
    /// checks only.
    #[cfg(test)]
    pub async fn entry_count(&self) -> u64 {
        match &self.entries {
            Some(entries) => {
                entries.run_pending_tasks().await;
                entries.entry_count()
            }
            None => 0,
        }
    }
}

/// The key of a statement: its rewritten text, the engine and the settings it
/// is sent with, each length-prefixed so no two inputs share a digest by
/// concatenation.
#[must_use]
pub fn key(engine: &str, statement: &str, settings: &[(&str, u64)]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    let mut part = |bytes: &[u8]| {
        hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
        hasher.update(bytes);
    };
    part(engine.as_bytes());
    part(statement.as_bytes());
    for (name, value) in settings {
        part(name.as_bytes());
        part(&value.to_be_bytes());
    }
    hasher.finalize().into()
}

/// The request's view of the store: the rules, whether this read bypasses
/// what is remembered, and the fingerprints read so far.
#[derive(Clone)]
pub struct CacheScope {
    shared: Arc<Shared>,
    board: CacheRule,
    chart: CacheRule,
    fresh: bool,
}

struct Shared {
    cache: Arc<ResultCache>,
    install: CacheRule,
    /// Tables the request is known to read; the first fingerprint read asks
    /// about all of them at once (one query per request, plan D6).
    candidates: Vec<TableName>,
    prints: tokio::sync::Mutex<HashMap<TableName, Option<Print>>>,
}

tokio::task_local! {
    static SCOPE: CacheScope;
}

/// The scope of the request being served, if one was opened.
#[must_use]
pub fn current() -> Option<CacheScope> {
    SCOPE.try_with(Clone::clone).ok()
}

/// Run `fut` with `scope` as the current scope.
pub async fn scoped<F: Future>(scope: CacheScope, fut: F) -> F::Output {
    SCOPE.scope(scope, fut).await
}

/// Run `fut` under the current scope narrowed to one chart's own rule; with no
/// scope open it just runs (and so remembers nothing).
pub async fn in_chart<F: Future>(chart: Option<CacheRule>, fut: F) -> F::Output {
    match current() {
        Some(scope) => scoped(scope.for_chart(chart), fut).await,
        None => fut.await,
    }
}

impl CacheScope {
    /// A scope for one request, opened by the handler that serves it. `fresh`
    /// is `fresh=1` on the read: nothing is looked up, and what is computed
    /// replaces what was remembered. The dashboard's and the charts' own rules
    /// are added by [`Self::for_board`] and [`Self::for_chart`] once the board
    /// is read.
    #[must_use]
    pub fn new(cache: Arc<ResultCache>, install: CacheRule, fresh: bool) -> Self {
        Self {
            shared: Arc::new(Shared {
                cache,
                install,
                candidates: Vec::new(),
                prints: tokio::sync::Mutex::default(),
            }),
            board: CacheRule::Follow,
            chart: CacheRule::Follow,
            fresh,
        }
    }

    /// The same request, narrowed to the dashboard's own rule (`None`
    /// follows the install), told which tables the dashboard reads so their
    /// fingerprints are read together. Call it before any tile runs: it starts
    /// the request's fingerprints afresh.
    #[must_use]
    pub fn for_board(&self, board: Option<CacheRule>, candidates: Vec<TableName>) -> Self {
        Self {
            shared: Arc::new(Shared {
                cache: Arc::clone(&self.shared.cache),
                install: self.shared.install,
                candidates,
                prints: tokio::sync::Mutex::default(),
            }),
            board: board.unwrap_or(CacheRule::Follow),
            chart: CacheRule::Follow,
            fresh: self.fresh,
        }
    }

    /// The same request, narrowed to one chart's own rule (`None` follows the
    /// dashboard).
    #[must_use]
    pub fn for_chart(&self, chart: Option<CacheRule>) -> Self {
        Self {
            chart: chart.unwrap_or(CacheRule::Follow),
            ..self.clone()
        }
    }

    /// The rule in force for the current chart.
    #[must_use]
    pub fn rule(&self) -> Effective {
        Effective::resolve(self.chart, self.board, self.shared.install)
    }

    /// Answer a statement from memory when a remembered result is still valid,
    /// otherwise by `compute`, remembering the result when it may be.
    ///
    /// `tables` are the tables the rewritten statement reads (`None` when they
    /// could not be named: the statement then runs and is not stored).
    ///
    /// # Errors
    ///
    /// Whatever `compute` fails with. A failure is never stored.
    pub(crate) async fn run<F, Fut>(
        &self,
        ch: &ChClient,
        (statement, settings): (&str, &[(&str, u64)]),
        tables: Option<Vec<TableName>>,
        compute: F,
    ) -> Result<Answer, SpecRunFailure>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<Computed, SpecRunFailure>> + Send,
    {
        let cache = &self.shared.cache;
        let now = (cache.clock)();
        let uncached = |compute: F| async move {
            compute().await.map(|done| Answer {
                value: done.value,
                computed_at: now,
                remembered: false,
            })
        };
        let (Some(entries), Some(tables)) = (&cache.entries, tables) else {
            return uncached(compute).await;
        };
        if self.rule() == Effective::Off {
            return uncached(compute).await;
        }
        let mut names = tables;
        names.sort();
        names.dedup();
        let Some(prints) = self.prints_of(ch, &names).await else {
            // Fail closed: compute fresh, store nothing.
            return uncached(compute).await;
        };
        let key = key(ENGINE, statement, settings);
        if self.fresh {
            entries.invalidate(&key).await;
        } else if let Some(entry) = entries.get(&key).await {
            if self.valid(&entry, &prints, now) {
                return Ok(Answer {
                    value: entry.value.clone(),
                    computed_at: entry.computed_at,
                    remembered: true,
                });
            }
            entries.invalidate(&key).await;
        }

        // One computation per key: a second tile with the same statement waits
        // for the first and shares its entry.
        let leader = AtomicBool::new(false);
        let work = Mutex::new(Some(compute));
        let outcome: Mutex<Option<Result<Value, SpecRunFailure>>> = Mutex::new(None);
        let stored = entries
            .try_get_with(key, async {
                leader.store(true, Ordering::SeqCst);
                let Some(compute) = work.lock().unwrap_or_else(PoisonError::into_inner).take()
                else {
                    return Err(NotStored);
                };
                match compute().await {
                    Ok(done) => {
                        let bytes = done.value.to_string().len();
                        if done.complete && bytes <= MAX_ENTRY_BYTES {
                            Ok(Arc::new(CachedResult {
                                value: done.value,
                                computed_at: (cache.clock)(),
                                prints,
                                bytes,
                            }))
                        } else {
                            *outcome.lock().unwrap_or_else(PoisonError::into_inner) =
                                Some(Ok(done.value));
                            Err(NotStored)
                        }
                    }
                    Err(failure) => {
                        *outcome.lock().unwrap_or_else(PoisonError::into_inner) =
                            Some(Err(failure));
                        Err(NotStored)
                    }
                }
            })
            .await;
        match stored {
            Ok(entry) => Ok(Answer {
                value: entry.value.clone(),
                computed_at: entry.computed_at,
                remembered: entry.computed_at < now,
            }),
            Err(_) if leader.load(Ordering::SeqCst) => {
                match outcome
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()
                {
                    Some(Ok(value)) => Ok(Answer {
                        value,
                        computed_at: now,
                        remembered: false,
                    }),
                    Some(Err(failure)) => Err(failure),
                    // Unreachable by construction (the leader sets one of the
                    // two before it returns `NotStored`); computing again is
                    // the safe answer rather than inventing a result.
                    None => Err(SpecRunFailure::Refused("The result could not be read.")),
                }
            }
            // The tile this one waited for did not produce a result to keep
            // (it failed or was cut); this tile asks the engine itself.
            Err(_) => {
                let take = work.lock().unwrap_or_else(PoisonError::into_inner).take();
                match take {
                    Some(compute) => uncached(compute).await,
                    None => Err(SpecRunFailure::Refused("The result could not be read.")),
                }
            }
        }
    }

    /// Whether `entry` may still be used now (feature decision 1).
    fn valid(&self, entry: &CachedResult, prints: &Prints, now: DateTime<Utc>) -> bool {
        let Ok(age) = (now - entry.computed_at).to_std() else {
            // The clock went backwards: the age cannot be known.
            return false;
        };
        let bound = match self.rule() {
            Effective::Off => return false,
            Effective::Default if entry.unwatchable() => UNWATCHABLE_BOUND,
            Effective::Default => OUTER_TTL,
            Effective::Minutes(n) => Duration::from_secs(u64::from(n) * 60),
        };
        age < bound && entry.prints == *prints
    }

    /// The fingerprint of `names`, or `None` when it cannot be read.
    async fn prints_of(&self, ch: &ChClient, names: &[TableName]) -> Option<Prints> {
        let mut memo = self.shared.prints.lock().await;
        let mut missing: Vec<&TableName> = names
            .iter()
            .filter(|name| !memo.contains_key(*name))
            .collect();
        if !missing.is_empty() {
            for candidate in &self.shared.candidates {
                if !memo.contains_key(candidate) && !missing.contains(&candidate) {
                    missing.push(candidate);
                }
            }
            let read = read_prints(ch, &missing).await?;
            memo.extend(read);
        }
        Some(
            names
                .iter()
                .map(|name| (name.clone(), memo.get(name).cloned().flatten()))
                .collect(),
        )
    }
}

/// Why an entry was not stored; carries nothing because the leader keeps the
/// real outcome.
#[derive(Debug)]
struct NotStored;

/// One `system.parts` read for `names`. A failed read is logged (the text
/// stays in the log, never in a response) and answers `None`.
async fn read_prints(
    ch: &ChClient,
    names: &[&TableName],
) -> Option<Vec<(TableName, Option<Print>)>> {
    let condition = names
        .iter()
        .map(|name| match name.split_once('.') {
            Some((db, table)) => format!(
                "(database = {} AND table = {})",
                SqlLiteral::from(db),
                SqlLiteral::from(table)
            ),
            None => format!(
                "(database = currentDatabase() AND table = {})",
                SqlLiteral::from(name.as_str())
            ),
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    let sql = format!(
        "SELECT database, table, toString(max(modification_time)) AS modified, \
         toString(sum(rows)) AS row_count FROM system.parts \
         WHERE active AND ({condition}) GROUP BY database, table"
    );
    let rows = match ch.rows(&sql, None).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "table fingerprints could not be read; results are not remembered");
            return None;
        }
    };
    let text = |row: &serde_json::Map<String, Value>, key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    Some(
        names
            .iter()
            .map(|name| {
                let (db, table) = match name.split_once('.') {
                    Some((db, table)) => (Some(db), table),
                    None => (None, name.as_str()),
                };
                let mut found = rows.iter().filter(|row| {
                    text(row, "table") == table && db.is_none_or(|db| text(row, "database") == db)
                });
                // Two rows for one bare name would be two databases: not
                // knowable, so not watchable.
                let print = match (found.next(), found.next()) {
                    (Some(row), None) => Some(Print {
                        modified: text(row, "modified"),
                        rows: text(row, "row_count"),
                    }),
                    _ => None,
                };
                ((*name).clone(), print)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests;
