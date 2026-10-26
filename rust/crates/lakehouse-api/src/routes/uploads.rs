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
//!    A large file, or several, takes the chunked route instead (`DATA-17`,
//!    [`sessions`]): `POST /api/uploads/sessions`, `PUT .../parts/{n}`,
//!    `GET .../parts`, `POST .../complete`. It ends in the same state, an
//!    `uploaded` file, after the same checks and the same sentences, so every
//!    step below works on either.
//! 4. `GET /api/uploads` and `GET /api/uploads/{id}` report the outcome. The
//!    job writes nothing to Postgres (ADR 0014, decision 5): the API reads its
//!    result back from `lake.bronze_meta.ingest_run` and settles the row when
//!    it is read ([`Settler`]).
//! 5. `DELETE /api/uploads/{id}` removes the object and the row. The table the
//!    upload became stays, and so does the claim on its name.
//!
//! # Folders (`DATA-35`)
//!
//! The Uploaded files tab groups files in folders ([`folders`]): the folder routes,
//! `POST /api/uploads/move`, and `folderId` on `GET /api/uploads` and on an ingest.
//! A folder is a grouping only; a file's table does not change with it.
//!
//! # Chunked uploads (`DATA-17`, plan AD1)
//!
//! `POST /api/uploads` reads the whole file in one request, and the proxy in
//! front of the API cuts a body past 51 MiB, so it stays at 50 MiB for API
//! callers and scripts (feature page decision 14). The console uses the
//! chunked route: the file arrives as 8 MiB pieces, each stored as one part of
//! an S3 multipart upload, with the multipart id and every piece's `ETag` and
//! SHA-256 in Postgres (`file_upload_part`, migration `0090`), so a restart
//! loses nothing and any API process can take the next piece. The file is
//! `receiving` until it completes; preview and ingest refuse it (409) and
//! delete aborts it. The protocol, its memory bound and the limit it shares
//! with `POST /api/uploads` are written at the top of [`sessions`]; the
//! hourly sweep that removes unfinished sessions is `crate::upload_sweep`.
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
//!   eleven the API knows ([`JOB_FAILURE_REASONS`], review findings B6 and C2);
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
use lakehouse_store::upload_folders;
use lakehouse_store::uploads::{
    self, FolderFilter, ListFilter, LoadClaim, LoadLimits, LoadMode, MarkOutcome, NewUpload,
    TableClaim, Upload,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use super::catalog::registered_slug;
use super::catalog_source::{PresenceUnknown, iceberg_table_presence};
use super::governance::{IngestRunRow, ingest_runs_or_empty};
use super::lakehouse::is_unknown_table_or_database_error;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::upload_formats::{self, ByName, Compression, Format, Layout, Refusal};
use crate::upload_limits::too_many_uploads;
use crate::upload_parquet::{self, ParquetInfo};
use crate::upload_parquet_range::RangeReader;
use crate::upload_parse::{self, Encoding, Kind, Overrides, Preview};
use crate::upload_store::{PREFIX, UploadStore};
use crate::upload_workbook::{self, WorkbookError, WorkbookInfo};

pub(super) mod columns;
pub(super) mod folders;
pub(super) mod group;
pub(super) mod sessions;

use columns::{ColumnPlan, Sample, SpecView};

/// Largest file this endpoint accepts, in bytes.
///
/// The body is buffered in memory to hash and store it, so this is a memory
/// bound as much as a product limit: ten concurrent uploads at the cap is
/// 500 MB of API process (ADR 0014, decision 2). The message says "50 MB"
/// and means 50 MiB, as the plan words it.
pub(super) const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;

/// The most bytes a gzip or zip file, or the files of one load together, may
/// unpack to or add up to: 2 GiB, exactly 2,147,483,648. Its own constant and
/// not [`MAX_UPLOAD_BYTES`], which means the `POST /api/uploads` file size
/// only: the API's gzip and zip check, the grouped-load total
/// (`group::members`) and the `max_bytes` that [`run_config`] sends to the
/// job all read this one, so the three cannot disagree. Before it, the job was
/// told 50 MiB and refused any gzip above that once unpacked, a size the
/// session path already took (found in the measurement of
/// `docs/plans/DATA-17-RESULT.md`). A bomb is decoded by range and counted,
/// never held, so the size costs time and not memory. Plan T6.2, item 2.
pub(super) const MAX_UNPACKED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

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
const WORKBOOK_PREVIEW_RECORDS: usize = columns::SAMPLE_ROWS + 50;

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
/// [`MAX_UNPACKED_BYTES`] cap, not a measurement of the bound itself: the loads
/// of `docs/plans/DATA-17-RESULT.md` took under two minutes at 2 GiB on one
/// runner, so an hour leaves a wide margin. It only has to be long enough that a
/// run which is merely slow, or about to start, is not failed under its own feet.
const UNKNOWN_RUN_BOUND: time::Duration = time::Duration::hours(1);

/// How long a read waits for the orchestrator or for `ClickHouse` while it
/// settles a loading upload. A bound so that a page that polls is not held
/// for the whole request deadline by a hung connection; not a measurement.
/// The upload is returned unchanged when it elapses.
const SETTLE_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// The Dagster job that loads an uploaded file
/// (`dagster/dispar_orchestrate/file_ingest.py`).
pub(super) const FILE_INGEST_JOB: &str = "file_ingest_job";

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
const WORKBOOK: &str = "This looks like a workbook or a zip archive that is not named like one. Excel workbooks must end in .xls or .xlsx and zip archives in .zip; or save the sheet as .xlsx or CSV first.";
const PARQUET: &str = "This looks like a Parquet file, but its name does not end in .parquet. Rename it to end in .parquet and upload it again.";
const OTHER_BINARY: &str = "This is not a file this upload reads. Upload CSV, TSV, JSON, JSON Lines, Avro, Parquet or Excel (.xls, .xlsx) files, a .gz of a text or JSON file, or a .zip of such files.";
const GZIP: &str = "This looks like a gzip file, but its name does not end in .gz. Rename it to end in .gz (for example data.csv.gz) and upload it again.";
const AVRO: &str = "This looks like an Avro file, but its name does not end in .avro. Rename it to end in .avro and upload it again.";
const NOT_AVRO: &str = "This file is named .avro but is not an Avro file.";
const NOT_AN_ARCHIVE: &str =
    "This file is named like a .gz or .zip archive but is not one. Rename it to match what it is.";
const ZIP_COLUMNS_DIFFER: &str = "The files of the zip archive do not all have the same columns. Remove the ones that differ before loading.";
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
const ALSO_UPLOADS_RULE: &str =
    "alsoUploads must be a list of at most 9 upload ids, each used once, none of them this upload.";
const FOLDER_ID_RULE: &str = "folderId must be text or null.";
const OTHER_BUSY: &str = "One of the other files is already being loaded.";
const ENCODING_RULE: &str = "encoding must be utf-8 or utf-16.";
const DELIMITER_RULE: &str = "delimiter must be a comma, a semicolon, a tab or a pipe.";
const HEADER_ROW_RULE: &str = "headerRow must be a whole number, 0 or more.";
const QUERY_UNREADABLE: &str = "The query string could not be read.";
const ALREADY_LOADING: &str = "This upload is already being loaded.";
const TABLE_BUSY: &str = "Another upload is loading into that table.";
const CONNECTOR_TABLE: &str = "A connector loads that table, so a file cannot be loaded into it.";
/// The one sentence for a table name that is not this tenant's to load into:
/// another tenant holds the claim on it, another tenant's connector loads it,
/// a lost claim, or it exists and nobody claimed it. Deliberately the same
/// for all of them, so that the answer does not tell a caller which it is
/// (review finding B4, `SEC-16`): a tenant must not be able to learn what
/// other tenants have claimed or connected, or which names are theirs. It
/// names no reason and no connector. The constant keeps its name; the text is
/// `SEC-16`'s, replacing a sentence that said "no upload of this tenant
/// created it".
const TABLE_NOT_FREE: &str = "That table name cannot be used. Choose another name.";
const TABLE_UNCHECKED: &str =
    "Could not check whether that table already exists, so nothing was loaded.";
const NO_QUERY_DATABASE: &str = "Uploads need ICEBERG_QUERY_DB to be set, so the API can check that a table name is free. Nothing was loaded.";
const LAUNCH_UNREACHABLE: &str =
    "The orchestrator could not be reached, so the load was not started.";
const LAUNCH_REFUSED: &str = "The orchestrator refused to start the load.";
const NOT_RECORDED: &str = "The load was started but could not be recorded.";
const DELETE_WHILE_LOADING: &str = "This upload is being loaded, so it cannot be deleted yet.";
/// A chunked upload (`DATA-17`) whose pieces are still arriving can be
/// neither previewed nor loaded: its object does not exist yet.
const STILL_RECEIVING: &str = "This upload is still being received, so it cannot be used yet.";

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
// shows a recorded reason only when it is one of these eleven (review finding
// B6): `ingest_run.error` is free text, so one `str(exc)` in the job would
// otherwise put exception text, a host or a path into a response. These are
// the same eleven as `ops/fixtures/upload_load_failure_reasons.json`, which a
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
// `SEC-17`: the one sentence for the column cap, as the preview's and the
// ingest's 400 and as the job's failure reason (`TOO_MANY_COLUMNS` in
// `file_ingest.py`). One constant, so the three cannot word it differently.
const JOB_TOO_MANY_COLUMNS: &str = "The file has more than 1,000 columns.";
// `DATA-18`: a load told to stop at a value that does not fit its column's type.
const JOB_BAD_VALUES: &str = "Some values do not fit their column's type.";
// `DATA-17` slice 4 (decision 2): the decoded bytes of a gzip or zip file passed the cap.
const JOB_TOO_LARGE_UNPACKED: &str = "The file is larger than the upload limit once unpacked.";
// PR #186 review SHOULD-FIX 1: the column list sent with the load is not the file's (a JSON key that
// first appears after the rows the preview sampled), so the person is told to look at the preview again.
const JOB_COLUMNS_NOT_PREVIEWED: &str = "The file's columns are not the ones the preview showed.";

/// Every reason the job may record, in the order of
/// `ops/fixtures/upload_load_failure_reasons.json`.
const JOB_FAILURE_REASONS: [&str; 11] = [
    JOB_FILE_UNREADABLE,
    JOB_HEADER_PAST_END,
    JOB_HEADER_NO_COLUMNS,
    JOB_NO_ROWS,
    JOB_TOO_MANY_ROWS,
    JOB_LOAD_FAILED,
    JOB_NOT_REGISTERED,
    JOB_TOO_MANY_COLUMNS,
    JOB_BAD_VALUES,
    JOB_TOO_LARGE_UNPACKED,
    JOB_COLUMNS_NOT_PREVIEWED,
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
    /// A top-level JSON array, named `.json` (`DATA-17` slice 4).
    Json,
    /// JSON Lines, named `.jsonl` or `.ndjson`.
    JsonLines,
    /// An Avro container, named `.avro`.
    Avro,
    /// A gzip stream named `.gz` that holds text or JSON.
    Gzip,
    /// A zip archive named `.zip`; its files are listed when it is opened
    /// ([`open_whole`](sessions)) and each time it is read.
    Zip,
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
    classify_head(bytes, filename)
}

/// What a file is, from its first bytes and its name: the kind rules of
/// [`check_file`] without its size rules. A chunked upload (`sessions`) has
/// had its size checked when the session opened and is classified from its
/// first 64 KiB, so both paths refuse with the same sentences.
fn classify_head(head: &[u8], filename: &str) -> Result<Accepted, ApiError> {
    if has_parquet_name(filename) && upload_parquet::has_encrypted_magic(head) {
        // Not a magic `sniff` knows; the reader names the real reason.
        return Ok(Accepted::Parquet);
    }
    let named = upload_formats::by_name(filename);
    let message = match upload_parse::sniff(head) {
        Kind::DelimitedText => return classify_text(head, named),
        Kind::Workbook if has_workbook_name(filename) => return Ok(Accepted::Workbook),
        Kind::Workbook if named == ByName::Zip => return Ok(Accepted::Zip),
        Kind::Workbook => WORKBOOK,
        Kind::Parquet if has_parquet_name(filename) => return Ok(Accepted::Parquet),
        Kind::Parquet => PARQUET,
        Kind::Gzip if matches!(named, ByName::Gzip(_)) => return classify_gzip(head),
        Kind::Gzip => GZIP,
        Kind::Avro if named == ByName::Plain(Format::Avro) => return Ok(Accepted::Avro),
        Kind::Avro => AVRO,
        Kind::OtherBinary => OTHER_BINARY,
    };
    Err(ApiError::BadRequest(message.to_owned()))
}

/// Text by its first bytes: delimited text, unless its name says JSON, JSON Lines or an
/// archive, in which case the first byte of the document must agree (`[` for an array, `{`
/// for lines): the name is never enough, as for a workbook.
fn classify_text(head: &[u8], named: ByName) -> Result<Accepted, ApiError> {
    let first = head
        .strip_prefix(b"\xef\xbb\xbf")
        .unwrap_or(head)
        .iter()
        .find(|byte| !byte.is_ascii_whitespace());
    let refuse = |message: &str| Err(ApiError::BadRequest(message.to_owned()));
    match named {
        ByName::Other => Ok(Accepted::Text),
        ByName::Plain(Format::Json) if first == Some(&b'[') => Ok(Accepted::Json),
        ByName::Plain(Format::Json) => refuse(&Refusal::JsonNotArray.to_string()),
        ByName::Plain(Format::JsonLines) if first == Some(&b'{') => Ok(Accepted::JsonLines),
        ByName::Plain(Format::JsonLines) => refuse(&Refusal::JsonNotObjects.to_string()),
        ByName::Plain(_) => refuse(NOT_AVRO),
        ByName::Gzip(_) | ByName::Zip => refuse(NOT_AN_ARCHIVE),
    }
}

/// A gzip stream: its first decoded bytes must be text (not another archive, not binary).
fn classify_gzip(head: &[u8]) -> Result<Accepted, ApiError> {
    let (inner, _more) = upload_formats::gunzip_prefix(head, 64 * 1024)
        .map_err(|refusal| ApiError::BadRequest(refusal.to_string()))?;
    match upload_parse::sniff(&inner) {
        Kind::DelimitedText => Ok(Accepted::Gzip),
        _ => Err(ApiError::BadRequest(Refusal::GzipHoldsOther.to_string())),
    }
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
/// 429 (`Retry-After: 5`) when the caller already has the configured number of
/// files being received, or the installation has (`SEC-17`); 400 for a caller
/// in no tenant, a request that is not a multipart form, no
/// `file` part, an empty file, a file over [`MAX_UPLOAD_BYTES`], a workbook
/// that is not an `.xls` or `.xlsx` or does not open (damaged, password
/// protected, no sheet), a Parquet file that is not named `.parquet` or does
/// not open (damaged, encrypted, a binary or nested column, over the cell cap)
/// or another binary (each with its own fixed sentence), and a header with
/// more than 1,000 columns (`SEC-17`); 404 if `X-Tenant` names a tenant the caller does not belong to;
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
    // `SEC-17`: a place is taken before the body is read, because each file
    // being received is held in memory (up to 50 MB), and is given back when
    // `_place` drops at the end of the handler, on every path. Keyed by the
    // display name, the same text `uploaded_by` records, so the per-user
    // number means the same for receiving and for loading.
    let Some(_place) = state.upload_receive.try_acquire(&principal.display_name) else {
        return Err(too_many_uploads().into());
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
    let cap = MAX_UNPACKED_BYTES;
    let (workbook, parquet) = match accepted {
        Accepted::Text | Accepted::Json | Accepted::JsonLines | Accepted::Avro => (None, None),
        // `DATA-17` slice 4 (decision 2): what a gzip or zip file unpacks to counts toward
        // the cap, and a bomb is refused here, before anything is stored.
        Accepted::Gzip => {
            let bytes = part.bytes.clone();
            blocking(move || upload_formats::check_gzip(std::io::Cursor::new(bytes), cap)).await?;
            (None, None)
        }
        Accepted::Zip => {
            let bytes = part.bytes.clone();
            blocking(move || upload_formats::check_zip(std::io::Cursor::new(bytes), cap)).await?;
            (None, None)
        }
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
        if let Some(primary_id) = row.loaded_with.clone() {
            return self.settle_with_primary(row, &primary_id).await;
        }
        self.settle_own(row).await
    }

    /// A file of a load of several files has no run of its own (`DATA-17` slice 5): it ends
    /// when the primary ends, in the transaction that settles the primary
    /// ([`uploads::mark_finished`]). So settling it is settling the primary and reading it
    /// again. A file whose primary is gone or no longer loading was left by a claim that
    /// never finished; it is failed after the same two minutes a claim with no run is
    /// ([`STALE_CLAIM`]), and not before, because the primary's claim is written in the same
    /// transaction as its own and a younger orphan is not an orphan yet.
    async fn settle_with_primary(&mut self, row: Upload, primary_id: &str) -> Upload {
        let primary = match uploads::get(self.pool, primary_id).await {
            Ok(primary) => primary,
            Err(err) => {
                tracing::warn!(%err, upload_id = %row.id, "the primary of a loading file could not be read");
                return row;
            }
        };
        if let Some(primary) =
            primary.filter(|p| p.status == "ingesting" && p.loaded_with.is_none())
        {
            self.settle_own(primary).await;
            return uploads::get(self.pool, &row.id)
                .await
                .ok()
                .flatten()
                .unwrap_or(row);
        }
        if claim_is_stale(row.updated_at, OffsetDateTime::now_utc()) {
            return self
                .record(row, None, &Outcome::Failed(NOT_STARTED.to_owned()))
                .await;
        }
        row
    }

    async fn settle_own(&mut self, row: Upload) -> Upload {
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
            Ok(Some(updated)) => match run_id {
                // `DATA-18`: a typed load may have left a report of values that did not fit.
                Some(run_id) => columns::keep_report(self.state, self.pool, updated, run_id).await,
                None => updated,
            },
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

/// Settle one upload that is `ingesting`, through the same recording path a
/// reader's list or page uses ([`Settler`]), for the run-failed event
/// (`SRC-7` F6): an upload nobody opens would otherwise stay `ingesting`
/// after its run failed. Returns the row as it is afterwards.
pub(super) async fn settle_loading_upload(state: &AppState, pool: &PgPool, row: Upload) -> Upload {
    Settler::new(state, pool).settle(row).await
}

/// `GET /api/uploads` — the active tenant's uploads, newest first, at most
/// [`LIST_LIMIT`]. A caller in no tenant gets an empty list, never another
/// tenant's. A loading upload is settled first. `folderId=<id>|none` and `q=<text>`
/// narrow the list before the window is cut (`DATA-35`, [`ListQuery`]).
///
/// # Errors
///
/// 400 for a query string that cannot be read; 404 if `X-Tenant` names a tenant the
/// caller does not belong to; 503 if no pool is configured; 500 on a database
/// failure.
pub async fn list(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
    query: Result<Query<ListQuery>, axum::extract::rejection::QueryRejection>,
) -> ApiResult<ApiJson<Vec<UploadView>>> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let Query(query) = query.map_err(|err| {
        tracing::warn!(%err, "an upload list query string could not be read");
        ApiError::BadRequest(QUERY_UNREADABLE.to_owned())
    })?;
    let Some(tenant_id) = crate::tenant_scope::resolve(&principal, &headers)? else {
        return Ok(ApiJson(Vec::new()));
    };
    let pool = pool(&state)?;
    // `DATA-35` AD-5: the filters narrow the rows BEFORE the newest-100 window is cut,
    // so a folder's files and a search are not hidden by newer files elsewhere. With
    // neither, this is the list it always was.
    let text = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let filter = ListFilter {
        folder: match query.folder_id.as_deref() {
            None | Some("") => FolderFilter::Any,
            Some("none") => FolderFilter::NoFolder,
            Some(id) => FolderFilter::In(id),
        },
        text,
    };
    let rows = uploads::list_filtered(pool, tenant_id, LIST_LIMIT, &filter).await?;
    let mut settler = Settler::new(&state, pool);
    let mut views = Vec::with_capacity(rows.len());
    for row in rows {
        views.push(settler.settle(row).await.into());
    }
    Ok(ApiJson(views))
}

/// Query parameters of [`list`] (`DATA-35`, plan AD-5).
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// A folder id, or `none` for the files in no folder; absent: every file.
    #[serde(rename = "folderId")]
    folder_id: Option<String>,
    /// Text a file name or a table name must contain, case-insensitively.
    q: Option<String>,
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
    /// A table the file will be added to: when it is a table of the caller's
    /// tenant, the columns' names and types are the table's (`DATA-18`
    /// decision 9). Anything else is ignored, so the answer says nothing about
    /// tables of other tenants.
    table: Option<String>,
}

/// `GET /api/uploads/{id}/preview`'s response: the [`Preview`] and, for a
/// workbook, which sheet it shows and which sheets there are.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResponse {
    #[serde(flatten)]
    preview: Preview,
    #[serde(skip_serializing_if = "Option::is_none")]
    workbook: Option<WorkbookView>,
    /// For a Parquet file: its columns with the types the file declares and
    /// the row count its footer states (`preview.rows` are the first ones).
    #[serde(skip_serializing_if = "Option::is_none")]
    parquet: Option<ParquetInfo>,
    /// For JSON, JSON Lines, Avro, gzip and zip (`DATA-17` slice 4): what the file is
    /// read as. Absent for text, workbooks and Parquet.
    #[serde(skip_serializing_if = "Option::is_none")]
    layout: Option<LayoutView>,
    /// For a zip: its files and whether each has the first file's columns.
    #[serde(skip_serializing_if = "Option::is_none")]
    archive: Option<ArchiveView>,
    /// Every column with its name, its starting type and its date reading
    /// (`DATA-18`).
    column_specs: Vec<SpecView>,
    /// How many data rows the guess read (at most 10,000, or 16 MiB).
    sampled_rows: usize,
    /// The names and types are a table's, not a guess.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    from_table: bool,
}

/// What a file is read as (`DATA-17` slice 4).
#[derive(Debug, Serialize)]
pub struct LayoutView {
    /// `delimited`, `json`, `jsonl` or `avro`.
    format: &'static str,
    /// `none`, `gzip` or `zip`.
    compression: &'static str,
}

/// The files of a zip archive.
#[derive(Debug, Serialize)]
pub struct ArchiveView {
    files: Vec<ArchiveFileView>,
}

/// One file of a zip archive.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveFileView {
    /// Its name inside the archive.
    name: String,
    /// Its uncompressed size as the archive declares it.
    size_bytes: u64,
    /// How many columns its header (or keys, or fields) has.
    columns: usize,
    /// Whether it has the first file's columns, whatever their order.
    matches_first: bool,
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
/// fixed sentence), and for a text file with a header of more than 1,000
/// columns (`SEC-17`); 404
/// for an unknown upload or another tenant's; 409 for an upload still being
/// received (`DATA-17`); 503 when Postgres or the object store is unavailable.
pub async fn preview(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    query: Result<Query<PreviewQuery>, axum::extract::rejection::QueryRejection>,
) -> ApiResult<ApiJson<PreviewResponse>> {
    let Query(query) = query.map_err(|err| {
        tracing::warn!(%err, "a preview query string could not be read");
        ApiError::BadRequest(QUERY_UNREADABLE.to_owned())
    })?;
    let row = uploads::get(pool(&state)?, &id)
        .await?
        .ok_or_else(not_found)?;
    let (mut response, sample) = preview_with_sample(&state, &row, &query).await?;
    // `DATA-18` decision 9: when the file will be added to a table of the
    // caller's tenant, that table's names and types are the columns'.
    let held = match (&query.table, principal) {
        (Some(table), Some(Extension(principal))) => {
            match crate::tenant_scope::resolve(&principal, &headers)? {
                Some(tenant_id) => columns::own_table_columns(&state, tenant_id, table)
                    .await
                    .filter(|held| held.len() == sample.header.len()),
                None => None,
            }
        }
        _ => None,
    };
    response.column_specs = columns::spec_views(&sample, held.as_ref());
    response.sampled_rows = sample.rows.len();
    response.from_table = held.is_some();
    Ok(ApiJson(response))
}

/// The preview of an upload and the sample its columns are guessed from: the
/// header and up to 10,000 data rows (`DATA-18`, decision 1). Shared by the
/// preview and the check; the columns of the response are filled in by the
/// caller.
///
/// # Errors
///
/// As [`preview`].
async fn preview_with_sample(
    state: &AppState,
    row: &Upload,
    query: &PreviewQuery,
) -> Result<(PreviewResponse, Sample), ApiError> {
    let overrides = overrides_of(query)?;
    if row.status == "receiving" {
        return Err(ApiError::Conflict(STILL_RECEIVING.to_owned()));
    }
    let store = UploadStore::connect(&state.config).await?;
    let head = store.head_bytes(&row.storage_key, PREVIEW_BYTES).await?;
    match conversion_of_head(&row.storage_key, &head) {
        Some(Conversion::Workbook) => {
            return workbook_preview(&store, row, query.sheet.clone(), overrides.header_row).await;
        }
        Some(Conversion::Parquet) => {
            return parquet_preview(&store, row).await;
        }
        None => {}
    }
    // `DATA-17` slice 4: JSON, JSON Lines, Avro, gzip and zip.
    if let Some(layout) = layout_of(&store, row).await? {
        let sampled = layout_sample(&store, row, layout, overrides).await?;
        return Ok((
            PreviewResponse {
                preview: sampled.preview,
                workbook: None,
                parquet: None,
                layout: Some(LayoutView {
                    format: layout.format.as_str(),
                    compression: layout.compression.as_str(),
                }),
                archive: sampled.archive,
                column_specs: Vec::new(),
                sampled_rows: 0,
                from_table: false,
            },
            sampled.sample,
        ));
    }
    // A preview reads one chunk, so "20 rows" never implies the file has only
    // 20: `truncated` says the file goes on.
    let head_is_truncated = row.size_bytes > i64::try_from(PREVIEW_BYTES).unwrap_or(i64::MAX);
    // `SEC-17`: a header with more than `MAX_COLUMNS` cells is refused here,
    // not shown. The 400 is per reading, so the Check step can still change
    // the delimiter or header row (a wrong guess must not lock a good file
    // out). Workbook and Parquet paths run their own refusal (`upload_workbook`
    // and `upload_parquet`), so the column cap is checked only for text.
    if upload_parse::header_width(&head, head_is_truncated, overrides) > upload_parse::MAX_COLUMNS {
        return Err(ApiError::BadRequest(JOB_TOO_MANY_COLUMNS.to_owned()));
    }
    let preview = upload_parse::preview(&head, head_is_truncated, overrides, PREVIEW_ROWS);
    // The guess reads more than the preview shows (decision 1): up to 16 MiB,
    // and of that the first 10,000 data rows.
    let sample_bytes = columns::SAMPLE_BYTES;
    let wide = if head_is_truncated {
        store.head_bytes(&row.storage_key, sample_bytes).await?
    } else {
        head
    };
    let wide_truncated = row.size_bytes > i64::try_from(sample_bytes).unwrap_or(i64::MAX);
    let rows =
        upload_parse::sample_records(&wide, wide_truncated, preview.using, columns::SAMPLE_ROWS);
    let sample = Sample {
        header: preview.columns.clone(),
        rows,
        declared: None,
    };
    Ok((
        PreviewResponse {
            preview,
            workbook: None,
            parquet: None,
            layout: None,
            archive: None,
            column_specs: Vec::new(),
            sampled_rows: 0,
            from_table: false,
        },
        sample,
    ))
}

/// What an upload is read as when it is JSON, JSON Lines, Avro, gzip or zip, or `None` for
/// everything else (text, a workbook, Parquet). The name says it, and the first bytes agreed
/// when the file arrived ([`classify_head`]); a gzip file with no inner name is classed by its
/// first decoded byte, and a zip by the extensions of its files.
///
/// # Errors
///
/// 400 with the reader's own sentence for a gzip or zip file that no longer opens; 503 when
/// the object store is unavailable.
async fn layout_of(store: &UploadStore, row: &Upload) -> Result<Option<Layout>, ApiError> {
    Ok(match upload_formats::by_name(&row.original_filename) {
        ByName::Other => None,
        ByName::Plain(format) => Some(Layout {
            compression: Compression::None,
            format,
        }),
        ByName::Gzip(Some(format)) => Some(Layout {
            compression: Compression::Gzip,
            format,
        }),
        ByName::Gzip(None) => {
            let reader = RangeReader::open(store, &row.storage_key).await?;
            let (prefix, _more) =
                blocking(move || upload_formats::gunzip_prefix(reader.sequential(), 4096)).await?;
            Some(Layout {
                compression: Compression::Gzip,
                format: upload_formats::format_of_text(&prefix),
            })
        }
        ByName::Zip => {
            let reader = RangeReader::open(store, &row.storage_key).await?;
            let listing = blocking(move || upload_formats::zip_listing(reader.seekable())).await?;
            Some(Layout {
                compression: Compression::Zip,
                format: listing.format,
            })
        }
    })
}

/// [`layout_of`] for an upload that is not a workbook or a Parquet file.
///
/// # Errors
///
/// As [`layout_of`].
async fn layout_unless_converted(
    state: &AppState,
    row: &Upload,
    conversion: Option<Conversion>,
) -> Result<Option<Layout>, ApiError> {
    if conversion.is_some() {
        return Ok(None);
    }
    layout_of(&UploadStore::connect(&state.config).await?, row).await
}

/// What the job is told the object is: its format and its wrapper (`DATA-17` slice 4 adds
/// the wrapper).
fn job_kind(
    conversion: Option<Conversion>,
    layout: Option<Layout>,
) -> (&'static str, &'static str) {
    (
        if conversion == Some(Conversion::Parquet) {
            "parquet"
        } else {
            layout.map_or("delimited", |l| l.format.as_str())
        },
        layout.map_or("none", |l| l.compression.as_str()),
    )
}

/// How many decoded bytes of each file after a zip's first are read to learn its columns.
const ARCHIVE_HEADER_BYTES: usize = 256 * 1024;

/// A preview and its sample, read from the decoded bytes of a layout.
struct Sampled {
    preview: Preview,
    sample: Sample,
    archive: Option<ArchiveView>,
}

/// The preview and sample of decoded bytes of one `format`: delimited text as for a CSV, JSON
/// and Avro from their own readers (the dialect is fixed: UTF-8, comma, the keys or fields
/// are the header).
///
/// # Errors
///
/// 400 with the refusal's sentence.
fn sample_of(
    format: Format,
    bytes: &[u8],
    truncated: bool,
    overrides: Overrides,
) -> Result<(Preview, Sample), ApiError> {
    let refused = |refusal: Refusal| ApiError::BadRequest(refusal.to_string());
    let parsed = match format {
        Format::Delimited => {
            let head_len = bytes.len().min(PREVIEW_BYTES);
            let head = &bytes[..head_len];
            let head_truncated = truncated || head_len < bytes.len();
            if upload_parse::header_width(head, head_truncated, overrides)
                > upload_parse::MAX_COLUMNS
            {
                return Err(ApiError::BadRequest(JOB_TOO_MANY_COLUMNS.to_owned()));
            }
            let preview = upload_parse::preview(head, head_truncated, overrides, PREVIEW_ROWS);
            let rows =
                upload_parse::sample_records(bytes, truncated, preview.using, columns::SAMPLE_ROWS);
            let sample = Sample {
                header: preview.columns.clone(),
                rows,
                declared: None,
            };
            return Ok((preview, sample));
        }
        Format::Json => upload_formats::read_json(bytes, truncated, false, columns::SAMPLE_ROWS),
        Format::JsonLines => {
            upload_formats::read_json(bytes, truncated, true, columns::SAMPLE_ROWS)
        }
        Format::Avro => upload_formats::read_avro(bytes, truncated, columns::SAMPLE_ROWS),
    }
    .map_err(refused)?;
    let reading = upload_parse::Reading {
        encoding: Encoding::Utf8,
        delimiter: ',',
        header_row: 0,
    };
    let preview = Preview {
        detected: reading,
        using: reading,
        columns: parsed.columns.clone(),
        rows: parsed.rows.iter().take(PREVIEW_ROWS).cloned().collect(),
        truncated: parsed.more,
    };
    let sample = Sample {
        header: parsed.columns,
        rows: parsed.rows,
        declared: parsed.declared,
    };
    Ok((preview, sample))
}

/// The columns of one file of an archive: its header, keys or fields.
fn header_of(
    format: Format,
    bytes: &[u8],
    truncated: bool,
    overrides: Overrides,
) -> Result<Vec<String>, ApiError> {
    Ok(sample_of(format, bytes, truncated, overrides)?.1.header)
}

/// Read the first rows of a layout: the file itself, the decoded stream of a gzip file, or the
/// first file of a zip with a check that every other file has the same columns.
///
/// # Errors
///
/// 400 with the refusal's sentence; 503 when the object store is unavailable.
async fn layout_sample(
    store: &UploadStore,
    row: &Upload,
    layout: Layout,
    overrides: Overrides,
) -> Result<Sampled, ApiError> {
    let want = columns::SAMPLE_BYTES;
    let plain = |bytes: &[u8], truncated: bool| -> Result<Sampled, ApiError> {
        let (preview, sample) = sample_of(layout.format, bytes, truncated, overrides)?;
        Ok(Sampled {
            preview,
            sample,
            archive: None,
        })
    };
    match layout.compression {
        Compression::None => {
            let bytes = store.head_bytes(&row.storage_key, want).await?;
            let truncated = row.size_bytes > i64::try_from(want).unwrap_or(i64::MAX);
            plain(&bytes, truncated)
        }
        Compression::Gzip => {
            let reader = RangeReader::open(store, &row.storage_key).await?;
            let (bytes, more) =
                blocking(move || upload_formats::gunzip_prefix(reader.sequential(), want)).await?;
            plain(&bytes, more)
        }
        Compression::Zip => {
            let reader = RangeReader::open(store, &row.storage_key).await?;
            let (listing, heads) = blocking(move || {
                let listing = upload_formats::zip_listing(reader.seekable())?;
                let mut heads = Vec::with_capacity(listing.files.len());
                for position in 0..listing.files.len() {
                    let limit = if position == 0 {
                        want
                    } else {
                        ARCHIVE_HEADER_BYTES
                    };
                    heads.push(upload_formats::zip_file_prefix(
                        reader.seekable(),
                        position,
                        limit,
                    )?);
                }
                Ok::<_, Refusal>((listing, heads))
            })
            .await?;
            let (first_bytes, first_more) = heads.first().cloned().unwrap_or_default();
            let mut sampled = plain(&first_bytes, first_more)?;
            let wanted = crate::upload_types::clean_names(&sampled.sample.header);
            let mut files = Vec::with_capacity(heads.len());
            for (info, (bytes, more)) in listing.files.iter().zip(&heads) {
                let header = header_of(layout.format, bytes, *more, overrides)?;
                let mut names = crate::upload_types::clean_names(&header);
                names.sort();
                let mut expected = wanted.clone();
                expected.sort();
                files.push(ArchiveFileView {
                    name: info.name.clone(),
                    size_bytes: info.size,
                    columns: header.len(),
                    matches_first: names == expected,
                });
            }
            sampled.archive = Some(ArchiveView { files });
            Ok(sampled)
        }
    }
}

/// The preview of a workbook: its sheet as text, read the way the load reads
/// it (UTF-8, comma).
async fn workbook_preview(
    store: &UploadStore,
    row: &Upload,
    sheet: Option<String>,
    header_row: Option<usize>,
) -> Result<(PreviewResponse, Sample), ApiError> {
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
    let preview = upload_parse::preview(&csv, cut, overrides, PREVIEW_ROWS);
    let rows = upload_parse::sample_records(&csv, cut, preview.using, columns::SAMPLE_ROWS);
    let sample = Sample {
        header: preview.columns.clone(),
        rows,
        declared: None,
    };
    Ok((
        PreviewResponse {
            preview,
            workbook: Some(WorkbookView { sheet: name, info }),
            parquet: None,
            layout: None,
            archive: None,
            column_specs: Vec::new(),
            sampled_rows: 0,
            from_table: false,
        },
        sample,
    ))
}

/// The preview of a Parquet file: its first rows as text, read the way the load
/// reads them (UTF-8, comma, the first record is the header).
async fn parquet_preview(
    store: &UploadStore,
    row: &Upload,
) -> Result<(PreviewResponse, Sample), ApiError> {
    // `DATA-17` T2.2: the footer and the first row group, read by range; the file
    // is never held (`RangeReader`), and what the footer declares is bounded.
    let reader = RangeReader::open(store, &row.storage_key).await?;
    let converted =
        blocking(move || upload_parquet::convert(&reader, Some(PARQUET_PREVIEW_ROWS))).await?;
    let cut = converted.info.rows > u64::try_from(PARQUET_PREVIEW_ROWS).unwrap_or(u64::MAX);
    let overrides = Overrides {
        encoding: Some(Encoding::Utf8),
        delimiter: Some(','),
        header_row: Some(0),
    };
    let preview = upload_parse::preview(&converted.csv, cut, overrides, PREVIEW_ROWS);
    let rows =
        upload_parse::sample_records(&converted.csv, cut, preview.using, columns::SAMPLE_ROWS);
    // A Parquet file's own types are the starting types (decision 10).
    let declared = converted
        .info
        .columns
        .iter()
        .map(|column| crate::upload_types::declared_from_parquet(&column.type_name))
        .collect();
    let sample = Sample {
        header: preview.columns.clone(),
        rows,
        declared: Some(declared),
    };
    Ok((
        PreviewResponse {
            preview,
            workbook: None,
            parquet: Some(converted.info),
            layout: None,
            archive: None,
            column_specs: Vec::new(),
            sampled_rows: 0,
            from_table: false,
        },
        sample,
    ))
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
    /// The columns as confirmed and what to do with a value that does not fit
    /// (`DATA-18`); `None`: every column text.
    columns: Option<ColumnPlan>,
    /// The other files of a load of several files (`DATA-17` slice 5), at most 9, as
    /// named; empty for a load of one file.
    also_uploads: Vec<String>,
    /// The folder every file of the load is put in and the table remembers
    /// (`DATA-35` decision 2, AD-4). [`folders::FolderRef::Absent`]: the request
    /// said nothing, so the table's remembered folder applies
    /// ([`effective_folder`]); [`folders::FolderRef::Null`]: `folderId: null`,
    /// "no folder" chosen on purpose.
    folder_id: folders::FolderRef,
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
/// `headerRow` and, optional, `sheet`, `columns` with `onBadValue`, `alsoUploads`
/// (`DATA-17` slice 5) and `folderId` (`DATA-35`). All are checked; the load uses exactly what was confirmed
/// and nothing is guessed (ADR 0014, decision 3).
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
    layout: Option<Layout>,
) -> Result<IngestRequest, ApiError> {
    // JSON, JSON Lines and Avro have no dialect to choose (`DATA-17` slice 4): UTF-8, a
    // comma, and the keys or fields as the header, as for Parquet. A gzip or zip file of
    // delimited text keeps the person's choice.
    let fixed = conversion.is_some() || layout.is_some_and(|l| l.format != Format::Delimited);
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
    let (encoding, delimiter) = if fixed {
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
        _ if layout.is_some_and(|l| l.format != Format::Delimited) => 0,
        (_, None | Some(Value::Null)) => return Err(bad("headerRow is required.")),
        (_, Some(value)) => value.as_u64().ok_or_else(|| bad(HEADER_ROW_RULE))?,
    };
    let sheet = match fields.get("sheet") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => Some(name.clone()),
        Some(_) => return Err(bad(SHEET_RULE)),
    };
    let columns = columns::parse_column_plan(&fields)?;
    let also_uploads = match fields.get("alsoUploads") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(ids)) => ids
            .iter()
            .map(|id| id.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| bad(ALSO_UPLOADS_RULE))?,
        Some(_) => return Err(bad(ALSO_UPLOADS_RULE)),
    };
    group::check_also_list(&also_uploads)?;
    let folder_id = match fields.get("folderId") {
        None => folders::FolderRef::Absent,
        Some(Value::Null) => folders::FolderRef::Null,
        Some(Value::String(id)) if !id.is_empty() => folders::FolderRef::Id(id.clone()),
        Some(_) => return Err(bad(FOLDER_ID_RULE)),
    };
    Ok(IngestRequest {
        table: table.to_owned(),
        mode,
        encoding,
        delimiter,
        header_row,
        sheet,
        columns,
        also_uploads,
        folder_id,
    })
}

