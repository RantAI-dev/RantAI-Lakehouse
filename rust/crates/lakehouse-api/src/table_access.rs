//! Read access per schema and table — which tables a principal's roles may
//! read at all (`GOV-7`).
//!
//! # The rule
//!
//! [`TableAccess`] is what one principal may read, resolved once from the
//! names of the roles it holds. The stored default is [`TableAccess::All`]: a role
//! reads every table until an administrator narrows it, so an upgrade never
//! changes anybody's access (`GOV-7` D1, owner 2026-11-09). A role holding
//! the unrestricted `*:*` grant is [`TableAccess::All`] too and cannot be
//! narrowed (D4): an administrator cannot lock the platform's administrators
//! out of the data they administer. Several roles combine as a union, and any
//! one role on `All` makes the whole principal `All` (spec "Several roles").
//!
//! A hidden table is HIDDEN, never "forbidden" (D2): a statement naming it is
//! refused with the tenant gate's own not-found sentence, and every list
//! omits it. Consumers do that; this module only answers
//! [`TableAccess::allows`].
//!
//! # Not included
//!
//! Column hiding (masks already do that), service identities (governed by
//! their scopes), Dagster's own runs, and the Iceberg door. A principal with
//! NO roles is [`TableAccess::All`]: that is a service identity or a system
//! run with no person behind it (`gold.rs`, `service_token.rs`), and it is
//! governed by its scopes rather than by role access.
//!
//! Feature page: `docs/core/features/table-access-per-role.md`.
//!
//! # Matching
//!
//! Names are compared exactly as the mask matcher compares
//! [`crate::policy_engine::PolicyCondition::table`] against a statement's
//! tables: case-insensitively, after [`crate::sql_rewrite::canonicalize`] has
//! dropped a leading catalog part. That is why a `Trino` catalog-qualified
//! read (`iceberg.serving.mart_x`) and a `ClickHouse` `DataLakeCatalog` read
//! (``icecat.`serving.mart_x` ``) reach the same grant as `serving.mart_x`
//! (`DATA-21`). There is no second normaliser (`AGENTS.md` rule 4).

use std::collections::HashSet;

use lakehouse_auth::PermissionSet;
use lakehouse_store::{PgPool, StoreError};

/// What one principal may read, from the roles it holds.
///
/// No `Default` on purpose (`GOV-7` S1 review round 1, S1): a default of
/// `All` would make `unwrap_or_default()` on a failed read mean "every
/// table".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableAccess {
    /// Every table. No role narrowed the principal, or one role grants `*:*`,
    /// or there is no principal at all.
    All,
    /// Only the listed tables (the first set, canonical `(schema, table)`
    /// pairs) and every table of the listed schemas (the second set).
    /// Anything not named by one of them is hidden.
    Listed(HashSet<(String, String)>, HashSet<String>),
}

impl TableAccess {
    /// Whether this principal reads every table.
    #[must_use]
    pub fn is_all(&self) -> bool {
        matches!(self, Self::All)
    }

    /// Whether `schema.table` may be read. A bare, unqualified `table` is
    /// hidden for a narrowed principal: without a schema nothing proves the
    /// grant covers it (fail closed).
    ///
    /// `schema`/`table` may be catalog-qualified (`iceberg` +
    /// `serving.mart_x`); both sides go through
    /// [`crate::sql_rewrite::canonicalize`] before they are compared.
    #[must_use]
    pub fn allows(&self, schema: &str, table: &str) -> bool {
        let Self::Listed(tables, schemas) = self else {
            return true;
        };
        let key = crate::sql_rewrite::canonicalize(&[schema.to_owned(), table.to_owned()]);
        let Some((key_schema, key_table)) = key.split_once('.') else {
            return false;
        };
        if schemas.iter().any(|s| s.eq_ignore_ascii_case(key_schema)) {
            return true;
        }
        tables.iter().any(|(granted_schema, granted_table)| {
            granted_schema.eq_ignore_ascii_case(key_schema)
                && (granted_table == "*" || granted_table.eq_ignore_ascii_case(key_table))
        })
    }

    /// [`Self::allows`] over one canonical `"schema.table"` key, as the
    /// rewrite and the lists hold them.
    #[must_use]
    pub fn allows_key(&self, key: &str) -> bool {
        match key.split_once('.') {
            Some((schema, table)) => self.allows(schema, table),
            None => self.is_all(),
        }
    }
}

