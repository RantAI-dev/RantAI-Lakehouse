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
    Ok(out)
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
        assert!(left.is_empty());
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
}
