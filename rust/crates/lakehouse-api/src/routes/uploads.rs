//! `/api/uploads/*` — files a user uploads through the console, and the load
//! of each one into a raw table. T6 of
//! `docs/superpowers/plans/2026-10-02-upload-file.md`, under ADR 0014.
//!
//! Until this existed, raw data could only enter the lakehouse through a
//! database connection an operator configured in `.env`. A file exported
//! from an ERP, a spreadsheet or a vendor portal had no way in: someone had
//! to convert it and load it by hand, on the server.
//!
//! # The flow
//!
//! 1. `POST /api/uploads` stores the bytes (object first, row second: a row
//!    that exists means its object exists) and registers the file.
//! 2. `GET /api/uploads/{id}/preview` reports what the file LOOKS like —
//!    encoding, delimiter, header row, first rows — as a proposal the user can
//!    correct. Detection is not reliable enough to be silent: the file that
//!    motivated the feature was named `.xls`, was UTF-16 tab-separated text and
//!    carried six report lines above its real header (ADR 0014, decision 3).
//! 3. `POST /api/uploads/{id}/ingest` hands the file to `file_ingest_job` with
//!    what the user confirmed. The job writes Bronze through the shared sink.
//!    This module never writes Iceberg.
//! 4. `GET /api/uploads` and `GET /api/uploads/{id}` report the outcome. The
//!    job writes nothing to Postgres (ADR 0014, decision 5): the API reads its
//!    result back from `lake.bronze_meta.ingest_run` and settles the row when
//!    it is read ([`Settler`]).
//! 5. `DELETE /api/uploads/{id}` removes the object and soft-deletes the row.
//!    The table the upload became stays.
//!
//! # Posture
//!
//! * **Tenant.** An upload belongs to the uploader's active tenant
//!   (`tenant_scope::resolve`, never a value from the body). Every
//!   `/api/uploads/{id}*` route sits behind [`require_upload_in_tenants`],
//!   mounted by `routes::uploads_router`, and answers the same 404 for an
//!   unknown id and for another tenant's upload.
//! * **No upstream text in a response.** Storage, orchestrator and
//!   `ClickHouse` errors are logged and answered with a fixed sentence
//!   written here (or, for storage, in [`crate::upload_store`]); a database
//!   error reaches a response only as the store's own fixed `database error`.
//!   The one piece of recorded text a response carries is the reason
//!   `file_ingest_job` itself wrote into `ingest_run` (plan T7 fixes its
//!   wording).
//! * **Fail closed on a table name.** A load may target a table that does not
//!   exist or one an upload of the same tenant claimed; never a connector's
//!   table, never anything else. When the API cannot tell, it answers 503 and
//!   does not guess that the name is free ([`ensure_table_free`]).
//! * **One load per upload, and the row is claimed before the job is
//!   launched** (review finding B2): see [`ingest`].
//!
//! # Statuses
//!
//! Every refusal of the upload itself (no part, empty, too large, a workbook
//! or another binary) is a 400 with a fixed sentence; `lakehouse_core::
//! ApiError` has no 413 and this change does not add one.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::Duration;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::multipart::{Multipart, MultipartError, MultipartRejection};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use lakehouse_auth::Principal;
use lakehouse_clickhouse::ChError;
use lakehouse_core::ApiError;
use lakehouse_core::ident::Ident;
use lakehouse_dagster::map_run_status;
use lakehouse_store::PgPool;
use lakehouse_store::audit::{self as store_audit, NewAuditEvent};
use lakehouse_store::connectors;
use lakehouse_store::uploads::{self, LoadMode, NewUpload, Upload};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use super::catalog::registered_slug;
use super::catalog_source::{PresenceUnknown, iceberg_table_presence};
use super::governance::{IngestRunRow, ingest_runs_or_empty};
use super::lakehouse::is_unknown_table_error;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::upload_parse::{self, Encoding, Kind, Overrides, Preview};
use crate::upload_store::{PREFIX, UploadStore};

/// Largest file this endpoint accepts, in bytes.
///
/// The body is buffered in memory to hash and store it, so this is a memory
/// bound as much as a product limit: ten concurrent uploads at the cap is
/// 500 MB of API process (ADR 0014, decision 2). The message says "50 MB"
/// and means 50 MiB, as the plan words it.
pub(super) const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;

/// What `DefaultBodyLimit` allows the `POST /api/uploads` request as a whole:
/// the cap plus 1 MiB for the multipart framing around the file. Applied to
/// that route only (`routes::uploads_router`); every other route keeps
/// axum's default 2 MB.
pub(super) const MAX_REQUEST_BODY_BYTES: usize = MAX_UPLOAD_BYTES + 1024 * 1024;

/// How much of a file the preview reads: enough to see an encoding marker, a
/// delimiter and a few dozen rows of a wide export, and the same cost for a
/// 5 GB file as for a 5 KB one.
const PREVIEW_BYTES: usize = 256 * 1024;

/// Rows the preview returns.
const PREVIEW_ROWS: usize = 20;

/// Uploads one `GET /api/uploads` returns, newest first. There is no paging:
/// an older upload is not listed.
const LIST_LIMIT: i64 = 100;

/// A claim ([`uploads::mark_ingesting`]) that has no run after this long was
/// never launched: the request died between the claim and the launch. The
/// request deadline for `/api/uploads/{id}/ingest` is 60 s, so two minutes is
/// past any request that could still be about to attach a run.
const STALE_CLAIM: time::Duration = time::Duration::minutes(2);

/// How long a read waits for the orchestrator or for `ClickHouse` while it
/// settles a loading upload. A bound so that a page that polls is not held
/// for the whole request deadline by a hung connection; not a measurement.
/// The upload is returned unchanged when it elapses.
const SETTLE_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// The Dagster job that loads an uploaded file
/// (`dagster/dispar_orchestrate/file_ingest.py`).
const FILE_INGEST_JOB: &str = "file_ingest_job";

/// The longest raw table name accepted, in characters. The console's rule has
/// no bound; this one keeps a name that is a path segment of the table's
/// object keys well inside S3's 1,024-byte key limit.
const MAX_TABLE_NAME_CHARS: usize = 128;

/// The longest file name or content type stored, in characters.
const MAX_STORED_TEXT_CHARS: usize = 255;

// ── Messages. Every one is fixed text; none carries upstream text. ───────

const NOT_FOUND: &str = "Upload not found.";
const NO_TENANT: &str = "Your account belongs to no tenant, so a file cannot be uploaded.";
const NOT_MULTIPART: &str = "The request must be a multipart form with one part named file.";
const NO_FILE_PART: &str = "The form has no part named file.";
const UNREADABLE: &str = "The upload could not be read.";
const EMPTY_FILE: &str = "The file is empty.";
const TOO_LARGE: &str = "The file is larger than the 50 MB limit.";
const WORKBOOK: &str = "This looks like an Excel workbook or a zip archive. Only delimited text files (CSV, TSV) can be uploaded; save the sheet as CSV first.";
const PARQUET: &str =
    "This looks like a Parquet file. Only delimited text files (CSV, TSV) can be uploaded.";
const OTHER_BINARY: &str =
    "This is not a delimited text file. Only delimited text files (CSV, TSV) can be uploaded.";
