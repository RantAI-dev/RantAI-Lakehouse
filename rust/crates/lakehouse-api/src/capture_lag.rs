//! What "lag" means for a change-capture connector, and when it alerts
//! (`SRC-4` slice 4, decisions S4-c and S4-d; feature page decisions 6 and 7).
//!
//! The orchestrator observes and this API decides, as for pipeline lag
//! (`pipeline_lag`): the orchestrator posts a table's backlog and the source
//! commit time of the last change it applied, and the functions here turn
//! those and the connector's state into one of five answers. Every function
//! takes `now` as a parameter, so a test pins a time instead of waiting for
//! one and nothing here reads a clock.
//!
//! # The five answers ([`TableLag`])
//!
//! - **Paused**: a person paused the connector. No number: a pause is not lag.
//! - **Not known yet**: nothing was ever reported for the table.
//! - **Not known since `<time>`**: the newest backlog report is older than
//!   [`STALE_AFTER`], which means the orchestrator is not reporting. The last
//!   number is not shown as if it were current (principle 2).
//! - **Up to date**: the backlog was 0 while `Kafka Connect` said `running`. A
//!   backlog of 0 alone proves nothing: with `Kafka Connect` stopped, nothing
//!   reaches the topic either.
//! - **Behind by `n` seconds**: now minus the later of the last applied
//!   change's source time and the last moment the table was up to date. The
//!   later of the two keeps a table that was quiet for a day, then got one
//!   change, from reading a day behind: it was up to date until that change
//!   was made. The number is an upper bound, never smaller than the true lag
//!   (caught up at 1000, a change made at 1020 not yet applied, now 1030: the
//!   number is 30 s, the true lag 10 s). Slice 4 review SHOULD-FIX 1.

use lakehouse_store::capture_lag::CaptureTable;
use time::{Duration, OffsetDateTime};

/// A backlog report older than this is "not known since". The sensor reports
/// every 30 seconds, so two minutes allows a few missed ticks.
pub const STALE_AFTER: Duration = Duration::minutes(2);

/// One table's lag, in the words the console shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableLag {
    /// A person paused the connector.
    Paused,
    /// Nothing was reported for the table yet.
    NotKnownYet,
    /// The newest backlog report is older than [`STALE_AFTER`].
    NotKnownSince {
        /// When the newest backlog report was made.
        since: OffsetDateTime,
    },
    /// Backlog 0 while `Kafka Connect` was running.
    UpToDate,
    /// Behind the source by this many seconds.
    Behind {
        /// Whole seconds, never negative.
        seconds: i64,
    },
}

/// One table's lag at `now`. `running` is whether `Kafka Connect` says the
/// connector runs; `paused_by_person` whether a person paused it.
#[must_use]
pub fn table_lag(
    table: Option<&CaptureTable>,
    running: bool,
    paused_by_person: bool,
    now: OffsetDateTime,
) -> TableLag {
    if paused_by_person {
        return TableLag::Paused;
    }
    let Some(table) = table else {
        return TableLag::NotKnownYet;
    };
    let Some(observed_at) = table.observed_at else {
        return TableLag::NotKnownYet;
    };
    if now - observed_at > STALE_AFTER {
        return TableLag::NotKnownSince { since: observed_at };
    }
    if running && table.backlog == Some(0) {
        return TableLag::UpToDate;
    }
    // The later of the two times: see the module doc.
    let anchor = match (table.last_source_ts, table.caught_up_at) {
        (Some(changed), Some(caught_up)) => changed.max(caught_up),
        (Some(only), None) | (None, Some(only)) => only,
        (None, None) => return TableLag::NotKnownYet,
    };
    TableLag::Behind {
        seconds: (now - anchor).whole_seconds().max(0),
    }
}

/// The lag of every table the connector selects, in the order given. A
/// selected table with no stored row is "not known yet".
#[must_use]
pub fn tables_lag(
    selected: &[String],
    stored: &[CaptureTable],
    running: bool,
    paused_by_person: bool,
    now: OffsetDateTime,
) -> Vec<(String, TableLag)> {
    selected
        .iter()
        .map(|name| {
            let row = stored.iter().find(|row| &row.table_name == name);
            (name.clone(), table_lag(row, running, paused_by_person, now))
        })
        .collect()
}

/// The worst table: the one furthest behind, with its seconds. `None` when no
/// table is behind by a number.
#[must_use]
pub fn worst(lags: &[(String, TableLag)]) -> Option<(&str, i64)> {
    lags.iter()
        .filter_map(|(name, lag)| match lag {
            TableLag::Behind { seconds } => Some((name.as_str(), *seconds)),
            _ => None,
        })
        .max_by_key(|(_, seconds)| *seconds)
}

