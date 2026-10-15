//! BLD-2: how fresh a pipeline's output has to be, and the ONE place that
//! decides whether it is currently fresh enough (plan D4/D5).
//!
//! # The shape of the problem
//!
//! A pipeline with a target lag runs when one of its INPUTS changed and the
//! lag would otherwise be missed. That is two questions — "did an input
//! change?" and "is the change old enough to be worth a run?" — and a
//! third, easier to get wrong: "has the run we already asked for landed
//! yet?". All three live here, in one function,
//! [`run_lag_check`], so the orchestrator's sensor (plan D6) has exactly one
//! thing to call and cannot reimplement a piece of the arithmetic.
//!
//! # Why the tokens, not the timestamps
//!
//! An input's token is a value that changes when — and only when — its
//! CONTENT changes (plan D4): a Bronze (Iceberg) table's current snapshot
//! id, a Silver/Gold (`ClickHouse`) table's merge-proof content fingerprint
//! (`DATA-10`). Both are already read by this crate for other reasons
//! ([`lakehouse_iceberg::rest::load_table_summary`] in `routes::alerts`,
//! [`crate::gold_export_history::mart_content_fingerprint`] in
//! `routes::gold`), so this feature adds no new access path — it reuses the
//! two readers that already exist rather than writing a third way to ask
//! "did this table change".
//!
//! # Why "not measured" is its own answer
//!
//! An input this process cannot read makes the whole check `not_measured`:
//! no run is requested on a guess, and the page says why. `not_measured` is
//! NOT `met` (AGENTS.md principle 2) — a pipeline nobody can measure is
//! not a pipeline that is proven fresh.
//!
//! # One clock
//!
//! [`run_lag_check`] takes `now` as a parameter. Every test that pins a lag
//! to a number therefore pins the clock to the same number, instead of
//! sleeping and hoping.

use std::collections::BTreeMap;

use lakehouse_core::ApiError;
use lakehouse_dagster::{DgClient, DgError};
use lakehouse_store::pipelines::{self, LagStateUpdate, Pipeline, TargetLag};
use lakehouse_store::{PgPool, StoreError};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

/// How often the orchestrator asks (plan D6): one sensor tick per pipeline
/// with a target lag. Two things depend on this constant and must move
/// together: the sensor's `minimum_interval_seconds`, and the "stale" rule
/// that turns a state row into `not_measured` after three missed checks
/// (plan D8).
pub const CHECK_INTERVAL_SECONDS: i64 = 30;

/// The run budget's fallback: how long a pipeline is assumed to take when
/// it has never run successfully and there is no measurement to use
/// (plan D5 step 4).
const ASSUMED_FIRST_RUN_SECONDS: i64 = 60;

/// How many recent successful runs the budget is the MEDIAN of (plan D5
/// step 4). A median, not a mean, so one slow run does not push every
/// later decision; five, so a pipeline that has been running regularly has
/// a window but a new one is not held back by a single sample.
const BUDGET_RUNS: usize = 5;

/// One table a pipeline reads: its source, or the target of a pipeline it
/// depends on (plan D4). Built from the stored `"<zone>.<table>"`
/// location, the same shape `pipeline_definition.source`/`.target` store.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InputTable {
    /// The zone (`bronze`, `silver`, `gold`, …).
    pub zone: String,
    /// The table within the zone.
    pub table: String,
}

impl InputTable {
    /// Parse a stored `"<zone>.<table>"` location. `None` for a location
    /// with no `.` in it: a row this code base did not write, which is
    /// never guessed at (fail closed — an unparseable input makes the check
    /// `not_measured`, it does not become an input that silently never
    /// changes).
    #[must_use]
    pub fn parse(location: &str) -> Option<Self> {
        let (zone, table) = location.split_once('.')?;
        if zone.is_empty() || table.is_empty() {
            return None;
        }
        Some(Self {
            zone: zone.to_owned(),
            table: table.to_owned(),
        })
    }

    /// The stable key this input is stored under in the token map, and the
    /// one the `reason` string names when an input cannot be read.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}.{}", self.zone, self.table)
    }
}

/// The tokens of every input of one pipeline: `{"<zone>.<table>": "<token>"}`
/// (plan D4). A `BTreeMap`, not a JSON object, so the equality a decision
/// rests on (`tokens == consumed_tokens`) does not depend on key order, and
/// so the run key's hash is stable across checks.
pub type TokenMap = BTreeMap<String, String>;

/// Why an input's token could not be read. Both variants mean the SAME
/// thing to the decision (`not_measured`, no run requested); they are kept
/// apart only so the reason the page shows can say which it was.
///
/// The messages are fixed strings written here: a `ClickHouse` or catalog
/// error's own text never reaches one (AGENTS.md principle 4 — see
/// [`crate::upstream_error`] for where the raw text goes instead).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    /// The upstream refused or could not be reached.
    Unreadable,
    /// The table is not there at all, so it cannot have changed.
    Missing,
}

impl TokenError {
    /// The wording the API and the page both show. Never upstream text.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::Unreadable => "an input could not be read",
            Self::Missing => "an input table does not exist",
        }
    }
}

/// Reads one input's change token. A trait, not a bare function, because the
/// production reader needs the process's `Iceberg`/`ClickHouse` clients
/// while a test needs to hand back a fixed table — and because plan D4's
/// rule ("an input that cannot be read fails the whole check") is a rule
/// about this boundary, so the boundary is worth naming.
#[async_trait::async_trait]
pub trait LagTokenReader: Send + Sync {
    /// The token for `input`, or `None` when the table does not exist.
    ///
    /// # Errors
    ///
    /// [`TokenError`] when the input could not be read for any other
    /// reason; both kinds fail the whole check.
    async fn token(&self, input: &InputTable) -> Result<Option<String>, TokenError>;

    /// Read every input, failing on the FIRST one that cannot be read.
    ///
    /// # Errors
    ///
    /// [`TokenError`] from the first unreadable input. One unreadable input
    /// is enough: a pipeline whose upstream table we cannot see cannot be
    /// honestly called fresh, and reading the rest would not change that.
    async fn tokens(&self, inputs: &[InputTable]) -> Result<TokenMap, TokenError> {
        let mut out = TokenMap::new();
        for input in inputs {
            let Some(token) = self.token(input).await? else {
                return Err(TokenError::Missing);
            };
            out.insert(input.key(), token);
        }
        Ok(out)
    }
}

