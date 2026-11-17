//! The sweep of abandoned upload sessions (`DATA-17`, decision 8 of
//! `docs/core/features/upload-big-files-and-more-formats.md`).
//!
//! A chunked upload (`routes::uploads`, plan AD1) is a `receiving` row in
//! Postgres plus an S3 multipart upload in the bucket. A person who closes the
//! tab leaves both behind, and S3 keeps the pieces of an unfinished multipart
//! upload, and bills for them, until somebody aborts it. This module is that
//! somebody: [`sweep_stale_sessions`] aborts and removes every session older
//! than `UPLOAD_SESSION_TTL_HOURS` (24 by default). `main` runs it every hour.
//!
//! It also deletes the object at the session's key, tolerating its absence. An
//! API process that died between S3's "complete" and the row's move to
//! `uploaded` leaves a finished object under a `receiving` row, and no abort
//! reaches that.
//!
//! The order is the one the delete route uses: storage first, row second. A
//! storage failure leaves the row, so the next pass tries again and nothing is
//! forgotten while its bytes still exist.

use lakehouse_store::{PgPool, StoreError, uploads};

use crate::upload_store::UploadStore;

/// Sessions read per query. A bound on one pass, not on the backlog: the loop
/// reads again while a batch removed something.
const BATCH: i64 = 100;

/// What one sweep did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SweepOutcome {
    /// Sessions whose storage and row are gone.
    pub removed: usize,
    /// Sessions left for the next pass because storage failed.
    pub failed: usize,
}

/// Abort and remove every `receiving` upload created more than `ttl_hours`
/// ago.
///
/// # Errors
///
/// Returns [`StoreError`] when the list of stale sessions cannot be read or a
/// row cannot be deleted; a storage failure for one session is not an error,
/// it is counted in [`SweepOutcome::failed`] and logged.
pub async fn sweep_stale_sessions(
    pool: &PgPool,
    store: &UploadStore,
    ttl_hours: u32,
) -> Result<SweepOutcome, StoreError> {
    let hours = i32::try_from(ttl_hours).unwrap_or(i32::MAX);
    let mut outcome = SweepOutcome::default();
    loop {
        let stale = uploads::stale_receiving(pool, hours, BATCH).await?;
        if stale.is_empty() {
            return Ok(outcome);
        }
        let mut removed_here = 0_usize;
        for row in &stale {
            if let Some(multipart_id) = row.multipart_id.as_deref()
                && let Err(err) = store.abort_multipart(&row.storage_key, multipart_id).await
            {
                tracing::warn!(%err, upload_id = %row.id, "an expired upload session could not be aborted");
                outcome.failed += 1;
                continue;
            }
            if let Err(err) = store.delete(&row.storage_key).await {
                tracing::warn!(%err, upload_id = %row.id, "an expired upload session's object could not be removed");
                outcome.failed += 1;
                continue;
            }
            // `false`: the session was finished or deleted meanwhile, which is
            // not a failure of the sweep.
            if uploads::delete_receiving(pool, &row.id).await? {
                removed_here += 1;
            }
        }
        outcome.removed += removed_here;
        // A batch that removed nothing (all failed) would be read again for
        // ever: stop and let the next pass retry.
        if removed_here == 0 {
            return Ok(outcome);
        }
    }
}