/// What one evaluation of a connector's lag against its limit asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LagAlert {
    /// Nothing to do: no change of state, or not enough is known to say.
    Nothing,
    /// The lag passed the limit while no alert was open.
    Crossed {
        /// The worst table.
        table: String,
        /// Its lag, in seconds.
        seconds: i64,
    },
    /// The lag is back under the limit while an alert was open.
    BackUnder,
}

/// Compare `lags` with the limit. `open` is whether an alert has fired and not
/// been answered. A crossing needs one table over the limit; "back under" needs
/// every table to be known (up to date, or behind by no more than the limit):
/// a table whose lag is not known is not a reason to say the problem is over.
/// Paused tables are not evaluated (the caller skips a paused connector).
#[must_use]
pub fn alert_step(open: bool, limit_minutes: i32, lags: &[(String, TableLag)]) -> LagAlert {
    let limit_seconds = i64::from(limit_minutes) * 60;
    if let Some((table, seconds)) = worst(lags)
        && seconds > limit_seconds
    {
        return if open {
            LagAlert::Nothing
        } else {
            LagAlert::Crossed {
                table: table.to_owned(),
                seconds,
            }
        };
    }
    let all_known = lags
        .iter()
        .all(|(_, lag)| matches!(lag, TableLag::UpToDate | TableLag::Behind { .. }));
    if open && all_known {
        LagAlert::BackUnder
    } else {
        LagAlert::Nothing
    }
}

/// The text of the alert for a crossing, in fixed words: the connector, the
/// worst table, its lag in whole minutes and the limit. Names and numbers only.
#[must_use]
pub fn crossed_text(
    connector_name: &str,
    connector_id: &str,
    table: &str,
    seconds: i64,
    limit_minutes: i32,
) -> String {
    format!(
        "Connector {connector_name} ({connector_id}): change capture of {table} is {} min behind \
         the source, over the limit of {limit_minutes} min. View: /connectors/{connector_id}",
        seconds / 60,
    )
}

