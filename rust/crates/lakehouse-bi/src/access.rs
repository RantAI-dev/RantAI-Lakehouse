//! Who may see a board, a chart or a folder: the personal space and the trash
//! (`BI-18` part C, part 2; plan
//! `docs/superpowers/plans/2026-10-14-bi-18c-layout.md`).
//!
//! # Why one type decides
//!
//! A board in the personal space carries its owner's principal id in
//! `owner_id` (`''` for every other board). Whether a reader may see such a
//! board is decided here and nowhere else, and every store read takes a
//! [`Viewer`], so a new read path cannot compile without saying who is reading.
//! The rule:
//!
//! - a board with no owner is visible to everyone who may read dashboards;
//! - a board with an owner is visible to that owner and to a viewer built with
//!   `all = true` (a principal with `identity:write`, or the server itself in
//!   a sweep, a snapshot or a cascade);
//! - nobody else sees it: not in a list, not by id, not through a chart on it,
//!   not through a public link or a signed embed ([`Viewer::anonymous`] has no
//!   id and sees no owned board).
//!
//! A chart has no owner of its own: it is visible when its board is. The
//! statement for that is [`Viewer::chart_clause`], one sub-select on the boards
//! table, so no second round trip and no list to filter in memory.
//!
//! # Permissions on top (`BI-19` part B)
//!
//! A person may also be given a level on one dashboard or folder, and an item
//! can be marked "only the people listed". [`AccessMap`] is the one pure
//! function of the folder tree, the rules, the marks and the person that says
//! which [`Level`] they hold on an item; it is built once per request and
//! turned into the [`Viewer`] every store read already takes, so a read path
//! cannot skip it. The resolved set rides on the viewer as either "everything
//! except these ids" (somebody whose role gives the starting point) or "only
//! these ids" (somebody with no starting point, who sees what they were
//! given), and the statement fragments add `AND id NOT IN (...)` or
//! `AND id IN (...)`. A list over [`MAX_SCOPE_IDS`] ids is an error, never a
//! truncated list: a truncated "only" list would hide, but a truncated
//! "except" list would show.
//!
//! # The trash
//!
//! A deleted board or chart keeps its row with `is_deleted = 2`; every read
//! that asks for `is_deleted = 0` therefore skips it without knowing the trash
//! exists. [`Viewer::chart_clause`] also hides a live chart whose board is in
//! the trash, so a chart restored on its own while its board is still deleted
//! comes back with the board, not before it.

use std::collections::{HashMap, HashSet};

use lakehouse_core::ident::SqlLiteral;

use crate::folders::{Folder, MAX_FOLDER_DEPTH};

/// The most ids a [`Viewer`]'s list may hold. Beyond it the request is
/// refused (see [`AccessMap::viewer`]); the figure is a plan decision (D7) and
/// is raised only after counting a real deployment's boards.
pub const MAX_SCOPE_IDS: usize = 5_000;

/// What a person may do with a dashboard or folder. Ordered: a higher level
/// includes the lower ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Open it.
    View,
    /// Change its charts, filters and layout.
    Edit,
    /// Edit, plus share it and delete it.
    Manage,
}

impl Level {
    /// The word stored and sent on the wire.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Edit => "edit",
            Self::Manage => "manage",
        }
    }

    /// The level a stored word names; anything else is `None`, so an
    /// unknown word grants nothing.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "view" => Some(Self::View),
            "edit" => Some(Self::Edit),
            "manage" => Some(Self::Manage),
            _ => None,
        }
    }
}

/// What a rule or a mark is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// A dashboard.
    Board,
    /// A folder.
    Folder,
}

impl ItemKind {
    /// The word stored.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Board => "board",
            Self::Folder => "folder",
        }
    }

    /// The kind a stored word names, `None` for anything else.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "board" => Some(Self::Board),
            "folder" => Some(Self::Folder),
            _ => None,
        }
    }
}

/// A rule that applies to the person the [`AccessMap`] is built for (the
/// caller has already kept the rules naming them or one of their roles).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// Board or folder.
    pub kind: ItemKind,
    /// The item's id.
    pub item_id: String,
    /// The level it gives.
    pub level: Level,
}

/// Where a dashboard sits, for [`AccessMap::viewer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardPlacement {
    /// The dashboard's id.
    pub id: String,
    /// Its folder, `""` for the root.
    pub folder_id: String,
    /// The owner of a personal-space dashboard, `""` otherwise.
    pub owner_id: String,
}

/// More ids than [`MAX_SCOPE_IDS`] would have to ride on the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooManyIds;

impl TooManyIds {
    /// The sentence the caller gets: fixed, and it names no id.
    pub const MESSAGE: &'static str = "There are too many dashboards to work out what you may see. Ask an administrator to reduce the number of dashboards or folders with their own permissions.";
}

