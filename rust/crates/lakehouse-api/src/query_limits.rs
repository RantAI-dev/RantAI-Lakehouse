//! The limits a query runs under (`BI-5`, plan decisions D1 and D2): how long
//! a chart or Query Studio query may run and how many rows or points it may
//! return, resolved once per request from the install's settings and the
//! caller's roles.
//!
//! # Where the numbers live
//!
//! In Postgres (`lakehouse_store::query_limits`, migration `0101`). Reading
//! them per tile would put a Postgres round trip in front of every tile of a
//! dashboard, so [`QueryLimitsMemo`] keeps the last read for
//! [`MEMO_TTL`] (30 s). A `PUT /api/settings/limits` clears the memo of the
//! process that handled it; **another API process sees the change within
//! 30 s**, not at once. That delay is the price of not reading per tile and
//! is stated here so nobody has to rediscover it.
//!
//! # Which limit a person gets
//!
//! Each role the person holds has the time limit of its own, or the
//! install's when it has none. The person gets the longest of those (feature
//! decision 1); a person with no role gets the install's. The row limits are
//! the install's for everyone (decision 2). [`TileLimits::source`] says which
//! one applied, so a tile can say "the limit for your role" or "the limit for
//! this install" (decision 5).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use lakehouse_auth::Principal;
use lakehouse_clickhouse::QueryOptions;
use lakehouse_core::ApiError;
use lakehouse_store::query_limits::{QueryLimits, RoleLimit};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU32, Ordering};
use uuid::Uuid;

use crate::state::AppState;
use crate::upstream_error;

/// How long a read of the settings is reused (plan D2).
pub const MEMO_TTL: Duration = Duration::from_secs(30);

/// Where the limit a query ran under came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitSource {
    /// The install-wide setting.
    Install,
    /// The time limit of the role with this name.
    Role(String),
}

impl LimitSource {
    /// The console's shape: `{ "kind": "install" }` or
    /// `{ "kind": "role", "name": "..." }`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::Install => json!({ "kind": "install" }),
            Self::Role(name) => json!({ "kind": "role", "name": name }),
        }
    }
}

/// The limits one request runs under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileLimits {
    /// How long a query may run.
    pub timeout: Duration,
    /// Rows a chart reads.
    pub chart_rows: u32,
    /// Points a map shows.
    pub map_points: u32,
    /// Rows Query Studio returns.
    pub studio_rows: u32,
    /// Where `timeout` came from.
    pub source: LimitSource,
}

impl TileLimits {
    /// The time limit in whole seconds.
    #[must_use]
    pub fn timeout_seconds(&self) -> u64 {
        self.timeout.as_secs()
    }

    /// The `effective` object of `GET /api/settings/limits`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "timeoutSeconds": self.timeout_seconds(),
            "chartRows": self.chart_rows,
            "mapPoints": self.map_points,
            "studioRows": self.studio_rows,
            "source": self.source.to_json(),
        })
    }
}

/// One read of the settings: the install row and the role rows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    /// The install-wide limits.
    pub install: QueryLimits,
    /// The roles that have a time limit of their own.
    pub roles: Vec<RoleLimit>,
}

fn to_u32(n: i32) -> u32 {
    u32::try_from(n).unwrap_or(0)
}

impl Snapshot {
    /// The limits for someone who holds `role_names`.
    #[must_use]
    pub fn effective_for_roles(&self, role_names: &[String]) -> TileLimits {
        let install_seconds = self.install.timeout_seconds;
        let mut best: Option<(i32, LimitSource)> = None;
        let mut consider = |seconds: i32, source: LimitSource| {
            if best.as_ref().is_none_or(|(have, _)| seconds > *have) {
                best = Some((seconds, source));
            }
        };
        for name in role_names {
            match self.roles.iter().find(|r| &r.role_name == name) {
                Some(rule) => consider(rule.timeout_seconds, LimitSource::Role(name.clone())),
                None => consider(install_seconds, LimitSource::Install),
            }
        }
        let (seconds, source) = best.unwrap_or((install_seconds, LimitSource::Install));
        TileLimits {
            timeout: Duration::from_secs(u64::from(to_u32(seconds))),
            chart_rows: to_u32(self.install.chart_rows),
            map_points: to_u32(self.install.map_points),
            studio_rows: to_u32(self.install.studio_rows),
            source,
        }
    }

