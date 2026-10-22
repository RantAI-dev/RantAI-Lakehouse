//! Dashboard folders — a nested tree that dashboards (boards) and SQL
//! sources live in. Plan: `docs/plans/DASHBOARD-SQL-SOURCES-FOLDERS-PLAN.md`
//! §4, stage 1: folders only; no per-folder permissions, no personal
//! folders (boards have no owner column yet), no revision history.
//!
//! # The personal space (`BI-18` part C, part 2)
//!
//! A folder with an `owner_id` is one user's personal space: created on first
//! use ([`ensure_personal_folder`], with an id derived from the owner so two
//! first uses are one folder), at the root, holding dashboards only (no
//! subfolders, no rename, no delete). [`crate::access::Viewer`] decides who
//! sees it: its owner, and a principal with `identity:write`.
//!
//! Stored in `console.bi_folder` with the same versioned-insert / tombstone
//! semantics as `bi_board` (see [`crate::store::ensure_bi_table`]). A board
//! or source points at its folder with `folder_id`; `""` is the root.
//!
//! The tree rules live here as pure functions ([`check_placement`]) so they
//! are unit-tested without `ClickHouse`, and the API applies them before
//! every create/rename/move:
//!
//! - at most [`MAX_FOLDER_DEPTH`] levels — Grafana's nested-folder limit,
//!   the shallowest of the tools compared in the plan's research;
//! - no cycles (a folder cannot move under itself or a descendant);
//! - names unique among siblings, compared case-insensitively.

use std::collections::HashSet;

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::SqlLiteral;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::access::Viewer;
use crate::store::{ensure_bi_table, random_hex};

/// Deepest a folder may sit: a folder at the root is depth 1.
pub const MAX_FOLDER_DEPTH: usize = 4;

/// Longest folder name accepted.
pub const MAX_FOLDER_NAME_CHARS: usize = 100;

/// A dashboard folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    /// `f_<8 hex>`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Parent folder id; `""` = root.
    pub parent_id: String,
    /// Principal id of whoever last saved it.
    pub created_by: String,
    /// When this version was saved (`ClickHouse`-formatted).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub updated_at: Option<String>,
    /// The principal id whose personal space this is; `""` for an ordinary
    /// folder, which is left off the wire.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub owner_id: String,
}

/// Why a folder cannot be created, renamed or moved as asked. Every message
/// is fixed text, safe to return to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderRule {
    /// Blank name.
    NameRequired,
    /// Over [`MAX_FOLDER_NAME_CHARS`].
    NameTooLong,
    /// A sibling already has this name.
    DuplicateName,
    /// The parent folder does not exist.
    ParentNotFound,
    /// Moving a folder under itself or one of its descendants.
    Cycle,
    /// The result would be deeper than [`MAX_FOLDER_DEPTH`].
    TooDeep,
}

impl FolderRule {
    /// The caller-facing message.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::NameRequired => "folder name is required.",
            Self::NameTooLong => "folder name is too long (max 100 characters).",
            Self::DuplicateName => "a folder with this name already exists here.",
            Self::ParentNotFound => "parent folder not found.",
            Self::Cycle => "a folder cannot be moved into itself or one of its subfolders.",
            Self::TooDeep => "folders can be nested at most 4 levels deep.",
        }
    }
}

fn find<'a>(folders: &'a [Folder], id: &str) -> Option<&'a Folder> {
    folders.iter().find(|f| f.id == id)
}

/// Depth of folder `id`: 1 at the root, `0` for an unknown id. A corrupt
/// cycle already in storage stops the walk instead of looping.
#[must_use]
pub fn depth(folders: &[Folder], id: &str) -> usize {
    let mut seen = HashSet::new();
    let mut current = find(folders, id);
    let mut d = 0;
    while let Some(f) = current {
        if !seen.insert(f.id.as_str()) {
            break;
        }
        d += 1;
        current = if f.parent_id.is_empty() {
            None
        } else {
            find(folders, &f.parent_id)
        };
    }
    d
}

/// Levels in the subtree rooted at `id`, counting `id` itself (a leaf is 1).
#[must_use]
pub fn subtree_height(folders: &[Folder], id: &str) -> usize {
    fn walk<'a>(folders: &'a [Folder], id: &'a str, seen: &mut HashSet<&'a str>) -> usize {
        if !seen.insert(id) {
            return 0;
        }
        1 + folders
            .iter()
            .filter(|f| f.parent_id == id)
            .map(|f| walk(folders, &f.id, seen))
            .max()
            .unwrap_or(0)
    }
    walk(folders, id, &mut HashSet::new())
}

