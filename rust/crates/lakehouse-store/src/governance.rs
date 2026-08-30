//! Repository layer for the *authored* half of the governance domain:
//! policies, and the quality/classification/residency rules an operator
//! writes down (as opposed to the *observed* results `ClickHouse` computes
//! from actually running them).
//!
//! # Postgres vs `ClickHouse`, entity by entity
//!
//! `src/services/clients/governance.ts` splits `GovernanceService` into a
//! real half (backed by `ClickHouse`/`Dagster`, ported in Phase 1 —
//! `routes::governance::{quality, audit, classification, lineage}` plus the
//! hardcoded `residency_body`) and a mock half (`listPolicies`,
//! `createPolicy`, `createQualityRule`, `createClassificationRule`,
//! `createResidencyRule`). This module is the Postgres backing for that
//! second half, and only that half:
//!
//! * **`Policy`** (list + create) — entirely new to Postgres. There was no
//!   `ClickHouse`-backed read for policies at all (`listPolicies` was 100%
//!   mock), so this is a plain authored-config CRUD, the same shape as
//!   `identity::Tenant`/`Role`.
//! * **`QualityRule`, `ClassificationRule`, `ResidencyRule`** — `create_*`
//!   lives here, same as `Policy`. So do `list_quality_rules`/
//!   `list_classification_rules`/`list_residency_rules`: **gap fix** (see
//!   `routes::governance::{quality, classification, residency}`) — a rule
//!   authored through `create_*` is unioned into the `GET
//!   /api/governance/{kind}` response on top of the `ClickHouse`-derived
//!   observations, rather than only ever living in Postgres unseen. The two
//!   sources stay genuinely distinct (authored intent vs. observed
//!   outcome), so there is no true join key between them — dedup is by the
//!   only field both a Postgres row and its `ClickHouse` id-namespace can
//!   never collide on today (`ClickHouse` ids are `"q-<n>"`/`"c-<slug>"`
//!   synthesized per-row, `Policy`/rule ids from Postgres are `UUID`s), so
//!   union-by-id is a no-op in practice and a safety net if that ever
//!   changes. An authored rule that has never been evaluated is presented
//!   with the same "not yet run" defaults `create_*` already gives it —
//!   `lastStatus: "warning"` / `lastRunAt: now()` for quality,
//!   `reviewStatus: "needs-review"` for classification,
//!   `violations7d: 0` for residency — the contract has no "unevaluated"
//!   state, so this is the most honest value already representable: it
//!   reads as "authored, not yet contradicted by evidence" rather than
//!   fabricating a pass/fail verdict nobody observed.

use serde::Serialize;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{PgPool, StoreError};

/// Render a timestamp the way JavaScript's `Date.prototype.toISOString`
/// does. Duplicated from `identity.rs` rather than shared: the two modules
/// have no other coupling, and a one-line private helper is cheaper than a
/// new shared module for it.
fn iso_millis(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    )
}

// ── Policy ──────────────────────────────────────────────────────────────

/// An authored policy. Mirrors `Policy` in `contracts/governance.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    /// `policy.id`, as a string.
    pub id: String,
    /// Policy name; the table's natural key.
    pub name: String,
    /// `"draft"` or `"ready"` in practice (see [`CreatePolicyInput::activate`]).
    pub status: String,
    /// Policy kind (e.g. `"Row filter"`, `"Agent autonomy"`).
    pub kind: String,
    /// Who/what the policy applies to.
    pub subjects: String,
    /// What the policy applies to.
    pub resources: String,
    /// The policy's effect (e.g. `"Permit with obligation"`).
    pub effect: String,
    /// Revision number; always `1` for a freshly created policy.
    pub version: i32,
    /// Who owns this policy.
    pub owner: String,
    /// When the policy was last written, ISO 8601. Serializes as `updatedAt`.
    pub updated_at: String,
}

#[derive(Debug, FromRow)]
struct PolicyRow {
    id: Uuid,
    name: String,
    status: String,
    kind: String,
    subjects: String,
    resources: String,
    effect: String,
    version: i32,
    owner: String,
    updated_at: OffsetDateTime,
}

