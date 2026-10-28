//! The trash: deleted dashboards and charts, kept for 30 days (`BI-18` part C,
//! part 2; decision "Trash" in
//! `docs/superpowers/plans/2026-10-14-bi-18c-layout.md`).
//!
//! # Shape
//!
//! Deleting copies the live row with `is_deleted = 2`, a `deleted_at` and the
//! display name of who deleted it ([`crate::store::delete_board`],
//! [`crate::store::delete_chart`]). State `2` is deliberate: every read in the
//! workspace asks for `is_deleted = 0`, so a trashed row is invisible to a
//! path that has never heard of the trash. A new reader cannot forget to
//! exclude it, which is the property "every read path leaves the trash out"
//! needs.
//!
//! # Restore
//!
//! Restoring a dashboard brings back exactly the charts its delete took
//! (`deleted_with = <board>`), charts first so a failure in the middle leaves
//! the dashboard in the trash to retry. Restoring a chart on its own puts the
//! row back, unless its dashboard is in the trash too: then it is refused
//! ([`DASHBOARD_IN_TRASH`]), because restoring the dashboard does not bring back
//! a chart deleted before it. If the dashboard is gone it is a chart on no
//! dashboard.
//! The route requires `dashboard:write`, the permission that deletes.
//!
//! # Sweep
//!
//! [`purge_expired`] tombstones what has been in the trash longer than
//! [`RETENTION_DAYS`], and the version history of the dashboards it removes.
//! Fail closed: it reads the expired ids first, and a read that fails returns
//! the error with nothing deleted. A board's tombstone keeps its `owner_id`,
//! so a chart of a private board that was restored on its own after the board
//! was removed stays private.

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::SqlLiteral;
use serde::Serialize;
use serde_json::Value;

use crate::access::Viewer;
use crate::store::{BiError, DEFAULT_BOARD_ID, ensure_bi_table};

/// How long a deleted dashboard or chart stays restorable.
pub const RETENTION_DAYS: u32 = 30;

/// Most items one sweep removes, so a first run over a long backlog is
/// several small statements across hours, not one huge one.
const SWEEP_BATCH: usize = 500;

/// Most items the trash lists.
const LIST_LIMIT: usize = 200;

/// Said when the item is not in the trash for this viewer: unknown id, already
/// restored, removed for good, or somebody else's private draft.
const NOT_IN_TRASH: &str = "That item is not in the trash.";

/// Said when a chart is restored while its dashboard is in the trash too
/// (`BI-18` part C review fix (SHOULD-FIX) R10). The route answers it as a 409.
pub const DASHBOARD_IN_TRASH: &str =
    "Its dashboard is in the trash too: restore the dashboard first.";

/// One item in the trash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashItem {
    /// `"board"` or `"chart"`.
    pub kind: &'static str,
    /// The board or chart id.
    pub id: String,
    /// The dashboard name or chart title.
    pub name: String,
    /// For a chart, the dashboard it was on; `""` for a dashboard.
    pub board: String,
    /// For a chart, that dashboard's name (`""` when it is gone).
    pub board_name: String,
    /// For a chart: `"live"`, `"trash"` (it comes back with the dashboard) or
    /// `"gone"` (it will be on no dashboard). `""` for a dashboard.
    pub board_state: &'static str,
    /// Display name of whoever deleted it.
    pub deleted_by: String,
    /// When it was deleted, `ClickHouse`-formatted.
    pub deleted_at: String,
    /// When it is removed for good, `ClickHouse`-formatted.
    pub expires_at: String,
    /// Whether the dashboard is in somebody's personal space.
    pub personal: bool,
}

/// What a restore did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Restored {
    /// `"board"` or `"chart"`.
    pub kind: &'static str,
    /// The id restored.
    pub id: String,
    /// For a dashboard, how many of its charts came back with it.
    pub charts: u64,
    /// For a chart, where it went: `"live"`, `"trash"` or `"gone"` as in
    /// [`TrashItem::board_state`]; `""` for a dashboard.
    pub board_state: &'static str,
}

/// What a sweep removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Purged {
    /// Dashboards removed for good.
    pub boards: usize,
    /// Charts removed for good.
    pub charts: usize,
}

fn row_str<'a>(row: &'a serde_json::Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

/// A `UInt8` column: a number in `ClickHouse`'s JSON, a string for the 64-bit
/// ones, so both are accepted.
fn row_state(row: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    let v = row.get(key)?;
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

/// What a chart's dashboard is, from the dashboard's latest row (`None` = it
/// has none): live, in the trash, or gone (removed for good).
fn board_state_of(state: Option<u64>) -> &'static str {
    match state {
        Some(0) => "live",
        Some(2) => "trash",
        _ => "gone",
    }
}

