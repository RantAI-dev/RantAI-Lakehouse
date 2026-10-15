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
//! # The trash
//!
//! A deleted board or chart keeps its row with `is_deleted = 2`; every read
//! that asks for `is_deleted = 0` therefore skips it without knowing the trash
//! exists. [`Viewer::chart_clause`] also hides a live chart whose board is in
//! the trash, so a chart restored on its own while its board is still deleted
//! comes back with the board, not before it.

use lakehouse_core::ident::SqlLiteral;

/// Who is reading, as far as the personal space is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    id: String,
    all: bool,
}

impl Viewer {
    /// A signed-in principal: sees the boards nobody owns, and its own.
    /// `all` is true for a principal with `identity:write`.
    #[must_use]
    pub fn user(id: impl Into<String>, all: bool) -> Self {
        Self { id: id.into(), all }
    }

    /// Nobody in particular: a public link or a signed embed. Sees no owned
    /// board.
    #[must_use]
    pub fn anonymous() -> Self {
        Self {
            id: String::new(),
            all: false,
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

    /// The `AND ...` fragment that keeps `bi_board` / `bi_folder` rows this
    /// viewer may see. Empty for a viewer who sees all.
    #[must_use]
    pub fn owner_clause(&self) -> String {
        if self.all {
            return String::new();
        }
        format!(
            " AND (owner_id = '' OR owner_id = {})",
            SqlLiteral::from(self.id.as_str())
        )
    }

    /// The `AND ...` fragment that keeps `bi_chart` rows off another user's
    /// personal board, and nothing else. Empty for a viewer who sees all.
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
            " AND board NOT IN (SELECT id FROM console.bi_board FINAL WHERE owner_id != '' AND owner_id != {})",
            SqlLiteral::from(self.id.as_str())
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
}
