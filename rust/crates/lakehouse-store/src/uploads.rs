//! Repository layer for `file_upload` — the registry of files a user
//! has uploaded through the console, and what became of each one.
//!
//! The file's BYTES are not here: they live in the warehouse bucket under
//! an `uploads/` prefix, and [`Upload::storage_key`] points at them. See
//! `migrations/0054_upload.sql`'s header for why, and for why
//! `storage_key` is always a server-generated name rather than the user's
//! own filename.
//!
//! # Tenant scope (ADR 0014, decision 6)
//!
//! An upload belongs to a tenant (`tenant_id`, `0055_upload_tenant_mode
//! .sql`). [`list`] and [`find_by_sha256`] take the tenant and cannot be
//! called without one: there is no "unscoped" variant, because no caller
//! exists that has no tenant (`connectors::ConnectorFilter` keeps one for
//! its non-route callers; uploads have none). [`upload_in_tenants`] is the
//! per-id access rule the routes apply, shaped like
//! `connectors::connector_in_tenants`: `false` for an unknown id, another
//! tenant's upload, a deleted upload and an upload with no tenant alike, so
//! the answer says nothing about which ids exist. [`get`], [`mark_ingesting`],
//! [`attach_run`], [`mark_finished`] and [`soft_delete`] take only an id and
//! are for a caller that has already passed [`upload_in_tenants`] for it.
//!
//! # Deleting is soft (T5a, review finding B1)
//!
//! [`soft_delete`] sets `deleted_at` and keeps the row. Deleting an upload
//! keeps the table it loaded (ADR 0014, decision 1), and a later upload may
//! load into that table only because some upload of the same tenant claimed
//! the name (see below). The row is that record, so it stays. Every function
//! here that reads or changes an upload sees LIVE rows only (`deleted_at IS
//! NULL`); the two that answer "who owns this table name",
//! [`table_claimed_by_upload`] and [`table_loaded_by_upload`], count deleted
//! rows on purpose.
//!
//! # Who owns a table (T5a, review finding B1)
//!
//! `bronze_table` is set only by [`mark_ingesting`], which the ingest route
//! calls after it has checked that the name is free for the caller. A row
//! that names a table is therefore a claim that was allowed, whatever became
//! of the load. [`table_claimed_by_upload`] is that rule: any row of the
//! tenant names the table, in any status, deleted or not. Without it a load
//! that failed after it had written, and an upload that was deleted, both
//! left a table nobody could load into again.
//!
//! # Lifecycle: claim, then launch (T5a, review finding B2)
//!
//! `uploaded` -> `ingesting` -> `ingested` | `failed`, and `ingested` or
//! `failed` -> `ingesting` again for the next load of the same file. Each
//! transition is an UPDATE of the same row (unlike [`crate::audit`], which
//! appends): this table answers "what is the current state of this file",
//! and the durable record of who did what and when is the audit trail's
//! job.
//!
//! The ingest route CLAIMS the row first and launches second:
//! [`mark_ingesting`] with no run id, then the launch, then [`attach_run`]
//! with the id the orchestrator returned. Marking first is what stops two
//! requests for one upload, sent at the same moment, from both launching: the
//! second finds the row already `ingesting` and is refused before it launches
//! anything. The transitions are conditional in SQL, so a handler that races
//! another cannot overwrite a load in flight or settle a load that is not the
//! one it asked about:
//!
//! - [`mark_ingesting`] refuses a row that is already `ingesting`.
//! - [`attach_run`] names a run only on a row that is `ingesting` and has none
//!   yet, and never moves `updated_at`.
//! - [`mark_finished`] settles only a row that is `ingesting` under the
//!   `run_id` the caller names (`None` names a claim that never got a run).
//! - [`soft_delete`] refuses a row that is `ingesting`.
//!
//! All four return `None` or `false` when they change nothing; the caller
//! re-reads the row to tell "gone" from "not in that state".

use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;
use sqlx::types::Json;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{PgPool, StoreError};

fn ser_ts<S: serde::Serializer>(at: &OffsetDateTime, s: S) -> Result<S::Ok, S::Error> {
    let at = at.to_offset(time::UtcOffset::UTC);
    s.serialize_str(&format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second()
    ))
}

