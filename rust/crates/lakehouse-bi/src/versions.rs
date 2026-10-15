//! Version history of a dashboard (`BI-18` part C, part 2; decision "Version
//! history" in `docs/superpowers/plans/2026-10-14-bi-18c-layout.md`).
//!
//! # What a version is
//!
//! `console.bi_board_version` holds, per dashboard, a numbered snapshot of the
//! dashboard's CONTENT as JSON ([`BoardState`]): name, description, layout,
//! filters, refresh, grain, parameter controls, tabs, phone layout, and the
//! definition of every chart on it. It deliberately holds nothing about
//! sharing (public link, signed embeds, framing sites), the folder, the owner
//! or the author: going back must never re-open a link that was turned off or
//! move a dashboard. The author recorded is whoever made the change.
//!
//! A version is written once per request that changed the content, after the
//! change succeeded ([`record`]); a change that leaves the content as it was
//! writes none. The last [`KEEP`] are kept: older ones are tombstoned in the
//! same versioned-insert shape as every `console.bi_*` table (no mutation).
//! A dashboard removed from the trash for good takes its history with it
//! ([`remove_history`]).
//!
//! # Going back
//!
//! The API route re-checks the stored state against today's rules before it
//! writes anything (a column or metric that no longer exists makes the whole
//! revert fail with a plain sentence) and then writes the state back as a NEW
//! version, so going back is itself undoable. Charts deleted since come back
//! only from the trash; charts added since move to the trash.

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::SqlLiteral;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::access::Viewer;
use crate::filters::FilterDef;
use crate::layout::{PhoneTile, Tab};
use crate::params::BoardParam;
use crate::store::{DEFAULT_BOARD_ID, LayoutMap, ensure_bi_table};

/// Versions kept per dashboard.
pub const KEEP: usize = 15;

/// Who made a change, as the history shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    /// Principal id, for the record; never shown.
    pub id: String,
    /// Display name, shown in the history.
    pub name: String,
}

/// One chart as a version holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateChart {
    /// Chart id.
    pub id: String,
    /// Title, for messages.
    pub title: String,
    /// `ui` or `ai`, as `bi_chart.created_by`.
    pub created_by: String,
    /// The stored `spec_json` document: `{ spec, def, hasYear }`.
    pub spec: Value,
}

/// The content of a dashboard at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardState {
    /// Format of this document, for a later reader.
    pub v: u32,
    /// Display name.
    pub name: String,
    /// One-line purpose.
    #[serde(default)]
    pub description: String,
    /// Tile layout.
    #[serde(default)]
    pub layout: LayoutMap,
    /// Dashboard filters.
    #[serde(default)]
    pub filters: Vec<FilterDef>,
    /// Saved auto-refresh in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_seconds: Option<u32>,
    /// Saved grain.
    #[serde(default)]
    pub grain: String,
    /// Shown parameters with their saved values.
    #[serde(default)]
    pub params: Vec<BoardParam>,
    /// Tabs.
    #[serde(default)]
    pub tabs: Vec<Tab>,
    /// Phone layout.
    #[serde(default)]
    pub phone_layout: Vec<PhoneTile>,
    /// Every chart on the dashboard.
    #[serde(default)]
    pub charts: Vec<StateChart>,
}

/// One row of the history, without the state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionEntry {
    /// Version number, 1 for the first.
    pub version: u32,
    /// Display name of whoever made the change.
    pub author: String,
    /// When, `ClickHouse`-formatted.
    pub at: String,
    /// What changed against the version before it, in a few words.
    pub summary: String,
    /// Charts on the dashboard in this version.
    pub charts: usize,
}

fn row_str<'a>(row: &'a serde_json::Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

fn row_u32(row: &serde_json::Map<String, Value>, key: &str) -> u32 {
    row.get(key)
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

/// Read the current content of dashboard `id`: `None` when it is not live.
/// The built-in board is read as "Main" with whatever layout it has saved.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn capture(ch: &ChClient, id: &str) -> Result<Option<BoardState>, ChError> {
    ensure_bi_table(ch).await?;
    let system = Viewer::system();
    let board = crate::store::get_board(ch, id, &system).await?;
    let (name, board) = match board {
        Some(b) => (b.name.clone(), Some(b)),
        None if id == DEFAULT_BOARD_ID => ("Main".to_owned(), None),
        None => return Ok(None),
    };
    let rows = ch
        .rows(
            &format!(
                "SELECT id, title, spec_json, created_by FROM console.bi_chart FINAL \
                 WHERE is_deleted = 0 AND (board = {id_lit}{blank}) ORDER BY created_at",
                id_lit = SqlLiteral::from(id),
                blank = if id == DEFAULT_BOARD_ID {
                    " OR board = ''"
                } else {
                    ""
                },
            ),
            None,
        )
        .await?;
    let charts = rows
        .iter()
        .filter_map(|r| {
            let spec: Value = serde_json::from_str(row_str(r, "spec_json")).ok()?;
            Some(StateChart {
                id: row_str(r, "id").to_owned(),
                title: row_str(r, "title").to_owned(),
                created_by: row_str(r, "created_by").to_owned(),
                spec,
            })
        })
        .collect();
    let b = board.as_ref();
    Ok(Some(BoardState {
        v: 1,
        name,
        description: b.and_then(|b| b.description.clone()).unwrap_or_default(),
        layout: b.and_then(|b| b.layout.clone()).unwrap_or_default(),
        filters: b.and_then(|b| b.filters.clone()).unwrap_or_default(),
        refresh_seconds: b.and_then(|b| b.refresh_seconds),
        grain: b.and_then(|b| b.grain.clone()).unwrap_or_default(),
        params: b.and_then(|b| b.params.clone()).unwrap_or_default(),
        tabs: b.and_then(|b| b.tabs.clone()).unwrap_or_default(),
        phone_layout: b.and_then(|b| b.phone_layout.clone()).unwrap_or_default(),
        charts,
    }))
}

