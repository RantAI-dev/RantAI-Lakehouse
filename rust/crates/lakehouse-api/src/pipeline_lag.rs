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

use std::collections::{BTreeMap, HashMap};

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

/// How old a recorded check may be and still be believed (plan D8): three
/// check intervals. Past that the orchestrator has stopped asking — the
/// row then describes when it last worked, not how fresh the pipeline is —
/// so the payload says `not_measured` instead of repeating a stale "met"
/// (`BLD-2-AC5`). Derived from the interval, because the two must move
/// together.
pub const STALE_AFTER_SECONDS: i64 = 3 * CHECK_INTERVAL_SECONDS;

/// The run budget's fallback: how long a pipeline is assumed to take when
/// it has never run successfully and there is no measurement to use
/// (plan D5 step 4).
const ASSUMED_FIRST_RUN_SECONDS: i64 = 60;

/// How many recent successful runs the budget is the MEDIAN of (plan D5
/// step 4). A median, not a mean, so one slow run does not push every
/// later decision; five, so a pipeline that has been running regularly has
/// a window but a new one is not held back by a single sample.
const BUDGET_RUNS: usize = 5;

/// How many check intervals the run budget adds ON TOP of the measured
/// request-to-land time.
///
/// Two, and both are waits the measurement cannot see (review round 1,
/// decision R6a). The batching rule requests a run once
/// `lag >= target - budget`, and a change that landed at the top of a check
/// interval is first measurable up to one interval later — on the tick that
/// notices it, whose decision is to request the run, not to run it. A budget
/// that leaves out either interval is short by that much for EVERY batched
/// run, whatever the target is: the round-1 gate measured exactly that, with
/// the lag growing as the target grew (60 s target, worst sample 68.94 s;
/// 120 s target, worst sample 123.77 s), because the time between the
/// sensor's `RunRequest` and the run's own `startTime` — the daemon queue
/// and the run-worker launch — was outside the budget as well. That part is
/// measured, not assumed: the run part below is the median of
/// `creationTime -> endTime`, and `creationTime` is when the run REQUEST
/// existed.
const BUDGET_CHECK_INTERVALS: i64 = 2;

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
    /// `lag:<pipeline id>:<12 hex chars of the token map's hash>:<attempt>`.
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
pub(crate) fn target_of(pipeline: &Pipeline) -> (Option<i32>, bool) {
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
// BLD-2 review R1 SHOULD-FIX 13: return Ok(None) on unparseable source
// BLD-2 review R1 decision R3 / SHOULD-FIX 9: tenant-scoped
/// BLD-2 review R3 BLOCKER 2 / decision R13: read inputs on existing connection
pub async fn inputs_of_conn(
    conn: &mut sqlx::PgConnection,
    pipeline: &Pipeline,
) -> Result<Option<Vec<InputTable>>, StoreError> {
    let mut inputs = Vec::new();
    let Some(source) = InputTable::parse(&pipeline.source) else {
        return Ok(None);
    };
    inputs.push(source);
    for (id, target) in
        pipelines::targets_for_conn(conn, pipeline.tenant_id, &pipeline.depends_on).await?
    {
        if let Some(input) = InputTable::parse(&target) {
            inputs.push(input);
        } else {
            tracing::warn!(
                pipeline = %id,
                "target lag: an upstream pipeline's stored target is not a <zone>.<table> location;                  it is not watched as an input"
            );
        }
    }
    inputs.sort();
    inputs.dedup();
    Ok(Some(inputs))
}

/// Read input tables for `pipeline`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if acquiring a connection or querying fails.
pub async fn inputs_of(
    pool: &PgPool,
    pipeline: &Pipeline,
) -> Result<Option<Vec<InputTable>>, StoreError> {
    let mut conn = pool.acquire().await?;
    inputs_of_conn(&mut conn, pipeline).await
}

/// `lag:<pipeline id>:<hash>:<attempt>` — the run key a requested run
/// carries, so the next check can find it (plan D5 step 6). The hash is over
/// the canonical token map, so the same change at the same attempt always
/// produces the same key (a repeated tick cannot fan out into two runs) and
/// a different change produces a different one (a genuinely new run is not
/// mistaken for the old request's). The attempt is part of the key because
/// the orchestrator de-duplicates by key: a retry of the same change after a
/// FAILURE must carry a new one (BLD-2 review R3 SHOULD-FIX 5).
// BLD-2 review R1 decision R2 / BLOCKER 4: run key carries attempt number
fn run_key_for(pipeline_id: &str, tokens: &TokenMap, attempt: i32) -> String {
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
    format!("lag:{pipeline_id}:{hex}:{attempt}")
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

/// The run budget: the median of the pipeline's last [`BUDGET_RUNS`]
/// successful runs measured from the run REQUEST to its landing, plus
/// [`BUDGET_CHECK_INTERVALS`] check intervals — or
/// [`ASSUMED_FIRST_RUN_SECONDS`] plus the same intervals when there is
/// nothing to measure (plan D5 step 4).
///
/// The measurement starts at `creationTime`, not `startTime`: what a target
/// lag is spent on is the whole time from the change to the rows, and the
/// daemon's queue and the run-worker launch sit between the request and the
/// run's first step. Measuring from the start left that gap out of the
/// budget, so every batched run landed late by roughly the gap — which is
/// what the round-1 gate measured, with the lag growing as the target grew
/// (review R1 decision R6a). A run Dagster reports without a `creationTime`
/// is measured from its `startTime`, which omits the gap again rather than
/// dropping the run out of the window.
///
/// # Errors
///
/// Returns [`DgError`] if the run list cannot be read. The caller turns
/// this into a skipped tick rather than a decision on a guessed duration.
// BLD-2 review R1 SHOULD-FIX 5: fetch up to 25 runs to get 5 successful runs, doc link BUDGET_RUNS
// BLD-2 review R1 decision R6a: creationTime -> endTime, plus two check intervals
async fn run_budget_seconds(dagster: &DgClient, job_name: &str) -> Result<i64, DgError> {
    let runs = dagster.list_runs_for_job(job_name, 25).await?;
    let mut durations: Vec<i64> = runs
        .iter()
        .filter(|r| r.status == "SUCCESS")
        .filter_map(|r| {
            let created = r.creation_time.or(r.start_time)?;
            let end = r.end_time?;
            #[allow(
                clippy::cast_possible_truncation,
                reason = "a run's wall-clock duration in whole seconds is far inside i64"
            )]
            let seconds = (end - created) as i64;
            (seconds > 0).then_some(seconds)
        })
        .take(BUDGET_RUNS)
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
    Ok(median + BUDGET_CHECK_INTERVALS * CHECK_INTERVAL_SECONDS)
}

/// BLD-2 review R1 decision R1: how long a request may sit in Dagster
/// without a run appearing before the check calls it lost.
///
/// Four checks at [`CHECK_INTERVAL_SECONDS`] — the sensor's evaluation
/// period, so "4 checks" and "2 minutes" are the same thing said twice.
/// It has to be longer than the sensor's own timeout per tick, or a run
/// that is still being created would be declared lost and requested
/// twice, and it has to be short enough that a lost request does not pin
/// the pipeline behind for long.
const LOST_REQUEST_SECONDS: i64 = 120;

/// What the previous check left behind, plus what this check adds.
// BLD-2 review R1 decision R1/R2: CheckState holds requested_at and request_attempt
pub(crate) struct CheckState {
    consumed_tokens: Option<TokenMap>,
    consumed_at: Option<OffsetDateTime>,
    pending_since: Option<OffsetDateTime>,
    requested_run_key: Option<String>,
    requested_tokens: Option<TokenMap>,
    requested_at: Option<OffsetDateTime>,
    request_attempt: i32,
}

impl CheckState {
    /// The state a pipeline starts from: the stored row, or nothing yet.
    /// BLD-2 review R3 SHOULD-FIX 6: one place for the empty state (rule 4).
    pub(crate) fn from_stored(row: Option<&pipelines::PipelineLagState>) -> Self {
        row.map_or_else(
            || Self {
                consumed_tokens: None,
                consumed_at: None,
                pending_since: None,
                requested_run_key: None,
                requested_tokens: None,
                requested_at: None,
                request_attempt: 0,
            },
            Self::from_row,
        )
    }

    fn from_row(row: &pipelines::PipelineLagState) -> Self {
        Self {
            consumed_tokens: tokens_from_json(row.consumed_tokens.as_ref()),
            consumed_at: row.consumed_at,
            pending_since: row.pending_since,
            requested_run_key: row.requested_run_key.clone(),
            requested_tokens: tokens_from_json(row.requested_tokens.as_ref()),
            requested_at: row.requested_at,
            request_attempt: row.request_attempt,
        }
    }
}

/// Whether the requested run has already landed, and what that does to the
/// state.
///
/// * `SUCCESS` → the tokens it consumed become `consumed_tokens`, so the
///   next check compares against them and finds nothing to do.
/// * `FAILURE`/`CANCELED`/`CANCELING` → the request is cleared, nothing is
///   consumed and `request_attempt` goes up by one: the next check asks
///   again under a new run key until `request_attempt` passes the pipeline's
///   `max_retries` (BLD-2 review R3 SHOULD-FIX 5; the retry limit is handled
///   here, so this loop cannot spin).
/// * still running, or not created yet → `true`, and the caller waits.
/// * not found after 2 minutes ([`LOST_REQUEST_SECONDS`]) → request is cleared
///   as lost to decide again without incrementing `request_attempt` (R1/R9).
/// * the lookup itself failed → an error the caller propagates, because a
///   tick that cannot see Dagster must not decide anything (fail closed:
///   the alternative is a second run for the same change).
// BLD-2 review R1 BLOCKER 2: check found run's jobName == expected_job_name
// BLD-2 review R1 decision R1 / BLOCKER 3: clear request if missing after 2 minutes (4 checks)
// BLD-2 review R1 SHOULD-FIX 4: on SUCCESS clear pending_since
// BLD-2 review R1 decision R2 / BLOCKER 4: on FAILURE increment request_attempt
async fn settle_request(
    dagster: &DgClient,
    state: &mut CheckState,
    expected_job_name: &str,
    now: OffsetDateTime,
) -> Result<bool, DgError> {
    let Some(run_key) = state.requested_run_key.clone() else {
        return Ok(false);
    };
    let found = dagster.find_run_by_run_key(&run_key).await?;
    let run = match found {
        Some(r) if r.job_name == expected_job_name => Some(r),
        Some(r) => {
            tracing::warn!(
                %run_key,
                found_job = %r.job_name,
                %expected_job_name,
                "target lag: found run for key has different jobName than requested; ignoring and treating as not created"
            );
            None
        }
        None => None,
    };

    let Some(run) = run else {
        if let Some(requested_at) = state.requested_at
            && (now - requested_at).whole_seconds() >= LOST_REQUEST_SECONDS
        {
            tracing::warn!(
                %run_key,
                %requested_at,
                lost_after_seconds = LOST_REQUEST_SECONDS,
                "target lag: requested run not found in Dagster after 2 minutes; request lost, clearing to decide again"
            );
            state.requested_run_key = None;
            state.requested_at = None;
            // BLD-2 review R2 decision R9: a lost request (R1) is not an attempt (R2).
            // Only a run that Dagster created and that ended FAILURE/CANCELED counts
            // toward request_attempt.
            return Ok(false);
        }
        tracing::info!(%run_key, "target lag: requested run not created yet; waiting");
        return Ok(true);
    };

    match run.status.as_str() {
        "SUCCESS" => {
            state.consumed_tokens = state.requested_tokens.take();
            state.consumed_at = run
                .start_time
                .and_then(|seconds| {
                    #[allow(
                        clippy::cast_possible_truncation,
                        reason = "Dagster run start times are unix seconds; the fractional part is dropped on purpose"
                    )]
                    let whole = seconds as i64;
                    OffsetDateTime::from_unix_timestamp(whole).ok()
                });
            state.pending_since = None;
            state.requested_run_key = None;
            state.requested_tokens = None;
            state.requested_at = None;
            state.request_attempt = 0;
            Ok(false)
        }
        "FAILURE" | "CANCELED" | "CANCELING" => {
            tracing::info!(%run_key, status = %run.status, "target lag: requested run did not succeed; clearing request and incrementing attempt");
            state.requested_run_key = None;
            // BLD-2 review R1 decision R2: only the KEY goes — the tokens
            // stay as the anchor `request_attempt` is reset against when the
            // inputs change, and the next check asks again under
            // `run_key_for(.., attempt + 1)`, a key `Dagster` has never seen
            // (it de-duplicates by key, which is why a retry needs a new
            // one).
            state.requested_at = None;
            state.request_attempt += 1;
            Ok(false)
        }
        _ => Ok(true),
    }
}

/// BLD-2 review R4 SHOULD-FIX 4: is this failure the lag check's own budget
/// being enforced inside its transaction — the row-level `lock_timeout`
/// (55P03) or the per-statement `statement_timeout` (57014) the check sets
/// under R13? It chases `source` links because the store folds a raw
/// `sqlx::Error` into [`StoreError::Database`], which keeps it as `source`;
/// the raw `sqlx::Error` itself is the chain's first hop when the check
/// makes one directly (`SET LOCAL`s, commits).
// BLD-2 review R5 SHOULD-FIX 6: the two SQLSTATEs are matched here instead
// of through an existing helper, deliberately (rule 4). Both of those take a
// `&sqlx::Error` and answer a different question:
// `connector_probe::classify_sqlx_error` is a generic CONNECTION failure
// class for probe messages (its `Database` arm is about authentication),
// and `connector_listing::class_of_sqlx` classes 57014 as a timeout but not
// 55P03 as a budget error — it does not know the check's two cuts. Neither
// walks a whole error CHAIN, which is what this predicate needs, because the
// store folds the `sqlx::Error` into `StoreError::Database` and only the raw
// error is its `source`. Sharing a helper would have given it a question no
// other caller has; the two tests below pin both SQLSTATEs and one code that
// is neither.
// BLD-2 review R5 SHOULD-FIX 4: the stray `R2 SHOULD-FIX 6` line that stood
// above this doc labelled nothing here — that finding's helper is
// `make_lag_update`, whose own site carries the tag.
fn is_check_budget_error(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = current {
        let is_budget = e.downcast_ref::<sqlx::Error>().is_some_and(|err| {
            matches!(err, sqlx::Error::Database(db)
                if matches!(db.code().as_deref(), Some("55P03" | "57014")))
        });
        if is_budget {
            return true;
        }
        current = e.source();
    }
    false
}

fn store_err(pipeline_id: &str, msg: &str, err: impl std::error::Error + 'static) -> ApiError {
    // BLD-2 review R4 SHOULD-FIX 4: the two cuts the check sets itself
    // enforce the same 8 s budget as the outer clock (they exist for it, R13),
    // so they answer the same classified 503 the clock answers — "this tick
    // decides nothing" is one failure to the caller whichever tripwire
    // fired. Every other storage failure keeps the generic 500 and neither
    // branch ever carries upstream text (principle 4).
    if is_check_budget_error(&err) {
        tracing::warn!(
            pipeline = %pipeline_id,
            "target lag: {msg} (the check's own budget was enforced)"
        );
        crate::upstream_error::report(
            &crate::upstream_error::ORCHESTRATOR_UNREACHABLE,
            crate::upstream_error::Class::Unavailable,
            &"storage was locked or filtered slower than the check's own budget",
        );
        return ApiError::Unavailable("the orchestrator is unavailable".to_owned());
    }
    tracing::error!(%err, pipeline = %pipeline_id, "target lag: {msg}");
    crate::upstream_error::report(
        &crate::upstream_error::STORE,
        crate::upstream_error::Class::Unavailable,
        &err.to_string(),
    );
    ApiError::Internal("the request could not be completed".to_owned())
}