/// `INSERT ... SELECT` that copies the trashed row of board `id` back to live.
fn restore_board_statement(id: &str) -> String {
    let cols = crate::store::BOARD_KEEP_COLS;
    format!(
        "INSERT INTO console.bi_board ({cols}, is_deleted, deleted_at, deleted_by) \
         SELECT {cols}, 0, toDateTime(0), '' \
         FROM console.bi_board FINAL WHERE is_deleted = 2 AND id = {}",
        SqlLiteral::from(id)
    )
}

/// The same for the charts a board's delete took.
fn restore_charts_of_statement(board: &str) -> String {
    format!(
        "INSERT INTO console.bi_chart (id, title, spec_json, board, created_by, is_deleted, deleted_at, deleted_by, deleted_with) \
         SELECT id, title, spec_json, board, created_by, 0, toDateTime(0), '', '' \
         FROM console.bi_chart FINAL WHERE is_deleted = 2 AND deleted_with = {}",
        SqlLiteral::from(board)
    )
}

/// The same for one chart.
fn restore_chart_statement(id: &str) -> String {
    format!(
        "INSERT INTO console.bi_chart (id, title, spec_json, board, created_by, is_deleted, deleted_at, deleted_by, deleted_with) \
         SELECT id, title, spec_json, board, created_by, 0, toDateTime(0), '', '' \
         FROM console.bi_chart FINAL WHERE is_deleted = 2 AND id = {}",
        SqlLiteral::from(id)
    )
}

/// The ids in the trash for longer than [`RETENTION_DAYS`], oldest first. A row
/// whose `deleted_at` is the zero default is never expired: that is a row we
/// cannot date, and a sweep never guesses.
fn expired_ids_statement(table: &str, id_col: &str) -> String {
    format!(
        "SELECT {id_col} AS id FROM console.{table} FINAL \
         WHERE is_deleted = 2 AND deleted_at > toDateTime('1971-01-01 00:00:00') \
           AND deleted_at < now() - INTERVAL {RETENTION_DAYS} DAY \
         ORDER BY deleted_at LIMIT {SWEEP_BATCH}"
    )
}

fn id_list(ids: &[String]) -> String {
    ids.iter()
        .map(|i| SqlLiteral::from(i.as_str()).to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The ids of the charts of dashboard `board` that are in the trash (deleted on
/// their own, or with the dashboard at some time). The version revert uses it to
/// tell a chart that can come back from one removed for good.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn trashed_chart_ids(
    ch: &ChClient,
    board: &str,
) -> Result<std::collections::HashSet<String>, ChError> {
    ensure_bi_table(ch).await?;
    Ok(ch
        .rows(
            &format!(
                "SELECT id FROM console.bi_chart FINAL WHERE is_deleted = 2 AND board = {}",
                SqlLiteral::from(board)
            ),
            None,
        )
        .await?
        .iter()
        .map(|r| row_str(r, "id").to_owned())
        .collect())
}

/// Everything in the trash that `viewer` may see: dashboards, and charts that
/// were deleted on their own (a chart that went with its dashboard is part of
/// that dashboard's entry). Newest first.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn list_trash(ch: &ChClient, viewer: &Viewer) -> Result<Vec<TrashItem>, ChError> {
    ensure_bi_table(ch).await?;
    let boards = ch
        .rows(
            &format!(
                "SELECT id, name, owner_id, deleted_by, toString(deleted_at) AS deleted_at, \
                        toString(deleted_at + INTERVAL {RETENTION_DAYS} DAY) AS expires_at \
                 FROM console.bi_board FINAL WHERE is_deleted = 2{} \
                 ORDER BY deleted_at DESC LIMIT {LIST_LIMIT}",
                viewer.owner_clause()
            ),
            None,
        )
        .await?;
    let charts = ch
        .rows(
            &format!(
                "SELECT id, title, board, deleted_by, toString(deleted_at) AS deleted_at, \
                        toString(deleted_at + INTERVAL {RETENTION_DAYS} DAY) AS expires_at \
                 FROM console.bi_chart FINAL WHERE is_deleted = 2 AND deleted_with = ''{} \
                 ORDER BY deleted_at DESC LIMIT {LIST_LIMIT}",
                viewer.chart_owner_clause()
            ),
            None,
        )
        .await?;
    // The dashboards the charts were on, with their state, in one read.
    let board_ids: Vec<String> = charts
        .iter()
        .map(|r| row_str(r, "board").to_owned())
        .filter(|b| !b.is_empty() && b != DEFAULT_BOARD_ID)
        .collect();
    let states = if board_ids.is_empty() {
        Vec::new()
    } else {
        ch.rows(
            &format!(
                "SELECT id, name, is_deleted FROM console.bi_board FINAL WHERE id IN ({})",
                id_list(&board_ids)
            ),
            None,
        )
        .await?
    };
    let mut out: Vec<TrashItem> = boards
        .iter()
        .map(|r| TrashItem {
            kind: "board",
            id: row_str(r, "id").to_owned(),
            name: row_str(r, "name").to_owned(),
            board: String::new(),
            board_name: String::new(),
            board_state: "",
            deleted_by: row_str(r, "deleted_by").to_owned(),
            deleted_at: row_str(r, "deleted_at").to_owned(),
            expires_at: row_str(r, "expires_at").to_owned(),
            personal: !row_str(r, "owner_id").is_empty(),
        })
        .collect();
    for r in &charts {
        let board = row_str(r, "board");
        let (board_name, board_state) = if board.is_empty() || board == DEFAULT_BOARD_ID {
            ("Main".to_owned(), "live")
        } else {
            match states.iter().find(|s| row_str(s, "id") == board) {
                Some(s) => (
                    row_str(s, "name").to_owned(),
                    board_state_of(row_state(s, "is_deleted")),
                ),
                None => (String::new(), "gone"),
            }
        };
        out.push(TrashItem {
            kind: "chart",
            id: row_str(r, "id").to_owned(),
            name: row_str(r, "title").to_owned(),
            board: board.to_owned(),
            board_name,
            board_state,
            deleted_by: row_str(r, "deleted_by").to_owned(),
            deleted_at: row_str(r, "deleted_at").to_owned(),
            expires_at: row_str(r, "expires_at").to_owned(),
            personal: false,
        });
    }
    out.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at));
    Ok(out)
}