/// Whether `candidate` is `ancestor` or lies anywhere below it.
#[must_use]
pub fn is_self_or_descendant(folders: &[Folder], ancestor: &str, candidate: &str) -> bool {
    let mut seen = HashSet::new();
    let mut current = Some(candidate);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        if !seen.insert(id) {
            return false;
        }
        current = find(folders, id)
            .map(|f| f.parent_id.as_str())
            .filter(|p| !p.is_empty());
    }
    false
}

/// Check that folder `moving` (`None` for a new folder) may be named `name`
/// and sit under `parent` (`""` = root), returning the trimmed name.
///
/// # Errors
///
/// The first [`FolderRule`] the placement breaks.
pub fn check_placement(
    folders: &[Folder],
    moving: Option<&str>,
    name: &str,
    parent: &str,
) -> Result<String, FolderRule> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FolderRule::NameRequired);
    }
    if name.chars().count() > MAX_FOLDER_NAME_CHARS {
        return Err(FolderRule::NameTooLong);
    }
    let parent_depth = if parent.is_empty() {
        0
    } else {
        if find(folders, parent).is_none() {
            return Err(FolderRule::ParentNotFound);
        }
        depth(folders, parent)
    };
    if let Some(id) = moving
        && !parent.is_empty()
        && is_self_or_descendant(folders, id, parent)
    {
        return Err(FolderRule::Cycle);
    }
    let height = moving.map_or(1, |id| subtree_height(folders, id).max(1));
    if parent_depth + height > MAX_FOLDER_DEPTH {
        return Err(FolderRule::TooDeep);
    }
    let lower = name.to_lowercase();
    let clash = folders.iter().any(|f| {
        f.parent_id == parent && Some(f.id.as_str()) != moving && f.name.to_lowercase() == lower
    });
    if clash {
        return Err(FolderRule::DuplicateName);
    }
    Ok(name.to_owned())
}

/// A fresh folder id.
#[must_use]
pub fn new_folder_id() -> String {
    format!("f_{}", random_hex(4))
}

const FOLDER_COLS: &str =
    "id, name, parent_id, created_by, owner_id, toString(created_at) AS updated_at";

fn row_str<'a>(row: &'a serde_json::Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

fn row_to_folder(r: &serde_json::Map<String, Value>) -> Folder {
    Folder {
        id: row_str(r, "id").to_owned(),
        name: row_str(r, "name").to_owned(),
        parent_id: row_str(r, "parent_id").to_owned(),
        created_by: row_str(r, "created_by").to_owned(),
        updated_at: Some(row_str(r, "updated_at").to_owned()),
        owner_id: row_str(r, "owner_id").to_owned(),
    }
}

/// Every live folder `viewer` may see, by name: the ordinary ones, and the
/// personal spaces the viewer owns (all of them for a viewer who sees all).
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn list_folders(ch: &ChClient, viewer: &Viewer) -> Result<Vec<Folder>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {FOLDER_COLS} FROM console.bi_folder FINAL WHERE is_deleted = 0{} ORDER BY name",
                viewer.owner_clause()
            ),
            None,
        )
        .await?;
    Ok(rows.iter().map(row_to_folder).collect())
}

/// The owner of folder `id`: `""` for the root, an ordinary folder or an
/// unknown id (the caller checks existence separately).
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn folder_owner(ch: &ChClient, id: &str) -> Result<String, ChError> {
    if id.is_empty() {
        return Ok(String::new());
    }
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {FOLDER_COLS} FROM console.bi_folder FINAL WHERE is_deleted = 0 AND id = {} LIMIT 1",
                SqlLiteral::from(id)
            ),
            None,
        )
        .await?;
    Ok(rows
        .first()
        .map(|r| row_str(r, "owner_id").to_owned())
        .unwrap_or_default())
}

/// The id of the personal space of `owner`: derived from the principal id, so
/// two first uses at once write the same row and `ReplacingMergeTree` keeps
/// one.
#[must_use]
pub fn personal_folder_id(owner: &str) -> String {
    let compact: String = owner
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(32)
        .collect();
    format!("fp_{compact}")
}

/// The personal space of `owner` (a principal id, never empty), creating it
/// on first use. `display_name` only labels it for an administrator who sees
/// everyone's.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn ensure_personal_folder(
    ch: &ChClient,
    owner: &str,
    display_name: &str,
) -> Result<Folder, ChError> {
    let id = personal_folder_id(owner);
    let mine = list_folders(ch, &Viewer::user(owner, false)).await?;
    if let Some(found) = mine.into_iter().find(|f| f.id == id) {
        return Ok(found);
    }
    let name: String = format!("Personal space of {}", display_name.trim())
        .chars()
        .take(MAX_FOLDER_NAME_CHARS)
        .collect();
    let folder = Folder {
        id,
        name,
        parent_id: String::new(),
        created_by: owner.to_owned(),
        updated_at: None,
        owner_id: owner.to_owned(),
    };
    save_folder(ch, &folder).await?;
    Ok(folder)
}