/// Write a version of dashboard `id` if its content differs from the last one.
/// Returns the new number, `None` when nothing changed or the board is not
/// live. Call it once per request, after the change succeeded.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure (the caller logs it: a
/// history write that fails must not fail the change that already happened).
pub async fn record(ch: &ChClient, id: &str, author: &Author) -> Result<Option<u32>, ChError> {
    // The built-in dashboard is code plus a saved layout; it keeps no history.
    if id == DEFAULT_BOARD_ID {
        return Ok(None);
    }
    let Some(state) = capture(ch, id).await? else {
        return Ok(None);
    };
    let json = serde_json::to_string(&state).unwrap_or_else(|_| "{}".to_owned());
    let last = ch
        .rows(
            &format!(
                "SELECT version, state_json FROM console.bi_board_version FINAL \
                 WHERE board = {} AND is_deleted = 0 ORDER BY version DESC LIMIT 1",
                SqlLiteral::from(id)
            ),
            None,
        )
        .await?;
    if last
        .first()
        .is_some_and(|r| row_str(r, "state_json") == json)
    {
        return Ok(None);
    }
    // The highest number ever used, tombstones included, so a number is never
    // given twice.
    let top = ch
        .rows(
            &format!(
                "SELECT max(version) AS v FROM console.bi_board_version FINAL WHERE board = {}",
                SqlLiteral::from(id)
            ),
            None,
        )
        .await?;
    let next = top.first().map_or(0, |r| row_u32(r, "v")) + 1;
    ch.exec(
        &format!(
            "INSERT INTO console.bi_board_version (board, version, state_json, author_id, author_name) \
             VALUES ({}, {next}, {}, {}, {})",
            SqlLiteral::from(id),
            SqlLiteral::from(json),
            SqlLiteral::from(author.id.as_str()),
            SqlLiteral::from(author.name.as_str()),
        ),
        None,
    )
    .await?;
    prune(ch, id).await?;
    Ok(Some(next))
}

/// Tombstone every version of `id` beyond the newest [`KEEP`].
async fn prune(ch: &ChClient, id: &str) -> Result<(), ChError> {
    let old = ch
        .rows(
            &format!(
                "SELECT version FROM console.bi_board_version FINAL \
                 WHERE board = {} AND is_deleted = 0 ORDER BY version DESC LIMIT 1000 OFFSET {KEEP}",
                SqlLiteral::from(id)
            ),
            None,
        )
        .await?;
    if old.is_empty() {
        return Ok(());
    }
    let numbers: Vec<String> = old
        .iter()
        .map(|r| row_u32(r, "version").to_string())
        .collect();
    ch.exec(
        &format!(
            "INSERT INTO console.bi_board_version (board, version, state_json, is_deleted) \
             SELECT board, version, '', 1 FROM console.bi_board_version FINAL \
             WHERE board = {} AND is_deleted = 0 AND version IN ({})",
            SqlLiteral::from(id),
            numbers.join(", ")
        ),
        None,
    )
    .await
}

/// The history of dashboard `id`, newest first, each entry with what it
/// changed against the version before it.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn list(ch: &ChClient, id: &str) -> Result<Vec<VersionEntry>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT version, author_name, toString(created_at) AS created_at, state_json \
                 FROM console.bi_board_version FINAL WHERE board = {} AND is_deleted = 0 \
                 ORDER BY version DESC LIMIT {}",
                SqlLiteral::from(id),
                KEEP + 1
            ),
            None,
        )
        .await?;
    let states: Vec<Option<BoardState>> = rows
        .iter()
        .map(|r| serde_json::from_str::<BoardState>(row_str(r, "state_json")).ok())
        .collect();
    Ok(rows
        .iter()
        .enumerate()
        .take(KEEP)
        .map(|(i, r)| {
            let before = states.get(i + 1).and_then(Option::as_ref);
            let now = states.get(i).and_then(Option::as_ref);
            VersionEntry {
                version: row_u32(r, "version"),
                author: row_str(r, "author_name").to_owned(),
                at: row_str(r, "created_at").to_owned(),
                summary: now.map_or_else(String::new, |n| describe_change(before, n)),
                charts: now.map_or(0, |n| n.charts.len()),
            }
        })
        .collect())
}