/// Whether `table` may be loaded into by an upload of `tenant_id`, or why
/// not. The rule (ADR 0014, decision 5, review finding B4), in this order:
///
/// 1. A table a connector loads: never. The sentence says so
///    ([`CONNECTOR_TABLE`]) only when the connector is the caller's own
///    tenant's; for another tenant's (or an unassigned) connector it is the
///    one sentence of [`TABLE_NOT_FREE`], so the answer does not tell a
///    person that someone else has a connector writing a table of that name
///    (`SEC-16`).
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
/// 409 for a connector's table ([`CONNECTOR_TABLE`] for the caller's own
/// tenant's connector, otherwise [`TABLE_NOT_FREE`]), for a name another
/// tenant holds, and for an existing table nobody claimed (those with the one
/// sentence, [`TABLE_NOT_FREE`]); 503 when the question cannot be answered (Postgres,
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
    // SEC-16: the connector's own sentence only for the caller's tenant. A
    // connector of anyone else still closes the name (ADR 0014, decision 5:
    // raw table names are shared), but with the sentence every other reason
    // gets.
    if connectors::any_connector_of_tenant_targets(pool, tenant_id, table)
        .await
        .map_err(|err| unchecked(&err))?
    {
        return Err(ApiError::Conflict(CONNECTOR_TABLE.to_owned()));
    }
    if connectors::any_connector_targets(pool, table)
        .await
        .map_err(|err| unchecked(&err))?
    {
        return Err(ApiError::Conflict(TABLE_NOT_FREE.to_owned()));
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
        // No registry table yet means nothing has ever been registered. On a
        // fresh install the whole `lake` database is absent too (`Code: 81`):
        // the orchestrator's `register_bronze_table` creates the database and
        // both registry tables (`EXPECTED_SCHEMAS`, their single owner) before
        // its first write, so an upload may be the first thing a tenant does.
        // SRC-9 finding G4. An absent registry holds no name, so nothing is
        // claimed here; the Iceberg half below still runs, and every other
        // error stays `Unanswered`.
        Err(ChError::Server(body)) if is_unknown_table_or_database_error(&body) => {}
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
/// closed, so nothing about the sheet can be added here. `load_keys` lists every
/// file of the load, the primary first (`DATA-17` slice 5, plan AD5); the
/// `bronze_table_name` path the `DATA-19` sensor reads does not move.
fn run_config(
    upload_id: &str,
    request: &IngestRequest,
    load_keys: &[String],
    (format, compression): (&str, &str),
) -> Value {
    let (columns, on_bad_value) = columns::run_config_columns(request.columns.as_ref());
    json!({
        "ops": {
            "ingest_uploaded_file": {
                "config": {
                    "upload_id": upload_id,
                    "storage_keys": load_keys,
                    "format": format,
                    "compression": compression,
                    "max_bytes": MAX_UNPACKED_BYTES,
                    "bronze_table_name": request.table,
                    "load_mode": request.mode.as_str(),
                    "encoding": request.encoding.as_str(),
                    "delimiter": request.delimiter.to_string(),
                    "header_row": request.header_row,
                    "columns": columns,
                    "on_bad_value": on_bad_value,
                }
            }
        }
    })
}

/// The load limits of `SEC-17`, from the configuration.
fn load_limits(state: &AppState) -> LoadLimits {
    LoadLimits {
        per_user: i64::from(state.config.upload_max_concurrent_per_user),
        total: i64::from(state.config.upload_max_concurrent),
    }
}

/// `SEC-17`: count the header's cells under the reading the user confirmed,
/// AND ask the load limit, before any claim, mark or launch. A file the job
/// would refuse leaves no table claim behind, and a request that passes
/// the read but loses the locked count in `mark_ingesting_within_limits`
/// below keeps its claim, as a claim does when a launch fails (a claim
/// is never released).
///
/// # Errors
///
/// 429 (`Retry-After: 5`) when the `uploaded_by` already has the configured
/// number of loads running, or the installation has; 400 when the header
/// has more than 1,000 columns under the confirmed reading; 503 when the
/// object store cannot serve the head.
async fn check_within_limits_and_column_cap(
    state: &AppState,
    pool: &PgPool,
    row: &Upload,
    request: &IngestRequest,
    layout: Option<Layout>,
) -> ApiResult<LoadLimits> {
    let limits = load_limits(state);
    if !uploads::loads_under_limits(pool, &row.id, limits).await? {
        return Err(too_many_uploads().into());
    }
    let store = UploadStore::connect(&state.config).await?;
    if let Some(layout) = layout {
        // `DATA-17` slice 4: the same read as the preview (it refuses more than 1,000
        // columns), and a zip whose files differ in their columns is not loaded.
        let confirmed = Overrides {
            encoding: Some(request.encoding),
            delimiter: Some(request.delimiter),
            header_row: Some(usize::try_from(request.header_row).unwrap_or(usize::MAX)),
        };
        let sampled = layout_sample(&store, row, layout, confirmed).await?;
        if sampled
            .archive
            .is_some_and(|archive| archive.files.iter().any(|file| !file.matches_first))
        {
            return Err(ApiError::BadRequest(ZIP_COLUMNS_DIFFER.to_owned()).into());
        }
        return Ok(limits);
    }
    let head = store.head_bytes(&row.storage_key, PREVIEW_BYTES).await?;
    let head_is_truncated = row.size_bytes > i64::try_from(PREVIEW_BYTES).unwrap_or(i64::MAX);
    let confirmed = Overrides {
        encoding: Some(request.encoding),
        delimiter: Some(request.delimiter),
        header_row: Some(usize::try_from(request.header_row).unwrap_or(usize::MAX)),
    };
    if upload_parse::header_width(&head, head_is_truncated, confirmed) > upload_parse::MAX_COLUMNS {
        return Err(ApiError::BadRequest(JOB_TOO_MANY_COLUMNS.to_owned()).into());
    }
    Ok(limits)
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
    load_keys: &[String],
    kind: (&str, &str),
) -> Result<String, ApiError> {
    let failure = match state
        .dagster
        .launch_run_with_config(
            FILE_INGEST_JOB,
            &run_config(&claimed.id, request, load_keys, kind),
        )
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
/// The checks the Parquet conversion used to make at ingest, now made on the
/// footer alone (`DATA-17` T2.2): the file opens, has rows, and has no more
/// columns than a table may. Nothing is decoded and the file is not held.
///
/// # Errors
///
/// 400 with the reader's own sentence for a file that no longer opens, is
/// encrypted, has a binary or nested column, or is over the cell cap; with
/// [`EMPTY_PARQUET`] for a file with no rows and [`JOB_TOO_MANY_COLUMNS`] for
/// one with more than 1,000 columns; 503 when the object store is unavailable.
async fn check_parquet_footer(state: &AppState, row: &Upload) -> Result<(), ApiError> {
    let store = UploadStore::connect(&state.config).await?;
    let reader = RangeReader::open(&store, &row.storage_key).await?;
    let info = blocking(move || upload_parquet::inspect(&reader)).await?;
    if info.rows == 0 {
        return Err(ApiError::BadRequest(EMPTY_PARQUET.to_owned()));
    }
    if info.columns.len() > upload_parse::MAX_COLUMNS {
        return Err(ApiError::BadRequest(JOB_TOO_MANY_COLUMNS.to_owned()));
    }
    Ok(())
}

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
        // A Parquet file is not converted: the job reads it natively (`DATA-17`
        // T2.2), and `ingest` checks its footer.
        Conversion::Parquet => return Err(ApiError::Internal(CONVERSION_FAILED.to_owned())),
    };
    let key = converted_key(&row.storage_key);
    store.put(&key, Bytes::from(csv)).await?;
    Ok(Converted { key, sheet })
}

