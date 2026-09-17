//! Computes real read-time obligations (`mask`/`rowFilter`) from
//! authored `policy.conditions` for a `(principal, table)` pair.
//!
//! See the WS7 plan §0 item 3 for why this does NOT parse
//! `policy.subjects`/`policy.resources` (free-text prose) — only a
//! structured `conditions` JSON blob participates in enforcement.
//!
//! # Hard Requirement 1 — an unparseable authored condition is never
//! "no restriction"
//!
//! [`PolicyCondition::parse`] returns `None` for two very different
//! situations: nothing was authored (or legacy free-text prose, which
//! Phase A deliberately still allows), and a condition that IS
//! JSON-shaped — very likely an attempt at a structured, enforceable
//! clause — but fails to parse into a valid [`PolicyCondition`] (a
//! missing field, empty `roles`/`table`, or no real obligation). WS7 item A4
//! refuses the second shape at `POST /api/governance/policies` time, but
//! a `policy` row written before A4 landed, or written directly against
//! Postgres, bypasses that guard. At READ time — this module —
//! [`load_enforceable_conditions`] tells the two apart: legacy prose (not
//! JSON at all) is silently skipped, exactly as `PolicyCondition::parse`'s
//! own doc comment says; a JSON-shaped-but-unparseable `status = 'ready'`
//! condition instead fails the whole prefetch
//! ([`PolicyEngineError::UnenforceableAuthoredCondition`]), refusing every
//! query for every principal until an admin fixes or removes the row.
//! This is deliberately broad — the broken condition might name any
//! role/table, and there is no safe narrower scope to refuse instead —
//! rather than the alternative this Hard Requirement exists to forbid:
//! silently treating a broken authored restriction as "nothing to
//! enforce".

use serde::Deserialize;
use serde_json::Value;

use lakehouse_clickhouse::ChClient;
use lakehouse_core::ident::SqlLiteral;
use lakehouse_store::PgPool;
use lakehouse_store::governance;

use crate::sql_rewrite::{self, ObligationsSource, SystemTablesCatalog, TableObligations};

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

/// Errors resolving real obligations for a principal's query — never
/// forwarded verbatim to an HTTP response (`AGENTS.md` rule 4); callers
/// map every variant through [`engine_error_message`] instead.
#[derive(Debug, thiserror::Error)]
pub enum PolicyEngineError {
    /// `sql` did not parse under the configured dialect at all — this
    /// module cannot even enumerate which tables it touches.
    #[error("statement did not parse")]
    Unparseable,
    /// Loading `policy` rows from Postgres failed.
    #[error("{0}")]
    Store(#[from] lakehouse_store::StoreError),
    /// See this module's own doc comment ("Hard Requirement 1").
    #[error(
        "policy `{policy_name}` has an authored, JSON-shaped condition that cannot be enforced"
    )]
    UnenforceableAuthoredCondition {
        /// The offending policy's own name — for logs only, never placed
        /// in a response.
        policy_name: String,
    },
}

/// Fixed, non-leaking messages for [`PolicyEngineError`] — the Postgres/
/// `sql_rewrite` internals behind each variant never reach a caller.
#[must_use]
pub fn engine_error_message(err: &PolicyEngineError) -> &'static str {
    match err {
        PolicyEngineError::Unparseable => {
            "kebijakan tidak dapat dievaluasi: query tidak dapat diproses"
        }
        PolicyEngineError::Store(_) => "kebijakan tidak dapat dievaluasi: kesalahan basis data",
        PolicyEngineError::UnenforceableAuthoredCondition { .. } => {
            "kebijakan tidak dapat dievaluasi: ada kebijakan aktif dengan kondisi yang tidak \
             dapat ditegakkan"
        }
    }
}