/// A lag check that cannot ask the orchestrator decides nothing and answers
/// the classified 503 (BLD-2 review R3 SHOULD-FIX 6: one helper for the five
/// copies of this mapping).
fn orchestrator_unavailable(err: impl std::fmt::Display, pipeline_id: &str, msg: &str) -> ApiError {
    tracing::warn!(%err, pipeline = %pipeline_id, "target lag: {msg}");
    crate::upstream_error::report(
        &crate::upstream_error::ORCHESTRATOR_UNREACHABLE,
        crate::upstream_error::Class::Unavailable,
        &err.to_string(),
    );
    ApiError::Unavailable("the orchestrator is unavailable".to_owned())
}

// BLD-2 review R2 SHOULD-FIX 6: helper for constructing LagStateUpdate
pub(crate) fn make_lag_update(
    pipeline: &Pipeline,
    state: &CheckState,
    now: OffsetDateTime,
    last_status: &str,
    behind_streak: i32,
) -> LagStateUpdate {
    LagStateUpdate {
        pipeline_id: pipeline.id.clone(),
        tenant_id: pipeline.tenant_id,
        consumed_tokens: state.consumed_tokens.as_ref().map(tokens_to_json),
        consumed_at: state.consumed_at,
        pending_since: state.pending_since,
        requested_run_key: state.requested_run_key.clone(),
        requested_tokens: state.requested_tokens.as_ref().map(tokens_to_json),
        requested_at: state.requested_at,
        request_attempt: state.request_attempt,
        last_check_at: now,
        last_status: last_status.to_owned(),
        behind_streak,
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
#[allow(
    clippy::too_many_lines,
    reason = "orchestrates the single lag-check decision flow, settling requests, upstream dependencies, and local state"
)]
pub async fn run_lag_check(
    pool: &PgPool,
    dagster: &DgClient,
    reader: &impl LagTokenReader,
    pipeline: &Pipeline,
    inputs: &[InputTable],
    now: OffsetDateTime,
) -> Result<LagCheckOutcome, ApiError> {
    let (target_seconds, downstream) = target_of(pipeline);
    let job_name = crate::routes::authored_pipelines::job_name(&pipeline.id);

    // BLD-2 review R1 SHOULD-FIX 6: A pipeline with no target lag must not decide run / behind:
    // answer wait with not_measured and touch nothing.
    if target_seconds.is_none() && !downstream {
        return Ok(LagCheckOutcome {
            decision: Decision::Wait,
            run_key: None,
            job_name,
            status: LagStatus::NotMeasured,
            lag_seconds: None,
            target_seconds: None,
            behind_streak: 0,
            reason: Some("pipeline has no target lag configured".to_owned()),
        });
    }

    // Plan D7: a downstream pipeline that no lag pipeline reads never runs:
    // its page says "Downstream: no pipeline with a target lag reads this table".
    if downstream {
        // BLD-2 review R1 decision R3 / SHOULD-FIX 9: tenant-scoped
        let readers = pipelines::count_lag_pipelines_reading(
            pool,
            &pipeline.id,
            &pipeline.target,
            pipeline.tenant_id,
        )
        .await
        .map_err(|err| store_err(&pipeline.id, "reader count failed", err))?;
        if readers == 0 {
            return Ok(LagCheckOutcome {
                decision: Decision::Wait,
                run_key: None,
                job_name,
                status: LagStatus::NotMeasured,
                lag_seconds: None,
                target_seconds: None,
                behind_streak: 0,
                reason: Some(
                    "Downstream: no pipeline with a target lag reads this table".to_owned(),
                ),
            });
        }

        // BLD-2 review R2 SHOULD-FIX 3: A manual lag-check on a downstream pipeline answers wait.
        // A downstream pipeline is refreshed when the pipeline that reads it runs (plan D7),
        // so checking it directly never starts a run.
        let stored = pipelines::get_lag_state(pool, &pipeline.id)
            .await
            .map_err(|err| store_err(&pipeline.id, "state row unreadable", err))?;
        let status = stored
            .as_ref()
            .and_then(|s| s.last_status.as_deref())
            .map_or(LagStatus::NotMeasured, |st| match st {
                "met" => LagStatus::Met,
                "behind" => LagStatus::Behind,
                _ => LagStatus::NotMeasured,
            });
        let behind_streak = stored.as_ref().map_or(0, |s| s.behind_streak);
        return Ok(LagCheckOutcome {
            decision: Decision::Wait,
            run_key: None,
            job_name,
            status,
            lag_seconds: None,
            target_seconds: None,
            behind_streak,
            reason: Some("downstream pipelines are triggered by their readers".to_owned()),
        });
    }

    // BLD-2 review R3 BLOCKER 2 / decisions R12 & R13:
    // Open transaction, configure timeouts, and bound network calls under 8s timeout.
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| store_err(&pipeline.id, "transaction begin failed", err))?;

    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await
        .map_err(|err| store_err(&pipeline.id, "set lock_timeout failed", err))?;

    sqlx::query("SET LOCAL statement_timeout = '8s'")
        .execute(&mut *tx)
        .await
        .map_err(|err| store_err(&pipeline.id, "set statement_timeout failed", err))?;

    let work = async {
        // BLD-2 review R3 BLOCKER 2 / decision R13: use tx for reads inside the transaction
        let chain = pipelines::upstream_downstream_chain_conn(&mut tx, pipeline)
            .await
            .map_err(|err| store_err(&pipeline.id, "upstream chain failed", err))?;

        // BLD-2 review R3 BLOCKER 2 / decision R12: lock P and every U in ONE statement.
        // BLD-2 review R4 SHOULD-FIX 1: the id order is decided inside
        // `lock_lag_states_ordered`, so no caller can lock in a
        // different order and reintroduce a interleaving deadlock.
        let all_ids: Vec<String> = [pipeline.id.clone()]
            .into_iter()
            .chain(chain.iter().map(|u| u.id.clone()))
            .collect();

        let mut locked_states =
            pipelines::lock_lag_states_ordered(&mut tx, &all_ids, pipeline.tenant_id)
                .await
                .map_err(|err| store_err(&pipeline.id, "ordered lock failed", err))?;

        let stored = locked_states.remove(&pipeline.id).flatten();
        let mut state = CheckState::from_stored(stored.as_ref());

        let outstanding = settle_request(dagster, &mut state, &job_name, now)
            .await
            .map_err(|err| {
                orchestrator_unavailable(
                    err,
                    &pipeline.id,
                    "orchestrator unreachable; skipping this tick",
                )
            })?;

        if let Some(early) = evaluate_upstream_chain(
            &mut tx,
            dagster,
            reader,
            pipeline,
            &chain,
            &mut state,
            stored.as_ref(),
            &mut locked_states,
            target_seconds,
            &job_name,
            now,
        )
        .await?
        {
            tx.commit()
                .await
                .map_err(|err| store_err(&pipeline.id, "transaction commit failed", err))?;
            return Ok(early);
        }

        let mut extra_budget = 0;
        for up in &chain {
            let up_job = crate::routes::authored_pipelines::job_name(&up.id);
            extra_budget += run_budget_seconds(dagster, &up_job).await.map_err(|err| {
                orchestrator_unavailable(err, &pipeline.id, "upstream run budget unreadable")
            })?;
        }

        let outcome = decide(
            dagster,
            reader,
            pipeline,
            &mut state,
            inputs,
            outstanding,
            target_seconds,
            extra_budget,
            &job_name,
            now,
        )
        .await?;

        let behind_streak = match outcome.status {
            LagStatus::Behind => stored.as_ref().map_or(1, |row| row.behind_streak + 1),
            LagStatus::Met | LagStatus::NotMeasured => 0,
        };
        let update = make_lag_update(
            pipeline,
            &state,
            now,
            outcome.status.as_str(),
            behind_streak,
        );
        pipelines::upsert_lag_state_conn(&mut tx, &update)
            .await
            .map_err(|err| store_err(&pipeline.id, "state row unwritable", err))?;
        tx.commit()
            .await
            .map_err(|err| store_err(&pipeline.id, "transaction commit failed", err))?;

        Ok(LagCheckOutcome {
            behind_streak,
            ..outcome
        })
    };

    if let Ok(res) = tokio::time::timeout(std::time::Duration::from_secs(8), work).await {
        res
    } else {
        // BLD-2 review R3 BLOCKER 2 / decision R13: roll back and answer classified 503
        tracing::warn!(
            pipeline = %pipeline.id,
            "target lag: check timed out while holding locks; transaction rolled back"
        );
        crate::upstream_error::report(
            &crate::upstream_error::ORCHESTRATOR_UNREACHABLE,
            crate::upstream_error::Class::Unavailable,
            &"check timed out while holding locks",
        );
        Err(ApiError::Unavailable(
            "the orchestrator is unavailable".to_owned(),
        ))
    }
}