/// Claim the upload, and the other files of its load, as `ingesting`: the reading, the table,
/// the mode and the column list are written by the claim itself, so a row is never
/// `ingesting` with half of what its load was told (`DATA-18`).
///
/// `SEC-17`: the count and the marks are one locked transaction, so two starts at the same
/// moment cannot both take the last place. `DATA-17` slice 5: the other files are claimed in
/// the same transaction, so a load of several files is never half taken.
///
/// # Errors
///
/// 429 when a limit is reached; 409 when this upload was claimed by another request, or one of
/// the other files was taken or went away since the checks; 404 when this upload went away.
async fn claim_load(
    pool: &PgPool,
    row: &Upload,
    request: &IngestRequest,
    folder_id: Option<&str>,
    converted: Option<&Converted>,
    limits: LoadLimits,
) -> ApiResult<Upload> {
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
    ) = (&mut parse_options, converted)
    {
        map.insert("sheet".to_owned(), json!(sheet));
    }
    let (column_spec, on_bad_value) = columns::claim_fields(request.columns.as_ref());
    let outcome = uploads::mark_group_within_limits(
        pool,
        &row.id,
        &request.also_uploads,
        &LoadClaim {
            parse_options: &parse_options,
            bronze_table: &request.table,
            load_mode: request.mode,
            column_spec: column_spec.as_ref(),
            on_bad_value,
            folder_id,
        },
        limits,
    )
    .await?;
    Ok(match outcome {
        MarkOutcome::Marked(claimed) => *claimed,
        MarkOutcome::LimitReached => return Err(too_many_uploads().into()),
        MarkOutcome::GroupNotMarked => {
            // One of the other files was taken, or went away, since the checks.
            return Err(ApiError::Conflict(OTHER_BUSY.to_owned()).into());
        }
        MarkOutcome::NotMarked => {
            // Lost the race to another request, or the upload went away.
            return Err(match uploads::get(pool, &row.id).await? {
                Some(_) => ApiError::Conflict(ALREADY_LOADING.to_owned()),
                None => not_found(),
            }
            .into());
        }
    })
}