/// Fixed, non-leaking messages for every [`sql_rewrite::RewriteError`]
/// variant — `enforce`'s own internals (a `sqlparser` message can echo the
/// raw SQL fragment back) never reach a caller, matching this module's
/// `engine_error_message` and `AGENTS.md` rule 4.
#[must_use]
pub fn refusal_message(err: &sql_rewrite::RewriteError) -> &'static str {
    match err {
        sql_rewrite::RewriteError::Unparseable => {
            "Query tidak dapat diproses oleh mesin kebijakan (sintaks tidak dikenali)."
        }
        sql_rewrite::RewriteError::UnprovableSubstitution { .. } => {
            "Query menyentuh tabel yang diatur kebijakan melalui konstruksi yang tidak didukung \
             (mis. table function, ARRAY JOIN)."
        }
        sql_rewrite::RewriteError::InvalidRowFilter { .. } => {
            "Filter baris yang diautorisasi untuk tabel ini tidak valid; hubungi admin \
             kebijakan."
        }
        sql_rewrite::RewriteError::TableFunctionDenied { .. } => {
            "Table function tidak diizinkan pada query yang diatur kebijakan."
        }
        sql_rewrite::RewriteError::SensitiveSystemTable { .. } => {
            "Membaca tabel sistem ini memerlukan izin audit:read."
        }
        sql_rewrite::RewriteError::DictOrJoinFunctionDenied { .. } => {
            "Fungsi dictGet/joinGet tidak diizinkan untuk principal dengan kebijakan aktif — \
             sumber dictionary/join table tidak dapat diverifikasi."
        }
        sql_rewrite::RewriteError::ViewOverGovernedTable { .. } => {
            "Query membaca view yang menyingkap tabel yang diatur kebijakan; tidak didukung."
        }
    }
}

/// Loads every `status = 'ready'` policy's `conditions`, applying Hard
/// Requirement 1 (see this module's own doc comment): legacy free-text
/// prose is skipped, a JSON-shaped-but-unparseable condition fails the
/// whole load.
///
/// # Errors
/// [`PolicyEngineError::Store`] if the read fails;
/// [`PolicyEngineError::UnenforceableAuthoredCondition`] per the rule
/// above.
async fn load_enforceable_conditions(
    pg: &PgPool,
) -> Result<Vec<PolicyCondition>, PolicyEngineError> {
    let policies = governance::list_policies(pg).await?;
    let mut out = Vec::new();
    for policy in policies.into_iter().filter(|p| p.status == "ready") {
        let Some(raw) = policy.conditions.as_deref() else {
            continue;
        };
        if raw.trim().is_empty() {
            continue;
        }
        let is_json = serde_json::from_str::<Value>(raw).is_ok();
        if !is_json {
            continue; // legacy prose — genuinely unenforceable, Phase A allows this
        }
        match PolicyCondition::parse(raw) {
            Some(cond) => out.push(cond),
            None => {
                return Err(PolicyEngineError::UnenforceableAuthoredCondition {
                    policy_name: policy.name,
                });
            }
        }
    }
    Ok(out)
}

/// Extends `mask` to every column whose `default_kind`/`default_expression`
/// (`ClickHouse`'s `ALIAS`/`MATERIALIZED`/`DEFAULT` computed-column
/// mechanism) references an already-masked column, iterating to a
/// fixpoint (a two-level chain needs two passes). A column whose
/// `default_expression` is non-empty but fails to parse — or parses into
/// a shape [`sql_rewrite::referenced_identifiers`] does not fully
/// enumerate — is masked too: cannot prove it does NOT reference a masked
/// column, so it fails closed (WS7 item C1, N2). A `default_expression`
/// calling `dictGet*`/`joinGet*` is masked unconditionally regardless of
/// its identifier references (N5, closes Deviation 9's cross-table gap).
fn transitive_mask_closure(
    columns: &[(String, String, String)],
    mut mask: std::collections::HashSet<String>,
) -> std::collections::HashSet<String> {
    loop {
        let mut changed = false;
        for (name, default_kind, default_expression) in columns {
            if mask.contains(name) || default_kind.is_empty() {
                continue; // already masked, or an ordinary stored column
            }
            let calls_dict_or_join = sql_rewrite::contains_dict_or_join_call(default_expression);
            let refs = sql_rewrite::referenced_identifiers(default_expression);
            let derives_from_masked = match &refs {
                Some(names) => names.iter().any(|n| mask.contains(n)),
                None => true, // unparseable/unenumerable — fail closed
            };
            if (derives_from_masked || calls_dict_or_join) && mask.insert(name.clone()) {
                changed = true;
            }
        }
        if !changed {
            return mask;
        }
    }
}

/// Computes real obligations for a `(table, roles)` pair from authored
/// `policy.conditions`, backed by real Postgres (`policy` rows) and
/// `ClickHouse` (`system.columns`/`system.tables`) reads.
pub struct PolicyEngineObligations<'a> {
    pg: Option<&'a PgPool>,
    ch: &'a ChClient,
}

impl<'a> PolicyEngineObligations<'a> {
    /// `pg: None` degrades every method to "no obligations, no views" —
    /// matching every other Postgres-optional read path in this crate
    /// (`AppState::pg`'s own doc comment) rather than a panic or a 500 for
    /// a deployment running with no Postgres pool configured.
    #[must_use]
    pub fn new(pg: Option<&'a PgPool>, ch: &'a ChClient) -> Self {
        Self { pg, ch }
    }