/// The production [`LagTokenReader`]: a Bronze input's current Iceberg
/// snapshot id, and a Silver/Gold input's `ClickHouse` content fingerprint.
///
/// The zone decides the reader, which is the same rule the orchestrator's
/// own `_resolve_source` applies when it reads a pipeline's source
/// (`authored_factory.py`: `bronze` goes through the `DataLakeCatalog`
/// database, everything else is a plain `ClickHouse` table). Two readers,
/// not one: a Bronze table's authoritative change marker is its Iceberg
/// snapshot, and a `ClickHouse` fingerprint over an Iceberg table is not
/// possible — it is not a `ClickHouse` table at all.
pub struct StateTokenReader<'a> {
    /// The process state holding both clients.
    pub(crate) state: &'a crate::state::AppState,
}

#[async_trait::async_trait]
impl LagTokenReader for StateTokenReader<'_> {
    async fn token(&self, input: &InputTable) -> Result<Option<String>, TokenError> {
        if input.zone == BRONZE_ZONE {
            return iceberg_snapshot_token(self.state, input).await;
        }
        clickhouse_fingerprint_token(self.state, input).await
    }
}

/// The zone whose tables live in the Iceberg catalog rather than in
/// `ClickHouse`. Named once because `authored_factory.py`'s
/// `_resolve_source` and this reader must agree on it — they are two
/// languages reading the same stored `source_zone`.
pub const BRONZE_ZONE: &str = "bronze";

async fn iceberg_snapshot_token(
    state: &crate::state::AppState,
    input: &InputTable,
) -> Result<Option<String>, TokenError> {
    let client = crate::lakehouse_catalog::client(state)
        .await
        .map_err(|err| {
            // `CatalogAccessError` has no `Display` (it wraps a token error or
            // a `RestError`); its `Debug` names which of the two it was, and
            // goes to the log through `report`, never to a response.
            crate::upstream_error::report(
                &crate::upstream_error::CATALOG,
                crate::upstream_error::Class::Unavailable,
                &format!("{err:?}"),
            );
            TokenError::Unreadable
        })?;
    let namespace = iceberg::NamespaceIdent::from_strs([input.zone.as_str()])
        .map_err(|_| TokenError::Missing)?;
    let ident = iceberg::TableIdent::new(namespace, input.table.clone());
    match lakehouse_iceberg::rest::load_table_summary(&client, &ident).await {
        // A table with no snapshot yet has changed zero times; that is a
        // real token (an empty table that later gets rows will move away
        // from it), so it is NOT `None`/Missing.
        Ok(summary) => Ok(Some(format!(
            "iceberg:{}",
            token_value(summary.current_snapshot_id)
        ))),
        Err(lakehouse_iceberg::rest::RestError::NotFound) => Ok(None),
        Err(err) => {
            crate::upstream_error::report(
                &crate::upstream_error::CATALOG,
                crate::upstream_error::Class::Unavailable,
                &err.to_string(),
            );
            Err(TokenError::Unreadable)
        }
    }
}

/// An absent snapshot id renders as `-` rather than as a null: the token is
/// a string the pipeline compares by equality, and `None` here means "no
/// snapshots yet", which is a state, not a missing value.
fn token_value(snapshot_id: Option<i64>) -> String {
    snapshot_id.map_or_else(|| "-".to_owned(), |id| id.to_string())
}

async fn clickhouse_fingerprint_token(
    state: &crate::state::AppState,
    input: &InputTable,
) -> Result<Option<String>, TokenError> {
    match crate::gold_export_history::mart_content_fingerprint(
        &state.clickhouse,
        &input.zone,
        &input.table,
    )
    .await
    {
        Ok(Some(fp)) => Ok(Some(format!("fp:{fp}"))),
        // `mart_content_fingerprint`'s own doc: `None` means "not in
        // `system.tables`", which for an input is a table that does not
        // exist — not an unreadable one.
        Ok(None) => Ok(None),
        Err(err) => {
            let _failure = crate::upstream_error::report_ch(&crate::upstream_error::DATABASE, &err);
            Err(TokenError::Unreadable)
        }
    }
}

/// What the orchestrator should do after a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Launch the job named by [`LagCheckOutcome::job_name`] with the run
    /// key in the same struct.
    Run,
    /// Do nothing this tick. Either nothing changed, a run we already asked
    /// for is still outstanding, or nothing could be measured.
    Wait,
}

/// How fresh the pipeline's output is right now (plan D8's vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LagStatus {
    /// Current lag is within the target.
    Met,
    /// Current lag is past the target.
    Behind,
    /// Nothing could be measured — an input could not be read, or no check
    /// has run recently. NEVER reported as `met`.
    NotMeasured,
}

impl LagStatus {
    /// The wire word. The fixed vocabulary the `pipeline_lag_state` CHECK
    /// constraint enforces and the console contract uses.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Met => "met",
            Self::Behind => "behind",
            Self::NotMeasured => "not_measured",
        }
    }
}

/// The answer one check gives (plan D5 step 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LagCheckOutcome {
    /// Run or wait.
    pub decision: Decision,
    /// The `Dagster` run key to launch with, when the decision is
    /// [`Decision::Run`]; the outstanding request's key when one is still
    /// in flight; `None` when there is no request at all. It is
    /// `lag:<pipeline id>:<12 hex chars of the token map's hash>`.
    pub run_key: Option<String>,
    /// The job to launch — the pipeline's own `authored__<id>` job, or an
    /// upstream pipeline's when plan D7's downstream rule moved the
    /// decision. Sent back so the sensor does not have to re-derive which
    /// job the decision belongs to.
    pub job_name: String,
    /// The status the page shows.
    pub status: LagStatus,
    /// Seconds behind, when it could be measured. `None` when
    /// [`Self::status`] is [`LagStatus::NotMeasured`], and when nothing is
    /// pending (a pipeline with no change is not "0 seconds behind" — it
    /// has no lag to report).
    pub lag_seconds: Option<i64>,
    /// The target in seconds, when the pipeline has a numeric one.
    pub target_seconds: Option<i32>,
    /// The streak the row now holds, for the plan D9 alert.
    pub behind_streak: i32,
    /// Why the status is what it is, in fixed words (plan D8's `reason`).
    pub reason: Option<String>,
}