// BLD-2 review R2 BLOCKER 2: helper for resetting attempt on token change and checking max_retries
fn check_retries_and_reset(state: &mut CheckState, tokens: &TokenMap, max_retries: i32) -> bool {
    if state.requested_tokens.as_ref() != Some(tokens) {
        state.request_attempt = 0;
    }
    state.request_attempt > max_retries
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "evaluates chain of upstream downstream pipelines in one coherent loop"
)]
async fn evaluate_upstream_chain(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    dagster: &DgClient,
    reader: &impl LagTokenReader,
    pipeline: &Pipeline,
    chain: &[Pipeline],
    state: &mut CheckState,
    stored: Option<&pipelines::PipelineLagState>,
    locked_states: &mut std::collections::BTreeMap<String, Option<pipelines::PipelineLagState>>,
    target_seconds: Option<i32>,
    job_name: &str,
    now: OffsetDateTime,
) -> Result<Option<LagCheckOutcome>, ApiError> {
    for u_pipe in chain {
        // BLD-2 review R3 BLOCKER 2 / decision R12: U row was locked in one ordered statement with P
        let u_stored = locked_states.remove(&u_pipe.id).flatten();
        let mut u_state = CheckState::from_stored(u_stored.as_ref());
        let u_job_name = crate::routes::authored_pipelines::job_name(&u_pipe.id);
        let u_outstanding = settle_request(dagster, &mut u_state, &u_job_name, now)
            .await
            .map_err(|err| {
                orchestrator_unavailable(
                    err,
                    &u_pipe.id,
                    "orchestrator unreachable; skipping this tick",
                )
            })?;

        if u_outstanding {
            // BLD-2 review R3 SHOULD-FIX 3: record U's check while its run is
            // outstanding, so a run over 90 s does not read "no check recorded".
            let u_update =
                make_lag_update(u_pipe, &u_state, now, LagStatus::NotMeasured.as_str(), 0);
            pipelines::upsert_lag_state_conn(tx, &u_update)
                .await
                .map_err(|err| store_err(&u_pipe.id, "upstream state row unwritable", err))?;
            let pending_since = state.pending_since.unwrap_or(now);
            state.pending_since = Some(pending_since);
            let lag_seconds = (now - pending_since).whole_seconds();
            // BLD-2 review R3 BLOCKER 1: U's run is merely outstanding here (not exhausted),
            // so P is judged by its own lag against its target as before; only an
            // exhausted U (R14) forces `behind`.
            let target = target_seconds.map_or(i64::MAX, i64::from);
            let status = if state.consumed_tokens.is_none() || lag_seconds > target {
                LagStatus::Behind
            } else {
                LagStatus::Met
            };
            let behind_streak = match status {
                LagStatus::Behind => stored.map_or(1, |row| row.behind_streak + 1),
                LagStatus::Met | LagStatus::NotMeasured => 0,
            };
            // BLD-2 review R1 BLOCKER 5: keep P request
            let p_update = make_lag_update(pipeline, state, now, status.as_str(), behind_streak);
            pipelines::upsert_lag_state_conn(tx, &p_update)
                .await
                .map_err(|err| store_err(&pipeline.id, "state row unwritable", err))?;
            return Ok(Some(LagCheckOutcome {
                decision: Decision::Wait,
                run_key: state.requested_run_key.clone(),
                job_name: job_name.to_owned(),
                status,
                lag_seconds: Some(lag_seconds),
                target_seconds,
                behind_streak,
                reason: Some(format!(
                    "waiting for upstream pipeline {} to finish",
                    u_pipe.id
                )),
            }));
        }

        // BLD-2 review R1 SHOULD-FIX 1 / SHOULD-FIX 13: an upstream whose
        // source cannot be parsed is not measurable, so this branch writes
        // P not_measured instead of returning without a write (the page
        // would otherwise keep showing the old status).
        // BLD-2 review R3 BLOCKER 2 / decision R13: read inputs on existing connection tx
        let u_inputs_maybe = inputs_of_conn(tx, u_pipe)
            .await
            .map_err(|err| store_err(&u_pipe.id, "upstream inputs unreadable", err))?;
        let Some(u_inputs) = u_inputs_maybe else {
            let p_update =
                make_lag_update(pipeline, state, now, LagStatus::NotMeasured.as_str(), 0);
            pipelines::upsert_lag_state_conn(tx, &p_update)
                .await
                .map_err(|err| store_err(&pipeline.id, "state row unwritable", err))?;
            return Ok(Some(LagCheckOutcome {
                decision: Decision::Wait,
                run_key: state.requested_run_key.clone(),
                job_name: job_name.to_owned(),
                status: LagStatus::NotMeasured,
                lag_seconds: None,
                target_seconds,
                behind_streak: 0,
                reason: Some(format!(
                    "upstream pipeline {} has unparseable source",
                    u_pipe.id
                )),
            }));
        };

        let u_tokens = match reader.tokens(&u_inputs).await {
            Ok(tokens) => tokens,
            Err(err) => {
                let p_update =
                    make_lag_update(pipeline, state, now, LagStatus::NotMeasured.as_str(), 0);
                pipelines::upsert_lag_state_conn(tx, &p_update)
                    .await
                    .map_err(|err| store_err(&pipeline.id, "state row unwritable", err))?;
                return Ok(Some(LagCheckOutcome {
                    decision: Decision::Wait,
                    run_key: state.requested_run_key.clone(),
                    job_name: job_name.to_owned(),
                    status: LagStatus::NotMeasured,
                    lag_seconds: None,
                    target_seconds,
                    behind_streak: 0,
                    reason: Some(err.reason().to_owned()),
                }));
            }
        };

        // BLD-2 review R2 BLOCKER 2: reset request_attempt when tokens change; stop after max_retries + 1
        let u_max_retries = i32::from(u_pipe.max_retries);
        let u_retries_exhausted = check_retries_and_reset(&mut u_state, &u_tokens, u_max_retries);
        let u_changed = u_state.consumed_tokens.as_ref() != Some(&u_tokens);

        if !u_changed {
            u_state.pending_since = None;
            u_state.request_attempt = 0;
            // BLD-2 review R2 SHOULD-FIX 1: record check for unchanged U; never copy old status forward on settle
            // BLD-2 review R3 SHOULD-FIX 3: a U has no target, so it is never `met`.
            let u_update =
                make_lag_update(u_pipe, &u_state, now, LagStatus::NotMeasured.as_str(), 0);
            pipelines::upsert_lag_state_conn(tx, &u_update)
                .await
                .map_err(|err| store_err(&u_pipe.id, "upstream state row unwritable", err))?;
            continue;
        }

        if u_retries_exhausted {
            // BLD-2 review R2 BLOCKER 2: do not request U when retries exhausted
            // BLD-2 review R3 SHOULD-FIX 3: a chain U is `downstream`, it has no target,
            // so it is `not_measured` (the `u_target.is_some()` branch was unreachable).
            let u_update =
                make_lag_update(u_pipe, &u_state, now, LagStatus::NotMeasured.as_str(), 0);
            pipelines::upsert_lag_state_conn(tx, &u_update)
                .await
                .map_err(|err| store_err(&u_pipe.id, "upstream state row unwritable", err))?;

            let pending_since = state.pending_since.unwrap_or(now);
            state.pending_since = Some(pending_since);
            let lag_seconds = (now - pending_since).whole_seconds();
            // BLD-2 review R3 BLOCKER 1 / decision R14: For a P waiting on an exhausted U,
            // P is behind with a reason naming U whatever the lag.
            let status = LagStatus::Behind;
            let behind_streak = stored.map_or(1, |row| row.behind_streak + 1);
            let p_update = make_lag_update(pipeline, state, now, status.as_str(), behind_streak);
            pipelines::upsert_lag_state_conn(tx, &p_update)
                .await
                .map_err(|err| store_err(&pipeline.id, "state row unwritable", err))?;

            return Ok(Some(LagCheckOutcome {
                decision: Decision::Wait,
                run_key: state.requested_run_key.clone(),
                job_name: job_name.to_owned(),
                status,
                lag_seconds: Some(lag_seconds),
                target_seconds,
                behind_streak,
                // BLD-2 review R4 BLOCKER 1 (b) / decision R16: an
                // exhausted U is a different state from an outstanding
                // one, so it gets its own words — the same sentence
                // `lag_json` renders for the pages, with the count
                // derived from U's own `max_retries`.
                reason: Some(format!(
                    "upstream pipeline {}'s last {} for this change failed; \
                     the next input change starts again",
                    u_pipe.id,
                    failed_runs(i32::from(u_pipe.max_retries) + 1)
                )),
            }));
        }

        let mut total_budget = run_budget_seconds(dagster, job_name)
            .await
            .map_err(|err| orchestrator_unavailable(err, &pipeline.id, "run budget unreadable"))?;
        for up in chain {
            let up_job = crate::routes::authored_pipelines::job_name(&up.id);
            total_budget += run_budget_seconds(dagster, &up_job).await.map_err(|err| {
                orchestrator_unavailable(err, &pipeline.id, "upstream run budget unreadable")
            })?;
        }

        let pending_since = state.pending_since.unwrap_or(now);
        state.pending_since = Some(pending_since);
        let lag_seconds = (now - pending_since).whole_seconds();
        let never_consumed = state.consumed_tokens.is_none();
        let target = target_seconds.map_or(i64::MAX, i64::from);
        let due = if never_consumed || target == i64::MAX {
            true
        } else {
            lag_seconds >= target.saturating_sub(total_budget)
        };

        // BLD-2 review R3 SHOULD-FIX 3: a chain U has no target; never `met` or `behind`.
        let (u_status, u_streak) = (LagStatus::NotMeasured.as_str(), 0);

        if due {
            // BLD-2 review R1 decision R2 / BLOCKER 4: run_key carries attempt
            let u_key = run_key_for(&u_pipe.id, &u_tokens, u_state.request_attempt);
            u_state.requested_run_key = Some(u_key.clone());
            u_state.requested_tokens = Some(u_tokens);
            u_state.requested_at = Some(now);

            // BLD-2 review R2 BLOCKER 3: write U update in tx
            let u_update = make_lag_update(u_pipe, &u_state, now, u_status, u_streak);
            pipelines::upsert_lag_state_conn(tx, &u_update)
                .await
                .map_err(|err| store_err(&u_pipe.id, "upstream state row unwritable", err))?;

            let status = if never_consumed || lag_seconds > target {
                LagStatus::Behind
            } else {
                LagStatus::Met
            };
            let behind_streak = match status {
                LagStatus::Behind => stored.map_or(1, |row| row.behind_streak + 1),
                LagStatus::Met | LagStatus::NotMeasured => 0,
            };
            // BLD-2 review R1 BLOCKER 5: keep P request
            let p_update = make_lag_update(pipeline, state, now, status.as_str(), behind_streak);
            pipelines::upsert_lag_state_conn(tx, &p_update)
                .await
                .map_err(|err| store_err(&pipeline.id, "state row unwritable", err))?;

            return Ok(Some(LagCheckOutcome {
                decision: Decision::Run,
                run_key: Some(u_key),
                job_name: u_job_name,
                status,
                lag_seconds: Some(lag_seconds),
                target_seconds,
                behind_streak,
                reason: Some(format!("requesting upstream pipeline {} first", u_pipe.id)),
            }));
        }

        let u_update = make_lag_update(u_pipe, &u_state, now, u_status, u_streak);
        pipelines::upsert_lag_state_conn(tx, &u_update)
            .await
            .map_err(|err| store_err(&u_pipe.id, "upstream state row unwritable", err))?;

        let status = if never_consumed || lag_seconds > target {
            LagStatus::Behind
        } else {
            LagStatus::Met
        };
        let behind_streak = match status {
            LagStatus::Behind => stored.map_or(1, |row| row.behind_streak + 1),
            LagStatus::Met | LagStatus::NotMeasured => 0,
        };
        // BLD-2 review R1 BLOCKER 5: keep P request
        let p_update = make_lag_update(pipeline, state, now, status.as_str(), behind_streak);
        pipelines::upsert_lag_state_conn(tx, &p_update)
            .await
            .map_err(|err| store_err(&pipeline.id, "state row unwritable", err))?;

        return Ok(Some(LagCheckOutcome {
            decision: Decision::Wait,
            run_key: state.requested_run_key.clone(),
            job_name: job_name.to_owned(),
            status,
            lag_seconds: Some(lag_seconds),
            target_seconds,
            behind_streak,
            reason: Some("waiting for target lag budget window".to_owned()),
        }));
    }
    Ok(None)
}

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
    extra_budget: i64,
    job_name: &str,
    now: OffsetDateTime,
) -> Result<LagCheckOutcome, ApiError> {
    let tokens = match reader.tokens(inputs).await {
        Ok(tokens) => tokens,
        Err(err) => {
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

    // BLD-2 review R2 BLOCKER 2: helper for resetting attempt on token change and checking max_retries
    let max_retries = i32::from(pipeline.max_retries);
    let retries_exhausted = check_retries_and_reset(state, &tokens, max_retries);

    if state.consumed_tokens.as_ref() == Some(&tokens) {
        state.pending_since = None;
        state.request_attempt = 0;
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

    let pending_since = state.pending_since.unwrap_or(now);
    state.pending_since = Some(pending_since);
    let lag_seconds = (now - pending_since).whole_seconds();
    let never_consumed = state.consumed_tokens.is_none();
    let target = target_seconds.map_or(i64::MAX, i64::from);

    let mut decision = Decision::Wait;
    if !outstanding {
        // BLD-2 review R1 decision R2 / BLOCKER 4: stop after max_retries + 1 attempts
        if retries_exhausted {
            decision = Decision::Wait;
        } else {
            let due = if never_consumed || target == i64::MAX {
                true
            } else {
                let budget = run_budget_seconds(dagster, job_name).await.map_err(|err| {
                    // BLD-2 review R4 SHOULD-FIX 4: the pipeline's own
                    // budget reads answer the same classified 503 as the
                    // chain's (the shared helper), chosen explicitly —
                    // not a `FailedAs::Internal` request folded over a
                    // `Class::Unavailable` failure.
                    orchestrator_unavailable(err, &pipeline.id, "run budget unreadable")
                })? + extra_budget;
                lag_seconds >= target.saturating_sub(budget)
            };
            if due {
                decision = Decision::Run;
                let key = run_key_for(&pipeline.id, &tokens, state.request_attempt);
                state.requested_run_key = Some(key);
                state.requested_tokens = Some(tokens);
                state.requested_at = Some(now);
            }
        }
    }

    // BLD-2 review R2 decision R10: retries exhausted reads behind whatever the lag
    let (status, reason) = if retries_exhausted {
        let n = max_retries + 1;
        (
            LagStatus::Behind,
            format!(
                "the last {} for this change failed; the next input change starts again",
                failed_runs(n)
            ),
        )
    } else {
        let st = if never_consumed || lag_seconds > target {
            LagStatus::Behind
        } else {
            LagStatus::Met
        };
        (st, reason_for(st, never_consumed).to_owned())
    };
    Ok(LagCheckOutcome {
        decision,
        run_key: state.requested_run_key.clone(),
        job_name: job_name.to_owned(),
        status,
        lag_seconds: Some(lag_seconds),
        target_seconds,
        behind_streak: 0,
        reason: Some(reason),
    })
}

fn reason_for(status: LagStatus, never_consumed: bool) -> &'static str {
    match (status, never_consumed) {
        (LagStatus::NotMeasured, _) => "not measured",
        (LagStatus::Behind, true) => "waiting for this pipeline's first successful run",
        (LagStatus::Behind, false) => "past the target lag",
        (LagStatus::Met, _) => "within the target lag",
    }
}

/// BLD-2 review R5 SHOULD-FIX 7: "last 1 run", not "last 1 runs" — one
/// helper for all four places the count is written (the check's exhausted
/// reason and `lag_json`'s two), because the check and the page must not
/// even disagree about the plural: the same sentence is built twice from
/// one number, and a page quoting "1 runs" was a number badly written.
///
/// `n` is 1 whenever `max_retries` is 0 — a pipeline allowed no retry fails
/// its first run — so this is the DEFAULT page text, not an edge case.
fn failed_runs(n: i32) -> String {
    format!("{n} run{}", if n == 1 { "" } else { "s" })
}

/// BLD-2 plan D9: does THIS check raise the behind-target alert?
///
/// The streak grows by one per check that ends `behind` and is reset to
/// zero by any other outcome, so "exactly 3" fires once per streak and
/// never again while the streak continues — the same once-per-streak shape
/// as `connectors.rs`'s repeated-failure alert, with no second table to
/// dedupe against. The number is the store's, not a local copy: the
/// `pipeline_lag_state` column, this predicate and the alert kind's doc
/// must not be able to disagree.
#[must_use]
pub const fn behind_alert_due(behind_streak: i32) -> bool {
    behind_streak == pipelines::BEHIND_STREAK_ALERT_AFTER
}

/// The `downstream` pipelines in `pipelines` that no lag pipeline reads —
/// plan D7's honest note ("Downstream: no pipeline with a target lag reads
/// this table"), as a set of ids.
///
/// One call to [`pipelines::lag_pipelines_without_readers`] for the whole
/// candidate list, which answers with the SAME predicate the lag check
/// itself uses to decide whether a downstream pipeline is read
/// (`pipelines::count_lag_pipelines_reading`, review R1 SHOULD-FIX 3):
/// reusing that predicate is what makes the two answers agree — an earlier
/// batched version answered the `dependsOn` edge differently from the
/// check, so a pipeline read only through `depends_on` was told on the page
/// that nobody reads it while the check kept scheduling it. It used to ask
/// `count_lag_pipelines_reading` once per candidate, one pool checkout per
/// row; the batched query is one checkout for the page, and the answer is
/// unchanged (`BLD-5`, a follow-up of the same finding).
///
/// The error is the caller's, never a silent zero (a swallowed failure
/// would report every downstream pipeline as having no reader).
///
/// # Errors
///
/// Returns [`StoreError`] if the reader query fails.
// BLD-2 review R1 SHOULD-FIX 3: use the count_lag_pipelines_reading predicate for exact parity including dependsOn
pub async fn downstream_without_readers(
    pool: &PgPool,
    candidates: &[Pipeline],
) -> Result<std::collections::HashSet<String>, StoreError> {
    let candidates: Vec<(&str, &str, Option<uuid::Uuid>)> = candidates
        .iter()
        .filter(|p| p.target_lag == Some(TargetLag::Downstream))
        .map(|p| (p.id.as_str(), p.target.as_str(), p.tenant_id))
        .collect();
    if candidates.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let without = pipelines::lag_pipelines_without_readers(pool, &candidates).await?;
    Ok(without.into_iter().collect())
}

/// BLD-2 review R4 BLOCKER 1 (a) / decision R16, widened by decision R18:
/// which upstream in `pipeline`'s (deepest-first) chain the check's
/// upstream evaluation stops at. The page and the check must name the SAME
/// `U` — see [`first_upstream_stop`] for the walk.
pub struct UpstreamStop {
    /// The id of the upstream pipeline the check's evaluation stops at.
    pub pipeline_id: String,
    /// `Some(u.max_retries + 1)` when U's retries are exhausted — how many
    /// runs of U for this input change were attempted and failed, the
    /// count the reason text names; `None` when U's run is merely
    /// outstanding. The state row's `request_attempt` is the checker's
    /// attempt bookkeeping; the exhausted reason claims
    /// `u.max_retries + 1` from U's OWN definition, so a page and the
    /// check cannot quote different numbers for one state (R16).
    pub failed_runs: Option<i32>,
}

/// The three-part condition every retry-exhausted consumer derives from U's
/// lag-state row: U was requested for this input change, those runs failed
/// without ever being consumed, and the attempt counter has passed
/// `u.max_retries`. The check's own counters (`check_retries_and_reset`)
/// arrive at this state in the transaction that also writes it; this is the
/// page-side and `lag_json`-side derivation of the SAME condition, so the
/// page and the decision cannot disagree about what "exhausted" means.
#[must_use]
pub fn upstream_retries_exhausted(
    u_state: &pipelines::PipelineLagState,
    u_max_retries: i32,
) -> bool {
    u_state.request_attempt > u_max_retries
        && u_state.requested_tokens.is_some()
        && u_state.consumed_tokens.as_ref() != u_state.requested_tokens.as_ref()
}

/// A request to `u_state`'s pipeline is still open: it was asked (its
/// requested tokens are set) and what it was asked to produce has not been
/// consumed. This is the state the check's upstream evaluation stops at —
/// exhausted (attempt counter past `max_retries`, predicate
/// [`upstream_retries_exhausted`]) or merely outstanding (an in-flight
/// run, decision R18).
#[must_use]
fn request_still_open(u_state: &pipelines::PipelineLagState) -> bool {
    u_state.requested_tokens.is_some()
        && u_state.consumed_tokens.as_ref() != u_state.requested_tokens.as_ref()
}

/// BLD-2 review R5 SHOULD-FIX 2 / decision R18 — the page stops where the
/// check stops: which upstream does [`evaluate_upstream_chain`] name? Its
/// `for u_pipe in chain` loop walks the FULL chain in the check's own
/// deepest-first order and answers at the FIRST upstream whose request is
/// still open — exhausted (its own distinct reason, R4 BLOCKER 1 (b)) or
/// merely outstanding ("waiting for upstream pipeline {U} to finish") —
/// and so does this. It used to walk past outstanding upstreams to the
/// first EXHAUSTED one: with a deeper U1 outstanding and a nearer U2
/// exhausted, the check names U1 while the page named U2. A nearer
/// exhausted U2 is never named while a deeper U1 is still being waited
/// on — the check reports U1 first and only reaches U2 after U1 finishes.
///
/// The same rule on the state side: a U with no state row has never been
/// checked, so it cannot be what the check stops at (R16, unchanged).
///
/// Empirically pinned by the route test
/// `the_page_names_the_upstream_the_check_names_when_a_deeper_upstream_is_outstanding`
/// in `tests/pipeline_lag_page.rs`.
pub fn first_upstream_stop<'a>(
    chain: &[Pipeline],
    state_of: &dyn Fn(&str) -> Option<&'a pipelines::PipelineLagState>,
) -> Option<UpstreamStop> {
    for u in chain {
        let Some(u_state) = state_of(&u.id) else {
            continue;
        };
        if !request_still_open(u_state) {
            continue;
        }
        let failed_runs = if upstream_retries_exhausted(u_state, i32::from(u.max_retries)) {
            Some(i32::from(u.max_retries) + 1)
        } else {
            None
        };
        return Some(UpstreamStop {
            pipeline_id: u.id.clone(),
            failed_runs,
        });
    }
    None
}

/// The upstream stop for ONE pipeline, answered by walking the SAME chain
/// the check walks (`pipelines::upstream_downstream_chain`) — never
/// `pipeline.depends_on`
/// alone, because an edge the check schedules on (P's source = U's target)
/// shows up there and not in `dependsOn` (R16).
///
/// # Errors
///
/// Returns [`StoreError`] if chain discovery or a lag-state read fails —
/// a swallowed failure would report every blocked pipeline as unblocked.
pub async fn upstream_stop_of(
    pool: &PgPool,
    pipeline: &Pipeline,
) -> Result<Option<UpstreamStop>, StoreError> {
    let chain = pipelines::upstream_downstream_chain(pool, pipeline).await?;
    let chain_ids: Vec<&str> = chain.iter().map(|u| u.id.as_str()).collect();
    let states = pipelines::list_lag_states(pool, &chain_ids).await?;
    Ok(first_upstream_stop(&chain, &|id| states.get(id)))
}

/// BLD-2 review R5 SHOULD-FIX 1: the WHOLE list's upstream stops, in one
/// connection. The list route used to call [`upstream_stop_of`] per row,
/// and each of those paid one chain walk plus one `list_lag_states` for
/// its chain — an `N+1` (a list page of 100 lag pipelines paid hundreds of
/// round trips, and every one of them is a page nobody is waiting for
/// interactively). One walk per row is unavoidable (the chains differ),
/// but one connection for all of them is not, and the per-row lag-state
/// read is not needed at all.
///
/// `states` is the map the list route already loaded with
/// `ANY($1)` over the rows it is listing, and reading the chain's state
/// from it rather than from a fresh query is SAFE — not merely cheaper —
/// because every chain upstream is already a key of that map:
///
/// * `upstream_downstream_pipelines_conn` only returns rows with
///   `target_lag_downstream = true`, and `TargetLag::from_columns` maps
///   that column to `Some(TargetLag::Downstream)` — so a chain upstream
///   always has a target lag, which is the same predicate the route built
///   `states`' ids from;
/// * that query is tenant-scoped to the pipeline being walked
///   (`tenant_id = $4`, and a tenantless pipeline walks nothing at all),
///   and the route listed under that same tenant (or under `all_tenants`,
///   which lists more), so the upstream row was in `authored` too.
///
/// A chain upstream with no state row is absent from `states` — which is
/// what the query would have answered as well, and what
/// [`first_upstream_stop`] already treats as "never checked, not a stop".
///
/// Pinned by the route test
/// `adding_a_listed_lag_pipeline_does_not_add_a_round_trip_to_the_list`
/// in `tests/pipeline_lag_page.rs`, which counts the pool checkouts one
/// request makes on the test's own database.
///
/// # Errors
///
/// Returns [`StoreError`] if any chain read fails — a swallowed failure
/// would report every blocked pipeline as unblocked.
// BLD-2 review R5 SHOULD-FIX 1
#[allow(
    clippy::implicit_hasher,
    reason = "the map is the store's own `list_lag_states` return value, \
              default SipHash; generalizing over BuildHasher would add a \
              type parameter to the one caller for a capability nothing \
              here uses"
)]
pub async fn upstream_stops_for_list(
    pool: &PgPool,
    authored: &[Pipeline],
    states: &HashMap<String, pipelines::PipelineLagState>,
) -> Result<HashMap<String, Option<UpstreamStop>>, StoreError> {
    let mut conn = pool.acquire().await?;
    let mut stops = HashMap::new();
    for p in authored.iter().filter(|p| p.target_lag.is_some()) {
        let chain = pipelines::upstream_downstream_chain_conn(&mut conn, p).await?;
        stops.insert(
            p.id.clone(),
            first_upstream_stop(&chain, &|id| states.get(id)),
        );
    }
    Ok(stops)
}