/// Save (create, rename or move) a folder. The caller has already applied
/// [`check_placement`].
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn save_folder(ch: &ChClient, folder: &Folder) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let sql = format!(
        "INSERT INTO console.bi_folder (id, name, parent_id, created_by, owner_id) VALUES ({}, {}, {}, {}, {})",
        SqlLiteral::from(folder.id.as_str()),
        SqlLiteral::from(folder.name.as_str()),
        SqlLiteral::from(folder.parent_id.as_str()),
        SqlLiteral::from(folder.created_by.as_str()),
        SqlLiteral::from(folder.owner_id.as_str()),
    );
    ch.exec(&sql, None).await
}

/// Tombstone a folder. The caller has already checked it is empty.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn delete_folder(ch: &ChClient, id: &str) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let sql = format!(
        "INSERT INTO console.bi_folder (id, name, is_deleted) VALUES ({}, '', 1)",
        SqlLiteral::from(id)
    );
    ch.exec(&sql, None).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn f(id: &str, name: &str, parent: &str) -> Folder {
        Folder {
            id: id.to_owned(),
            name: name.to_owned(),
            parent_id: parent.to_owned(),
            created_by: String::new(),
            updated_at: None,
            owner_id: String::new(),
        }
    }

    /// a (root) › b › c, plus d at the root.
    fn tree() -> Vec<Folder> {
        vec![
            f("a", "Sales", ""),
            f("b", "Q1", "a"),
            f("c", "Jan", "b"),
            f("d", "Ops", ""),
        ]
    }

    #[test]
    fn depth_counts_from_one_at_the_root() {
        let t = tree();
        assert_eq!(depth(&t, "a"), 1);
        assert_eq!(depth(&t, "c"), 3);
        assert_eq!(depth(&t, "missing"), 0);
    }

    #[test]
    fn a_new_folder_is_placed_with_its_name_trimmed() {
        assert_eq!(
            check_placement(&tree(), None, "  Feb  ", "b"),
            Ok("Feb".to_owned())
        );
    }

    #[test]
    fn a_fifth_level_is_refused() {
        let t = tree();
        assert!(check_placement(&t, None, "Week 1", "c").is_ok());
        let mut deeper = t;
        deeper.push(f("e", "Week 1", "c"));
        assert_eq!(
            check_placement(&deeper, None, "Day 1", "e"),
            Err(FolderRule::TooDeep)
        );
    }

    #[test]
    fn moving_a_subtree_counts_its_own_height() {
        // a›b›c is 3 levels; under d (depth 1) it would reach 4 — allowed.
        assert!(check_placement(&tree(), Some("a"), "Sales", "d").is_ok());
        // One more level under c makes the subtree 4 high: 1 + 4 > 4.
        let mut t = tree();
        t.push(f("e", "Week 1", "c"));
        assert_eq!(
            check_placement(&t, Some("a"), "Sales", "d"),
            Err(FolderRule::TooDeep)
        );
    }

    #[test]
    fn moving_a_folder_under_itself_or_a_descendant_is_a_cycle() {
        let t = tree();
        assert_eq!(
            check_placement(&t, Some("a"), "Sales", "a"),
            Err(FolderRule::Cycle)
        );
        assert_eq!(
            check_placement(&t, Some("a"), "Sales", "c"),
            Err(FolderRule::Cycle)
        );
    }

    #[test]
    fn sibling_names_are_unique_case_insensitively_but_renaming_in_place_is_fine() {
        let t = tree();
        assert_eq!(
            check_placement(&t, None, "sales", ""),
            Err(FolderRule::DuplicateName)
        );
        assert!(
            check_placement(&t, None, "Sales", "d").is_ok(),
            "other parent"
        );
        assert!(
            check_placement(&t, Some("a"), "SALES", "").is_ok(),
            "same folder"
        );
    }

    #[test]
    fn blank_long_and_orphaned_placements_are_refused() {
        let t = tree();
        assert_eq!(
            check_placement(&t, None, "   ", ""),
            Err(FolderRule::NameRequired)
        );
        assert_eq!(
            check_placement(&t, None, &"x".repeat(101), ""),
            Err(FolderRule::NameTooLong)
        );
        assert_eq!(
            check_placement(&t, None, "X", "nope"),
            Err(FolderRule::ParentNotFound)
        );
    }

    #[test]
    fn a_corrupt_stored_cycle_does_not_hang_the_walks() {
        let t = vec![f("x", "X", "y"), f("y", "Y", "x")];
        assert_eq!(depth(&t, "x"), 2);
        assert!(subtree_height(&t, "x") >= 1);
        assert!(!is_self_or_descendant(&t, "z", "x"));
    }
}
