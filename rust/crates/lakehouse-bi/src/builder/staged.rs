//! The statement of a chart with table calculations, period comparisons
//! (`BI-8` part 2). Everything else about a chart is built in [`super`].
//!
//! # Shape
//!
//! ```text
//! SELECT dim, brk, names           -- 4. the chart's order and limit
//! FROM (SELECT ..., dense_rank() OVER (ORDER BY <key>) AS __rk   -- 3. which groups are kept
//!   FROM (SELECT dim, brk, <plain measures>, <table calculations> AS name   -- 2. windows
//!     FROM (SELECT cur.*, p0.<hidden> AS <shifted> ...                     -- 1. shifted copies
//!       FROM (<grouped>) AS cur LEFT JOIN (<grouped>) AS p0 ON ...)))      -- 0. GROUP BY
//! ```
//!
//! The windows run over the *whole* grouped result, so a running total at
//! the latest bucket is the true running total, not the total of the buckets
//! kept; the chart's limit is applied afterwards, by rank. With a breakdown
//! the dimension values are kept in dimension order. The grouped result may
//! not exceed [`MAX_GROUPS`] groups: past that the engine refuses
//! (`group_by_overflow_mode = 'throw'`) rather than cutting the data and
//! letting a total or a rank be wrong without saying so.
//!
//! # Safety of the SQL
//!
//! Identifiers are validated [`Ident`]s or hash aliases the compiler made;
//! window text comes from `formula::compile::staged`; the join conditions are
//! written here from a closed table of function names.

use lakehouse_core::ident::Ident;

use super::{QueryBuilder, Ready, add_settings};
use crate::formula::compile::{Shift, ShiftKind};
use crate::grain::{self, Grain};

/// Most groups the grouped result of such a chart may have.
pub const MAX_GROUPS: u32 = 10_000;

/// The engine function that moves `column` forward by the period `shift` looks
/// back over, so `cur.dim = <this>(prior.dim)` pairs a bucket with the one
/// before it. A week looks a year back as 52 weeks, which keeps the weekday.
fn forward(kind: ShiftKind, grain: Grain, column: &str) -> Option<String> {
    let (func, n) = match (kind, grain) {
        (ShiftKind::Previous, Grain::Minute) => ("addMinutes", 1),
        (ShiftKind::Previous, Grain::Hour) => ("addHours", 1),
        (ShiftKind::Previous, Grain::Day) => ("addDays", 1),
        (ShiftKind::Previous, Grain::Week) => ("addWeeks", 1),
        (ShiftKind::Previous, Grain::Month) => ("addMonths", 1),
        (ShiftKind::Previous, Grain::Quarter) => ("addQuarters", 1),
        (ShiftKind::Previous | ShiftKind::LastYear, Grain::Year)
        | (ShiftKind::LastYear, Grain::Day | Grain::Month | Grain::Quarter) => ("addYears", 1),
        (ShiftKind::LastYear, Grain::Week) => ("addWeeks", 52),
        _ => return None,
    };
    Some(format!("{func}({column}, {n})"))
}

impl QueryBuilder<Ready> {
    /// The statement, or `None` when no measure is a table calculation (the
    /// ordinary builder then runs). An impossible shape (no dimension, a
    /// period comparison without a grain) yields an empty statement, like the
    /// ordinary builder's missing-measure case; the field checks make it
    /// unreachable for a saved chart.
    pub(super) fn staged_sql(&self) -> Option<String> {
        let calcs: Vec<_> = self
            .measures
            .iter()
            .filter_map(|m| self.from.table_calc(m.as_str()))
            .collect();
        if calcs.is_empty() {
            return None;
        }
        Some(self.staged(&calcs).unwrap_or_default())
    }

