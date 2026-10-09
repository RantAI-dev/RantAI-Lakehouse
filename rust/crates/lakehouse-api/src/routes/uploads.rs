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
//! 5. `DELETE /api/uploads/{id}` removes the object and the row. The table the
//!    upload became stays, and so does the claim on its name.
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
//!   The one piece of recorded text a response can carry is a reason
//!   `file_ingest_job` wrote into `ingest_run`, and only when it is one of the
//!   seven the API knows ([`JOB_FAILURE_REASONS`], review findings B6 and C2);
//!   anything else the job recorded is shown as [`LOAD_FAILED`].
//! * **Fail closed on a table name.** A load may target a table that does not
//!   exist or one this tenant's uploads claimed; never a connector's table,
//!   never anything else. Who owns a name is the claim table's to say
//!   (`lakehouse_store::uploads::claim_table`, review finding B4), not the
//!   upload rows'. When the API cannot tell, it answers 503 and does not guess
//!   that the name is free ([`ensure_table_free`]).
//! * **One load per upload, and the row is claimed before the job is
//!   launched** (review finding B2): see [`ingest`].
//!
//! # Excel workbooks
//!
//! An `.xls` or `.xlsx` is accepted (ADR 0014, amendment of 2026-10-07): the
//! workbook is stored as it arrived, the preview converts the chosen sheet to
//! delimited text in memory ([`crate::upload_workbook`]) and shows it through
//! the same preview as a CSV, and an ingest stores that text as a CSV object
//! beside the original ([`converted_key`]) and launches the unchanged
//! `file_ingest_job` on it with a fixed dialect (UTF-8, comma). Whether an
//! upload IS a workbook is decided by its stored name AND its first bytes
//! ([`conversion_of_head`]): a text export named `.xls` (the file that motivated
//! the feature) is still text. The chosen sheet cannot go in the job's run
//! configuration (its schema is closed and the job does not change), so it is
//! recorded in the upload's `parse_options` and in the audit event.
//!
//! # Parquet files
//!
//! A `.parquet` file takes the same path as a workbook (section 11 of
//! `docs/superpowers/plans/2026-10-07-upload-excel.md`): stored as it arrived,
//! converted in memory for the preview ([`crate::upload_parquet`]) and, at
//! ingest, stored as a CSV object beside the original ([`converted_key`]) that
//! the unchanged job loads with the fixed dialect. A file has one table and its
//! own column names, so there is no sheet and the header row is fixed at 0
//! (`headerRow` in a request is not read). Whether an upload IS a Parquet file
//! is decided by its stored name AND its first bytes, as for a workbook
//! ([`conversion_of_head`]). Where the code branches on "is this converted
//! before it is loaded", it branches on [`Conversion`], not on a workbook.
//!
//! # Statuses
//!
//! Every refusal of the upload itself (no part, empty, too large, a workbook
//! that is not an `.xls` or `.xlsx` or does not open, a Parquet file that does
//! not open, is encrypted, has a binary or nested column or is past the cell
//! cap, or another binary) is a 400 with a fixed sentence; `lakehouse_core::
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
use lakehouse_dagster::map_run_status;
use lakehouse_store::PgPool;
use lakehouse_store::audit::{self as store_audit, NewAuditEvent};
use lakehouse_store::connectors;
use lakehouse_store::uploads::{self, LoadMode, NewUpload, TableClaim, Upload};
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
use crate::upload_parquet::{self, ParquetInfo};
use crate::upload_parse::{self, Encoding, Kind, Overrides, Preview};
use crate::upload_store::{PREFIX, UploadStore};
use crate::upload_workbook::{self, WorkbookError, WorkbookInfo};

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

/// How many records of a converted sheet the preview reads. A workbook is read
/// whole to be converted, but the preview runs on text, and a header-row
/// change re-runs it: a prefix keeps that cheap. A header row chosen past this
/// shows no columns, as for a text file whose head is shorter than the row.
const WORKBOOK_PREVIEW_RECORDS: usize = 2000;

/// How many rows of a Parquet file the preview decodes (the converted text has
/// one record more, the column names). Only these are decoded: the footer
/// already says how many rows the file has.
const PARQUET_PREVIEW_ROWS: usize = 2000;

/// How many leading bytes tell a workbook from text for an upload that is
/// named like one (the magic numbers are four bytes).
const SNIFF_PROBE_BYTES: usize = 8;

/// Uploads one `GET /api/uploads` returns, newest first. There is no paging:
/// an older upload is not listed.
const LIST_LIMIT: i64 = 100;

/// A claim ([`uploads::mark_ingesting`]) that has no run after this long was
/// never launched: the request died between the claim and the launch. The
/// request deadline for `/api/uploads/{id}/ingest` is 60 s, so two minutes is
/// past any request that could still be about to attach a run.
const STALE_CLAIM: time::Duration = time::Duration::minutes(2);

/// How old a claim must be before an upload whose run the orchestrator no
/// longer knows, and which recorded no result, is failed ([`RUN_UNKNOWN`];
/// review finding B5). Without an end such an upload stayed loading for ever,
/// and a loading upload can be neither deleted nor loaded again.
///
/// One hour is a bound chosen to be well past any load of a file at the
/// [`MAX_UPLOAD_BYTES`] cap, not a measurement: no load of a file that size has
/// been timed. It only has to be long enough that a run which is merely slow,
/// or about to start, is not failed under its own feet.
const UNKNOWN_RUN_BOUND: time::Duration = time::Duration::hours(1);

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
const WORKBOOK: &str = "This looks like a workbook or a zip archive that is not an .xls or .xlsx file. Only .xls and .xlsx workbooks, .parquet files and delimited text files (CSV, TSV) can be uploaded; save the sheet as .xlsx or CSV first.";
const PARQUET: &str = "This looks like a Parquet file, but its name does not end in .parquet. Rename it to end in .parquet and upload it again.";
const OTHER_BINARY: &str = "This is not a delimited text file, an Excel workbook or a Parquet file. Only .xls and .xlsx workbooks, .parquet files and delimited text files (CSV, TSV) can be uploaded.";
const EMPTY_SHEET: &str = "That sheet is empty, so there is nothing to load.";
const EMPTY_PARQUET: &str = "That Parquet file has no rows, so there is nothing to load.";
const SHEET_RULE: &str = "sheet must be text.";
const CONVERSION_FAILED: &str = "The file could not be converted.";
const BODY_NOT_AN_OBJECT: &str = "The request body must be a JSON object.";
/// The sentence for a table name [`table_name_problem`] refuses, in words a
/// person can follow (review finding C1). T10's `tableNameProblem` in the
/// console must say the same one; it does not exist yet.
const TABLE_NAME_RULE: &str = "Table names start with a lower-case letter and use lower-case letters and digits joined by single underscores, with at most 128 characters.";
const MODE_RULE: &str = "mode must be replace or append.";
const ENCODING_RULE: &str = "encoding must be utf-8 or utf-16.";
const DELIMITER_RULE: &str = "delimiter must be a comma, a semicolon, a tab or a pipe.";
const HEADER_ROW_RULE: &str = "headerRow must be a whole number, 0 or more.";
const QUERY_UNREADABLE: &str = "The query string could not be read.";
const ALREADY_LOADING: &str = "This upload is already being loaded.";
const TABLE_BUSY: &str = "Another upload is loading into that table.";
const CONNECTOR_TABLE: &str = "A connector loads that table, so a file cannot be loaded into it.";
/// The one sentence for a table name that is not this tenant's to load into:
/// another tenant holds the claim on it, or it exists and nobody claimed it.
/// Deliberately the same for both, so that the answer does not tell a caller
/// which of the two it is (review finding B4): a tenant must not be able to
/// learn what other tenants have claimed, or which names are theirs.
const TABLE_NOT_FREE: &str = "That table name is in use and no upload of this tenant created it, so a file cannot be loaded into it.";
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
/// The run recorded a failure with no reason, or with text that is not one of
/// [`JOB_FAILURE_REASONS`].
const LOAD_FAILED: &str = "The load failed.";
/// A claim older than [`UNKNOWN_RUN_BOUND`] whose run the orchestrator does not
/// know and which recorded no result (review finding B5).
const RUN_UNKNOWN: &str = "The orchestrator no longer knows this load.";

