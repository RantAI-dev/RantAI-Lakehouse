//! Public links to dashboards (`BI-19` part A; plan
//! `docs/superpowers/plans/2026-10-18-bi-19-sharing.md`, decisions D1 to D5;
//! feature page `docs/core/features/sharing-and-delivery.md`).
//!
//! This module holds the start-up copy of the links that existed before they
//! moved to Postgres (D2). The routes that make, change, end and open a link
//! are added by the next task of the plan.

use crate::state::AppState;
use crate::upstream_error;

/// How many times the copy tries before it gives up until the next start, and
/// how long it waits between tries. `ClickHouse` is often not ready for the
/// first seconds after a deployment starts; a copy that ran once at that moment
/// would leave every old link unreadable until somebody restarted the API.
const COPY_TRIES: u32 = 20;
const COPY_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// Copy the public links that live dashboards carried in their own row into
/// `dashboard_public_link`, once per database, each expiring 30 days from the
/// copy (D2, feature A2).
///
/// Runs in the background so a slow `ClickHouse` does not hold the API back.
/// Fail closed: until the copy has run, an old link answers "does not work";
/// it is never served from the old column. Without a Postgres pool it logs once
/// and does nothing (links need the database). The copy is idempotent and
/// safe on several API processes (`lakehouse_store::public_links::copy_legacy`
/// writes a marker in the same transaction), so every replica may run it.
pub fn spawn_legacy_copy(state: &AppState) {
    let Some(pool) = state.pg.clone() else {
        tracing::warn!("no Postgres pool configured; public links are unavailable");
        return;
    };
    let ch = state.clickhouse.clone();
    tokio::spawn(async move {
        for attempt in 1..=COPY_TRIES {
            let old = match lakehouse_bi::store::legacy_public_links(&ch).await {
                Ok(old) => old,
                Err(err) => {
                    let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
                    tracing::warn!(attempt, "public links could not be read for the copy");
                    tokio::time::sleep(COPY_WAIT).await;
                    continue;
                }
            };
            match lakehouse_store::public_links::copy_legacy(&pool, &old).await {
                Ok(copied) => {
                    tracing::info!(copied, "public links copied to Postgres");
                    return;
                }
                Err(err) => {
                    tracing::warn!(attempt, %err, "public links could not be copied");
                    tokio::time::sleep(COPY_WAIT).await;
                }
            }
        }
        tracing::error!("public links were not copied; old links stay off until the next start");
    });
}
