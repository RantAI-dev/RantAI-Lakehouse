//! Repository layer for storage lifecycle policies and tiering operations:
//! the Postgres backing for `listPolicies`/`createLifecyclePolicy` and
//! `listOperations`/`restoreAsset`. `getOverview` (byte/asset counts per
//! tier) is untouched by this module — it stays `ClickHouse`-backed
//! (`routes::storage::get`), observed fact about what's actually stored
//! rather than authored/operational config. See `0009_storage.sql`'s
//! header comment.

use serde::Serialize;
use sqlx::FromRow;
use time::OffsetDateTime;

use crate::{PgPool, StoreError};

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

/// A storage lifecycle policy. Mirrors `LifecyclePolicy` in
/// `contracts/storage.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecyclePolicy {
    /// `lifecycle_policy.id`.
    pub id: String,
    /// Policy name; the table's natural key.
    pub name: String,
    /// What the policy applies to (e.g. `"core.* analytical tables"`).
    pub scope: String,
    /// Days to keep data in Hot before demoting.
    pub hot_days: i32,
    /// Days to keep data in Warm before demoting.
    pub warm_days: i32,
    /// Days after which data moves to Cold.
    pub cold_after_days: i32,
    /// `"ready" | "draft" | "paused"`.
    pub status: String,
    /// Human-readable estimated savings.
    pub estimated_savings: String,
    /// When the policy was last applied, ISO 8601.
    pub last_applied_at: String,
}

#[derive(Debug, FromRow)]
struct LifecyclePolicyRow {
    id: String,
    name: String,
    scope: String,
    hot_days: i32,
    warm_days: i32,
    cold_after_days: i32,
    status: String,
    estimated_savings: String,
    last_applied_at: OffsetDateTime,
}

impl From<LifecyclePolicyRow> for LifecyclePolicy {
    fn from(row: LifecyclePolicyRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            scope: row.scope,
            hot_days: row.hot_days,
            warm_days: row.warm_days,
            cold_after_days: row.cold_after_days,
            status: row.status,
            estimated_savings: row.estimated_savings,
            last_applied_at: iso_millis(row.last_applied_at),
        }
    }
}

const POLICY_COLUMNS: &str = "id, name, scope, hot_days, warm_days, cold_after_days, status, \
     estimated_savings, last_applied_at";

/// List every lifecycle policy, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_policies(pool: &PgPool) -> Result<Vec<LifecyclePolicy>, StoreError> {
    let sql = format!("SELECT {POLICY_COLUMNS} FROM lifecycle_policy ORDER BY created_at DESC");
    let rows: Vec<LifecyclePolicyRow> = sqlx::query_as(&sql).fetch_all(pool).await?;
    Ok(rows.into_iter().map(LifecyclePolicy::from).collect())
}

/// Everything [`create_policy`] needs. Mirrors `CreateLifecyclePolicyInput`.
#[derive(Debug, Clone)]
pub struct CreateLifecyclePolicyInput {
    /// Policy name; must not collide with an existing policy.
    pub name: String,
    /// What the policy applies to.
    pub scope: String,
    /// Days to keep data in Hot.
    pub hot_days: i32,
    /// Days to keep data in Warm.
    pub warm_days: i32,
    /// Days after which data moves to Cold.
    pub cold_after_days: i32,
}

/// Create a lifecycle policy, matching `mock/storage.ts`'s
/// `createLifecyclePolicy`: always starts `status: "draft"`,
/// `estimatedSavings: "Pending estimate"`.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken, or
/// [`StoreError::Database`] on any other failure.
pub async fn create_policy(
    pool: &PgPool,
    input: &CreateLifecyclePolicyInput,
) -> Result<LifecyclePolicy, StoreError> {
    let id = format!("lp-{}", short_id());
    let sql = format!(
        "INSERT INTO lifecycle_policy (id, name, scope, hot_days, warm_days, cold_after_days, \
         status, estimated_savings) VALUES ($1, $2, $3, $4, $5, $6, 'draft', 'Pending estimate') \
         RETURNING {POLICY_COLUMNS}"
    );
    let row: LifecyclePolicyRow = sqlx::query_as(&sql)
        .bind(&id)
        .bind(&input.name)
        .bind(&input.scope)
        .bind(input.hot_days)
        .bind(input.warm_days)
        .bind(input.cold_after_days)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// A tiering operation. Mirrors `TieringOp` in `contracts/storage.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TieringOp {
    /// `tiering_op.id`.
    pub id: String,
    /// Asset display name.
    pub asset: String,
    /// Catalog asset id, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    /// Source tier.
    pub from: String,
    /// Destination tier.
    pub to: String,
    /// `"running" | "completed" | "failed" | "cancelled"`.
    pub status: String,
    /// When the operation was recorded, ISO 8601.
    pub at: String,
    /// Human-readable detail.
    pub detail: String,
}