/// Bring a dashboard back from the trash, with the charts its delete took.
///
/// The caller has checked `dashboard:write`. A dashboard whose folder has been
/// deleted since goes back to the root.
///
/// # Errors
///
/// [`BiError::Validation`] when `id` is not in the trash for `viewer` (unknown,
/// restored, removed for good, or another user's private draft);
/// [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn restore_board(ch: &ChClient, id: &str, viewer: &Viewer) -> Result<Restored, BiError> {
    ensure_bi_table(ch).await?;
    let found = ch
        .rows(
            &format!(
                "SELECT id, folder_id FROM console.bi_board FINAL WHERE is_deleted = 2 AND id = {}{} LIMIT 1",
                SqlLiteral::from(id),
                viewer.owner_clause()
            ),
            None,
        )
        .await?;
    let Some(row) = found.first() else {
        return Err(BiError::Validation(NOT_IN_TRASH.to_owned()));
    };
    let folder = row_str(row, "folder_id").to_owned();
    let charts = ch
        .rows(
            &format!(
                "SELECT count() AS n FROM console.bi_chart FINAL WHERE is_deleted = 2 AND deleted_with = {}",
                SqlLiteral::from(id)
            ),
            None,
        )
        .await?
        .first()
        .and_then(|r| r.get("n"))
        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or(0);
    // Charts first: while the board is in the trash they stay hidden, and a
    // failure below leaves the board in the trash to be restored again.
    ch.exec(&restore_charts_of_statement(id), None).await?;
    ch.exec(&restore_board_statement(id), None).await?;
    if !folder.is_empty() {
        let live = crate::folders::list_folders(ch, &Viewer::system()).await?;
        if !live.iter().any(|f| f.id == folder) {
            // Back to the root: never a personal space, so no link matters here.
            crate::store::move_board(ch, id, "", false).await?;
        }
    }
    Ok(Restored {
        kind: "board",
        id: id.to_owned(),
        charts,
        board_state: "",
    })
}

/// Bring a chart back from the trash. If its dashboard is in the trash too it
/// reappears when that is restored; if the dashboard is gone it is a chart on
/// no dashboard.
///
/// # Errors
///
/// [`BiError::Validation`] when `id` is not in the trash for `viewer`;
/// [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn restore_chart(ch: &ChClient, id: &str, viewer: &Viewer) -> Result<Restored, BiError> {
    ensure_bi_table(ch).await?;
    let found = ch
        .rows(
            &format!(
                "SELECT id, board FROM console.bi_chart FINAL WHERE is_deleted = 2 AND id = {}{} LIMIT 1",
                SqlLiteral::from(id),
                viewer.chart_owner_clause()
            ),
            None,
        )
        .await?;
    let Some(row) = found.first() else {
        return Err(BiError::Validation(NOT_IN_TRASH.to_owned()));
    };
    let board = row_str(row, "board").to_owned();
    let board_state = if board.is_empty() || board == DEFAULT_BOARD_ID {
        "live"
    } else {
        let state = ch
            .rows(
                &format!(
                    "SELECT is_deleted FROM console.bi_board FINAL WHERE id = {} LIMIT 1",
                    SqlLiteral::from(board.as_str())
                ),
                None,
            )
            .await?;
        board_state_of(state.first().and_then(|r| row_state(r, "is_deleted")))
    };
    // BI-18 part C review fix (SHOULD-FIX) R10: a chart deleted on its own
    // while its dashboard is in the trash too is restored after the dashboard,
    // which does not bring it back; the entry says so and this refuses.
    if board_state == "trash" {
        return Err(BiError::Validation(DASHBOARD_IN_TRASH.to_owned()));
    }
    ch.exec(&restore_chart_statement(id), None).await?;
    Ok(Restored {
        kind: "chart",
        id: id.to_owned(),
        charts: 0,
        board_state,
    })
}