/// One list of ids a viewer is limited by.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Scope {
    /// No limit beyond the personal space.
    Unrestricted,
    /// Everything except these ids.
    Except(Vec<String>),
    /// Only these ids.
    Only(Vec<String>),
}

impl Scope {
    /// Whether `id` passes.
    fn allows(&self, id: &str) -> bool {
        match self {
            Self::Unrestricted => true,
            Self::Except(ids) => !ids.iter().any(|i| i == id),
            Self::Only(ids) => ids.iter().any(|i| i == id),
        }
    }

    /// The `AND <column> [NOT] IN (...)` fragment, empty for no limit.
    fn clause(&self, column: &str) -> String {
        let list = |ids: &[String]| {
            ids.iter()
                .map(|i| SqlLiteral::from(i.as_str()).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        match self {
            Self::Unrestricted => String::new(),
            Self::Except(ids) if ids.is_empty() => String::new(),
            Self::Except(ids) => format!(" AND {column} NOT IN ({})", list(ids)),
            // `IN ()` is not valid SQL, and an empty "only" list means nothing.
            Self::Only(ids) if ids.is_empty() => " AND 1 = 0".to_owned(),
            Self::Only(ids) => format!(" AND {column} IN ({})", list(ids)),
        }
    }
}

/// Who is reading, as far as the personal space and the permissions are
/// concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    id: String,
    all: bool,
    boards: Scope,
    folders: Scope,
}

impl Viewer {
    /// A signed-in principal: sees the boards nobody owns, and its own.
    /// `all` is true for a principal with `identity:write`.
    #[must_use]
    pub fn user(id: impl Into<String>, all: bool) -> Self {
        Self {
            id: id.into(),
            all,
            boards: Scope::Unrestricted,
            folders: Scope::Unrestricted,
        }
    }

    /// Nobody in particular: a public link or a signed embed. Sees no owned
    /// board.
    #[must_use]
    pub fn anonymous() -> Self {
        Self {
            id: String::new(),
            all: false,
            boards: Scope::Unrestricted,
            folders: Scope::Unrestricted,
        }
    }

    /// Somebody who sees no dashboard at all: no unowned board, no personal
    /// space, no folder. For the callers that need a viewer but hold neither
    /// `dashboard:read` nor a rule (`BI-19` B1 review R3), where the baseline
    /// would show them what they must not see. Fails closed.
    #[must_use]
    pub fn nothing() -> Self {
        Self {
            id: String::new(),
            all: false,
            boards: Scope::Only(Vec::new()),
            folders: Scope::Only(Vec::new()),
        }
    }

    /// The server itself, for the work that must see every row: the
    /// version snapshot, the trash sweep, the cascade of a delete. Never built
    /// from a request.
    #[must_use]
    pub fn system() -> Self {
        Self {
            id: String::new(),
            all: true,
            boards: Scope::Unrestricted,
            folders: Scope::Unrestricted,
        }
    }

    /// The principal id, `""` for [`Self::anonymous`] and [`Self::system`].
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Whether this viewer sees every owned board.
    #[must_use]
    pub fn sees_all(&self) -> bool {
        self.all
    }

    /// Whether a board owned by `owner` (`""` = nobody) is visible.
    #[must_use]
    pub fn sees(&self, owner: &str) -> bool {
        owner.is_empty() || self.all || (!self.id.is_empty() && owner == self.id)
    }

    /// Whether the dashboard `id`, owned by `owner` (`""` = nobody), is
    /// visible: its owner rule ([`Self::sees`]) and the permission lists. The
    /// check every by-id read makes; [`Self::sees`] alone ignores the lists.
    #[must_use]
    pub fn sees_board(&self, id: &str, owner: &str) -> bool {
        self.sees(owner) && (self.all || self.boards.allows(id))
    }

    /// Whether the built-in dashboard (and the built-in tiles on `board=all`)
    /// is shown. It has no rule of its own, so it belongs to the starting
    /// point: somebody who sees only what they were given does not see it.
    #[must_use]
    pub fn sees_builtin(&self) -> bool {
        self.all || !matches!(self.boards, Scope::Only(_))
    }

    /// Whether the folder `id`, owned by `owner` (`""` = nobody), is visible.
    #[must_use]
    pub fn sees_folder(&self, id: &str, owner: &str) -> bool {
        self.sees(owner) && (self.all || self.folders.allows(id))
    }

    /// The `AND ...` fragment that keeps `bi_board` rows this viewer may see:
    /// the personal space, then the permission list. Empty for a viewer who
    /// sees all.
    #[must_use]
    pub fn owner_clause(&self) -> String {
        if self.all {
            return String::new();
        }
        format!("{}{}", self.personal_clause(), self.boards.clause("id"))
    }

