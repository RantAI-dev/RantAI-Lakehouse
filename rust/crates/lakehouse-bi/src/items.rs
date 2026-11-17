//! Favourites and recent items, per user (`BI-18` part C, part 2; decision
//! "Favourites, recent, search").
//!
//! # Where they are kept, and why
//!
//! In `console.bi_user_item` (`ClickHouse`), not Postgres: the things they
//! point at (dashboards and charts) live in `ClickHouse`, so a list is read
//! with the same store, an id needs no cross-database join, and no migration
//! number is spent (`main` and this branch both add to the Postgres series).
//! The table is the versioned-insert shape of every `console.bi_*` table; a
//! favourite removed is an `is_deleted = 1` row, and "recent" is the same key
//! rewritten with a newer time, so a user has at most one row per item.
//!
//! # What a read returns
//!
//! A saved id is resolved against the LIVE items the viewer may see
//! ([`crate::access::Viewer`]): an item in the trash, removed for good, or in
//! somebody else's personal space is simply absent from the list, never shown
//! as a name. The lists carry ids and names only.

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::SqlLiteral;
use serde::Serialize;
use serde_json::Value;

use crate::access::Viewer;
use crate::store::{BiError, DEFAULT_BOARD_ID, ensure_bi_table};

/// Most favourites a user keeps.
pub const MAX_FAVOURITES: usize = 100;

/// Recent items shown.
pub const RECENT_SHOWN: usize = 8;

/// Which of a user's lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    /// Items the user marked.
    Favourite,
    /// Items the user opened lately.
    Recent,
}

impl List {
    fn key(self) -> &'static str {
        match self {
            Self::Favourite => "favourite",
            Self::Recent => "recent",
        }
    }
}

/// What kind of thing an item is. Parsed from the request, so an unknown word
/// is refused rather than stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A dashboard.
    Board,
    /// A chart.
    Chart,
}

impl Kind {
    /// Parse the request's word.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "board" => Some(Self::Board),
            "chart" => Some(Self::Chart),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Board => "board",
            Self::Chart => "chart",
        }
    }
}

/// One entry of a list or of a search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// `"board"` or `"chart"`.
    pub kind: &'static str,
    /// The id.
    pub id: String,
    /// Dashboard name or chart title.
    pub name: String,
    /// For a chart, the dashboard it is on.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub board: String,
    /// For a chart, that dashboard's name.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub board_name: String,
}

fn row_str<'a>(row: &'a serde_json::Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Mark or unmark an item as a favourite.
///
/// Marking checks that the viewer can see the item, so a favourite cannot be
/// used to probe an id; unmarking does not need to.
///
/// # Errors
///
/// [`BiError::Validation`] when the item is not visible to `viewer` or the
/// user already has [`MAX_FAVOURITES`]; [`BiError::Clickhouse`] on a
/// `ClickHouse` failure.
pub async fn set_favourite(
    ch: &ChClient,
    viewer: &Viewer,
    kind: Kind,
    id: &str,
    on: bool,
) -> Result<(), BiError> {
    ensure_bi_table(ch).await?;
    if on {
        if resolve(ch, viewer, &[(kind, id.to_owned())])
            .await?
            .is_empty()
        {
            return Err(BiError::Validation("That item was not found.".to_owned()));
        }
        let held = ch
            .rows(
                &format!(
                    "SELECT count() AS n FROM console.bi_user_item FINAL \
                     WHERE user_id = {} AND list = 'favourite' AND is_deleted = 0 \
                       AND NOT (kind = {} AND item_id = {})",
                    SqlLiteral::from(viewer.id()),
                    SqlLiteral::from(kind.key()),
                    SqlLiteral::from(id)
                ),
                None,
            )
            .await?;
        let n: usize = held
            .first()
            .and_then(|r| r.get("n"))
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0);
        if n >= MAX_FAVOURITES {
            return Err(BiError::Validation(format!(
                "You can keep at most {MAX_FAVOURITES} favourites."
            )));
        }
    }
    write(ch, viewer, List::Favourite, kind, id, !on).await?;
    Ok(())
}

/// Note that the viewer opened an item. Best effort by design: the caller
/// ignores the result of a recent-list write.
///
/// # Errors
///
/// [`BiError::Validation`] when the item is not visible to `viewer`;
/// [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn touch_recent(
    ch: &ChClient,
    viewer: &Viewer,
    kind: Kind,
    id: &str,
) -> Result<(), BiError> {
    ensure_bi_table(ch).await?;
    if resolve(ch, viewer, &[(kind, id.to_owned())])
        .await?
        .is_empty()
    {
        return Err(BiError::Validation("That item was not found.".to_owned()));
    }
    write(ch, viewer, List::Recent, kind, id, false).await?;
    Ok(())
}

async fn write(
    ch: &ChClient,
    viewer: &Viewer,
    list: List,
    kind: Kind,
    id: &str,
    deleted: bool,
) -> Result<(), ChError> {
    ch.exec(
        &format!(
            "INSERT INTO console.bi_user_item (user_id, list, kind, item_id, is_deleted) VALUES ({}, {}, {}, {}, {})",
            SqlLiteral::from(viewer.id()),
            SqlLiteral::from(list.key()),
            SqlLiteral::from(kind.key()),
            SqlLiteral::from(id),
            u8::from(deleted)
        ),
        None,
    )
    .await
}

