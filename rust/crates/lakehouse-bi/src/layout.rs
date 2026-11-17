//! Dashboard tabs, the ties of a filter to a tab, and the phone layout
//! (`BI-18` part C, `docs/superpowers/plans/2026-10-14-bi-18c-layout.md`).
//!
//! A board may be split into tabs. Every tile of its layout then carries the
//! tab it sits on ([`crate::store::TileBox::tab`]), and a board with no tabs
//! is one implicit tab that renders exactly as before. A tile that names no
//! tab (a chart added after the tabs were, or a board saved by an older
//! console) sits on the first tab: [`tab_of`] is the one place that decides.
//!
//! A dashboard filter may be tied to some tabs ([`FilterDef::tabs`]). The tie
//! only chooses which filters a tile is given ([`filters_for_tab`]); it never
//! reaches a statement, and a tile on a tab the filter is not tied to is not
//! told the filter was skipped. A tie to a tab that does not exist is refused
//! at save ([`check_filter_ties`]) so a deleted tab cannot leave a filter that
//! silently applies nowhere.
//!
//! The phone layout is an order and a height per tile, never a width: below
//! 640 px every tile is full width. It is a list so the order survives any
//! JSON reader.
//!
//! Everything here is plain data and checks: no SQL, no `ClickHouse`.

use std::borrow::Cow;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::filters::FilterDef;
use crate::specs::ChartKind;
use crate::store::LayoutMap;

/// Most tabs a board may have.
pub const MAX_TABS: usize = 20;
/// Longest tab title, in characters.
pub const TAB_TITLE_MAX_CHARS: usize = 60;
/// Longest tab id, in characters.
pub const TAB_ID_MAX_CHARS: usize = 40;
/// Most tiles a phone layout may order.
pub const MAX_PHONE_TILES: usize = 500;
/// Tallest a tile may be in a phone layout, in grid rows.
pub const MAX_PHONE_ROWS: i32 = 60;
/// Longest chart id a phone layout may name.
const PHONE_ID_MAX_CHARS: usize = 64;

/// One tab of a board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tab {
    /// Stable id the tiles and the filters refer to; it does not change when
    /// the tab is renamed or moved.
    pub id: String,
    /// The title shown on the tab, 1 to [`TAB_TITLE_MAX_CHARS`] characters.
    pub title: String,
}

/// A tile in the phone layout: its place in the one column and its height.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneTile {
    /// The chart id.
    pub id: String,
    /// Height in grid rows.
    pub h: i32,
}

/// A tab id's shape: 1 to [`TAB_ID_MAX_CHARS`] letters, digits, `_` or `-`.
///
/// # Errors
///
/// A message for the caller. It never echoes the offending value.
pub fn check_tab_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.chars().count() <= TAB_ID_MAX_CHARS
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "a tab id is 1 to {TAB_ID_MAX_CHARS} letters, digits, _ or -"
        ))
    }
}

/// Check a board's tabs.
///
/// # Errors
///
/// A message for the caller: more than [`MAX_TABS`] tabs, an id of the wrong
/// shape or used twice, or a title that is blank, too long or has a control
/// character.
pub fn validate_tabs(tabs: &[Tab]) -> Result<(), String> {
    if tabs.len() > MAX_TABS {
        return Err(format!("a dashboard has at most {MAX_TABS} tabs."));
    }
    let mut seen = HashSet::new();
    for tab in tabs {
        check_tab_id(&tab.id).map_err(|m| format!("{m}."))?;
        if !seen.insert(tab.id.as_str()) {
            return Err("two tabs have the same id.".to_owned());
        }
        let title = tab.title.trim();
        if title.is_empty() {
            return Err("a tab needs a title.".to_owned());
        }
        if title.chars().count() > TAB_TITLE_MAX_CHARS {
            return Err(format!(
                "a tab title is at most {TAB_TITLE_MAX_CHARS} characters."
            ));
        }
        if title.chars().any(char::is_control) {
            return Err("a tab title is one line of text.".to_owned());
        }
    }
    Ok(())
}

/// The tabs as they are stored: titles trimmed.
#[must_use]
pub fn clean_tabs(tabs: &[Tab]) -> Vec<Tab> {
    tabs.iter()
        .map(|t| Tab {
            id: t.id.clone(),
            title: t.title.trim().to_owned(),
        })
        .collect()
}

/// Check that every tab a tile names is one of `tabs`. A board with no tabs
/// has tiles that name none.
///
/// # Errors
///
/// A message for the caller.
pub fn check_layout_tabs(layout: &LayoutMap, tabs: &[Tab]) -> Result<(), String> {
    for tile in layout.values() {
        let Some(tab) = tile.tab.as_deref() else {
            continue;
        };
        if !tabs.iter().any(|t| t.id == tab) {
            return Err("a tile is on a tab that does not exist.".to_owned());
        }
    }
    Ok(())
}

/// Check that every tab a filter is tied to is one of `tabs`.
///
/// # Errors
///
/// A message for the caller.
pub fn check_filter_ties(filters: &[FilterDef], tabs: &[Tab]) -> Result<(), String> {
    for f in filters {
        for tie in f.tabs.iter().flatten() {
            if !tabs.iter().any(|t| &t.id == tie) {
                return Err("a filter is tied to a tab that does not exist.".to_owned());
            }
        }
    }
    Ok(())
}

/// Check a phone layout.
///
/// # Errors
///
/// A message for the caller: too many tiles, a repeated or oversized id, or a
/// height outside `1..=`[`MAX_PHONE_ROWS`].
pub fn validate_phone_layout(phone: &[PhoneTile]) -> Result<(), String> {
    if phone.len() > MAX_PHONE_TILES {
        return Err(format!(
            "a phone layout orders at most {MAX_PHONE_TILES} tiles."
        ));
    }
    let mut seen = HashSet::new();
    for tile in phone {
        if tile.id.is_empty() || tile.id.chars().count() > PHONE_ID_MAX_CHARS {
            return Err("a phone layout names a tile that is not valid.".to_owned());
        }
        if !seen.insert(tile.id.as_str()) {
            return Err("a phone layout names a tile twice.".to_owned());
        }
        if !(1..=MAX_PHONE_ROWS).contains(&tile.h) {
            return Err(format!(
                "a tile in the phone layout is 1 to {MAX_PHONE_ROWS} rows high."
            ));
        }
    }
    Ok(())
}