    /// The same for `bi_folder` rows (the folder id list, not the dashboard
    /// one).
    #[must_use]
    pub fn folder_clause(&self) -> String {
        if self.all {
            return String::new();
        }
        format!("{}{}", self.personal_clause(), self.folders.clause("id"))
    }

    fn personal_clause(&self) -> String {
        format!(
            " AND (owner_id = '' OR owner_id = {})",
            SqlLiteral::from(self.id.as_str())
        )
    }

    /// The `AND ...` fragment that keeps `bi_chart` rows off another user's
    /// personal board and off a board the permission list leaves out, and
    /// nothing else. Empty for a viewer who sees all.
    ///
    /// The sub-select reads the LATEST row of a board (`FINAL`) whatever its
    /// state, so a private board that was deleted, and one that has been
    /// removed for good (whose tombstone keeps the owner), still hide what was
    /// on it: a chart restored from the trash after its board is gone stays
    /// private.
    #[must_use]
    pub fn chart_owner_clause(&self) -> String {
        if self.all {
            return String::new();
        }
        format!(
            " AND board NOT IN (SELECT id FROM console.bi_board FINAL WHERE owner_id != '' AND owner_id != {}){}",
            SqlLiteral::from(self.id.as_str()),
            self.boards.clause("board")
        )
    }

    /// The `AND ...` fragment that keeps the `bi_chart` rows of a live read:
    /// [`Self::chart_owner_clause`], and not on a board in the trash.
    #[must_use]
    pub fn chart_clause(&self) -> String {
        format!(
            "{} AND board NOT IN (SELECT id FROM console.bi_board FINAL WHERE is_deleted = 2)",
            self.chart_owner_clause()
        )
    }
}

/// The levels one person holds, worked out once per request from the folder
/// tree, the rules that name them, the "only the people listed" marks and the
/// starting point their roles give (feature page decisions B1 to B5).
///
/// Pure: it reads nothing and writes nothing, so the whole rule set is
/// unit-tested without a database. The rules are:
///
/// - the starting point is [`Level::View`] for `dashboard:read` and
///   [`Level::Manage`] for `dashboard:write` (the caller decides which and
///   passes it), and it is dropped for an item with a mark on it or on a
///   folder above it (B3);
/// - a rule on a folder applies to everything below it, and a rule on a
///   dashboard adds to those of its folders, never takes away: the highest
///   level among all that apply wins (B4);
/// - a dashboard in a personal space is [`Level::Manage`] for its owner and
///   nothing for anybody else, rules and starting point notwithstanding; and a
///   person who manages users (`all`) holds [`Level::Manage`] on everything
///   (B3, and today's behaviour);
/// - anything that cannot be worked out (a folder chain deeper than
///   [`MAX_FOLDER_DEPTH`], which only corrupt data produces) gives no level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessMap {
    user_id: String,
    all: bool,
    baseline: Option<Level>,
    /// Folder id to `(parent id, owner id)`.
    folders: HashMap<String, (String, String)>,
    grants: HashMap<(ItemKind, String), Level>,
    marks: HashSet<(ItemKind, String)>,
}

impl AccessMap {
    /// Build the map for `user_id`. `grants` are the rules that name the
    /// person or one of their roles; `marks` are every item marked "only the
    /// people listed"; `baseline` is the starting point their permissions
    /// give (`None` for somebody with neither `dashboard:read` nor
    /// `dashboard:write`); `all` is true for `identity:write`.
    #[must_use]
    pub fn new(
        user_id: impl Into<String>,
        all: bool,
        baseline: Option<Level>,
        folders: &[Folder],
        grants: &[Grant],
        marks: &[(ItemKind, String)],
    ) -> Self {
        let mut granted: HashMap<(ItemKind, String), Level> = HashMap::new();
        for g in grants {
            // Two rules for one item (the person's own, and a role's): the
            // higher wins, as everywhere else.
            let entry = granted
                .entry((g.kind, g.item_id.clone()))
                .or_insert(g.level);
            *entry = (*entry).max(g.level);
        }
        Self {
            user_id: user_id.into(),
            all,
            baseline,
            folders: folders
                .iter()
                .map(|f| (f.id.clone(), (f.parent_id.clone(), f.owner_id.clone())))
                .collect(),
            grants: granted,
            marks: marks.iter().cloned().collect(),
        }
    }

    /// Whether this person may open the dashboard area at all: they hold a
    /// starting point, manage users, or were given something. Somebody who is
    /// none of these gets the same refusal as before the permissions existed.
    #[must_use]
    pub fn is_entitled(&self) -> bool {
        self.all || self.baseline.is_some() || !self.grants.is_empty()
    }