/// The viewer's list, newest first for recents and by name for favourites,
/// resolved to live items the viewer may see.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn entries(ch: &ChClient, viewer: &Viewer, list: List) -> Result<Vec<Entry>, ChError> {
    ensure_bi_table(ch).await?;
    let limit = match list {
        List::Favourite => MAX_FAVOURITES,
        List::Recent => RECENT_SHOWN * 3,
    };
    let rows = ch
        .rows(
            &format!(
                "SELECT kind, item_id FROM console.bi_user_item FINAL \
                 WHERE user_id = {} AND list = {} AND is_deleted = 0 \
                 ORDER BY at DESC LIMIT {limit}",
                SqlLiteral::from(viewer.id()),
                SqlLiteral::from(list.key())
            ),
            None,
        )
        .await?;
    let wanted: Vec<(Kind, String)> = rows
        .iter()
        .filter_map(|r| {
            Some((
                Kind::parse(row_str(r, "kind"))?,
                row_str(r, "item_id").to_owned(),
            ))
        })
        .collect();
    let mut found = resolve(ch, viewer, &wanted).await?;
    // The saved order is the order shown.
    found.sort_by_key(|e| {
        wanted
            .iter()
            .position(|(k, id)| k.key() == e.kind && *id == e.id)
            .unwrap_or(usize::MAX)
    });
    if list == List::Recent {
        found.truncate(RECENT_SHOWN);
    } else {
        found.sort_by_key(|e| e.name.to_lowercase());
    }
    Ok(found)
}

/// Keep the items among `wanted` that are live and visible to `viewer`, with
/// their names.
async fn resolve(
    ch: &ChClient,
    viewer: &Viewer,
    wanted: &[(Kind, String)],
) -> Result<Vec<Entry>, ChError> {
    let ids = |k: Kind| -> Vec<String> {
        wanted
            .iter()
            .filter(|(kind, _)| *kind == k)
            .map(|(_, id)| SqlLiteral::from(id.as_str()).to_string())
            .collect()
    };
    let mut out = Vec::new();
    let boards = ids(Kind::Board);
    if !boards.is_empty() {
        let rows = ch
            .rows(
                &format!(
                    "SELECT id, name FROM console.bi_board FINAL WHERE is_deleted = 0 AND id IN ({}){}",
                    boards.join(", "),
                    viewer.owner_clause()
                ),
                None,
            )
            .await?;
        out.extend(rows.iter().map(|r| Entry {
            kind: "board",
            id: row_str(r, "id").to_owned(),
            name: row_str(r, "name").to_owned(),
            board: String::new(),
            board_name: String::new(),
        }));
        // The built-in board has no row until its layout is saved, and is
        // always there.
        if viewer.sees_builtin()
            && wanted
                .iter()
                .any(|(k, id)| *k == Kind::Board && id == DEFAULT_BOARD_ID)
            && !out.iter().any(|e| e.id == DEFAULT_BOARD_ID)
        {
            out.push(Entry {
                kind: "board",
                id: DEFAULT_BOARD_ID.to_owned(),
                name: "Main".to_owned(),
                board: String::new(),
                board_name: String::new(),
            });
        }
    }
    let charts = ids(Kind::Chart);
    if !charts.is_empty() {
        let rows = ch
            .rows(
                &format!(
                    "SELECT id, title, board FROM console.bi_chart FINAL WHERE is_deleted = 0 AND id IN ({}){}",
                    charts.join(", "),
                    viewer.chart_clause()
                ),
                None,
            )
            .await?;
        let names = board_names(ch, rows.iter().map(|r| row_str(r, "board").to_owned())).await?;
        out.extend(rows.iter().map(|r| {
            let board = row_str(r, "board").to_owned();
            Entry {
                kind: "chart",
                id: row_str(r, "id").to_owned(),
                name: row_str(r, "title").to_owned(),
                board_name: names(&board),
                board,
            }
        }));
    }
    Ok(out)
}

/// A lookup from a dashboard id to its name, for the charts in a result.
pub(crate) async fn board_names(
    ch: &ChClient,
    boards: impl Iterator<Item = String>,
) -> Result<impl Fn(&str) -> String, ChError> {
    let mut ids: Vec<String> = boards
        .filter(|b| !b.is_empty() && b != DEFAULT_BOARD_ID)
        .collect();
    ids.sort();
    ids.dedup();
    let rows = if ids.is_empty() {
        Vec::new()
    } else {
        ch.rows(
            &format!(
                "SELECT id, name FROM console.bi_board FINAL WHERE is_deleted = 0 AND id IN ({})",
                ids.iter()
                    .map(|i| SqlLiteral::from(i.as_str()).to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            None,
        )
        .await?
    };
    let map: std::collections::HashMap<String, String> = rows
        .iter()
        .map(|r| (row_str(r, "id").to_owned(), row_str(r, "name").to_owned()))
        .collect();
    Ok(move |board: &str| {
        if board.is_empty() || board == DEFAULT_BOARD_ID {
            "Main".to_owned()
        } else {
            map.get(board).cloned().unwrap_or_default()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_kinds_are_accepted() {
        assert_eq!(Kind::parse("board"), Some(Kind::Board));
        assert_eq!(Kind::parse("chart"), Some(Kind::Chart));
        assert_eq!(Kind::parse("folder"), None);
        assert_eq!(Kind::parse(""), None);
    }
}
