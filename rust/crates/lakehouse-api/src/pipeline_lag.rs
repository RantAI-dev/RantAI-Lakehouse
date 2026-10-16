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
// BLD-2 review R1 SHOULD-FIX 13: return Ok(None) on unparseable source
// BLD-2 review R1 decision R3 / SHOULD-FIX 9: tenant-scoped
pub async fn inputs_of(
    pool: &PgPool,
    pipeline: &Pipeline,
) -> Result<Option<Vec<InputTable>>, StoreError> {
    let mut inputs = Vec::new();
    let Some(source) = InputTable::parse(&pipeline.source) else {
        return Ok(None);
    };
    inputs.push(source);
    for (id, target) in
        pipelines::targets_for(pool, pipeline.tenant_id, &pipeline.depends_on).await?
    {
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
    Ok(Some(inputs))
}

/// `lag:<pipeline id>:<hash>` — the run key a requested run carries, so the
/// next check can find it (plan D5 step 6). The hash is over the canonical
/// token map, so the SAME change always produces the same key (a retry
/// cannot fan out into two runs) and a DIFFERENT change produces a
/// different one (a genuinely new run is not mistaken for the old
/// request's).
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
struct CheckState {
    consumed_tokens: Option<TokenMap>,
    consumed_at: Option<OffsetDateTime>,
    pending_since: Option<OffsetDateTime>,
    requested_run_key: Option<String>,
    requested_tokens: Option<TokenMap>,
    requested_at: Option<OffsetDateTime>,
    request_attempt: i32,
}

impl CheckState {
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
/// * `FAILURE`/`CANCELED` → the request is cleared and nothing is
///   consumed: the next check sees the same difference and may ask again.
///   (The pipeline's own failure alert and `maxRetries` are what cover a
///   run that keeps failing; this loop must not spin.)
/// * still running, or not created yet → `true`, and the caller waits.
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
        .map_err(|err| {
            tracing::error!(%err, pipeline = %pipeline.id, "target lag: reader count failed");
            crate::upstream_error::report(
                &crate::upstream_error::STORE,
                crate::upstream_error::Class::Unavailable,
                &err.to_string(),
            );
            ApiError::Internal("the request could not be completed".to_owned())
        })?;
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
    }

    // BLD-2 review R1 decision R5 / SHOULD-FIX 7: concurrency: read with SELECT FOR UPDATE in tx
    let mut tx = pool.begin().await.map_err(|err| {
        tracing::error!(%err, pipeline = %pipeline.id, "target lag: transaction begin failed");
        crate::upstream_error::report(
            &crate::upstream_error::STORE,
            crate::upstream_error::Class::Unavailable,
            &err.to_string(),
        );
        ApiError::Internal("the request could not be completed".to_owned())
    })?;

    let stored = pipelines::get_lag_state_for_update(&mut tx, &pipeline.id, pipeline.tenant_id)
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
            requested_at: None,
            request_attempt: 0,
        },
        CheckState::from_row,
    );

    let outstanding = settle_request(dagster, &mut state, &job_name, now)
        .await
        .map_err(|err| {
            tracing::warn!(%err, pipeline = %pipeline.id, "target lag: orchestrator unreachable; skipping this tick");
            crate::upstream_error::report(
                &crate::upstream_error::ORCHESTRATOR_UNREACHABLE,
                crate::upstream_error::Class::Unavailable,
                &err.to_string(),
            );
            ApiError::Unavailable("the orchestrator is unavailable".to_owned())
        })?;

    // Plan D7: walk the chain of upstream downstream pipelines, deepest first.
    let chain = pipelines::upstream_downstream_chain(pool, pipeline)
        .await
        .map_err(|err| {
            tracing::error!(%err, pipeline = %pipeline.id, "target lag: upstream chain failed");
            crate::upstream_error::report(
                &crate::upstream_error::STORE,
                crate::upstream_error::Class::Unavailable,
                &err.to_string(),
            );
            ApiError::Internal("the request could not be completed".to_owned())
        })?;

    if let Some(early) = evaluate_upstream_chain(
        pool,
        &mut tx,
        dagster,
        reader,
        pipeline,
        &chain,
        &mut state,
        stored.as_ref(),
        target_seconds,
        &job_name,
        now,
    )
    .await?
    {
        tx.commit().await.map_err(|err| {
            tracing::error!(%err, pipeline = %pipeline.id, "target lag: transaction commit failed");
            crate::upstream_error::report(
                &crate::upstream_error::STORE,
                crate::upstream_error::Class::Unavailable,
                &err.to_string(),
            );
            ApiError::Internal("the request could not be completed".to_owned())
        })?;
        return Ok(early);
    }

    let mut extra_budget = 0;
    for up in &chain {
        let up_job = crate::routes::authored_pipelines::job_name(&up.id);
        extra_budget += run_budget_seconds(dagster, &up_job).await.map_err(|err| {
            tracing::warn!(%err, "target lag: upstream run budget unreadable");
            crate::upstream_error::report(
                &crate::upstream_error::ORCHESTRATOR_UNREACHABLE,
                crate::upstream_error::Class::Unavailable,
                &err.to_string(),
            );
            ApiError::Unavailable("the orchestrator is unavailable".to_owned())
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
    let update = LagStateUpdate {
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
        last_status: outcome.status.as_str().to_owned(),
        behind_streak,
    };
    pipelines::upsert_lag_state_conn(&mut tx, &update)
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
    tx.commit().await.map_err(|err| {
        tracing::error!(%err, pipeline = %pipeline.id, "target lag: transaction commit failed");
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

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "evaluates chain of upstream downstream pipelines in one coherent loop"
)]
async fn evaluate_upstream_chain(
    pool: &PgPool,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    dagster: &DgClient,
    reader: &impl LagTokenReader,
    pipeline: &Pipeline,
    chain: &[Pipeline],
    state: &mut CheckState,
    stored: Option<&pipelines::PipelineLagState>,
    target_seconds: Option<i32>,
    job_name: &str,
    now: OffsetDateTime,
) -> Result<Option<LagCheckOutcome>, ApiError> {
    for u_pipe in chain {
        let u_stored = pipelines::get_lag_state(pool, &u_pipe.id)
            .await
            .map_err(|err| {
                tracing::error!(%err, pipeline = %u_pipe.id, "target lag: upstream state row unreadable");
                crate::upstream_error::report(
                    &crate::upstream_error::STORE,
                    crate::upstream_error::Class::Unavailable,
                    &err.to_string(),
                );
                ApiError::Internal("the request could not be completed".to_owned())
            })?;
        let mut u_state = u_stored.as_ref().map_or_else(
            || CheckState {
                consumed_tokens: None,
                consumed_at: None,
                pending_since: None,
                requested_run_key: None,
                requested_tokens: None,
                requested_at: None,
                request_attempt: 0,
            },
            CheckState::from_row,
        );
        let u_job_name = crate::routes::authored_pipelines::job_name(&u_pipe.id);
        let u_outstanding =
            settle_request(dagster, &mut u_state, &u_job_name, now)
                .await
                .map_err(|err| {
                    tracing::warn!(%err, pipeline = %u_pipe.id, "target lag: orchestrator unreachable; skipping this tick");
                    crate::upstream_error::report(
                        &crate::upstream_error::ORCHESTRATOR_UNREACHABLE,
                        crate::upstream_error::Class::Unavailable,
                        &err.to_string(),
                    );
                    ApiError::Unavailable("the orchestrator is unavailable".to_owned())
                })?;

        // If settle_request changed U's state, persist it.
        if u_state.requested_run_key != u_stored.as_ref().and_then(|s| s.requested_run_key.clone())
            || u_state.consumed_at != u_stored.as_ref().and_then(|s| s.consumed_at)
        {
            let u_update = LagStateUpdate {
                pipeline_id: u_pipe.id.clone(),
                tenant_id: u_pipe.tenant_id,
                consumed_tokens: u_state.consumed_tokens.as_ref().map(tokens_to_json),
                consumed_at: u_state.consumed_at,
                pending_since: u_state.pending_since,
                requested_run_key: u_state.requested_run_key.clone(),
                requested_tokens: u_state.requested_tokens.as_ref().map(tokens_to_json),
                requested_at: u_state.requested_at,
                request_attempt: u_state.request_attempt,
                last_check_at: now,
                // BLD-2 review R1 SHOULD-FIX 2: never-measured U is not_measured, not met
                last_status: u_stored
                    .as_ref()
                    .and_then(|s| s.last_status.clone())
                    .unwrap_or_else(|| "not_measured".to_owned()),
                behind_streak: u_stored.as_ref().map_or(0, |s| s.behind_streak),
            };
            pipelines::upsert_lag_state(pool, &u_update).await.map_err(|err| {
                tracing::error!(%err, pipeline = %u_pipe.id, "target lag: upstream state row unwritable");
                crate::upstream_error::report(
                    &crate::upstream_error::STORE,
                    crate::upstream_error::Class::Unavailable,
                    &err.to_string(),
                );
                ApiError::Internal("the request could not be completed".to_owned())
            })?;
        }

        if u_outstanding {
            let pending_since = state.pending_since.unwrap_or(now);
            state.pending_since = Some(pending_since);
            let lag_seconds = (now - pending_since).whole_seconds();
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
            // BLD-2 review R1 BLOCKER 5: keep P's request
            let p_update = LagStateUpdate {
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
                last_status: status.as_str().to_owned(),
                behind_streak,
            };
            pipelines::upsert_lag_state_conn(tx, &p_update).await.map_err(|err| {
                tracing::error!(%err, pipeline = %pipeline.id, "target lag: state row unwritable");
                crate::upstream_error::report(
                    &crate::upstream_error::STORE,
                    crate::upstream_error::Class::Unavailable,
                    &err.to_string(),
                );
                ApiError::Internal("the request could not be completed".to_owned())
            })?;
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
        let u_inputs_maybe = inputs_of(pool, u_pipe).await.map_err(|err| {
            tracing::error!(%err, pipeline = %u_pipe.id, "target lag: upstream inputs unreadable");
            crate::upstream_error::report(
                &crate::upstream_error::STORE,
                crate::upstream_error::Class::Unavailable,
                &err.to_string(),
            );
            ApiError::Internal("the request could not be completed".to_owned())
        })?;
        let Some(u_inputs) = u_inputs_maybe else {
            // BLD-2 review R1 SHOULD-FIX 1 / SHOULD-FIX 13: unparseable upstream source -> write not_measured
            let p_update = LagStateUpdate {
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
                last_status: LagStatus::NotMeasured.as_str().to_owned(),
                behind_streak: 0,
            };
            pipelines::upsert_lag_state_conn(tx, &p_update).await.map_err(|err| {
                tracing::error!(%err, pipeline = %pipeline.id, "target lag: state row unwritable");
                crate::upstream_error::report(
                    &crate::upstream_error::STORE,
                    crate::upstream_error::Class::Unavailable,
                    &err.to_string(),
                );
                ApiError::Internal("the request could not be completed".to_owned())
            })?;
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
                // BLD-2 review R1 SHOULD-FIX 1: write P's state as not_measured and reset streak to 0
                let p_update = LagStateUpdate {
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
                    last_status: LagStatus::NotMeasured.as_str().to_owned(),
                    behind_streak: 0,
                };
                pipelines::upsert_lag_state_conn(tx, &p_update).await.map_err(|err| {
                    tracing::error!(%err, pipeline = %pipeline.id, "target lag: state row unwritable");
                    crate::upstream_error::report(
                        &crate::upstream_error::STORE,
                        crate::upstream_error::Class::Unavailable,
                        &err.to_string(),
                    );
                    ApiError::Internal("the request could not be completed".to_owned())
                })?;
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

        let u_changed = u_state.consumed_tokens.as_ref() != Some(&u_tokens);
        if u_changed {
            let mut total_budget = run_budget_seconds(dagster, job_name).await.map_err(|err| {
                tracing::warn!(%err, "target lag: run budget unreadable");
                crate::upstream_error::report(
                    &crate::upstream_error::ORCHESTRATOR_UNREACHABLE,
                    crate::upstream_error::Class::Unavailable,
                    &err.to_string(),
                );
                ApiError::Unavailable("the orchestrator is unavailable".to_owned())
            })?;
            for up in chain {
                let up_job = crate::routes::authored_pipelines::job_name(&up.id);
                total_budget += run_budget_seconds(dagster, &up_job).await.map_err(|err| {
                    tracing::warn!(%err, "target lag: upstream run budget unreadable");
                    crate::upstream_error::report(
                        &crate::upstream_error::ORCHESTRATOR_UNREACHABLE,
                        crate::upstream_error::Class::Unavailable,
                        &err.to_string(),
                    );
                    ApiError::Unavailable("the orchestrator is unavailable".to_owned())
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

            if due {
                // BLD-2 review R1 decision R2 / BLOCKER 4: run_key carries attempt
                let u_key = run_key_for(&u_pipe.id, &u_tokens, u_state.request_attempt);
                u_state.requested_run_key = Some(u_key.clone());
                u_state.requested_tokens = Some(u_tokens);
                u_state.requested_at = Some(now);

                let u_update = LagStateUpdate {
                    pipeline_id: u_pipe.id.clone(),
                    tenant_id: u_pipe.tenant_id,
                    consumed_tokens: u_state.consumed_tokens.as_ref().map(tokens_to_json),
                    consumed_at: u_state.consumed_at,
                    pending_since: u_state.pending_since.or(Some(now)),
                    requested_run_key: u_state.requested_run_key.clone(),
                    requested_tokens: u_state.requested_tokens.as_ref().map(tokens_to_json),
                    requested_at: u_state.requested_at,
                    request_attempt: u_state.request_attempt,
                    last_check_at: now,
                    last_status: "behind".to_owned(),
                    // BLD-2 review R1 SHOULD-FIX 2: behind streak should start at 1 or increment
                    behind_streak: u_stored.as_ref().map_or(1, |s| s.behind_streak + 1),
                };
                pipelines::upsert_lag_state(pool, &u_update).await.map_err(|err| {
                    tracing::error!(%err, pipeline = %u_pipe.id, "target lag: upstream state row unwritable");
                    crate::upstream_error::report(
                        &crate::upstream_error::STORE,
                        crate::upstream_error::Class::Unavailable,
                        &err.to_string(),
                    );
                    ApiError::Internal("the request could not be completed".to_owned())
                })?;

                let status = if never_consumed || lag_seconds > target {
                    LagStatus::Behind
                } else {
                    LagStatus::Met
                };
                let behind_streak = match status {
                    LagStatus::Behind => stored.map_or(1, |row| row.behind_streak + 1),
                    LagStatus::Met | LagStatus::NotMeasured => 0,
                };
                // BLD-2 review R1 BLOCKER 5: keep P's request
                let p_update = LagStateUpdate {
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
                    last_status: status.as_str().to_owned(),
                    behind_streak,
                };
                pipelines::upsert_lag_state_conn(tx, &p_update).await.map_err(|err| {
                    tracing::error!(%err, pipeline = %pipeline.id, "target lag: state row unwritable");
                    crate::upstream_error::report(
                        &crate::upstream_error::STORE,
                        crate::upstream_error::Class::Unavailable,
                        &err.to_string(),
                    );
                    ApiError::Internal("the request could not be completed".to_owned())
                })?;

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

            let status = if never_consumed || lag_seconds > target {
                LagStatus::Behind
            } else {
                LagStatus::Met
            };
            let behind_streak = match status {
                LagStatus::Behind => stored.map_or(1, |row| row.behind_streak + 1),
                LagStatus::Met | LagStatus::NotMeasured => 0,
            };
            // BLD-2 review R1 BLOCKER 5: keep P's request
            let p_update = LagStateUpdate {
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
                last_status: status.as_str().to_owned(),
                behind_streak,
            };
            pipelines::upsert_lag_state_conn(tx, &p_update).await.map_err(|err| {
                tracing::error!(%err, pipeline = %pipeline.id, "target lag: state row unwritable");
                crate::upstream_error::report(
                    &crate::upstream_error::STORE,
                    crate::upstream_error::Class::Unavailable,
                    &err.to_string(),
                );
                ApiError::Internal("the request could not be completed".to_owned())
            })?;

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

    // BLD-2 review R1 decision R2 / BLOCKER 4: reset request_attempt when tokens change
    if state.requested_tokens.as_ref() != Some(&tokens) {
        state.request_attempt = 0;
    }

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
    let max_retries = i32::from(pipeline.max_retries);
    if !outstanding {
        // BLD-2 review R1 decision R2 / BLOCKER 4: stop after max_retries + 1 attempts
        if state.request_attempt > max_retries {
            decision = Decision::Wait;
        } else {
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
    let (status, reason) = if state.request_attempt > max_retries {
        let n = max_retries + 1;
        (
            LagStatus::Behind,
            format!("the last {n} runs for this change failed; the next input change starts again"),
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
/// One call to [`pipelines::count_lag_pipelines_reading`] per candidate,
/// which is the SAME function the lag check itself uses to decide whether a
/// downstream pipeline is read (review R1 SHOULD-FIX 3). Reusing it is what
/// makes the two answers agree: the earlier batched version answered the
/// `dependsOn` edge differently from the check, so a pipeline read only
/// through `depends_on` was told on the page that nobody reads it while the
/// check kept scheduling it. The cost is one round trip per downstream
/// candidate; the list this is called with is a page of pipelines, and the
/// wrong answer is a false statement about a user's data.
///
/// The error is the caller's, never a silent zero (a swallowed failure
/// would report every downstream pipeline as having no reader).
///
/// # Errors
///
/// Returns [`StoreError`] if a reader count query fails.
// BLD-2 review R1 SHOULD-FIX 3: use count_lag_pipelines_reading for exact parity including dependsOn
pub async fn downstream_without_readers(
    pool: &PgPool,
    candidates: &[Pipeline],
) -> Result<std::collections::HashSet<String>, StoreError> {
    let candidates: Vec<&Pipeline> = candidates
        .iter()
        .filter(|p| p.target_lag == Some(TargetLag::Downstream))
        .collect();
    if candidates.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let mut without = std::collections::HashSet::new();
    for candidate in candidates {
        let readers = pipelines::count_lag_pipelines_reading(
            pool,
            &candidate.id,
            &candidate.target,
            candidate.tenant_id,
        )
        .await?;
        if readers == 0 {
            without.insert(candidate.id.clone());
        }
    }
    Ok(without)
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
#[must_use]
pub fn lag_json(
    target_lag: Option<TargetLag>,
    state: Option<&pipelines::PipelineLagState>,
    now: OffsetDateTime,
    downstream_no_readers: bool,
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
    let status = match st.last_status.as_deref() {
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
    let reason = reason_for(status, st.consumed_tokens.is_none());
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

        /// BLD-2 review R1 SHOULD-FIX 1: an input whose token cannot be
        /// read at all, for the test of the branch that writes
        /// `not_measured` instead of leaving the old status.
        fn with_error(mut self, key: &str, _why: &str) -> Self {
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
        // BLD-2 review round 1 / D11: the floor is 2 minutes (measured), so
        // no fixture may ask for 1 — the column CHECK refuses it.
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
        assert_eq!(lag_json(None, None, now, false), serde_json::Value::Null);
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
                false
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
        let res = lag_json(Some(TargetLag::Downstream), Some(&state), now, true);
        assert_eq!(res["status"], "not_measured");
        assert_eq!(
            res["reason"],
            "Downstream: no pipeline with a target lag reads this table"
        );
        assert_eq!(res["lagSeconds"], serde_json::Value::Null);

        let read = lag_json(Some(TargetLag::Downstream), Some(&state), now, false);
        assert_eq!(
            read["status"], "met",
            "a downstream pipeline somebody reads reports what its own check measured"
        );
    }

    /// Plan D9: the alert fires on the check that completes the streak, so
    /// once per streak and never twice for it.
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
        let reader =
            FakeTokens::new().with_error("bronze.orders", "upstream iceberg metadata corrupted");
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
}