impl From<PolicyRow> for Policy {
    fn from(row: PolicyRow) -> Self {
        Self {
            id: row.id.to_string(),
            name: row.name,
            status: row.status,
            kind: row.kind,
            subjects: row.subjects,
            resources: row.resources,
            effect: row.effect,
            version: row.version,
            owner: row.owner,
            updated_at: iso_millis(row.updated_at),
        }
    }
}

/// The column list shared by every policy read/write. Deliberately just the
/// columns — no `FROM`/`WHERE` — so it can be reused verbatim both after a
/// `SELECT` ([`list_policies`]) and after a `RETURNING`
/// ([`create_policy`]), which need the same columns but never the same
/// clause around them.
const POLICY_COLUMNS: &str =
    "id, name, status, kind, subjects, resources, effect, version, owner, updated_at";

/// List every authored quality rule, newest first. Used by
/// `GET /api/governance/quality` to union authored rules on top of the
/// `ClickHouse`-derived observations — see the gap-fix note on
/// `routes::governance::quality`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_quality_rules(pool: &PgPool) -> Result<Vec<QualityRule>, StoreError> {
    let rows: Vec<QualityRuleRow> = sqlx::query_as(
        "SELECT id, name, asset, dimension, threshold, severity, last_status, last_run_at \
         FROM quality_rule ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(QualityRule::from).collect())
}