/// BLD-2 plan D8: compute the `lag: { status, lagSeconds, lastCheckAt,
/// reason }` object for the pipeline list and detail payloads.
///
/// `null` for a pipeline with no target lag: the two fields are a pair,
/// and a `lag` next to no `targetLag` would claim a measurement nobody
/// makes. Everything else is `not_measured` until a check has run within
/// the last three check intervals (90 s) — the orchestrator is what checks
/// the lag, so a stale row means the orchestrator stopped asking and
/// nothing is known (`BLD-2-AC5`), which is never reported as `met`.
// BLD-2 review R3 BLOCKER 1 / decision R14: lag_json derives exhausted retries and upstream wait from state
// BLD-2 review R4 BLOCKER 1 (b): an exhausted U gets its own reason — the
// outstanding one ("waiting for upstream pipeline U to finish") must not
// describe a state where the next check will never unblock P on its own.
#[must_use]
pub fn lag_json(
    target_lag: Option<TargetLag>,
    state: Option<&pipelines::PipelineLagState>,
    now: OffsetDateTime,
    downstream_no_readers: bool,
    max_retries: i32,
    upstream_stop: Option<&UpstreamStop>,
) -> serde_json::Value {
    if target_lag.is_none() {
        return serde_json::Value::Null;
    }
    let not_measured = |reason: String| {
        serde_json::json!({
            "status": LagStatus::NotMeasured.as_str(),
            "lagSeconds": serde_json::Value::Null,
            "lastCheckAt": serde_json::Value::Null,
            "reason": reason,
        })
    };
    let Some(st) = state else {
        return not_measured("no lag check has run for this pipeline".to_owned());
    };
    let Some(last_check) = st.last_check_at else {
        return not_measured("no lag check has run for this pipeline".to_owned());
    };
    let last_check_iso = last_check
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
        .map_or(serde_json::Value::Null, serde_json::Value::String);
    let stale_read = |reason: String| {
        serde_json::json!({
            "status": LagStatus::NotMeasured.as_str(),
            "lagSeconds": serde_json::Value::Null,
            "lastCheckAt": last_check_iso,
            "reason": reason,
        })
    };
    if (now - last_check).whole_seconds() > STALE_AFTER_SECONDS {
        return stale_read(format!(
            "no check recorded in the last {STALE_AFTER_SECONDS} seconds"
        ));
    }
    if target_lag == Some(TargetLag::Downstream) && downstream_no_readers {
        return stale_read("Downstream: no pipeline with a target lag reads this table".to_owned());
    }
    let mut status = match st.last_status.as_deref() {
        Some("met") => LagStatus::Met,
        Some("behind") => LagStatus::Behind,
        // The three words are a CHECK constraint on the column, so this arm
        // is only reachable for a row written before a check ever ran.
        _ => LagStatus::NotMeasured,
    };
    let lag_seconds = if status == LagStatus::NotMeasured {
        None
    } else {
        st.pending_since.map(|ps| (now - ps).whole_seconds().max(0))
    };

    // BLD-2 review R3 BLOCKER 1 / decision R14 derives exhausted retries
    // from the state row; BLD-2 review R4 BLOCKER 1 (a) / decision R16
    // moved that derivation into the one shared predicate so the page and
    // `lag_json` cannot disagree about what "exhausted" means.
    let retries_exhausted = upstream_retries_exhausted(st, max_retries);

    let reason = if let Some(u) = upstream_stop {
        // BLD-2 review R5 SHOULD-FIX 7: the binding is `runs`, not
        // `failed_runs` — the helper of that name must stay reachable.
        if let Some(runs) = u.failed_runs {
            status = LagStatus::Behind;
            // BLD-2 review R4 BLOCKER 1 (b) / decision R16: the same words the
            // check's exhausted branch orders for P, with the count derived
            // from U's own `max_retries` in [`UpstreamStop::failed_runs`].
            format!(
                "upstream pipeline {}'s last {} for this change failed; \
                 the next input change starts again",
                u.pipeline_id,
                failed_runs(runs)
            )
        } else {
            // BLD-2 review R5 decision R18: the walk stopped at an
            // OUTSTANDING upstream — the same one the check names with
            // these same words in its outstanding branch, so the page
            // keeps the status derived from P's own row here (the check
            // does not force `behind` for an outstanding U either).
            format!("waiting for upstream pipeline {} to finish", u.pipeline_id)
        }
    } else if retries_exhausted {
        status = LagStatus::Behind;
        let n = max_retries + 1;
        format!(
            "the last {} for this change failed; the next input change starts again",
            failed_runs(n)
        )
    } else {
        reason_for(status, st.consumed_tokens.is_none()).to_owned()
    };

    serde_json::json!({
        "status": status.as_str(),
        "lagSeconds": lag_seconds,
        "lastCheckAt": last_check_iso,
        "reason": reason,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;
    use uuid::Uuid;

    use lakehouse_dagster::DgClient;
    use serde_json::json;
    use sqlx::PgPool;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::routes::authored_pipelines::job_name;

    /// BLD-2 review R4 SHOULD-FIX 4: a Postgres `DatabaseError` double with
    /// a chosen SQLSTATE, so the budget classification can be pinned without
    /// waiting for a real `lock_timeout` under contention. The driver error
    /// traits are what `sqlx::Error::Database` carries, so this is all the
    /// classifier can ever see.
    #[derive(Debug)]
    struct FakePgError {
        code: &'static str,
        message: String,
    }

    impl std::fmt::Display for FakePgError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.message)
        }
    }

    impl std::error::Error for FakePgError {}

    impl sqlx::error::DatabaseError for FakePgError {
        fn message(&self) -> &str {
            &self.message
        }

        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            Some(std::borrow::Cow::Borrowed(self.code))
        }

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
    }

    /// BLD-2 review R4 SHOULD-FIX 4: `lock_timeout` (55P03) and
    /// `statement_timeout` (57014) — the two cuts the lag check sets inside
    /// its own transaction — are the same budget tripwire as the outer clock,
    /// so through `store_err` they answer the same classified 503. The chase
    /// must reach the postgre SQLSTATE through `StoreError::Database`
    /// wrapping it, exactly as production store failures arrive.
    #[test]
    fn a_set_lock_or_statement_timeout_maps_store_err_to_the_classified_503() {
        for (code, what) in [("55P03", "lock_timeout"), ("57014", "statement_timeout")] {
            let err = StoreError::Database(sqlx::Error::Database(Box::new(FakePgError {
                code,
                message: format!("{what} fired while a chain writer held the row"),
            })));
            assert!(
                matches!(
                    super::store_err("pl-x", "upstream state row unwritable", err),
                    ApiError::Unavailable(msg) if msg == "the orchestrator is unavailable"
                ),
                "{what} must be the classified 503, not the generic 500"
            );
        }
    }

    /// Any other storage failure — here a check-violation SQLSTATE, which is
    /// neither of the two budget cuts — keeps `store_err`'s generic 500 with
    /// the generic words; upstream text still never reaches the response.
    #[test]
    fn another_store_error_than_the_two_budget_cuts_keeps_the_generic_500() {
        let err = StoreError::Database(sqlx::Error::Database(Box::new(FakePgError {
            code: "23514",
            message: "a check-violation state that is nobody's 8 s tripwire".to_owned(),
        })));
        assert!(
            matches!(
                super::store_err("pl-x", "state row unreadable", err),
                ApiError::Internal(msg) if msg == "the request could not be completed"
            ),
            "a non-budget store failure keeps the generic 500"
        );
    }

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
            // BLD-1 / D1: this fixture is about the lag decision, not about
            // who the pipeline reads as.
            run_as: None,
        }
    }

    fn source_input() -> InputTable {
        input_of("bronze", "orders")
    }

    /// A `Dagster` stub whose answers both queries the check can make: the
    /// run-key lookup and the run list the budget is the median of.
    /// `run_status` is the status the run-key lookup reports (`None` =
    /// "no run carries that key"), `runs` are the successful runs' own
    /// `(creationTime, startTime, endTime)` triples.
    ///
    /// The [`MockServer`] is returned alongside the client and MUST be kept
    /// alive by the caller: dropping it closes the listener, and a later
    /// test's server can then be reached through the pooled connection of
    /// an earlier one — which shows up as a check answering with another
    /// test's fixture rather than as an error.
    // BLD-2 review R1 decision R6a: creation and start are separate inputs,
    // because the budget is measured from the run REQUEST, not from the run.
    async fn dagster_with_runs(
        job: &str,
        run_status: Option<&str>,
        runs: &[(f64, f64, f64)],
    ) -> (DgClient, MockServer) {
        let results: Vec<_> = runs
            .iter()
            .map(|(created, start, end)| {
                json!({
                    "runId": format!("r-{created}"),
                    "jobName": job,
                    "status": "SUCCESS",
                    "creationTime": created,
                    "startTime": start,
                    "endTime": end,
                })
            })
            .collect();
        let by_key = run_status.map_or_else(Vec::new, |status| {
            vec![json!({
                "runId": "requested",
                // BLD-2 review R1 BLOCKER 2: the run-key lookup's answer
                // has to carry the job that was requested, or
                // `settle_request` treats it as somebody else's run.
                "jobName": job,
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

    /// [`dagster_with_runs`] for runs that were created and started at the
    /// same instant, which is what every test that is not about the budget
    /// means by a duration.
    async fn dagster_with(
        job: &str,
        run_status: Option<&str>,
        durations: &[(f64, f64)],
    ) -> (DgClient, MockServer) {
        let runs: Vec<(f64, f64, f64)> = durations.iter().map(|(s, e)| (*s, *s, *e)).collect();
        dagster_with_runs(job, run_status, &runs).await
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
                // BLD-1 / D1: these fixtures are about the lag check, not
                // about who the pipeline reads as.
                run_as: None,
            },
            None,
        )
        .await
        .expect("seed the pipeline row");
        assert_eq!(created.target_lag, Some(TargetLag::Lag { minutes }));
        created
    }

    /// A real `tenant` row: `pipeline_lag_state.tenant_id` references
    /// `tenant (id)`, so a test that writes lag state (directly or through
    /// `run_lag_check`) needs a tenant to exist. Inventing a bare UUID
    /// instead fails the foreign key, and a pipeline with
    /// `tenant_id IS NULL` links to nothing under R3, so the tenant-scoped
    /// tests cannot be written that way either.
    async fn seed_tenant(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO tenant (id, name, slug, plan, residency) \
             VALUES ($1, 'Fixture tenant', $2, 'Standard', 'US')",
        )
        .bind(id)
        .bind(format!("fixture-{id}"))
        .execute(pool)
        .await
        .expect("seed a tenant row");
        id
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
                tenant_id: pipeline.tenant_id,
                consumed_tokens: consumed.map(|t| tokens_to_json(&t)),
                consumed_at: None,
                pending_since,
                requested_run_key: key.clone(),
                requested_tokens: tokens,
                requested_at: if key.is_some() { Some(now) } else { None },
                request_attempt: 0,
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
        let (dagster, _server) = dagster_with(&job_name("pl-x"), Some("SUCCESS"), &[]).await;

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
        let (dagster, _server) = dagster_with(&job_name("pl-x"), Some("STARTED"), &[]).await;
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
        let (dagster, _server) = dagster_with(&job_name("pl-x"), Some("SUCCESS"), &[]).await;

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
        let (dagster, _server) = dagster_with(&job_name("pl-x"), Some("FAILURE"), &[]).await;

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

    /// Plan D5 step 4's batching: with a 10-minute target and a 120-second
    /// budget (a 60-second median request-to-land run plus two 30-second
    /// check intervals, review R1 decision R6a), a change noticed 5 minutes
    /// ago waits, the same change at 7.9 minutes still waits, and at 8
    /// minutes it runs — so several changes inside one window cost one run.
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
        // Two runs that were created when they started and lasted 60 s: the
        // median request-to-land is 60 s, plus two 30 s intervals.
        let (dagster, _server) =
            dagster_with(&job_name("pl-x"), None, &[(0.0, 60.0), (1_000.0, 1_060.0)]).await;

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

        let just_before = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            now + time::Duration::seconds(479),
        )
        .await
        .expect("the check succeeds");
        assert_eq!(
            just_before.decision,
            Decision::Wait,
            "600 s target - 120 s budget: still one second inside the window"
        );

        let due = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            now + time::Duration::seconds(480),
        )
        .await
        .expect("the check succeeds");
        assert_eq!(due.lag_seconds, Some(480), "600 s target - 120 s budget");
        assert_eq!(due.decision, Decision::Run);
        assert!(due.run_key.is_some());
    }

    /// An input that cannot be read fails the whole check: no run on a
    /// guess, and `not_measured` rather than `met` (plan D4).
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unreadable_input_is_not_measured_and_requests_nothing(pool: PgPool) {
        // BLD-2 review R3 BLOCKER 3: the floor is 1 minute (measured, R8), so
        // fixtures may use 1; this one uses 2 only because its numbers do not depend on it.
        let pipeline = seed_pipeline(&pool, "pl-x", 2).await;
        let now = at(1_000);
        let reader = FakeTokens::new().unreadable("bronze.orders");
        let (dagster, _server) = dagster_with(&job_name("pl-x"), Some("SUCCESS"), &[]).await;

        let outcome = run_lag_check(&pool, &dagster, &reader, &pipeline, &[source_input()], now)
            .await
            .expect("the check itself succeeds");

        assert_eq!(outcome.decision, Decision::Wait);
        assert_eq!(outcome.status, LagStatus::NotMeasured);
        assert_eq!(outcome.lag_seconds, None);
        assert_eq!(outcome.target_seconds, Some(120));
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

    /// A pipeline with no target lag has nothing to lag: the check must not
    /// decide `run` or `behind` for it, and must not write a state row that
    /// claims a measurement nobody asked for (review R1 SHOULD-FIX 6).
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_pipeline_without_a_target_lag_is_never_decided(pool: PgPool) {
        let seeded = seed_pipeline(&pool, "pl-none", 2).await;
        let now = at(1_000);
        // A row that says "met, streak 2" before the check: a check that
        // decided anything for a pipeline with no target would overwrite it.
        update_in(
            &pool,
            &seeded,
            Some(tokens_of(&[("bronze.orders", "iceberg:7")])),
            None,
            None,
            "met",
            2,
            now,
        )
        .await;
        let mut pipeline = pipeline("pl-none", 2);
        pipeline.target_lag = None;
        let reader = FakeTokens::new().with("bronze.orders", "iceberg:9");
        let (dagster, _server) = dagster_with(&job_name("pl-none"), None, &[]).await;

        let outcome = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            at(1_200),
        )
        .await
        .expect("the check itself succeeds");

        assert_eq!(outcome.decision, Decision::Wait);
        assert_eq!(outcome.status, LagStatus::NotMeasured);
        assert_eq!(outcome.run_key, None);
        assert_eq!(outcome.target_seconds, None);
        assert_eq!(
            outcome.reason.as_deref(),
            Some("pipeline has no target lag configured")
        );
        let stored = pipelines::get_lag_state(&pool, "pl-none")
            .await
            .expect("read the state row")
            .expect("a row exists");
        assert_eq!(
            stored.last_check_at,
            Some(now),
            "a check for a pipeline with no target lag touches nothing"
        );
        assert_eq!(stored.last_status.as_deref(), Some("met"));
        assert_eq!(stored.behind_streak, 2, "no streak is decided either");
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
        // BLD-2 review R1 decision R3: both ends of the link are inside the
        // tenant, because a `tenant_id IS NULL` pipeline links to nothing.
        let tenant_id = seed_tenant(&pool).await;
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
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: None,
                target_lag: None,
                // BLD-1 / D1: these fixtures are about the lag check, not
                // about who the pipeline reads as.
                run_as: None,
            },
            None,
        )
        .await
        .expect("create the upstream pipeline");
        let mut downstream = pipeline("pl-x", 10);
        downstream.tenant_id = Some(tenant_id);
        downstream.depends_on = vec![upstream.id.clone(), "ingest_job".to_owned()];

        let inputs = inputs_of(&pool, &downstream)
            .await
            .expect("resolve the inputs")
            .expect("inputs parseable");
        assert_eq!(
            inputs,
            vec![input_of("bronze", "orders"), input_of("silver", "staged")]
        );
    }

    /// The budget is the MEDIAN of the last five successful runs measured
    /// from the run REQUEST to its landing, plus two check intervals — and
    /// the assumed 60 s plus the same two intervals when there is nothing to
    /// measure (review R1 decision R6a).
    #[tokio::test]
    async fn the_run_budget_is_the_request_to_land_median_plus_two_intervals() {
        let (dagster, _server) = dagster_with_runs(
            &job_name("pl-x"),
            None,
            &[(0.0, 0.0, 1.0), (1.0, 1.0, 11.0), (2.0, 2.0, 22.0)],
        )
        .await;
        assert_eq!(
            run_budget_seconds(&dagster, &job_name("pl-x"))
                .await
                .expect("the budget is readable"),
            10 + BUDGET_CHECK_INTERVALS * CHECK_INTERVAL_SECONDS
        );
        let (empty, _server2) = dagster_with(&job_name("pl-x"), None, &[]).await;
        assert_eq!(
            run_budget_seconds(&empty, &job_name("pl-x"))
                .await
                .expect("the budget is readable"),
            ASSUMED_FIRST_RUN_SECONDS + BUDGET_CHECK_INTERVALS * CHECK_INTERVAL_SECONDS
        );
    }

    /// R6a: the measured part starts when the run was REQUESTED, not when it
    /// started. A run that waited 100 s in the queue and ran for 60 s costs
    /// the pipeline 160 s of target, and a budget built from
    /// `startTime -> endTime` would promise 60 — which is how every batched
    /// run landed late by the queue gap, whatever the target was.
    #[tokio::test]
    async fn the_run_budget_counts_the_queue_before_the_run_started() {
        let (dagster, _server) = dagster_with_runs(
            &job_name("pl-q"),
            None,
            &[(0.0, 100.0, 160.0), (200.0, 300.0, 360.0)],
        )
        .await;
        assert_eq!(
            run_budget_seconds(&dagster, &job_name("pl-q"))
                .await
                .expect("the budget is readable"),
            160 + BUDGET_CHECK_INTERVALS * CHECK_INTERVAL_SECONDS,
            "both runs waited 100 s in the queue: 160 s from request to land"
        );
    }

    /// The run key is stable for the same change and different for another,
    /// so a retry cannot fan out into two runs and a genuinely new change
    /// is not mistaken for the old request's.
    #[test]
    fn the_run_key_is_derived_from_the_pipeline_and_its_tokens() {
        let first = run_key_for("pl-x", &tokens_of(&[("bronze.orders", "iceberg:7")]), 0);
        let same = run_key_for("pl-x", &tokens_of(&[("bronze.orders", "iceberg:7")]), 0);
        let other = run_key_for("pl-x", &tokens_of(&[("bronze.orders", "iceberg:8")]), 0);
        let elsewhere = run_key_for("pl-y", &tokens_of(&[("bronze.orders", "iceberg:7")]), 0);
        assert_eq!(first, same);
        assert_ne!(first, other);
        assert_ne!(first, elsewhere);
        assert_eq!(
            first.len(),
            "lag:pl-x:".len() + 12 + ":0".len(),
            "lag:<id>:<12 hex chars>:<attempt>"
        );
    }

    /// BLD-2 review R1 decision R2 / BLOCKER 4: the attempt number is part
    /// of the key, so asking again for the SAME tokens after a failed run
    /// produces a key `Dagster` has never seen (it de-duplicates by key)
    /// and a genuinely new change is still not confused with the retry.
    #[test]
    fn the_run_key_carries_the_attempt_so_a_retry_is_a_new_key() {
        let tokens = tokens_of(&[("bronze.orders", "iceberg:7")]);
        let first = run_key_for("pl-x", &tokens, 0);
        let retry = run_key_for("pl-x", &tokens, 1);
        assert_ne!(first, retry);
        assert!(
            retry.ends_with(":1"),
            "the attempt is the last segment: {retry}"
        );
    }

    /// BLD-2 plan D7: when a downstream pipeline's inputs change and the
    /// reading pipeline's decision would be run, the decision is run for U
    /// first, and P waits. A second check while U is outstanding waits.
    #[sqlx::test(migrations = "../../migrations")]
    async fn downstream_pipeline_changes_request_upstream_first_while_downstream_waits(
        pool: PgPool,
    ) {
        let (dagster, _server) = dagster_with(&job_name("pl-u"), Some("STARTED"), &[]).await;
        let tenant_id = seed_tenant(&pool).await;

        // Create U: bronze.orders -> silver.u (downstream)
        let u = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "u_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "orders".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "u_clean".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-u".to_owned()),
                target_lag: Some(TargetLag::Downstream),
                // BLD-1 / D1: these fixtures are about the lag check, not
                // about who the pipeline reads as.
                run_as: None,
            },
            None,
        )
        .await
        .expect("create u");

        // Create P: silver.u_clean -> gold.p (lag 10 minutes)
        let p = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "p_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "silver".to_owned(),
                source_table: "u_clean".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "gold".to_owned(),
                target_table: "p_mart".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-p".to_owned()),
                target_lag: Some(TargetLag::Lag { minutes: 10 }),
                // BLD-1 / D1: these fixtures are about the lag check, not
                // about who the pipeline reads as.
                run_as: None,
            },
            None,
        )
        .await
        .expect("create p");

        let p_inputs = inputs_of(&pool, &p)
            .await
            .expect("p inputs")
            .expect("p parseable");
        let reader = FakeTokens::new()
            .with("bronze.orders", "iceberg:1")
            .with("silver.u_clean", "fp:1");

        let now = at(1_000);
        let outcome = run_lag_check(&pool, &dagster, &reader, &p, &p_inputs, now)
            .await
            .expect("check on p");

        // Decision is run for U first!
        assert_eq!(outcome.decision, Decision::Run);
        assert_eq!(outcome.job_name, "authored__pl_u");
        assert!(outcome.run_key.is_some());

        // U now has requested_run_key in stored state
        let u_state = pipelines::get_lag_state(&pool, &u.id)
            .await
            .expect("read u state")
            .expect("u state exists");
        assert_eq!(u_state.requested_run_key, outcome.run_key);

        // A second check while U is in-flight:
        // Dagster find_run_by_run_key reports STARTED, so U is outstanding.
        let outcome2 = run_lag_check(&pool, &dagster, &reader, &p, &p_inputs, now)
            .await
            .expect("second check on p");
        assert_eq!(outcome2.decision, Decision::Wait);
    }

    /// BLD-2 plan D7: when U's run lands (succeeds in Dagster), P runs for itself.
    #[sqlx::test(migrations = "../../migrations")]
    async fn upstream_landing_triggers_downstream_pipeline_run(pool: PgPool) {
        let (dagster, _server) = dagster_with(&job_name("pl-u2"), Some("SUCCESS"), &[]).await;
        let tenant_id = seed_tenant(&pool).await;

        let u = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "u_pipe2".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "orders".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "u_clean2".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-u2".to_owned()),
                target_lag: Some(TargetLag::Downstream),
                // BLD-1 / D1: these fixtures are about the lag check, not
                // about who the pipeline reads as.
                run_as: None,
            },
            None,
        )
        .await
        .expect("create u");

        let p = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "p_pipe2".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "silver".to_owned(),
                source_table: "u_clean2".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "gold".to_owned(),
                target_table: "p_mart2".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-p2".to_owned()),
                target_lag: Some(TargetLag::Lag { minutes: 10 }),
                // BLD-1 / D1: these fixtures are about the lag check, not
                // about who the pipeline reads as.
                run_as: None,
            },
            None,
        )
        .await
        .expect("create p");

        let now = at(1_000);
        let u_run_key = "lag:pl-u2:token123456";

        // U was already requested and is now settled as SUCCESS in Dagster
        pipelines::upsert_lag_state(
            &pool,
            &pipelines::LagStateUpdate {
                pipeline_id: u.id.clone(),
                tenant_id: Some(tenant_id),
                consumed_tokens: None,
                consumed_at: None,
                pending_since: Some(now),
                requested_run_key: Some(u_run_key.to_owned()),
                requested_tokens: Some(json!({"bronze.orders": "iceberg:1"})),
                // BLD-2 review R1 decisions R1/R2: the seeded row is a
                // request that Dagster answered, so it carries the time it
                // was made and its first attempt.
                requested_at: Some(now),
                request_attempt: 1,
                last_check_at: now,
                last_status: "behind".to_owned(),
                behind_streak: 1,
            },
        )
        .await
        .expect("seed u lag state");

        let p_inputs = inputs_of(&pool, &p)
            .await
            .expect("p inputs")
            .expect("p parseable");
        // U's run has landed, so U consumed bronze.orders: iceberg:1.
        // And U's output silver.u_clean2 now has a fresh token: fp:2.
        let reader = FakeTokens::new()
            .with("bronze.orders", "iceberg:1")
            .with("silver.u_clean2", "fp:2");

        let outcome = run_lag_check(&pool, &dagster, &reader, &p, &p_inputs, now)
            .await
            .expect("check on p");

        // Since U settled and consumed, U's inputs didn't change.
        // P's input silver.u_clean2 is new (never consumed), so P runs for itself!
        assert_eq!(outcome.decision, Decision::Run);
        assert_eq!(outcome.job_name, "authored__pl_p2");
        assert!(outcome.run_key.unwrap().starts_with("lag:pl-p2:"));
    }

    /// BLD-2 plan D7: a downstream pipeline that no lag pipeline reads shows
    /// the honest note "Downstream: no pipeline with a target lag reads this table".
    #[sqlx::test(migrations = "../../migrations")]
    async fn downstream_pipeline_with_no_lag_readers_shows_honest_note(pool: PgPool) {
        let (dagster, _server) = dagster_with(&job_name("pl-orphan"), None, &[]).await;

        let u = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "lonely_downstream".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "orders".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "lonely_clean".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: None,
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-lonely".to_owned()),
                target_lag: Some(TargetLag::Downstream),
                // BLD-1 / D1: these fixtures are about the lag check, not
                // about who the pipeline reads as.
                run_as: None,
            },
            None,
        )
        .await
        .expect("create u");

        let u_inputs = inputs_of(&pool, &u)
            .await
            .expect("u inputs")
            .expect("u parseable");
        let reader = FakeTokens::new().with("bronze.orders", "iceberg:1");

        let now = at(1_000);
        let outcome = run_lag_check(&pool, &dagster, &reader, &u, &u_inputs, now)
            .await
            .expect("check on u");

        assert_eq!(outcome.decision, Decision::Wait);
        assert_eq!(outcome.status, LagStatus::NotMeasured);
        assert_eq!(
            outcome.reason.as_deref(),
            Some("Downstream: no pipeline with a target lag reads this table")
        );
    }

    /// A `pipeline_lag_state` row, for the payload tests that only care
    /// about what the row says, not about who wrote it.
    fn lag_state(
        last_check_at: Option<OffsetDateTime>,
        last_status: &str,
    ) -> pipelines::PipelineLagState {
        pipelines::PipelineLagState {
            pipeline_id: "pl-x".to_owned(),
            tenant_id: None,
            consumed_tokens: Some(serde_json::json!({"bronze.orders": "iceberg:1"})),
            consumed_at: Some(at(100)),
            pending_since: None,
            requested_run_key: None,
            requested_tokens: None,
            requested_at: None,
            request_attempt: 0,
            last_check_at,
            last_status: Some(last_status.to_owned()),
            behind_streak: 0,
        }
    }

    /// Without a target lag there is nothing to report, and the field is
    /// absent rather than a null that looks like a measurement.
    #[test]
    fn lag_json_is_null_when_the_pipeline_has_no_target_lag() {
        let now = at(1_000);
        assert_eq!(
            lag_json(None, None, now, false, 0, None),
            serde_json::Value::Null
        );
    }

    #[test]
    fn lag_json_is_not_measured_before_the_first_check() {
        let now = at(1_000);
        for state in [None, Some(lag_state(None, "met"))] {
            let res = lag_json(
                Some(TargetLag::Lag { minutes: 5 }),
                state.as_ref(),
                now,
                false,
                0,
                None,
            );
            assert_eq!(res["status"], "not_measured");
            assert_eq!(res["lagSeconds"], serde_json::Value::Null);
            assert_eq!(res["lastCheckAt"], serde_json::Value::Null);
            assert_eq!(res["reason"], "no lag check has run for this pipeline");
        }
    }

    /// `BLD-2-AC5`: a check that stopped happening is not a check that
    /// says "met". Three intervals old is the boundary, and one second
    /// past it is already `not_measured`.
    #[test]
    fn lag_json_is_not_measured_once_the_last_check_is_older_than_three_intervals() {
        let now = at(1_000);
        let fresh = lag_state(
            Some(now - time::Duration::seconds(STALE_AFTER_SECONDS)),
            "met",
        );
        assert_eq!(
            lag_json(
                Some(TargetLag::Lag { minutes: 5 }),
                Some(&fresh),
                now,
                false,
                0,
                None,
            )["status"],
            "met",
            "a check exactly three intervals old is still believed"
        );
        let stale = lag_state(
            Some(now - time::Duration::seconds(STALE_AFTER_SECONDS + 1)),
            "met",
        );
        let res = lag_json(
            Some(TargetLag::Lag { minutes: 5 }),
            Some(&stale),
            now,
            false,
            0,
            None,
        );
        assert_eq!(res["status"], "not_measured");
        assert_eq!(
            res["reason"],
            format!("no check recorded in the last {STALE_AFTER_SECONDS} seconds")
        );
        assert!(
            !res["lastCheckAt"].is_null(),
            "the page still shows when the last check ran"
        );
    }

    /// The lag in seconds is `now - pending_since`, and it is reported only
    /// for a status that was measured; a `not_measured` row carries no lag
    /// rather than a stale one.
    #[test]
    fn lag_json_reports_the_pending_seconds_behind_only_when_measured() {
        let now = at(1_000);
        let mut state = lag_state(Some(now - time::Duration::seconds(10)), "behind");
        state.pending_since = Some(now - time::Duration::seconds(125));
        let res = lag_json(
            Some(TargetLag::Lag { minutes: 5 }),
            Some(&state),
            now,
            false,
            0,
            None,
        );
        assert_eq!(res["status"], "behind");
        assert_eq!(res["lagSeconds"], 125);
        assert_eq!(res["reason"], "past the target lag");

        state.last_status = Some("not_measured".to_owned());
        let unmeasured = lag_json(
            Some(TargetLag::Lag { minutes: 5 }),
            Some(&state),
            now,
            false,
            0,
            None,
        );
        assert_eq!(unmeasured["status"], "not_measured");
        assert_eq!(unmeasured["lagSeconds"], serde_json::Value::Null);
    }

    /// A pipeline that has never run under its lag is not "within the
    /// target lag" — it is behind, and says why.
    #[test]
    fn lag_json_says_a_never_consumed_pipeline_is_waiting_for_its_first_run() {
        let now = at(1_000);
        let mut state = lag_state(Some(now - time::Duration::seconds(5)), "behind");
        state.consumed_tokens = None;
        let res = lag_json(
            Some(TargetLag::Lag { minutes: 5 }),
            Some(&state),
            now,
            false,
            0,
            None,
        );
        assert_eq!(
            res["reason"],
            "waiting for this pipeline's first successful run"
        );
    }

    /// Plan D7's note: a `downstream` pipeline nobody reads cannot report
    /// a lag, because nothing asks it to move.
    #[test]
    fn lag_json_reports_the_downstream_note_for_a_pipeline_with_no_readers() {
        let now = at(1_000);
        let state = lag_state(Some(now - time::Duration::seconds(10)), "met");
        let res = lag_json(
            Some(TargetLag::Downstream),
            Some(&state),
            now,
            true,
            0,
            None,
        );
        assert_eq!(res["status"], "not_measured");
        assert_eq!(
            res["reason"],
            "Downstream: no pipeline with a target lag reads this table"
        );
        assert_eq!(res["lagSeconds"], serde_json::Value::Null);

        let read = lag_json(
            Some(TargetLag::Downstream),
            Some(&state),
            now,
            false,
            0,
            None,
        );
        assert_eq!(
            read["status"], "met",
            "a downstream pipeline somebody reads reports what its own check measured"
        );
    }

    /// Plan D9: the alert fires on the check that completes the streak, so
    /// once per streak and never twice for it.
    // BLD-2 review R3 BLOCKER 1 / decision R14: lag_json derives retries exhausted
    #[test]
    fn lag_json_reports_retries_exhausted_when_attempts_exceed_max_retries_even_with_low_lag() {
        let now = at(1_000);
        let mut state = lag_state(Some(now - time::Duration::seconds(10)), "behind");
        state.pending_since = Some(now - time::Duration::seconds(30));
        state.request_attempt = 2;
        state.consumed_tokens = Some(tokens_to_json(&tokens_of(&[(
            "bronze.orders",
            "iceberg:7",
        )])));
        state.requested_tokens = Some(tokens_to_json(&tokens_of(&[(
            "bronze.orders",
            "iceberg:8",
        )])));
        let res = lag_json(
            Some(TargetLag::Lag { minutes: 10 }),
            Some(&state),
            now,
            false,
            1, // max_retries
            None,
        );
        assert_eq!(res["status"], "behind");
        assert_eq!(res["lagSeconds"], 30);
        assert_eq!(
            res["reason"],
            "the last 2 runs for this change failed; the next input change starts again"
        );
    }

    /// BLD-2 review R5 SHOULD-FIX 7: the count and its plural come from one
    /// helper, so "last 1 run" is what every exhausted reason says when a
    /// pipeline is allowed no retry at all (`max_retries: 0` → one failed
    /// run) — the default text, not an edge case.
    #[test]
    fn the_failed_run_count_is_written_with_its_plural() {
        assert_eq!(super::failed_runs(1), "1 run");
        assert_eq!(super::failed_runs(2), "2 runs");
        assert_eq!(super::failed_runs(3), "3 runs");
    }

    // BLD-2 review R3 BLOCKER 1 / decision R14: lag_json derives waiting on exhausted upstream
    // BLD-2 review R4 BLOCKER 1 (b): the exhausted-U reason is its own
    // sentence, naming the count from U's own `max_retries` — saying
    // "waiting ... to finish" would claim a check can still unblock it.
    #[test]
    fn lag_json_reports_the_exhausted_upstream_whose_runs_failed_when_upstream_retries_exhausted() {
        let now = at(1_000);
        let mut state = lag_state(Some(now - time::Duration::seconds(10)), "behind");
        state.pending_since = Some(now - time::Duration::seconds(30));
        state.request_attempt = 0;
        state.consumed_tokens = Some(tokens_to_json(&tokens_of(&[("silver.clean", "iceberg:1")])));
        let res = lag_json(
            Some(TargetLag::Lag { minutes: 10 }),
            Some(&state),
            now,
            false,
            1,
            Some(&UpstreamStop {
                pipeline_id: "pl-upstream-orders".to_owned(),
                failed_runs: Some(2),
            }),
        );
        assert_eq!(res["status"], "behind");
        assert_eq!(res["lagSeconds"], 30);
        assert_eq!(
            res["reason"],
            "upstream pipeline pl-upstream-orders's last 2 runs for this change failed; \
             the next input change starts again"
        );
    }

    #[test]
    fn the_behind_target_alert_fires_exactly_at_the_third_check() {
        for streak in 0..=2 {
            assert!(!behind_alert_due(streak), "streak {streak} must not alert");
        }
        assert!(behind_alert_due(3), "the third check in a row alerts");
        for streak in 4..=9 {
            assert!(
                !behind_alert_due(streak),
                "streak {streak} must not alert again for the same streak"
            );
        }
        assert!(
            !behind_alert_due(0),
            "a met check resets the streak, so this never wraps into an alert"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn downstream_without_readers_names_only_the_table_nobody_reads(pool: PgPool) {
        let tenant_id = seed_tenant(&pool).await;
        let created_orphan = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "pl-orphan".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "silver".to_owned(),
                source_table: "upstream_table".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "orphan_table".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-orphan".to_owned()),
                target_lag: Some(TargetLag::Downstream),
                // BLD-1 / D1: the fixture is about downstream readers, not
                // about a pipeline author.
                run_as: None,
            },
            None,
        )
        .await
        .expect("seed the downstream pipeline");
        let created_wanted = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "pl-wanted".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "silver".to_owned(),
                source_table: "reader_source".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "wanted_table".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-wanted".to_owned()),
                target_lag: Some(TargetLag::Downstream),
                // BLD-1 / D1: the fixture is about downstream readers, not
                // about a pipeline author.
                run_as: None,
            },
            None,
        )
        .await
        .expect("seed the second downstream pipeline");
        // The reader now reads the wanted table, so only the orphan is
        // without one.
        pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "pl-consumer".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "silver".to_owned(),
                source_table: "wanted_table".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "gold".to_owned(),
                target_table: "consumer_out".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-consumer".to_owned()),
                target_lag: Some(TargetLag::Lag { minutes: 5 }),
                // BLD-1 / D1: the fixture is about downstream readers, not
                // about a pipeline author.
                run_as: None,
            },
            None,
        )
        .await
        .expect("seed the pipeline that reads the wanted table");

        let candidates = vec![created_orphan.clone(), created_wanted.clone()];
        let without = downstream_without_readers(&pool, &candidates)
            .await
            .expect("read the downstream readers");
        assert_eq!(
            without,
            std::collections::HashSet::from([created_orphan.id.clone()]),
            "only the pipeline nothing reads is without readers"
        );
    }

    /// BLD-2 review R1 BLOCKER 2: `settle_request` checks the found run's
    /// `jobName` is the job that was requested.
    /// If Dagster has a run for the key but with a different jobName, ignore it and treat as not created.
    #[sqlx::test(migrations = "../../migrations")]
    async fn settle_request_ignores_run_with_different_job_name(pool: PgPool) {
        let pipeline = seed_pipeline(&pool, "pl-wrong-job", 10).await;
        let now = at(1_000);
        let tokens = tokens_of(&[("bronze.orders", "iceberg:8")]);
        let key = run_key_for("pl-wrong-job", &tokens, 0);
        update_in(
            &pool,
            &pipeline,
            Some(tokens_of(&[("bronze.orders", "iceberg:7")])),
            Some(now),
            Some((&key, tokens.clone())),
            "behind",
            1,
            now,
        )
        .await;
        let reader = FakeTokens::new().with("bronze.orders", "iceberg:8");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "pipelineRunsOrError": {
                        "__typename": "Runs",
                        "results": [{
                            "runId": "r-foreign",
                            "jobName": "authored__foreign",
                            "status": "SUCCESS",
                            "startTime": 1_000.0,
                            "endTime": 1_060.0,
                        }]
                    },
                    "runsOrError": { "__typename": "Runs", "results": [] }
                }
            })))
            .mount(&server)
            .await;
        let dagster = DgClient::new(format!("{}/graphql", server.uri()));

        let outcome = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            now + time::Duration::seconds(30),
        )
        .await
        .expect("check succeeds");

        assert_eq!(outcome.decision, Decision::Wait);
        let stored = pipelines::get_lag_state(&pool, "pl-wrong-job")
            .await
            .expect("read state")
            .expect("row exists");
        assert_eq!(
            stored.requested_run_key.as_deref(),
            Some(key.as_str()),
            "request remains outstanding"
        );
        let consumed = tokens_from_json(stored.consumed_tokens.as_ref()).expect("consumed map");
        assert_eq!(
            consumed.get("bronze.orders").map(String::as_str),
            Some("iceberg:7"),
            "not consumed"
        );
    }

    /// BLD-2 review R1 decision R1 / BLOCKER 3: a requested run that Dagster does not have after
    /// 2 minutes (120s / 4 checks) is treated as lost: clear the request and decide again on same tick.
    #[sqlx::test(migrations = "../../migrations")]
    async fn lost_request_is_cleared_after_two_minutes_and_decided_again(pool: PgPool) {
        let pipeline = seed_pipeline(&pool, "pl-lost", 10).await;
        let t0 = at(1_000);
        let tokens = tokens_of(&[("bronze.orders", "iceberg:8")]);
        let key = run_key_for("pl-lost", &tokens, 0);
        update_in(
            &pool,
            &pipeline,
            Some(tokens_of(&[("bronze.orders", "iceberg:7")])),
            Some(t0),
            Some((&key, tokens.clone())),
            "behind",
            1,
            t0,
        )
        .await;
        let reader = FakeTokens::new().with("bronze.orders", "iceberg:8");
        let (dagster, _server) = dagster_with(&job_name("pl-lost"), None, &[]).await;

        let waiting = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            t0 + time::Duration::seconds(60),
        )
        .await
        .expect("check succeeds");
        assert_eq!(waiting.decision, Decision::Wait);
        assert_eq!(waiting.run_key.as_deref(), Some(key.as_str()));

        // Past the 2-minute lost-request window AND past the target, so
        // the tick that follows the clear is one that owes a run anyway:
        // the point of the assertion is that the lost request stops
        // blocking, not that the batching window is ignored.
        let decided_again = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            t0 + time::Duration::seconds(900),
        )
        .await
        .expect("check succeeds");
        assert_eq!(
            decided_again.decision,
            Decision::Run,
            "decides to run again after lost request is cleared"
        );
        let new_key = decided_again.run_key.expect("a new run key");
        // BLD-2 review R2 decision R9: lost request does not count toward attempt
        assert!(
            new_key.ends_with(":0"),
            "the lost request is not an attempt, so the new key remains attempt 0: {new_key}"
        );
    }

    /// BLD-2 review R1 decision R2 / BLOCKER 4: retry with real derived key carrying attempt,
    /// and stop requesting after maxRetries + 1 attempts leaving status behind.
    #[sqlx::test(migrations = "../../migrations")]
    async fn failed_run_retries_with_real_derived_key_and_stops_at_max_retries(pool: PgPool) {
        let pipe_input = pipelines::CreatePipelineInput {
            name: "pl-retry".to_owned(),
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
            max_retries: Some(1),
            tenant_id: None,
            depends_on: Vec::new(),
            connector_id: None,
            id: Some("pl-retry".to_owned()),
            target_lag: Some(TargetLag::Lag { minutes: 10 }),
            run_as: None,
        };
        let pipeline = pipelines::create_pipeline(&pool, &pipe_input, None)
            .await
            .expect("create pipeline");
        assert_eq!(pipeline.max_retries, 1);

        let reader = FakeTokens::new().with("bronze.orders", "iceberg:8");
        let t0 = at(1_000);

        let (dagster_empty, _s1) = dagster_with(&job_name("pl-retry"), None, &[]).await;
        let c1 = run_lag_check(
            &pool,
            &dagster_empty,
            &reader,
            &pipeline,
            &[source_input()],
            t0,
        )
        .await
        .expect("check 1");
        assert_eq!(c1.decision, Decision::Run);
        let key0 = c1.run_key.expect("key 0");
        assert!(
            key0.ends_with(":0"),
            "first attempt key must end with :0, got {key0}"
        );

        let (dagster_fail, _s2) = dagster_with(&job_name("pl-retry"), Some("FAILURE"), &[]).await;
        let c2 = run_lag_check(
            &pool,
            &dagster_fail,
            &reader,
            &pipeline,
            &[source_input()],
            t0 + time::Duration::seconds(30),
        )
        .await
        .expect("check 2");
        assert_eq!(
            c2.decision,
            Decision::Run,
            "retries on next check with attempt 1"
        );
        let key1 = c2.run_key.expect("key 1");
        assert!(
            key1.ends_with(":1"),
            "second attempt key must end with :1, got {key1}"
        );

        let c3 = run_lag_check(
            &pool,
            &dagster_fail,
            &reader,
            &pipeline,
            &[source_input()],
            t0 + time::Duration::seconds(60),
        )
        .await
        .expect("check 3");
        assert_eq!(
            c3.decision,
            Decision::Wait,
            "stops requesting after max_retries + 1 attempts"
        );
        assert_eq!(c3.status, LagStatus::Behind, "status stays behind");
        assert_eq!(
            c3.reason.as_deref(),
            Some("the last 2 runs for this change failed; the next input change starts again")
        );
        assert!(c3.run_key.is_none(), "no run key requested");

        // BLD-2 review R2 decision R10 / SHOULD-FIX 5: consumed-before case.
        // Even when lag (30s) is within target (10m = 600s), exhausted retries
        // must read behind with the exhausted reason, not met.
        let pipe_consumed = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "pl-retry-consumed".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "orders".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "orders_clean2".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: Some(1),
                tenant_id: None,
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-retry-consumed".to_owned()),
                target_lag: Some(TargetLag::Lag { minutes: 10 }),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create pipeline");

        let t_past = t0 - time::Duration::seconds(30);
        let old_tokens = tokens_of(&[("bronze.orders", "iceberg:7")]);
        let new_tokens = tokens_of(&[("bronze.orders", "iceberg:8")]);
        let update_consumed = LagStateUpdate {
            pipeline_id: pipe_consumed.id.clone(),
            tenant_id: pipe_consumed.tenant_id,
            consumed_tokens: Some(tokens_to_json(&old_tokens)),
            consumed_at: Some(t_past),
            pending_since: Some(t_past),
            requested_run_key: None,
            requested_tokens: Some(tokens_to_json(&new_tokens)),
            requested_at: None,
            request_attempt: 2,
            last_check_at: t0,
            last_status: "behind".to_owned(),
            behind_streak: 2,
        };
        pipelines::upsert_lag_state(&pool, &update_consumed)
            .await
            .unwrap();

        let (dagster_fail2, _s3) = dagster_with(&job_name("pl-retry-consumed"), None, &[]).await;
        let c_consumed = run_lag_check(
            &pool,
            &dagster_fail2,
            &reader,
            &pipe_consumed,
            &[source_input()],
            t0,
        )
        .await
        .expect("check consumed-before with retries exhausted");

        assert_eq!(c_consumed.decision, Decision::Wait);
        assert_eq!(
            c_consumed.status,
            LagStatus::Behind,
            "retries exhausted reads behind even when lag is within target"
        );
        assert_eq!(
            c_consumed.reason.as_deref(),
            Some("the last 2 runs for this change failed; the next input change starts again")
        );

        // BLD-2 review R3 BLOCKER 1 / decision R14: test the page payload for the case the R10 test uses
        let state_consumed = pipelines::get_lag_state(&pool, &pipe_consumed.id)
            .await
            .unwrap();
        let page = lag_json(
            pipe_consumed.target_lag,
            state_consumed.as_ref(),
            t0,
            false,
            i32::from(pipe_consumed.max_retries),
            None,
        );
        assert_eq!(page["status"], "behind");
        assert_eq!(page["lagSeconds"], 30);
        assert_eq!(
            page["reason"],
            "the last 2 runs for this change failed; the next input change starts again"
        );
    }

    /// BLD-2 review R3 BLOCKER 1 / decision R14: For a P waiting on an exhausted U,
    /// P is behind with a reason naming U in both check outcome and page payload.
    #[sqlx::test(migrations = "../../migrations")]
    async fn downstream_waiting_on_exhausted_upstream_reads_behind_with_reason_naming_u(
        pool: PgPool,
    ) {
        let tenant_id = seed_tenant(&pool).await;
        let u = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "u_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "orders".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "u_clean".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: Some(1),
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-u-exh".to_owned()),
                target_lag: Some(TargetLag::Downstream),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create u");

        let p = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "p_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "silver".to_owned(),
                source_table: "u_clean".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "gold".to_owned(),
                target_table: "p_mart".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: Some(1),
                tenant_id: Some(tenant_id),
                depends_on: vec!["pl-u-exh".to_owned()],
                connector_id: None,
                id: Some("pl-p-exh".to_owned()),
                target_lag: Some(TargetLag::Lag { minutes: 10 }),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create p");

        let t0 = at(1_000);
        let t_past = t0 - time::Duration::seconds(30);
        let u_old = tokens_of(&[("bronze.orders", "iceberg:1")]);
        let u_new = tokens_of(&[("bronze.orders", "iceberg:2")]);
        // Set U to exhausted: request_attempt = 2 > max_retries(1), requested != consumed
        let update_u = LagStateUpdate {
            pipeline_id: u.id.clone(),
            tenant_id: u.tenant_id,
            consumed_tokens: Some(tokens_to_json(&u_old)),
            consumed_at: Some(t_past),
            pending_since: Some(t_past),
            requested_run_key: None,
            requested_tokens: Some(tokens_to_json(&u_new)),
            requested_at: None,
            request_attempt: 2,
            last_check_at: t0,
            last_status: "not_measured".to_owned(),
            behind_streak: 0,
        };
        pipelines::upsert_lag_state(&pool, &update_u).await.unwrap();

        // P has consumed silver.u_clean before, lag is 30s <= target 600s
        let p_tokens = tokens_of(&[("silver.u_clean", "fp:1")]);
        let update_p = LagStateUpdate {
            pipeline_id: p.id.clone(),
            tenant_id: p.tenant_id,
            consumed_tokens: Some(tokens_to_json(&p_tokens)),
            consumed_at: Some(t_past),
            pending_since: Some(t_past),
            requested_run_key: None,
            requested_tokens: None,
            requested_at: None,
            request_attempt: 0,
            last_check_at: t0,
            last_status: "met".to_owned(),
            behind_streak: 0,
        };
        pipelines::upsert_lag_state(&pool, &update_p).await.unwrap();

        let reader = FakeTokens::new()
            .with("bronze.orders", "iceberg:2")
            .with("silver.u_clean", "fp:1");
        let (dagster, _s) = dagster_with(&job_name(&p.id), None, &[]).await;
        let p_inputs = inputs_of(&pool, &p).await.unwrap().unwrap();
        let outcome = run_lag_check(&pool, &dagster, &reader, &p, &p_inputs, t0)
            .await
            .expect("run check");

        assert_eq!(outcome.decision, Decision::Wait);
        assert_eq!(
            outcome.status,
            LagStatus::Behind,
            "P waiting on exhausted U must be behind"
        );
        assert_eq!(
            outcome.reason.as_deref(),
            Some(
                "upstream pipeline pl-u-exh's last 2 runs for this change failed; \
                 the next input change starts again"
            ),
            "the check's exhausted-U reason names U and the count, R4 BLOCKER 1 (b)"
        );

        // Page payload for P
        let p_state = pipelines::get_lag_state(&pool, &p.id).await.unwrap();
        let page = lag_json(
            p.target_lag,
            p_state.as_ref(),
            t0,
            false,
            i32::from(p.max_retries),
            Some(&UpstreamStop {
                pipeline_id: "pl-u-exh".to_owned(),
                failed_runs: Some(2),
            }),
        );
        assert_eq!(page["status"], "behind");
        assert_eq!(
            page["reason"],
            "upstream pipeline pl-u-exh's last 2 runs for this change failed; \
             the next input change starts again",
            "the page says the same words the check said"
        );
    }

    /// BLD-2 review R3 BLOCKER 1: a U whose run is merely OUTSTANDING (not exhausted)
    /// does not make P `behind`: P is judged by its own lag (30 s) against its
    /// target (600 s), so it stays `met`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn downstream_waiting_on_an_outstanding_upstream_run_stays_met_inside_its_target(
        pool: PgPool,
    ) {
        let tenant_id = seed_tenant(&pool).await;
        let u = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "u_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "orders".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "u_clean".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: Some(1),
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-u-out".to_owned()),
                target_lag: Some(TargetLag::Downstream),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create u");

        let p = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "p_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "silver".to_owned(),
                source_table: "u_clean".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "gold".to_owned(),
                target_table: "p_mart".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: Some(1),
                tenant_id: Some(tenant_id),
                depends_on: vec!["pl-u-out".to_owned()],
                connector_id: None,
                id: Some("pl-p-out".to_owned()),
                target_lag: Some(TargetLag::Lag { minutes: 10 }),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create p");

        let t0 = at(1_000);
        let t_past = t0 - time::Duration::seconds(30);
        let u_old = tokens_of(&[("bronze.orders", "iceberg:1")]);
        let u_new = tokens_of(&[("bronze.orders", "iceberg:2")]);
        // Set U to outstanding: a run is requested but not yet consumed
        // (request_attempt = 0, within max_retries(1); requested_tokens !=
        // consumed_tokens). The exhausted case (attempt > max_retries) has
        // its own test above — BLD-2 review R4 SHOULD-FIX 5.
        let update_u = LagStateUpdate {
            pipeline_id: u.id.clone(),
            tenant_id: u.tenant_id,
            consumed_tokens: Some(tokens_to_json(&u_old)),
            consumed_at: Some(t_past),
            pending_since: Some(t_past),
            requested_run_key: Some("lag:pl-u-out:abc:0".to_owned()),
            requested_tokens: Some(tokens_to_json(&u_new)),
            requested_at: Some(t_past),
            request_attempt: 0,
            last_check_at: t0,
            last_status: "not_measured".to_owned(),
            behind_streak: 0,
        };
        pipelines::upsert_lag_state(&pool, &update_u).await.unwrap();

        // P has consumed silver.u_clean before, lag is 30s <= target 600s
        let p_tokens = tokens_of(&[("silver.u_clean", "fp:1")]);
        let update_p = LagStateUpdate {
            pipeline_id: p.id.clone(),
            tenant_id: p.tenant_id,
            consumed_tokens: Some(tokens_to_json(&p_tokens)),
            consumed_at: Some(t_past),
            pending_since: Some(t_past),
            requested_run_key: None,
            requested_tokens: None,
            requested_at: None,
            request_attempt: 0,
            last_check_at: t0,
            last_status: "met".to_owned(),
            behind_streak: 0,
        };
        pipelines::upsert_lag_state(&pool, &update_p).await.unwrap();

        let reader = FakeTokens::new()
            .with("bronze.orders", "iceberg:2")
            .with("silver.u_clean", "fp:1");
        let (dagster, _s) = dagster_with(&job_name(&u.id), Some("STARTED"), &[]).await;
        let p_inputs = inputs_of(&pool, &p).await.unwrap().unwrap();
        let outcome = run_lag_check(&pool, &dagster, &reader, &p, &p_inputs, t0)
            .await
            .expect("run check");

        assert_eq!(outcome.decision, Decision::Wait);
        assert_eq!(
            outcome.status,
            LagStatus::Met,
            "an outstanding U run is not an exhausted U"
        );
        assert_eq!(
            outcome.reason.as_deref(),
            Some("waiting for upstream pipeline pl-u-out to finish")
        );
        // BLD-2 review R3 SHOULD-FIX 3: U's check is recorded while its run is
        // outstanding, and a U (no target) is never `met`.
        let u_row = pipelines::get_lag_state(&pool, &u.id)
            .await
            .unwrap()
            .expect("u row");
        assert_eq!(u_row.last_check_at, Some(t0));
        assert_eq!(u_row.last_status.as_deref(), Some("not_measured"));
    }

    // BLD-2 review R3 BLOCKER 2 / decision R13:
    // A fake Dagster that never answers makes the check return 503 within the budget
    // and leaves the state row unchanged.
    #[sqlx::test(migrations = "../../migrations")]
    async fn unanswering_dagster_times_out_within_budget_returns_503_and_leaves_row_unchanged(
        pool: PgPool,
    ) {
        let pipeline = seed_pipeline(&pool, "pl-hang", 10).await;
        let t0 = at(1_000);
        let old_tokens = tokens_of(&[("bronze.orders", "iceberg:1")]);
        let initial_update = LagStateUpdate {
            pipeline_id: pipeline.id.clone(),
            tenant_id: pipeline.tenant_id,
            consumed_tokens: Some(tokens_to_json(&old_tokens)),
            consumed_at: Some(t0),
            pending_since: None,
            requested_run_key: Some("lag:pl-hang:abc:0".to_owned()),
            requested_tokens: Some(tokens_to_json(&old_tokens)),
            requested_at: Some(t0),
            request_attempt: 0,
            last_check_at: t0,
            last_status: "met".to_owned(),
            behind_streak: 0,
        };
        pipelines::upsert_lag_state(&pool, &initial_update)
            .await
            .expect("upsert initial state");

        // Mock Dagster server that delays response beyond the 8s budget
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(12)))
            .mount(&server)
            .await;

        let dagster = DgClient::new(format!("{}/graphql", server.uri()));
        let reader = FakeTokens::new().with("bronze.orders", "iceberg:2");
        let inputs = inputs_of(&pool, &pipeline).await.unwrap().unwrap();

        let start = std::time::Instant::now();
        let err = run_lag_check(&pool, &dagster, &reader, &pipeline, &inputs, at(1_030))
            .await
            .expect_err("check must time out and return 503");
        let elapsed = start.elapsed();

        assert!(
            elapsed >= std::time::Duration::from_secs(8)
                && elapsed < std::time::Duration::from_secs(10),
            "timeout must happen at the 8s budget (elapsed: {elapsed:?})"
        );
        match err {
            ApiError::Unavailable(msg) => {
                assert_eq!(msg, "the orchestrator is unavailable");
            }
            other => panic!("expected 503 Unavailable, got {other:?}"),
        }

        // Verify state row is completely unchanged (rollback succeeded)
        let state_after = pipelines::get_lag_state(&pool, &pipeline.id)
            .await
            .unwrap()
            .expect("state row exists");
        assert_eq!(state_after.last_status.as_deref(), Some("met"));
        assert_eq!(state_after.last_check_at, Some(t0));
        assert_eq!(
            state_after.requested_run_key.as_deref(),
            Some("lag:pl-hang:abc:0")
        );
    }

    /// BLD-2 review R1 BLOCKER 5: downstream keeps P's outstanding `requested_run_key`
    /// even when upstream U has work to do.
    #[sqlx::test(migrations = "../../migrations")]
    async fn downstream_preserves_outstanding_request_when_upstream_has_work(pool: PgPool) {
        let tenant_id = seed_tenant(&pool).await;
        let _u = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "u_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "orders".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "u_clean".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-u-b5".to_owned()),
                target_lag: Some(TargetLag::Downstream),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create u");

        let p = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "p_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "silver".to_owned(),
                source_table: "u_clean".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "gold".to_owned(),
                target_table: "p_mart".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-p-b5".to_owned()),
                target_lag: Some(TargetLag::Lag { minutes: 10 }),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create p");

        let t0 = at(1_000);
        let p_tokens = tokens_of(&[("silver.u_clean", "fp:1")]);
        let p_req_key = run_key_for("pl-p-b5", &p_tokens, 0);
        update_in(
            &pool,
            &p,
            None,
            Some(t0),
            Some((&p_req_key, p_tokens)),
            "behind",
            1,
            t0,
        )
        .await;

        let p_inputs = inputs_of(&pool, &p)
            .await
            .expect("p inputs")
            .expect("p parseable");
        let reader = FakeTokens::new()
            .with("bronze.orders", "iceberg:2")
            .with("silver.u_clean", "fp:1");

        let (dagster, _server) = dagster_with(&job_name("pl-u-b5"), Some("STARTED"), &[]).await;

        let outcome = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &p,
            &p_inputs,
            t0 + time::Duration::seconds(30),
        )
        .await
        .expect("check p");

        assert_eq!(outcome.decision, Decision::Run);
        assert_eq!(outcome.job_name, "authored__pl_u_b5");

        let stored_p = pipelines::get_lag_state(&pool, "pl-p-b5")
            .await
            .expect("read p state")
            .expect("p row exists");
        assert_eq!(
            stored_p.requested_run_key.as_deref(),
            Some(p_req_key.as_str()),
            "P's outstanding requested_run_key must NOT be cleared when U is requested"
        );
    }

    /// BLD-2 review R1 SHOULD-FIX 3: downstream pipeline read only through dependsOn
    /// is recognized as read by `count_lag_pipelines_reading` and
    /// `downstream_without_readers`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn downstream_read_only_through_depends_on_is_recognized_as_read(pool: PgPool) {
        let tenant_id = seed_tenant(&pool).await;
        let u = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "u_dep_read".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "orders".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "u_dep_target".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-u-dep".to_owned()),
                target_lag: Some(TargetLag::Downstream),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create u");

        let _reader = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "reader_dep".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "silver".to_owned(),
                source_table: "other_table".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "gold".to_owned(),
                target_table: "reader_out".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: vec!["pl-u-dep".to_owned()],
                connector_id: None,
                id: Some("pl-reader-dep".to_owned()),
                target_lag: Some(TargetLag::Lag { minutes: 10 }),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create reader");

        let without = downstream_without_readers(&pool, std::slice::from_ref(&u))
            .await
            .expect("downstream without readers");
        assert!(
            !without.contains("pl-u-dep"),
            "pipeline read via depends_on must NOT be reported as without readers"
        );
    }

    /// BLD-2 review R1 SHOULD-FIX 1: When downstream input cannot be read,
    /// write P's state as `not_measured` and reset the streak to 0.
    #[sqlx::test(migrations = "../../migrations")]
    async fn downstream_unreadable_token_persists_not_measured(pool: PgPool) {
        let tenant_id = seed_tenant(&pool).await;
        let _u = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "u_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "orders".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "u_clean_unreadable".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-u-unreadable".to_owned()),
                target_lag: Some(TargetLag::Downstream),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create u");

        let p = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "p_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "silver".to_owned(),
                source_table: "u_clean_unreadable".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "gold".to_owned(),
                target_table: "p_mart_unreadable".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-p-unreadable".to_owned()),
                target_lag: Some(TargetLag::Lag { minutes: 10 }),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create p");

        let t0 = at(1_000);
        update_in(
            &pool,
            &p,
            Some(tokens_of(&[("silver.u_clean_unreadable", "fp:1")])),
            None,
            None,
            "met",
            0,
            t0,
        )
        .await;

        let p_inputs = inputs_of(&pool, &p)
            .await
            .expect("p inputs")
            .expect("p parseable");
        let reader = FakeTokens::new().unreadable("bronze.orders");
        let (dagster, _server) = dagster_with(&job_name("pl-p-unreadable"), None, &[]).await;

        let outcome = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &p,
            &p_inputs,
            t0 + time::Duration::seconds(30),
        )
        .await
        .expect("check succeeds");

        assert_eq!(outcome.status, LagStatus::NotMeasured);
        assert_eq!(outcome.behind_streak, 0);

        let stored_p = pipelines::get_lag_state(&pool, "pl-p-unreadable")
            .await
            .expect("read p state")
            .expect("p row exists");
        assert_eq!(
            stored_p.last_status.as_deref(),
            Some("not_measured"),
            "P's state must be updated to not_measured, not remain old status"
        );
        assert_eq!(stored_p.behind_streak, 0);
    }

    /// BLD-2 review R1 SHOULD-FIX 12: drive three consecutive behind checks
    /// through `run_lag_check` to the alert
    #[sqlx::test(migrations = "../../migrations")]
    async fn three_consecutive_behind_checks_drive_alert_due(pool: PgPool) {
        let pipeline = seed_pipeline(&pool, "pl-streak-alert", 5).await;
        let reader = FakeTokens::new().with("bronze.orders", "iceberg:1");
        let (dagster, _server) =
            dagster_with(&job_name("pl-streak-alert"), Some("STARTED"), &[]).await;
        let t0 = at(1_000);

        let outcome1 = run_lag_check(&pool, &dagster, &reader, &pipeline, &[source_input()], t0)
            .await
            .expect("check 1 succeeds");
        assert_eq!(outcome1.status, LagStatus::Behind);
        assert_eq!(outcome1.behind_streak, 1);
        assert!(!behind_alert_due(outcome1.behind_streak));

        let outcome2 = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            t0 + time::Duration::seconds(30),
        )
        .await
        .expect("check 2 succeeds");
        assert_eq!(outcome2.status, LagStatus::Behind);
        assert_eq!(outcome2.behind_streak, 2);
        assert!(!behind_alert_due(outcome2.behind_streak));

        let outcome3 = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            t0 + time::Duration::seconds(60),
        )
        .await
        .expect("check 3 succeeds");
        assert_eq!(outcome3.status, LagStatus::Behind);
        assert_eq!(outcome3.behind_streak, 3);
        assert!(
            behind_alert_due(outcome3.behind_streak),
            "the third consecutive behind check must trigger the alert"
        );

        let outcome4 = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &pipeline,
            &[source_input()],
            t0 + time::Duration::seconds(90),
        )
        .await
        .expect("check 4 succeeds");
        assert_eq!(outcome4.status, LagStatus::Behind);
        assert_eq!(outcome4.behind_streak, 4);
        assert!(
            !behind_alert_due(outcome4.behind_streak),
            "subsequent behind checks do not re-alert"
        );
    }

    /// BLD-2 review R2 BLOCKER 2 / BLOCKER 3: downstream upstream U whose run keeps failing:
    /// increments `request_attempt`, stops requesting after `max_retries` + 1, persists in tx,
    /// and resets attempt when U tokens change.
    #[sqlx::test(migrations = "../../migrations")]
    async fn upstream_retries_exhausted_stops_requesting_and_resets_on_token_change(pool: PgPool) {
        let (dagster, _server) =
            dagster_with(&job_name("pl-u-retries"), Some("FAILURE"), &[]).await;
        let tenant_id = seed_tenant(&pool).await;

        let u = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "u_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "orders".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "u_clean_retries".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: Some(1),
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-u-retries".to_owned()),
                target_lag: Some(TargetLag::Downstream),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create u");

        let p = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "p_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "silver".to_owned(),
                source_table: "u_clean_retries".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "gold".to_owned(),
                target_table: "p_mart_retries".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-p-retries".to_owned()),
                target_lag: Some(TargetLag::Lag { minutes: 10 }),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create p");

        let p_inputs = inputs_of(&pool, &p)
            .await
            .expect("p inputs")
            .expect("p parseable");
        let reader = FakeTokens::new()
            .with("bronze.orders", "iceberg:1")
            .with("silver.u_clean_retries", "fp:1");

        let now = at(1_000);

        // Attempt 0: initial run requested for U
        let o0 = run_lag_check(&pool, &dagster, &reader, &p, &p_inputs, now)
            .await
            .expect("check 0");
        assert_eq!(o0.decision, Decision::Run);
        assert_eq!(o0.job_name, "authored__pl_u_retries");
        let key0 = o0.run_key.expect("key0");
        assert!(
            key0.ends_with(":0"),
            "initial key should have :0 suffix: {key0}"
        );

        // Attempt 1: Dagster settled FAILURE -> attempt incremented to 1 <= max_retries(1)
        let o1 = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &p,
            &p_inputs,
            now + time::Duration::seconds(30),
        )
        .await
        .expect("check 1");
        assert_eq!(o1.decision, Decision::Run);
        let key1 = o1.run_key.expect("key1");
        assert!(
            key1.ends_with(":1"),
            "first retry should have :1 suffix: {key1}"
        );

        // Attempt 2: Dagster settled FAILURE -> attempt incremented to 2 > max_retries(1) -> exhausted!
        let o2 = run_lag_check(
            &pool,
            &dagster,
            &reader,
            &p,
            &p_inputs,
            now + time::Duration::seconds(60),
        )
        .await
        .expect("check 2");
        assert_eq!(o2.decision, Decision::Wait);
        assert_eq!(o2.run_key, None);
        let u_state = pipelines::get_lag_state(&pool, &u.id)
            .await
            .expect("get u state")
            .expect("u state exists");
        assert_eq!(u_state.request_attempt, 2);
        assert_eq!(u_state.requested_run_key, None);
        assert_eq!(u_state.last_status.as_deref(), Some("not_measured")); // SHOULD-FIX 1: no target -> not_measured

        // Tokens change! U tokens become iceberg:2 -> attempt resets to 0!
        let reader2 = FakeTokens::new()
            .with("bronze.orders", "iceberg:2")
            .with("silver.u_clean_retries", "fp:1");
        let o3 = run_lag_check(
            &pool,
            &dagster,
            &reader2,
            &p,
            &p_inputs,
            now + time::Duration::seconds(90),
        )
        .await
        .expect("check 3");
        assert_eq!(o3.decision, Decision::Run);
        let key3 = o3.run_key.expect("key3");
        assert!(
            key3.ends_with(":0"),
            "reset attempt should have :0 suffix: {key3}"
        );
        let u_state_after = pipelines::get_lag_state(&pool, &u.id)
            .await
            .expect("get u state")
            .expect("u state exists");
        assert_eq!(u_state_after.request_attempt, 0);
    }

    /// BLD-2 review R2 SHOULD-FIX 3: A manual `lag-check` on a `downstream` pipeline answers wait.
    #[sqlx::test(migrations = "../../migrations")]
    async fn manual_lag_check_on_downstream_pipeline_answers_wait(pool: PgPool) {
        let (dagster, _server) = dagster_with(&job_name("pl-u-manual"), None, &[]).await;
        let tenant_id = seed_tenant(&pool).await;

        let u = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "u_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "orders".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "u_clean_manual".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-u-manual".to_owned()),
                target_lag: Some(TargetLag::Downstream),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create u");

        let _p = pipelines::create_pipeline(
            &pool,
            &pipelines::CreatePipelineInput {
                name: "p_pipe".to_owned(),
                kind: "batch".to_owned(),
                source_zone: "silver".to_owned(),
                source_table: "u_clean_manual".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "gold".to_owned(),
                target_table: "p_mart_manual".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: Some(tenant_id),
                depends_on: Vec::new(),
                connector_id: None,
                id: Some("pl-p-manual".to_owned()),
                target_lag: Some(TargetLag::Lag { minutes: 10 }),
                run_as: None,
            },
            None,
        )
        .await
        .expect("create p");

        let u_inputs = inputs_of(&pool, &u)
            .await
            .expect("u inputs")
            .expect("u parseable");
        let reader = FakeTokens::new().with("bronze.orders", "iceberg:1");

        let now = at(1_000);
        let outcome = run_lag_check(&pool, &dagster, &reader, &u, &u_inputs, now)
            .await
            .expect("check on u");

        assert_eq!(outcome.decision, Decision::Wait);
        assert_eq!(outcome.run_key, None);
        assert_eq!(
            outcome.reason.as_deref(),
            Some("downstream pipelines are triggered by their readers")
        );
    }
}