/// The tab a chart sits on: the one its tile names, else the first tab (a
/// chart with no tile yet, or a tile saved before the tabs). `None` when the
/// board has no tabs.
#[must_use]
pub fn tab_of<'a>(tabs: &'a [Tab], layout: &LayoutMap, chart_id: &str) -> Option<&'a str> {
    let first = tabs.first()?;
    let named = layout
        .get(chart_id)
        .and_then(|tile| tile.tab.as_deref())
        .and_then(|name| tabs.iter().find(|t| t.id == name));
    Some(named.unwrap_or(first).id.as_str())
}

/// Write the first tab on every tile that names none (`BI-18` part C review
/// fix (BLOCKER) R1). A tile with no tab falls back to the first tab BY POSITION
/// when it is read, so leaving it untagged lets a reorder of the tabs carry it
/// to whatever tab is first afterwards. Called on every write of a board's
/// layout or tabs with `before`, the tabs as stored before the write: those
/// tiles are on `before`'s first tab; only a board that had no tabs takes the
/// first of `after`. A tab that `after` no longer has falls back to `after`'s
/// first. A board that ends with no tabs has no tile naming one. Returns whether
/// a tile was changed.
pub fn stamp_untagged(layout: &mut LayoutMap, before: &[Tab], after: &[Tab]) -> bool {
    let Some(first_after) = after.first() else {
        // A board with no tabs has tiles that name none (deleting the last tab
        // can reach here after the layout write stamped them).
        let mut changed = false;
        for tile in layout.values_mut().filter(|t| t.tab.is_some()) {
            tile.tab = None;
            changed = true;
        }
        return changed;
    };
    let target = untagged_tab(before, after)
        .unwrap_or(first_after.id.as_str())
        .to_owned();
    let mut changed = false;
    for tile in layout.values_mut().filter(|t| t.tab.is_none()) {
        tile.tab = Some(target.clone());
        changed = true;
    }
    changed
}

/// The tab a tile that names none sits on while `before` becomes `after`: the
/// first of `before` while `after` still has it, else the first of `after`;
/// `None` when `after` has no tab.
fn untagged_tab<'a>(before: &'a [Tab], after: &'a [Tab]) -> Option<&'a str> {
    let first_after = after.first()?;
    Some(
        before
            .first()
            .filter(|b| after.iter().any(|t| t.id == b.id))
            .unwrap_or(first_after)
            .id
            .as_str(),
    )
}

/// A live chart of a board, as far as its place on the grid is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveChart {
    /// The chart's id, the key of its tile.
    pub id: String,
    /// The chart's grid span (`2` = full width), as the builder saved it.
    pub span: Option<u8>,
    /// The chart's kind: a heading, a section header and a divider are one row high.
    pub kind: ChartKind,
}

/// Grid columns, as `COLS` in `dashboard-grid.tsx`.
const GRID_COLS: i32 = 12;
/// Width and height of a chart tile with no box, as `DEFAULT` there.
const DEFAULT_TILE: (i32, i32) = (6, 6);

/// Give every live chart that has no tile a box (`BI-18` part C review fix
/// (BLOCKER) R13). The console drew such a chart wherever its grid's `resolve()`
/// put it and never stored the place, so the server did not know it: a tab
/// reorder, a move and the closing of a space all missed it. The rule is
/// `resolve()`'s, in `dashboard-grid.tsx` (`placeUnboxed` in
/// `src/lib/dashboard-tabs.ts`), and both are tested on
/// `ops/fixtures/place_unboxed.json`: below the stored bottom of the first
/// tab (of the board with no tabs), in the order of `charts` (creation order),
/// half width (6 by 6) or 12 wide for `span: 2`, a heading, section header or
/// divider one row high, two half-width tiles to a row and a row as tall as its
/// tallest tile. On a tabbed board the new tile names the tab a tile with none
/// sits on ([`stamp_untagged`]'s target), so call this after it. The bottom
/// counts every stored tile of that tab, even one whose chart is gone: it can
/// only leave a gap, never an overlap. Returns whether a tile was added.
pub fn place_unboxed(
    layout: &mut LayoutMap,
    before: &[Tab],
    after: &[Tab],
    charts: &[LiveChart],
) -> bool {
    let tab = untagged_tab(before, after).map(str::to_owned);
    let mut y = match &tab {
        Some(t) => bottom_of_tab(after, layout, t, &HashSet::new()),
        None => layout.values().map(|t| t.y + t.h).max().unwrap_or(0),
    };
    let (mut col, mut row_h, mut added) = (0, 0, false);
    for chart in charts {
        if layout.contains_key(&chart.id) {
            continue;
        }
        let w = if chart.span == Some(2) {
            GRID_COLS
        } else {
            DEFAULT_TILE.0
        };
        let h = match chart.kind {
            ChartKind::Heading | ChartKind::Section | ChartKind::Divider => 1,
            _ => DEFAULT_TILE.1,
        };
        if col + w > GRID_COLS {
            y += row_h;
            (col, row_h) = (0, 0);
        }
        layout.insert(
            chart.id.clone(),
            crate::store::TileBox {
                x: col,
                y,
                w,
                h,
                tab: tab.clone(),
            },
        );
        added = true;
        col += w;
        row_h = row_h.max(h);
        if col >= GRID_COLS {
            y += row_h;
            (col, row_h) = (0, 0);
        }
    }
    added
}