    /// `(name, default_kind, default_expression)` for every real column of
    /// `schema.table`, from `system.columns` — `None` when the table is
    /// unknown to the catalog (zero rows) or the query itself fails; both
    /// are "cannot determine this table's real columns", the same
    /// fail-closed signal [`sql_rewrite::TableObligations::real_columns`]
    /// being `None` already carries into `substitute_governed_tables`.
    async fn real_columns_with_defaults(
        &self,
        schema: &str,
        table: &str,
    ) -> Option<Vec<(String, String, String)>> {
        let sql = format!(
            "SELECT name, default_kind, default_expression FROM system.columns \
             WHERE database={} AND table={}",
            SqlLiteral::from(schema),
            SqlLiteral::from(table),
        );
        let rows = self.ch.rows(&sql, None).await.ok()?;
        if rows.is_empty() {
            return None;
        }
        Some(
            rows.iter()
                .map(|r| {
                    let name = r
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let default_kind = r
                        .get("default_kind")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let default_expression = r
                        .get("default_expression")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    (name, default_kind, default_expression)
                })
                .collect(),
        )
    }

    /// The mask/row-filter obligations `table` carries for a principal
    /// holding `roles`, unioning every matching `status = 'ready'` policy
    /// (masks union; row filters `AND` together) — or `None` if none
    /// apply. `mask`/`real_columns` in the returned
    /// [`TableObligations`] are extended through
    /// [`transitive_mask_closure`] whenever `table` resolves against
    /// `system.columns` (N2).
    ///
    /// # Errors
    /// See [`PolicyEngineError`].
    pub async fn obligations_for_async(
        &self,
        table: &str,
        roles: &[String],
    ) -> Result<Option<TableObligations>, PolicyEngineError> {
        let Some(pg) = self.pg else {
            return Ok(None);
        };
        let conditions = load_enforceable_conditions(pg).await?;
        let matching: Vec<&PolicyCondition> = conditions
            .iter()
            .filter(|c| {
                c.table.eq_ignore_ascii_case(table) && c.roles.iter().any(|r| roles.contains(r))
            })
            .collect();
        if matching.is_empty() {
            return Ok(None);
        }
        let mut mask: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut filters: Vec<String> = Vec::new();
        for cond in &matching {
            mask.extend(cond.mask.iter().cloned());
            if let Some(f) = cond
                .row_filter
                .as_deref()
                .map(str::trim)
                .filter(|f| !f.is_empty())
            {
                filters.push(format!("({f})"));
            }
        }
        let row_filter = (!filters.is_empty()).then(|| filters.join(" AND "));

        let Some((schema, table_name)) = table.split_once('.') else {
            // An authored table name with no schema qualifier cannot be
            // resolved against `system.columns` (which needs both parts)
            // — fail closed (`real_columns: None`) rather than guess a
            // schema; `substitute_governed_tables` refuses on this.
            return Ok(Some(TableObligations {
                mask: mask.into_iter().collect(),
                row_filter,
                real_columns: None,
            }));
        };
        let columns = self.real_columns_with_defaults(schema, table_name).await;
        let real_columns = columns
            .as_ref()
            .map(|cols| cols.iter().map(|(name, ..)| name.clone()).collect());
        let mask = match &columns {
            Some(cols) => transitive_mask_closure(cols, mask).into_iter().collect(),
            None => mask.into_iter().collect(),
        };
        Ok(Some(TableObligations {
            mask,
            row_filter,
            real_columns,
        }))
    }

    /// N5: whether ANY `status = 'ready'` policy names ANY role in
    /// `roles`, for ANY table — never matched on table, since a
    /// `dictGet`/`joinGet` call names no table syntactically at all (see
    /// [`sql_rewrite::ObligationsSource::has_any_obligation`]'s own doc
    /// comment).
    ///
    /// # Errors
    /// See [`PolicyEngineError`].
    pub async fn has_any_obligation_async(
        &self,
        roles: &[String],
    ) -> Result<bool, PolicyEngineError> {
        let Some(pg) = self.pg else {
            return Ok(false);
        };
        let conditions = load_enforceable_conditions(pg).await?;
        Ok(conditions
            .iter()
            .any(|c| c.roles.iter().any(|r| roles.contains(r))))
    }