const BODY_NOT_AN_OBJECT: &str = "The request body must be a JSON object.";
const TABLE_NAME_RULE: &str = "Table names use lower-case letters, digits and _ only, do not start with a digit, and have at most 128 characters.";
const MODE_RULE: &str = "mode must be replace or append.";
const ENCODING_RULE: &str = "encoding must be utf-8 or utf-16.";
const DELIMITER_RULE: &str = "delimiter must be a comma, a semicolon, a tab or a pipe.";
const HEADER_ROW_RULE: &str = "headerRow must be a whole number, 0 or more.";
const QUERY_UNREADABLE: &str = "The query string could not be read.";
const ALREADY_LOADING: &str = "This upload is already being loaded.";
const TABLE_BUSY: &str = "Another upload is loading into that table.";
const CONNECTOR_TABLE: &str = "A connector loads that table, so a file cannot be loaded into it.";
const TABLE_NOT_FREE: &str = "That table already exists and no upload of this tenant created it, so a file cannot be loaded into it.";
const TABLE_UNCHECKED: &str =
    "Could not check whether that table already exists, so nothing was loaded.";
const NO_QUERY_DATABASE: &str = "Uploads need ICEBERG_QUERY_DB to be set, so the API can check that a table name is free. Nothing was loaded.";
const LAUNCH_UNREACHABLE: &str =
    "The orchestrator could not be reached, so the load was not started.";
const LAUNCH_REFUSED: &str = "The orchestrator refused to start the load.";
const NOT_RECORDED: &str = "The load was started but could not be recorded.";
const DELETE_WHILE_LOADING: &str = "This upload is being loaded, so it cannot be deleted yet.";

// What the row records when a load ends without the job's own reason.

/// The orchestrator refused or could not be reached at launch.
const COULD_NOT_START: &str = "The load could not be started.";
/// A claim older than [`STALE_CLAIM`] that never got a run.
const NOT_STARTED: &str = "The load was not started.";
/// The run ended and recorded nothing.
const NO_RESULT: &str = "The load stopped before it recorded a result.";
/// The run recorded a failure with no reason.
const LOAD_FAILED: &str = "The load failed.";

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "upload store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

fn not_found() -> ApiError {
    ApiError::NotFound(NOT_FOUND.to_owned())
}

// ── Tenant access ────────────────────────────────────────────────────────

/// The access rule of every `/api/uploads/{id}*` route: the caller must
/// belong to the upload's tenant. `POLICY_TABLE` only says what a caller may
/// do to uploads in general; without this, `connector:manage` in one tenant
/// would reach every tenant's files by id.
///
/// Refused with the same 404 an unknown id (or a deleted upload) gets, so the
/// answer is no oracle for which ids exist in other tenants. Shaped like
/// `routes::connectors::ensure_connector_in_tenants`, over
/// [`uploads::upload_in_tenants`].
///
/// # Errors
///
/// 404 as above; 401 with no principal; 503 if no pool is configured; 500 on
/// a database failure.
async fn ensure_upload_in_tenants(
    state: &AppState,
    principal: Option<&Principal>,
    id: &str,
) -> Result<(), ApiError> {
    let Some(principal) = principal else {
        return Err(ApiError::unauthorized());
    };
    if uploads::upload_in_tenants(pool(state)?, id, &principal.tenant_ids).await? {
        Ok(())
    } else {
        Err(not_found())
    }
}

/// [`ensure_upload_in_tenants`] as the route layer of every
/// `/api/uploads/{id}*` route (`routes::uploads_router`), so no per-upload
/// route can be added without it.
///
/// # Errors
///
/// As [`ensure_upload_in_tenants`].
pub(super) async fn require_upload_in_tenants(
    State(state): State<AppState>,
    Path(params): Path<HashMap<String, String>>,
    request: Request,
    next: Next,
) -> ApiResult<Response> {
    let Some(id) = params.get("id") else {
        return Err(not_found().into());
    };
    ensure_upload_in_tenants(&state, request.extensions().get::<Principal>(), id).await?;
    Ok(next.run(request).await)
}

// ── What a response carries ──────────────────────────────────────────────

/// An upload as a response carries it: the store's [`Upload`] (which hides
/// the object key, the tenant, the content type and the checksum) and, once
/// it is loaded, the catalog slug of its table.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadView {
    #[serde(flatten)]
    upload: Upload,
    /// The slug the load registered its table under: the table name with `_`
    /// as `-` (`connector_catalog.register_connector_table` does the same
    /// for a connector's table). Only when the upload is `ingested`.
    #[serde(skip_serializing_if = "Option::is_none")]
    asset_id: Option<String>,
}

impl From<Upload> for UploadView {
    fn from(upload: Upload) -> Self {
        let asset_id = match (upload.status.as_str(), upload.bronze_table.as_deref()) {
            ("ingested", Some(table)) => Some(table.replace('_', "-")),
            _ => None,
        };
        Self { upload, asset_id }
    }
}

/// `POST /api/uploads`'s response: the new upload and, when this tenant
/// already holds the same bytes, the earlier upload.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateResponse {
    #[serde(flatten)]
    upload: UploadView,
    /// The most recent live upload of this tenant with the same checksum. A
    /// duplicate is reported, never refused: the same file legitimately
    /// arrives twice (a corrected re-export), and the user decides, knowing
    /// that loading it again with "add" doubles its rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    duplicate_of: Option<UploadView>,
}

/// `POST /api/uploads/{id}/ingest`'s response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestResponse {
    upload: UploadView,
    /// The Dagster run carrying the load out.
    run_id: String,
}

// ── Creating an upload ───────────────────────────────────────────────────

/// The extension of `filename`, lowercased, without a dot — `""` when the
/// name has none.
///
/// Used only to give the stored object a recognisable suffix. It is NOT
/// trusted to decide how the file is parsed: see this module's header.
fn extension_of(filename: &str) -> String {
    filename
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| {
            !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// The object key an upload is stored at.
///
/// Built entirely from server-controlled parts — tenant and a fresh
/// UUID — with the user's filename contributing at most a sanitised
/// extension. A key derived from a caller-supplied name is how `../`
/// escapes, 2 KB paths and cross-user collisions happen.
fn storage_key(tenant: &str, id: &str, filename: &str) -> String {
    let ext = extension_of(filename);
    let tenant = tenant
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    if ext.is_empty() {
        format!("{PREFIX}/{tenant}/{id}")
    } else {
        format!("{PREFIX}/{tenant}/{id}.{ext}")
    }
}

/// A file name fit to store and show: the last path segment (an old browser
/// sends the whole path), no control characters, at most
/// [`MAX_STORED_TEXT_CHARS`] characters, `unnamed` when nothing is left. It is
/// for display only and never becomes a path ([`storage_key`]).
fn display_name(raw: Option<&str>) -> String {
    let last_segment = raw
        .unwrap_or_default()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default();
    let name: String = last_segment
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_STORED_TEXT_CHARS)
        .collect();
    let name = name.trim();
    if name.is_empty() {
        "unnamed".to_owned()
    } else {
        name.to_owned()
    }
}

/// Lower-case hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The part named `file` of an upload form.
struct FilePart {
    filename: String,
    content_type: String,
    bytes: Bytes,
}

/// A form that could not be read to its end. A body past the request limit
/// is the one case with its own sentence; the rest of `multer`'s detail is
/// logged.
fn refused_read(err: &MultipartError) -> ApiError {
    if err.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return ApiError::BadRequest(TOO_LARGE.to_owned());
    }
    tracing::warn!(%err, "an upload form could not be read");
    ApiError::BadRequest(UNREADABLE.to_owned())
}