/// The folder a load puts its files in (`DATA-35` decision 2, plan AD-4): the one
/// the request named, which must be the upload's tenant's (decision 5, `SEC-16`:
/// another tenant's reads as a folder that does not exist, and nothing has been
/// claimed yet); nothing when it said `null`; and, when it said nothing, the folder
/// the table's claim remembers from its latest load, so a re-upload joins the
/// table's folder (`None` for a new table or one whose folder was deleted).
///
/// # Errors
///
/// 404 for a folder that is not the tenant's; 500 on a database failure.
async fn effective_folder(
    pool: &PgPool,
    tenant_id: Uuid,
    request: &IngestRequest,
) -> ApiResult<Option<String>> {
    match &request.folder_id {
        folders::FolderRef::Id(folder) => {
            if upload_folders::folder_in_tenant(pool, tenant_id, folder).await? {
                Ok(Some(folder.clone()))
            } else {
                Err(ApiError::NotFound(folders::FOLDER_NOT_FOUND.to_owned()).into())
            }
        }
        folders::FolderRef::Null => Ok(None),
        folders::FolderRef::Absent => {
            Ok(upload_folders::claim_folder(pool, tenant_id, &request.table).await?)
        }
    }
}

/// The reading the request confirmed, as the preview takes it.
fn reading_of(request: &IngestRequest) -> PreviewQuery {
    PreviewQuery {
        encoding: Some(request.encoding.as_str().to_owned()),
        delimiter: Some(request.delimiter.to_string()),
        header_row: Some(request.header_row.to_string()),
        sheet: request.sheet.clone(),
        table: None,
    }
}