/// Remove for good what has been in the trash longer than [`RETENTION_DAYS`],
/// and the history of the dashboards removed. Run by the API's hourly sweep.
///
/// Fail closed: the expired ids are read first and any failure returns the
/// error before a single row is written, so a sweep that cannot read deletes
/// nothing.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn purge_expired(ch: &ChClient) -> Result<Purged, ChError> {
    ensure_bi_table(ch).await?;
    let boards: Vec<String> = ch
        .rows(&expired_ids_statement("bi_board", "id"), None)
        .await?
        .iter()
        .map(|r| row_str(r, "id").to_owned())
        .filter(|id| !id.is_empty() && id != DEFAULT_BOARD_ID)
        .collect();
    let charts: Vec<String> = ch
        .rows(&expired_ids_statement("bi_chart", "id"), None)
        .await?
        .iter()
        .map(|r| row_str(r, "id").to_owned())
        .filter(|id| !id.is_empty())
        .collect();
    if !charts.is_empty() {
        // The `is_deleted = 2` guard re-checks the state at write time, so a
        // restore that landed after the read above is not undone.
        ch.exec(
            &format!(
                "INSERT INTO console.bi_chart (id, title, spec_json, board, created_by, is_deleted) \
                 SELECT id, '', '{{}}', board, 'system', 1 FROM console.bi_chart FINAL \
                 WHERE is_deleted = 2 AND id IN ({})",
                id_list(&charts)
            ),
            None,
        )
        .await?;
    }
    if !boards.is_empty() {
        // Keeps `owner_id`, so what is left of a private board's charts stays
        // private (see the module comment).
        ch.exec(
            &format!(
                "INSERT INTO console.bi_board (id, name, owner_id, is_deleted) \
                 SELECT id, '', owner_id, 1 FROM console.bi_board FINAL \
                 WHERE is_deleted = 2 AND id IN ({})",
                id_list(&boards)
            ),
            None,
        )
        .await?;
        crate::versions::remove_history(ch, &boards).await?;
    }
    Ok(Purged {
        boards: boards.len(),
        charts: charts.len(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_restore_copies_the_trashed_row_back_as_live() {
        let sql = restore_board_statement("b_1");
        assert!(sql.contains("is_deleted = 2 AND id = 'b_1'"), "{sql}");
        assert!(sql.contains("SELECT id, name"), "{sql}");
        assert!(sql.contains(", 0, toDateTime(0), ''"), "{sql}");
        let charts = restore_charts_of_statement("b_1");
        assert!(charts.contains("deleted_with = 'b_1'"), "{charts}");
        assert!(restore_chart_statement("u_1").contains("id = 'u_1'"));
    }

    #[test]
    fn a_dashboards_state_is_read_from_a_number_or_a_string() {
        let row = |v: Value| {
            let mut m = serde_json::Map::new();
            m.insert("is_deleted".to_owned(), v);
            m
        };
        assert_eq!(
            board_state_of(row_state(&row(serde_json::json!(0)), "is_deleted")),
            "live"
        );
        assert_eq!(
            board_state_of(row_state(&row(serde_json::json!(2)), "is_deleted")),
            "trash"
        );
        assert_eq!(
            board_state_of(row_state(&row(serde_json::json!("2")), "is_deleted")),
            "trash"
        );
        assert_eq!(
            board_state_of(row_state(&row(serde_json::json!(1)), "is_deleted")),
            "gone"
        );
        assert_eq!(board_state_of(None), "gone");
    }

    #[test]
    fn a_value_with_a_quote_is_escaped_in_every_statement() {
        for sql in [
            restore_board_statement("b'1"),
            restore_charts_of_statement("b'1"),
            restore_chart_statement("b'1"),
        ] {
            assert!(sql.contains("'b''1'"), "{sql}");
        }
    }

    #[test]
    fn the_sweep_reads_only_dated_items_older_than_thirty_days() {
        let sql = expired_ids_statement("bi_board", "id");
        assert!(sql.contains("is_deleted = 2"), "{sql}");
        assert!(sql.contains("INTERVAL 30 DAY"), "{sql}");
        assert!(sql.contains("deleted_at > toDateTime('1971-01-01"), "{sql}");
        assert!(sql.contains("LIMIT 500"), "{sql}");
    }
}