    /// The level at the top of the tree, where a dashboard or folder with no
    /// parent is filed (and the built-in dashboard lives): only the starting
    /// point can give one there, as no rule can name the root (`BI-19` part
    /// B2: creating at the root needs `edit`, and `dashboard:read` alone is
    /// `view`).
    #[must_use]
    pub fn level_at_root(&self) -> Option<Level> {
        if self.all {
            return Some(Level::Manage);
        }
        self.baseline
    }

    /// The level on the dashboard `id`, which sits in folder `folder_id`
    /// (`""` = root) and belongs to `owner` (`""` = nobody). `None`: no
    /// access.
    #[must_use]
    pub fn level_for_board(&self, id: &str, folder_id: &str, owner: &str) -> Option<Level> {
        if self.all {
            return Some(Level::Manage);
        }
        if !owner.is_empty() {
            return self.owner_level(owner);
        }
        self.resolve(ItemKind::Board, id, folder_id)
    }

    /// The level on the folder `id`.
    #[must_use]
    pub fn level_for_folder(&self, id: &str) -> Option<Level> {
        if self.all {
            return Some(Level::Manage);
        }
        match self.folders.get(id) {
            Some((_, owner)) if !owner.is_empty() => self.owner_level(owner),
            Some((parent, _)) => self.resolve(ItemKind::Folder, id, parent),
            // Unknown folder: only its own rule and mark can speak.
            None => self.resolve(ItemKind::Folder, id, ""),
        }
    }

    /// Whether the dashboard `id` in folder `folder_id` is restricted, by its
    /// own mark or by a folder above it. What leaves the console to people
    /// outside the rules (a digest, a report to an outside address) is refused
    /// for such a dashboard (`BI-19` B1 owner decision Q-R12). A corrupt
    /// folder chain counts as restricted (fail closed).
    #[must_use]
    pub fn board_is_restricted(&self, id: &str, folder_id: &str) -> bool {
        self.path(ItemKind::Board, id, folder_id)
            .is_none_or(|path| path.iter().any(|item| self.marks.contains(item)))
    }

    fn owner_level(&self, owner: &str) -> Option<Level> {
        (!self.user_id.is_empty() && owner == self.user_id).then_some(Level::Manage)
    }

    /// The items from `kind`/`id` up through its folders, nearest first, or
    /// `None` when the chain is longer than a folder tree may be (corrupt).
    fn path(&self, kind: ItemKind, id: &str, folder_id: &str) -> Option<Vec<(ItemKind, String)>> {
        let mut out = vec![(kind, id.to_owned())];
        let mut next = folder_id.to_owned();
        while !next.is_empty() {
            if out.len() > MAX_FOLDER_DEPTH {
                return None;
            }
            out.push((ItemKind::Folder, next.clone()));
            next = self
                .folders
                .get(&next)
                .map(|(parent, _)| parent.clone())
                .unwrap_or_default();
        }
        Some(out)
    }

    fn resolve(&self, kind: ItemKind, id: &str, folder_id: &str) -> Option<Level> {
        let path = self.path(kind, id, folder_id)?;
        // BI-19 B1 owner decision Q-R14: a restriction also ends the rules
        // inherited from above it. Going up from the item, the rules count up
        // to and including the nearest restricted item on the path (the first
        // mark): "only the people listed" is a fresh list, so
        // managing a parent folder never opens a restricted child. Without a
        // mark the whole path counts, as before.
        let nearest_mark = path.iter().position(|item| self.marks.contains(item));
        let from_start = if nearest_mark.is_some() {
            None
        } else {
            self.baseline
        };
        let counted = nearest_mark.map_or(path.len(), |i| i + 1);
        let from_rules = path[..counted]
            .iter()
            .filter_map(|item| self.grants.get(item))
            .max();
        from_start.into_iter().chain(from_rules.copied()).max()
    }