/// `DATA-17` slice 5, decision 4: refuse a load whose other files cannot go into the
/// first file's table, with the list `compare` gives ([`group::refusal`]). Nothing to
/// check for one file.
///
/// # Errors
///
/// 400 with the list, or what [`group::mismatches`] answers.
async fn check_group(
    state: &AppState,
    first: &Upload,
    others: &[group::Member],
    request: &IngestRequest,
) -> Result<(), ApiError> {
    if others.is_empty() {
        return Ok(());
    }
    // No column list is every column text, which every value fits.
    let typing = request
        .columns
        .as_ref()
        .map_or(group::Typing::Text, |plan| {
            group::Typing::Plan(&plan.columns)
        });
    let found = group::mismatches(state, first, others, &reading_of(request), &typing).await?;
    if found.is_empty() {
        Ok(())
    } else {
        Err(group::refusal(&found))
    }
}

/// The objects the job reads, the primary's first: each file's own object, or its converted
/// sheet when it is a workbook (a Parquet file's footer is checked, and it is read as it is).
/// A refusal here claims nothing.
///
/// # Errors
///
/// As [`convert_upload`] and [`check_parquet_footer`].
async fn load_keys_of(
    state: &AppState,
    row: &Upload,
    converted: Option<&Converted>,
    others: &[group::Member],
    request: &IngestRequest,
) -> Result<Vec<String>, ApiError> {
    let mut keys = Vec::with_capacity(1 + others.len());
    keys.push(converted.map_or_else(|| row.storage_key.clone(), |done| done.key.clone()));
    for other in others {
        keys.push(match other.conversion {
            Some(Conversion::Workbook) => {
                convert_upload(
                    state,
                    &other.row,
                    Conversion::Workbook,
                    request.sheet.as_deref(),
                )
                .await?
                .key
            }
            Some(Conversion::Parquet) => {
                check_parquet_footer(state, &other.row).await?;
                other.row.storage_key.clone()
            }
            None => other.row.storage_key.clone(),
        });
    }
    Ok(keys)
}