/// The target lag's own value, split for the decision: a numeric target in
/// seconds, and whether this pipeline is `downstream` (plan D7), which has
/// a target but none of its own.
fn target_of(pipeline: &Pipeline) -> (Option<i32>, bool) {
    match pipeline.target_lag {
        Some(TargetLag::Lag { minutes }) => (Some(minutes.saturating_mul(60)), false),
        Some(TargetLag::Downstream) => (None, true),
        None => (None, false),
    }
}

/// The inputs of `pipeline` (plan D4): its own source, plus the target of
/// every pipeline in `depends_on` that resolves to an authored pipeline.
///
/// A `depends_on` entry naming a `Dagster`-native job is skipped rather
/// than failing the check: such a job has no `pipeline_definition` row, so
/// there is no stored target to watch, and the pipeline already follows it
/// through the `_after` sensor path that plan D3 leaves in place for a
/// pipeline with no lag of its own.
///
/// # Errors
///
/// Returns [`StoreError`] if the dependency query fails.
pub async fn inputs_of(pool: &PgPool, pipeline: &Pipeline) -> Result<Vec<InputTable>, StoreError> {
    let mut inputs = Vec::new();
    let Some(source) = InputTable::parse(&pipeline.source) else {
        return Err(StoreError::Database(sqlx::Error::Protocol(
            "pipeline_definition.source is not a <zone>.<table> location".to_owned(),
        )));
    };
    inputs.push(source);
    for (id, target) in pipelines::targets_for(pool, &pipeline.depends_on).await? {
        if let Some(input) = InputTable::parse(&target) {
            inputs.push(input);
        } else {
            tracing::warn!(
                pipeline = %id,
                "target lag: an upstream pipeline's stored target is not a <zone>.<table> location; \
                 it is not watched as an input"
            );
        }
    }
    inputs.sort();
    inputs.dedup();
    Ok(inputs)
}

/// `lag:<pipeline id>:<hash>` — the run key a requested run carries, so the
/// next check can find it (plan D5 step 6). The hash is over the canonical
/// token map, so the SAME change always produces the same key (a retry
/// cannot fan out into two runs) and a DIFFERENT change produces a
/// different one (a genuinely new run is not mistaken for the old
/// request's).
fn run_key_for(pipeline_id: &str, tokens: &TokenMap) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pipeline_id.as_bytes());
    for (key, token) in tokens {
        hasher.update(key.as_bytes());
        hasher.update(b"=");
        hasher.update(token.as_bytes());
        hasher.update(b";");
    }
    let digest = hasher.finalize();
    // Six bytes of digest = 12 hex characters: enough that two different
    // token maps cannot collide in practice, short enough to stay readable
    // in a `Dagster` tag.
    let mut hex = String::with_capacity(12);
    for byte in &digest[..6] {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    format!("lag:{pipeline_id}:{hex}")
}

/// The JSON document stored in `consumed_tokens`/`requested_tokens`.
fn tokens_to_json(tokens: &TokenMap) -> Value {
    let mut map = serde_json::Map::new();
    for (key, token) in tokens {
        map.insert(key.clone(), Value::String(token.clone()));
    }
    Value::Object(map)
}

/// The inverse of [`tokens_to_json`]. `None` when the stored value is not
/// the object this code writes (a hand-edited row, a future format): an
/// unreadable stored token map is treated as "never consumed", which makes
/// the check ask for a run — the same answer a first-ever check gives, and
/// the safe one, rather than reporting a fresh pipeline nobody measured.
fn tokens_from_json(value: Option<&Value>) -> Option<TokenMap> {
    let object = value?.as_object()?;
    let mut out = TokenMap::new();
    for (key, token) in object {
        out.insert(key.clone(), token.as_str()?.to_owned());
    }
    Some(out)
}

/// The run budget: the median duration of the pipeline's last
/// [`BUDGET_WINDOW_RUNS`] successful runs, plus the check interval, or
/// [`ASSUMED_FIRST_RUN_SECONDS`] + the interval when there is nothing to
/// measure (plan D5 step 4).
///
/// The check interval is added because a change noticed by the NEXT tick
/// still costs one interval: without it, a pipeline whose runs take as long
/// as its target would be asked to run again before the first one could
/// have landed.
///
/// # Errors
///
/// Returns [`DgError`] if the run list cannot be read. The caller turns
/// this into a skipped tick rather than a decision on a guessed duration.
async fn run_budget_seconds(dagster: &DgClient, job_name: &str) -> Result<i64, DgError> {
    let runs = dagster
        .list_runs_for_job(
            job_name,
            #[allow(
                clippy::cast_possible_truncation,
                reason = "BUDGET_RUNS is the constant 5"
            )]
            u32::try_from(BUDGET_RUNS).unwrap_or(u32::MAX),
        )
        .await?;
    let mut durations: Vec<i64> = runs
        .iter()
        .filter(|r| r.status == "SUCCESS")
        .filter_map(|r| {
            let start = r.start_time?;
            let end = r.end_time?;
            #[allow(
                clippy::cast_possible_truncation,
                reason = "a run's wall-clock duration in whole seconds is far inside i64"
            )]
            let seconds = (end - start) as i64;
            (seconds > 0).then_some(seconds)
        })
        .collect();
    let median = if durations.is_empty() {
        ASSUMED_FIRST_RUN_SECONDS
    } else {
        durations.sort_unstable();
        let middle = durations.len() / 2;
        if durations.len().is_multiple_of(2) {
            // An even window has no middle element; the lower of the two is
            // used, so the budget errs toward running sooner rather than
            // later (a later run risks missing the target; an earlier one
            // costs at most one extra run inside the batching window).
            durations[middle - 1]
        } else {
            durations[middle]
        }
    };
    Ok(median + CHECK_INTERVAL_SECONDS)
}

/// What the previous check left behind, plus what this check adds.
struct CheckState {
    consumed_tokens: Option<TokenMap>,
    consumed_at: Option<OffsetDateTime>,
    pending_since: Option<OffsetDateTime>,
    requested_run_key: Option<String>,
    requested_tokens: Option<TokenMap>,
}