#[derive(Debug, FromRow)]
struct TieringOpRow {
    id: String,
    asset: String,
    asset_id: Option<String>,
    from_tier: String,
    to_tier: String,
    status: String,
    at: OffsetDateTime,
    detail: String,
}

impl From<TieringOpRow> for TieringOp {
    fn from(row: TieringOpRow) -> Self {
        Self {
            id: row.id,
            asset: row.asset,
            asset_id: row.asset_id,
            from: row.from_tier,
            to: row.to_tier,
            status: row.status,
            at: iso_millis(row.at),
            detail: row.detail,
        }
    }
}

const TIERING_OP_COLUMNS: &str = "id, asset, asset_id, from_tier, to_tier, status, at, detail";

/// List every tiering operation, most recent first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_operations(pool: &PgPool) -> Result<Vec<TieringOp>, StoreError> {
    let sql = format!("SELECT {TIERING_OP_COLUMNS} FROM tiering_op ORDER BY at DESC");
    let rows: Vec<TieringOpRow> = sqlx::query_as(&sql).fetch_all(pool).await?;
    Ok(rows.into_iter().map(TieringOp::from).collect())
}

/// Everything [`restore_asset`] needs. Mirrors `RestoreAssetInput`.
#[derive(Debug, Clone)]
pub struct RestoreAssetInput {
    /// The asset's catalog id.
    pub asset_id: String,
    /// The asset's display name.
    pub asset_name: String,
    /// The tier being restored from.
    pub from: String,
    /// The tier being restored to; defaults to `"hot"`.
    pub to: Option<String>,
}

/// Record a restore/rehydrate operation, matching `mock/storage.ts`'s
/// `restoreAsset`: always starts `status: "running"`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any failure.
pub async fn restore_asset(
    pool: &PgPool,
    input: &RestoreAssetInput,
) -> Result<TieringOp, StoreError> {
    let id = format!("op-restore-{}", short_id());
    let to = input.to.as_deref().unwrap_or("hot");
    let detail = format!("Restore / rehydrate requested to {to}");
    let sql = format!(
        "INSERT INTO tiering_op (id, asset, asset_id, from_tier, to_tier, status, detail) \
         VALUES ($1, $2, $3, $4, $5, 'running', $6) RETURNING {TIERING_OP_COLUMNS}"
    );
    let row: TieringOpRow = sqlx::query_as(&sql)
        .bind(&id)
        .bind(&input.asset_name)
        .bind(&input.asset_id)
        .bind(&input.from)
        .bind(to)
        .bind(&detail)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// A short, url-safe, time-derived id suffix — `<base36 millis>`, same
/// convention as `pipelines::slug_id`'s suffix, matching `mock/storage.ts`'s
/// `Date.now().toString(36)`.
fn short_id() -> String {
    let millis = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
    #[allow(
        clippy::cast_sign_loss,
        reason = "unix millis since epoch is always positive"
    )]
    let millis = millis as u128;
    radix36(millis)
}

fn radix36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_owned();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn serialized_field_names_match_the_typescript_contract() {
        let policy = LifecyclePolicy {
            id: "lp-1".to_owned(),
            name: "n".to_owned(),
            scope: "s".to_owned(),
            hot_days: 1,
            warm_days: 2,
            cold_after_days: 3,
            status: "draft".to_owned(),
            estimated_savings: "x".to_owned(),
            last_applied_at: "2026-01-01T00:00:00.000Z".to_owned(),
        };
        let value = serde_json::to_value(&policy).unwrap();
        for key in [
            "id",
            "name",
            "scope",
            "hotDays",
            "warmDays",
            "coldAfterDays",
            "status",
            "estimatedSavings",
            "lastAppliedAt",
        ] {
            assert!(
                value.get(key).is_some(),
                "LifecyclePolicy is missing `{key}`"
            );
        }

        let op = TieringOp {
            id: "op-1".to_owned(),
            asset: "a".to_owned(),
            asset_id: None,
            from: "hot".to_owned(),
            to: "warm".to_owned(),
            status: "running".to_owned(),
            at: "2026-01-01T00:00:00.000Z".to_owned(),
            detail: "d".to_owned(),
        };
        let value = serde_json::to_value(&op).unwrap();
        for key in ["id", "asset", "from", "to", "status", "at", "detail"] {
            assert!(value.get(key).is_some(), "TieringOp is missing `{key}`");
        }
        assert!(value.get("assetId").is_none());
    }

    #[test]
    fn radix36_matches_js_to_string_36() {
        assert_eq!(radix36(0), "0");
        assert_eq!(radix36(35), "z");
        assert_eq!(radix36(36), "10");
    }
}