/// `POST /api/uploads/{id}/ingest` — load this file into a raw table, with
/// the encoding, delimiter and header row the user confirmed. Body
/// `{ bronzeTable, mode?, encoding, delimiter, headerRow, sheet? }`; `mode` is
/// `replace` (default) or `append`. Returns `{ upload, runId }`.
///
/// # Several files (`DATA-17` slice 5)
///
/// `alsoUploads` (at most 9 upload ids of the same tenant and the same kind of file, none
/// being received) makes one load of all of them into the one table: one run, whose config
/// lists every file's object in `storage_keys`, this upload first. The other files are
/// settled with this one ([`Settler`]); the response is for this upload. The columns of the
/// others are compared with this one's first ([`group`]) and a load whose files differ is
/// refused with the list before anything is claimed. The cap counts the files together.
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
/// ([`uploads::mark_ingesting_within_limits`], no run id); launch; attach the run
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
/// 429 (`Retry-After: 5`) when the person who uploaded the file already has
/// the configured number of loads running, or the installation has (`SEC-17`);
/// 400 for a body that is not valid (each field has its own sentence), for a
/// workbook or Parquet file that cannot be read, a sheet a workbook does not
/// have, a sheet or file over the cell limit, an empty sheet or a Parquet
/// file with no rows, and for a header with more than 1,000 columns under the
/// confirmed reading (`SEC-17`, checked before any claim); 404
/// for an unknown upload or another tenant's, or an unknown or other tenant's file of
/// `alsoUploads`, or a `folderId` that is not a folder of the upload's tenant
/// (`DATA-35`; a `folderId` that is not text is 400); 400 also for more than nine of them, a file of another kind, the files
/// together over the cap and files whose columns differ; 409 when this upload (or one of
/// `alsoUploads`) is still being received (`DATA-17`), is already loading, another is
/// loading into that table, a connector loads it, or the
/// name cannot be used ([`TABLE_NOT_FREE`]: another tenant's connector loads
/// it, another tenant holds the claim, the table exists and nobody claimed it,
/// or another tenant won the claim a moment ago); 422 when the orchestrator
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
    if row.status == "receiving" {
        return Err(ApiError::Conflict(STILL_RECEIVING.to_owned()).into());
    }
    // The row first: what the body must carry depends on what the upload is.
    let conversion = conversion_of(&state, &row).await?;
    let layout = layout_unless_converted(&state, &row, conversion).await?;
    let request = parse_ingest_request(&body, conversion, layout)?;
    let row = Settler::new(&state, pool).settle(row).await;
    if row.status == "ingesting" {
        return Err(ApiError::Conflict(ALREADY_LOADING.to_owned()).into());
    }
    let Some(tenant_id) = row.tenant_id else {
        return Err(not_found().into());
    };
    let folder = effective_folder(pool, tenant_id, &request).await?;
    let others = group::members(
        &state,
        pool,
        &principal,
        &row,
        &request.also_uploads,
        group::kind_of(conversion, layout),
    )
    .await?;
    let limits = check_within_limits_and_column_cap(&state, pool, &row, &request, layout).await?;
    ensure_table_free(&state, pool, tenant_id, &request.table).await?;
    if uploads::table_being_loaded(pool, &request.table, &row.id).await? {
        return Err(ApiError::Conflict(TABLE_BUSY.to_owned()).into());
    }
    // `DATA-18`: the column list is checked before anything is claimed.
    columns::plan_columns(&state, &row, &request, conversion).await?;
    // `DATA-17` slice 5: the other files have the first one's columns, or the load is
    // refused with the list, before anything is claimed (decision 4).
    check_group(&state, &row, &others, &request).await?;
    let converted = match conversion {
        Some(Conversion::Workbook) => Some(
            convert_upload(&state, &row, Conversion::Workbook, request.sheet.as_deref()).await?,
        ),
        // `DATA-17` T2.2: a Parquet file is read by the job as it is. Its footer
        // is checked here, as the conversion used to check it, so a refusal
        // claims no table name.
        Some(Conversion::Parquet) => {
            check_parquet_footer(&state, &row).await?;
            None
        }
        None => None,
    };
    let load_keys = load_keys_of(&state, &row, converted.as_ref(), &others, &request).await?;
    let kind = job_kind(conversion, layout);
    // Review finding B4: the name is claimed here, in one statement the
    // database arbitrates, and not before. `false` is a tenant that took the
    // name between the checks above and now, or one whose claim was always
    // there and whose own tenant is gone: the same refusal as a claim seen
    // earlier. Nothing is marked or launched for it.
    if !uploads::claim_table(pool, tenant_id, &request.table, &row.id).await? {
        return Err(ApiError::Conflict(TABLE_NOT_FREE.to_owned()).into());
    }

    let claimed = claim_load(
        pool,
        &row,
        &request,
        folder.as_deref(),
        converted.as_ref(),
        limits,
    )
    .await?;

    let run_id = launch(&state, pool, &claimed, &request, &load_keys, kind).await?;
    let attached = finish_launch_and_audit(
        pool,
        &principal,
        &claimed,
        run_id.clone(),
        &request,
        converted.as_ref(),
        conversion,
    )
    .await?;
    Ok(ApiJson(IngestResponse {
        upload: attached.into(),
        run_id,
    }))
}

