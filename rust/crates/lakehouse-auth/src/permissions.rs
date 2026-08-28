//! [`PermissionSet`]: the parsed, checkable form of `role.permissions`.
//!
//! # The real seeded format
//!
//! `role.permissions` (`0001_init.sql`) is a free-text `TEXT` column. The
//! actual seeded values (`0002_seed_identity.sql`), read directly from
//! Postgres, are comma-separated `resource:action` pairs where either half
//! may be a literal `*` wildcard:
//!
//! ```text
//! Analyst           -> "query:read, catalog:read, lineage:read"
//! Approver          -> "agent:approve, policy:review"
//! Governance Admin  -> "policy:*, residency:*, audit:read"
//! Data Engineer     -> "pipeline:*, catalog:write, connector:manage"
//! Data Scientist    -> "query:read, feature:write, notebook:run"
//! Platform Admin    -> "*:*"
//! Dashboard Viewer  -> "dashboard:read"
//! ```
//!
//! # Wildcard semantics
//!
//! A granted token `g` and a required permission `r` are each parsed as
//! `resource:action`. `g` satisfies `r` iff `(g.resource == "*" ||
//! g.resource == r.resource) && (g.action == "*" || g.action ==
//! r.action)`. This is whole-segment wildcarding only — `*` must be the
//! entire resource or action, never a prefix/suffix glob like `pol*` — that
//! is the only convention the seeded data uses, and inventing a richer glob
//! grammar the data never exercises would be unverifiable. So:
//!
//! * `"*:*"` (Platform Admin) satisfies every `resource:action` check.
//! * `"policy:*"` (Governance Admin) satisfies `policy:read`,
//!   `policy:write`, ... but not `audit:read`.
//! * `"query:read"` (Analyst) satisfies only that exact pair.
//!
//! A token that doesn't parse as exactly two `:`-separated, non-empty parts
//! is skipped rather than treated as a literal resource with no action —
//! `role.permissions` is operator-edited free text, and a malformed token
//! granting nothing is a safer failure mode than a malformed token being
//! silently reinterpreted as something it doesn't say.

use std::fmt;

/// A parsed, checkable permission grant — what `role.permissions` for every
/// role a [`crate::Principal`] holds becomes once merged, per the module
/// doc comment.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct PermissionSet(Vec<(String, String)>);

impl PermissionSet {
    /// Parse one `role.permissions` value (e.g. `"policy:*, audit:read"`)
    /// into a [`PermissionSet`]. Malformed tokens are silently dropped; see
    /// the module doc comment.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let tokens = raw
            .split(',')
            .filter_map(|token| {
                let token = token.trim();
                let (resource, action) = token.split_once(':')?;
                let (resource, action) = (resource.trim(), action.trim());
                if resource.is_empty() || action.is_empty() {
                    return None;
                }
                Some((resource.to_owned(), action.to_owned()))
            })
            .collect();
        Self(tokens)
    }

    /// Merge several sets (e.g. one per role a user holds) into one,
    /// deduplicating identical tokens.
    #[must_use]
    pub fn merge(sets: impl IntoIterator<Item = Self>) -> Self {
        let mut tokens: Vec<(String, String)> = Vec::new();
        for set in sets {
            for token in set.0 {
                if !tokens.contains(&token) {
                    tokens.push(token);
                }
            }
        }
        Self(tokens)
    }

    /// Whether this set grants `permission` (a `"resource:action"` string,
    /// e.g. `"policy:read"`), per the wildcard semantics in the module doc
    /// comment. A `permission` that doesn't itself parse as
    /// `resource:action` never matches anything.
    #[must_use]
    pub fn has(&self, permission: &str) -> bool {
        let Some((resource, action)) = permission.split_once(':') else {
            return false;
        };
        self.0.iter().any(|(g_resource, g_action)| {
            (g_resource == "*" || g_resource == resource) && (g_action == "*" || g_action == action)
        })
    }

    /// Whether this set grants nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for PermissionSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PermissionSet")
            .field("count", &self.0.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::PermissionSet;

    #[test]
    fn exact_permission_matches_only_itself() {
        let set = PermissionSet::parse("query:read, catalog:read, lineage:read");
        assert!(set.has("query:read"));
        assert!(set.has("catalog:read"));
        assert!(!set.has("query:write"));
        assert!(!set.has("catalog:write"));
        assert!(!set.has("dashboard:read"));
    }

    #[test]
    fn resource_wildcard_matches_any_action_on_that_resource() {
        let set = PermissionSet::parse("policy:*, residency:*, audit:read");
        assert!(set.has("policy:read"));
        assert!(set.has("policy:write"));
        assert!(set.has("policy:review"));
        assert!(set.has("residency:anything"));
        assert!(set.has("audit:read"));
        assert!(!set.has("audit:write"));
        assert!(!set.has("catalog:read"));
    }

    #[test]
    fn full_wildcard_matches_everything() {
        let set = PermissionSet::parse("*:*");
        assert!(set.has("query:read"));
        assert!(set.has("anything:whatsoever"));
        assert!(set.has("*:*"));
    }

    #[test]
    fn action_wildcard_is_symmetric_with_resource_wildcard() {
        // Not present in the current seed, but the grammar allows it and
        // the semantics must hold in both directions.
        let set = PermissionSet::parse("*:read");
        assert!(set.has("query:read"));
        assert!(set.has("catalog:read"));
        assert!(!set.has("query:write"));
    }

    #[test]
    fn malformed_tokens_grant_nothing() {
        let set = PermissionSet::parse("not-a-pair, :missing-resource, missing-action:, ,");
        assert!(set.is_empty());
        assert!(!set.has("not-a-pair:anything"));
    }

    #[test]
    fn a_malformed_required_permission_never_matches() {
        let set = PermissionSet::parse("*:*");
        assert!(!set.has("no-colon-here"));
    }

    #[test]
    fn merge_deduplicates_across_roles() {
        let analyst = PermissionSet::parse("query:read, catalog:read");
        let approver = PermissionSet::parse("agent:approve, catalog:read");
        let merged = PermissionSet::merge([analyst, approver]);
        assert!(merged.has("query:read"));
        assert!(merged.has("agent:approve"));
        assert!(merged.has("catalog:read"));
    }

    #[test]
    fn dashboard_viewer_role_grants_only_dashboard_read() {
        let set = PermissionSet::parse("dashboard:read");
        assert!(set.has("dashboard:read"));
        assert!(!set.has("dashboard:write"));
        assert!(!set.has("query:read"));
    }

    #[test]
    fn debug_does_not_enumerate_individual_permissions() {
        // Not a secrecy requirement (permissions aren't secret), but a
        // deliberately compact Debug avoids spamming logs with the full
        // grant list on every principal.
        let set = PermissionSet::parse("query:read, catalog:read");
        let rendered = format!("{set:?}");
        assert!(!rendered.contains("query:read"));
    }
}