/// How a load treats the rows a table already holds (ADR 0014, decision 5).
/// The wire form is the lowercase name, and matches the `CHECK` on
/// `file_upload.load_mode` (`0055_upload_tenant_mode.sql`). Both are names
/// the shared sink accepts (`LOAD_MODES` in
/// `dagster/dispar_orchestrate/adapters/sink.py`).
///
/// Not [`crate::ingest_spec::LoadMode`]: that one also has `incremental`,
/// which only a `sql` connector's source object can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoadMode {
    /// The file's rows replace what the table holds. The default.
    #[default]
    Replace,
    /// The file's rows are added to what the table holds.
    Append,
}

impl LoadMode {
    /// The wire and column form: `"replace"` or `"append"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Append => "append",
        }
    }

    /// Parse the wire form. Exactly the two lowercase names; anything else
    /// (another case, whitespace, an unknown mode) is `None`, so a handler
    /// refuses it instead of guessing.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "replace" => Some(Self::Replace),
            "append" => Some(Self::Append),
            _ => None,
        }
    }
}

/// One uploaded file, as the console lists and inspects it.
///
/// `storage_key`, `tenant_id`, `content_type` and `sha256` are fields the API
/// needs and the wire must not carry: the key is an internal object path, the
/// tenant is the caller's own scope, and the other two are what the console
/// has no use for (review finding B3). `#[serde(skip)]` keeps them out of
/// every response built from this type. A deleted upload is never returned
/// (see the module doc), so there is no `deleted_at` here.
#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Upload {
    /// Opaque upload id, generated by the API.
    pub id: String,
    /// The browser-reported name, for display only — never a storage path.
    pub original_filename: String,
    /// Object key in the warehouse bucket (`uploads/<tenant>/<id>.<ext>`).
    /// Internal: never serialized.
    #[serde(skip)]
    pub storage_key: String,
    /// MIME type the browser claimed. Advisory only — an export named
    /// `.xls` is routinely UTF-16 TSV, so detection wins over this. Internal:
    /// never serialized (review finding B3).
    #[serde(skip)]
    pub content_type: String,
    /// Size of the stored object, in bytes.
    pub size_bytes: i64,
    /// Hex SHA-256 of the stored bytes, for duplicate detection. Internal:
    /// never serialized (review finding B3).
    #[serde(skip)]
    pub sha256: String,
    /// Display name of the principal who uploaded it.
    pub uploaded_by: String,
    /// The tenant the upload belongs to; `None` when its tenant has been
    /// deleted (`ON DELETE SET NULL`), which makes it invisible to every
    /// tenant-scoped read. Internal: never serialized.
    #[serde(skip)]
    pub tenant_id: Option<Uuid>,
    /// `uploaded` | `ingesting` | `ingested` | `failed`.
    pub status: String,
    /// What the last load was told to use (encoding, delimiter, header
    /// row), as the API recorded it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_options: Option<Json<Value>>,
    /// Bronze table the last load targeted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bronze_table: Option<String>,
    /// `replace` | `append`: what the last load was told to do with the
    /// table's existing rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_mode: Option<String>,
    /// The row count the load reported (`row_count` column). `None` means
    /// NOT MEASURED — a load that is still running, one that failed, one
    /// whose sink reported no total — and is never `0`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[sqlx(rename = "row_count")]
    pub rows: Option<i64>,
    /// Dagster run id of the last load, for linking to its logs and for
    /// asking whether it has ended. `None` while a claim has not yet been
    /// given its run ([`attach_run`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Failure reason when `status` is `failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// When the bytes were stored.
    #[serde(serialize_with = "ser_ts")]
    pub created_at: OffsetDateTime,
    /// When the row last changed state. [`attach_run`] does not move it: for
    /// a load, this is the time of the claim.
    #[serde(serialize_with = "ser_ts")]
    pub updated_at: OffsetDateTime,
}

const COLS: &str = "id, original_filename, storage_key, content_type, size_bytes, sha256, \
                    uploaded_by, tenant_id, status, parse_options, bronze_table, load_mode, \
                    row_count, run_id, error, created_at, updated_at";