/// List every authored classification rule, newest first. Used by
/// `GET /api/governance/classification`; see [`list_quality_rules`].
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_classification_rules(
    pool: &PgPool,
) -> Result<Vec<ClassificationRule>, StoreError> {
    let rows: Vec<ClassificationRuleRow> = sqlx::query_as(
        "SELECT id, asset, column_name, classification, confidence, review_status, masking_rule \
         FROM classification_rule ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(ClassificationRule::from).collect())
}

/// List every authored residency rule, newest first. Used by
/// `GET /api/governance/residency`; see [`list_quality_rules`].
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_residency_rules(pool: &PgPool) -> Result<Vec<ResidencyRule>, StoreError> {
    let rows: Vec<ResidencyRuleRow> = sqlx::query_as(
        "SELECT id, tenant, classification, approved_sites, cross_site_allowed, \
         allowed_output, violations_7d FROM residency_rule ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(ResidencyRule::from).collect())
}

/// List every authored policy, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_policies(pool: &PgPool) -> Result<Vec<Policy>, StoreError> {
    let sql = format!("SELECT {POLICY_COLUMNS} FROM policy ORDER BY created_at DESC, name");
    let rows: Vec<PolicyRow> = sqlx::query_as(&sql).fetch_all(pool).await?;
    Ok(rows.into_iter().map(Policy::from).collect())
}

/// Everything [`create_policy`] needs. Mirrors `CreatePolicyInput`.
#[derive(Debug, Clone)]
pub struct CreatePolicyInput {
    /// Policy name; must not collide with an existing policy.
    pub name: String,
    /// Policy kind.
    pub kind: String,
    /// Who/what the policy applies to.
    pub subjects: String,
    /// What the policy applies to.
    pub resources: String,
    /// The policy's effect.
    pub effect: String,
    /// Free-text condition expression. Stored (an authored policy's
    /// conditions are as much a fact about it as its effect), but not part
    /// of the `Policy` wire type — the contract never reads it back.
    pub conditions: Option<String>,
    /// `true` -> status `"ready"`, `false`/absent -> `"draft"`, matching
    /// `mock/identity.ts`'s `createPolicy`.
    pub activate: bool,
    /// Policy owner; defaults to [`DEFAULT_OWNER`] when absent.
    pub owner: Option<String>,
}

/// The owner a policy gets when the caller doesn't name one, matching
/// `mock/governance.ts`'s `createPolicy` (`input.owner ?? "Current user"`).
const DEFAULT_OWNER: &str = "Current user";

/// Create a policy. A freshly created policy is always version 1.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken, or
/// [`StoreError::Database`] on any other failure.
pub async fn create_policy(pool: &PgPool, input: &CreatePolicyInput) -> Result<Policy, StoreError> {
    let status = if input.activate { "ready" } else { "draft" };
    let owner = input.owner.as_deref().unwrap_or(DEFAULT_OWNER);
    let sql = format!(
        "INSERT INTO policy (name, status, kind, subjects, resources, effect, conditions, owner) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING {POLICY_COLUMNS}"
    );
    let row: PolicyRow = sqlx::query_as(&sql)
        .bind(&input.name)
        .bind(status)
        .bind(&input.kind)
        .bind(&input.subjects)
        .bind(&input.resources)
        .bind(&input.effect)
        .bind(input.conditions.as_deref())
        .bind(owner)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

// ── Quality rule (create only — see module doc comment) ────────────────

/// An authored quality rule. Mirrors `QualityRule` in
/// `contracts/governance.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityRule {
    /// `quality_rule.id`, as a string.
    pub id: String,
    /// Rule name; the table's natural key.
    pub name: String,
    /// The asset (table) this rule checks.
    pub asset: String,
    /// The quality dimension (e.g. `"completeness"`).
    pub dimension: String,
    /// The threshold expression (e.g. `">= 95%"`).
    pub threshold: String,
    /// Severity if the rule fails.
    pub severity: String,
    /// Most recent verdict. Always `"warning"` for a freshly authored rule
    /// (see [`create_quality_rule`]). Serializes as `lastStatus`.
    pub last_status: String,
    /// When the rule was last (attempted to be) run, ISO 8601. Serializes
    /// as `lastRunAt`.
    pub last_run_at: String,
}

#[derive(Debug, FromRow)]
struct QualityRuleRow {
    id: Uuid,
    name: String,
    asset: String,
    dimension: String,
    threshold: String,
    severity: String,
    last_status: String,
    last_run_at: OffsetDateTime,
}

impl From<QualityRuleRow> for QualityRule {
    fn from(row: QualityRuleRow) -> Self {
        Self {
            id: row.id.to_string(),
            name: row.name,
            asset: row.asset,
            dimension: row.dimension,
            threshold: row.threshold,
            severity: row.severity,
            last_status: row.last_status,
            last_run_at: iso_millis(row.last_run_at),
        }
    }
}

/// Everything [`create_quality_rule`] needs. Mirrors `CreateQualityRuleInput`.
#[derive(Debug, Clone)]
pub struct CreateQualityRuleInput {
    /// Rule name; must not collide with an existing rule.
    pub name: String,
    /// The asset (table) this rule checks.
    pub asset: String,
    /// The quality dimension.
    pub dimension: String,
    /// The threshold expression.
    pub threshold: String,
    /// Severity if the rule fails.
    pub severity: String,
}

/// Create a quality rule. Freshly authored rules start `"warning"` /
/// `now()`, matching `mock/governance.ts`'s `createQualityRule` (a rule
/// nobody has run yet has no real verdict).
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken, or
/// [`StoreError::Database`] on any other failure.
pub async fn create_quality_rule(
    pool: &PgPool,
    input: &CreateQualityRuleInput,
) -> Result<QualityRule, StoreError> {
    let row: QualityRuleRow = sqlx::query_as(
        "INSERT INTO quality_rule (name, asset, dimension, threshold, severity) \
         VALUES ($1, $2, $3, $4, $5) \
         RETURNING id, name, asset, dimension, threshold, severity, last_status, last_run_at",
    )
    .bind(&input.name)
    .bind(&input.asset)
    .bind(&input.dimension)
    .bind(&input.threshold)
    .bind(&input.severity)
    .fetch_one(pool)
    .await?;
    Ok(row.into())
}

// ── Classification rule (create only — see module doc comment) ─────────

/// An authored classification/masking rule. Mirrors `ClassificationRule` in
/// `contracts/governance.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationRule {
    /// `classification_rule.id`, as a string.
    pub id: String,
    /// The asset (table) this rule classifies.
    pub asset: String,
    /// The specific column, if this rule is column-level rather than
    /// asset-level. Omitted from JSON when absent (see [`Option::is_none`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    /// The classification level.
    pub classification: String,
    /// Confidence in this classification, `0`-`1`; always `1` for a
    /// freshly authored rule (a human just asserted it).
    pub confidence: i32,
    /// `"auto"`, `"reviewed"`, or `"needs-review"`. Serializes as
    /// `reviewStatus`.
    pub review_status: String,
    /// The masking technique to apply, if any. Omitted from JSON when
    /// absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub masking_rule: Option<String>,
}

