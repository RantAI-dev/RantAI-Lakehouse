//! `GET /api/storage` — storage-tier overview (Hot/Warm/Cold/AI).
//!
//! Ports `src/app/api/storage/route.ts`. Hot is measured directly from
//! `system.parts`; Warm is estimated from the Bronze registry's row counts
//! (`rows * 220` bytes, a fixed per-row estimate used across every ported
//! domain); Cold/AI are always zero — nothing occupies those tiers yet.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_clickhouse::{ChClient, ChError};
use serde_json::{Value, json};

use crate::routes::support::{js_error, num_or_zero};
use crate::state::AppState;

/// `GET /api/storage`.
pub async fn get(State(state): State<AppState>) -> Response {
    match get_body(&state.clickhouse).await {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        // `catch (e) { return NextResponse.json({ error: String(e) }, {
        // status: 503 }); }` in `storage/route.ts`.
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": js_error(err) })),
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
