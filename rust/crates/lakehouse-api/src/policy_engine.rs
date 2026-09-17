//! Computes real read-time obligations (`mask`/`rowFilter`) from
//! authored `policy.conditions` for a `(principal, table)` pair.
//!
//! See the WS7 plan §0 item 3 for why this does NOT parse
//! `policy.subjects`/`policy.resources` (free-text prose) — only a
//! structured `conditions` JSON blob participates in enforcement.

use serde::Deserialize;

/// A parsed, structurally valid enforcement clause. Never constructed
/// directly outside [`PolicyCondition::parse`] — that constructor is the
/// one place "well-formed JSON" and "actually enforceable" are both
/// checked together.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PolicyCondition {
    /// Role names this policy applies to (matched against
    /// `lakehouse_auth::Principal::role_names`, WS7 item A1). Never empty
    /// for a parsed condition — see [`PolicyCondition::parse`].
    pub roles: Vec<String>,
    /// The fully-qualified table this policy governs, e.g.
    /// `"serving.mart_customer_segment"`. Matched against every table a
    /// query references (Phase B's `sql_rewrite`, not built yet).
    pub table: String,
    /// Columns to mask on read. May be empty IFF `row_filter` is
    /// present (a condition needs at least one real obligation).
    #[serde(default)]
    pub mask: Vec<String>,
    /// A row-filter expression, authored by an admin, appended to the
    /// query's `WHERE` (Phase B). Re-parsed and re-validated before use —
    /// never trusted as a raw string (Hard Requirement 1's "validated
    /// grammar, not string concatenation").
    #[serde(default, rename = "rowFilter")]
    pub row_filter: Option<String>,
}

impl PolicyCondition {
    /// Parse `raw` as a [`PolicyCondition`]. Returns `None` — not an
    /// error — for anything that isn't well-formed enforceable JSON:
    /// legacy prose, absent conditions, or a condition with no real
    /// obligation (empty `mask` AND blank/absent `rowFilter`). A
    /// genuinely malformed but JSON-shaped condition (e.g. `roles: []`)
    /// also returns `None` here; `routes::governance::create_policy_body`
    /// (WS7 item A4) is where AUTHORING such a condition is refused with a
    /// 400 — this function's job is only "can this be enforced right
    /// now", for every read-time caller that must never error on an old,
    /// non-JSON policy row.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let cond: Self = serde_json::from_str(raw).ok()?;
        if cond.roles.is_empty() || cond.table.is_empty() {
            return None;
        }
        let has_mask = !cond.mask.is_empty();
        let has_filter = cond
            .row_filter
            .as_deref()
            .is_some_and(|f| !f.trim().is_empty());
        if !has_mask && !has_filter {
            return None;
        }
        Some(cond)
    }

    /// [`Self::parse`] over an `Option<&str>`, for the common
    /// `policy.conditions: Option<String>` shape.
    #[must_use]
    #[allow(
        dead_code,
        reason = "no read-time caller exists yet in Phase A; Phase B's sql_rewrite \
                  enforcement path (and WS7 item A5's preview route) call this on every \
                  policy.conditions it loads"
    )]
    pub fn parse_opt(raw: Option<&str>) -> Option<Self> {
        raw.and_then(Self::parse)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn parses_a_well_formed_condition() {
        let raw = r#"{"roles":["Analyst"],"table":"serving.mart_customer_segment","mask":["email"],"rowFilter":"tenant_id = 'x'"}"#;
        let cond = PolicyCondition::parse(raw).expect("valid condition");
        assert_eq!(cond.roles, vec!["Analyst"]);
        assert_eq!(cond.table, "serving.mart_customer_segment");
        assert_eq!(cond.mask, vec!["email"]);
        assert_eq!(cond.row_filter.as_deref(), Some("tenant_id = 'x'"));
    }

    #[test]
    fn legacy_prose_conditions_parse_to_none_not_an_error() {
        // A pre-WS7 authored policy's `conditions` is a free-text
        // sentence or absent entirely — never a hard failure to read,
        // just "not enforceable" (WS7 plan §0 item 3).
        assert!(PolicyCondition::parse("Applies to all analysts").is_none());
    }

    #[test]
    fn absent_conditions_parse_to_none() {
        assert!(PolicyCondition::parse_opt(None).is_none());
    }

    #[test]
    fn rejects_empty_mask_and_empty_row_filter() {
        let raw = r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":[],"rowFilter":""}"#;
        // Neither obligation is present — a condition with nothing to
        // enforce is not "valid but inert", it is an authoring mistake
        // that create_policy (WS7 item A4) refuses outright, so an admin
        // never believes they wrote an enforced policy that does nothing.
        assert!(PolicyCondition::parse(raw).is_none());
    }

    #[test]
    fn rejects_empty_roles_even_with_a_real_obligation() {
        // Fail closed: a condition naming no role at all cannot be
        // matched against any principal, so it must never be treated as
        // "applies to everyone" by silently parsing.
        let raw = r#"{"roles":[],"table":"serving.mart_x","mask":["email"]}"#;
        assert!(PolicyCondition::parse(raw).is_none());
    }

    #[test]
    fn rejects_an_empty_table() {
        let raw = r#"{"roles":["Analyst"],"table":"","mask":["email"]}"#;
        assert!(PolicyCondition::parse(raw).is_none());
    }
}
