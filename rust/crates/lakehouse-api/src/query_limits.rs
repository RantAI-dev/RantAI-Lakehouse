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

use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::query_limits::{QueryLimits, RoleLimit};
use serde_json::{Value, json};

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
    fn the_memo_serves_a_read_until_it_is_cleared() {
        let memo = QueryLimitsMemo::default();
        assert!(!memo.is_warm());
        memo.store(Arc::new(Snapshot::default()));
        assert!(memo.is_warm());
        memo.clear();
        assert!(!memo.is_warm());
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