// The reasons `file_ingest_job` records when a load fails (plan T7). The API
// shows a recorded reason only when it is one of these seven (review finding
// B6): `ingest_run.error` is free text, so one `str(exc)` in the job would
// otherwise put exception text, a host or a path into a response. These are
// the same seven as `ops/fixtures/upload_load_failure_reasons.json`, which a
// test below reads and which the job's own tests assert its constants against.

const JOB_FILE_UNREADABLE: &str = "The stored file could not be read.";
const JOB_HEADER_PAST_END: &str = "The header row is past the end of the file.";
// Review finding C2: a record at the header row that has no cells (an empty
// line chosen as the header) used to be recorded as "past the end".
const JOB_HEADER_NO_COLUMNS: &str = "The header row has no columns.";
const JOB_NO_ROWS: &str = "The file has no rows below the header row.";
const JOB_TOO_MANY_ROWS: &str = "The file has more than 2,000,000 rows.";
const JOB_LOAD_FAILED: &str = "The load into the table failed.";
const JOB_NOT_REGISTERED: &str = "The table was loaded but could not be registered in the catalog.";

/// Every reason the job may record, in the order of
/// `ops/fixtures/upload_load_failure_reasons.json`.
const JOB_FAILURE_REASONS: [&str; 7] = [
    JOB_FILE_UNREADABLE,
    JOB_HEADER_PAST_END,
    JOB_HEADER_NO_COLUMNS,
    JOB_NO_ROWS,
    JOB_TOO_MANY_ROWS,
    JOB_LOAD_FAILED,
    JOB_NOT_REGISTERED,
];

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
    /// For a workbook: its sheets and the one a preview reads when none is
    /// asked for. Absent for a text file.
    #[serde(skip_serializing_if = "Option::is_none")]
    workbook: Option<WorkbookInfo>,
    /// For a Parquet file: its columns with their declared types and its row
    /// count. Absent for anything else.
    #[serde(skip_serializing_if = "Option::is_none")]
    parquet: Option<ParquetInfo>,
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
/// Built entirely from server-controlled parts — the tenant's id (a UUID,
/// `create` passes `tenant_id.to_string()`) and a fresh upload id — with the
/// user's filename contributing at most a sanitised extension. A key derived
/// from a caller-supplied name is how `../` escapes, 2 KB paths and cross-user
/// collisions happen.
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

/// Whether a name (or a storage key, which ends in the stored extension) is
/// one an Excel workbook may have: `.xls` or `.xlsx`. Necessary, not
/// sufficient: a text file can carry either ([`conversion_of`]).
fn has_workbook_name(name: &str) -> bool {
    matches!(extension_of(name).as_str(), "xls" | "xlsx")
}

/// Whether a name (or a storage key) is `.parquet`. Necessary, not
/// sufficient, as for a workbook.
fn has_parquet_name(name: &str) -> bool {
    extension_of(name) == "parquet"
}

/// What an upload is turned into before the load reads it: the load job reads
/// delimited text and nothing else, so these two are converted in the API, once
/// (ADR 0014, decision 3 and its amendments).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Conversion {
    /// An Excel workbook: one sheet becomes the text.
    Workbook,
    /// A Parquet file: its one table becomes the text.
    Parquet,
}

/// The conversion a stored name allows, before its bytes are looked at.
fn conversion_for_name(name: &str) -> Option<Conversion> {
    if has_workbook_name(name) {
        Some(Conversion::Workbook)
    } else if has_parquet_name(name) {
        Some(Conversion::Parquet)
    } else {
        None
    }
}

/// The key a converted upload is stored under, beside the original:
/// server-made like [`storage_key`], inside the same prefix.
fn converted_key(storage_key: &str) -> String {
    format!("{storage_key}.converted.csv")
}

/// The conversion an upload needs, judged from its stored name AND its first
/// bytes: a text export called `.xls` (ADR 0014's motivating file) is text, and
/// so is a text file called `.parquet`. A file whose footer is encrypted starts
/// with `PARE` and not with the magic [`upload_parse::sniff`] knows, so a
/// `.parquet` that starts so is still a Parquet file (the reader refuses it
/// with its own reason).
fn conversion_of_head(key: &str, head: &[u8]) -> Option<Conversion> {
    match (conversion_for_name(key)?, upload_parse::sniff(head)) {
        (Conversion::Workbook, Kind::Workbook) => Some(Conversion::Workbook),
        (Conversion::Parquet, Kind::Parquet) => Some(Conversion::Parquet),
        (Conversion::Parquet, _) if upload_parquet::has_encrypted_magic(head) => {
            Some(Conversion::Parquet)
        }
        _ => None,
    }
}

/// The conversion a stored upload needs, or `None` for text. An upload that is
/// not named like a workbook or a Parquet file costs no storage read.
///
/// # Errors
///
/// 503 when the object store is unavailable.
async fn conversion_of(state: &AppState, row: &Upload) -> Result<Option<Conversion>, ApiError> {
    if conversion_for_name(&row.storage_key).is_none() {
        return Ok(None);
    }
    let store = UploadStore::connect(&state.config).await?;
    let head = store
        .head_bytes(&row.storage_key, SNIFF_PROBE_BYTES)
        .await?;
    Ok(conversion_of_head(&row.storage_key, &head))
}

/// Run a conversion on a blocking thread (parsing is CPU work on up to 50 MB)
/// and turn its refusal into a 400 with the refusal's own sentence. A task that
/// did not finish is logged and answered with a fixed sentence.
async fn blocking<T: Send + 'static, E: std::fmt::Display + Send + 'static>(
    work: impl FnOnce() -> Result<T, E> + Send + 'static,
) -> Result<T, ApiError> {
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(done)) => Ok(done),
        Ok(Err(refusal)) => Err(ApiError::BadRequest(refusal.to_string())),
        Err(err) => {
            tracing::warn!(%err, "a file conversion task did not finish");
            Err(ApiError::Internal(CONVERSION_FAILED.to_owned()))
        }
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

/// What an acceptable upload is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Accepted {
    Text,
    /// Starts like a workbook and is named `.xls` or `.xlsx`; whether it
    /// really opens is for `upload_workbook` to say.
    Workbook,
    /// Starts like a Parquet file (or like an encrypted one) and is named
    /// `.parquet`; whether it really opens is for `upload_parquet` to say.
    Parquet,
}