#[derive(Debug, FromRow)]
struct ClassificationRuleRow {
    id: Uuid,
    asset: String,
    column_name: Option<String>,
    classification: String,
    confidence: f64,
    review_status: String,
    masking_rule: Option<String>,
}

impl From<ClassificationRuleRow> for ClassificationRule {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "confidence is always in [0,1]; a freshly authored rule is exactly 1"
    )]
    fn from(row: ClassificationRuleRow) -> Self {
        Self {
            id: row.id.to_string(),
            asset: row.asset,
            column: row.column_name,
            classification: row.classification,
            confidence: row.confidence.round() as i32,
            review_status: row.review_status,
            masking_rule: row.masking_rule,
        }
    }
}

/// Everything [`create_classification_rule`] needs. Mirrors
/// `CreateClassificationRuleInput`.
#[derive(Debug, Clone)]
pub struct CreateClassificationRuleInput {
    /// The asset (table) this rule classifies.
    pub asset: String,
    /// The specific column, if column-level.
    pub column: Option<String>,
    /// The classification level.
    pub classification: String,
    /// The masking technique to apply, if any.
    pub masking_rule: Option<String>,
}

/// Create a classification rule. Freshly authored rules start
/// `confidence: 1`, `reviewStatus: "needs-review"`, matching
/// `mock/governance.ts`'s `createClassificationRule` (a human just asserted
/// this classification; it hasn't been reviewed yet).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any failure. No uniqueness
/// constraint exists on this table: unlike `Policy`/`QualityRule`, an
/// asset+column combination legitimately has no natural key that
/// `mock/governance.ts` treats as unique (a fixture could plausibly gain a
/// second, superseding classification rule).
pub async fn create_classification_rule(
    pool: &PgPool,
    input: &CreateClassificationRuleInput,
) -> Result<ClassificationRule, StoreError> {
    let row: ClassificationRuleRow = sqlx::query_as(
        "INSERT INTO classification_rule (asset, column_name, classification, masking_rule) \
         VALUES ($1, $2, $3, $4) \
         RETURNING id, asset, column_name, classification, confidence, review_status, masking_rule",
    )
    .bind(&input.asset)
    .bind(input.column.as_deref())
    .bind(&input.classification)
    .bind(input.masking_rule.as_deref())
    .fetch_one(pool)
    .await?;
    Ok(row.into())
}

// ── Residency rule (create only — see module doc comment) ──────────────

/// An authored residency rule. Mirrors `ResidencyRule` in
/// `contracts/governance.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidencyRule {
    /// `residency_rule.id`, as a string.
    pub id: String,
    /// The tenant this rule applies to (by slug).
    pub tenant: String,
    /// The classification level this rule governs.
    pub classification: String,
    /// Sites approved to hold this data. Serializes as `approvedSites`.
    pub approved_sites: Vec<String>,
    /// Whether data may move between approved sites. Serializes as
    /// `crossSiteAllowed`.
    pub cross_site_allowed: bool,
    /// What output form is allowed to leave the approved sites. Serializes
    /// as `allowedOutput`.
    pub allowed_output: String,
    /// Observed violations in the last 7 days; always `0` for a freshly
    /// authored rule.
    pub violations7d: i32,
}

#[derive(Debug, FromRow)]
struct ResidencyRuleRow {
    id: Uuid,
    tenant: String,
    classification: String,
    approved_sites: Vec<String>,
    cross_site_allowed: bool,
    allowed_output: String,
    violations_7d: i32,
}

impl From<ResidencyRuleRow> for ResidencyRule {
    fn from(row: ResidencyRuleRow) -> Self {
        Self {
            id: row.id.to_string(),
            tenant: row.tenant,
            classification: row.classification,
            approved_sites: row.approved_sites,
            cross_site_allowed: row.cross_site_allowed,
            allowed_output: row.allowed_output,
            violations7d: row.violations_7d,
        }
    }
}

/// Everything [`create_residency_rule`] needs. Mirrors
/// `CreateResidencyRuleInput`.
#[derive(Debug, Clone)]
pub struct CreateResidencyRuleInput {
    /// The tenant this rule applies to (by slug).
    pub tenant: String,
    /// The classification level this rule governs.
    pub classification: String,
    /// Sites approved to hold this data.
    pub approved_sites: Vec<String>,
    /// Whether data may move between approved sites.
    pub cross_site_allowed: bool,
    /// What output form is allowed to leave the approved sites.
    pub allowed_output: String,
}

