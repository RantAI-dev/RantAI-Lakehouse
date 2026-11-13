//! The central list of permission strings the API checks, and the one
//! function that answers whether a token is one of them (`WEB-8` D1/D2).
//!
//! # Why this list exists
//!
//! Before `WEB-8`, the checked permission strings lived only at their
//! check sites — `lakehouse-api::policy::POLICY_TABLE`, handler
//! `principal.has("…")` literals, and the AI tool table's `permission`
//! fields — so the console's role and service-identity forms had nothing
//! to offer but free text, and the role form's example (`query:run`)
//! named a permission that does not exist. This module is the one list
//! every picker reads (`GET /api/identity/permissions`), and
//! `lakehouse-api`'s drift test (`tests/permission_catalog.rs`) fails
//! when a checked string is missing from it, so the list cannot silently
//! fall behind what the API enforces.
//!
//! # What is deliberately NOT here
//!
//! Four strings are seeded into built-in roles but are never checked
//! anywhere (`policy:review`, `residency:*`, `feature:write`,
//! `notebook:run` — see the seed and `permissions.rs`'s module doc
//! comment). They grant nothing, so listing them would offer operators a
//! permission that does nothing. An old role that already holds one keeps
//! it on edit (`routes::identity` keeps held tokens), and the console
//! shows it as "Not recognised" — the honest answer, not a silent drop.
//!
//! # What "known" means (`is_known`)
//!
//! The wildcard grammar is [`crate::permissions`]'s: whole-segment
//! wildcards only. A token is known when
//!
//! * it is `*:*` — always valid, it is the seeded Platform Admin grant;
//! * it is `<resource>:*` and at least one listed permission has that
//!   resource — the wildcard is a shorthand for permissions that exist;
//! * it is exactly a listed permission.
//!
//! Anything else — including `<action>` wildcards over a wildcard
//! resource (`*:read`, which no check uses and the picker never offers)
//! and every unlisted `resource:action` pair — is not known, and the
//! identity create/update routes refuse it with a 400 naming it
//! (`WEB-8` D3). Refusing is the safe direction: a stored token that
//! grants nothing is also one the console cannot explain.

use serde::Serialize;

/// One entry of the permission catalog (`WEB-8` D1): the permission
/// string the API checks, the area the console groups it under, and one
/// plain-English line saying what the permission opens.
#[derive(Debug, Clone, Serialize)]
pub struct PermissionInfo {
    /// The permission string exactly as the API checks it, e.g.
    /// `query:read`.
    pub name: &'static str,
    /// The picker area this permission is grouped under, one of the
    /// seven areas of `WEB-8` D1.
    pub area: &'static str,
    /// One plain-English line saying what the permission opens, written
    /// for the person picking, not for the code ("Run SQL and read table
    /// data in Query Studio", not "query read").
    pub description: &'static str,
}

