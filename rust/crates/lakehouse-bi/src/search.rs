//! Search dashboards and charts by name (`BI-18` part C, part 2).
//!
//! One read per kind, each carrying the viewer's visibility in the statement
//! ([`crate::access::Viewer`]), so a hit is never fetched and then filtered:
//! another user's private draft and anything in the trash are not in the
//! result set at all. The text is a bound literal and matched with
//! `positionCaseInsensitiveUTF8`, never as a pattern, so `%` and `_` are
//! ordinary characters.
//!
//! Tiles that show text rather than data (`text`, `heading`, `divider`,
//! `section`) are not searched: a heading is a label on a dashboard, not
//! something a person looks for by name.

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::SqlLiteral;
use serde_json::Value;

use crate::access::Viewer;
use crate::items::{Entry, board_names};
use crate::store::{BiError, DEFAULT_BOARD_ID, ensure_bi_table};

/// Longest search text, in characters.
pub const MAX_QUERY_CHARS: usize = 100;

/// Most hits per kind.
pub const HITS_PER_KIND: usize = 8;

fn row_str<'a>(row: &'a serde_json::Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Dashboards, then charts, whose name contains `text`; names that start with
/// it first.
///
/// # Errors
///
/// [`BiError::Validation`] for a blank or over-long text;
/// [`BiError::Clickhouse`] on a `ClickHouse` failure.
pub async fn search(ch: &ChClient, viewer: &Viewer, text: &str) -> Result<Vec<Entry>, BiError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    if text.chars().count() > MAX_QUERY_CHARS {
        return Err(BiError::Validation(format!(
            "Search text can be at most {MAX_QUERY_CHARS} characters."
        )));
    }
    ensure_bi_table(ch).await?;
    Ok(find(ch, viewer, text).await?)
}

async fn find(ch: &ChClient, viewer: &Viewer, text: &str) -> Result<Vec<Entry>, ChError> {
    let needle = SqlLiteral::from(text);
    let boards = ch
        .rows(
            &format!(
                "SELECT id, name FROM console.bi_board FINAL WHERE is_deleted = 0 AND id != {default} \
                   AND positionCaseInsensitiveUTF8(name, {needle}) > 0{owner} \
                 ORDER BY positionCaseInsensitiveUTF8(name, {needle}) != 1, name LIMIT {HITS_PER_KIND}",
                default = SqlLiteral::from(DEFAULT_BOARD_ID),
                owner = viewer.owner_clause(),
            ),
            None,
        )
        .await?;
    // `kind` lives in the stored document: `{spec:{kind}}`, or at the top of a
    // chart saved before the envelope existed.
    let kind = "if(JSONHas(spec_json, 'spec'), JSONExtractString(spec_json, 'spec', 'kind'), JSONExtractString(spec_json, 'kind'))";
    let charts = ch
        .rows(
            &format!(
                "SELECT id, title, board FROM console.bi_chart FINAL WHERE is_deleted = 0 \
                   AND positionCaseInsensitiveUTF8(title, {needle}) > 0 \
                   AND {kind} NOT IN ('text', 'heading', 'divider', 'section'){chart} \
                 ORDER BY positionCaseInsensitiveUTF8(title, {needle}) != 1, title LIMIT {HITS_PER_KIND}",
                chart = viewer.chart_clause(),
            ),
            None,
        )
        .await?;
    let names = board_names(ch, charts.iter().map(|r| row_str(r, "board").to_owned())).await?;
    let mut out: Vec<Entry> = boards
        .iter()
        .map(|r| Entry {
            kind: "board",
            id: row_str(r, "id").to_owned(),
            name: row_str(r, "name").to_owned(),
            board: String::new(),
            board_name: String::new(),
        })
        .collect();
    // The built-in board has no row; it matches by its name like the rest.
    // BI-19 part B1: and belongs to the starting point, like the rest of the
    // permissions.
    if viewer.sees_builtin() && "main".contains(&text.to_lowercase()) {
        out.insert(
            0,
            Entry {
                kind: "board",
                id: DEFAULT_BOARD_ID.to_owned(),
                name: "Main".to_owned(),
                board: String::new(),
                board_name: String::new(),
            },
        );
    }
    out.extend(charts.iter().map(|r| {
        let board = row_str(r, "board").to_owned();
        Entry {
            kind: "chart",
            id: row_str(r, "id").to_owned(),
            name: row_str(r, "title").to_owned(),
            board_name: names(&board),
            board,
        }
    }));
    Ok(out)
}