    /// The longest time limit anyone can have: the install's or any role's.
    /// The route deadline of a limit-bearing route is this plus
    /// [`DEADLINE_MARGIN`] (plan D4).
    #[must_use]
    pub fn longest_timeout(&self) -> Duration {
        let longest = self
            .roles
            .iter()
            .map(|r| r.timeout_seconds)
            .chain(std::iter::once(self.install.timeout_seconds))
            .max()
            .unwrap_or(self.install.timeout_seconds);
        Duration::from_secs(u64::from(to_u32(longest)))
    }

    /// The limits for `principal`.
    #[must_use]
    pub fn effective_for(&self, principal: &Principal) -> TileLimits {
        self.effective_for_roles(&principal.role_names)
    }
}

type Slot = Option<(Instant, Arc<Snapshot>)>;

/// The 30-second in-process memo of the settings (plan D2). Shared across
/// every clone of `AppState`.
#[derive(Clone, Default)]
pub struct QueryLimitsMemo(Arc<Mutex<Slot>>);

impl QueryLimitsMemo {
    fn fresh(&self) -> Option<Arc<Snapshot>> {
        let slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        slot.as_ref()
            .filter(|(at, _)| at.elapsed() < MEMO_TTL)
            .map(|(_, snap)| Arc::clone(snap))
    }

    fn store(&self, snapshot: Arc<Snapshot>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some((Instant::now(), snapshot));
    }

    /// Forget the last read. Called by `PUT /api/settings/limits`.
    pub fn clear(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Whether a read is currently remembered.
    #[cfg(test)]
    #[must_use]
    pub fn is_warm(&self) -> bool {
        self.fresh().is_some()
    }
}

/// The settings, from the memo or from Postgres. Without a Postgres pool the
/// feature defaults stand (the same posture as the reporting settings).
///
/// # Errors
///
/// A failing read is reported through `upstream_error`.
pub async fn snapshot(state: &AppState) -> Result<Arc<Snapshot>, ApiError> {
    if let Some(hit) = state.query_limits.fresh() {
        return Ok(hit);
    }
    let Some(pool) = state.pg.as_deref() else {
        return Ok(Arc::new(Snapshot::default()));
    };
    let read = async {
        let install = lakehouse_store::query_limits::get(pool).await?;
        let roles = lakehouse_store::query_limits::role_limits(pool).await?;
        Ok::<_, lakehouse_store::StoreError>(Snapshot { install, roles })
    };
    let fresh = Arc::new(
        read.await
            .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?,
    );
    state.query_limits.store(Arc::clone(&fresh));
    Ok(fresh)
}

/// The request header that names a run (plan D5): a UUID the console makes
/// for each Run or dashboard load, so Stop can name it.
pub const RUN_ID_HEADER: &str = "x-run-id";

/// The principal part of a query id for a request nobody signed in to (a
/// public link, an embed, a detached query). It is the nil UUID: no person has
/// it, so no Stop, which always names the caller's own id, can match it.
pub const ANONYMOUS_PRINCIPAL: &str = "00000000-0000-0000-0000-000000000000";

tokio::task_local! {
    static CURRENT: Arc<QueryRun>;
}

/// The identity and limits context of the queries one request sends.
pub struct QueryRun {
    principal: String,
    run_id: String,
    next: AtomicU32,
    roles: Vec<String>,
    state: Option<AppState>,
}

impl QueryRun {
    /// The run of a request: `principal` is `None` for a public or embed
    /// request, and `header` is the raw `X-Run-Id` value. A value that is not
    /// a UUID is replaced by a new one (the API "makes one" when the header is
    /// absent, plan D5), so the id can be put in a `LIKE` pattern only after
    /// this check.
    #[must_use]
    pub fn for_request(
        state: &AppState,
        principal: Option<&Principal>,
        header: Option<&str>,
    ) -> Self {
        let run_id = header
            .and_then(|h| Uuid::parse_str(h.trim()).ok())
            .unwrap_or_else(Uuid::new_v4)
            .to_string();
        Self {
            principal: principal.map_or_else(
                || ANONYMOUS_PRINCIPAL.to_owned(),
                |p| p.id.uuid().to_string(),
            ),
            run_id,
            next: AtomicU32::new(0),
            roles: principal.map(|p| p.role_names.clone()).unwrap_or_default(),
            state: Some(state.clone()),
        }
    }