/// Every permission string the API checks today, in the order the picker
/// lists them (`WEB-8` D1). `lakehouse-api`'s drift test holds this
/// against every `POLICY_TABLE` entry, every AI tool permission and every
/// handler `has()` literal; adding a check without adding it here fails
/// that test.
pub const PERMISSIONS: &[PermissionInfo] = &[
    // ── Data ─────────────────────────────────────────────────────────────
    PermissionInfo {
        name: "catalog:read",
        area: "Data",
        description: "Browse tables and columns in the catalog.",
    },
    PermissionInfo {
        name: "catalog:write",
        area: "Data",
        description: "Edit catalog entries, descriptions and semantic types.",
    },
    PermissionInfo {
        name: "query:read",
        area: "Data",
        description: "Run SQL and read table data in Query Studio.",
    },
    PermissionInfo {
        name: "iceberg:read",
        area: "Data",
        description: "Read published tables directly over the Iceberg catalog.",
    },
    PermissionInfo {
        name: "lineage:read",
        area: "Data",
        description: "Follow table and column lineage.",
    },
    PermissionInfo {
        name: "metric:write",
        area: "Data",
        description: "Create and edit reusable metrics.",
    },
    // ── Dashboards ───────────────────────────────────────────────────────
    PermissionInfo {
        name: "dashboard:read",
        area: "Dashboards",
        description: "View dashboards and charts.",
    },
    PermissionInfo {
        name: "dashboard:sql",
        area: "Dashboards",
        description: "Write raw SQL in dashboard charts.",
    },
    PermissionInfo {
        name: "dashboard:write",
        area: "Dashboards",
        description: "Create and edit dashboards and charts.",
    },
    // ── Pipelines ────────────────────────────────────────────────────────
    PermissionInfo {
        name: "pipeline:read",
        area: "Pipelines",
        description: "View pipelines and their runs.",
    },
    PermissionInfo {
        name: "pipeline:write",
        area: "Pipelines",
        description: "Create, edit and trigger pipelines.",
    },
    PermissionInfo {
        name: "connector:manage",
        area: "Pipelines",
        description: "Manage source connections and run ingestion.",
    },
    PermissionInfo {
        name: "ingest:read",
        area: "Pipelines",
        description: "Read ingestible tables and ingestion specs, and report ingest runs.",
    },
    PermissionInfo {
        name: "gold:export",
        area: "Pipelines",
        description: "Publish Gold marts and export them as open files.",
    },
    // ── Governance ───────────────────────────────────────────────────────
    PermissionInfo {
        name: "policy:read",
        area: "Governance",
        description: "View governance policies and SLAs.",
    },
    PermissionInfo {
        name: "policy:write",
        area: "Governance",
        description: "Create and edit governance policies and SLAs.",
    },
    PermissionInfo {
        name: "governance:read",
        area: "Governance",
        description: "Data Quality and Audit Log lists (GET /api/governance/{kind}).",
    },
    PermissionInfo {
        name: "governance:write",
        area: "Governance",
        description: "Certify assets, set SLAs, run table maintenance, approve restorations, drop a table made by a file upload and restore a dropped table.",
    },
    PermissionInfo {
        name: "access:approve",
        area: "Governance",
        description: "Decide catalog access requests and manage table grants.",
    },
    PermissionInfo {
        name: "audit:read",
        area: "Governance",
        description: "Query the sensitive system tables an audit needs.",
    },
    // ── Administration ───────────────────────────────────────────────────
    PermissionInfo {
        name: "identity:read",
        area: "Administration",
        description: "View users, roles, tenants and service identities.",
    },
    PermissionInfo {
        name: "identity:write",
        area: "Administration",
        description: "Manage users, roles, tenants and service identities.",
    },
    PermissionInfo {
        name: "identity:sessions:manage",
        area: "Administration",
        description: "Revoke any person's live session.",
    },
    PermissionInfo {
        name: "settings:write",
        area: "Administration",
        description: "Change workspace settings.",
    },
    PermissionInfo {
        name: "storage:restore",
        area: "Administration",
        description: "Restore storage from a backup.",
    },
    // ── Assistant ────────────────────────────────────────────────────────
    PermissionInfo {
        name: "assistant:use",
        area: "Assistant",
        description: "The assistant, as today. Tools still need their own permissions.",
    },
    PermissionInfo {
        name: "assistant:ask",
        area: "Assistant",
        description: "The assistant in Ask mode only. Its read tools may read tables, metrics and the catalog that the person may read (masks, row filters and GOV-7 still apply).",
    },
    PermissionInfo {
        name: "agent:approve",
        area: "Assistant",
        description: "Approve the actions assistant agents ask about.",
    },
    PermissionInfo {
        name: "agent:manage",
        area: "Assistant",
        description: "Manage assistant agents.",
    },
    // ── Operations ───────────────────────────────────────────────────────
    PermissionInfo {
        name: "ops:read",
        area: "Operations",
        description: "System status (/api/ops/{kind}, /api/ops/logs's floor).",
    },
    PermissionInfo {
        name: "alert:read",
        area: "Operations",
        description: "The Alerts list (GET /api/alerts and its other GETs).",
    },
    PermissionInfo {
        name: "alert:write",
        area: "Operations",
        description: "Create and edit alert rules that watch charts and tables.",
    },
    PermissionInfo {
        name: "ops:logs",
        area: "Operations",
        description: "Read the service's own logs.",
    },
    PermissionInfo {
        name: "workload:cancel",
        area: "Operations",
        description: "Cancel running workloads.",
    },
];

/// Split a token into its `resource:action` halves the way
/// [`crate::permissions::PermissionSet::parse`] does: one `:` separator,
/// both halves trimmed, neither empty. A token that does not split is
/// `None`, and both [`is_known`] and the parse treat it as granting
/// nothing.
fn split_parts(token: &str) -> Option<(&str, &str)> {
    let (resource, action) = token.split_once(':')?;
    let (resource, action) = (resource.trim(), action.trim());
    if resource.is_empty() || action.is_empty() {
        return None;
    }
    Some((resource, action))
}