    /// `(engine, create_table_query)` for every table in `tables`, from
    /// `system.tables`, in one batched query — an entry absent from the
    /// result (query failure, or the table genuinely unknown) is simply
    /// missing from the returned map, matching
    /// [`sql_rewrite::SystemTablesCatalog::engine_and_definition`]'s own
    /// `None`-for-unknown contract.
    async fn engine_and_definitions(
        &self,
        tables: &[String],
    ) -> std::collections::HashMap<String, (String, Option<String>)> {
        let pairs: Vec<(&str, &str)> = tables.iter().filter_map(|t| t.split_once('.')).collect();
        if pairs.is_empty() {
            return std::collections::HashMap::new();
        }
        let list = pairs
            .iter()
            .map(|(db, name)| format!("({}, {})", SqlLiteral::from(*db), SqlLiteral::from(*name)))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT database, name, engine, create_table_query FROM system.tables \
             WHERE (database, name) IN ({list})"
        );
        let Ok(rows) = self.ch.rows(&sql, None).await else {
            return std::collections::HashMap::new();
        };
        rows.iter()
            .filter_map(|r| {
                let db = r.get("database").and_then(Value::as_str)?;
                let name = r.get("name").and_then(Value::as_str)?;
                let engine = r.get("engine").and_then(Value::as_str)?.to_owned();
                let create = r
                    .get("create_table_query")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                Some((format!("{db}.{name}"), (engine, create)))
            })
            .collect()
    }

    /// Resolves everything [`sql_rewrite::enforce`] needs for `tables`
    /// (the caller's own [`sql_rewrite::referenced_tables`] result — a
    /// plain `[String]`, never a `&dyn Dialect`/raw `sql`: a `sqlparser`
    /// `Dialect` trait object is not `Send`, so it must never be held
    /// across an `.await` point, which every step in this method crosses;
    /// the caller resolves `tables` synchronously, before calling this,
    /// with whichever dialect matches the engine it is about to run
    /// against), asynchronously, up front — the "prefetch, then rewrite"
    /// shape the WS7 plan's item C1 Step 2 describes:
    /// `sql_rewrite::ObligationsSource`/`SystemTablesCatalog` stay sync
    /// per Phase B's design, since `sqlparser`'s own AST walk is sync.
    /// Returns a [`PrefetchedObligations`]/[`PrefetchedViews`] pair
    /// implementing those sync traits by plain lookup into what this call
    /// already fetched.
    ///
    /// # Errors
    /// [`PolicyEngineError::Store`]/[`PolicyEngineError::UnenforceableAuthoredCondition`]
    /// per [`Self::obligations_for_async`]/[`Self::has_any_obligation_async`].
    pub async fn prefetch_for_tables(
        &self,
        tables: &[String],
        roles: &[String],
    ) -> Result<(PrefetchedObligations, PrefetchedViews), PolicyEngineError> {
        let has_any = self.has_any_obligation_async(roles).await?;
        let mut obligations = std::collections::HashMap::new();
        for table in tables {
            if let Some(obl) = self.obligations_for_async(table, roles).await? {
                obligations.insert(table.clone(), obl);
            }
        }
        let views = self.engine_and_definitions(tables).await;
        Ok((
            PrefetchedObligations {
                map: obligations,
                has_any,
            },
            PrefetchedViews(views),
        ))
    }
}

/// The sync `sql_rewrite::ObligationsSource` adapter over what
/// [`PolicyEngineObligations::prefetch_for_principal`] already resolved
/// for ONE principal's ONE query — `has_any_obligation` ignores its
/// `principal_roles` argument because the stored `has_any` was already
/// computed for the real caller.
pub struct PrefetchedObligations {
    map: std::collections::HashMap<String, TableObligations>,
    has_any: bool,
}

impl ObligationsSource for PrefetchedObligations {
    fn obligations_for(
        &self,
        table: &str,
        _principal_roles: &[String],
    ) -> Option<TableObligations> {
        self.map.get(table).cloned()
    }

    fn has_any_obligation(&self, _principal_roles: &[String]) -> bool {
        self.has_any
    }
}

/// The sync `sql_rewrite::SystemTablesCatalog` adapter over
/// [`PolicyEngineObligations::engine_and_definitions`]'s prefetched result.
pub struct PrefetchedViews(std::collections::HashMap<String, (String, Option<String>)>);

impl SystemTablesCatalog for PrefetchedViews {
    fn engine_and_definition(&self, table: &str) -> Option<(String, Option<String>)> {
        self.0.get(table).cloned()
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