/// The filters a tile on `tab` is given.
#[must_use]
pub fn filters_for_tab(filters: &[FilterDef], tab: Option<&str>) -> Vec<FilterDef> {
    filters
        .iter()
        .filter(|f| f.applies_on(tab))
        .cloned()
        .collect()
}

/// The filters the tile of `chart_id` is given: all of them when none is tied
/// to a tab (nothing is copied), else the ones that apply on the chart's tab
/// ([`tab_of`]). Every read that applies the board's filters to a tile asks
/// this, so the dashboard, an embed and a public link agree.
#[must_use]
pub fn filters_for_chart<'a>(
    filters: &'a [FilterDef],
    tabs: &[Tab],
    layout: &LayoutMap,
    chart_id: &str,
) -> Cow<'a, [FilterDef]> {
    let tied = |f: &FilterDef| f.tabs.as_ref().is_some_and(|t| !t.is_empty());
    if tabs.is_empty() || !filters.iter().any(tied) {
        return Cow::Borrowed(filters);
    }
    Cow::Owned(filters_for_tab(filters, tab_of(tabs, layout, chart_id)))
}

/// A fresh tab id: `t_` and eight hex digits.
#[must_use]
pub fn new_tab_id() -> String {
    format!("t_{}", crate::store::random_hex(4))
}

/// The tab `key` names: by id, else by title (case-insensitively), so a person
/// or the assistant can say "Maps" as well as `t_1a2b3c4d`.
#[must_use]
pub fn find_tab<'a>(tabs: &'a [Tab], key: &str) -> Option<&'a Tab> {
    let key = key.trim();
    tabs.iter().find(|t| t.id == key).or_else(|| {
        let mut by_title = tabs.iter().filter(|t| t.title.eq_ignore_ascii_case(key));
        let first = by_title.next();
        // Two tabs with one title are ambiguous: name neither.
        first.filter(|_| by_title.next().is_none())
    })
}

/// The y a tile moved onto `tab` starts at: below everything there already.
fn bottom_of_tab(tabs: &[Tab], layout: &LayoutMap, tab: &str, except: &HashSet<&str>) -> i32 {
    layout
        .iter()
        .filter(|(id, _)| !except.contains(id.as_str()))
        .filter(|(id, _)| tab_of(tabs, layout, id) == Some(tab))
        .map(|(_, t)| t.y + t.h)
        .max()
        .unwrap_or(0)
}

/// A new tab at the end. Adding the first tab puts every existing tile in it
/// (decision 1 of the plan), so a board that had none keeps showing
/// everything.
///
/// # Errors
///
/// A message for the caller: at [`MAX_TABS`], or tabs that are not valid.
pub fn add_tab(
    tabs: &[Tab],
    layout: &LayoutMap,
    title: &str,
    id: &str,
) -> Result<(Vec<Tab>, LayoutMap), String> {
    let mut next = tabs.to_vec();
    next.push(Tab {
        id: id.to_owned(),
        title: title.to_owned(),
    });
    validate_tabs(&next)?;
    let mut layout = layout.clone();
    if tabs.is_empty() {
        for tile in layout.values_mut() {
            tile.tab = Some(id.to_owned());
        }
    }
    Ok((clean_tabs(&next), layout))
}

/// Rename a tab.
///
/// # Errors
///
/// A message for the caller: no such tab, or a title that is not valid.
pub fn rename_tab(tabs: &[Tab], id: &str, title: &str) -> Result<Vec<Tab>, String> {
    if !tabs.iter().any(|t| t.id == id) {
        return Err("that tab does not exist.".to_owned());
    }
    let next: Vec<Tab> = tabs
        .iter()
        .map(|t| Tab {
            id: t.id.clone(),
            title: if t.id == id {
                title.to_owned()
            } else {
                t.title.clone()
            },
        })
        .collect();
    validate_tabs(&next)?;
    Ok(clean_tabs(&next))
}

/// Put the tabs in a new order. `order` names every tab id exactly once.
///
/// The ids, titles and the tiles' tabs are unchanged, so nothing but the order
/// of the tab bar moves (`BI-18` part C, planner answer 6).
///
/// # Errors
///
/// A message for the caller when `order` leaves a tab out, repeats one or
/// names one the board does not have.
pub fn reorder_tabs(tabs: &[Tab], order: &[String]) -> Result<Vec<Tab>, String> {
    let mut seen = HashSet::new();
    let mut next = Vec::with_capacity(order.len());
    for id in order {
        let Some(tab) = tabs.iter().find(|t| &t.id == id) else {
            return Err("that order names a tab the dashboard does not have.".to_owned());
        };
        if !seen.insert(id.as_str()) {
            return Err("that order names a tab twice.".to_owned());
        }
        next.push(tab.clone());
    }
    if next.len() != tabs.len() {
        return Err("that order must name every tab once.".to_owned());
    }
    validate_tabs(&next)?;
    Ok(clean_tabs(&next))
}

/// Move one tab to `position` (1 is first; past the end means last).
///
/// # Errors
///
/// A message for the caller when the tab does not exist or `position` is 0.
pub fn move_tab(tabs: &[Tab], id: &str, position: usize) -> Result<Vec<Tab>, String> {
    if position == 0 {
        return Err("position starts at 1 (the first tab).".to_owned());
    }
    let Some(from) = tabs.iter().position(|t| t.id == id) else {
        return Err("that tab does not exist.".to_owned());
    };
    let mut order: Vec<String> = tabs.iter().map(|t| t.id.clone()).collect();
    let moved = order.remove(from);
    order.insert((position - 1).min(order.len()), moved);
    reorder_tabs(tabs, &order)
}