/// The text of the alert for "back under".
#[must_use]
pub fn back_under_text(connector_name: &str, connector_id: &str, limit_minutes: i32) -> String {
    format!(
        "Connector {connector_name} ({connector_id}): change capture is back under the limit of \
         {limit_minutes} min. View: /connectors/{connector_id}",
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn at(secs: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_700_000_000 + secs).unwrap()
    }

    fn row(
        last: Option<i64>,
        caught_up: Option<i64>,
        backlog: Option<i64>,
        observed: Option<i64>,
    ) -> CaptureTable {
        CaptureTable {
            table_name: "public.orders".to_owned(),
            last_source_ts: last.map(at),
            caught_up_at: caught_up.map(at),
            backlog,
            observed_at: observed.map(at),
        }
    }

    #[test]
    fn a_paused_connector_has_no_number() {
        let r = row(Some(0), Some(0), Some(5), Some(10));
        assert_eq!(table_lag(Some(&r), true, true, at(20)), TableLag::Paused);
    }

    #[test]
    fn a_table_nobody_reported_is_not_known_yet() {
        assert_eq!(table_lag(None, true, false, at(0)), TableLag::NotKnownYet);
        // A last-change report alone proves nobody is looking at the backlog.
        let r = row(Some(0), None, None, None);
        assert_eq!(
            table_lag(Some(&r), true, false, at(5)),
            TableLag::NotKnownYet
        );
    }

    #[test]
    fn a_backlog_report_with_no_time_to_count_from_is_not_known_yet() {
        let r = row(None, None, Some(7), Some(0));
        assert_eq!(
            table_lag(Some(&r), true, false, at(10)),
            TableLag::NotKnownYet
        );
    }

    #[test]
    fn an_old_report_is_not_known_since_its_time_whatever_it_said() {
        let r = row(Some(0), Some(0), Some(0), Some(0));
        assert_eq!(
            table_lag(Some(&r), true, false, at(121)),
            TableLag::NotKnownSince { since: at(0) }
        );
        // Exactly two minutes is still current.
        assert_eq!(
            table_lag(Some(&r), true, false, at(120)),
            TableLag::UpToDate
        );
    }

    #[test]
    fn an_empty_topic_is_up_to_date_only_while_kafka_connect_runs() {
        let r = row(None, Some(50), Some(0), Some(50));
        assert_eq!(table_lag(Some(&r), true, false, at(60)), TableLag::UpToDate);
        // Stopped: nothing reaches the topic either, so 0 proves nothing; the
        // lag counts from the last moment the table was known up to date.
        assert_eq!(
            table_lag(Some(&r), false, false, at(100)),
            TableLag::Behind { seconds: 50 }
        );
    }

    #[test]
    fn lag_counts_from_the_later_of_the_last_change_and_the_last_caught_up_time() {
        // Quiet for a long time (caught up at 1000), then one change made at 900
        // is applied: the later time, 1000, is the anchor.
        let r = row(Some(900), Some(1000), Some(3), Some(1010));
        assert_eq!(
            table_lag(Some(&r), true, false, at(1030)),
            TableLag::Behind { seconds: 30 }
        );
        // The last change is later than the last caught-up time.
        let r = row(Some(1020), Some(1000), Some(3), Some(1025));
        assert_eq!(
            table_lag(Some(&r), true, false, at(1030)),
            TableLag::Behind { seconds: 10 }
        );
    }

    #[test]
    fn a_change_time_in_the_future_never_gives_a_negative_lag() {
        let r = row(Some(500), None, Some(1), Some(100));
        assert_eq!(
            table_lag(Some(&r), true, false, at(110)),
            TableLag::Behind { seconds: 0 }
        );
    }

    fn lags(values: &[(&str, TableLag)]) -> Vec<(String, TableLag)> {
        values.iter().map(|(n, l)| ((*n).to_owned(), *l)).collect()
    }

    #[test]
    fn the_worst_table_is_the_one_furthest_behind() {
        let l = lags(&[
            ("a", TableLag::Behind { seconds: 10 }),
            ("b", TableLag::Behind { seconds: 90 }),
            ("c", TableLag::UpToDate),
            ("d", TableLag::NotKnownYet),
        ]);
        assert_eq!(worst(&l), Some(("b", 90)));
        assert_eq!(worst(&lags(&[("c", TableLag::UpToDate)])), None);
    }

    #[test]
    fn the_alert_fires_once_when_the_limit_is_passed_and_not_again_while_open() {
        let over = lags(&[("a", TableLag::Behind { seconds: 61 })]);
        assert_eq!(
            alert_step(false, 1, &over),
            LagAlert::Crossed {
                table: "a".to_owned(),
                seconds: 61
            }
        );
        assert_eq!(alert_step(true, 1, &over), LagAlert::Nothing);
        // Exactly at the limit is not over it.
        let at_limit = lags(&[("a", TableLag::Behind { seconds: 60 })]);
        assert_eq!(alert_step(false, 1, &at_limit), LagAlert::Nothing);
    }

    #[test]
    fn the_second_alert_says_back_under_once_and_only_when_every_table_is_known() {
        let under = lags(&[
            ("a", TableLag::UpToDate),
            ("b", TableLag::Behind { seconds: 30 }),
        ]);
        assert_eq!(alert_step(true, 1, &under), LagAlert::BackUnder);
        assert_eq!(alert_step(false, 1, &under), LagAlert::Nothing);
        let unknown = lags(&[
            ("a", TableLag::UpToDate),
            ("b", TableLag::NotKnownSince { since: at(0) }),
        ]);
        assert_eq!(alert_step(true, 1, &unknown), LagAlert::Nothing);
    }

    #[test]
    fn an_unknown_table_does_not_hide_another_table_that_is_over() {
        let l = lags(&[
            ("a", TableLag::NotKnownYet),
            ("b", TableLag::Behind { seconds: 7200 }),
        ]);
        assert!(matches!(
            alert_step(false, 15, &l),
            LagAlert::Crossed { seconds: 7200, .. }
        ));
    }

    #[test]
    fn the_texts_hold_only_names_and_numbers() {
        assert_eq!(
            crossed_text("Orders DB", "conn-1", "public.orders", 125, 1),
            "Connector Orders DB (conn-1): change capture of public.orders is 2 min behind the \
             source, over the limit of 1 min. View: /connectors/conn-1"
        );
        assert_eq!(
            back_under_text("Orders DB", "conn-1", 1),
            "Connector Orders DB (conn-1): change capture is back under the limit of 1 min. \
             View: /connectors/conn-1"
        );
    }

    #[test]
    fn tables_with_no_stored_row_are_not_known_yet() {
        let stored = vec![row(Some(0), Some(0), Some(0), Some(0))];
        let selected = vec!["public.orders".to_owned(), "public.items".to_owned()];
        let l = tables_lag(&selected, &stored, true, false, at(10));
        assert_eq!(l[0].1, TableLag::UpToDate);
        assert_eq!(l[1].1, TableLag::NotKnownYet);
    }
}