/// Everything needed to record a file whose bytes are already stored.
///
/// Deliberately taken AFTER the upload to object storage succeeds: a row
/// here means "these bytes exist at this key". Writing the row first would
/// leave a registry entry pointing at nothing whenever the store write
/// failed, and every consumer would have to treat every row as maybe-real.
#[derive(Debug, Clone)]
pub struct NewUpload<'a> {
    /// Upload id, generated by the caller before the object is stored.
    pub id: &'a str,
    /// Browser-reported filename, for display only.
    pub original_filename: &'a str,
    /// Object key the bytes were written to.
    pub storage_key: &'a str,
    /// Browser-reported MIME type; advisory (see [`Upload::content_type`]).
    pub content_type: &'a str,
    /// Size of the stored object, in bytes.
    pub size_bytes: i64,
    /// Hex SHA-256 of the stored bytes.
    pub sha256: &'a str,
    /// Display name of the uploading principal.
    pub uploaded_by: &'a str,
    /// The caller's active tenant (`tenant_scope::resolve`), never a value
    /// taken from the request body.
    pub tenant_id: Uuid,
}

/// Record an uploaded file, in the `uploaded` state.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] when `storage_key` (or `id`) is already
/// registered, [`StoreError::ForeignKeyViolation`] when `tenant_id` names no
/// tenant, and [`StoreError::Database`] on any other failure.
pub async fn insert(pool: &PgPool, new: &NewUpload<'_>) -> Result<Upload, StoreError> {
    let sql = format!(
        "INSERT INTO file_upload \
         (id, original_filename, storage_key, content_type, size_bytes, sha256, uploaded_by, \
          tenant_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING {COLS}"
    );
    Ok(sqlx::query_as(&sql)
        .bind(new.id)
        .bind(new.original_filename)
        .bind(new.storage_key)
        .bind(new.content_type)
        .bind(new.size_bytes)
        .bind(new.sha256)
        .bind(new.uploaded_by)
        .bind(new.tenant_id)
        .fetch_one(pool)
        .await?)
}

/// `tenant_id`'s live uploads, newest first. `limit` is clamped to
/// `1..=500`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on a database failure.
pub async fn list(pool: &PgPool, tenant_id: Uuid, limit: i64) -> Result<Vec<Upload>, StoreError> {
    let sql = format!(
        "SELECT {COLS} FROM file_upload WHERE tenant_id = $1 AND deleted_at IS NULL \
         ORDER BY created_at DESC, id DESC LIMIT $2"
    );
    Ok(sqlx::query_as(&sql)
        .bind(tenant_id)
        .bind(limit.clamp(1, 500))
        .fetch_all(pool)
        .await?)
}

/// One live upload by id, or `None`. NOT tenant-scoped: the caller has
/// passed [`upload_in_tenants`] for this id (see the module doc).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on a database failure.
pub async fn get(pool: &PgPool, id: &str) -> Result<Option<Upload>, StoreError> {
    let sql = format!("SELECT {COLS} FROM file_upload WHERE id = $1 AND deleted_at IS NULL");
    Ok(sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?)
}

/// Whether live upload `id` belongs to one of `tenant_ids`: the access rule
/// every `/api/uploads/{id}` route applies. `false` for an unknown id,
/// another tenant's upload, a deleted upload, an upload whose tenant was
/// deleted and an empty `tenant_ids` alike, so the answer says nothing about
/// which ids exist.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on a database failure.
pub async fn upload_in_tenants(
    pool: &PgPool,
    id: &str,
    tenant_ids: &[Uuid],
) -> Result<bool, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM file_upload \
         WHERE id = $1 AND tenant_id = ANY($2) AND deleted_at IS NULL)",
    )
    .bind(id)
    .bind(tenant_ids)
    .fetch_one(pool)
    .await?)
}