/// Put charts on `tab`, in the order given, below what is already there. A
/// chart with no tile yet gets a half-width one.
///
/// # Errors
///
/// A message for the caller when `tab` is not one of `tabs`.
pub fn move_to_tab(
    tabs: &[Tab],
    layout: &LayoutMap,
    tab: &str,
    chart_ids: &[String],
) -> Result<LayoutMap, String> {
    if !tabs.iter().any(|t| t.id == tab) {
        return Err("that tab does not exist.".to_owned());
    }
    let moving: HashSet<&str> = chart_ids.iter().map(String::as_str).collect();
    // Where each moved tile was, and on which tab, to close the space it leaves.
    let holes: Vec<((i32, i32, i32, i32), String)> = chart_ids
        .iter()
        .filter_map(|id| {
            let t = layout.get(id)?;
            let from = tab_of(tabs, layout, id)?;
            (from != tab).then(|| ((t.x, t.w, t.y, t.h), from.to_owned()))
        })
        .collect();
    let mut y = bottom_of_tab(tabs, layout, tab, &moving);
    let mut out = layout.clone();
    for id in chart_ids {
        let mut tile = layout.get(id).cloned().unwrap_or(crate::store::TileBox {
            x: 0,
            y: 0,
            w: 6,
            h: 6,
            tab: None,
        });
        tile.x = 0;
        tile.y = y;
        tile.tab = Some(tab.to_owned());
        y += tile.h;
        out.insert(id.clone(), tile);
    }
    // BI-18 part C review fix (SHOULD-FIX) R4: the space a tile leaves on the
    // tab it came from is closed.
    for (rect, from) in &holes {
        let peers: Vec<String> = out
            .keys()
            .filter(|id| !moving.contains(id.as_str()))
            .filter(|id| tab_of(tabs, &out, id) == Some(from.as_str()))
            .cloned()
            .collect();
        close_space(&mut out, *rect, &peers);
    }
    Ok(out)
}

/// Move the `peers` that sit below `rect` (x, w, top, h: the space a tile
/// left) and over any of its columns up to its top, as far as the tiles above
/// them let them (`BI-18` part C review fix (SHOULD-FIX) R4, R9). Used when a
/// tile leaves a tab or is deleted; the general rule that a layout save pulls
/// nothing up stays, so a gap the user made by hand is kept.
pub fn close_space(layout: &mut LayoutMap, rect: (i32, i32, i32, i32), peers: &[String]) {
    let (x, w, top, h) = rect;
    let mut below: Vec<&String> = peers
        .iter()
        .filter(|id| {
            layout
                .get(id.as_str())
                .is_some_and(|t| t.y >= top + h && t.x < x + w && t.x + t.w > x)
        })
        .collect();
    below.sort_by_key(|id| layout[id.as_str()].y);
    for id in below {
        let tile = layout[id.as_str()].clone();
        let floor = peers
            .iter()
            .filter(|other| *other != id)
            .filter_map(|other| layout.get(other.as_str()))
            .filter(|o| o.y < tile.y && o.x < tile.x + tile.w && o.x + o.w > tile.x)
            .map(|o| o.y + o.h)
            .max()
            .unwrap_or(0)
            .max(top);
        if floor < tile.y
            && let Some(slot) = layout.get_mut(id.as_str())
        {
            slot.y = floor;
        }
    }
}

/// The layout without the tile of a deleted chart, the tiles under it moved up
/// to close its space on its tab (`BI-18` part C review fix (SHOULD-FIX) R9).
/// A chart with no tile changes nothing.
#[must_use]
pub fn without_tile(tabs: &[Tab], layout: &LayoutMap, chart_id: &str) -> LayoutMap {
    let Some(gone) = layout.get(chart_id) else {
        return layout.clone();
    };
    let rect = (gone.x, gone.w, gone.y, gone.h);
    let tab = tab_of(tabs, layout, chart_id).map(str::to_owned);
    let mut out = layout.clone();
    out.shift_remove(chart_id);
    let peers: Vec<String> = out
        .keys()
        .filter(|id| tab_of(tabs, &out, id).map(str::to_owned) == tab)
        .cloned()
        .collect();
    close_space(&mut out, rect, &peers);
    out
}

/// What deleting a tab leaves: the tabs, the layout and the filters.
pub type TabsAfterDelete = (Vec<Tab>, LayoutMap, Vec<FilterDef>);