/// Create a residency rule. `violations7d` starts at `0`, matching
/// `mock/governance.ts`'s `createResidencyRule` (a brand-new rule has no
/// observed violations yet).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any failure.
pub async fn create_residency_rule(
    pool: &PgPool,
    input: &CreateResidencyRuleInput,
) -> Result<ResidencyRule, StoreError> {
    let row: ResidencyRuleRow = sqlx::query_as(
        "INSERT INTO residency_rule (tenant, classification, approved_sites, cross_site_allowed, allowed_output) \
         VALUES ($1, $2, $3, $4, $5) \
         RETURNING id, tenant, classification, approved_sites, cross_site_allowed, allowed_output, violations_7d",
    )
    .bind(&input.tenant)
    .bind(&input.classification)
    .bind(&input.approved_sites)
    .bind(input.cross_site_allowed)
    .bind(&input.allowed_output)
    .fetch_one(pool)
    .await?;
    Ok(row.into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// The wire format is the contract: every key the browser reads must be
    /// the camelCase name `contracts/governance.ts` declares.
    #[test]
    fn serialized_field_names_match_the_typescript_contract() {
        let policy = Policy {
            id: "p".to_owned(),
            name: "n".to_owned(),
            status: "draft".to_owned(),
            kind: "k".to_owned(),
            subjects: "s".to_owned(),
            resources: "r".to_owned(),
            effect: "e".to_owned(),
            version: 1,
            owner: "o".to_owned(),
            updated_at: "2026-01-01T00:00:00.000Z".to_owned(),
        };
        let value = serde_json::to_value(&policy).unwrap();
        for key in [
            "id",
            "name",
            "status",
            "kind",
            "subjects",
            "resources",
            "effect",
            "version",
            "owner",
            "updatedAt",
        ] {
            assert!(value.get(key).is_some(), "Policy is missing `{key}`");
        }

        let quality = QualityRule {
            id: "q".to_owned(),
            name: "n".to_owned(),
            asset: "a".to_owned(),
            dimension: "d".to_owned(),
            threshold: "t".to_owned(),
            severity: "high".to_owned(),
            last_status: "warning".to_owned(),
            last_run_at: "2026-01-01T00:00:00.000Z".to_owned(),
        };
        let value = serde_json::to_value(&quality).unwrap();
        for key in [
            "id",
            "name",
            "asset",
            "dimension",
            "threshold",
            "severity",
            "lastStatus",
            "lastRunAt",
        ] {
            assert!(value.get(key).is_some(), "QualityRule is missing `{key}`");
        }

        let residency = ResidencyRule {
            id: "r".to_owned(),
            tenant: "t".to_owned(),
            classification: "restricted".to_owned(),
            approved_sites: vec!["Jakarta".to_owned()],
            cross_site_allowed: false,
            allowed_output: "aggregates only".to_owned(),
            violations7d: 0,
        };
        let value = serde_json::to_value(&residency).unwrap();
        for key in [
            "id",
            "tenant",
            "classification",
            "approvedSites",
            "crossSiteAllowed",
            "allowedOutput",
            "violations7d",
        ] {
            assert!(value.get(key).is_some(), "ResidencyRule is missing `{key}`");
        }
    }

    /// `ClassificationRule.column`/`maskingRule` are optional in the
    /// contract (`column?: string`); a rule created without them must omit
    /// the keys entirely, not emit `null`, matching how the mock's
    /// `undefined` fields serialized.
    #[test]
    fn classification_rule_omits_absent_optional_fields() {
        let rule = ClassificationRule {
            id: "c".to_owned(),
            asset: "a".to_owned(),
            column: None,
            classification: "internal".to_owned(),
            confidence: 1,
            review_status: "needs-review".to_owned(),
            masking_rule: None,
        };
        let value = serde_json::to_value(&rule).unwrap();
        assert!(value.get("column").is_none());
        assert!(value.get("maskingRule").is_none());
        assert!(value.get("asset").is_some());
    }
}