    /// A run outside any request: the install defaults, the nil principal.
    #[must_use]
    pub fn detached() -> Self {
        Self {
            principal: ANONYMOUS_PRINCIPAL.to_owned(),
            run_id: Uuid::new_v4().to_string(),
            next: AtomicU32::new(0),
            roles: Vec::new(),
            state: None,
        }
    }

    /// The run of the request being served, or a [detached](Self::detached)
    /// one.
    #[must_use]
    pub fn current_or_detached() -> Arc<Self> {
        CURRENT
            .try_with(Arc::clone)
            .unwrap_or_else(|_| Arc::new(Self::detached()))
    }

    /// The run id (a UUID in its hyphenated form).
    #[cfg(test)]
    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// The next query id of this run: `<principal uuid>:<run id>:<n>`.
    #[must_use]
    pub fn next_query_id(&self) -> String {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        format!("{}:{}:{n}", self.principal, self.run_id)
    }

    /// The limits this run's caller is under.
    ///
    /// # Errors
    ///
    /// As [`snapshot`].
    pub async fn limits(&self) -> Result<TileLimits, ApiError> {
        match &self.state {
            Some(state) => Ok(snapshot(state).await?.effective_for_roles(&self.roles)),
            None => Ok(Snapshot::default().effective_for_roles(&self.roles)),
        }
    }

    /// The options of one query of this run: the time limit, one row more
    /// than `rows` (so a cut is told from an exact fit, plan D3) and a fresh
    /// query id.
    #[must_use]
    pub fn options(&self, limits: &TileLimits, rows: u32) -> QueryOptions {
        QueryOptions {
            max_execution_time: Some(limits.timeout),
            max_result_rows: Some(u64::from(rows) + 1),
            query_id: Some(self.next_query_id()),
        }
    }
}

/// Run `fut` with `run` as the current run.
pub async fn scope<F: std::future::Future>(run: QueryRun, fut: F) -> F::Output {
    CURRENT.scope(Arc::new(run), fut).await
}

/// Middleware: give every request its [`QueryRun`], from the principal the
/// auth gate set (none on a public route) and the `X-Run-Id` header. It sits
/// inside `auth_gate`, so the principal is already there, and does no work for
/// a route that sends no query (the limits are read only when a query asks).
pub async fn scope_middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let header = req
        .headers()
        .get(RUN_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let run = QueryRun::for_request(
        &state,
        req.extensions().get::<Principal>(),
        header.as_deref(),
    );
    scope(run, next.run(req)).await
}

/// The most a board request may take (plan D4): 35 minutes, the 30-minute
/// ceiling of the time limit plus the margin below.
pub const BOARD_DEADLINE_CAP: Duration = Duration::from_mins(35);

/// What the deadline adds to the time the tiles may need, for the answer.
pub const DEADLINE_MARGIN: Duration = Duration::from_secs(15);

/// How long a board of `tiles` tiles may take (plan D4): the tiles run
/// `TILES_PER_REQUEST` at a time, so it needs `ceil(tiles / 6)` rounds of the
/// time limit, plus the margin, never more than [`BOARD_DEADLINE_CAP`].
#[must_use]
pub fn board_deadline(tiles: usize, limit: Duration) -> Duration {
    let rounds = u32::try_from(tiles.div_ceil(crate::tile_pool::TILES_PER_REQUEST).max(1))
        .unwrap_or(u32::MAX);
    limit
        .saturating_mul(rounds)
        .saturating_add(DEADLINE_MARGIN)
        .min(BOARD_DEADLINE_CAP)
}

/// What a tile says when the board's deadline passed before it finished.
pub const NOT_REACHED_SENTENCE: &str = "Not reached in time.";

/// The sentence of a tile or statement the time limit ended (plan D6): it
/// names the limit and where it came from, and never the engine's text.
#[must_use]
pub fn time_sentence(subject: &str, limits: &TileLimits) -> String {
    let seconds = limits.timeout_seconds();
    let whose = match limits.source {
        LimitSource::Role(_) => "the limit for your role",
        LimitSource::Install => "the limit for this install",
    };
    format!(
        "This {subject} stopped after {seconds} {}, {whose}.",
        if seconds == 1 { "second" } else { "seconds" }
    )
}

/// What a query the person stopped on the engine says (plan D6).
pub const STOPPED_SENTENCE: &str = "Stopped";

impl TileLimits {
    /// The `limit` object of a tile the time limit ended (plan D6).
    #[must_use]
    pub fn time_limit_json(&self) -> Value {
        json!({ "kind": "time", "value": self.timeout_seconds(), "source": self.source.to_json() })
    }