/// Delete a tab. Its tiles go to the first remaining tab, below what is
/// there. A filter tied to it loses that tie, and one left with no tie is tied
/// to the tab its tiles went to, so deleting a tab never makes a filter apply
/// where it did not. Deleting the last tab leaves a board with no tabs and no
/// ties; its tiles stay.
///
/// # Errors
///
/// A message for the caller when the tab does not exist.
pub fn delete_tab(
    tabs: &[Tab],
    layout: &LayoutMap,
    filters: &[FilterDef],
    id: &str,
) -> Result<TabsAfterDelete, String> {
    if !tabs.iter().any(|t| t.id == id) {
        return Err("that tab does not exist.".to_owned());
    }
    let remaining: Vec<Tab> = tabs.iter().filter(|t| t.id != id).cloned().collect();
    let Some(target) = remaining.first().map(|t| t.id.clone()) else {
        let mut layout = layout.clone();
        for tile in layout.values_mut() {
            tile.tab = None;
        }
        let filters = filters
            .iter()
            .cloned()
            .map(|mut f| {
                f.tabs = None;
                f
            })
            .collect();
        return Ok((Vec::new(), layout, filters));
    };
    let own: Vec<String> = layout
        .keys()
        .filter(|k| tab_of(tabs, layout, k) == Some(id))
        .cloned()
        .collect();
    let moved = move_to_tab(&remaining, layout, &target, &own)?;
    // `move_to_tab` judged "which tab is each tile on" against the remaining
    // tabs, where a tile naming the deleted one already counted as on the first;
    // the stamp it wrote is the one that is kept.
    let filters = filters
        .iter()
        .cloned()
        .map(|mut f| {
            if let Some(ties) = f.tabs.as_mut().filter(|t| t.iter().any(|x| x == id)) {
                ties.retain(|x| x != id);
                if ties.is_empty() {
                    ties.push(target.clone());
                }
            }
            f
        })
        .collect();
    Ok((remaining, moved, filters))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::store::TileBox;

    fn tab(id: &str, title: &str) -> Tab {
        Tab {
            id: id.to_owned(),
            title: title.to_owned(),
        }
    }

    fn tile(tab: Option<&str>) -> TileBox {
        TileBox {
            x: 0,
            y: 0,
            w: 6,
            h: 6,
            tab: tab.map(str::to_owned),
        }
    }

    fn tied(tabs: Option<&[&str]>) -> FilterDef {
        let mut f = FilterDef::in_values("province", vec!["Bali".to_owned()]);
        f.tabs = tabs.map(|t| t.iter().map(|s| (*s).to_owned()).collect());
        f
    }

    #[test]
    fn tabs_are_limited_in_number_title_and_id() {
        assert!(validate_tabs(&[]).is_ok());
        assert!(validate_tabs(&[tab("a", "Overview"), tab("b_2", "Maps")]).is_ok());
        let many: Vec<Tab> = (0..=MAX_TABS).map(|i| tab(&format!("t{i}"), "x")).collect();
        assert!(validate_tabs(&many).unwrap_err().contains("at most 20"));
        assert!(
            validate_tabs(&[tab("a", "  ")])
                .unwrap_err()
                .contains("title")
        );
        assert!(
            validate_tabs(&[tab("a", &"x".repeat(61))])
                .unwrap_err()
                .contains("60")
        );
        assert!(validate_tabs(&[tab("a", &"x".repeat(60))]).is_ok());
        assert!(
            validate_tabs(&[tab("a", "x"), tab("a", "y")])
                .unwrap_err()
                .contains("same id")
        );
        assert!(validate_tabs(&[tab("a b", "x")]).is_err());
        assert!(validate_tabs(&[tab("", "x")]).is_err());
        assert!(validate_tabs(&[tab("a", "one\ntwo")]).is_err());
    }

    #[test]
    fn a_tile_names_a_tab_the_board_has_or_none() {
        let tabs = [tab("a", "A"), tab("b", "B")];
        let mut layout = LayoutMap::new();
        layout.insert("c1".to_owned(), tile(Some("b")));
        layout.insert("c2".to_owned(), tile(None));
        assert!(check_layout_tabs(&layout, &tabs).is_ok());
        layout.insert("c3".to_owned(), tile(Some("gone")));
        assert!(check_layout_tabs(&layout, &tabs).is_err());
        // With no tabs at all a tile may not name one.
        assert!(check_layout_tabs(&layout, &[]).is_err());
    }

    #[test]
    fn a_tile_with_no_tab_sits_on_the_first_and_a_board_without_tabs_has_none() {
        let tabs = [tab("a", "A"), tab("b", "B")];
        let mut layout = LayoutMap::new();
        layout.insert("on_b".to_owned(), tile(Some("b")));
        layout.insert("bare".to_owned(), tile(None));
        layout.insert("stale".to_owned(), tile(Some("deleted")));
        assert_eq!(tab_of(&tabs, &layout, "on_b"), Some("b"));
        assert_eq!(tab_of(&tabs, &layout, "bare"), Some("a"));
        assert_eq!(tab_of(&tabs, &layout, "no_tile_yet"), Some("a"));
        assert_eq!(tab_of(&tabs, &layout, "stale"), Some("a"));
        assert_eq!(tab_of(&[], &layout, "on_b"), None);
    }

    #[test]
    fn a_filter_applies_on_every_tab_unless_tied() {
        assert!(tied(None).applies_on(Some("a")));
        assert!(tied(None).applies_on(None));
        assert!(
            tied(Some(&[])).applies_on(Some("a")),
            "empty ties are no ties"
        );
        let one = tied(Some(&["a"]));
        assert!(one.applies_on(Some("a")));
        assert!(!one.applies_on(Some("b")));
        // A board with no tabs ignores ties rather than dropping the filter.
        assert!(one.applies_on(None));
    }

    #[test]
    fn filters_for_a_tab_keep_the_untied_and_the_tied_to_it() {
        let filters = [tied(None), tied(Some(&["a"])), tied(Some(&["b"]))];
        assert_eq!(filters_for_tab(&filters, Some("a")).len(), 2);
        assert_eq!(filters_for_tab(&filters, Some("b")).len(), 2);
        assert_eq!(filters_for_tab(&filters, Some("c")).len(), 1);
        assert_eq!(filters_for_tab(&filters, None).len(), 3);
    }

    #[test]
    fn a_tile_is_given_the_filters_of_its_own_tab_and_untied_boards_copy_nothing() {
        let tabs = [tab("a", "A"), tab("b", "B")];
        let mut layout = LayoutMap::new();
        layout.insert("on_b".to_owned(), tile(Some("b")));
        let filters = [tied(None), tied(Some(&["a"]))];
        let on_b = filters_for_chart(&filters, &tabs, &layout, "on_b");
        assert_eq!(on_b.len(), 1);
        // No tile yet: the first tab.
        assert_eq!(filters_for_chart(&filters, &tabs, &layout, "new").len(), 2);
        let untied = [tied(None)];
        assert!(matches!(
            filters_for_chart(&untied, &tabs, &layout, "on_b"),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            filters_for_chart(&filters, &[], &layout, "on_b"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn a_tie_to_a_missing_tab_is_refused_with_our_sentence() {
        let tabs = [tab("a", "A")];
        assert!(check_filter_ties(&[tied(Some(&["a"]))], &tabs).is_ok());
        let err = check_filter_ties(&[tied(Some(&["x"]))], &tabs).unwrap_err();
        assert!(err.contains("tab that does not exist"), "{err}");
    }

    #[test]
    fn a_phone_layout_is_an_ordered_list_of_heights() {
        let t = |id: &str, h: i32| PhoneTile {
            id: id.to_owned(),
            h,
        };
        assert!(validate_phone_layout(&[t("a", 4), t("b", 60)]).is_ok());
        assert!(validate_phone_layout(&[t("a", 0)]).is_err());
        assert!(validate_phone_layout(&[t("a", 61)]).is_err());
        assert!(validate_phone_layout(&[t("a", 4), t("a", 5)]).is_err());
        assert!(validate_phone_layout(&[t("", 4)]).is_err());
        let many: Vec<PhoneTile> = (0..=MAX_PHONE_TILES)
            .map(|i| t(&format!("c{i}"), 4))
            .collect();
        assert!(validate_phone_layout(&many).is_err());
    }

    fn layout_of(tiles: &[(&str, Option<&str>, i32, i32)]) -> LayoutMap {
        let mut l = LayoutMap::new();
        for (id, tab, y, h) in tiles {
            l.insert(
                (*id).to_owned(),
                TileBox {
                    x: 0,
                    y: *y,
                    w: 6,
                    h: *h,
                    tab: tab.map(str::to_owned),
                },
            );
        }
        l
    }

    #[test]
    fn a_tab_is_found_by_id_or_by_an_unambiguous_title() {
        let tabs = [
            tab("t_a", "Overview"),
            tab("t_b", "Maps"),
            tab("t_c", "maps"),
        ];
        assert_eq!(find_tab(&tabs, "t_a").unwrap().title, "Overview");
        assert_eq!(find_tab(&tabs, " overview ").unwrap().id, "t_a");
        assert!(
            find_tab(&tabs, "Maps").is_none(),
            "two tabs share the title"
        );
        assert!(find_tab(&tabs, "nope").is_none());
    }

    #[test]
    fn the_first_tab_takes_every_tile_and_later_ones_start_empty() {
        let layout = layout_of(&[("a", None, 0, 4), ("b", None, 4, 4)]);
        let (tabs, first) = add_tab(&[], &layout, " Overview ", "t_a").unwrap();
        assert_eq!(tabs[0].title, "Overview");
        assert!(first.values().all(|t| t.tab.as_deref() == Some("t_a")));
        let (tabs, second) = add_tab(&tabs, &first, "Maps", "t_b").unwrap();
        assert_eq!(tabs.len(), 2);
        assert_eq!(second, first);
        assert!(add_tab(&tabs, &second, " ", "t_c").is_err());
        assert!(add_tab(&tabs, &second, "x", "t_a").is_err(), "id reused");
    }

    #[test]
    fn charts_move_to_a_tab_below_what_is_there_in_the_order_given() {
        let tabs = [tab("t_a", "A"), tab("t_b", "B")];
        let layout = layout_of(&[
            ("a", Some("t_a"), 0, 4),
            ("b", Some("t_b"), 0, 3),
            ("c", Some("t_a"), 4, 4),
        ]);
        let moved =
            move_to_tab(&tabs, &layout, "t_b", &["c".to_owned(), "new".to_owned()]).unwrap();
        assert_eq!((moved["c"].tab.as_deref(), moved["c"].y), (Some("t_b"), 3));
        // A chart with no tile yet gets a half-width tile under the one before.
        assert_eq!(
            (moved["new"].tab.as_deref(), moved["new"].y, moved["new"].w),
            (Some("t_b"), 7, 6)
        );
        assert_eq!(moved["a"], layout["a"]);
        assert!(move_to_tab(&tabs, &layout, "gone", &["a".to_owned()]).is_err());
    }

    #[test]
    fn deleting_a_tab_moves_its_tiles_and_re_ties_the_filters_that_named_it() {
        let tabs = [tab("t_a", "A"), tab("t_b", "B")];
        let layout = layout_of(&[
            ("a", Some("t_a"), 0, 5),
            ("b", Some("t_b"), 0, 3),
            ("c", Some("t_b"), 3, 4),
        ]);
        let filters = [
            tied(Some(&["t_b"])),
            tied(Some(&["t_a", "t_b"])),
            tied(None),
        ];
        let (left, moved, filters) = delete_tab(&tabs, &layout, &filters, "t_b").unwrap();
        assert_eq!(left, [tab("t_a", "A")]);
        assert_eq!((moved["b"].tab.as_deref(), moved["b"].y), (Some("t_a"), 5));
        assert_eq!((moved["c"].tab.as_deref(), moved["c"].y), (Some("t_a"), 8));
        let ties: Vec<Option<Vec<String>>> = filters.iter().map(|f| f.tabs.clone()).collect();
        assert_eq!(
            ties,
            [
                Some(vec!["t_a".to_owned()]),
                Some(vec!["t_a".to_owned()]),
                None
            ]
        );
        // The result is one the save rules accept.
        assert!(check_layout_tabs(&moved, &left).is_ok());
        assert!(check_filter_ties(&filters, &left).is_ok());
        assert!(delete_tab(&tabs, &layout, &[], "gone").is_err());
    }

    #[test]
    fn deleting_the_last_tab_leaves_no_tabs_no_tile_tabs_and_no_ties() {
        let tabs = [tab("t_a", "A")];
        let layout = layout_of(&[("a", Some("t_a"), 0, 5)]);
        let (left, moved, filters) =
            delete_tab(&tabs, &layout, &[tied(Some(&["t_a"]))], "t_a").unwrap();
        assert!(left.is_empty(), "{left:?}");
        assert_eq!(moved["a"].tab, None);
        assert_eq!(filters[0].tabs, None);
    }

    #[test]
    fn tabs_are_reordered_by_a_full_list_or_moved_to_a_position() {
        let tabs = vec![tab("a", "A"), tab("b", "B"), tab("c", "C")];
        let ids = |t: &[Tab]| t.iter().map(|x| x.id.as_str()).collect::<String>();
        let order = |s: &[&str]| s.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            ids(&reorder_tabs(&tabs, &order(&["c", "a", "b"])).unwrap()),
            "cab"
        );
        // Titles travel with their ids.
        assert_eq!(
            reorder_tabs(&tabs, &order(&["b", "a", "c"])).unwrap()[0].title,
            "B"
        );
        assert!(reorder_tabs(&tabs, &order(&["a", "b"])).is_err());
        assert!(reorder_tabs(&tabs, &order(&["a", "a", "b"])).is_err());
        assert!(reorder_tabs(&tabs, &order(&["a", "b", "zz"])).is_err());
        assert_eq!(ids(&move_tab(&tabs, "c", 1).unwrap()), "cab");
        assert_eq!(ids(&move_tab(&tabs, "a", 2).unwrap()), "bac");
        assert_eq!(ids(&move_tab(&tabs, "a", 99).unwrap()), "bca");
        assert!(move_tab(&tabs, "a", 0).is_err());
        assert!(move_tab(&tabs, "zz", 1).is_err());
    }

    #[test]
    fn renaming_a_tab_keeps_its_id_and_checks_the_title() {
        let tabs = [tab("t_a", "A"), tab("t_b", "B")];
        let next = rename_tab(&tabs, "t_b", "  Maps ").unwrap();
        assert_eq!(next[1], tab("t_b", "Maps"));
        assert!(rename_tab(&tabs, "t_b", "").is_err());
        assert!(rename_tab(&tabs, "gone", "x").is_err());
    }

    fn tiles(names: &[(&str, Option<&str>)]) -> LayoutMap {
        names
            .iter()
            .map(|(id, t)| ((*id).to_owned(), tile(*t)))
            .collect()
    }

    /// BI-18 part C review fix (BLOCKER) R1: the three-tile board of the QA.
    /// Tabs A and B are added, one tile moves to B, then B moves left: A keeps
    /// its two tiles and B its one.
    #[test]
    fn reordering_tabs_leaves_each_tile_on_the_tab_it_showed_on() {
        let (a, b) = (tab("t_a", "A"), tab("t_b", "B"));
        let mut layout = tiles(&[("c1", None), ("c2", None), ("c3", None)]);
        // The first tab is added: every tile is stamped with it.
        assert!(stamp_untagged(&mut layout, &[], std::slice::from_ref(&a)));
        let two = vec![a, b];
        let layout = move_to_tab(&two, &layout, "t_b", &["c3".to_owned()]).unwrap();
        // Legacy: a stored board whose tiles name nothing, then B moves left.
        let mut legacy = tiles(&[("c1", None), ("c2", None), ("c3", Some("t_b"))]);
        let reordered = reorder_tabs(&two, &["t_b".to_owned(), "t_a".to_owned()]).unwrap();
        assert!(stamp_untagged(&mut legacy, &two, &reordered));
        for (l, tabs) in [(&layout, &reordered), (&legacy, &reordered)] {
            assert_eq!(tab_of(tabs, l, "c1"), Some("t_a"));
            assert_eq!(tab_of(tabs, l, "c2"), Some("t_a"));
            assert_eq!(tab_of(tabs, l, "c3"), Some("t_b"));
            // A filter tied to B is not given to A's tiles after the reorder.
            let tied_to_b = tied(Some(&["t_b"]));
            let filters = [tied_to_b];
            assert!(filters_for_chart(&filters, tabs, l, "c1").is_empty());
            assert!(filters_for_chart(&filters, tabs, l, "c2").is_empty());
            assert_eq!(filters_for_chart(&filters, tabs, l, "c3").len(), 1);
        }
    }

    #[test]
    fn the_first_tab_is_the_one_before_the_write_and_a_deleted_one_falls_back() {
        let (a, b) = (tab("t_a", "A"), tab("t_b", "B"));
        let mut layout = tiles(&[("c1", None), ("c2", Some("t_b"))]);
        assert!(stamp_untagged(
            &mut layout,
            &[a.clone(), b.clone()],
            &[b.clone(), a.clone()]
        ));
        assert_eq!(layout["c1"].tab.as_deref(), Some("t_a"));
        let mut gone = tiles(&[("c1", None)]);
        assert!(stamp_untagged(&mut gone, &[a], &[b]));
        assert_eq!(gone["c1"].tab.as_deref(), Some("t_b"));
        let mut stamped = tiles(&[("c1", Some("t_b"))]);
        assert!(!stamp_untagged(&mut stamped, &[], &[tab("t_b", "B")]));
    }

    #[test]
    fn a_board_with_no_tabs_has_tiles_that_name_none() {
        let mut layout = tiles(&[("c1", Some("t_a")), ("c2", None)]);
        assert!(stamp_untagged(&mut layout, &[tab("t_a", "A")], &[]));
        assert!(layout.values().all(|t| t.tab.is_none()));
        assert!(!stamp_untagged(&mut layout, &[], &[]));
    }

    fn at(x: i32, y: i32, w: i32, h: i32, tab: &str) -> TileBox {
        TileBox {
            x,
            y,
            w,
            h,
            tab: Some(tab.to_owned()),
        }
    }

    /// BI-18 part C review fix (SHOULD-FIX) R4: the choropleth at the top of
    /// Overview moves to Maps; what was below it moves up into its space, a tile
    /// beside it that did not sit below it stays, and nothing overlaps.
    #[test]
    fn moving_a_tile_to_another_tab_closes_the_space_it_leaves() {
        let tabs = vec![tab("t_a", "Overview"), tab("t_b", "Maps")];
        let layout: LayoutMap = [
            ("map", at(0, 0, 12, 6, "t_a")),
            ("kpi", at(0, 6, 6, 4, "t_a")),
            ("bar", at(6, 6, 6, 4, "t_a")),
            ("low", at(0, 10, 6, 4, "t_a")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let out = move_to_tab(&tabs, &layout, "t_b", &["map".to_owned()]).unwrap();
        assert_eq!((out["map"].tab.as_deref(), out["map"].y), (Some("t_b"), 0));
        assert_eq!(out["kpi"].y, 0);
        assert_eq!(out["bar"].y, 0);
        assert_eq!(out["low"].y, 4, "stays under the tile that moved up");
    }

    #[test]
    fn a_gap_the_user_made_and_a_move_within_a_tab_are_left_alone() {
        let tabs = vec![tab("t_a", "A"), tab("t_b", "B")];
        let layout: LayoutMap = [
            ("one", at(0, 0, 6, 4, "t_a")),
            ("gap", at(0, 20, 6, 4, "t_a")),
            ("two", at(0, 0, 6, 4, "t_b")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        // Moving `two` onto B again leaves no hole; `gap` on A is untouched.
        let out = move_to_tab(&tabs, &layout, "t_b", &["two".to_owned()]).unwrap();
        assert_eq!(out["gap"].y, 20);
        // A tile that was not under the one that left does not move.
        let out = move_to_tab(&tabs, &layout, "t_b", &["one".to_owned()]).unwrap();
        assert_eq!(out["gap"].y, 0, "it was under it, so it closes up");
        let beside: LayoutMap = [("a", at(0, 0, 6, 4, "t_a")), ("b", at(6, 8, 6, 4, "t_a"))]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect();
        let out = move_to_tab(&tabs, &beside, "t_b", &["a".to_owned()]).unwrap();
        assert_eq!(out["b"].y, 8);
    }

    /// BI-18 part C review fix (SHOULD-FIX) R9: deleting a chart removes its tile
    /// and closes the space on its tab, like a move to another tab does.
    #[test]
    fn deleting_a_chart_drops_its_tile_and_closes_the_space_on_its_tab() {
        let tabs = vec![tab("t_a", "A"), tab("t_b", "B")];
        let layout: LayoutMap = [
            ("map", at(0, 0, 12, 6, "t_a")),
            ("kpi", at(0, 6, 6, 4, "t_a")),
            ("other", at(0, 6, 6, 4, "t_b")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let out = without_tile(&tabs, &layout, "map");
        assert!(!out.contains_key("map"));
        assert_eq!(out["kpi"].y, 0);
        assert_eq!(out["other"].y, 6, "another tab is not touched");
        // No tabs: every tile is a peer.
        let plain: LayoutMap = [("a", tile(None)), ("b", TileBox { y: 6, ..tile(None) })]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect();
        assert_eq!(without_tile(&[], &plain, "a")["b"].y, 0);
        assert_eq!(without_tile(&[], &plain, "zz"), plain);
    }

    /// BI-18 part C review fix (BLOCKER) R13: the cases the console's
    /// `placeUnboxed` is tested on too (`src/lib/dashboard-tabs.test.ts`).
    #[test]
    fn charts_with_no_box_get_the_place_the_grid_gave_them_on_the_shared_fixture() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../../ops/fixtures/place_unboxed.json"))
                .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let tabs: Vec<Tab> = case["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| tab(t.as_str().unwrap(), "T"))
                .collect();
            let mut layout: LayoutMap = serde_json::from_value(case["layout"].clone()).unwrap();
            let charts: Vec<LiveChart> = case["charts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| LiveChart {
                    id: c["id"].as_str().unwrap().to_owned(),
                    span: c["span"].as_u64().map(|n| u8::try_from(n).unwrap()),
                    kind: serde_json::from_value(c["kind"].clone()).unwrap(),
                })
                .collect();
            // The order a board write does it in: stamp, then place.
            stamp_untagged(&mut layout, &tabs, &tabs);
            let stored = layout.clone();
            assert!(place_unboxed(&mut layout, &tabs, &tabs, &charts), "{name}");
            let placed: LayoutMap = serde_json::from_value(case["placed"].clone()).unwrap();
            for (id, want) in &placed {
                let mut want = want.clone();
                if let Some(first) = tabs.first() {
                    want.tab = Some(first.id.clone());
                }
                assert_eq!(layout.get(id), Some(&want), "{name}: {id}");
            }
            for (id, box_) in &stored {
                assert_eq!(layout.get(id), Some(box_), "{name}: stored {id} moved");
            }
            assert_eq!(layout.len(), stored.len() + placed.len(), "{name}");
            // Idempotent: a second write adds nothing.
            assert!(!place_unboxed(&mut layout, &tabs, &tabs, &charts), "{name}");
        }
    }

    /// R13 with R1: a reorder of the tabs that is also the write that places
    /// the charts leaves an unboxed chart on the tab it showed on.
    #[test]
    fn an_unboxed_chart_stays_on_the_tab_it_showed_on_when_the_tabs_are_reordered() {
        let (a, b) = (tab("t_a", "A"), tab("t_b", "B"));
        let before = vec![a.clone(), b.clone()];
        let after = vec![b, a];
        let mut layout = layout_of(&[("c1", None, 0, 6)]);
        stamp_untagged(&mut layout, &before, &after);
        let charts: Vec<LiveChart> = ["c1", "c2"]
            .iter()
            .map(|id| LiveChart {
                id: (*id).to_owned(),
                span: Some(1),
                kind: ChartKind::Bar,
            })
            .collect();
        assert!(place_unboxed(&mut layout, &before, &after, &charts));
        assert_eq!(tab_of(&after, &layout, "c1"), Some("t_a"));
        assert_eq!(tab_of(&after, &layout, "c2"), Some("t_a"));
        assert_eq!(layout["c2"].y, 6);
    }

    /// R14 (closed by R13): once the charts under a deleted one have boxes,
    /// the whole column follows it up.
    #[test]
    fn deleting_a_chart_closes_the_space_for_charts_that_had_no_box_before_the_write() {
        let tabs = vec![tab("t_a", "A")];
        let mut layout = layout_of(&[("top", Some("t_a"), 0, 6)]);
        let charts: Vec<LiveChart> = ["top", "mid", "low"]
            .iter()
            .map(|id| LiveChart {
                id: (*id).to_owned(),
                span: Some(2),
                kind: ChartKind::Bar,
            })
            .collect();
        assert!(place_unboxed(&mut layout, &tabs, &tabs, &charts));
        assert_eq!((layout["mid"].y, layout["low"].y), (6, 12));
        let out = without_tile(&tabs, &layout, "top");
        assert_eq!((out["mid"].y, out["low"].y), (0, 6));
    }
}