/// The first part named `file`, read whole. Other parts are skipped, unread.
async fn read_file_part(mut form: Multipart) -> Result<FilePart, ApiError> {
    while let Some(field) = form.next_field().await.map_err(|err| refused_read(&err))? {
        if field.name() != Some("file") {
            continue;
        }
        let filename = display_name(field.file_name());
        let content_type: String = field
            .content_type()
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_STORED_TEXT_CHARS)
            .collect();
        let bytes = field.bytes().await.map_err(|err| refused_read(&err))?;
        return Ok(FilePart {
            filename,
            content_type,
            bytes,
        });
    }
    Err(ApiError::BadRequest(NO_FILE_PART.to_owned()))
}

/// Whether the bytes of an upload are acceptable: not empty, within the cap,
/// and delimited text by their first bytes (never by the name or the type the
/// browser claimed: the file that motivated this was called `.xls`).
fn check_file(bytes: &[u8]) -> Result<(), ApiError> {
    if bytes.is_empty() {
        return Err(ApiError::BadRequest(EMPTY_FILE.to_owned()));
    }
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(ApiError::BadRequest(TOO_LARGE.to_owned()));
    }
    let message = match upload_parse::sniff(bytes) {
        Kind::DelimitedText => return Ok(()),
        Kind::Workbook => WORKBOOK,
        Kind::Parquet => PARQUET,
        Kind::OtherBinary => OTHER_BINARY,
    };
    Err(ApiError::BadRequest(message.to_owned()))
}

/// `POST /api/uploads` — one multipart part named `file`. 201 with the
/// upload (see [`UploadView`]) and, when this tenant already holds the same
/// bytes, `duplicateOf`.
///
/// The order is chosen so that nothing is stored for a request that will be
/// refused, and nothing is read for one that cannot be served: tenant, pool
/// and storage first, then the form, then the checks, then the bytes, then the
/// row. A row whose insert fails takes its object back out.
///
/// # Errors
///
/// 400 for a caller in no tenant, a request that is not a multipart form, no
/// `file` part, an empty file, a file over [`MAX_UPLOAD_BYTES`], and a
/// workbook, Parquet file or other binary (each with its own fixed
/// sentence); 404 if `X-Tenant` names a tenant the caller does not belong to;
/// 503 when Postgres or the object store is unavailable.
pub async fn create(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
    form: Result<Multipart, MultipartRejection>,
) -> ApiResult<(StatusCode, ApiJson<CreateResponse>)> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let Some(tenant_id) = crate::tenant_scope::resolve(&principal, &headers)? else {
        return Err(ApiError::BadRequest(NO_TENANT.to_owned()).into());
    };
    let pool = pool(&state)?;
    let store = UploadStore::connect(&state.config).await?;
    let form = form.map_err(|err| {
        tracing::warn!(%err, "an upload request was not a readable multipart form");
        ApiError::BadRequest(NOT_MULTIPART.to_owned())
    })?;
    let part = read_file_part(form).await?;
    check_file(&part.bytes)?;

    let id = format!("up-{}", Uuid::new_v4());
    let key = storage_key(&tenant_id.to_string(), &id, &part.filename);
    let digest = sha256_hex(&part.bytes);
    let size = i64::try_from(part.bytes.len()).unwrap_or(i64::MAX);

    // Bytes first, row second: a row that exists means its object exists. The
    // reverse order leaves rows pointing at nothing whenever the store write
    // fails.
    store.put(&key, part.bytes).await?;
    let row = match uploads::insert(
        pool,
        &NewUpload {
            id: &id,
            original_filename: &part.filename,
            storage_key: &key,
            content_type: &part.content_type,
            size_bytes: size,
            sha256: &digest,
            uploaded_by: &principal.display_name,
            tenant_id,
        },
    )
    .await
    {
        Ok(row) => row,
        Err(err) => {
            if let Err(cleanup) = store.delete(&key).await {
                tracing::warn!(%cleanup, key = %key, "an upload object could not be removed after its row failed");
            }
            return Err(err.into());
        }
    };

    // Best effort: the upload exists, so a failed lookup costs the notice and
    // nothing else.
    let duplicate = match uploads::find_by_sha256(pool, tenant_id, &digest, &id).await {
        Ok(found) => found,
        Err(err) => {
            tracing::warn!(%err, upload_id = %id, "the duplicate lookup for an upload failed");
            None
        }
    };

    let event = upload_audit_event(
        &principal,
        "upload.create",
        &id,
        json!({ "fileName": row.original_filename, "sizeBytes": row.size_bytes }),
        "executed",
    );
    record_audit(pool, event).await;
    Ok((
        StatusCode::CREATED,
        ApiJson(CreateResponse {
            upload: row.into(),
            duplicate_of: duplicate.map(Into::into),
        }),
    ))
}

// ── Reading uploads, and settling the ones that are loading ─────────────

/// What a load that ended is recorded to have done.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// `succeeded`, with the row count the job reported (`None`: not
    /// measured, never 0).
    Ingested { rows: Option<i64> },
    /// Anything else, with the reason to show.
    Failed(String),
}

/// What looking for a load's recorded result found.
enum Recorded {
    Found(Outcome),
    /// Nothing recorded after the claim.
    Missing,
    /// `ClickHouse` could not be asked.
    Unreadable,
}

/// Whether the load a result row describes ended after `since`. A result
/// whose time cannot be read does not count.
///
/// Both times are UTC clocks on the same host in the compose stack
/// (`datetime.now(timezone.utc)` in the job, `now()` in Postgres); across
/// hosts this assumes their clocks agree to better than a load takes.
fn ended_after(ended_at: &str, since: OffsetDateTime) -> bool {
    OffsetDateTime::parse(ended_at, &Rfc3339).is_ok_and(|ended| ended > since)
}

/// What a recorded result means for the upload.
fn outcome_of(result: &IngestRunRow) -> Outcome {
    if result.status == "succeeded" {
        return Outcome::Ingested {
            rows: result.rows.and_then(|rows| i64::try_from(rows).ok()),
        };
    }
    let reason = result.error.trim();
    Outcome::Failed(
        if reason.is_empty() {
            LOAD_FAILED
        } else {
            reason
        }
        .to_owned(),
    )
}

/// Whether a claim with no run is old enough to have been abandoned.
fn claim_is_stale(updated_at: OffsetDateTime, now: OffsetDateTime) -> bool {
    now - updated_at > STALE_CLAIM
}

/// Settles an upload that is `ingesting` when it is read, since nothing else
/// writes the outcome of a load into Postgres (ADR 0014, decision 5). One
/// per request, so that a list stops asking the orchestrator, and stops
/// reading results, once either has failed to answer.
///
/// - A claim with no run, older than [`STALE_CLAIM`], becomes `failed`
///   ([`NOT_STARTED`]).
/// - A claim with a run: ask Dagster. Queued or running: leave it. Ended:
///   take the newest `ingest_run` result for `upload:<id>` that ended after
///   the claim; `succeeded` gives `ingested` with its rows, anything else
///   `failed` with its reason, and no result gives `failed`
///   ([`NO_RESULT`]).
/// - When Dagster or `ClickHouse` cannot be asked, or Dagster does not know
///   the run, the row is returned unchanged: that is a doubt, not an
///   outcome. The cost is that a run Dagster has lost (its run storage was
///   reset) leaves the upload loading, and so neither deletable nor loadable
///   again, until Dagster knows the run again or the row is fixed by hand.
///   Settling it as failed instead would also settle a run Dagster merely
///   failed to answer for, because `pipeline_run_status` reads a GraphQL error
///   as "no such run".
struct Settler<'a> {
    state: &'a AppState,
    pool: &'a PgPool,
    orchestrator_unreachable: bool,
    results_unreadable: bool,
}

impl<'a> Settler<'a> {
    fn new(state: &'a AppState, pool: &'a PgPool) -> Self {
        Self {
            state,
            pool,
            orchestrator_unreachable: false,
            results_unreadable: false,
        }
    }