impl CheckState {
    fn from_row(row: &pipelines::PipelineLagState) -> Self {
        Self {
            consumed_tokens: tokens_from_json(row.consumed_tokens.as_ref()),
            consumed_at: row.consumed_at,
            pending_since: row.pending_since,
            requested_run_key: row.requested_run_key.clone(),
            requested_tokens: tokens_from_json(row.requested_tokens.as_ref()),
        }
    }
}

/// Whether the requested run has already landed, and what that does to the
/// state.
///
/// * `SUCCESS` → the tokens it consumed become `consumed_tokens`, so the
///   next check compares against them and finds nothing to do.
/// * `FAILURE`/`CANCELED` → the request is cleared and nothing is
///   consumed: the next check sees the same difference and may ask again.
///   (The pipeline's own failure alert and `maxRetries` are what cover a
///   run that keeps failing; this loop must not spin.)
/// * still running, or not created yet → `true`, and the caller waits.
/// * the lookup itself failed → an error the caller propagates, because a
///   tick that cannot see Dagster must not decide anything (fail closed:
///   the alternative is a second run for the same change).
async fn settle_request(dagster: &DgClient, state: &mut CheckState) -> Result<bool, DgError> {
    let Some(run_key) = state.requested_run_key.clone() else {
        return Ok(false);
    };
    let Some(run) = dagster.find_run_by_run_key(&run_key).await? else {
        // Requested, but Dagster has no such run yet. This is the normal
        // state for the first ticks after a request (the sensor has not
        // launched it). Waiting is the fail-closed choice.
        tracing::info!(%run_key, "target lag: requested run not created yet; waiting");
        return Ok(true);
    };
    match run.status.as_str() {
        "SUCCESS" => {
            state.consumed_tokens = state.requested_tokens.take();
            state.consumed_at = run
                .start_time
                .and_then(|seconds| {
                    // A `Dagster` timestamp is whole seconds in practice;
                    // the truncation is deliberate, not a silent cast.
                    #[allow(
                        clippy::cast_possible_truncation,
                        reason = "Dagster run start times are unix seconds; the fractional part is dropped on purpose"
                    )]
                    let whole = seconds as i64;
                    OffsetDateTime::from_unix_timestamp(whole).ok()
                });
            state.requested_run_key = None;
            state.requested_tokens = None;
            Ok(false)
        }
        "FAILURE" | "CANCELED" | "CANCELING" => {
            tracing::info!(%run_key, status = %run.status, "target lag: requested run did not succeed; the request is cleared");
            state.requested_run_key = None;
            state.requested_tokens = None;
            Ok(false)
        }
        _ => Ok(true),
    }
}

/// One lag check: read the inputs, settle any outstanding run, decide, and
/// write the whole outcome in one statement (plan D5).
///
/// `inputs` is the caller's resolved [`inputs_of`] list, passed in so this
/// function stays free of store reads beyond the state row itself.
///
/// # Errors
///
/// [`ApiError::Unavailable`] when the orchestrator cannot be asked about
/// the outstanding run (the tick is skipped; the next one asks again),
/// [`StoreError`] (as an internal error) when the state row cannot be read
/// or written.
pub async fn run_lag_check(
    pool: &PgPool,
    dagster: &DgClient,
    reader: &impl LagTokenReader,
    pipeline: &Pipeline,
    inputs: &[InputTable],
    now: OffsetDateTime,
) -> Result<LagCheckOutcome, ApiError> {
    let (target_seconds, _downstream) = target_of(pipeline);
    let job_name = crate::routes::authored_pipelines::job_name(&pipeline.id);
    let stored = pipelines::get_lag_state(pool, &pipeline.id)
        .await
        .map_err(|err| {
            tracing::error!(%err, pipeline = %pipeline.id, "target lag: state row unreadable");
            crate::upstream_error::report(
                &crate::upstream_error::STORE,
                crate::upstream_error::Class::Unavailable,
                &err.to_string(),
            );
            ApiError::Internal("the request could not be completed".to_owned())
        })?;
    let mut state = stored.as_ref().map_or_else(
        || CheckState {
            consumed_tokens: None,
            consumed_at: None,
            pending_since: None,
            requested_run_key: None,
            requested_tokens: None,
        },
        CheckState::from_row,
    );

    let outstanding = settle_request(dagster, &mut state).await.map_err(|err| {
        tracing::warn!(%err, pipeline = %pipeline.id, "target lag: orchestrator unreachable; skipping this tick");
        crate::upstream_error::report(
            &crate::upstream_error::ORCHESTRATOR_UNREACHABLE,
            crate::upstream_error::Class::Unavailable,
            &err.to_string(),
        );
        ApiError::Unavailable("the orchestrator is unavailable".to_owned())
    })?;

    let outcome = decide(
        dagster,
        reader,
        pipeline,
        &mut state,
        inputs,
        outstanding,
        target_seconds,
        &job_name,
        now,
    )
    .await?;

    // Plan D5 step 6: +1 while behind, 0 otherwise — so the alert (which
    // fires when the streak EQUALS 3) fires once per streak and needs no
    // second dedupe table.
    let behind_streak = match outcome.status {
        LagStatus::Behind => stored.as_ref().map_or(1, |row| row.behind_streak + 1),
        // `not_measured` is deliberately NOT "keep counting": a pipeline
        // whose inputs cannot be read has no lag to be behind by, and
        // counting a streak it cannot measure would alert on an absence.
        LagStatus::Met | LagStatus::NotMeasured => 0,
    };
    let update = LagStateUpdate {
        pipeline_id: pipeline.id.clone(),
        tenant_id: pipeline.tenant_id,
        consumed_tokens: state.consumed_tokens.as_ref().map(tokens_to_json),
        consumed_at: state.consumed_at,
        pending_since: state.pending_since,
        requested_run_key: state.requested_run_key.clone(),
        requested_tokens: state.requested_tokens.as_ref().map(tokens_to_json),
        last_check_at: now,
        last_status: outcome.status.as_str().to_owned(),
        behind_streak,
    };
    pipelines::upsert_lag_state(pool, &update)
        .await
        .map_err(|err| {
            tracing::error!(%err, pipeline = %pipeline.id, "target lag: state row unwritable");
            crate::upstream_error::report(
                &crate::upstream_error::STORE,
                crate::upstream_error::Class::Unavailable,
                &err.to_string(),
            );
            ApiError::Internal("the request could not be completed".to_owned())
        })?;

    Ok(LagCheckOutcome {
        behind_streak,
        ..outcome
    })
}