    /// The [`Viewer`] this person reads as, given where every dashboard sits.
    /// Only the dashboards the person's starting point does not already
    /// settle are listed on the viewer.
    ///
    /// # Errors
    ///
    /// [`TooManyIds`] when a list would pass [`MAX_SCOPE_IDS`]. The request
    /// is refused rather than served with a shortened list.
    pub fn viewer(&self, boards: &[BoardPlacement]) -> Result<Viewer, TooManyIds> {
        if self.all {
            return Ok(Viewer::user(self.user_id.clone(), true));
        }
        let mut viewer = Viewer::user(self.user_id.clone(), false);
        let mut board_ids = Vec::new();
        for b in boards {
            // Another person's personal space is already out by the owner
            // clause; listing it here would only spend the cap.
            if !b.owner_id.is_empty() && b.owner_id != self.user_id {
                continue;
            }
            let seen = self
                .level_for_board(&b.id, &b.folder_id, &b.owner_id)
                .is_some();
            if seen == self.baseline.is_none() {
                board_ids.push(b.id.clone());
            }
        }
        let mut folder_ids = Vec::new();
        for (id, (_, owner)) in &self.folders {
            if !owner.is_empty() && *owner != self.user_id {
                continue;
            }
            let seen = self.level_for_folder(id).is_some();
            if seen == self.baseline.is_none() {
                folder_ids.push(id.clone());
            }
        }
        if board_ids.len() > MAX_SCOPE_IDS || folder_ids.len() > MAX_SCOPE_IDS {
            return Err(TooManyIds);
        }
        board_ids.sort();
        folder_ids.sort();
        // With a starting point the lists are the ids to hide, without one
        // the ids to show: the shorter list in each world.
        if self.baseline.is_some() {
            viewer.boards = Scope::Except(board_ids);
            viewer.folders = Scope::Except(folder_ids);
        } else {
            viewer.boards = Scope::Only(board_ids);
            viewer.folders = Scope::Only(folder_ids);
        }
        Ok(viewer)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_board_nobody_owns_is_visible_to_every_viewer() {
        assert!(Viewer::anonymous().sees(""));
        assert!(Viewer::user("u1", false).sees(""));
        assert!(Viewer::system().sees(""));
    }

    #[test]
    fn an_owned_board_is_visible_to_its_owner_and_to_an_administrator_only() {
        assert!(Viewer::user("u1", false).sees("u1"));
        assert!(!Viewer::user("u2", false).sees("u1"));
        assert!(Viewer::user("u2", true).sees("u1"));
        assert!(Viewer::system().sees("u1"));
    }

    #[test]
    fn a_public_link_and_a_signed_embed_see_no_owned_board() {
        assert!(!Viewer::anonymous().sees("u1"));
        // An owner id that is empty never matches an anonymous viewer's empty id.
        assert!(!Viewer::user("", false).sees("u1"));
    }

    #[test]
    fn the_statement_fragments_name_the_viewer_as_a_bound_literal() {
        let clause = Viewer::user("u'1", false).owner_clause();
        assert!(clause.contains("owner_id = ''"), "{clause}");
        assert!(clause.contains("'u''1'"), "{clause}");
        assert_eq!(Viewer::system().owner_clause(), "");
        let chart = Viewer::user("u1", false).chart_clause();
        assert!(chart.contains("owner_id != ''"), "{chart}");
        assert!(chart.contains("is_deleted = 2"), "{chart}");
        // Even the server's own read leaves out a chart whose board is deleted.
        assert!(Viewer::system().chart_clause().contains("is_deleted = 2"));
    }

    fn folder(id: &str, parent: &str) -> Folder {
        Folder {
            id: id.to_owned(),
            name: id.to_owned(),
            parent_id: parent.to_owned(),
            created_by: String::new(),
            updated_at: None,
            owner_id: String::new(),
        }
    }

    fn personal(id: &str, owner: &str) -> Folder {
        Folder {
            owner_id: owner.to_owned(),
            ..folder(id, "")
        }
    }

    fn grant(kind: ItemKind, id: &str, level: Level) -> Grant {
        Grant {
            kind,
            item_id: id.to_owned(),
            level,
        }
    }

    fn placed(id: &str, folder_id: &str, owner: &str) -> BoardPlacement {
        BoardPlacement {
            id: id.to_owned(),
            folder_id: folder_id.to_owned(),
            owner_id: owner.to_owned(),
        }
    }

    fn four_levels() -> Vec<Folder> {
        vec![
            folder("f1", ""),
            folder("f2", "f1"),
            folder("f3", "f2"),
            folder("f4", "f3"),
        ]
    }

    fn map(
        baseline: Option<Level>,
        folders: &[Folder],
        grants: &[Grant],
        marks: &[(ItemKind, &str)],
    ) -> AccessMap {
        let marks: Vec<_> = marks.iter().map(|(k, i)| (*k, (*i).to_owned())).collect();
        AccessMap::new("u1", false, baseline, folders, grants, &marks)
    }

    #[test]
    fn a_rule_on_a_folder_reaches_a_dashboard_four_levels_down() {
        let m = map(
            None,
            &four_levels(),
            &[grant(ItemKind::Folder, "f1", Level::View)],
            &[],
        );
        assert_eq!(m.level_for_board("b1", "f4", ""), Some(Level::View));
        assert_eq!(m.level_for_folder("f4"), Some(Level::View));
        // The root and a folder outside the tree get nothing.
        assert_eq!(m.level_for_board("b2", "", ""), None);
        assert_eq!(m.level_for_board("b3", "other", ""), None);
    }

    #[test]
    fn a_restriction_drops_the_starting_point_below_it_and_nowhere_else() {
        let m = map(
            Some(Level::View),
            &four_levels(),
            &[],
            &[(ItemKind::Folder, "f2")],
        );
        assert_eq!(m.level_for_board("above", "f1", ""), Some(Level::View));
        assert_eq!(m.level_for_board("root", "", ""), Some(Level::View));
        assert_eq!(m.level_for_board("below", "f3", ""), None);
        assert_eq!(m.level_for_board("in", "f2", ""), None);
        assert_eq!(m.level_for_folder("f1"), Some(Level::View));
        assert_eq!(m.level_for_folder("f4"), None);
        // A restriction on one dashboard leaves its neighbours alone.
        let m = map(
            Some(Level::View),
            &four_levels(),
            &[],
            &[(ItemKind::Board, "secret")],
        );
        assert_eq!(m.level_for_board("secret", "f1", ""), None);
        assert_eq!(m.level_for_board("open", "f1", ""), Some(Level::View));
    }

    /// BI-19 B1 owner decision Q-R14: a restriction ends the rules inherited
    /// from above it.
    #[test]
    fn a_rule_on_a_parent_folder_does_not_open_a_restricted_child() {
        let grants = [grant(ItemKind::Folder, "f1", Level::Manage)];
        // The person manages f1 through a rule: f1 and an unrestricted f2
        // below it are theirs, a restricted f3 below that is not.
        let m = map(None, &four_levels(), &grants, &[(ItemKind::Folder, "f3")]);
        assert_eq!(m.level_for_folder("f1"), Some(Level::Manage));
        assert_eq!(m.level_for_folder("f2"), Some(Level::Manage));
        assert_eq!(m.level_for_folder("f3"), None);
        assert_eq!(m.level_for_board("deep", "f4", ""), None);
        // A restricted dashboard directly under f1 is closed too, and a rule
        // on the dashboard itself, or on a restricted item between, opens it.
        let m = map(None, &four_levels(), &grants, &[(ItemKind::Board, "b1")]);
        assert_eq!(m.level_for_board("b1", "f2", ""), None);
        assert_eq!(m.level_for_board("b2", "f2", ""), Some(Level::Manage));
        let mut with_own = grants.to_vec();
        with_own.push(grant(ItemKind::Board, "b1", Level::View));
        let m = map(None, &four_levels(), &with_own, &[(ItemKind::Board, "b1")]);
        assert_eq!(m.level_for_board("b1", "f2", ""), Some(Level::View));
        // A rule above the nearest restricted item does not count, but one on
        // a restricted item between the target and the top still does.
        let mid = [grant(ItemKind::Folder, "f2", Level::Edit)];
        let m = map(
            None,
            &four_levels(),
            &[mid[0].clone(), grant(ItemKind::Folder, "f1", Level::Manage)],
            &[(ItemKind::Folder, "f2"), (ItemKind::Folder, "f4")],
        );
        assert_eq!(m.level_for_board("b", "f4", ""), None);
        assert_eq!(m.level_for_board("b", "f3", ""), Some(Level::Edit));
    }

    /// BI-19 B1 review R3: the viewer for somebody with neither a starting
    /// point nor a rule sees no dashboard, chart or folder at all.
    #[test]
    fn the_nothing_viewer_sees_no_dashboard_and_no_folder() {
        let v = Viewer::nothing();
        assert!(!v.sees_board("b1", ""));
        assert!(!v.sees_folder("f1", ""));
        assert!(!v.sees_builtin());
        assert!(v.owner_clause().contains("1 = 0"), "{}", v.owner_clause());
        assert!(v.chart_clause().contains("1 = 0"), "{}", v.chart_clause());
    }

    #[test]
    fn a_dashboard_is_restricted_by_its_own_mark_or_a_folder_above() {
        let m = map(
            Some(Level::View),
            &four_levels(),
            &[],
            &[(ItemKind::Folder, "f2"), (ItemKind::Board, "b9")],
        );
        assert!(m.board_is_restricted("b1", "f3"));
        assert!(m.board_is_restricted("b9", "f1"));
        assert!(!m.board_is_restricted("b1", "f1"));
        assert!(!m.board_is_restricted("b1", ""));
    }

    #[test]
    fn a_rule_under_a_restriction_still_counts_and_the_higher_level_wins() {
        let m = map(
            Some(Level::Manage),
            &four_levels(),
            &[
                grant(ItemKind::Folder, "f2", Level::View),
                grant(ItemKind::Board, "b1", Level::Edit),
            ],
            &[(ItemKind::Folder, "f2")],
        );
        // The restriction removed the Manage starting point; the folder rule
        // gives View, the dashboard rule adds Edit.
        assert_eq!(m.level_for_board("b1", "f3", ""), Some(Level::Edit));
        assert_eq!(m.level_for_board("b2", "f3", ""), Some(Level::View));
        // A dashboard rule never takes away what the folder gives.
        let m = map(
            None,
            &four_levels(),
            &[
                grant(ItemKind::Folder, "f1", Level::Manage),
                grant(ItemKind::Board, "b1", Level::View),
            ],
            &[],
        );
        assert_eq!(m.level_for_board("b1", "f2", ""), Some(Level::Manage));
        // A user rule and a role rule on one item: the higher.
        let m = map(
            None,
            &[],
            &[
                grant(ItemKind::Board, "b1", Level::View),
                grant(ItemKind::Board, "b1", Level::Edit),
            ],
            &[],
        );
        assert_eq!(m.level_for_board("b1", "", ""), Some(Level::Edit));
    }

    #[test]
    fn the_personal_space_is_unchanged_by_rules_and_the_starting_point() {
        let folders = vec![personal("fp_u1", "u1"), personal("fp_u2", "u2")];
        let m = map(
            Some(Level::Manage),
            &folders,
            &[grant(ItemKind::Folder, "fp_u2", Level::Manage)],
            &[],
        );
        assert_eq!(
            m.level_for_board("mine", "fp_u1", "u1"),
            Some(Level::Manage)
        );
        assert_eq!(m.level_for_board("theirs", "fp_u2", "u2"), None);
        assert_eq!(m.level_for_folder("fp_u1"), Some(Level::Manage));
        assert_eq!(m.level_for_folder("fp_u2"), None);
        let admin = AccessMap::new("u9", true, None, &folders, &[], &[]);
        assert_eq!(
            admin.level_for_board("theirs", "fp_u2", "u2"),
            Some(Level::Manage)
        );
        assert_eq!(admin.level_for_folder("fp_u2"), Some(Level::Manage));
        assert_eq!(
            admin.level_for_board("anything", "", ""),
            Some(Level::Manage)
        );
    }

    #[test]
    fn a_folder_chain_that_never_ends_gives_no_level() {
        let cycle = vec![folder("a", "b"), folder("b", "a")];
        let m = map(Some(Level::Manage), &cycle, &[], &[]);
        assert_eq!(m.level_for_board("b1", "a", ""), None);
        let too_deep: Vec<Folder> = (0..6)
            .map(|i| {
                folder(
                    &format!("d{i}"),
                    &if i == 0 {
                        String::new()
                    } else {
                        format!("d{}", i - 1)
                    },
                )
            })
            .collect();
        let m = map(Some(Level::Manage), &too_deep, &[], &[]);
        assert_eq!(m.level_for_board("b1", "d5", ""), None);
        assert_eq!(m.level_for_board("b1", "d3", ""), Some(Level::Manage));
    }

    #[test]
    fn nobody_is_entitled_without_a_starting_point_or_a_rule() {
        assert!(!map(None, &[], &[], &[]).is_entitled());
        assert!(map(Some(Level::View), &[], &[], &[]).is_entitled());
        assert!(map(None, &[], &[grant(ItemKind::Board, "b", Level::View)], &[]).is_entitled());
        assert!(AccessMap::new("u", true, None, &[], &[], &[]).is_entitled());
    }

    #[test]
    fn an_analyst_with_one_dashboard_rule_reads_only_that_dashboard_and_its_charts() {
        let m = map(None, &[], &[grant(ItemKind::Board, "b1", Level::View)], &[]);
        let viewer = m
            .viewer(&[
                placed("b1", "", ""),
                placed("b2", "", ""),
                placed("b3", "", ""),
            ])
            .unwrap();
        let boards = viewer.owner_clause();
        assert!(boards.contains("AND id IN ('b1')"), "{boards}");
        assert!(!boards.contains("b2"), "{boards}");
        let charts = viewer.chart_clause();
        assert!(charts.contains("AND board IN ('b1')"), "{charts}");
        assert!(viewer.sees_board("b1", ""));
        assert!(!viewer.sees_board("b2", ""));
        assert!(!viewer.sees_board("never-existed", ""));
        // The built-in board has no rule, so it is not theirs either.
        assert!(!viewer.sees_board("main", ""));
    }

    #[test]
    fn the_built_in_dashboard_belongs_to_the_starting_point() {
        let only = map(None, &[], &[grant(ItemKind::Board, "b1", Level::View)], &[])
            .viewer(&[placed("b1", "", "")])
            .unwrap();
        assert!(!only.sees_builtin());
        let start = map(Some(Level::View), &[], &[], &[(ItemKind::Board, "b1")])
            .viewer(&[placed("b1", "", "")])
            .unwrap();
        assert!(start.sees_builtin());
        assert!(Viewer::anonymous().sees_builtin());
    }

    #[test]
    fn somebody_with_no_rule_and_no_starting_point_sees_no_row_at_all() {
        let viewer = map(None, &[], &[], &[])
            .viewer(&[placed("b1", "", "")])
            .unwrap();
        assert!(viewer.owner_clause().contains("AND 1 = 0"));
        assert!(viewer.folder_clause().contains("AND 1 = 0"));
        assert!(viewer.chart_clause().contains("AND 1 = 0"));
    }

    #[test]
    fn a_viewer_with_a_starting_point_loses_only_the_restricted_ids() {
        let m = map(
            Some(Level::View),
            &four_levels(),
            &[grant(ItemKind::Board, "granted", Level::View)],
            &[(ItemKind::Folder, "f2")],
        );
        let viewer = m
            .viewer(&[
                placed("open", "f1", ""),
                placed("hidden", "f3", ""),
                placed("granted", "f3", ""),
                placed("theirs", "fp_u2", "u2"),
            ])
            .unwrap();
        let boards = viewer.owner_clause();
        assert!(boards.contains("AND id NOT IN ('hidden')"), "{boards}");
        assert!(
            !boards.contains("'open'") && !boards.contains("granted"),
            "{boards}"
        );
        assert!(viewer.sees_board("open", ""));
        assert!(!viewer.sees_board("hidden", ""));
        assert!(viewer.sees_board("granted", ""));
        let folders = viewer.folder_clause();
        assert!(
            folders.contains("AND id NOT IN ('f2', 'f3', 'f4')"),
            "{folders}"
        );
        assert!(
            viewer
                .chart_owner_clause()
                .contains("AND board NOT IN ('hidden')")
        );
    }

    #[test]
    fn an_administrator_and_the_server_carry_no_list() {
        let admin = AccessMap::new("u9", true, None, &four_levels(), &[], &[])
            .viewer(&[placed("b", "f1", "")])
            .unwrap();
        assert_eq!(admin.owner_clause(), "");
        assert_eq!(admin.folder_clause(), "");
        assert!(admin.sees_board("b", "somebody-else"));
        assert_eq!(Viewer::system().folder_clause(), "");
    }

    #[test]
    fn ids_are_quoted_as_literals_in_the_lists() {
        let m = map(
            None,
            &[],
            &[grant(ItemKind::Board, "b'1", Level::View)],
            &[],
        );
        let viewer = m.viewer(&[placed("b'1", "", "")]).unwrap();
        assert!(
            viewer.owner_clause().contains("IN ('b''1')"),
            "{}",
            viewer.owner_clause()
        );
    }

    #[test]
    fn more_than_the_cap_fails_closed_in_both_directions() {
        let many: Vec<BoardPlacement> = (0..=MAX_SCOPE_IDS)
            .map(|i| placed(&format!("b{i}"), "", ""))
            .collect();
        // Grant-only: the "only" list would be over the cap.
        let grants: Vec<Grant> = many
            .iter()
            .map(|b| grant(ItemKind::Board, &b.id, Level::View))
            .collect();
        let only = map(None, &[], &grants, &[]);
        assert_eq!(only.viewer(&many), Err(TooManyIds));
        // With a starting point and every dashboard marked: the "except" list.
        let marks: Vec<(ItemKind, String)> = many
            .iter()
            .map(|b| (ItemKind::Board, b.id.clone()))
            .collect();
        let except = AccessMap::new("u1", false, Some(Level::View), &[], &[], &marks);
        assert_eq!(except.viewer(&many), Err(TooManyIds));
        // Exactly at the cap is served.
        assert!(only.viewer(&many[..MAX_SCOPE_IDS]).is_ok());
    }

    #[test]
    fn a_level_word_round_trips_and_an_unknown_word_grants_nothing() {
        for level in [Level::View, Level::Edit, Level::Manage] {
            assert_eq!(Level::parse(level.as_str()), Some(level));
        }
        assert_eq!(Level::parse("owner"), None);
        assert_eq!(Level::parse(""), None);
        assert!(Level::View < Level::Edit && Level::Edit < Level::Manage);
        assert_eq!(ItemKind::parse("board"), Some(ItemKind::Board));
        assert_eq!(ItemKind::parse("chart"), None);
    }

    #[test]
    fn the_root_has_only_the_starting_point_and_an_administrator_manages_it() {
        let rule = grant(ItemKind::Folder, "f1", Level::Manage);
        // A rule on a folder never reaches the root.
        let granted = AccessMap::new("u1", false, None, &[folder("f1", "")], &[rule], &[]);
        assert_eq!(granted.level_at_root(), None);
        let viewer = AccessMap::new("u1", false, Some(Level::View), &[], &[], &[]);
        assert_eq!(viewer.level_at_root(), Some(Level::View));
        let admin = AccessMap::new("u1", true, None, &[], &[], &[]);
        assert_eq!(admin.level_at_root(), Some(Level::Manage));
    }
}