/// One `(role, grant)` row as [`for_roles`] reads it — a role with no grant
/// row has `NULL` grant columns, which is "a listed role with nothing in it".
#[derive(sqlx::FromRow)]
struct RoleGrantRow {
    permissions: String,
    read_scope: String,
    schema_name: Option<String>,
    table_name: Option<String>,
}

/// Resolve what a principal holding `roles` may read, by role NAME.
///
/// Role names are the key because that is what a session carries: an
/// `OIDC`-mapped role is a name too (`0045`). A name no role matches
/// contributes nothing and is never `All` (D4).
///
/// # Errors
///
/// [`StoreError::Database`] if the roles cannot be read. The caller must
/// treat that as a refusal, never as "no restriction" (`AGENTS.md` principle
/// 3): `?` at a handler, `enforcement`'s error at the rewrite.
pub async fn for_roles(pg: Option<&PgPool>, roles: &[String]) -> Result<TableAccess, StoreError> {
    // No principal: a service identity or a system run (`gold.rs`,
    // `service_token.rs`). Those are governed by their scopes, not by role
    // table access (spec "Not included"), matching how every other
    // Postgres-optional read in this crate degrades to "no restriction".
    let Some(pg) = pg else {
        return Ok(TableAccess::All);
    };
    if roles.is_empty() {
        return Ok(TableAccess::All);
    }
    // One row per (role, grant); a role on `'all'` (or holding `*:*`) has no
    // grant rows of its own that matter. Names are bound, never formatted in.
    let rows: Vec<RoleGrantRow> = sqlx::query_as(
        "SELECT r.permissions, r.read_scope, g.schema_name, g.table_name \
         FROM role r LEFT JOIN role_read_grant g ON g.role_id = r.id \
         WHERE r.name = ANY($1)",
    )
    .bind(roles)
    .fetch_all(pg)
    .await
    .map_err(StoreError::from)?;

    let mut tables: HashSet<(String, String)> = HashSet::new();
    let mut schemas: HashSet<String> = HashSet::new();
    for row in rows {
        // `*:*` cannot be narrowed (D4), and a name that never matched a role
        // is simply absent from `rows` (D4: it contributes nothing).
        if row.read_scope == "all" || PermissionSet::parse(&row.permissions).grants_unrestricted() {
            return Ok(TableAccess::All);
        }
        let (Some(schema), Some(table)) = (row.schema_name, row.table_name) else {
            continue; // a listed role with no grant row
        };
        if table == "*" {
            // A whole-schema grant names a schema, not a table; it is stored
            // with `table_name = '*'` and matches every table of it.
            schemas.insert(canonical_schema(&schema));
        } else {
            let key = crate::sql_rewrite::canonicalize(&[schema, table]);
            if let Some((schema, table)) = key.split_once('.') {
                tables.insert((schema.to_owned(), table.to_owned()));
            }
        }
    }
    Ok(TableAccess::Listed(tables, schemas))
}