/// The decision itself, over values that are already read — the only I/O
/// left is the input tokens and, when a change is pending, the run budget.
///
/// `state` is mutated for the two things this check decides: a fresh
/// `pending_since`, and (when the decision is [`Decision::Run`]) the
/// outstanding request. Both are written by the caller in ONE statement
/// after this returns, so a request and the status that justified it can
/// never be stored apart.
///
/// # Errors
///
/// [`ApiError`] when the orchestrator's run history cannot be read, so the
/// tick decides nothing and the next one asks again. A [`TokenError`] is
/// NOT propagated: it is the `not_measured` answer this function returns.
#[allow(
    clippy::too_many_arguments,
    reason = "one parameter per input of the decision; a struct here would only move them"
)]
async fn decide(
    dagster: &DgClient,
    reader: &impl LagTokenReader,
    pipeline: &Pipeline,
    state: &mut CheckState,
    inputs: &[InputTable],
    outstanding: bool,
    target_seconds: Option<i32>,
    job_name: &str,
    now: OffsetDateTime,
) -> Result<LagCheckOutcome, ApiError> {
    let tokens = match reader.tokens(inputs).await {
        Ok(tokens) => tokens,
        Err(err) => {
            // Plan D4: no run is requested on a guess, and the answer is
            // "not measured", never "met". `pending_since` and any
            // outstanding request are left exactly as they were: the change
            // is still pending, this check simply cannot see it right now.
            return Ok(LagCheckOutcome {
                decision: Decision::Wait,
                run_key: state.requested_run_key.clone(),
                job_name: job_name.to_owned(),
                status: LagStatus::NotMeasured,
                lag_seconds: None,
                target_seconds,
                behind_streak: 0,
                reason: Some(err.reason().to_owned()),
            });
        }
    };

    // Plan D5 step 3: tokens equal to what the last successful run consumed
    // means there is nothing to do, whatever the clock says — and the
    // pending window closes, so the next change starts its lag from zero.
    if state.consumed_tokens.as_ref() == Some(&tokens) {
        state.pending_since = None;
        return Ok(LagCheckOutcome {
            decision: Decision::Wait,
            run_key: state.requested_run_key.clone(),
            job_name: job_name.to_owned(),
            status: LagStatus::Met,
            lag_seconds: None,
            target_seconds,
            behind_streak: 0,
            reason: Some("no input changed since the last successful run".to_owned()),
        });
    }

    // Plan D5 step 4: a difference, so the lag is measured from the first
    // check that saw it, and is never reset until the change is consumed.
    let pending_since = state.pending_since.unwrap_or(now);
    state.pending_since = Some(pending_since);
    let lag_seconds = (now - pending_since).whole_seconds();

    // Plan D5 step 5: a pipeline that has never consumed anything under
    // its lag runs now and is behind until that run succeeds. No budget
    // question arises: there is nothing to wait for.
    let never_consumed = state.consumed_tokens.is_none();
    // A `downstream` pipeline has no target of its own (plan D7): its
    // inputs decide, and a lag pipeline that reads it decides WHEN.
    let target = target_seconds.map_or(i64::MAX, i64::from);

    let mut decision = Decision::Wait;
    if !outstanding {
        let due = if never_consumed || target == i64::MAX {
            true
        } else {
            let budget = run_budget_seconds(dagster, job_name).await.map_err(|err| {
                tracing::warn!(
                    %err,
                    pipeline = %pipeline.id,
                    "target lag: run budget unreadable; this tick decides nothing"
                );
                crate::upstream_error::report(
                    &crate::upstream_error::ORCHESTRATOR_UNREACHABLE,
                    crate::upstream_error::Class::Unavailable,
                    &err.to_string(),
                )
                .into_api_error(crate::upstream_error::FailedAs::Internal)
            })?;
            // Plan D5 step 4: "changes inside one lag window are batched
            // into one run", which is this comparison — run once the
            // pending change would otherwise make the target unmeetable.
            lag_seconds >= target.saturating_sub(budget)
        };
        if due {
            decision = Decision::Run;
            let key = run_key_for(&pipeline.id, &tokens);
            state.requested_run_key = Some(key);
            state.requested_tokens = Some(tokens);
        }
    }

    // Plan D5 steps 4 and 5: "behind" is a statement about the target, not
    // about whether a run is coming. A pipeline that has never consumed
    // anything is behind until its first run succeeds, whatever its
    // target; after that, the lag decides.
    let status = if never_consumed || lag_seconds > target {
        LagStatus::Behind
    } else {
        LagStatus::Met
    };
    Ok(LagCheckOutcome {
        decision,
        // The key of the run this check just requested, or of the one that
        // is still outstanding; `None` when there is neither.
        run_key: state.requested_run_key.clone(),
        job_name: job_name.to_owned(),
        status,
        lag_seconds: Some(lag_seconds),
        target_seconds,
        behind_streak: 0,
        reason: Some(reason_for(status, never_consumed).to_owned()),
    })
}

