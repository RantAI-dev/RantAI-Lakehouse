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
//!
//! The table fingerprint cannot vouch for everything a statement reads
//! (`BI-4` B review P-B1, P-S1). A statement that calls a clock or random
//! function (`now`, `today`, `rand*`, `generateUUID*`, ...) or reads data
//! through a name no `FROM`/`JOIN` carries (`dictGet*`, `joinGet*`,
//! `IN <table>`) gets an unwatchable print, so it lives for
//! [`UNWATCHABLE_BOUND`] or the person's minutes. A clock statement is also
//! never served across a calendar-day boundary in the report time zone
//! ([`CacheScope::with_zone`]): "Today" and "This month" are
//! `toDate(now('<zone>'))`, a text that is the same every day, so without
//! the day check a nightly mart would show yesterday's "Today" for 24 hours.
//! With no zone known, a clock statement is never served from memory.
//!
//! `system.parts` is read on the node that answers the query. On a replicated
//! table a replica that has not fetched a part yet reports an older
//! fingerprint, which can only make a result look older than it is (a miss),
//! never fresher.

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
    let trimmed = raw.map(str::trim).filter(|s| !s.is_empty());
    match trimmed.map(str::parse::<u64>) {
        Some(Ok(n)) => n.min(MAX_MB_LIMIT),
        Some(Err(_)) => {
            // BI-4 B review P-S4: a typo must not pass for the default.
            tracing::warn!(
                default_mb = DEFAULT_MAX_MB,
                "RESULT_CACHE_MAX_MB is not a whole number; using the default"
            );
            DEFAULT_MAX_MB
        }
        None => DEFAULT_MAX_MB,
    }
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

impl Effective {
    /// The rule as the payload's `cacheRule` says it.
    #[must_use]
    pub fn to_json(self) -> Value {
        match self {
            Self::Off => CacheRule::Fresh.to_json(),
            Self::Default => CacheRule::Follow.to_json(),
            Self::Minutes(n) => CacheRule::Minutes(n).to_json(),
        }
    }
}

/// What a read asks to be computed fresh (`fresh=` on `GET /api/dashboard`,
/// feature decision 6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Fresh {
    /// Use what is remembered.
    #[default]
    No,
    /// `fresh=1`: every tile of the read is computed and replaces its entry.
    All,
    /// `fresh=<chart id>[,<chart id>...]`: only these charts ("Refresh this
    /// tile").
    Charts(Vec<String>),
}

impl Fresh {
    /// The query value: absent or empty is [`Self::No`], `1` or `true` is
    /// [`Self::All`], anything else is a comma list of chart ids.
    #[must_use]
    pub fn parse(raw: Option<&str>) -> Self {
        let raw = raw.map(str::trim).unwrap_or_default();
        match raw {
            "" => Self::No,
            "1" | "true" => Self::All,
            list => Self::Charts(
                list.split(',')
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .collect(),
            ),
        }
    }