/// Whether the bytes of an upload are acceptable: not empty, within the cap,
/// and delimited text, a workbook or a Parquet file by their first bytes (never
/// by the type the browser claimed, and the name only to refuse a zip, OLE or
/// Parquet file that is not named for what it is: the file that motivated this
/// was called `.xls` and was text).
fn check_file(bytes: &[u8], filename: &str) -> Result<Accepted, ApiError> {
    if bytes.is_empty() {
        return Err(ApiError::BadRequest(EMPTY_FILE.to_owned()));
    }
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(ApiError::BadRequest(TOO_LARGE.to_owned()));
    }
    if has_parquet_name(filename) && upload_parquet::has_encrypted_magic(bytes) {
        // Not a magic `sniff` knows; the reader names the real reason.
        return Ok(Accepted::Parquet);
    }
    let message = match upload_parse::sniff(bytes) {
        Kind::DelimitedText => return Ok(Accepted::Text),
        Kind::Workbook if has_workbook_name(filename) => return Ok(Accepted::Workbook),
        Kind::Workbook => WORKBOOK,
        Kind::Parquet if has_parquet_name(filename) => return Ok(Accepted::Parquet),
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
/// `file` part, an empty file, a file over [`MAX_UPLOAD_BYTES`], a workbook
/// that is not an `.xls` or `.xlsx` or does not open (damaged, password
/// protected, no sheet), a Parquet file that is not named `.parquet` or does
/// not open (damaged, encrypted, a binary or nested column, over the cell cap)
/// or another binary (each with its own fixed sentence); 404 if `X-Tenant` names a tenant the caller does not belong to;
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
    let accepted = check_file(&part.bytes, &part.filename)?;
    // A workbook or a Parquet file is opened before anything is stored: only
    // what opens is kept. (A Parquet file is opened by its footer; a page that
    // cannot be decoded is found when the file is converted.)
    let (workbook, parquet) = match accepted {
        Accepted::Text => (None, None),
        Accepted::Workbook => {
            let bytes = part.bytes.clone();
            (
                Some(blocking(move || upload_workbook::list_sheets(&bytes)).await?),
                None,
            )
        }
        Accepted::Parquet => {
            let bytes = part.bytes.clone();
            (
                None,
                Some(blocking(move || upload_parquet::inspect(&bytes)).await?),
            )
        }
    };

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
            workbook,
            parquet,
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
///
/// A failure shows the reason the job recorded only when it is one of
/// [`JOB_FAILURE_REASONS`]; any other text, an empty one included, is
/// [`LOAD_FAILED`] (review finding B6). `ingest_run.error` is free text, and
/// what the job happened to write must not decide what a response says: shown
/// as written, one `str(exc)` in the job would put exception text, a host or a
/// path into a response. What is shown is the API's own constant, never the
/// recorded string.
fn outcome_of(result: &IngestRunRow) -> Outcome {
    if result.status == "succeeded" {
        return Outcome::Ingested {
            rows: result.rows.and_then(|rows| i64::try_from(rows).ok()),
        };
    }
    let recorded = result.error.trim();
    let reason = JOB_FAILURE_REASONS
        .into_iter()
        .find(|known| *known == recorded)
        .unwrap_or(LOAD_FAILED);
    Outcome::Failed(reason.to_owned())
}

/// Whether a claim with no run is old enough to have been abandoned.
fn claim_is_stale(updated_at: OffsetDateTime, now: OffsetDateTime) -> bool {
    now - updated_at > STALE_CLAIM
}

/// Whether a claim whose run the orchestrator does not know has been waiting
/// long enough to be failed ([`UNKNOWN_RUN_BOUND`], review finding B5).
fn unknown_run_is_overdue(updated_at: OffsetDateTime, now: OffsetDateTime) -> bool {
    now - updated_at > UNKNOWN_RUN_BOUND
}

/// What the orchestrator said about a loading upload's run, once it has said
/// the run is not still going.
enum RunEnd {
    /// The run ended: completed, failed or cancelled.
    Ended,
    /// The orchestrator does not know the run. `pipeline_run_status` answers
    /// `None` for a run it never had or has lost (its run storage was reset)
    /// and also for a GraphQL error with no data, and the two cannot be told
    /// apart.
    Unknown,
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
///   `failed` with its reason ([`outcome_of`]), and no result gives `failed`
///   ([`NO_RESULT`]).
/// - Dagster does not know the run (review finding B5): read the recorded
///   result all the same. A result found settles it, as above. No result and
///   a claim older than [`UNKNOWN_RUN_BOUND`] gives `failed`
///   ([`RUN_UNKNOWN`]); before this an upload whose run Dagster had lost
///   stayed loading for ever, neither deletable nor loadable again. No result
///   and a younger claim is left as it is: the run may be about to start or
///   to record its result. The cost of the bound is that a run which Dagster
///   merely failed to answer for (a GraphQL error reads as "no such run") and
///   which is still loading after the bound would be failed, and a later
///   result of that run would be lost to the row.
/// - When Dagster or `ClickHouse` cannot be asked, or does not answer in time,
///   the row is returned unchanged: that is a doubt, not an outcome.
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
        let end = match tokio::time::timeout(
            SETTLE_CALL_TIMEOUT,
            self.state.dagster.pipeline_run_status(&run_id),
        )
        .await
        {
            Ok(Ok(Some(info))) => {
                if !matches!(
                    map_run_status(&info.status),
                    "completed" | "failed" | "cancelled"
                ) {
                    return row;
                }
                RunEnd::Ended
            }
            // Review finding B5: not a reason to leave the upload loading for
            // ever. The recorded result is read below, and a claim that is
            // old enough and has none is failed.
            Ok(Ok(None)) => {
                tracing::warn!(upload_id = %row.id, %run_id, "Dagster does not know the run of a loading upload");
                RunEnd::Unknown
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
        if self.results_unreadable {
            return row;
        }
        let outcome = match self.recorded(&row).await {
            Recorded::Found(outcome) => outcome,
            Recorded::Missing => match end {
                RunEnd::Ended => Outcome::Failed(NO_RESULT.to_owned()),
                // Finding B5: "not found and younger" is left alone.
                RunEnd::Unknown
                    if unknown_run_is_overdue(row.updated_at, OffsetDateTime::now_utc()) =>
                {
                    Outcome::Failed(RUN_UNKNOWN.to_owned())
                }
                RunEnd::Unknown => return row,
            },
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
    /// For a workbook: the sheet to read (its default when absent). Ignored
    /// for a text file.
    sheet: Option<String>,
}

/// `GET /api/uploads/{id}/preview`'s response: the [`Preview`] and, for a
/// workbook, which sheet it shows and which sheets there are.
#[derive(Debug, Serialize)]
pub struct PreviewResponse {
    #[serde(flatten)]
    preview: Preview,
    #[serde(skip_serializing_if = "Option::is_none")]
    workbook: Option<WorkbookView>,
    /// For a Parquet file: its columns with the types the file declares and
    /// the row count its footer states (`preview.rows` are the first ones).
    #[serde(skip_serializing_if = "Option::is_none")]
    parquet: Option<ParquetInfo>,
}

/// What a workbook preview adds to a [`Preview`].
#[derive(Debug, Serialize)]
pub struct WorkbookView {
    /// The sheet the preview was read from.
    sheet: String,
    #[serde(flatten)]
    info: WorkbookInfo,
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

/// `GET /api/uploads/{id}/preview?encoding=&delimiter=&headerRow=&sheet=` —
/// what the file appears to contain: `detected`, `using`, `columns`, at most
/// 20 `rows`, and `truncated`.
///
/// Reads only the first [`PREVIEW_BYTES`] of a text object (a range read), so
/// the cost does not grow with the file. Every detected value can be
/// overridden; whatever the user settles on is what [`ingest`] is handed.
///
/// A workbook is read whole, converted on a blocking thread, and its first
/// [`WORKBOOK_PREVIEW_RECORDS`] records go through the same preview as a CSV,
/// under the fixed dialect the load will use. `encoding` and `delimiter` are
/// validated and then ignored for a workbook (they have no meaning there; the
/// answer says `utf-8` and `,`), `headerRow` applies, and `sheet` picks the
/// sheet. The answer adds `workbook`: the sheet read and the sheets there are.
///
/// A Parquet file is converted the same way (only the first
/// [`PARQUET_PREVIEW_ROWS`] rows are decoded). `encoding`, `delimiter`,
/// `headerRow` and `sheet` are validated and then ignored: the header is the
/// file's own column names, always the first record. The answer adds
/// `parquet`: each column's declared type and the file's row count.
///
/// # Errors
///
/// 400 for an `encoding`, `delimiter` or `headerRow` that is not valid, and
/// for a workbook or Parquet file that cannot be read, a `sheet` a workbook
/// does not have or a sheet or file over the cell limit (each with its own
/// fixed sentence); 404
/// for an unknown upload or another tenant's; 503 when Postgres or the object
/// store is unavailable.
pub async fn preview(
    State(state): State<AppState>,
    Path(id): Path<String>,
    query: Result<Query<PreviewQuery>, axum::extract::rejection::QueryRejection>,
) -> ApiResult<ApiJson<PreviewResponse>> {
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
    match conversion_of_head(&row.storage_key, &head) {
        Some(Conversion::Workbook) => {
            let response =
                workbook_preview(&store, &row, query.sheet, overrides.header_row).await?;
            return Ok(ApiJson(response));
        }
        Some(Conversion::Parquet) => {
            return Ok(ApiJson(parquet_preview(&store, &row).await?));
        }
        None => {}
    }
    // A preview reads one chunk, so "20 rows" never implies the file has only
    // 20: `truncated` says the file goes on.
    let head_is_truncated = row.size_bytes > i64::try_from(PREVIEW_BYTES).unwrap_or(i64::MAX);
    Ok(ApiJson(PreviewResponse {
        preview: upload_parse::preview(&head, head_is_truncated, overrides, PREVIEW_ROWS),
        workbook: None,
        parquet: None,
    }))
}

/// The preview of a workbook: its sheet as text, read the way the load reads
/// it (UTF-8, comma).
async fn workbook_preview(
    store: &UploadStore,
    row: &Upload,
    sheet: Option<String>,
    header_row: Option<usize>,
) -> Result<PreviewResponse, ApiError> {
    let bytes = store.get_all(&row.storage_key).await?;
    let (info, name, csv, cut) = blocking(move || {
        let read = upload_workbook::read_sheet(&bytes, sheet.as_deref())?;
        let cut = read.data.rows() > WORKBOOK_PREVIEW_RECORDS;
        let csv = read.data.to_csv(Some(WORKBOOK_PREVIEW_RECORDS));
        Ok::<_, WorkbookError>((read.info, read.data.name().to_owned(), csv, cut))
    })
    .await?;
    let overrides = Overrides {
        encoding: Some(Encoding::Utf8),
        delimiter: Some(','),
        header_row,
    };
    Ok(PreviewResponse {
        preview: upload_parse::preview(&csv, cut, overrides, PREVIEW_ROWS),
        workbook: Some(WorkbookView { sheet: name, info }),
        parquet: None,
    })
}

/// The preview of a Parquet file: its first rows as text, read the way the load
/// reads them (UTF-8, comma, the first record is the header).
async fn parquet_preview(store: &UploadStore, row: &Upload) -> Result<PreviewResponse, ApiError> {
    let bytes = store.get_all(&row.storage_key).await?;
    let converted =
        blocking(move || upload_parquet::convert(&bytes, Some(PARQUET_PREVIEW_ROWS))).await?;
    let cut = converted.info.rows > u64::try_from(PARQUET_PREVIEW_ROWS).unwrap_or(u64::MAX);
    let overrides = Overrides {
        encoding: Some(Encoding::Utf8),
        delimiter: Some(','),
        header_row: Some(0),
    };
    Ok(PreviewResponse {
        preview: upload_parse::preview(&converted.csv, cut, overrides, PREVIEW_ROWS),
        workbook: None,
        parquet: Some(converted.info),
    })
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
    /// The sheet to load, for a workbook (`None`: its default sheet).
    sheet: Option<String>,
}

/// Why a raw table name is refused, or `None` (review finding C1).
///
/// The rule is `^[a-z][a-z0-9]*(_[a-z0-9]+)*$` and at most
/// [`MAX_TABLE_NAME_CHARS`] characters: a lower-case ASCII letter first, then
/// lower-case ASCII letters and digits in groups joined by single
/// underscores, so no leading, trailing or doubled `_`. It is checked below a
/// character at a time, left to right, because this crate has no regex
/// dependency to spend on it.
///
/// It is tighter than the console's rule for a connector's target
/// (`^[a-z_][a-z0-9_]*$`, which connectors keep) because the writer renames
/// some names that rule admits: `x_` is written as `xx`, `__x` as `x` and
/// `s__1` as `s___1`. The rows then land in a table nobody asked for, and
/// all the user would learn is that the load failed. Measured on 2026-10-02,
/// the writer's naming changed none of the 59,052 names this rule admits
/// (plan section 9, review of slice C); `file_ingest.py` keeps the writer's
/// own question as a second line (`_dlt_keeps_table_name`). `Orders` is
/// refused, not folded to `orders`.
///
/// The shared `Ident` is not asked as well: every name this rule admits is
/// already one (ASCII letters, digits and `_`, no leading digit), so it would
/// add no guarantee.
fn table_name_problem(name: &str) -> Option<&'static str> {
    let mut previous: Option<char> = None;
    for c in name.chars() {
        let allowed = match previous {
            None => c.is_ascii_lowercase(),
            // After `_` only a letter or a digit: never `__`.
            Some('_') => c.is_ascii_lowercase() || c.is_ascii_digit(),
            Some(_) => c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_',
        };
        if !allowed {
            return Some(TABLE_NAME_RULE);
        }
        previous = Some(c);
    }
    // An empty name never set `previous`, and one ending in `_` has an empty
    // last group. Every character that got here is ASCII, so the byte length
    // is the character count.
    let ends_in_a_group = previous.is_some_and(|last| last != '_');
    (!ends_in_a_group || name.len() > MAX_TABLE_NAME_CHARS).then_some(TABLE_NAME_RULE)
}

fn text_field<'a>(fields: &'a Map<String, Value>, name: &str) -> Result<&'a str, ApiError> {
    match fields.get(name) {
        Some(Value::String(text)) => Ok(text),
        None | Some(Value::Null) => Err(ApiError::BadRequest(format!("{name} is required."))),
        Some(_) => Err(ApiError::BadRequest(format!("{name} must be text."))),
    }
}

/// Validate the body of `POST /api/uploads/{id}/ingest`: `bronzeTable`,
/// `mode` (optional, `replace` by default), `encoding`, `delimiter`,
/// `headerRow` and, optional, `sheet`. All are checked; the load uses exactly
/// what was confirmed and nothing is guessed (ADR 0014, decision 3).
///
/// For a `conversion` (a workbook or a Parquet file) the dialect is not the
/// user's to choose: the converted text is always UTF-8 with commas, `encoding`
/// and `delimiter` are not read (a retry sends back what the upload recorded,
/// which may carry them). A workbook's `sheet` picks the sheet; a Parquet
/// file's header is its own column names, so `headerRow` is not read either
/// and is 0. For a text file `sheet` has no meaning and is only checked to be
/// text.
fn parse_ingest_request(
    body: &[u8],
    conversion: Option<Conversion>,
) -> Result<IngestRequest, ApiError> {
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
    let (encoding, delimiter) = if conversion.is_some() {
        (Encoding::Utf8, ',')
    } else {
        (
            Encoding::parse(text_field(&fields, "encoding")?).ok_or_else(|| bad(ENCODING_RULE))?,
            upload_parse::parse_delimiter(text_field(&fields, "delimiter")?)
                .ok_or_else(|| bad(DELIMITER_RULE))?,
        )
    };
    let header_row = match (conversion, fields.get("headerRow")) {
        (Some(Conversion::Parquet), _) => 0,
        (_, None | Some(Value::Null)) => return Err(bad("headerRow is required.")),
        (_, Some(value)) => value.as_u64().ok_or_else(|| bad(HEADER_ROW_RULE))?,
    };
    let sheet = match fields.get("sheet") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => Some(name.clone()),
        Some(_) => return Err(bad(SHEET_RULE)),
    };
    Ok(IngestRequest {
        table: table.to_owned(),
        mode,
        encoding,
        delimiter,
        header_row,
        sheet,
    })
}

/// Whether `table` may be loaded into by an upload of `tenant_id`, or why
/// not. The rule (ADR 0014, decision 5, review finding B4), in this order:
///
/// 1. A table a connector loads: never.
/// 2. The claim table's answer ([`uploads::table_claim`]): the name is this
///    tenant's, and it is free to load into, whether or not the table exists
///    yet; or it is another tenant's, and it is not.
/// 3. Nobody's claim: the table must not exist (a registry row or an Iceberg
///    table). Never "free" on a doubt.
///
/// This only READS the claim. A tenant that passes it still has to win
/// [`uploads::claim_table`], which [`ingest`] calls just before it marks the
/// upload as loading: two tenants can both pass this for one new name, and the
/// database decides between them there.
///
/// # Errors
///
/// 409 for a connector's table, for a name another tenant holds, and for an
/// existing table nobody claimed (the last two with one sentence,
/// [`TABLE_NOT_FREE`]); 503 when the question cannot be answered (Postgres,
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
    match uploads::table_claim(pool, tenant_id, table)
        .await
        .map_err(|err| unchecked(&err))?
    {
        // This tenant's own claim settles it, whether or not the table exists
        // yet: replacing or adding to one's own table is the point.
        TableClaim::Ours => return Ok(()),
        // Another tenant's claim, or a claim whose tenant is gone. The same
        // sentence as for an existing table nobody claimed, so the answer does
        // not say which it is.
        TableClaim::Theirs => return Err(ApiError::Conflict(TABLE_NOT_FREE.to_owned())),
        TableClaim::Unclaimed => {}
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
/// (plan T7). `load_key` is the object the job reads: the upload's own, or the
/// converted sheet of a workbook ([`converted_key`]). The config schema is
/// closed, so nothing about the sheet can be added here.
fn run_config(upload_id: &str, request: &IngestRequest, load_key: &str) -> Value {
    json!({
        "ops": {
            "ingest_uploaded_file": {
                "config": {
                    "upload_id": upload_id,
                    "storage_key": load_key,
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
    load_key: &str,
) -> Result<String, ApiError> {
    let failure = match state
        .dagster
        .launch_run_with_config(FILE_INGEST_JOB, &run_config(&claimed.id, request, load_key))
        .await
    {
        Ok(launched) => {
            if let Some(run_id) = launched.run_id {
                return Ok(run_id);
            }
            tracing::warn!(error = ?launched.failure, upload_id = %claimed.id, "the orchestrator refused to launch a file load");
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

/// An upload converted to delimited text and stored.
struct Converted {
    /// The object the job reads.
    key: String,
    /// A workbook's sheet, as the workbook spells it; `None` for a Parquet file.
    sheet: Option<String>,
}

/// Convert a workbook upload's chosen sheet, or a Parquet upload's table, and
/// store the text as a CSV beside the original. The object is written again by
/// the next load of the same upload, and removed with the upload ([`delete`]).
async fn convert_upload(
    state: &AppState,
    row: &Upload,
    conversion: Conversion,
    sheet: Option<&str>,
) -> Result<Converted, ApiError> {
    let store = UploadStore::connect(&state.config).await?;
    let bytes = store.get_all(&row.storage_key).await?;
    let (sheet, csv) = match conversion {
        Conversion::Workbook => {
            let asked = sheet.map(str::to_owned);
            let (name, empty, csv) = blocking(move || {
                let read = upload_workbook::read_sheet(&bytes, asked.as_deref())?;
                Ok::<_, WorkbookError>((
                    read.data.name().to_owned(),
                    read.data.is_empty(),
                    read.data.to_csv(None),
                ))
            })
            .await?;
            if empty {
                return Err(ApiError::BadRequest(EMPTY_SHEET.to_owned()));
            }
            (Some(name), csv)
        }
        Conversion::Parquet => {
            let converted = blocking(move || upload_parquet::convert(&bytes, None)).await?;
            if converted.info.rows == 0 {
                return Err(ApiError::BadRequest(EMPTY_PARQUET.to_owned()));
            }
            (None, converted.csv)
        }
    };
    let key = converted_key(&row.storage_key);
    store.put(&key, Bytes::from(csv)).await?;
    Ok(Converted { key, sheet })
}

/// `POST /api/uploads/{id}/ingest` — load this file into a raw table, with
/// the encoding, delimiter and header row the user confirmed. Body
/// `{ bronzeTable, mode?, encoding, delimiter, headerRow, sheet? }`; `mode` is
/// `replace` (default) or `append`. Returns `{ upload, runId }`.
///
/// For a workbook, `encoding` and `delimiter` are not read and `sheet` picks
/// the sheet (the default one when absent): the sheet is converted, stored as
/// a CSV beside the original ([`converted_key`]) after the table checks and
/// before the table is claimed, so a sheet that cannot be read claims
/// nothing, and the job is launched on that object. The sheet is recorded in
/// the upload's `parse_options` and in the audit event. A Parquet file is
/// converted the same way: `encoding`, `delimiter`, `headerRow` and `sheet`
/// are not read (the header is the file's column names, row 0), a file with no
/// rows is refused, and the audit event says `"format": "parquet"`.
///
/// # The order, and why
///
/// Validate the body; settle the upload if it is loading (a load that
/// finished must not block the next one); check the table is free
/// ([`ensure_table_free`]) and not being loaded by another upload; CLAIM THE
/// TABLE NAME ([`uploads::claim_table`]); CLAIM the row
/// ([`uploads::mark_ingesting`], no run id); launch; attach the run
/// ([`uploads::attach_run`]).
///
/// The table claim comes just before the row claim (review finding B4): the
/// checks above only read, so two tenants can both pass them for one new name,
/// and `claim_table` is where the database lets one of them through. It stands
/// from then on and is never released, even if the launch that follows fails
/// or the upload is deleted.
///
/// The row claim comes before the launch (review finding B2): two requests for
/// one upload sent at the same moment both pass the checks above, but only one
/// claim succeeds, so only one launches. When the orchestrator cannot be
/// reached or refuses, the claim is settled as failed before the answer.
///
/// # Errors
///
/// 400 for a body that is not valid (each field has its own sentence), and
/// for a workbook or Parquet file that cannot be read, a sheet a workbook does
/// not have, a sheet or file over the cell limit, an empty sheet or a Parquet
/// file with no rows; 404
/// for an unknown upload or another tenant's; 409 when this upload is already
/// loading, another is loading into that table, a connector loads it, or the
/// name is in use and no upload of this tenant created it ([`TABLE_NOT_FREE`]:
/// another tenant holds the claim, the table exists and nobody claimed it, or
/// another tenant won the claim a moment ago); 422 when the orchestrator
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
    let pool = pool(&state)?;
    let row = uploads::get(pool, &id).await?.ok_or_else(not_found)?;
    // The row first: what the body must carry depends on what the upload is.
    let conversion = conversion_of(&state, &row).await?;
    let request = parse_ingest_request(&body, conversion)?;
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
    let converted = match conversion {
        Some(kind) => Some(convert_upload(&state, &row, kind, request.sheet.as_deref()).await?),
        None => None,
    };
    let load_key = converted
        .as_ref()
        .map_or(row.storage_key.as_str(), |done| done.key.as_str());
    // Review finding B4: the name is claimed here, in one statement the
    // database arbitrates, and not before. `false` is a tenant that took the
    // name between the checks above and now, or one whose claim was always
    // there and whose own tenant is gone: the same refusal as a claim seen
    // earlier. Nothing is marked or launched for it.
    if !uploads::claim_table(pool, tenant_id, &request.table, &row.id).await? {
        return Err(ApiError::Conflict(TABLE_NOT_FREE.to_owned()).into());
    }

    let mut parse_options = json!({
        "encoding": request.encoding.as_str(),
        "delimiter": request.delimiter.to_string(),
        "headerRow": request.header_row,
    });
    if let (
        Value::Object(map),
        Some(Converted {
            sheet: Some(sheet), ..
        }),
    ) = (&mut parse_options, &converted)
    {
        map.insert("sheet".to_owned(), json!(sheet));
    }
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

    let run_id = launch(&state, pool, &claimed, &request, load_key).await?;
    let Some(attached) = uploads::attach_run(pool, &claimed.id, &run_id).await? else {
        // Not reachable through this API: only an `ingesting` row with no run
        // takes a run, and nothing settles or deletes the claim in the
        // meantime. If it ever happens a run exists that the row does not name.
        tracing::error!(upload_id = %claimed.id, %run_id, "a launched file load could not be attached to its upload");
        return Err(ApiError::Internal(NOT_RECORDED.to_owned()).into());
    };

    let mut args = json!({
        "fileName": attached.original_filename,
        "sizeBytes": attached.size_bytes,
        "table": request.table,
        "mode": request.mode.as_str(),
    });
    if let Value::Object(map) = &mut args {
        match (&converted, conversion) {
            (
                Some(Converted {
                    sheet: Some(sheet), ..
                }),
                _,
            ) => {
                map.insert("sheet".to_owned(), json!(sheet));
            }
            (Some(_), Some(Conversion::Parquet)) => {
                map.insert("format".to_owned(), json!("parquet"));
            }
            _ => {}
        }
    }
    let event = upload_audit_event(&principal, "upload.ingest", &attached.id, args, "executed");
    record_audit(pool, event).await;
    Ok(ApiJson(IngestResponse {
        upload: attached.into(),
        run_id,
    }))
}

// ── Delete ───────────────────────────────────────────────────────────────

/// `DELETE /api/uploads/{id}` — remove the object (and the converted text of a
/// workbook or Parquet file), then the row. 204. The
/// table the upload became is not touched, and neither is the claim on its
/// name: that is a record of its own ([`uploads::claim_table`]) and is never
/// released, so a later upload of the tenant can still load into the table and
/// a connector still cannot take it.
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
    if conversion_for_name(&row.storage_key).is_some() {
        // The converted text of a workbook or a Parquet file, if a load wrote
        // one: derived data goes first, and a key that is not there is
        // success. A text file named `.xls` or `.parquet` has none, and
        // deleting a missing key is a no-op.
        store.delete(&converted_key(&row.storage_key)).await?;
    }
    store.delete(&row.storage_key).await?;
    if !uploads::delete(pool, &row.id).await? {
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

    /// A tenant id, which is what `create` builds the key from
    /// (`tenant_id.to_string()`): review finding B7, these tests used a tenant
    /// name from this deployment's defaults, and no key is made from a name.
    const TENANT: &str = "5f0c2b7e-9a41-4c6d-8e3b-2d7a91c4f6a0";

    #[test]
    fn storage_key_never_uses_the_users_filename() {
        let key = storage_key(TENANT, "up-1", "../../etc/passwd");
        assert_eq!(key, format!("uploads/{TENANT}/up-1"));
        let key = storage_key(TENANT, "up-2", "rawdata.xls");
        assert_eq!(
            key,
            format!("uploads/{TENANT}/up-2.xls"),
            "a tenant id passes through the sanitiser unchanged"
        );
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
        let refusal = |bytes: &[u8]| check_file(bytes, "x.csv").unwrap_err().to_string();
        assert_eq!(refusal(b""), "The file is empty.");
        assert_eq!(
            check_file(b"id,name\n1,a\n", "x.csv").unwrap(),
            Accepted::Text
        );
        assert!(check_file(&[b'a'; 1024], "x.csv").is_ok());
        // X2 of the Excel plan, then Q2 of the Parquet plan: the sentence
        // changed with what is accepted.
        assert_eq!(
            refusal(b"PK\x03\x04rest of a zip"),
            "This looks like a workbook or a zip archive that is not an .xls or .xlsx file. Only .xls and .xlsx workbooks, .parquet files and delimited text files (CSV, TSV) can be uploaded; save the sheet as .xlsx or CSV first."
        );
        assert!(refusal(&[0xD0, 0xCF, 0x11, 0xE0, 1, 2, 3, 4]).contains("workbook"));
        assert_eq!(
            refusal(b"PAR1\x15\x04rest"),
            "This looks like a Parquet file, but its name does not end in .parquet. Rename it to end in .parquet and upload it again."
        );
        assert!(refusal(b"%PDF-1.7 ...").contains("not a delimited text file"));
        assert!(refusal(b"a,b\0c,d").contains("not a delimited text file"));
    }

    /// A Parquet file is accepted by its first bytes AND a `.parquet` name;
    /// text keeps its name's freedom; a Parquet file under another name is
    /// refused, and so is a binary file that merely has the name.
    #[test]
    fn a_parquet_file_needs_both_its_first_bytes_and_a_parquet_name() {
        let parquet = b"PAR1\x15\x04\x15\x30rest of a parquet file";
        for name in ["a.parquet", "A.PARQUET", "dir/a.b.parquet"] {
            assert_eq!(
                check_file(parquet, name).unwrap(),
                Accepted::Parquet,
                "{name}"
            );
            assert_eq!(
                check_file(b"id,name\n1,a\n", name).unwrap(),
                Accepted::Text,
                "text called {name} is text"
            );
            assert!(
                check_file(b"%PDF-1.7 ...", name).is_err(),
                "a PDF called {name} is not a Parquet file"
            );
        }
        for name in ["a.csv", "a.parq", "a.parquet.gz", "a", ""] {
            assert!(check_file(parquet, name).is_err(), "{name}");
        }
        // An encrypted file starts `PARE`: named `.parquet` it is taken so the
        // reader can say why it is refused; named anything else it is not.
        let encrypted = b"PARE\x00\x01\x02\x03 encrypted";
        assert_eq!(
            check_file(encrypted, "a.parquet").unwrap(),
            Accepted::Parquet
        );
        assert!(check_file(encrypted, "a.csv").is_err());
    }

    #[test]
    fn what_a_stored_upload_is_converted_from_follows_its_name_and_its_first_bytes() {
        let zip = b"PK\x03\x04rest";
        let parquet = b"PAR1\x15\x04rest";
        let text = b"id,name\n1,a\n";
        assert_eq!(
            conversion_of_head("uploads/t/up-1.xlsx", zip),
            Some(Conversion::Workbook)
        );
        assert_eq!(
            conversion_of_head("uploads/t/up-1.parquet", parquet),
            Some(Conversion::Parquet)
        );
        assert_eq!(
            conversion_of_head("uploads/t/up-1.parquet", b"PARE\x00rest"),
            Some(Conversion::Parquet),
            "an encrypted file is still read by the Parquet reader, which refuses it"
        );
        // Text called by either name is text; each kind under the other's name
        // is not converted either.
        for key in ["uploads/t/up-1.xlsx", "uploads/t/up-1.parquet"] {
            assert_eq!(conversion_of_head(key, text), None, "{key}");
        }
        assert_eq!(conversion_of_head("uploads/t/up-1.parquet", zip), None);
        assert_eq!(conversion_of_head("uploads/t/up-1.xlsx", parquet), None);
        assert_eq!(conversion_of_head("uploads/t/up-1.csv", parquet), None);
        assert_eq!(conversion_of_head("uploads/t/up-1", zip), None);
    }

    #[test]
    fn a_parquet_load_reads_neither_a_dialect_nor_a_header_row() {
        let bare = json!({ "bronzeTable": "orders_raw" });
        let request = parse_ingest_request(&body(&bare), Some(Conversion::Parquet)).unwrap();
        assert_eq!(request.encoding, Encoding::Utf8);
        assert_eq!(request.delimiter, ',');
        assert_eq!(request.header_row, 0);
        // A retry sends back what it recorded; none of it is read.
        let mut sent = bare.clone();
        sent["headerRow"] = json!(5);
        sent["encoding"] = json!("utf-16");
        sent["delimiter"] = json!(";");
        let request = parse_ingest_request(&body(&sent), Some(Conversion::Parquet)).unwrap();
        assert_eq!((request.header_row, request.encoding), (0, Encoding::Utf8));
        // The rest of the body is checked as for any upload.
        let mut bad_table = bare;
        bad_table["bronzeTable"] = json!("Orders");
        assert_eq!(
            parse_ingest_request(&body(&bad_table), Some(Conversion::Parquet))
                .unwrap_err()
                .to_string(),
            TABLE_NAME_RULE
        );
    }

    /// A workbook is accepted by its first bytes AND an `.xls` or `.xlsx`
    /// name; text keeps its name's freedom (the motivating export was text
    /// called `.xls`); a zip or OLE file with any other name is refused.
    #[test]
    fn a_workbook_needs_both_its_first_bytes_and_a_workbook_name() {
        let zip = b"PK\x03\x04rest of a zip";
        let ole = [0xD0, 0xCF, 0x11, 0xE0, 1, 2, 3, 4];
        for name in ["a.xlsx", "A.XLSX", "dir/a.xls", "a.b.xlsx"] {
            assert_eq!(check_file(zip, name).unwrap(), Accepted::Workbook, "{name}");
            assert_eq!(
                check_file(&ole, name).unwrap(),
                Accepted::Workbook,
                "{name}"
            );
        }
        for name in [
            "a.xlsm", "a.xlsb", "a.ods", "a.zip", "a.csv", "a", "a.docx", "",
        ] {
            assert!(check_file(zip, name).is_err(), "{name}");
            assert!(check_file(&ole, name).is_err(), "{name}");
        }
        for name in ["a.xls", "a.xlsx", "a.csv", "a"] {
            assert_eq!(check_file(b"id,name\n1,a\n", name).unwrap(), Accepted::Text);
        }
    }

    #[test]
    fn a_converted_sheet_is_stored_beside_the_original_inside_the_prefix() {
        let key = storage_key(TENANT, "up-9", "Stock.XLSX");
        assert_eq!(key, format!("uploads/{TENANT}/up-9.xlsx"));
        assert_eq!(
            converted_key(&key),
            format!("uploads/{TENANT}/up-9.xlsx.converted.csv")
        );
        assert!(converted_key(&key).starts_with(&format!("{PREFIX}/")));
        assert!(has_workbook_name(&key));
        assert!(!has_workbook_name(&converted_key(&key)));
        assert_eq!(conversion_for_name(&key), Some(Conversion::Workbook));
        let parquet_key = storage_key(TENANT, "up-10", "Orders.PARQUET");
        assert_eq!(parquet_key, format!("uploads/{TENANT}/up-10.parquet"));
        assert_eq!(conversion_for_name(&parquet_key), Some(Conversion::Parquet));
        assert_eq!(conversion_for_name(&converted_key(&parquet_key)), None);
        assert!(!has_workbook_name("uploads/t/up-1.csv"));
        assert!(!has_workbook_name("uploads/t/up-1"));
    }

    #[test]
    fn a_file_over_the_cap_is_refused_with_the_fixed_text() {
        let over = vec![b'a'; MAX_UPLOAD_BYTES + 1];
        assert_eq!(
            check_file(&over, "x.csv").unwrap_err().to_string(),
            "The file is larger than the 50 MB limit."
        );
        assert!(check_file(&over[..MAX_UPLOAD_BYTES], "x.csv").is_ok());
    }

    #[test]
    fn the_request_body_limit_leaves_room_for_the_form_around_the_file() {
        assert_eq!(MAX_REQUEST_BODY_BYTES - MAX_UPLOAD_BYTES, 1024 * 1024);
    }

    // ── the table name ──────────────────────────────────────────────────

    /// Review finding C1: a lower-case letter, then groups of lower-case
    /// letters and digits joined by single underscores. `_staging` was in the
    /// accepted list before this finding; `_x`, `x_` and `a__b` are the names
    /// the plan says must now be refused.
    #[test]
    fn a_table_name_is_a_lower_case_letter_then_groups_joined_by_single_underscores() {
        for ok in [
            "a",
            "a1",
            "a_1",
            "sap_material_master",
            "orders",
            "orders_raw",
            "t2025",
            "a_1_b",
            "g9_upload_0a1b2c3d",
            "a1_b2_c3",
        ] {
            assert_eq!(table_name_problem(ok), None, "{ok}");
        }
        for refused in [
            // The plan's refusals.
            "x_",
            "_x",
            "a__b",
            "1a",
            "Orders",
            // What the rule before C1 admitted and the writer renames:
            // `__x` as `x`, `s__1` as `s___1`, `a__` as `a`.
            "__x",
            "s__1",
            "a__",
            "_",
            "__",
            "_a_",
            "a_1_",
            "_staging",
            // Never valid.
            "",
            "orders 2025",
            "2025_orders",
            "orders-raw",
            "orders.raw",
            "orders;drop",
            "pesanan\u{e9}",
            " orders",
            "orders ",
            "orders\n",
            "ORDERS",
            "a\u{ff10}", // a fullwidth digit is not an ASCII digit
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
        // The bound counts the whole name, the underscores included.
        let grouped = format!("{}_{}", "a".repeat(64), "b".repeat(63));
        assert_eq!(grouped.len(), 128);
        assert_eq!(table_name_problem(&grouped), None);
        assert_eq!(
            table_name_problem(&format!("{grouped}b")),
            Some(TABLE_NAME_RULE)
        );
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
        parse_ingest_request(&body(value), None)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn a_complete_body_is_accepted_and_mode_defaults_to_replace() {
        let request = parse_ingest_request(&body(&valid()), None).unwrap();
        assert_eq!(
            request,
            IngestRequest {
                table: "stock_raw".to_owned(),
                mode: LoadMode::Replace,
                encoding: Encoding::Utf8,
                delimiter: ',',
                header_row: 0,
                sheet: None,
            }
        );
        let mut full = valid();
        full["mode"] = json!("append");
        full["encoding"] = json!("utf-16");
        full["delimiter"] = json!("\t");
        full["headerRow"] = json!(4);
        let request = parse_ingest_request(&body(&full), None).unwrap();
        assert_eq!(request.mode, LoadMode::Append);
        assert_eq!(request.encoding, Encoding::Utf16);
        assert_eq!(request.delimiter, '\t');
        assert_eq!(request.header_row, 4);
        let mut null_mode = valid();
        null_mode["mode"] = Value::Null;
        assert_eq!(
            parse_ingest_request(&body(&null_mode), None).unwrap().mode,
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

    /// Review finding C1, through the validation the route runs: the names the
    /// plan says must be refused get the plan's sentence, and the ones it says
    /// must be accepted reach the request unchanged.
    #[test]
    fn the_ingest_body_refuses_and_accepts_the_table_names_the_plan_lists() {
        let with_table = |table: &str| {
            let mut v = valid();
            v["bronzeTable"] = json!(table);
            v
        };
        let longest = "a".repeat(128);
        for ok in ["a", "a1", "a_1", "sap_material_master", longest.as_str()] {
            let request = parse_ingest_request(&body(&with_table(ok)), None).unwrap();
            assert_eq!(request.table, ok);
        }
        let too_long = "a".repeat(129);
        for refused in ["x_", "_x", "a__b", "1a", "Orders", too_long.as_str()] {
            assert_eq!(
                refusal_of(&with_table(refused)),
                "Table names start with a lower-case letter and use lower-case letters and digits joined by single underscores, with at most 128 characters.",
                "{refused:?}"
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
            let err = parse_ingest_request(raw, None).unwrap_err();
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
            sheet: None,
        }
    }

    // ── a workbook's ingest body ────────────────────────────────────────

    #[test]
    fn a_workbook_load_is_always_utf8_and_comma_and_names_its_sheet() {
        let mut v = valid();
        v["encoding"] = json!("utf-16");
        v["delimiter"] = json!(";");
        v["sheet"] = json!("Quirks");
        let request = parse_ingest_request(&body(&v), Some(Conversion::Workbook)).unwrap();
        assert_eq!(request.encoding, Encoding::Utf8);
        assert_eq!(request.delimiter, ',');
        assert_eq!(request.sheet.as_deref(), Some("Quirks"));
        // Not read at all, so not required either.
        let bare = json!({ "bronzeTable": "stock_raw", "headerRow": 2 });
        let request = parse_ingest_request(&body(&bare), Some(Conversion::Workbook)).unwrap();
        assert_eq!(request.header_row, 2);
        assert_eq!(request.sheet, None);
        // Still required and checked for a text file.
        assert_eq!(
            parse_ingest_request(&body(&bare), None)
                .unwrap_err()
                .to_string(),
            "encoding is required."
        );
        // The rest of the body is checked for a workbook as for text.
        let mut bad_table = bare.clone();
        bad_table["bronzeTable"] = json!("Orders");
        assert_eq!(
            parse_ingest_request(&body(&bad_table), Some(Conversion::Workbook))
                .unwrap_err()
                .to_string(),
            TABLE_NAME_RULE
        );
        // The sheet is checked last, after the fields a text file needs, so
        // the text case carries a complete body (CI failure on fe3dffc: the
        // test sent the bare body, and a text upload is rightly asked for its
        // encoding first).
        let mut bad_sheet = bare;
        bad_sheet["sheet"] = json!(3);
        let mut bad_text_sheet = valid();
        bad_text_sheet["sheet"] = json!(3);
        for (conversion, body_with_bad_sheet) in [
            (Some(Conversion::Workbook), &bad_sheet),
            (None, &bad_text_sheet),
        ] {
            assert_eq!(
                parse_ingest_request(&body(body_with_bad_sheet), conversion)
                    .unwrap_err()
                    .to_string(),
                "sheet must be text."
            );
        }
    }

    #[test]
    fn the_job_is_told_the_converted_object_and_the_fixed_dialect() {
        let request = parse_ingest_request(
            &body(&json!({ "bronzeTable": "stock_raw", "headerRow": 1, "sheet": "Stock" })),
            Some(Conversion::Workbook),
        )
        .unwrap();
        let config = run_config("up-1", &request, "uploads/t/up-1.xlsx.converted.csv");
        let job = &config["ops"]["ingest_uploaded_file"]["config"];
        assert_eq!(job["upload_id"], "up-1");
        assert_eq!(job["storage_key"], "uploads/t/up-1.xlsx.converted.csv");
        assert_eq!(job["encoding"], "utf-8");
        assert_eq!(job["delimiter"], ",");
        assert_eq!(job["header_row"], 1);
        assert!(
            job.get("sheet").is_none(),
            "the job's config schema is closed"
        );
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

    /// Review finding C3: `ended_after` reads what the job writes.
    /// `ingest_run.ended_at` is a text column, so the API gets the characters
    /// of Python's `datetime.now(timezone.utc).isoformat()` as they were
    /// written: six digits of microseconds and `+00:00`, never `Z`, and no
    /// fraction at all when the microsecond is 0. The route tests write `Z`
    /// only. The claim's time is Postgres's `updated_at`, which has
    /// microseconds too, so a result in the same second as the claim is
    /// ordered by its fraction.
    #[test]
    fn a_timestamp_the_job_writes_is_read_to_the_microsecond() {
        // `datetime(2026, 10, 2, 10, 0, 0, 123456, tzinfo=timezone.utc).isoformat()`
        let written = "2026-10-02T10:00:00.123456+00:00";
        for (claim, later) in [
            ("2026-10-02T09:59:59.999999Z", true),
            ("2026-10-02T10:00:00.123455Z", true),
            ("2026-10-02T10:00:00.123456Z", false),
            ("2026-10-02T10:00:00.123457Z", false),
            ("2026-10-02T10:00:01Z", false),
        ] {
            assert_eq!(
                ended_after(written, at(claim)),
                later,
                "{written} after {claim}"
            );
        }
        // `datetime(2026, 10, 2, 10, 0, 5, 0, tzinfo=timezone.utc).isoformat()`:
        // a whole second is written with no fraction.
        let whole = "2026-10-02T10:00:05+00:00";
        for (claim, later) in [
            ("2026-10-02T10:00:04.999999Z", true),
            ("2026-10-02T10:00:05Z", false),
            ("2026-10-02T10:00:05.000001Z", false),
        ] {
            assert_eq!(
                ended_after(whole, at(claim)),
                later,
                "{whole} after {claim}"
            );
        }
        // `+00:00` and `Z` are one instant, to the microsecond.
        assert_eq!(at(written), at("2026-10-02T10:00:00.123456Z"));
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
    fn any_status_but_succeeded_is_failed_and_a_known_recorded_reason_is_kept() {
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
            Outcome::Failed("The file has no rows below the header row.".to_owned()),
            "surrounding white space does not make it another reason"
        );
        assert_eq!(
            outcome_of(&result("failed", Some(10), "")),
            Outcome::Failed("The load failed.".to_owned()),
            "no reason recorded: a fixed one, and no count"
        );
    }

    /// Review finding B6: a recorded reason reaches a response only when it is
    /// one of the seven the API knows (six before finding C2). Each of the
    /// seven is kept; text that is near one of them, text with detail added,
    /// and exception text are all the fixed `The load failed.`.
    #[test]
    fn a_recorded_reason_is_shown_only_when_it_is_one_the_api_knows() {
        for known in JOB_FAILURE_REASONS {
            assert_eq!(
                outcome_of(&result("failed", None, known)),
                Outcome::Failed(known.to_owned()),
                "{known}"
            );
        }
        for other in [
            "KeyError: 'amount' at /app/dagster/dispar_orchestrate/file_ingest.py:88",
            "The load into the table failed. (OSError: [Errno 111] Connection refused)",
            "Traceback (most recent call last):",
            "the load into the table failed.",
            "The load into the table failed",
            "The stored file could not be read",
            "The header row has no columns",
            "the header row has no columns.",
            "The header row is empty.",
            "The file has more than 2000000 rows.",
            "The load stopped before it recorded a result.",
            "x",
            "",
            "   ",
        ] {
            assert_eq!(
                outcome_of(&result("failed", None, other)),
                Outcome::Failed(LOAD_FAILED.to_owned()),
                "{other:?}"
            );
        }
    }

    /// Review findings B6 and C2: the seven reasons the API knows are the
    /// seven in the file the job's own tests assert its constants against
    /// too, in the same order, so the two sides cannot drift apart
    /// unnoticed. Read here from test code only, with `include_str!`, so the
    /// release build of the API depends on nothing outside `rust/`.
    #[test]
    fn the_reasons_the_api_knows_are_the_reasons_in_the_shared_fixture() {
        const SHARED: &str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../ops/fixtures/upload_load_failure_reasons.json"
        ));
        let in_file: Vec<String> = serde_json::from_str(SHARED).unwrap();
        assert_eq!(in_file, JOB_FAILURE_REASONS);

        let mut distinct = JOB_FAILURE_REASONS.to_vec();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), 7, "seven different reasons");
        assert!(
            !JOB_FAILURE_REASONS.contains(&LOAD_FAILED),
            "the fixed fallback is not one of the job's reasons"
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

    /// Review finding B5: a run the orchestrator does not know is failed once
    /// its claim is more than an hour old, and not before.
    #[test]
    fn a_lost_run_is_overdue_after_one_hour_and_not_before() {
        let claimed = at("2026-10-02T10:00:00Z");
        assert!(!unknown_run_is_overdue(claimed, at("2026-10-02T10:30:00Z")));
        assert!(!unknown_run_is_overdue(claimed, at("2026-10-02T11:00:00Z")));
        assert!(unknown_run_is_overdue(claimed, at("2026-10-02T11:00:01Z")));
        assert!(
            !unknown_run_is_overdue(claimed, at("2026-10-02T09:00:00Z")),
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
        assert_eq!(RUN_UNKNOWN, "The orchestrator no longer knows this load.");
        assert_eq!(
            TABLE_NOT_FREE,
            "That table name is in use and no upload of this tenant created it, so a file cannot be loaded into it."
        );
        // T7a, review findings C1 and C2.
        assert_eq!(
            TABLE_NAME_RULE,
            "Table names start with a lower-case letter and use lower-case letters and digits joined by single underscores, with at most 128 characters."
        );
        assert_eq!(JOB_HEADER_NO_COLUMNS, "The header row has no columns.");
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
            EMPTY_PARQUET,
            CONVERSION_FAILED,
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
            COULD_NOT_START,
            NOT_STARTED,
            NO_RESULT,
            LOAD_FAILED,
            RUN_UNKNOWN,
            JOB_FILE_UNREADABLE,
            JOB_HEADER_PAST_END,
            JOB_HEADER_NO_COLUMNS,
            JOB_NO_ROWS,
            JOB_TOO_MANY_ROWS,
            JOB_LOAD_FAILED,
            JOB_NOT_REGISTERED,
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