/// Whether `token` is a permission the API knows: a listed permission, a
/// `<resource>:*` shorthand for a resource some listed permission has, or
/// the unrestricted `*:*` (see the module doc comment for the full rule).
///
/// The identity create/update routes run this over every NEW token a
/// request adds and refuse the unknown ones with a 400 naming them
/// (`WEB-8` D3); a token the role or identity already holds is exempt, so
/// editing an old role never fails.
#[must_use]
pub fn is_known(token: &str) -> bool {
    let Some((resource, action)) = split_parts(token) else {
        return false;
    };
    // The unrestricted grant is always known: it is a seeded role's
    // actual value, and the picker offers it as one "Full access" box.
    if resource == "*" && action == "*" {
        return true;
    }
    // A resource wildcard is a shorthand for permissions that exist: it
    // is known when some listed permission has that resource.
    if action == "*" {
        return PERMISSIONS.iter().any(
            |p| matches!(split_parts(p.name), Some((p_resource, _)) if p_resource == resource),
        );
    }
    // An action wildcard over a wildcard resource (`*:read`) is not part
    // of the rule: nothing checks it and no picker offers it, so it is
    // refused rather than guessed into validity. Everything else must be
    // exactly a listed permission.
    PERMISSIONS
        .iter()
        .any(|p| split_parts(p.name) == Some((resource, action)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::BTreeSet;

    use super::{PERMISSIONS, is_known, split_parts};

    /// `WEB-8` D1: one entry for every permission the API checks, so the
    /// picker can only ever offer real ones. Names are unique — a
    /// duplicate would make the picker draw the same checkbox twice.
    #[test]
    fn every_listed_name_is_unique_and_well_formed() {
        let mut names = BTreeSet::new();
        for p in PERMISSIONS {
            assert!(
                split_parts(p.name).is_some(),
                "{} must parse as resource:action",
                p.name
            );
            assert!(names.insert(p.name), "duplicate catalog entry: {}", p.name);
        }
    }

    /// `WEB-8` D1: the areas are exactly the seven the plan names, and
    /// every entry carries one of them.
    #[test]
    fn every_entry_has_one_of_the_seven_plan_areas() {
        const AREAS: [&str; 7] = [
            "Data",
            "Dashboards",
            "Pipelines",
            "Governance",
            "Administration",
            "Assistant",
            "Operations",
        ];
        for p in PERMISSIONS {
            assert!(
                AREAS.contains(&p.area),
                "{} names area {:?}, which is not one of the plan's seven",
                p.name,
                p.area
            );
            assert!(
                !p.description.is_empty(),
                "{} must say what it opens",
                p.name
            );
        }
    }

    /// `WEB-8` D1: the four strings seeded into built-in roles but never
    /// checked anywhere are NOT listed — listing them would offer a
    /// permission that grants nothing. If one of these ever becomes a
    /// real check, the drift test in `lakehouse-api` forces it into the
    /// catalog, and this test must be updated to drop the exclusion.
    #[test]
    fn the_seeded_but_never_checked_strings_are_not_listed() {
        for never_checked in [
            "policy:review",
            "residency:*",
            "feature:write",
            "notebook:run",
        ] {
            assert!(
                !PERMISSIONS.iter().any(|p| p.name == never_checked),
                "{never_checked} is seeded but checked nowhere; it must stay out of the catalog"
            );
        }
    }

    #[test]
    fn a_listed_permission_is_known() {
        assert!(is_known("query:read"));
        assert!(is_known("identity:sessions:manage"));
        assert!(is_known("alert:write"));
    }

    /// Same parsing as `PermissionSet::parse`: the halves are trimmed, so
    /// a spaced token is the same token.
    #[test]
    fn a_spaced_token_is_known_like_parse_reads_it() {
        assert!(is_known(" query:read "));
        assert!(is_known("query : read"));
    }

    #[test]
    fn an_unlisted_pair_is_not_known() {
        assert!(!is_known("query:run"));
        assert!(!is_known("notebook:run"));
        assert!(!is_known("policy:review"));
    }

    /// `WEB-8` D2: `resource:*` is valid when some listed permission has
    /// that resource.
    #[test]
    fn a_resource_wildcard_is_known_when_a_listed_permission_has_the_resource() {
        assert!(is_known("dashboard:*"));
        assert!(is_known("policy:*"));
    }

    #[test]
    fn a_resource_wildcard_for_an_unlisted_resource_is_not_known() {
        assert!(!is_known("residency:*"));
        assert!(!is_known("notebook:*"));
    }

    /// `WEB-8` D2: `*:*` is always valid.
    #[test]
    fn the_full_wildcard_is_always_known() {
        assert!(is_known("*:*"));
    }

    /// A wildcard resource with a concrete action is refused: nothing
    /// checks it, and inventing a rule that would accept it is exactly the
    /// "richer grammar the data never exercises" `permissions.rs` declines
    /// to invent.
    #[test]
    fn an_action_wildcard_over_a_wildcard_resource_is_not_known() {
        assert!(!is_known("*:read"));
    }

    #[test]
    fn malformed_tokens_are_not_known() {
        assert!(!is_known("not-a-pair"));
        assert!(!is_known(":missing-resource"));
        assert!(!is_known("missing-action:"));
        assert!(!is_known(""));
        assert!(!is_known("query:read:x"));
    }
}