    async fn settle(&mut self, row: Upload) -> Upload {
        if row.status != "ingesting" {
            return row;
        }
        let Some(run_id) = row.run_id.clone() else {
            return if claim_is_stale(row.updated_at, OffsetDateTime::now_utc()) {
                self.record(row, None, &Outcome::Failed(NOT_STARTED.to_owned()))
                    .await
            } else {
                row
            };
        };
        if self.orchestrator_unreachable {
            return row;
        }
        let ended = match tokio::time::timeout(
            SETTLE_CALL_TIMEOUT,
            self.state.dagster.pipeline_run_status(&run_id),
        )
        .await
        {
            Ok(Ok(Some(info))) => {
                matches!(
                    map_run_status(&info.status),
                    "completed" | "failed" | "cancelled"
                )
            }
            Ok(Ok(None)) => {
                tracing::warn!(upload_id = %row.id, %run_id, "Dagster does not know the run of a loading upload");
                return row;
            }
            Ok(Err(err)) => {
                tracing::warn!(%err, upload_id = %row.id, "the orchestrator could not be asked about a loading upload");
                self.orchestrator_unreachable = true;
                return row;
            }
            Err(_elapsed) => {
                tracing::warn!(upload_id = %row.id, "the orchestrator did not answer about a loading upload in time");
                self.orchestrator_unreachable = true;
                return row;
            }
        };
        if !ended {
            return row;
        }
        if self.results_unreadable {
            return row;
        }
        let outcome = match self.recorded(&row).await {
            Recorded::Found(outcome) => outcome,
            Recorded::Missing => Outcome::Failed(NO_RESULT.to_owned()),
            Recorded::Unreadable => {
                self.results_unreadable = true;
                return row;
            }
        };
        self.record(row, Some(&run_id), &outcome).await
    }

    /// The newest result the load of `row` recorded, if it ended after the
    /// claim (`updated_at` is the claim's time; see [`uploads::attach_run`]).
    async fn recorded(&self, row: &Upload) -> Recorded {
        let results = match tokio::time::timeout(
            SETTLE_CALL_TIMEOUT,
            ingest_runs_or_empty(&self.state.clickhouse, &format!("upload:{}", row.id)),
        )
        .await
        {
            Ok(Ok(results)) => results,
            Ok(Err(err)) => {
                tracing::warn!(%err, upload_id = %row.id, "the recorded result of a load could not be read");
                return Recorded::Unreadable;
            }
            Err(_elapsed) => {
                tracing::warn!(upload_id = %row.id, "the recorded result of a load was not read in time");
                return Recorded::Unreadable;
            }
        };
        results
            .iter()
            .find(|result| ended_after(&result.ended_at, row.updated_at))
            .map_or(Recorded::Missing, |result| {
                Recorded::Found(outcome_of(result))
            })
    }

    /// Write `outcome` under `run_id`. A reader that lost the race, or lost
    /// the row to a delete, returns whatever is there now.
    async fn record(&self, row: Upload, run_id: Option<&str>, outcome: &Outcome) -> Upload {
        let settled = match outcome {
            Outcome::Ingested { rows } => {
                uploads::mark_finished(self.pool, &row.id, run_id, None, *rows).await
            }
            Outcome::Failed(reason) => {
                uploads::mark_finished(self.pool, &row.id, run_id, Some(reason), None).await
            }
        };
        match settled {
            Ok(Some(updated)) => updated,
            Ok(None) => uploads::get(self.pool, &row.id)
                .await
                .ok()
                .flatten()
                .unwrap_or(row),
            Err(err) => {
                tracing::warn!(%err, upload_id = %row.id, "a settled load could not be written");
                row
            }
        }
    }
}

/// `GET /api/uploads` — the active tenant's uploads, newest first, at most
/// [`LIST_LIMIT`]. A caller in no tenant gets an empty list, never another
/// tenant's. A loading upload is settled first.
///
/// # Errors
///
/// 404 if `X-Tenant` names a tenant the caller does not belong to; 503 if no
/// pool is configured; 500 on a database failure.
pub async fn list(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Vec<UploadView>>> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let Some(tenant_id) = crate::tenant_scope::resolve(&principal, &headers)? else {
        return Ok(ApiJson(Vec::new()));
    };
    let pool = pool(&state)?;
    let rows = uploads::list(pool, tenant_id, LIST_LIMIT).await?;
    let mut settler = Settler::new(&state, pool);
    let mut views = Vec::with_capacity(rows.len());
    for row in rows {
        views.push(settler.settle(row).await.into());
    }
    Ok(ApiJson(views))
}

/// `GET /api/uploads/{id}` — one upload, settled first if it is loading.
///
/// # Errors
///
/// 404 for an unknown upload or another tenant's; 503/500 as above.
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<UploadView>> {
    let pool = pool(&state)?;
    let row = uploads::get(pool, &id).await?.ok_or_else(not_found)?;
    Ok(ApiJson(Settler::new(&state, pool).settle(row).await.into()))
}

// ── Preview ──────────────────────────────────────────────────────────────

/// Query parameters of [`preview`]. Text, parsed by hand, so that a bad value
/// gets one of this module's fixed sentences and not the extractor's.
#[derive(Debug, Deserialize)]
pub struct PreviewQuery {
    /// Read the file as this encoding instead of the detected one.
    encoding: Option<String>,
    /// Split on this delimiter instead of the detected one.
    delimiter: Option<String>,
    /// Zero-based index of the record that holds the column names.
    #[serde(rename = "headerRow")]
    header_row: Option<String>,
}

/// The overrides a preview request asks for. A parameter that is given must
/// be valid: an empty or unknown value is refused, never read as "detect".
fn overrides_of(query: &PreviewQuery) -> Result<Overrides, ApiError> {
    let bad = |message: &str| ApiError::BadRequest(message.to_owned());
    Ok(Overrides {
        encoding: query
            .encoding
            .as_deref()
            .map(|text| Encoding::parse(text).ok_or_else(|| bad(ENCODING_RULE)))
            .transpose()?,
        delimiter: query
            .delimiter
            .as_deref()
            .map(|text| upload_parse::parse_delimiter(text).ok_or_else(|| bad(DELIMITER_RULE)))
            .transpose()?,
        header_row: query
            .header_row
            .as_deref()
            .map(|text| text.parse::<usize>().map_err(|_err| bad(HEADER_ROW_RULE)))
            .transpose()?,
    })
}

/// `GET /api/uploads/{id}/preview?encoding=&delimiter=&headerRow=` — what the
/// file appears to contain: `detected`, `using`, `columns`, at most 20
/// `rows`, and `truncated`.
///
/// Reads only the first [`PREVIEW_BYTES`] of the object (a range read), so
/// the cost does not grow with the file. Every detected value can be
/// overridden; whatever the user settles on is what [`ingest`] is handed.
///
/// # Errors
///
/// 400 for an `encoding`, `delimiter` or `headerRow` that is not valid; 404
/// for an unknown upload or another tenant's; 503 when Postgres or the object
/// store is unavailable.
pub async fn preview(
    State(state): State<AppState>,
    Path(id): Path<String>,
    query: Result<Query<PreviewQuery>, axum::extract::rejection::QueryRejection>,
) -> ApiResult<ApiJson<Preview>> {
    let Query(query) = query.map_err(|err| {
        tracing::warn!(%err, "a preview query string could not be read");
        ApiError::BadRequest(QUERY_UNREADABLE.to_owned())
    })?;
    let overrides = overrides_of(&query)?;
    let row = uploads::get(pool(&state)?, &id)
        .await?
        .ok_or_else(not_found)?;
    let store = UploadStore::connect(&state.config).await?;
    let head = store.head_bytes(&row.storage_key, PREVIEW_BYTES).await?;
    // A preview reads one chunk, so "20 rows" never implies the file has only
    // 20: `truncated` says the file goes on.
    let head_is_truncated = row.size_bytes > i64::try_from(PREVIEW_BYTES).unwrap_or(i64::MAX);
    Ok(ApiJson(upload_parse::preview(
        &head,
        head_is_truncated,
        overrides,
        PREVIEW_ROWS,
    )))
}