/// The most recent live upload of `tenant_id` carrying `sha256`, excluding
/// `exclude_id`.
///
/// Used to tell a user they are uploading a file their tenant already
/// holds, BEFORE its rows are added to a table a second time. Scoped to the
/// tenant on purpose: reporting another tenant's earlier upload would tell
/// the caller what other tenants hold. A deleted upload is not a duplicate:
/// its file is gone. An empty `sha256` never matches.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on a database failure.
pub async fn find_by_sha256(
    pool: &PgPool,
    tenant_id: Uuid,
    sha256: &str,
    exclude_id: &str,
) -> Result<Option<Upload>, StoreError> {
    let sql = format!(
        "SELECT {COLS} FROM file_upload \
         WHERE tenant_id = $1 AND sha256 = $2 AND sha256 <> '' AND id <> $3 \
           AND deleted_at IS NULL \
         ORDER BY created_at DESC, id DESC LIMIT 1"
    );
    Ok(sqlx::query_as(&sql)
        .bind(tenant_id)
        .bind(sha256)
        .bind(exclude_id)
        .fetch_optional(pool)
        .await?)
}

/// CLAIM an upload for a load: move it to `ingesting`, recording what the
/// load was told to do. Clears the previous attempt's `error` and row count:
/// a number or a reason from an earlier load must never read as this one's.
///
/// The ingest route calls this BEFORE it launches the job, with `run_id =
/// None`, and gives the claim its run with [`attach_run`] afterwards (review
/// finding B2: two requests sent at the same moment cannot both pass this
/// call, so only one launches).
///
/// Returns `None` when no row changed: no such live upload, or it is already
/// `ingesting` (one load per upload at a time).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on a database failure.
pub async fn mark_ingesting(
    pool: &PgPool,
    id: &str,
    parse_options: &Value,
    bronze_table: &str,
    load_mode: LoadMode,
    run_id: Option<&str>,
) -> Result<Option<Upload>, StoreError> {
    let sql = format!(
        "UPDATE file_upload SET status = 'ingesting', parse_options = $2, \
         bronze_table = $3, load_mode = $4, run_id = $5, error = NULL, row_count = NULL, \
         updated_at = now() \
         WHERE id = $1 AND status <> 'ingesting' AND deleted_at IS NULL RETURNING {COLS}"
    );
    Ok(sqlx::query_as(&sql)
        .bind(id)
        .bind(Json(parse_options.clone()))
        .bind(bronze_table)
        .bind(load_mode.as_str())
        .bind(run_id)
        .fetch_optional(pool)
        .await?)
}

/// Give a claimed upload the Dagster run that carries its load out: sets
/// `run_id` on a live row that is `ingesting` and has no run yet.
///
/// Does NOT touch `updated_at`, on purpose. For a load it is the time of the
/// claim, and the API reads the load's outcome from the newest result that
/// ended after it (`bronze_meta.ingest_run`). A run that finished quickly
/// ended BEFORE this call: moving `updated_at` here would put its end earlier
/// than the row's own timestamp and its result would be taken for an old one.
///
/// Returns `None` when no row changed: no such live upload, it is not
/// `ingesting`, or it already has a run (the claim was settled or deleted
/// while the job was being launched).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on a database failure.
pub async fn attach_run(
    pool: &PgPool,
    id: &str,
    run_id: &str,
) -> Result<Option<Upload>, StoreError> {
    let sql = format!(
        "UPDATE file_upload SET run_id = $2 \
         WHERE id = $1 AND status = 'ingesting' AND run_id IS NULL AND deleted_at IS NULL \
         RETURNING {COLS}"
    );
    Ok(sqlx::query_as(&sql)
        .bind(id)
        .bind(run_id)
        .fetch_optional(pool)
        .await?)
}