/// Attach the run the launch gave us to the `ingesting` row, and record the
/// `upload.ingest` audit event the rest of the system reads. Extracted from
/// `ingest` so the handler stays under the clippy `too_many_lines` cap.
///
/// # Errors
///
/// 500 when the launched run could not be attached to its upload (an
/// `ingesting` row with no run is the only one that takes a run, so the only
/// way this happens is a row settled or was deleted between the launch and
/// the attach — a logged error, not a panic).
async fn finish_launch_and_audit(
    pool: &PgPool,
    principal: &Principal,
    claimed: &Upload,
    run_id: String,
    request: &IngestRequest,
    converted: Option<&Converted>,
    conversion: Option<Conversion>,
) -> ApiResult<Upload> {
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
    if let (Value::Object(map), false) = (&mut args, request.also_uploads.is_empty()) {
        // `DATA-17` slice 5: the other files of the load, so the audit trail of the
        // primary says which uploads went into the table with it.
        map.insert("alsoUploads".to_owned(), json!(request.also_uploads));
    }
    if let (Value::Object(map), Some(folder)) = (&mut args, &attached.folder_id) {
        map.insert("folderId".to_owned(), json!(folder));
    }
    if let Value::Object(map) = &mut args {
        match (converted, conversion) {
            (
                Some(Converted {
                    sheet: Some(sheet), ..
                }),
                _,
            ) => {
                map.insert("sheet".to_owned(), json!(sheet));
            }
            (_, Some(Conversion::Parquet)) => {
                map.insert("format".to_owned(), json!("parquet"));
            }
            _ => {}
        }
    }
    let event = upload_audit_event(principal, "upload.ingest", &attached.id, args, "executed");
    record_audit(pool, event).await;
    Ok(attached)
}

// ── Delete ───────────────────────────────────────────────────────────────