/// A schema grant, normalized through the same [`canonicalize`] the table
/// grants use, so a catalog-qualified grant (`iceberg` + `serving`) matches
/// the schema a statement's table canonicalizes to.
///
/// [`canonicalize`]: crate::sql_rewrite::canonicalize
fn canonical_schema(schema: &str) -> String {
    crate::sql_rewrite::canonicalize(&[schema.to_owned()])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use lakehouse_test_support as _;
    use uuid::Uuid;

    /// Insert a role with `permissions` and, when `scope` is `listed`, its
    /// grants. Returns the role id.
    async fn role(pool: &PgPool, name: &str, permissions: &str, scope: &str) -> Uuid {
        let (id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO role (name, permissions, read_scope) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(name)
        .bind(permissions)
        .bind(scope)
        .fetch_one(pool)
        .await
        .unwrap();
        id
    }

    async fn grant(pool: &PgPool, role_id: Uuid, schema: &str, table: &str) {
        sqlx::query(
            "INSERT INTO role_read_grant (role_id, schema_name, table_name) VALUES ($1, $2, $3)",
        )
        .bind(role_id)
        .bind(schema)
        .bind(table)
        .execute(pool)
        .await
        .unwrap();
    }

    /// D4: no roles at all is a service identity or a system run, which its
    /// scopes govern, so it reads every table.
    #[sqlx::test(migrations = "../../migrations")]
    async fn no_roles_gives_all(pool: PgPool) -> sqlx::Result<()> {
        assert_eq!(for_roles(Some(&pool), &[]).await.unwrap(), TableAccess::All);
        Ok(())
    }

    /// A deployment with no Postgres pool has no way to answer, and matches
    /// every other Postgres-optional read: no restriction.
    #[sqlx::test(migrations = "../../migrations")]
    async fn no_pool_gives_all(pool: PgPool) -> sqlx::Result<()> {
        let _ = &pool;
        assert_eq!(
            for_roles(None, &["Analyst".to_owned()]).await.unwrap(),
            TableAccess::All
        );
        Ok(())
    }

    /// D4: an `'all'` role among narrowed ones makes the whole principal
    /// unrestricted — several roles are a union.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_all_role_among_listed_ones_gives_all(pool: PgPool) -> sqlx::Result<()> {
        let narrowed = role(&pool, "Finance", "query:read", "listed").await;
        grant(&pool, narrowed, "finance", "*").await;
        role(&pool, "Every Table", "query:read", "all").await;

        let access = for_roles(
            Some(&pool),
            &["Finance".to_owned(), "Every Table".to_owned()],
        )
        .await
        .unwrap();
        assert_eq!(access, TableAccess::All, "an 'all' role wins");
        Ok(())
    }

    /// D4: `*:*` cannot be narrowed.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_star_colon_star_role_gives_all(pool: PgPool) -> sqlx::Result<()> {
        role(&pool, "Admin bundle", "*:*", "listed").await;
        role(&pool, "Finance", "query:read", "listed").await;
        let access = for_roles(
            Some(&pool),
            &["Finance".to_owned(), "Admin bundle".to_owned()],
        )
        .await
        .unwrap();
        assert_eq!(access, TableAccess::All);
        Ok(())
    }

    /// Spec "Several roles": otherwise the union of the listed grants.
    #[sqlx::test(migrations = "../../migrations")]
    async fn two_listed_roles_give_the_union(pool: PgPool) -> sqlx::Result<()> {
        let finance = role(&pool, "Finance", "query:read", "listed").await;
        grant(&pool, finance, "finance", "orders").await;
        let analytics = role(&pool, "Analytics", "query:read", "listed").await;
        grant(&pool, analytics, "silver", "customers").await;

        let access = for_roles(Some(&pool), &["Finance".to_owned(), "Analytics".to_owned()])
            .await
            .unwrap();
        assert!(access.allows("finance", "orders"));
        assert!(access.allows("silver", "customers"));
        assert!(!access.allows("serving", "mart_sales"));
        Ok(())
    }

    /// A whole-schema grant allows any table of it, including one created
    /// after the grant (names need not exist yet, D7).
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_whole_schema_grant_allows_any_table_in_it(pool: PgPool) -> sqlx::Result<()> {
        let finance = role(&pool, "Finance", "query:read", "listed").await;
        grant(&pool, finance, "finance", "*").await;

        let access = for_roles(Some(&pool), &["Finance".to_owned()])
            .await
            .unwrap();
        assert!(access.allows("finance", "orders"));
        assert!(access.allows("finance", "invoices_created_next_year"));
        assert!(!access.allows("silver", "orders"));
        Ok(())
    }

    /// D4: a name no role carries is not `All` and grants nothing.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unknown_role_name_gives_an_empty_listed(pool: PgPool) -> sqlx::Result<()> {
        role(&pool, "Finance", "query:read", "listed").await;
        let access = for_roles(Some(&pool), &["No Such Role".to_owned()])
            .await
            .unwrap();
        assert_eq!(access, TableAccess::Listed(HashSet::new(), HashSet::new()));
        assert!(!access.allows("finance", "orders"));
        Ok(())
    }

    /// Names compare as the mask matcher compares them (`DATA-21`): a Trino
    /// catalog-qualified read of a granted table matches the grant.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_trino_catalog_prefixed_name_matches_as_masks_match(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let analytics = role(&pool, "Analytics", "query:read", "listed").await;
        grant(&pool, analytics, "serving", "mart_x").await;

        let access = for_roles(Some(&pool), &["Analytics".to_owned()])
            .await
            .unwrap();
        // `Trino`'s `iceberg.serving.mart_x` canonicalizes to `serving.mart_x`.
        assert!(access.allows("iceberg", "serving.mart_x"));
        // `ClickHouse`'s `DataLakeCatalog` read of the same table.
        assert!(access.allows("lake", "serving.mart_x"));
        // The plain two-part name.
        assert!(access.allows("serving", "mart_x"));
        assert!(!access.allows("serving", "mart_y"), "only the grant");
        Ok(())
    }

    /// Case-insensitive, exactly like the mask matcher.
    #[sqlx::test(migrations = "../../migrations")]
    async fn names_compare_case_insensitively(pool: PgPool) -> sqlx::Result<()> {
        let analytics = role(&pool, "Analytics", "query:read", "listed").await;
        grant(&pool, analytics, "Serving", "Mart_X").await;

        let access = for_roles(Some(&pool), &["Analytics".to_owned()])
            .await
            .unwrap();
        assert!(access.allows("serving", "mart_x"));
        assert!(!access.allows("serving", "mart_xy"));
        Ok(())
    }

    /// `GOV-7` T5: the cost of the access check on the read path, measured
    /// rather than assumed — 1,000 iterations of `for_roles` plus ten
    /// `allows` calls, against a database holding 10 narrowed roles and 200
    /// grants (the plan's shape). The numbers are printed for the handoff and
    /// deliberately asserted only for correctness: a shared CI runner's p95
    /// says nothing about the product's budget, and a timing assertion would
    /// fail on a busy machine instead of reporting.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_access_check_costs_what_the_plan_budgeted(pool: PgPool) -> sqlx::Result<()> {
        let mut names = Vec::new();
        for role_index in 0..10 {
            let id = role(
                &pool,
                &format!("Measured {role_index}"),
                "query:read",
                "listed",
            )
            .await;
            for grant_index in 0..20 {
                let schema = format!("schema_{role_index}_{}", grant_index % 4);
                let table = format!("table_{grant_index}");
                grant(&pool, id, &schema, &table).await;
            }
            names.push(format!("Measured {role_index}"));
        }
        let probes: Vec<(String, String)> = (0..10)
            .map(|i| (format!("schema_0_{}", i % 4), format!("table_{i}")))
            .collect();

        let mut samples = Vec::with_capacity(1_000);
        for _ in 0..1_000 {
            let started = std::time::Instant::now();
            let access = for_roles(Some(&pool), &names).await.unwrap();
            for (schema, table) in &probes {
                assert!(access.allows(schema, table));
            }
            samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        }
        samples.sort_by(f64::total_cmp);
        // Nearest-rank, in integer arithmetic: the value at this index is the
        // percentile (`p` is per mille, so 500 is p50 and 950 is p95).
        let at = |permille: usize| {
            samples[(samples.len() * permille)
                .div_ceil(1000)
                .clamp(1, samples.len())
                - 1]
        };
        // Unrestricted is the shape most deployments run: it must not pay a
        // query at all.
        let unrestricted_started = std::time::Instant::now();
        for _ in 0..1_000 {
            let _ = for_roles(Some(&pool), &[]).await.unwrap();
        }
        let unrestricted_per_call =
            unrestricted_started.elapsed().as_secs_f64() * 1_000.0 / 1_000.0;
        println!(
            "GOV-7 T5: for_roles + 10 allows over 10 roles / 200 grants, 1,000 calls: \
             p50 {:.3} ms, p95 {:.3} ms, max {:.3} ms; no roles: {:.4} ms/call",
            at(500),
            at(950),
            samples[samples.len() - 1],
            unrestricted_per_call,
        );
        Ok(())
    }

    /// Fail closed: a bare, unqualified table name has no schema to match a
    /// grant against, so a narrowed principal cannot read it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unqualified_table_name_is_hidden(pool: PgPool) -> sqlx::Result<()> {
        let analytics = role(&pool, "Analytics", "query:read", "listed").await;
        grant(&pool, analytics, "serving", "mart_x").await;
        let access = for_roles(Some(&pool), &["Analytics".to_owned()])
            .await
            .unwrap();
        assert!(!access.allows("", "mart_x"));
        assert!(!access.allows_key("mart_x"));
        Ok(())
    }
}