// ── Ingest ───────────────────────────────────────────────────────────────

/// What the user confirmed, validated.
#[derive(Debug, PartialEq, Eq)]
struct IngestRequest {
    table: String,
    mode: LoadMode,
    encoding: Encoding,
    delimiter: char,
    header_row: u64,
}

/// Why a raw table name is refused, or `None`: the rule the console's
/// `targetProblem` applies (`^[a-z_][a-z0-9_]*$`), plus a length bound. The
/// shared [`Ident`] is the lexical check; lower-case only is the rule for a
/// raw table, which the name is not folded into: `Orders` is refused, not
/// loaded as `orders`.
fn table_name_problem(name: &str) -> Option<&'static str> {
    let plain = Ident::new(name).is_ok() && !name.chars().any(|c| c.is_ascii_uppercase());
    (!plain || name.len() > MAX_TABLE_NAME_CHARS).then_some(TABLE_NAME_RULE)
}

fn text_field<'a>(fields: &'a Map<String, Value>, name: &str) -> Result<&'a str, ApiError> {
    match fields.get(name) {
        Some(Value::String(text)) => Ok(text),
        None | Some(Value::Null) => Err(ApiError::BadRequest(format!("{name} is required."))),
        Some(_) => Err(ApiError::BadRequest(format!("{name} must be text."))),
    }
}

/// Validate the body of `POST /api/uploads/{id}/ingest`: `bronzeTable`,
/// `mode` (optional, `replace` by default), `encoding`, `delimiter` and
/// `headerRow`. All five are checked; the load uses exactly what was
/// confirmed and nothing is guessed (ADR 0014, decision 3).
fn parse_ingest_request(body: &[u8]) -> Result<IngestRequest, ApiError> {
    let bad = |message: &str| ApiError::BadRequest(message.to_owned());
    let Ok(Value::Object(fields)) = serde_json::from_slice::<Value>(body) else {
        return Err(bad(BODY_NOT_AN_OBJECT));
    };
    let table = text_field(&fields, "bronzeTable")?;
    if let Some(problem) = table_name_problem(table) {
        return Err(bad(problem));
    }
    let mode = match fields.get("mode") {
        None | Some(Value::Null) => LoadMode::default(),
        Some(Value::String(text)) => LoadMode::parse(text).ok_or_else(|| bad(MODE_RULE))?,
        Some(_) => return Err(bad(MODE_RULE)),
    };
    let encoding =
        Encoding::parse(text_field(&fields, "encoding")?).ok_or_else(|| bad(ENCODING_RULE))?;
    let delimiter = upload_parse::parse_delimiter(text_field(&fields, "delimiter")?)
        .ok_or_else(|| bad(DELIMITER_RULE))?;
    let header_row = match fields.get("headerRow") {
        None | Some(Value::Null) => return Err(bad("headerRow is required.")),
        Some(value) => value.as_u64().ok_or_else(|| bad(HEADER_ROW_RULE))?,
    };
    Ok(IngestRequest {
        table: table.to_owned(),
        mode,
        encoding,
        delimiter,
        header_row,
    })
}

/// Whether `table` may be loaded into by an upload of `tenant_id`, or why
/// not. The rule (ADR 0014, decision 5): never a table a connector loads;
/// otherwise a table an upload of this tenant claimed, or one that does not
/// exist. Never "free" on a doubt.
///
/// # Errors
///
/// 409 for a connector's table and for an existing table no upload of this
/// tenant claimed; 503 when the question cannot be answered (Postgres,
/// `ClickHouse` or the Iceberg query database did not give a definite
/// answer, or there is no Iceberg query database to ask).
async fn ensure_table_free(
    state: &AppState,
    pool: &PgPool,
    tenant_id: Uuid,
    table: &str,
) -> Result<(), ApiError> {
    let unchecked = |err: &dyn std::fmt::Display| {
        tracing::warn!(%err, table, "could not check whether a raw table name is free");
        ApiError::Unavailable(TABLE_UNCHECKED.to_owned())
    };
    if connectors::any_connector_targets(pool, table)
        .await
        .map_err(|err| unchecked(&err))?
    {
        return Err(ApiError::Conflict(CONNECTOR_TABLE.to_owned()));
    }
    // A claim by this tenant's own upload settles it, whether or not the
    // table exists yet: replacing or adding to one's own table is the point.
    if uploads::table_claimed_by_upload(pool, tenant_id, table)
        .await
        .map_err(|err| unchecked(&err))?
    {
        return Ok(());
    }
    match table_exists(state, table).await {
        Ok(false) => Ok(()),
        Ok(true) => Err(ApiError::Conflict(TABLE_NOT_FREE.to_owned())),
        Err(PresenceUnknown::NoQueryDatabase) => {
            Err(ApiError::Unavailable(NO_QUERY_DATABASE.to_owned()))
        }
        Err(PresenceUnknown::Unanswered) => Err(ApiError::Unavailable(TABLE_UNCHECKED.to_owned())),
    }
}

/// Whether a raw table named `table` exists: registered in the catalog (the
/// registry both Bronze catalogs share), or present as an Iceberg table.
async fn table_exists(state: &AppState, table: &str) -> Result<bool, PresenceUnknown> {
    match registered_slug(&state.clickhouse, table).await {
        Ok(Some(_)) => return Ok(true),
        Ok(None) => {}
        // No registry table yet means nothing has ever been registered.
        Err(ChError::Server(body)) if is_unknown_table_error(&body) => {}
        Err(err) => {
            tracing::warn!(%err, table, "the catalog registry could not be asked about a table name");
            return Err(PresenceUnknown::Unanswered);
        }
    }
    iceberg_table_presence(state, table).await
}

/// The audit record of one upload action, mirroring `routes::connectors::
/// connector_audit_event`: `resource_kind` is `upload`, paired with the
/// upload's own id.
///
/// `principal_kind` comes from [`Principal::kind_for_audit`]. `args` carries
/// the file name, size, table and mode, never a row of the file.
fn upload_audit_event(
    principal: &Principal,
    action: &str,
    upload_id: &str,
    args: Value,
    outcome: &str,
) -> NewAuditEvent {
    NewAuditEvent {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        action: action.to_owned(),
        resource_kind: Some("upload".to_owned()),
        resource_id: Some(upload_id.to_owned()),
        args: Some(args),
        outcome: outcome.to_owned(),
        detail: None,
        run_id: None,
        approval_id: None,
        session_id: None,
    }
}

/// Best effort: a failed audit write never fails, or undoes, the action it
/// records.
async fn record_audit(pool: &PgPool, event: NewAuditEvent) {
    let action = event.action.clone();
    if let Err(err) = store_audit::insert(pool, event).await {
        tracing::warn!(%err, action, "failed to record an upload audit event");
    }
}