/// `DELETE /api/uploads/{id}` — remove the object (and the converted text of a
/// workbook or Parquet file), then the row. 204. For an upload still being
/// received (`DATA-17`) it aborts the multipart upload first. The
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
    if row.status == "receiving" {
        // A session has no converted text and maybe no object: abort the
        // multipart upload (its pieces are bytes S3 keeps and bills for), then
        // remove whatever sits at the key. (A session that completed between
        // the read above and this call is simply deleted whole, which is what
        // the caller asked for.)
        sessions::abort_session(&store, &row).await?;
    } else if conversion_for_name(&row.storage_key).is_some() {
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
            "This looks like a workbook or a zip archive that is not named like one. Excel workbooks must end in .xls or .xlsx and zip archives in .zip; or save the sheet as .xlsx or CSV first."
        );
        assert!(refusal(&[0xD0, 0xCF, 0x11, 0xE0, 1, 2, 3, 4]).contains("workbook"));
        assert_eq!(
            refusal(b"PAR1\x15\x04rest"),
            "This looks like a Parquet file, but its name does not end in .parquet. Rename it to end in .parquet and upload it again."
        );
        assert!(refusal(b"%PDF-1.7 ...").contains("not a file this upload reads"));
        assert!(refusal(b"a,b\0c,d").contains("not a file this upload reads"));
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
        let request = parse_ingest_request(&body(&bare), Some(Conversion::Parquet), None).unwrap();
        assert_eq!(request.encoding, Encoding::Utf8);
        assert_eq!(request.delimiter, ',');
        assert_eq!(request.header_row, 0);
        // A retry sends back what it recorded; none of it is read.
        let mut sent = bare.clone();
        sent["headerRow"] = json!(5);
        sent["encoding"] = json!("utf-16");
        sent["delimiter"] = json!(";");
        let request = parse_ingest_request(&body(&sent), Some(Conversion::Parquet), None).unwrap();
        assert_eq!((request.header_row, request.encoding), (0, Encoding::Utf8));
        // The rest of the body is checked as for any upload.
        let mut bad_table = bare;
        bad_table["bronzeTable"] = json!("Orders");
        assert_eq!(
            parse_ingest_request(&body(&bad_table), Some(Conversion::Parquet), None)
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
        for name in ["a.xlsm", "a.xlsb", "a.ods", "a.csv", "a", "a.docx", ""] {
            assert!(check_file(zip, name).is_err(), "{name}");
            assert!(check_file(&ole, name).is_err(), "{name}");
        }
        for name in ["a.xls", "a.xlsx", "a.csv", "a"] {
            assert_eq!(check_file(b"id,name\n1,a\n", name).unwrap(), Accepted::Text);
        }
        // `DATA-17` slice 4: a `.zip` is an archive; its files are checked when it is opened.
        assert_eq!(check_file(zip, "a.zip").unwrap(), Accepted::Zip);
    }

    /// `DATA-17` slice 4: each new kind needs its first bytes AND its name; a name alone
    /// never makes a file one, as for a workbook.
    #[test]
    fn json_jsonl_avro_gzip_and_zip_need_both_their_first_bytes_and_their_name() {
        let gzip = |data: &[u8]| {
            use std::io::Write as _;
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(data).unwrap();
            encoder.finish().unwrap()
        };
        let gz = gzip(b"id,name\n1,a\n");
        let avro = b"Obj\x01\x04\x16avro.schema";
        assert_eq!(
            check_file(b" [{\"a\": 1}]", "a.json").unwrap(),
            Accepted::Json
        );
        assert_eq!(
            check_file(b"\xef\xbb\xbf[]", "A.JSON").unwrap(),
            Accepted::Json
        );
        assert_eq!(
            check_file(b"{\"a\": 1}\n", "a.jsonl").unwrap(),
            Accepted::JsonLines
        );
        assert_eq!(
            check_file(b"{\"a\": 1}\n", "a.ndjson").unwrap(),
            Accepted::JsonLines
        );
        assert_eq!(check_file(avro, "a.avro").unwrap(), Accepted::Avro);
        assert_eq!(check_file(&gz, "a.csv.gz").unwrap(), Accepted::Gzip);
        assert_eq!(check_file(&gz, "a.gz").unwrap(), Accepted::Gzip);
        // A name that says JSON over text that is not an array, an object per line, or Avro.
        let refused = |bytes: &[u8], name: &str| check_file(bytes, name).unwrap_err().to_string();
        assert_eq!(
            refused(b"id,name\n1,a\n", "a.json"),
            "A JSON file must hold an array of objects."
        );
        assert_eq!(
            refused(b"{\"a\": 1}", "a.json"),
            "A JSON file must hold an array of objects."
        );
        assert!(refused(b"[1]", "a.jsonl").contains("must be a JSON object"));
        assert!(refused(b"id,name\n", "a.avro").contains("is not an Avro file"));
        assert!(refused(b"id,name\n", "a.gz").contains("is not one"));
        assert!(refused(b"id,name\n", "a.zip").contains("is not one"));
        // The right bytes under the wrong name.
        assert!(refused(&gz, "a.csv").contains("does not end in .gz"));
        assert!(refused(&gz, "a.json").contains("does not end in .gz"));
        assert!(refused(avro, "a.csv").contains("does not end in .avro"));
        assert!(refused(b"PK\x03\x04x", "a.docx").contains("not named like one"));
        // A gzip file must hold text: another gzip, a zip, or binary data is refused.
        assert!(refused(&gzip(&gz), "a.gz").contains("not another archive"));
        assert!(refused(&gzip(b"PK\x03\x04x"), "a.gz").contains("not another archive"));
        assert!(refused(&gzip(b"%PDF-1.7 ..."), "a.gz").contains("must hold a CSV"));
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

    /// Plan T6.2 item 2: the unpacked cap (what the job is told, and what a gzip,
    /// a zip or a grouped load may add up to) is 2 GiB and is not the 50 MiB of
    /// one `POST /api/uploads` file.
    #[test]
    fn the_unpacked_cap_is_2_gib_and_is_not_the_cap_of_a_whole_file() {
        assert_eq!(MAX_UNPACKED_BYTES, 2_147_483_648);
        assert_ne!(MAX_UNPACKED_BYTES, u64::try_from(MAX_UPLOAD_BYTES).unwrap());
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
        parse_ingest_request(&body(value), None, None)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn the_folder_is_optional_text_or_null_and_nothing_else() {
        let read = |value: Value| parse_ingest_request(&body(&value), None, None);
        let mut with = valid();
        assert_eq!(
            read(with.clone()).unwrap().folder_id,
            folders::FolderRef::Absent,
            "said nothing"
        );
        with["folderId"] = json!(null);
        assert_eq!(
            read(with.clone()).unwrap().folder_id,
            folders::FolderRef::Null,
            "said none"
        );
        with["folderId"] = json!("uf-1");
        assert_eq!(
            read(with.clone()).unwrap().folder_id,
            folders::FolderRef::Id("uf-1".to_owned())
        );
        for bad in [json!(3), json!(""), json!(["uf-1"]), json!(true)] {
            with["folderId"] = bad;
            assert_eq!(read(with.clone()).unwrap_err().to_string(), FOLDER_ID_RULE);
        }
    }

    #[test]
    fn a_complete_body_is_accepted_and_mode_defaults_to_replace() {
        let request = parse_ingest_request(&body(&valid()), None, None).unwrap();
        assert_eq!(
            request,
            IngestRequest {
                table: "stock_raw".to_owned(),
                mode: LoadMode::Replace,
                encoding: Encoding::Utf8,
                delimiter: ',',
                header_row: 0,
                sheet: None,
                columns: None,
                also_uploads: Vec::new(),
                folder_id: folders::FolderRef::Absent,
            }
        );
        let mut full = valid();
        full["mode"] = json!("append");
        full["encoding"] = json!("utf-16");
        full["delimiter"] = json!("\t");
        full["headerRow"] = json!(4);
        let request = parse_ingest_request(&body(&full), None, None).unwrap();
        assert_eq!(request.mode, LoadMode::Append);
        assert_eq!(request.encoding, Encoding::Utf16);
        assert_eq!(request.delimiter, '\t');
        assert_eq!(request.header_row, 4);
        let mut null_mode = valid();
        null_mode["mode"] = Value::Null;
        assert_eq!(
            parse_ingest_request(&body(&null_mode), None, None)
                .unwrap()
                .mode,
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
            let request = parse_ingest_request(&body(&with_table(ok)), None, None).unwrap();
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
            let err = parse_ingest_request(raw, None, None).unwrap_err();
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
            table: None,
        }
    }

    // ── a workbook's ingest body ────────────────────────────────────────

    #[test]
    fn a_workbook_load_is_always_utf8_and_comma_and_names_its_sheet() {
        let mut v = valid();
        v["encoding"] = json!("utf-16");
        v["delimiter"] = json!(";");
        v["sheet"] = json!("Quirks");
        let request = parse_ingest_request(&body(&v), Some(Conversion::Workbook), None).unwrap();
        assert_eq!(request.encoding, Encoding::Utf8);
        assert_eq!(request.delimiter, ',');
        assert_eq!(request.sheet.as_deref(), Some("Quirks"));
        // Not read at all, so not required either.
        let bare = json!({ "bronzeTable": "stock_raw", "headerRow": 2 });
        let request = parse_ingest_request(&body(&bare), Some(Conversion::Workbook), None).unwrap();
        assert_eq!(request.header_row, 2);
        assert_eq!(request.sheet, None);
        // Still required and checked for a text file.
        assert_eq!(
            parse_ingest_request(&body(&bare), None, None)
                .unwrap_err()
                .to_string(),
            "encoding is required."
        );
        // The rest of the body is checked for a workbook as for text.
        let mut bad_table = bare.clone();
        bad_table["bronzeTable"] = json!("Orders");
        assert_eq!(
            parse_ingest_request(&body(&bad_table), Some(Conversion::Workbook), None)
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
                parse_ingest_request(&body(body_with_bad_sheet), conversion, None)
                    .unwrap_err()
                    .to_string(),
                "sheet must be text."
            );
        }
    }

    #[test]
    fn the_other_files_are_read_from_the_body_and_the_config_lists_every_object_primary_first() {
        let mut with_others = valid();
        with_others["alsoUploads"] = json!(["up-2", "up-3"]);
        let request = parse_ingest_request(&body(&with_others), None, None).unwrap();
        assert_eq!(request.also_uploads, ["up-2", "up-3"]);
        let keys = [
            "uploads/t/up-1.csv",
            "uploads/t/up-2.csv",
            "uploads/t/up-3.csv",
        ]
        .map(str::to_owned);
        let config = run_config("up-1", &request, &keys, ("delimited", "none"));
        let job = &config["ops"]["ingest_uploaded_file"]["config"];
        assert_eq!(job["storage_keys"], json!(keys));
        assert_eq!(job["upload_id"], "up-1");
        assert_eq!(
            job["bronze_table_name"], "stock_raw",
            "the DATA-19 sensor path does not move"
        );
        let mut null_list = valid();
        null_list["alsoUploads"] = Value::Null;
        assert_eq!(
            parse_ingest_request(&body(&null_list), None, None)
                .unwrap()
                .also_uploads,
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_job_is_told_the_converted_object_and_the_fixed_dialect() {
        let request = parse_ingest_request(
            &body(&json!({ "bronzeTable": "stock_raw", "headerRow": 1, "sheet": "Stock" })),
            Some(Conversion::Workbook),
            None,
        )
        .unwrap();
        let config = run_config(
            "up-1",
            &request,
            &["uploads/t/up-1.xlsx.converted.csv".to_owned()],
            ("delimited", "none"),
        );
        let job = &config["ops"]["ingest_uploaded_file"]["config"];
        assert_eq!(job["upload_id"], "up-1");
        assert_eq!(
            job["storage_keys"],
            json!(["uploads/t/up-1.xlsx.converted.csv"])
        );
        assert_eq!(job["format"], "delimited");
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
    /// one of the eleven the API knows (six before finding C2). Each of the
    /// eleven is kept; text that is near one of them, text with detail added,
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

    /// Review findings B6 and C2: the eleven reasons the API knows are the
    /// eleven in the file (Phase 3 slice 3 review BLOCKER 3: `DATA-18` added the ninth;
    /// `DATA-17` slice 4 the tenth; PR #186 review SHOULD-FIX 1 the eleventh) the job's own tests assert its constants against
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
        assert_eq!(distinct.len(), 11, "eleven different reasons");
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
            "That table name cannot be used. Choose another name."
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