    fn covers(&self, chart: Option<&str>) -> bool {
        match (self, chart) {
            (Self::All, _) => true,
            (Self::Charts(ids), Some(id)) => ids.iter().any(|x| x == id),
            _ => false,
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
    /// The statement calls a clock or random function: not served across a
    /// calendar day in the report time zone (`BI-4` B review P-B1).
    day_bound: bool,
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
    fresh: Arc<Fresh>,
    /// Whether the chart being read skips the lookup (see [`Fresh`]).
    bypass: bool,
    /// The report time zone: where a calendar day ends for a statement that
    /// calls a clock function. `None` serves such a statement never.
    zone: Option<Arc<str>>,
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
pub async fn in_chart<F: Future>(
    (id, rule): (Option<&str>, Option<CacheRule>),
    fut: F,
) -> F::Output {
    match current() {
        Some(scope) => scoped(scope.for_chart(id, rule), fut).await,
        None => fut.await,
    }
}

impl CacheScope {
    /// A scope for one request, opened by the handler that serves it. `fresh`
    /// is what the read asked to bypass: nothing is looked up for it, and what
    /// is computed replaces what was remembered. The dashboard's and the charts' own rules
    /// are added by [`Self::for_board`] and [`Self::for_chart`] once the board
    /// is read.
    #[must_use]
    pub fn new(cache: Arc<ResultCache>, install: CacheRule, fresh: Fresh) -> Self {
        Self {
            shared: Arc::new(Shared {
                cache,
                install,
                candidates: Vec::new(),
                prints: tokio::sync::Mutex::default(),
            }),
            board: CacheRule::Follow,
            chart: CacheRule::Follow,
            bypass: fresh == Fresh::All,
            fresh: Arc::new(fresh),
            zone: None,
        }
    }

    /// The same request, told the report time zone (`BI-4` B review P-B1).
    #[must_use]
    pub fn with_zone(self, zone: &str) -> Self {
        Self {
            zone: Some(Arc::from(zone)),
            ..self
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
            bypass: *self.fresh == Fresh::All,
            fresh: Arc::clone(&self.fresh),
            zone: self.zone.clone(),
        }
    }

    /// The same request, narrowed to one chart: its own rule (`None` follows
    /// the dashboard) and whether this read asked it to be fresh. `id` is
    /// `None` for a built-in tile.
    #[must_use]
    pub fn for_chart(&self, id: Option<&str>, rule: Option<CacheRule>) -> Self {
        Self {
            chart: rule.unwrap_or(CacheRule::Follow),
            bypass: self.fresh.covers(id),
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
        let Some(mut prints) = self.prints_of(ch, &names).await else {
            // Fail closed: compute fresh, store nothing.
            return uncached(compute).await;
        };
        let day_bound = add_unwatchable_reads(&mut prints, statement);
        let key = key(ENGINE, statement, settings);
        if self.bypass {
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
                    return Err(NotStored::Retry);
                };
                settle(
                    compute().await,
                    &outcome,
                    (prints, day_bound, (cache.clock)()),
                )
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
                    // two before it returns `NotStored`); the tile answers
                    // `Refused` rather than inventing a result.
                    None => Err(SpecRunFailure::Refused("The result could not be read.")),
                }
            }
            // BI-4 B review P-S2: the leader's deterministic outcome, shared.
            Err(shared) => match &*shared {
                NotStored::Answer(value) => Ok(Answer {
                    value: value.clone(),
                    computed_at: now,
                    remembered: false,
                }),
                NotStored::Failed(tile) => Err(SpecRunFailure::Tile(tile.clone())),
                // The tile this one waited for was stopped or failed on the
                // engine: that is its own run's outcome, so this tile asks the
                // engine itself.
                NotStored::Retry => {
                    let take = work.lock().unwrap_or_else(PoisonError::into_inner).take();
                    match take {
                        Some(compute) => uncached(compute).await,
                        None => Err(SpecRunFailure::Refused("The result could not be read.")),
                    }
                }
            },
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
        // BI-4 B review P-B1: a clock statement is not carried over midnight
        // of the report time zone, whatever its age.
        let same_day =
            !entry.day_bound || same_day_in_zone(self.zone.as_deref(), entry.computed_at, now);
        age < bound && same_day && entry.prints == *prints
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

/// BI-4 B review P-B1, P-S1: what the table names do not show. Each adds a
/// print `system.parts` cannot answer, which is what makes the entry
/// unwatchable (short-lived). Returns whether the statement calls a clock or
/// random function (so it is also bound to one calendar day).
fn add_unwatchable_reads(prints: &mut Prints, statement: &str) -> bool {
    let beyond = crate::sql_rewrite::reads_beyond_tables(
        statement,
        &sqlparser::dialect::ClickHouseDialect {},
    );
    if beyond.outside_from {
        prints.push(("(dictionary, join or IN table read)".to_owned(), None));
    }
    if beyond.clock_or_random {
        prints.push(("(clock or random function)".to_owned(), None));
    }
    beyond.clock_or_random
}

/// What a finished computation becomes: an entry to keep, or a [`NotStored`]
/// for the tiles that waited. The leader's own outcome goes into `outcome`
/// whole, because a follower only gets the shareable part.
fn settle(
    result: Result<Computed, SpecRunFailure>,
    outcome: &Mutex<Option<Result<Value, SpecRunFailure>>>,
    (prints, day_bound, computed_at): (Prints, bool, DateTime<Utc>),
) -> Result<Arc<CachedResult>, NotStored> {
    let mut slot = outcome.lock().unwrap_or_else(PoisonError::into_inner);
    match result {
        Ok(done) => {
            let bytes = done.value.to_string().len();
            if done.complete && bytes <= MAX_ENTRY_BYTES {
                Ok(Arc::new(CachedResult {
                    value: done.value,
                    computed_at,
                    prints,
                    day_bound,
                    bytes,
                }))
            } else {
                // BI-4 B review P-S2: a cut or over-size result is the same
                // for every tile that waited, so they share it instead of
                // asking the engine again.
                *slot = Some(Ok(done.value.clone()));
                Err(NotStored::Answer(done.value))
            }
        }
        Err(failure) => {
            // BI-4 B review P-S2: a time limit is deterministic for this
            // statement and settings, so followers get it too; a Stop or an
            // engine error is this run's own, so they ask for themselves.
            let shared = match &failure {
                SpecRunFailure::Tile(tile)
                    if tile.get("error").and_then(Value::as_str)
                        != Some(crate::query_limits::STOPPED_SENTENCE) =>
                {
                    NotStored::Failed(tile.clone())
                }
                _ => NotStored::Retry,
            };
            *slot = Some(Err(failure));
            Err(shared)
        }
    }
}

/// Whether `a` and `b` fall on one calendar day in `zone`. An absent or
/// unknown zone answers `false`: the day cannot be told, so nothing is reused.
fn same_day_in_zone(zone: Option<&str>, a: DateTime<Utc>, b: DateTime<Utc>) -> bool {
    let Some(Ok(tz)) = zone.map(jiff::tz::TimeZone::get) else {
        return false;
    };
    let day = |t: DateTime<Utc>| {
        jiff::Timestamp::from_second(t.timestamp())
            .ok()
            .map(|ts| ts.to_zoned(tz.clone()).date())
    };
    matches!((day(a), day(b)), (Some(x), Some(y)) if x == y)
}

/// Why an entry was not stored. A follower of the computation gets this
/// through moka's shared error, so only what is the same for every waiting
/// tile travels in it; the leader keeps its own real outcome.
#[derive(Debug)]
enum NotStored {
    /// The leader was stopped or failed on the engine: followers compute.
    Retry,
    /// A result that was answered but not kept (cut, over the size bound).
    Answer(Value),
    /// A tile failure that holds for every tile (the time limit).
    Failed(Value),
}

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