/// The run config `file_ingest_job` takes: `ops.ingest_uploaded_file.config`
/// (plan T7).
fn run_config(row: &Upload, request: &IngestRequest) -> Value {
    json!({
        "ops": {
            "ingest_uploaded_file": {
                "config": {
                    "upload_id": row.id,
                    "storage_key": row.storage_key,
                    "bronze_table_name": request.table,
                    "load_mode": request.mode.as_str(),
                    "encoding": request.encoding.as_str(),
                    "delimiter": request.delimiter.to_string(),
                    "header_row": request.header_row,
                }
            }
        }
    })
}

/// Launch `file_ingest_job` for the claimed upload and return the run id. A
/// launch that does not happen settles the claim as failed
/// ([`COULD_NOT_START`]) before the error is returned, so the upload is not
/// left loading until [`STALE_CLAIM`].
async fn launch(
    state: &AppState,
    pool: &PgPool,
    claimed: &Upload,
    request: &IngestRequest,
) -> Result<String, ApiError> {
    let failure = match state
        .dagster
        .launch_run_with_config(FILE_INGEST_JOB, &run_config(claimed, request))
        .await
    {
        Ok(launched) => {
            if let Some(run_id) = launched.run_id {
                return Ok(run_id);
            }
            tracing::warn!(error = ?launched.error, upload_id = %claimed.id, "the orchestrator refused to launch a file load");
            ApiError::Unprocessable(LAUNCH_REFUSED.to_owned())
        }
        Err(err) => {
            tracing::warn!(%err, upload_id = %claimed.id, "the orchestrator could not be reached to launch a file load");
            ApiError::Unavailable(LAUNCH_UNREACHABLE.to_owned())
        }
    };
    if let Err(err) =
        uploads::mark_finished(pool, &claimed.id, None, Some(COULD_NOT_START), None).await
    {
        tracing::warn!(%err, upload_id = %claimed.id, "a claim that could not be launched could not be settled");
    }
    Err(failure)
}

/// `POST /api/uploads/{id}/ingest` — load this file into a raw table, with
/// the encoding, delimiter and header row the user confirmed. Body
/// `{ bronzeTable, mode?, encoding, delimiter, headerRow }`; `mode` is
/// `replace` (default) or `append`. Returns `{ upload, runId }`.
///
/// # The order, and why
///
/// Validate the body; settle the upload if it is loading (a load that
/// finished must not block the next one); check the table is free
/// ([`ensure_table_free`]) and not being loaded by another upload; CLAIM the
/// row ([`uploads::mark_ingesting`], no run id); launch; attach the run
/// ([`uploads::attach_run`]).
///
/// The claim comes before the launch (review finding B2): two requests for
/// one upload sent at the same moment both pass the checks above, but only one
/// claim succeeds, so only one launches. When the orchestrator cannot be
/// reached or refuses, the claim is settled as failed before the answer.
///
/// # Errors
///
/// 400 for a body that is not valid (each field has its own sentence); 404
/// for an unknown upload or another tenant's; 409 when this upload is already
/// loading, another is loading into that table, a connector loads it, or it
/// exists and no upload of this tenant created it; 422 when the orchestrator
/// refuses the launch; 503 when the table check cannot be made or the
/// orchestrator cannot be reached; 500 when a launched run could not be
/// recorded.
pub async fn ingest(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<IngestResponse>> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let request = parse_ingest_request(&body)?;
    let pool = pool(&state)?;
    let row = uploads::get(pool, &id).await?.ok_or_else(not_found)?;
    let row = Settler::new(&state, pool).settle(row).await;
    if row.status == "ingesting" {
        return Err(ApiError::Conflict(ALREADY_LOADING.to_owned()).into());
    }
    let Some(tenant_id) = row.tenant_id else {
        return Err(not_found().into());
    };
    ensure_table_free(&state, pool, tenant_id, &request.table).await?;
    if uploads::table_being_loaded(pool, &request.table, &row.id).await? {
        return Err(ApiError::Conflict(TABLE_BUSY.to_owned()).into());
    }

    let parse_options = json!({
        "encoding": request.encoding.as_str(),
        "delimiter": request.delimiter.to_string(),
        "headerRow": request.header_row,
    });
    let Some(claimed) = uploads::mark_ingesting(
        pool,
        &row.id,
        &parse_options,
        &request.table,
        request.mode,
        None,
    )
    .await?
    else {
        // Lost the race to another request, or the upload went away.
        return Err(match uploads::get(pool, &row.id).await? {
            Some(_) => ApiError::Conflict(ALREADY_LOADING.to_owned()),
            None => not_found(),
        }
        .into());
    };

    let run_id = launch(&state, pool, &claimed, &request).await?;
    let Some(attached) = uploads::attach_run(pool, &claimed.id, &run_id).await? else {
        // Not reachable through this API: only an `ingesting` row with no run
        // takes a run, and nothing settles or deletes the claim in the
        // meantime. If it ever happens a run exists that the row does not name.
        tracing::error!(upload_id = %claimed.id, %run_id, "a launched file load could not be attached to its upload");
        return Err(ApiError::Internal(NOT_RECORDED.to_owned()).into());
    };

    let event = upload_audit_event(
        &principal,
        "upload.ingest",
        &attached.id,
        json!({
            "fileName": attached.original_filename,
            "sizeBytes": attached.size_bytes,
            "table": request.table,
            "mode": request.mode.as_str(),
        }),
        "executed",
    );
    record_audit(pool, event).await;
    Ok(ApiJson(IngestResponse {
        upload: attached.into(),
        run_id,
    }))
}

// ── Delete ───────────────────────────────────────────────────────────────