    /// The `limitHit` object of a tile cut at `rows` rows (plan D6, named
    /// `limitHit` by the plan review because `limit` is a raw table's page
    /// size).
    #[must_use]
    pub fn rows_limit_json(&self, rows: u32) -> Value {
        json!({ "kind": "rows", "value": rows, "source": self.source.to_json() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn role(name: &str, seconds: i32) -> RoleLimit {
        RoleLimit {
            role_id: Uuid::new_v4(),
            role_name: name.to_owned(),
            timeout_seconds: seconds,
        }
    }

    fn names(roles: &[&str]) -> Vec<String> {
        roles.iter().map(|r| (*r).to_owned()).collect()
    }

    fn snapshot_with(roles: Vec<RoleLimit>) -> Snapshot {
        Snapshot {
            install: QueryLimits::default(),
            roles,
        }
    }

    #[test]
    fn a_person_without_a_role_rule_gets_the_install_limit() {
        let snap = snapshot_with(vec![role("Analyst", 300)]);
        let got = snap.effective_for_roles(&names(&["Viewer"]));
        assert_eq!(got.timeout_seconds(), 60);
        assert_eq!(got.source, LimitSource::Install);
        assert_eq!(snap.effective_for_roles(&[]).source, LimitSource::Install);
    }

    #[test]
    fn a_role_with_a_longer_limit_wins_and_is_named() {
        let snap = snapshot_with(vec![role("Analyst", 300)]);
        let got = snap.effective_for_roles(&names(&["Analyst"]));
        assert_eq!(got.timeout_seconds(), 300);
        assert_eq!(got.source, LimitSource::Role("Analyst".to_owned()));
    }

    #[test]
    fn two_roles_give_the_longest() {
        let snap = snapshot_with(vec![role("Analyst", 300), role("Engineer", 900)]);
        let got = snap.effective_for_roles(&names(&["Analyst", "Engineer"]));
        assert_eq!(got.timeout_seconds(), 900);
        assert_eq!(got.source, LimitSource::Role("Engineer".to_owned()));
    }

    #[test]
    fn a_shorter_role_does_not_cut_a_person_who_holds_a_role_without_a_rule() {
        let snap = snapshot_with(vec![role("Intern", 20)]);
        let got = snap.effective_for_roles(&names(&["Intern", "Viewer"]));
        assert_eq!(got.timeout_seconds(), 60);
        assert_eq!(got.source, LimitSource::Install);
        let only = snap.effective_for_roles(&names(&["Intern"]));
        assert_eq!(only.timeout_seconds(), 20);
    }

    #[test]
    fn the_administrator_role_has_no_exemption_and_gets_the_install_limit_without_a_rule() {
        let snap = snapshot_with(vec![]);
        let got = snap.effective_for_roles(&names(&["Platform Admin"]));
        assert_eq!(got.timeout_seconds(), 60);
        assert_eq!(got.source, LimitSource::Install);
    }

    #[test]
    fn row_limits_are_the_installs_for_everyone() {
        let mut snap = snapshot_with(vec![role("Analyst", 300)]);
        snap.install.chart_rows = 500;
        snap.install.map_points = 1000;
        snap.install.studio_rows = 7000;
        let got = snap.effective_for_roles(&names(&["Analyst"]));
        assert_eq!(
            (got.chart_rows, got.map_points, got.studio_rows),
            (500, 1000, 7000)
        );
    }

    #[test]
    fn the_longest_timeout_covers_the_install_and_every_role() {
        let snap = snapshot_with(vec![role("A", 30), role("B", 600)]);
        assert_eq!(snap.longest_timeout(), Duration::from_secs(600));
        assert_eq!(
            snapshot_with(vec![role("A", 30)]).longest_timeout(),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn the_memo_serves_a_read_until_it_is_cleared() {
        let memo = QueryLimitsMemo::default();
        assert!(!memo.is_warm());
        memo.store(Arc::new(Snapshot::default()));
        assert!(memo.is_warm());
        memo.clear();
        assert!(!memo.is_warm());
    }

    /// Replaces the pin that tied `SQL_SOURCE_MAX_ROWS` to the settings text
    /// (rule 2: replaced, not deleted): the builder's default text, the
    /// store's defaults and migration `0101`'s column defaults must be the
    /// same numbers.
    #[test]
    fn the_builder_default_text_the_store_default_and_the_migration_agree() {
        let d = QueryLimits::default();
        let text = lakehouse_bi::builder::SQL_SOURCE_SETTINGS;
        assert!(
            text.contains(&format!("max_result_rows = {}", d.chart_rows)),
            "{text}"
        );
        assert!(
            text.contains(&format!("max_execution_time = {}", d.timeout_seconds)),
            "{text}"
        );
        let migration = include_str!("../../../migrations/0101_query_limits.sql");
        for (column, value) in [
            ("timeout_seconds INT NOT NULL DEFAULT", d.timeout_seconds),
            ("chart_rows      INT NOT NULL DEFAULT", d.chart_rows),
            ("map_points      INT NOT NULL DEFAULT", d.map_points),
            ("studio_rows     INT NOT NULL DEFAULT", d.studio_rows),
        ] {
            assert!(
                migration.contains(&format!("{column} {value}")),
                "{column} {value}"
            );
        }
    }

    #[test]
    fn query_ids_name_the_principal_the_run_and_a_counter() {
        let run = QueryRun::detached();
        let (a, b) = (run.next_query_id(), run.next_query_id());
        assert_eq!(a, format!("{ANONYMOUS_PRINCIPAL}:{}:0", run.run_id()));
        assert!(b.ends_with(":1"));
    }

    #[test]
    fn the_time_sentence_names_the_limit_and_its_source() {
        let mut limits = Snapshot::default().effective_for_roles(&[]);
        assert_eq!(
            time_sentence("chart", &limits),
            "This chart stopped after 60 seconds, the limit for this install."
        );
        limits.source = LimitSource::Role("Analyst".to_owned());
        assert!(time_sentence("query", &limits).ends_with("the limit for your role."));
    }

    #[test]
    fn a_boards_deadline_is_rounds_of_the_limit_plus_fifteen_seconds_capped() {
        let limit = Duration::from_secs(60);
        assert_eq!(board_deadline(0, limit), Duration::from_secs(75));
        assert_eq!(board_deadline(6, limit), Duration::from_secs(75));
        assert_eq!(board_deadline(7, limit), Duration::from_secs(135));
        assert_eq!(board_deadline(13, limit), Duration::from_secs(195));
        assert_eq!(
            board_deadline(600, Duration::from_mins(30)),
            BOARD_DEADLINE_CAP
        );
    }

    #[test]
    fn the_defaults_are_the_feature_page_numbers() {
        let d = Snapshot::default().effective_for_roles(&[]);
        assert_eq!(
            (
                d.timeout_seconds(),
                d.chart_rows,
                d.map_points,
                d.studio_rows
            ),
            (60, 2000, 5000, 2000)
        );
    }
}