    fn staged(&self, calcs: &[&super::TableCalc]) -> Option<String> {
        let dim: &Ident = self.dimension.as_ref()?;
        let brk = self.breakdown.as_ref();
        let key = self
            .grain
            .map_or_else(|| dim.to_string(), |(g, ws)| g.order_key(dim.as_str(), ws));
        let latest = self
            .grain
            .is_some_and(|(g, _)| grain::cuts_to_latest(g, &self.order));
        let keep = self.limit + u32::from(latest);
        let is_calc = |m: &Ident| calcs.iter().any(|c| c.name == m.as_str());

        // 0. the grouped result with the hidden columns.
        // A plain measure is named `__m<i>` here and takes its own name in
        // step 2: `round(sum(v)) AS v` beside a hidden `sum(v)` would make the
        // engine read the second `v` as the first (error 184, measured).
        let mut items: Vec<(String, String)> = Vec::new();
        for (i, m) in self
            .measures
            .iter()
            .enumerate()
            .filter(|(_, m)| !is_calc(m))
        {
            items.push((format!("__m{i}"), format!("{} AS __m{i}", self.agg_expr(m))));
        }
        for c in calcs {
            for h in &c.aggs {
                if !items.iter().any(|(n, _)| *n == h.alias) {
                    items.push((h.alias.clone(), format!("{} AS {}", h.sql, h.alias)));
                }
            }
        }
        let group_cols = brk.map_or_else(|| dim.to_string(), |b| format!("{dim}, {b}"));
        let where_sql = if self.where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {} ", self.where_clauses.join(" AND "))
        };
        let grouped = format!(
            "SELECT {group_cols}, {} FROM {} {where_sql}GROUP BY {group_cols}",
            items
                .iter()
                .map(|(_, e)| e.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            self.from.render()
        );

        // 1. shifted copies, by calendar date.
        let staged = self.shifted(calcs, &grouped, &items)?;

        // 2. the measures, windows over the grouped result.
        let names: Vec<String> = self.measures.iter().map(ToString::to_string).collect();
        let wrapped_items: Vec<String> = self
            .measures
            .iter()
            .enumerate()
            .map(|(i, m)| {
                calcs
                    .iter()
                    .find(|c| c.name == m.as_str())
                    .map_or_else(|| format!("__m{i} AS {m}"), |c| format!("{} AS {m}", c.sql))
            })
            .collect();
        let head = brk.map_or_else(|| dim.to_string(), |b| format!("{dim}, {b}"));
        let wrapped = format!(
            "SELECT {head}, {} FROM ({staged})",
            wrapped_items.join(", ")
        );

        // 3. which groups are kept, by rank.
        let first = names.first()?;
        let by_value = self.order != "none" && brk.is_none();
        let dir = if self.order == "asc" { "ASC" } else { "DESC" };
        let rank_key = if latest {
            format!("{key} DESC")
        } else if by_value {
            format!("{first} {dir}")
        } else {
            key.clone()
        };
        let all_names = names.join(", ");
        let ranked = format!(
            "SELECT {head}, {all_names}, dense_rank() OVER (ORDER BY {rank_key}) AS __rk FROM ({wrapped})"
        );

        // 4. the chart's order.
        let order = if by_value {
            format!("{first} {dir}")
        } else {
            brk.map_or_else(|| key.clone(), |b| format!("{key}, {b}"))
        };
        let settings = add_settings(
            &self.from.settings(),
            &[
                &format!("max_rows_to_group_by = {MAX_GROUPS}"),
                "group_by_overflow_mode = 'throw'",
                "join_use_nulls = 1",
            ],
        );
        Some(format!(
            "SELECT {head}, {all_names} FROM ({ranked}) WHERE __rk <= {keep} ORDER BY {order}{settings}"
        ))
    }

    /// Step 1: `grouped` joined to itself once per shifted copy, on the
    /// calendar shift of the dimension (and the breakdown, when there is one).
    /// `grouped` itself when no calculation shifts anything.
    fn shifted(
        &self,
        calcs: &[&super::TableCalc],
        grouped: &str,
        items: &[(String, String)],
    ) -> Option<String> {
        let dim: &Ident = self.dimension.as_ref()?;
        let brk = self.breakdown.as_ref();
        let mut shifts: Vec<&Shift> = Vec::new();
        for c in calcs {
            for s in &c.shifts {
                if !shifts.iter().any(|x| x.alias == s.alias) {
                    shifts.push(s);
                }
            }
        }
        if shifts.is_empty() {
            return Some(grouped.to_owned());
        }
        let (g, _) = self.grain?;
        let mut cols: Vec<String> = vec![format!("cur.{dim} AS {dim}")];
        cols.extend(brk.map(|b| format!("cur.{b} AS {b}")));
        cols.extend(items.iter().map(|(n, _)| format!("cur.{n} AS {n}")));
        let mut joins = String::new();
        for (i, s) in shifts.iter().enumerate() {
            cols.push(format!("p{i}.{} AS {}", s.source, s.alias));
            let on = forward(s.kind, g, &format!("p{i}.{dim}"))?;
            let same = brk.map_or_else(String::new, |b| format!(" AND cur.{b} = p{i}.{b}"));
            joins = format!("{joins} LEFT JOIN ({grouped}) AS p{i} ON cur.{dim} = {on}{same}");
        }
        Some(format!(
            "SELECT {} FROM ({grouped}) AS cur{joins}",
            cols.join(", ")
        ))
    }
}