/// Settle a load: `ingested` when `error` is `None`, `failed` otherwise.
///
/// `run_id` is the run the caller is settling, as recorded by
/// [`mark_ingesting`] or [`attach_run`]; `None` names a claim that never got
/// a run (the launch failed, or the request died before [`attach_run`]). Only
/// a live row that is `ingesting` under exactly that run is touched, so a
/// reader that learned the outcome of an earlier run cannot settle the load
/// that replaced it, and two readers settling the same run cannot both write.
///
/// `row_count` is kept only for a load that succeeded, and `None` stays
/// `None` (not measured), never `0`. A failed load stores no count.
///
/// Returns `None` when no row changed: no such live upload, it is not
/// `ingesting`, or it is `ingesting` under another run.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on a database failure.
pub async fn mark_finished(
    pool: &PgPool,
    id: &str,
    run_id: Option<&str>,
    error: Option<&str>,
    row_count: Option<i64>,
) -> Result<Option<Upload>, StoreError> {
    let sql = format!(
        "UPDATE file_upload \
         SET status = CASE WHEN $3::text IS NULL THEN 'ingested' ELSE 'failed' END, \
             error = $3, \
             row_count = CASE WHEN $3::text IS NULL THEN $4::bigint ELSE NULL END, \
             updated_at = now() \
         WHERE id = $1 AND status = 'ingesting' AND run_id IS NOT DISTINCT FROM $2 \
           AND deleted_at IS NULL \
         RETURNING {COLS}"
    );
    Ok(sqlx::query_as(&sql)
        .bind(id)
        .bind(run_id)
        .bind(error)
        .bind(row_count)
        .fetch_optional(pool)
        .await?)
}

/// Delete an upload SOFTLY: set `deleted_at` and keep the row (see the module
/// doc for why). The caller deletes the upload's object first — a row
/// without bytes is a broken link, bytes without a row are an orphan only a
/// storage sweep can find. The table the upload became is not touched (ADR
/// 0014: deleting an upload keeps its table).
///
/// Refuses a row that is `ingesting`, in SQL, so a delete cannot slip in
/// between a request that has just claimed the upload and its launch.
///
/// Returns whether a live row was marked. `false` means no such live upload,
/// or it is loading; the caller re-reads the row to tell the two apart.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on a database failure.
pub async fn soft_delete(pool: &PgPool, id: &str) -> Result<bool, StoreError> {
    let done = sqlx::query(
        "UPDATE file_upload SET deleted_at = now(), updated_at = now() \
         WHERE id = $1 AND deleted_at IS NULL AND status <> 'ingesting'",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Whether an upload of `tenant_id` has claimed `table`: some row of that
/// tenant names it as its `bronze_table`, in ANY status, deleted or not. The
/// test the ingest route applies to a table that already exists — only such
/// a table may be loaded into again (ADR 0014, decision 5).
///
/// `bronze_table` is set only by [`mark_ingesting`], after the route has
/// checked that the name is free, so a row that names a table is a claim that
/// was allowed (review finding B1). Counting every status and deleted rows is
/// what keeps a table loadable after a load that failed once it had written,
/// and after the upload that made it was deleted.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on a database failure.
pub async fn table_claimed_by_upload(
    pool: &PgPool,
    tenant_id: Uuid,
    table: &str,
) -> Result<bool, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM file_upload WHERE tenant_id = $1 AND bronze_table = $2)",
    )
    .bind(tenant_id)
    .bind(table)
    .fetch_one(pool)
    .await?)
}

/// Whether an upload of ANY tenant loaded `table`: an `ingested` row names
/// it, deleted or not. Raw table names are shared by every tenant, and the
/// check it serves has no tenant to ask on behalf of: a connector's ingest
/// spec may not take a table an upload loaded (plan task T8).
///
/// Only `ingested` counts: a load that is still running or that failed has
/// not loaded the table, as far as this record can say.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on a database failure.
pub async fn table_loaded_by_upload(pool: &PgPool, table: &str) -> Result<bool, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM file_upload \
         WHERE bronze_table = $1 AND status = 'ingested')",
    )
    .bind(table)
    .fetch_one(pool)
    .await?)
}

/// Whether another live upload (any tenant: table names are shared) is
/// `ingesting` into `table`. `exclude_id` is the upload asking, so its own
/// load never blocks itself.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on a database failure.
pub async fn table_being_loaded(
    pool: &PgPool,
    table: &str,
    exclude_id: &str,
) -> Result<bool, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM file_upload \
         WHERE bronze_table = $1 AND status = 'ingesting' AND id <> $2 AND deleted_at IS NULL)",
    )
    .bind(table)
    .bind(exclude_id)
    .fetch_one(pool)
    .await?)
}