/// One stored version of dashboard `id`: `None` when it is unknown, was
/// pruned or does not parse.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn load(ch: &ChClient, id: &str, version: u32) -> Result<Option<BoardState>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT state_json FROM console.bi_board_version FINAL \
                 WHERE board = {} AND version = {version} AND is_deleted = 0 LIMIT 1",
                SqlLiteral::from(id)
            ),
            None,
        )
        .await?;
    Ok(rows
        .first()
        .and_then(|r| serde_json::from_str::<BoardState>(row_str(r, "state_json")).ok()))
}

/// Drop the history of dashboards removed for good.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn remove_history(ch: &ChClient, boards: &[String]) -> Result<(), ChError> {
    if boards.is_empty() {
        return Ok(());
    }
    let list = boards
        .iter()
        .map(|b| SqlLiteral::from(b.as_str()).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    ch.exec(
        &format!(
            "INSERT INTO console.bi_board_version (board, version, state_json, is_deleted) \
             SELECT board, version, '', 1 FROM console.bi_board_version FINAL \
             WHERE is_deleted = 0 AND board IN ({list})"
        ),
        None,
    )
    .await
}

/// What changed from `before` to `now`, in a few words: "Created" for the
/// first, otherwise the parts that differ, or "Changed" when none of the named
/// parts does.
#[must_use]
pub fn describe_change(before: Option<&BoardState>, now: &BoardState) -> String {
    let Some(before) = before else {
        return "Created".to_owned();
    };
    let mut parts: Vec<String> = Vec::new();
    if before.name != now.name {
        parts.push("renamed".to_owned());
    }
    if before.description != now.description {
        parts.push("description".to_owned());
    }
    if before.layout != now.layout || before.phone_layout != now.phone_layout {
        parts.push("layout".to_owned());
    }
    if before.filters != now.filters {
        parts.push("filters".to_owned());
    }
    if before.tabs != now.tabs {
        parts.push("tabs".to_owned());
    }
    if before.refresh_seconds != now.refresh_seconds
        || before.grain != now.grain
        || before.params != now.params
    {
        parts.push("settings".to_owned());
    }
    let had = |id: &str| before.charts.iter().find(|c| c.id == id);
    let added = now.charts.iter().filter(|c| had(&c.id).is_none()).count();
    let removed = before
        .charts
        .iter()
        .filter(|c| !now.charts.iter().any(|n| n.id == c.id))
        .count();
    let edited = now
        .charts
        .iter()
        .filter(|c| had(&c.id).is_some_and(|b| b.spec != c.spec))
        .count();
    for (n, what) in [(added, "added"), (removed, "removed"), (edited, "edited")] {
        if n > 0 {
            parts.push(format!("{n} chart{} {what}", if n == 1 { "" } else { "s" }));
        }
    }
    if parts.is_empty() {
        return "Changed".to_owned();
    }
    let joined = parts.join(", ");
    let mut chars = joined.chars();
    chars.next().map_or_else(String::new, |f| {
        f.to_uppercase().collect::<String>() + chars.as_str()
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn state(name: &str, charts: &[(&str, i32)]) -> BoardState {
        BoardState {
            v: 1,
            name: name.to_owned(),
            description: String::new(),
            layout: LayoutMap::new(),
            filters: Vec::new(),
            refresh_seconds: None,
            grain: String::new(),
            params: Vec::new(),
            tabs: Vec::new(),
            phone_layout: Vec::new(),
            charts: charts
                .iter()
                .map(|(id, n)| StateChart {
                    id: (*id).to_owned(),
                    title: (*id).to_owned(),
                    created_by: "ui".to_owned(),
                    spec: serde_json::json!({ "n": n }),
                })
                .collect(),
        }
    }

    #[test]
    fn the_first_version_is_created_and_a_rename_is_named() {
        let first = state("A", &[]);
        assert_eq!(describe_change(None, &first), "Created");
        assert_eq!(describe_change(Some(&first), &state("B", &[])), "Renamed");
    }

    #[test]
    fn added_removed_and_edited_charts_are_counted() {
        let before = state("A", &[("u1", 1), ("u2", 1), ("u3", 1)]);
        let now = state("A", &[("u1", 1), ("u2", 2), ("u4", 1), ("u5", 1)]);
        assert_eq!(
            describe_change(Some(&before), &now),
            "2 charts added, 1 chart removed, 1 chart edited"
        );
    }

    #[test]
    fn a_state_with_no_named_difference_is_just_changed() {
        let s = state("A", &[]);
        assert_eq!(describe_change(Some(&s), &s.clone()), "Changed");
    }

    #[test]
    fn a_state_round_trips_and_leaves_sharing_out() {
        let json = serde_json::to_string(&state("A", &[("u1", 1)])).unwrap();
        assert!(!json.contains("public"), "{json}");
        assert!(!json.contains("embed"), "{json}");
        assert!(!json.contains("folder"), "{json}");
        assert!(!json.contains("owner"), "{json}");
        let back: BoardState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, state("A", &[("u1", 1)]));
    }
}
