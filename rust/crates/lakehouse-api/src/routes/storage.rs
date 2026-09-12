//! `GET /api/storage` — storage-tier overview (Hot/Warm/Cold/AI).
//!
//! Ports `src/app/api/storage/route.ts`. Hot is measured directly from
//! `system.parts`; Warm is estimated from the Bronze registry's row counts
//! (`rows * 220` bytes, a fixed per-row estimate used across every ported
//! domain); Cold/AI are always zero.
//!
//! **P6 note on Cold, revisited now that object storage (RustFS/SeaweedFS)
//! exists.** Before P1 there was nowhere cold to put anything, which is why
//! Cold/AI hardcoded to zero. That infrastructure gap is gone, but the data
//! gap is not: the only thing living in `RustFS` today is Bronze Iceberg data,
//! and that is already counted above, in **Warm** (`bronze_meta.*` row
//! counts) — see `routes::catalog::list_body`, which likewise reports every
//! Bronze Iceberg table's `tier` as `"warm"`, not `"cold"`. There is no
//! second, genuinely colder tier in this design: no lifecycle rule
//! demotes aged Iceberg snapshots to a separate cold bucket/storage class,
//! and `lakehouse-iceberg` (the crate that could read a bucket's actual
//! object sizes via its `object_store` client) is not called from any route
//! yet. Reporting a real `RustFS` bucket-size number here would double-count
//! the same bytes Warm already reports, under a tier label that does not
//! correspond to any distinct storage behavior today. So Cold/AI stay at
//! zero — not because they are unmeasurable, but because there is honestly
//! nothing distinct to measure yet. This will need revisiting the day a
//! real cold tier (e.g. an Iceberg lifecycle/archival policy, or a genuinely
//! separate object-storage class) exists.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::storage::{
    self, CreateLifecyclePolicyInput, LifecyclePolicy, RestoreAssetInput, TieringOp,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::support::{js_error, num_or_zero};
use crate::state::AppState;

/// `GET /api/storage`.
pub async fn get(State(state): State<AppState>) -> Response {
    match get_body(&state.clickhouse).await {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        // `catch (e) { return NextResponse.json({ error: String(e) }, {
        // status: 503 }); }` in `storage/route.ts`.
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err) })),
        )
            .into_response(),
    }
}

async fn get_body(ch: &ChClient) -> Result<Value, ChError> {
    let hot_rows = ch
        .rows(
            "SELECT toString(sum(bytes_on_disk)) bytes, toString(uniqExact(table)) assets
         FROM system.parts WHERE database='serving' AND active",
            None,
        )
        .await?;
    let warm_rows = ch
        .rows(
            "SELECT toString(sum(total)) rows, toString(count()) assets FROM (
           SELECT total FROM lake.`bronze_meta.dataset_sync`
           UNION ALL SELECT total FROM lake.`bronze_meta_sec.dataset_sync`)",
            None,
        )
        .await?;
    let hot = hot_rows.first();
    let warm = warm_rows.first();

    let hot_bytes = num_or_zero(hot, "bytes");
    let warm_bytes = num_or_zero(warm, "rows") * 220;

    Ok(json!({
        "byTier": {
            "hot": { "bytes": hot_bytes, "assets": num_or_zero(hot, "assets"), "growth7d": 0 },
            "warm": { "bytes": warm_bytes, "assets": num_or_zero(warm, "assets"), "growth7d": 0 },
            "cold": { "bytes": 0, "assets": 0, "growth7d": 0 },
            "ai": { "bytes": 0, "assets": 0, "growth7d": 0 },
        },
        "savingsVsAllHot": savings_vs_all_hot(hot_bytes, warm_bytes),
        "failedTieringOps": 0,
        "pendingRestores": 0,
    }))
}

/// `Math.round((warmBytes / Math.max(1, hotBytes + warmBytes)) * 100)`.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "byte counts rounded to a percentage; precision loss at this \
              magnitude is inconsequential and the result is always in 0..=100"
)]
fn savings_vs_all_hot(hot_bytes: i64, warm_bytes: i64) -> i64 {
    let denom = (hot_bytes + warm_bytes).max(1) as f64;
    ((warm_bytes as f64 / denom) * 100.0).round() as i64
}

// ── Postgres-backed writes (Task 2.6) ───────────────────────────────────

/// Borrow the Postgres pool, or fail with a 503. Same pattern as
/// `routes::governance::pool`/`routes::pipelines::pool`.
fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "storage store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

fn parse_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))
}

/// `GET /api/storage/policies` — every lifecycle policy.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_policies(
    State(state): State<AppState>,
) -> ApiResult<ApiJson<Vec<LifecyclePolicy>>> {
    Ok(ApiJson(storage::list_policies(pool(&state)?).await?))
}

/// The `POST /api/storage/policies` body. Mirrors `CreateLifecyclePolicyInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePolicyBody {
    name: String,
    scope: String,
    hot_days: i32,
    warm_days: i32,
    cold_after_days: i32,
}

/// `POST /api/storage/policies` — author a new lifecycle policy. Returns
/// 201.
///
/// # Errors
///
/// 400 on a malformed body; 409 if the name is taken; 503/500 as above.
pub async fn create_policy(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<LifecyclePolicy>)> {
    let body: CreatePolicyBody = parse_body(&body)?;
    let input = CreateLifecyclePolicyInput {
        name: body.name,
        scope: body.scope,
        hot_days: body.hot_days,
        warm_days: body.warm_days,
        cold_after_days: body.cold_after_days,
    };
    let created = storage::create_policy(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

/// `GET /api/storage/operations` — every tiering operation, most recent
/// first.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_operations(State(state): State<AppState>) -> ApiResult<ApiJson<Vec<TieringOp>>> {
    Ok(ApiJson(storage::list_operations(pool(&state)?).await?))
}

/// The `POST /api/storage/restore` body. Mirrors `RestoreAssetInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreAssetBody {
    asset_id: String,
    asset_name: String,
    from: String,
    #[serde(default)]
    to: Option<String>,
}

/// `POST /api/storage/restore` — request a restore/rehydrate. Returns 201
/// (creates a new tiering operation record).
///
/// # Errors
///
/// 400 on a malformed body; 503/500 as above.
pub async fn restore_asset(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<TieringOp>)> {
    let body: RestoreAssetBody = parse_body(&body)?;
    let input = RestoreAssetInput {
        asset_id: body.asset_id,
        asset_name: body.asset_name,
        from: body.from,
        to: body.to,
    };
    let created = storage::restore_asset(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn savings_all_warm_is_100_percent() {
        assert_eq!(savings_vs_all_hot(0, 55_779_460), 100);
    }

    #[test]
    fn savings_all_hot_is_0_percent() {
        assert_eq!(savings_vs_all_hot(11_701, 0), 0);
    }

    #[test]
    fn savings_zero_both_does_not_divide_by_zero() {
        // Math.max(1, 0) === 1, so this is 0 / 1, not NaN.
        assert_eq!(savings_vs_all_hot(0, 0), 0);
    }

    #[test]
    fn savings_matches_storage_get_corpus() {
        // hot=11701 warm=55779460 -> 100 (rounds up from 99.97...%).
        assert_eq!(savings_vs_all_hot(11_701, 55_779_460), 100);
    }
}