/// `DELETE /api/uploads/{id}` — remove the object, then soft-delete the row.
/// 204. The table the upload became is not touched, and the row stays as the
/// record that an upload of the tenant made it ([`uploads::soft_delete`]).
///
/// Object first: a failed delete leaves the row, and the file is still
/// reachable and still listed. Deleting the row first would hide an object
/// only a storage sweep can find.
///
/// # Errors
///
/// 404 for an unknown or already deleted upload, or another tenant's; 409
/// while the upload is loading; 503 when Postgres or the object store is
/// unavailable (the upload is then still listed).
pub async fn delete(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let pool = pool(&state)?;
    let row = uploads::get(pool, &id).await?.ok_or_else(not_found)?;
    let row = Settler::new(&state, pool).settle(row).await;
    if row.status == "ingesting" {
        return Err(ApiError::Conflict(DELETE_WHILE_LOADING.to_owned()).into());
    }
    let store = UploadStore::connect(&state.config).await?;
    store.delete(&row.storage_key).await?;
    if !uploads::soft_delete(pool, &row.id).await? {
        // Refused because a load was claimed after the check above, or the
        // row was deleted by another request meanwhile.
        return Err(match uploads::get(pool, &row.id).await? {
            Some(_) => ApiError::Conflict(DELETE_WHILE_LOADING.to_owned()),
            None => not_found(),
        }
        .into());
    }

    let mut args = json!({ "fileName": row.original_filename, "sizeBytes": row.size_bytes });
    if let (Value::Object(map), Some(table)) = (&mut args, &row.bronze_table) {
        map.insert("table".to_owned(), json!(table));
    }
    let event = upload_audit_event(&principal, "upload.delete", &row.id, args, "executed");
    record_audit(pool, event).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    // ── the storage key ─────────────────────────────────────────────────

    #[test]
    fn storage_key_never_uses_the_users_filename() {
        let key = storage_key("dispar-dki", "up-1", "../../etc/passwd");
        assert_eq!(key, "uploads/dispar-dki/up-1");
        let key = storage_key("dispar-dki", "up-2", "rawdata.xls");
        assert_eq!(key, "uploads/dispar-dki/up-2.xls");
    }

    #[test]
    fn storage_key_sanitises_the_tenant() {
        // Every offending character is replaced one-for-one, so `a/../b`
        // (four of them) becomes `a----b`: the traversal is gone and the
        // key stays inside the prefix.
        let key = storage_key("a/../b", "up-3", "x.csv");
        assert_eq!(key, "uploads/a----b/up-3.csv");
    }

    #[test]
    fn extension_is_dropped_when_implausible() {
        assert_eq!(extension_of("file.verylongextension"), "");
        assert_eq!(extension_of("file.cs v"), "");
        assert_eq!(extension_of("noext"), "");
        assert_eq!(extension_of("data.CSV"), "csv");
    }

    // ── the name shown ──────────────────────────────────────────────────

    #[test]
    fn a_display_name_is_the_last_path_segment_without_control_characters() {
        assert_eq!(display_name(Some("stock.csv")), "stock.csv");
        assert_eq!(display_name(Some("C:\\Users\\x\\stock.csv")), "stock.csv");
        assert_eq!(display_name(Some("../../etc/passwd")), "passwd");
        assert_eq!(display_name(Some("a\u{0}b\nc.csv")), "abc.csv");
        assert_eq!(display_name(Some("  padded.csv  ")), "padded.csv");
    }

    #[test]
    fn a_display_name_is_bounded_and_never_empty() {
        assert_eq!(display_name(Some(&"x".repeat(2000))).chars().count(), 255);
        for nothing in [None, Some(""), Some("   "), Some("dir/"), Some("\u{7}")] {
            assert_eq!(display_name(nothing), "unnamed", "{nothing:?}");
        }
    }

    #[test]
    fn a_checksum_is_lower_hex_of_the_whole_input() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    // ── what an upload may be ───────────────────────────────────────────

    #[test]
    fn a_file_must_not_be_empty_over_the_cap_a_workbook_or_binary() {
        let refusal = |bytes: &[u8]| check_file(bytes).unwrap_err().to_string();
        assert_eq!(refusal(b""), "The file is empty.");
        assert!(check_file(b"id,name\n1,a\n").is_ok());
        assert!(check_file(&[b'a'; 1024]).is_ok());
        assert_eq!(
            refusal(b"PK\x03\x04rest of a zip"),
            "This looks like an Excel workbook or a zip archive. Only delimited text files (CSV, TSV) can be uploaded; save the sheet as CSV first."
        );
        assert!(refusal(&[0xD0, 0xCF, 0x11, 0xE0, 1, 2, 3, 4]).contains("Excel workbook"));
        assert!(refusal(b"PAR1\x15\x04rest").contains("Parquet"));
        assert!(refusal(b"%PDF-1.7 ...").contains("not a delimited text file"));
        assert!(refusal(b"a,b\0c,d").contains("not a delimited text file"));
    }

    #[test]
    fn a_file_over_the_cap_is_refused_with_the_fixed_text() {
        let over = vec![b'a'; MAX_UPLOAD_BYTES + 1];
        assert_eq!(
            check_file(&over).unwrap_err().to_string(),
            "The file is larger than the 50 MB limit."
        );
        assert!(check_file(&over[..MAX_UPLOAD_BYTES]).is_ok());
    }

    #[test]
    fn the_request_body_limit_leaves_room_for_the_form_around_the_file() {
        assert_eq!(MAX_REQUEST_BODY_BYTES - MAX_UPLOAD_BYTES, 1024 * 1024);
    }

    // ── the table name ──────────────────────────────────────────────────

    #[test]
    fn a_table_name_is_the_console_rule_and_is_never_folded_to_it() {
        for ok in ["orders", "orders_raw", "_staging", "t2025", "a", "a_1_b"] {
            assert_eq!(table_name_problem(ok), None, "{ok}");
        }
        for refused in [
            "",
            "Orders",
            "orders 2025",
            "2025_orders",
            "orders-raw",
            "orders.raw",
            "orders;drop",
            "pesanan\u{e9}",
            " orders",
            "orders ",
            "ORDERS",
        ] {
            assert_eq!(
                table_name_problem(refused),
                Some(TABLE_NAME_RULE),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn a_table_name_has_a_length_bound() {
        assert_eq!(table_name_problem(&"a".repeat(128)), None);
        assert_eq!(table_name_problem(&"a".repeat(129)), Some(TABLE_NAME_RULE));
    }

    // ── the ingest body ─────────────────────────────────────────────────

    fn body(value: &Value) -> Vec<u8> {
        serde_json::to_vec(value).unwrap()
    }

    fn valid() -> Value {
        json!({
            "bronzeTable": "stock_raw",
            "encoding": "utf-8",
            "delimiter": ",",
            "headerRow": 0,
        })
    }

    fn refusal_of(value: &Value) -> String {
        parse_ingest_request(&body(value)).unwrap_err().to_string()
    }

    #[test]
    fn a_complete_body_is_accepted_and_mode_defaults_to_replace() {
        let request = parse_ingest_request(&body(&valid())).unwrap();
        assert_eq!(
            request,
            IngestRequest {
                table: "stock_raw".to_owned(),
                mode: LoadMode::Replace,
                encoding: Encoding::Utf8,
                delimiter: ',',
                header_row: 0,
            }
        );
        let mut full = valid();
        full["mode"] = json!("append");
        full["encoding"] = json!("utf-16");
        full["delimiter"] = json!("\t");
        full["headerRow"] = json!(4);
        let request = parse_ingest_request(&body(&full)).unwrap();
        assert_eq!(request.mode, LoadMode::Append);
        assert_eq!(request.encoding, Encoding::Utf16);
        assert_eq!(request.delimiter, '\t');
        assert_eq!(request.header_row, 4);
        let mut null_mode = valid();
        null_mode["mode"] = Value::Null;
        assert_eq!(
            parse_ingest_request(&body(&null_mode)).unwrap().mode,
            LoadMode::Replace
        );
    }

    #[test]
    fn each_of_the_five_fields_is_validated_with_its_own_sentence() {
        let with = |field: &str, value: Value| {
            let mut v = valid();
            v[field] = value;
            v
        };
        assert_eq!(
            refusal_of(&with("bronzeTable", json!("Orders 2025"))),
            TABLE_NAME_RULE
        );
        assert_eq!(
            refusal_of(&with("mode", json!("merge"))),
            "mode must be replace or append."
        );
        assert_eq!(
            refusal_of(&with("mode", json!("Append"))),
            "mode must be replace or append."
        );
        assert_eq!(
            refusal_of(&with("mode", json!(1))),
            "mode must be replace or append."
        );
        assert_eq!(
            refusal_of(&with("encoding", json!("latin-1"))),
            "encoding must be utf-8 or utf-16."
        );
        assert_eq!(
            refusal_of(&with("delimiter", json!("::"))),
            "delimiter must be a comma, a semicolon, a tab or a pipe."
        );
        assert_eq!(
            refusal_of(&with("delimiter", json!(""))),
            "delimiter must be a comma, a semicolon, a tab or a pipe."
        );
        for header_row in [json!(-1), json!(1.5), json!("2"), json!(true)] {
            assert_eq!(
                refusal_of(&with("headerRow", header_row)),
                "headerRow must be a whole number, 0 or more."
            );
        }
    }

    #[test]
    fn a_missing_or_mistyped_field_is_named() {
        for field in ["bronzeTable", "encoding", "delimiter", "headerRow"] {
            let mut v = valid();
            v.as_object_mut().unwrap().remove(field);
            assert_eq!(refusal_of(&v), format!("{field} is required."));
            v[field] = Value::Null;
            assert_eq!(refusal_of(&v), format!("{field} is required."));
        }
        for field in ["bronzeTable", "encoding", "delimiter"] {
            let mut v = valid();
            v[field] = json!(7);
            assert_eq!(refusal_of(&v), format!("{field} must be text."));
        }
    }

    #[test]
    fn a_body_that_is_not_a_json_object_is_refused_without_serde_text() {
        for raw in [&b""[..], b"not json", b"[1]", b"\"x\"", b"null", b"{"] {
            let err = parse_ingest_request(raw).unwrap_err();
            assert_eq!(err.status(), 400);
            assert_eq!(err.to_string(), "The request body must be a JSON object.");
        }
    }

    // ── the preview query ───────────────────────────────────────────────

    fn query(
        encoding: Option<&str>,
        delimiter: Option<&str>,
        header_row: Option<&str>,
    ) -> PreviewQuery {
        PreviewQuery {
            encoding: encoding.map(str::to_owned),
            delimiter: delimiter.map(str::to_owned),
            header_row: header_row.map(str::to_owned),
        }
    }

    #[test]
    fn preview_overrides_are_what_was_asked_for_and_nothing_else() {
        let none = overrides_of(&query(None, None, None)).unwrap();
        assert!(none.encoding.is_none() && none.delimiter.is_none() && none.header_row.is_none());
        let all = overrides_of(&query(Some("utf-16"), Some("\t"), Some("4"))).unwrap();
        assert_eq!(all.encoding, Some(Encoding::Utf16));
        assert_eq!(all.delimiter, Some('\t'));
        assert_eq!(all.header_row, Some(4));
    }

    #[test]
    fn a_preview_override_that_is_given_must_be_valid() {
        let refused = |q: PreviewQuery| overrides_of(&q).map(|_| ()).unwrap_err().to_string();
        assert_eq!(
            refused(query(Some("latin-1"), None, None)),
            "encoding must be utf-8 or utf-16."
        );
        assert_eq!(
            refused(query(Some(""), None, None)),
            "encoding must be utf-8 or utf-16."
        );
        assert_eq!(
            refused(query(None, Some(""), None)),
            "delimiter must be a comma, a semicolon, a tab or a pipe."
        );
        assert_eq!(
            refused(query(None, Some(";;"), None)),
            "delimiter must be a comma, a semicolon, a tab or a pipe."
        );
        for header_row in ["-1", "1.5", "x", "", "99999999999999999999999"] {
            assert_eq!(
                refused(query(None, None, Some(header_row))),
                "headerRow must be a whole number, 0 or more.",
                "{header_row:?}"
            );
        }
    }

    // ── settling a load ─────────────────────────────────────────────────

    fn at(rfc3339: &str) -> OffsetDateTime {
        OffsetDateTime::parse(rfc3339, &Rfc3339).unwrap()
    }

    #[test]
    fn a_result_counts_only_when_it_ended_after_the_claim() {
        let claim = at("2026-10-02T10:00:00Z");
        // What `datetime.now(timezone.utc).isoformat()` writes, with and
        // without a fraction.
        assert!(ended_after("2026-10-02T10:00:01.250000+00:00", claim));
        assert!(ended_after("2026-10-02T10:00:01+00:00", claim));
        assert!(!ended_after("2026-10-02T09:59:59.999999+00:00", claim));
        assert!(
            !ended_after("2026-10-02T10:00:00+00:00", claim),
            "not after"
        );
        assert!(ended_after(
            "2026-10-02T17:00:01+07:00",
            at("2026-10-02T10:00:00Z")
        ));
        for unreadable in ["", "yesterday", "2026-10-02", "2026-10-02 10:00:01"] {
            assert!(!ended_after(unreadable, claim), "{unreadable:?}");
        }
    }

    fn result(status: &str, rows: Option<u64>, error: &str) -> IngestRunRow {
        IngestRunRow {
            connector_id: "upload:up-1".to_owned(),
            job: "file_ingest_job".to_owned(),
            object: "stock_raw".to_owned(),
            rows,
            started_at: "2026-10-02T10:00:00+00:00".to_owned(),
            ended_at: "2026-10-02T10:00:05+00:00".to_owned(),
            status: status.to_owned(),
            error: error.to_owned(),
        }
    }

    #[test]
    fn a_succeeded_result_is_ingested_with_its_rows_and_never_a_made_up_zero() {
        assert_eq!(
            outcome_of(&result("succeeded", Some(35_000), "")),
            Outcome::Ingested { rows: Some(35_000) }
        );
        assert_eq!(
            outcome_of(&result("succeeded", None, "")),
            Outcome::Ingested { rows: None },
            "not measured stays not measured"
        );
        assert_eq!(
            outcome_of(&result("succeeded", Some(u64::MAX), "")),
            Outcome::Ingested { rows: None },
            "a count that does not fit is not shown as another number"
        );
    }

    #[test]
    fn anything_else_is_failed_with_the_recorded_reason() {
        assert_eq!(
            outcome_of(&result("failed", None, "The load into the table failed.")),
            Outcome::Failed("The load into the table failed.".to_owned())
        );
        assert_eq!(
            outcome_of(&result(
                "rejected",
                None,
                "  The file has no rows below the header row. "
            )),
            Outcome::Failed("The file has no rows below the header row.".to_owned())
        );
        assert_eq!(
            outcome_of(&result("failed", Some(10), "")),
            Outcome::Failed("The load failed.".to_owned()),
            "no reason recorded: a fixed one, and no count"
        );
    }

    #[test]
    fn a_claim_is_stale_after_two_minutes_and_not_before() {
        let claimed = at("2026-10-02T10:00:00Z");
        assert!(!claim_is_stale(claimed, at("2026-10-02T10:00:30Z")));
        assert!(!claim_is_stale(claimed, at("2026-10-02T10:02:00Z")));
        assert!(claim_is_stale(claimed, at("2026-10-02T10:02:01Z")));
        assert!(
            !claim_is_stale(claimed, at("2026-10-02T09:59:00Z")),
            "clock behind"
        );
    }

    // ── what the wire carries ───────────────────────────────────────────

    #[test]
    fn the_messages_the_plan_words_are_the_plans_words() {
        assert_eq!(ALREADY_LOADING, "This upload is already being loaded.");
        assert_eq!(TOO_LARGE, "The file is larger than the 50 MB limit.");
        assert_eq!(COULD_NOT_START, "The load could not be started.");
        assert_eq!(NOT_STARTED, "The load was not started.");
        assert_eq!(NO_RESULT, "The load stopped before it recorded a result.");
    }

    #[test]
    fn no_message_names_a_host_a_path_or_a_driver() {
        for message in [
            NOT_FOUND,
            NO_TENANT,
            NOT_MULTIPART,
            NO_FILE_PART,
            UNREADABLE,
            EMPTY_FILE,
            TOO_LARGE,
            WORKBOOK,
            PARQUET,
            OTHER_BINARY,
            TABLE_NAME_RULE,
            ALREADY_LOADING,
            TABLE_BUSY,
            CONNECTOR_TABLE,
            TABLE_NOT_FREE,
            TABLE_UNCHECKED,
            NO_QUERY_DATABASE,
            LAUNCH_UNREACHABLE,
            LAUNCH_REFUSED,
            NOT_RECORDED,
            DELETE_WHILE_LOADING,
        ] {
            for forbidden in ["http", "://", "uploads/", "127.0.0.1", "sqlx", "Exception"] {
                assert!(
                    !message.contains(forbidden),
                    "{message:?} has {forbidden:?}"
                );
            }
        }
    }
}