/// The one-line reason the page shows beside the status, in fixed words
/// (plan D8). Never upstream text.
fn reason_for(status: LagStatus, never_consumed: bool) -> &'static str {
    match (status, never_consumed) {
        (LagStatus::NotMeasured, _) => "not measured",
        (LagStatus::Behind, true) => "waiting for this pipeline's first successful run",
        (LagStatus::Behind, false) => "past the target lag",
        (LagStatus::Met, _) => "within the target lag",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_dagster::DgClient;
    use serde_json::json;
    use sqlx::PgPool;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// A reader that answers from a fixed map, so a test states the inputs
    /// it means instead of standing up `ClickHouse`/Lakekeeper. The
    /// production reader's two failure modes have their own sets, because
    /// both must fail the whole check.
    struct FakeTokens {
        tokens: HashMap<String, String>,
        missing: Vec<String>,
        unreadable: Vec<String>,
    }

    impl FakeTokens {
        fn new() -> Self {
            Self {
                tokens: HashMap::new(),
                missing: Vec::new(),
                unreadable: Vec::new(),
            }
        }

        fn with(mut self, key: &str, token: &str) -> Self {
            self.tokens.insert(key.to_owned(), token.to_owned());
            self
        }

        fn missing(mut self, key: &str) -> Self {
            self.missing.push(key.to_owned());
            self
        }

        fn unreadable(mut self, key: &str) -> Self {
            self.unreadable.push(key.to_owned());
            self
        }
    }

    #[async_trait::async_trait]
    impl LagTokenReader for FakeTokens {
        async fn token(&self, input: &InputTable) -> Result<Option<String>, TokenError> {
            let key = input.key();
            if self.missing.contains(&key) {
                return Ok(None);
            }
            if self.unreadable.contains(&key) {
                return Err(TokenError::Unreadable);
            }
            Ok(self.tokens.get(&key).cloned())
        }
    }

    fn input_of(zone: &str, table: &str) -> InputTable {
        InputTable {
            zone: zone.to_owned(),
            table: table.to_owned(),
        }
    }

    /// An authored Bronze-to-Silver pipeline with a `minutes`-target lag.
    fn pipeline(id: &str, minutes: i32) -> Pipeline {
        Pipeline {
            id: id.to_owned(),
            name: id.to_owned(),
            kind: "batch".to_owned(),
            status: "ready".to_owned(),
            owner: "o".to_owned(),
            source: "bronze.orders".to_owned(),
            target: "silver.orders_clean".to_owned(),
            connector_id: None,
            source_asset_id: None,
            target_asset_id: None,
            schedule: "manual".to_owned(),
            last_run_at: None,
            next_run_at: None,
            sla_ok: None,
            freshness_lag_seconds: None,
            max_retries: 2,
            description: None,
            depends_on: Vec::new(),
            tenant_id: None,
            target_lag: Some(TargetLag::Lag { minutes }),
        }
    }

    fn source_input() -> InputTable {
        input_of("bronze", "orders")
    }

    /// A `Dagster` stub whose answers both queries the check can make: the
    /// run-key lookup and the run list the budget is the median of.
    /// `run_status` is the status the run-key lookup reports (`None` =
    /// "no run carries that key"), `durations` are the successful runs'
    /// start/end pairs.
    ///
    /// The [`MockServer`] is returned alongside the client and MUST be kept
    /// alive by the caller: dropping it closes the listener, and a later
    /// test's server can then be reached through the pooled connection of
    /// an earlier one — which shows up as a check answering with another
    /// test's fixture rather than as an error.
    async fn dagster_with(
        run_status: Option<&str>,
        durations: &[(f64, f64)],
    ) -> (DgClient, MockServer) {
        let results: Vec<_> = durations
            .iter()
            .map(|(start, end)| {
                json!({
                    "runId": format!("r-{start}"),
                    "jobName": "authored__pl-x",
                    "status": "SUCCESS",
                    "startTime": start,
                    "endTime": end,
                })
            })
            .collect();
        let by_key = run_status.map_or_else(Vec::new, |status| {
            vec![json!({
                "runId": "requested",
                "jobName": "authored__pl-x",
                "status": status,
                "startTime": 1_000.0,
                "endTime": 1_060.0,
            })]
        });
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "pipelineRunsOrError": { "__typename": "Runs", "results": by_key },
                    "runsOrError": { "__typename": "Runs", "results": results },
                }
            })))
            .mount(&server)
            .await;
        (DgClient::new(format!("{}/graphql", server.uri())), server)
    }

    fn at(seconds: i64) -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(seconds)
    }

    fn tokens_of(entries: &[(&str, &str)]) -> TokenMap {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    /// Create the real `pipeline_definition` row a lag state hangs off
    /// (`pipeline_lag_state.pipeline_id` is a foreign key with
    /// `ON DELETE CASCADE`), with the id the fixture uses and the given
    /// target lag.
    async fn seed_pipeline(pool: &PgPool, id: &str, minutes: i32) -> Pipeline {
        let created = pipelines::create_pipeline(
            pool,
            &pipelines::CreatePipelineInput {
                name: id.to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "orders".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "orders_clean".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: None,
                depends_on: Vec::new(),
                connector_id: None,
                id: Some(id.to_owned()),
                target_lag: Some(TargetLag::Lag { minutes }),
            },
            None,
        )
        .await
        .expect("seed the pipeline row");
        assert_eq!(created.target_lag, Some(TargetLag::Lag { minutes }));
        created
    }

    async fn update_in(
        pool: &PgPool,
        pipeline: &Pipeline,
        consumed: Option<TokenMap>,
        pending_since: Option<OffsetDateTime>,
        request: Option<(&str, TokenMap)>,
        status: &str,
        streak: i32,
        now: OffsetDateTime,
    ) {
        let (key, tokens) = request.map_or((None, None), |(k, t)| {
            (Some(k.to_owned()), Some(tokens_to_json(&t)))
        });
        pipelines::upsert_lag_state(
            pool,
            &LagStateUpdate {
                pipeline_id: pipeline.id.clone(),
                tenant_id: None,
                consumed_tokens: consumed.map(|t| tokens_to_json(&t)),
                consumed_at: None,
                pending_since,
                requested_run_key: key,
                requested_tokens: tokens,
                last_check_at: now,
                last_status: status.to_owned(),
                behind_streak: streak,
            },
        )
        .await
        .expect("seed the lag state row");
    }

    /// A pipeline whose inputs have not changed since the last successful
    /// run: `wait`/`met`, whatever the clock says (plan D5 step 3).
    #[sqlx::test(migrations = "../../migrations")]
    async fn unchanged_inputs_are_met_without_a_run(pool: PgPool) {
        let pipeline = seed_pipeline(&pool, "pl-x", 10).await;
        let now = at(1_000);
        let tokens = tokens_of(&[("bronze.orders", "iceberg:7")]);
        update_in(
            &pool,
            &pipeline,
            Some(tokens.clone()),
            None,
            None,
            "met",
            0,
            now,
        )
        .await;
        let reader = FakeTokens::new().with("bronze.orders", "iceberg:7");
        let (dagster, _server) = dagster_with(Some("SUCCESS"), &[]).await;

        let outcome = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            now + time::Duration::minutes(9),
        )
        .await
        .expect("the check succeeds");

        assert_eq!(outcome.decision, Decision::Wait);
        assert_eq!(outcome.status, LagStatus::Met);
        assert_eq!(outcome.lag_seconds, None, "nothing is pending");
        assert_eq!(outcome.target_seconds, Some(600));
        assert_eq!(outcome.behind_streak, 0);
    }

    /// A change requests a run once; the next check, while that run is
    /// still outstanding, must not request a second one (plan D5 steps 2
    /// and 4).
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_change_requests_one_run_and_the_next_check_waits(pool: PgPool) {
        let pipeline = seed_pipeline(&pool, "pl-x", 10).await;
        let now = at(1_000);
        let reader = FakeTokens::new().with("bronze.orders", "iceberg:8");
        // No state row yet: nothing has ever been consumed, so the first
        // check runs (plan D5 step 5).
        let (dagster, _server) = dagster_with(Some("STARTED"), &[]).await;
        let first = run_lag_check(&pool, &dagster, &reader, &pipeline, &[source_input()], now)
            .await
            .expect("the first check succeeds");
        assert_eq!(first.decision, Decision::Run);
        assert_eq!(first.status, LagStatus::Behind);
        let key = first.run_key.clone().expect("a run key");
        assert!(key.starts_with("lag:pl-x:"));

        // Second check, the requested run still STARTED: wait, same key,
        // and no second request row.
        let second = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            now + time::Duration::seconds(30),
        )
        .await
        .expect("the second check succeeds");
        assert_eq!(second.decision, Decision::Wait);
        assert_eq!(second.run_key.as_deref(), Some(key.as_str()));
        let stored = pipelines::get_lag_state(&pool, "pl-x")
            .await
            .expect("read the state row")
            .expect("a row exists");
        assert_eq!(stored.requested_run_key.as_deref(), Some(key.as_str()));
        assert_eq!(stored.last_status.as_deref(), Some("behind"));
    }

    /// The requested run lands: its tokens become what the pipeline
    /// consumed, the request clears, and the next check is `met` again.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_successful_requested_run_is_consumed_and_the_next_check_is_met(pool: PgPool) {
        let pipeline = seed_pipeline(&pool, "pl-x", 10).await;
        let now = at(1_000);
        let tokens = tokens_of(&[("bronze.orders", "iceberg:8")]);
        update_in(
            &pool,
            &pipeline,
            Some(tokens_of(&[("bronze.orders", "iceberg:7")])),
            Some(now),
            Some(("lag:pl-x:abc", tokens)),
            "behind",
            1,
            now,
        )
        .await;
        let reader = FakeTokens::new().with("bronze.orders", "iceberg:8");
        let (dagster, _server) = dagster_with(Some("SUCCESS"), &[]).await;

        let outcome = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            now + time::Duration::seconds(30),
        )
        .await
        .expect("the check succeeds");

        assert_eq!(outcome.decision, Decision::Wait);
        assert_eq!(outcome.status, LagStatus::Met);
        assert!(outcome.run_key.is_none(), "the request is cleared");
        let stored = pipelines::get_lag_state(&pool, "pl-x")
            .await
            .expect("read the state row")
            .expect("a row exists");
        assert_eq!(stored.requested_run_key, None);
        assert_eq!(stored.pending_since, None, "nothing is pending any more");
        assert_eq!(
            stored.consumed_at,
            OffsetDateTime::from_unix_timestamp(1_000).ok(),
            "consumed_at is the requested run's start time"
        );
        let consumed = tokens_from_json(stored.consumed_tokens.as_ref()).expect("consumed map");
        assert_eq!(
            consumed.get("bronze.orders").map(String::as_str),
            Some("iceberg:8")
        );
    }

    /// A run that FAILED clears the request without consuming anything, so
    /// the next check sees the same difference and may ask again. The
    /// failure alert and `max_retries` are what cover a run that keeps
    /// failing (plan D5 step 2).
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_failed_requested_run_clears_the_request_without_consuming(pool: PgPool) {
        let pipeline = seed_pipeline(&pool, "pl-x", 10).await;
        let now = at(1_000);
        let tokens = tokens_of(&[("bronze.orders", "iceberg:8")]);
        update_in(
            &pool,
            &pipeline,
            Some(tokens_of(&[("bronze.orders", "iceberg:7")])),
            Some(now),
            Some(("lag:pl-x:abc", tokens)),
            "behind",
            1,
            now,
        )
        .await;
        let reader = FakeTokens::new().with("bronze.orders", "iceberg:8");
        let (dagster, _server) = dagster_with(Some("FAILURE"), &[]).await;

        let outcome = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            now + time::Duration::seconds(30),
        )
        .await
        .expect("the check succeeds");

        assert_eq!(
            outcome.decision,
            Decision::Wait,
            "a cleared request does not re-run immediately: the change is still inside the target"
        );
        assert_eq!(outcome.status, LagStatus::Met);
        let stored = pipelines::get_lag_state(&pool, "pl-x")
            .await
            .expect("read the state row")
            .expect("a row exists");
        assert_eq!(
            stored.requested_run_key, None,
            "the failed request is cleared, not retried under the same key"
        );
        let consumed = tokens_from_json(stored.consumed_tokens.as_ref()).expect("consumed map");
        assert_eq!(
            consumed.get("bronze.orders").map(String::as_str),
            Some("iceberg:7"),
            "a failed run consumes nothing"
        );
    }

    /// Plan D5 step 4's batching: with a 10-minute target and a 90-second
    /// budget (a 60-second median run plus the 30-second check), a change
    /// noticed 5 minutes ago waits, and the same change at 8.5 minutes
    /// runs — so several changes inside one window cost one run.
    #[sqlx::test(migrations = "../../migrations")]
    async fn changes_inside_one_window_are_batched_into_a_single_run(pool: PgPool) {
        let pipeline = seed_pipeline(&pool, "pl-x", 10).await;
        let now = at(1_000);
        let tokens = tokens_of(&[("bronze.orders", "iceberg:7")]);
        update_in(
            &pool,
            &pipeline,
            Some(tokens),
            Some(now),
            None,
            "met",
            0,
            now,
        )
        .await;
        let reader = FakeTokens::new().with("bronze.orders", "iceberg:8");
        // Two 60-second runs: the median is 60 s, plus the 30 s interval.
        let (dagster, _server) = dagster_with(None, &[(0.0, 60.0), (1_000.0, 1_060.0)]).await;

        let early = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            now + time::Duration::minutes(5),
        )
        .await
        .expect("the check succeeds");
        assert_eq!(early.lag_seconds, Some(300));
        assert_eq!(early.decision, Decision::Wait, "5 min into a 10 min target");
        assert_eq!(early.status, LagStatus::Met);

        let due = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            now + time::Duration::seconds(510),
        )
        .await
        .expect("the check succeeds");
        assert_eq!(due.lag_seconds, Some(510), "600 s target - 90 s budget");
        assert_eq!(due.decision, Decision::Run);
        assert!(due.run_key.is_some());
    }

    /// An input that cannot be read fails the whole check: no run on a
    /// guess, and `not_measured` rather than `met` (plan D4).
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unreadable_input_is_not_measured_and_requests_nothing(pool: PgPool) {
        let pipeline = seed_pipeline(&pool, "pl-x", 1).await;
        let now = at(1_000);
        let reader = FakeTokens::new().unreadable("bronze.orders");
        let (dagster, _server) = dagster_with(Some("SUCCESS"), &[]).await;

        let outcome = run_lag_check(&pool, &dagster, &reader, &pipeline, &[source_input()], now)
            .await
            .expect("the check itself succeeds");

        assert_eq!(outcome.decision, Decision::Wait);
        assert_eq!(outcome.status, LagStatus::NotMeasured);
        assert_eq!(outcome.lag_seconds, None);
        assert_eq!(outcome.target_seconds, Some(60));
        assert_eq!(
            outcome.reason.as_deref(),
            Some("an input could not be read")
        );
        let stored = pipelines::get_lag_state(&pool, "pl-x")
            .await
            .expect("read the state row")
            .expect("a row exists");
        assert_eq!(stored.last_status.as_deref(), Some("not_measured"));
        assert_eq!(stored.requested_run_key, None);
        assert_eq!(stored.behind_streak, 0);
    }

    /// A missing input table is not "changed" and not "met" either.
    #[tokio::test]
    async fn a_missing_input_table_is_not_measured() {
        let reader = FakeTokens::new().missing("bronze.orders");
        let err = reader.tokens(&[source_input()]).await.unwrap_err();
        assert_eq!(err, TokenError::Missing);
        assert_eq!(err.reason(), "an input table does not exist");
    }

    /// An orchestrator that cannot be asked about the outstanding run
    /// decides nothing: the check fails rather than requesting a second
    /// run for the same change (fail closed, plan D6).
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unreachable_orchestrator_fails_the_check_instead_of_deciding(pool: PgPool) {
        let pipeline = seed_pipeline(&pool, "pl-x", 10).await;
        let now = at(1_000);
        update_in(
            &pool,
            &pipeline,
            Some(tokens_of(&[("bronze.orders", "iceberg:7")])),
            Some(now),
            Some(("lag:pl-x:abc", tokens_of(&[("bronze.orders", "iceberg:8")]))),
            "behind",
            1,
            now,
        )
        .await;
        // No `Dagster` listening on the closed port: `find_run_by_run_key`
        // fails at the transport layer.
        let dagster = DgClient::new("http://127.0.0.1:1/graphql".to_owned());
        let reader = FakeTokens::new().with("bronze.orders", "iceberg:8");
        let err = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            at(1_000),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, ApiError::Unavailable(_)),
            "a skipped tick is a 503, got {err:?}"
        );
    }

    /// The inputs of a pipeline are its own source plus the target of each
    /// `depends_on` pipeline (plan D4) — and a `Dagster`-native upstream,
    /// which has no stored row, is skipped rather than failing the check.
    #[sqlx::test(migrations = "../../migrations")]
    async fn inputs_of_covers_the_source_and_every_authored_upstream_target(pool: PgPool) {
        let upstream = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "upstream".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "raw".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "staged".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: None,
                depends_on: Vec::new(),
                connector_id: None,
                id: None,
                target_lag: None,
            },
            None,
        )
        .await
        .expect("create the upstream pipeline");
        let mut downstream = pipeline("pl-x", 10);
        downstream.depends_on = vec![upstream.id.clone(), "ingest_job".to_owned()];

        let inputs = inputs_of(&pool, &downstream)
            .await
            .expect("resolve the inputs");
        assert_eq!(
            inputs,
            vec![input_of("bronze", "orders"), input_of("silver", "staged")]
        );
    }

    /// The budget is the MEDIAN of the last five successful runs, plus one
    /// check interval — and the assumed 60 s when there is nothing to
    /// measure.
    #[tokio::test]
    async fn the_run_budget_is_the_median_plus_one_interval() {
        let (dagster, _server) = dagster_with(None, &[(0.0, 1.0), (1.0, 11.0), (2.0, 22.0)]).await;
        assert_eq!(
            run_budget_seconds(&dagster, "authored__pl-x")
                .await
                .expect("the budget is readable"),
            10 + CHECK_INTERVAL_SECONDS
        );
        let (empty, _server2) = dagster_with(None, &[]).await;
        assert_eq!(
            run_budget_seconds(&empty, "authored__pl-x")
                .await
                .expect("the budget is readable"),
            ASSUMED_FIRST_RUN_SECONDS + CHECK_INTERVAL_SECONDS
        );
    }

    /// The run key is stable for the same change and different for another,
    /// so a retry cannot fan out into two runs and a genuinely new change
    /// is not mistaken for the old request's.
    #[test]
    fn the_run_key_is_derived_from_the_pipeline_and_its_tokens() {
        let first = run_key_for("pl-x", &tokens_of(&[("bronze.orders", "iceberg:7")]));
        let same = run_key_for("pl-x", &tokens_of(&[("bronze.orders", "iceberg:7")]));
        let other = run_key_for("pl-x", &tokens_of(&[("bronze.orders", "iceberg:8")]));
        let elsewhere = run_key_for("pl-y", &tokens_of(&[("bronze.orders", "iceberg:7")]));
        assert_eq!(first, same);
        assert_ne!(first, other);
        assert_ne!(first, elsewhere);
        assert_eq!(first.len(), "lag:pl-x:".len() + 12);
    }
}
