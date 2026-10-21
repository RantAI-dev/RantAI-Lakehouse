//! Chunked, resumable uploads: `POST /api/uploads/sessions`,
//! `PUT /api/uploads/{id}/parts/{n}`, `GET /api/uploads/{id}/parts` and
//! `POST /api/uploads/{id}/complete` (`DATA-17`, plan AD1, ADR 0014 decision 2
//! as amended).
//!
//! # The protocol
//!
//! 1. The browser opens a session with the file's name and size. The API makes
//!    a `receiving` row (`file_upload`, `0090_upload_parts.sql`), opens an S3
//!    multipart upload at the file's server-made key, and answers with the
//!    piece size ([`PART_SIZE`], 8 MiB) and the number of pieces.
//! 2. It sends the pieces, numbered from 1, in any order, up to four at a time.
//!    Each is stored as one part of the multipart upload and recorded with its
//!    `ETag` and SHA-256. A piece sent again replaces the earlier copy, which is
//!    what makes a retry safe. `GET .../parts` says which pieces the API
//!    holds, so a client that lost its connection (or met an API restart)
//!    sends only the rest.
//! 3. It completes the upload. The API checks that every piece is there and
//!    that their sizes add up to the size announced, asks S3 to assemble the
//!    object, reads the first 64 KiB and applies the same rules `POST
//!    /api/uploads` applies to a whole file, with the same sentences. A file
//!    the rules refuse is removed, object and row.
//!
//! The state lives in Postgres, not in this process, so any API process can
//! take the next piece and a restart loses nothing. The hourly sweep
//! (`upload_sweep`) removes sessions nobody finished.
//!
//! # What holds memory
//!
//! One piece, 8 MiB, while it is received: the [`crate::upload_limits::
//! ReceiveLimiter`] place of `SEC-17` is taken BEFORE the body is read and
//! held until the handler returns, so "four per user, sixteen in all" bounds
//! the memory of this route as it bounds `POST /api/uploads`. Opening a
//! session reads no file, and completing a text file reads 64 KiB. Completing a
//! Parquet file reads its footer, by range (`upload_parquet_range`), never the
//! file. Completing a workbook reads the whole object, as `POST /api/uploads`
//! does (it must be opened whole), so that path takes a place too and is refused
//! above [`super::MAX_UPLOAD_BYTES`]: a workbook stays at 50 MB (feature page,
//! decision 9).
//!
//! # Posture
//!
//! Every route needs `connector:manage` (`POLICY_TABLE`) and the `{id}` ones sit
//! behind `require_upload_in_tenants`, so another tenant's session answers the
//! 404 an unknown id gets. The key and the multipart id are server-made and
//! never leave the API. No response carries storage text: a store failure is
//! `UploadStoreError`'s fixed sentence.

use std::collections::BTreeSet;

use axum::Extension;
use axum::RequestExt as _;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::uploads::{self, NewReceiving, Upload, UploadPart};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    Accepted, CreateResponse, EMPTY_FILE, MAX_UPLOAD_BYTES, NO_TENANT, TOO_LARGE, UploadView,
    blocking, classify_head, display_name, not_found, pool, record_audit, sha256_hex, storage_key,
    upload_audit_event,
};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::upload_formats;
use crate::upload_limits::too_many_uploads;
use crate::upload_parquet;
use crate::upload_parquet_range::RangeReader;
use crate::upload_store::{UploadStore, UploadStoreError};
use crate::upload_workbook;

/// The size of every piece but the last: 8 MiB. S3 refuses to assemble an
/// upload whose pieces before the last are under 5 MiB (checked against
/// `RustFS`, plan section 8 item 1), and 8 MiB keeps a 2 GiB file at 256 pieces
/// against S3's limit of 10,000.
pub(crate) const PART_SIZE: u64 = 8 * 1024 * 1024;

/// What the body limit of the piece route allows: a piece plus 64 KiB, so the
/// limit is not what decides whether a piece is the right size (the size check
/// below does, with its own sentence).
pub(crate) const MAX_PART_BODY_BYTES: usize = 8 * 1024 * 1024 + 64 * 1024;

/// The largest file a session may announce, in bytes. Behind one constant so
/// that raising the cap (slice 6 of the plan, after the measurement) is one
/// edit; `POST /api/uploads` keeps [`MAX_UPLOAD_BYTES`]. Until that
/// measurement it is the same 50 MiB, and the sentence for a file over it is the
/// existing one.
const MAX_SESSION_BYTES: u64 = 50 * 1024 * 1024;

/// How much of the assembled file is read to tell what it is.
const COMPLETE_SNIFF_BYTES: u64 = 64 * 1024;

/// How many missing piece numbers an answer lists before it says "and N
/// more". A bound on the sentence, not on the check.
const MAX_LISTED_MISSING: usize = 20;

/// Open (`receiving`) sessions one user may hold. Feature page decision 5 needs
/// 10 at once; 20 leaves room for a retry or two that has not been swept (Phase 3
/// slice 1 review SHOULD-FIX 1). The count is read before the new session is made,
/// so two requests at the same moment can overshoot by one: it bounds the rows
/// and multipart uploads a user can leave behind, it is not a security limit.
const MAX_OPEN_SESSIONS: i64 = 20;

/// The 429 for a user at [`MAX_OPEN_SESSIONS`]: fixed text, and a longer wait
/// than the piece limit's, since a session is freed by finishing or removing
/// an upload and not by a request ending.
const TOO_MANY_SESSIONS: &str =
    "Too many uploads are open. Finish or remove some of them, then try again.";

const SESSION_BODY: &str =
    "The request body must be a JSON object with a filename and a sizeBytes (a whole number).";
const NOT_RECEIVING: &str = "This upload is not receiving pieces.";
const PIECE_NUMBER_RULE: &str = "That piece number is outside this upload.";
const PIECE_UNREADABLE: &str = "The piece could not be read.";
const PIECES_ADD_UP: &str =
    "The pieces do not add up to the size announced when the upload was opened.";

/// The number of pieces a file of `size` bytes is sent in.
fn part_count(size: u64) -> u64 {
    size.div_ceil(PART_SIZE)
}

/// The exact size piece `number` (from 1) of a file of `size` bytes must have:
/// [`PART_SIZE`] for every piece but the last, and what is left for the last.
fn expected_part_len(size: u64, number: u64) -> u64 {
    let count = part_count(size);
    if number < count {
        PART_SIZE
    } else {
        size.saturating_sub(count.saturating_sub(1) * PART_SIZE)
    }
}

/// The sentence for a piece of the wrong size. Built from the constant, so it
/// cannot disagree with the check.
fn piece_size_rule() -> String {
    format!(
        "Every piece must be exactly {PART_SIZE} bytes, except the last, which holds the rest of the file."
    )
}

/// What the session row says about its pieces, or the 409 that it is not
/// receiving: `(declared size, multipart id)`.
fn receiving_facts(row: &Upload) -> Result<(u64, &str), ApiError> {
    let declared = row.declared_size.and_then(|n| u64::try_from(n).ok());
    match (row.status.as_str(), declared, row.multipart_id.as_deref()) {
        ("receiving", Some(declared), Some(multipart_id)) => Ok((declared, multipart_id)),
        _ => Err(ApiError::Conflict(NOT_RECEIVING.to_owned())),
    }
}

// ── Open a session ───────────────────────────────────────────────────────

/// `POST /api/uploads/sessions`'s body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionRequest {
    filename: String,
    size_bytes: u64,
}

/// `POST /api/uploads/sessions`'s response: the new `receiving` upload and the
/// shape of the pieces to send.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    upload: UploadView,
    /// Bytes in every piece but the last.
    part_size: u64,
    /// How many pieces, numbered `1..=part_count`.
    part_count: u64,
}

/// `POST /api/uploads/sessions` — `{filename, sizeBytes}`. 201 with the
/// `receiving` upload, `partSize` and `partCount`.
///
/// # Errors
///
/// 400 for a caller in no tenant, a body that is not that object, a size of 0
/// ("The file is empty.") or above the cap (the existing size sentence); 404
/// if `X-Tenant` names a tenant the caller does not belong to; 503 when
/// Postgres or the object store is unavailable.
pub async fn create_session(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<SessionResponse>)> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let Some(tenant_id) = crate::tenant_scope::resolve(&principal, &headers)? else {
        return Err(ApiError::BadRequest(NO_TENANT.to_owned()).into());
    };
    let request: SessionRequest =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest(SESSION_BODY.to_owned()))?;
    if request.size_bytes == 0 {
        return Err(ApiError::BadRequest(EMPTY_FILE.to_owned()).into());
    }
    if request.size_bytes > MAX_SESSION_BYTES {
        return Err(ApiError::BadRequest(TOO_LARGE.to_owned()).into());
    }
    let declared = i64::try_from(request.size_bytes)
        .map_err(|_| ApiError::BadRequest(TOO_LARGE.to_owned()))?;
    let pool = pool(&state)?;
    // Phase 3 slice 1 review SHOULD-FIX 1: counted before anything is opened
    // in storage, so a refusal leaves no multipart upload behind.
    if uploads::count_receiving(pool, &principal.display_name).await? >= MAX_OPEN_SESSIONS {
        return Err(ApiError::TooManyRequests {
            message: TOO_MANY_SESSIONS.to_owned(),
            retry_after_secs: 30,
        }
        .into());
    }
    let store = UploadStore::connect(&state.config).await?;

    let filename = display_name(Some(&request.filename));
    let id = format!("up-{}", Uuid::new_v4());
    let key = storage_key(&tenant_id.to_string(), &id, &filename);
    // Storage first, row second, as for a whole file: a row that exists has an
    // upload to receive into. A row whose insert fails takes the upload back.
    let multipart_id = store.create_multipart(&key).await?;
    let row = match uploads::insert_receiving(
        pool,
        &NewReceiving {
            id: &id,
            original_filename: &filename,
            storage_key: &key,
            content_type: "",
            declared_size: declared,
            multipart_id: &multipart_id,
            uploaded_by: &principal.display_name,
            tenant_id,
        },
    )
    .await
    {
        Ok(row) => row,
        Err(err) => {
            if let Err(cleanup) = store.abort_multipart(&key, &multipart_id).await {
                tracing::warn!(%cleanup, key = %key, "an upload session could not be aborted after its row failed");
            }
            return Err(err.into());
        }
    };
    Ok((
        StatusCode::CREATED,
        ApiJson(SessionResponse {
            upload: row.into(),
            part_size: PART_SIZE,
            part_count: part_count(request.size_bytes),
        }),
    ))
}

// ── Send a piece ─────────────────────────────────────────────────────────

/// `PUT /api/uploads/{id}/parts/{n}`'s response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartResponse {
    part_number: u64,
    size_bytes: u64,
}

/// `PUT /api/uploads/{id}/parts/{n}` — the raw bytes of piece `n`. 200 with the
/// piece number and size. A piece sent again replaces the earlier copy.
///
/// The body is read AFTER the `SEC-17` place is taken, which is why this
/// handler takes the request and not a `Bytes`: an extractor would read the
/// body before the handler ran, and the place would not be held while the
/// memory is.
///
/// # Errors
///
/// 429 (`Retry-After: 5`) when the caller already has the configured number of
/// pieces or files being received; 400 for a piece number outside the upload, a
/// body that cannot be read or is not the exact size; 409 when the upload is
/// not receiving (finished, loading or removed meanwhile); 404 for an unknown
/// upload or another tenant's; 503 when Postgres or the object store is
/// unavailable.
pub async fn put_part(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path((id, number)): Path<(String, String)>,
    request: axum::extract::Request,
) -> ApiResult<ApiJson<PartResponse>> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    // A place BEFORE the body is read (`SEC-17`), given back when `_place`
    // drops, on every path.
    let Some(_place) = state.upload_receive.try_acquire(&principal.display_name) else {
        return Err(too_many_uploads().into());
    };
    let pool = pool(&state)?;
    let row = uploads::get(pool, &id).await?.ok_or_else(not_found)?;
    let (declared, multipart_id) = receiving_facts(&row)?;
    let number: u64 = number
        .parse()
        .ok()
        .filter(|n| (1..=part_count(declared)).contains(n))
        .ok_or_else(|| ApiError::BadRequest(PIECE_NUMBER_RULE.to_owned()))?;
    let expected = expected_part_len(declared, number);

    // The body limit of the route (`MAX_PART_BODY_BYTES`) is applied by
    // `into_limited_body`, and this read is bounded by the same number, so a
    // body past it is refused before it is held.
    let body = request.into_limited_body();
    let bytes = axum::body::to_bytes(body, MAX_PART_BODY_BYTES)
        .await
        .map_err(|err| {
            tracing::warn!(%err, upload_id = %id, "an upload piece could not be read");
            ApiError::BadRequest(PIECE_UNREADABLE.to_owned())
        })?;
    if u64::try_from(bytes.len()).ok() != Some(expected) {
        return Err(ApiError::BadRequest(piece_size_rule()).into());
    }

    let store = UploadStore::connect(&state.config).await?;
    let sha256 = sha256_hex(&bytes);
    let etag = store
        .put_part(
            &row.storage_key,
            multipart_id,
            u32::try_from(number).unwrap_or(u32::MAX),
            bytes,
        )
        .await?;
    let recorded = uploads::record_part(
        pool,
        &id,
        &UploadPart {
            part_number: i32::try_from(number).unwrap_or(i32::MAX),
            size_bytes: i64::try_from(expected).unwrap_or(i64::MAX),
            etag,
            sha256,
        },
    )
    .await?;
    if !recorded {
        // The upload was finished or removed while the piece was in flight.
        return Err(ApiError::Conflict(NOT_RECEIVING.to_owned()).into());
    }
    Ok(ApiJson(PartResponse {
        part_number: number,
        size_bytes: expected,
    }))
}

// ── Which pieces are there ───────────────────────────────────────────────

/// `GET /api/uploads/{id}/parts`'s response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartsResponse {
    /// The numbers of the pieces the API holds, ascending.
    received: Vec<u64>,
    part_size: u64,
    part_count: u64,
}

/// `GET /api/uploads/{id}/parts` — which pieces the API holds, so a client that
/// lost its connection sends only the rest.
///
/// # Errors
///
/// 409 when the upload is not receiving; 404 for an unknown upload or another
/// tenant's; 503 when Postgres is unavailable.
pub async fn list_parts(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<PartsResponse>> {
    let pool = pool(&state)?;
    let row = uploads::get(pool, &id).await?.ok_or_else(not_found)?;
    let (declared, _) = receiving_facts(&row)?;
    let received = uploads::list_parts(pool, &id)
        .await?
        .into_iter()
        .filter_map(|part| u64::try_from(part.part_number).ok())
        .collect();
    Ok(ApiJson(PartsResponse {
        received,
        part_size: PART_SIZE,
        part_count: part_count(declared),
    }))
}

// ── Complete ─────────────────────────────────────────────────────────────

/// The piece numbers in `1..=count` that are not in `have`, as a sentence.
fn missing_pieces(count: u64, have: &BTreeSet<u64>) -> Option<String> {
    let missing: Vec<u64> = (1..=count).filter(|n| !have.contains(n)).collect();
    if missing.is_empty() {
        return None;
    }
    let listed: Vec<String> = missing
        .iter()
        .take(MAX_LISTED_MISSING)
        .map(u64::to_string)
        .collect();
    let more = missing.len().saturating_sub(MAX_LISTED_MISSING);
    let tail = if more > 0 {
        format!(", and {more} more")
    } else {
        String::new()
    };
    Some(format!(
        "Some pieces have not arrived: {}{tail}.",
        listed.join(", ")
    ))
}

/// SHA-256 over the pieces' SHA-256s (32 raw bytes each) in part-number order:
/// the stand-in for a hash of the whole file, which is never held. `None` when
/// a recorded hash is not 64 hex digits (it is the API's own output, so this
/// is a database that was edited by hand).
fn parts_digest(parts: &[UploadPart]) -> Option<Vec<u8>> {
    let mut hasher = Sha256::new();
    for part in parts {
        let hex = part.sha256.as_bytes();
        if hex.len() != 64 {
            return None;
        }
        for pair in hex.chunks(2) {
            let text = std::str::from_utf8(pair).ok()?;
            hasher.update([u8::from_str_radix(text, 16).ok()?]);
        }
    }
    Some(hasher.finalize().to_vec())
}

/// Remove a file the rules refused, object and row, and give the refusal back.
/// A failed removal is logged: the sweep ([`crate::upload_sweep`]) finds the
/// row again, which is why the row is deleted last.
async fn refuse_and_remove(
    pool: &PgPool,
    store: &UploadStore,
    row: &Upload,
    refusal: ApiError,
) -> ApiError {
    let removed = store.delete(&row.storage_key).await;
    match removed {
        Ok(()) => {
            if let Err(err) = uploads::delete_receiving(pool, &row.id).await {
                tracing::warn!(%err, upload_id = %row.id, "a refused upload's row could not be removed");
            }
        }
        Err(err) => {
            tracing::warn!(%err, upload_id = %row.id, "a refused upload's object could not be removed");
        }
    }
    refusal
}

/// `POST /api/uploads/{id}/complete` — assemble the pieces and check the file.
/// 200 with the same body as `POST /api/uploads`: the upload (now `uploaded`)
/// and, for a workbook or a Parquet file, its facts, and `duplicateOf` when the
/// tenant already holds a chunked upload of the same pieces.
///
/// # Errors
///
/// 409 when the upload is not receiving, when pieces are missing (the sentence
/// lists their numbers) or when the pieces do not add up to the announced size;
/// 400 with the sentence `POST /api/uploads` gives for a file it refuses (a
/// workbook or Parquet file that does not open, another binary, a workbook
/// over 50 MB), after which the file is removed; 429 when the file must be
/// read whole (a workbook or a Parquet file) and the caller has no place; 404
/// for an unknown upload or another tenant's; 503 when Postgres or the object
/// store is unavailable.
pub async fn complete(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<CreateResponse>> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let pool = pool(&state)?;
    let row = uploads::get(pool, &id).await?.ok_or_else(not_found)?;
    let (declared, multipart_id) = receiving_facts(&row)?;
    let count = part_count(declared);
    let parts = uploads::list_parts(pool, &id).await?;
    let have: BTreeSet<u64> = parts
        .iter()
        .filter_map(|part| u64::try_from(part.part_number).ok())
        .collect();
    if let Some(sentence) = missing_pieces(count, &have) {
        return Err(ApiError::Conflict(sentence).into());
    }
    let total: i64 = parts.iter().map(|part| part.size_bytes).sum();
    if u64::try_from(total).ok() != Some(declared) {
        return Err(ApiError::Conflict(PIECES_ADD_UP.to_owned()).into());
    }
    let Some(digest) = parts_digest(&parts) else {
        tracing::warn!(upload_id = %id, "a recorded piece hash is not a SHA-256");
        return Err(ApiError::Internal(PIECES_ADD_UP.to_owned()).into());
    };

    let store = UploadStore::connect(&state.config).await?;
    let etags: Vec<String> = parts.iter().map(|part| part.etag.clone()).collect();
    if let Err(err) = store
        .complete_multipart(&row.storage_key, multipart_id, &etags)
        .await
    {
        // Phase 3 slice 1 review SHOULD-FIX 2: a completion that went through
        // whose rows did not (the database failed after it, or its answer was
        // lost) leaves the upload gone from S3 and the object in place. On the
        // retry, an object of exactly the declared size at this session's own
        // key IS the assembled file (the key is server-made and unique), so
        // carry on from the sniff. Anything else is the original failure.
        match store.object_size(&row.storage_key).await {
            Ok(Some(size)) if size == declared => {
                tracing::warn!(upload_id = %id, "completing an upload that was already assembled");
            }
            _ => return Err(err.into()),
        }
    }

    let head = store
        .get_range(&row.storage_key, 0, COMPLETE_SNIFF_BYTES)
        .await?;
    let accepted = match classify_head(&head, &row.original_filename) {
        Ok(accepted) => accepted,
        Err(refusal) => return Err(refuse_and_remove(pool, &store, &row, refusal).await.into()),
    };
    let facts = match open_whole(&state, &store, &row, accepted, declared).await {
        Ok(facts) => facts,
        Err(refusal) => return Err(refuse_and_remove(pool, &store, &row, refusal).await.into()),
    };

    let size = i64::try_from(declared).unwrap_or(i64::MAX);
    let Some(done) = uploads::finish_receiving(pool, &id, size, &digest).await? else {
        // Finished or removed by another request in the meantime.
        return Err(ApiError::Conflict(NOT_RECEIVING.to_owned()).into());
    };
    // Best effort, as for a whole file: the upload exists, so a failed lookup
    // costs the notice and nothing else.
    let duplicate =
        match uploads::find_by_parts_digest(pool, row.tenant_id.unwrap_or_default(), &digest, &id)
            .await
        {
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
        json!({ "fileName": done.original_filename, "sizeBytes": done.size_bytes, "chunked": true }),
        "executed",
    );
    record_audit(pool, event).await;
    Ok(ApiJson(CreateResponse {
        upload: done.into(),
        duplicate_of: duplicate.map(Into::into),
        workbook: facts.workbook,
        parquet: facts.parquet,
    }))
}

/// What opening a whole file taught: only a workbook or a Parquet file is
/// opened.
struct WholeFacts {
    workbook: Option<upload_workbook::WorkbookInfo>,
    parquet: Option<upload_parquet::ParquetInfo>,
}

/// Open a workbook or a Parquet file, the way `POST /api/uploads` does before
/// it stores one. A workbook must be read whole: the read takes a `SEC-17`
/// place (it holds up to [`MAX_UPLOAD_BYTES`] in memory), and a workbook above
/// that size is refused with the existing sentence; this is where a workbook's
/// 50 MB limit stays when sessions accept more (feature page, decision 9). A
/// Parquet file's footer is read by range and checked (`DATA-17` T2.2), and the
/// file is not held. A text file is not read at all.
async fn open_whole(
    state: &AppState,
    store: &UploadStore,
    row: &Upload,
    accepted: Accepted,
    declared: u64,
) -> Result<WholeFacts, ApiError> {
    let nothing = || WholeFacts {
        workbook: None,
        parquet: None,
    };
    let cap = u64::try_from(MAX_UPLOAD_BYTES).unwrap_or(u64::MAX);
    match accepted {
        Accepted::Text | Accepted::Json | Accepted::JsonLines | Accepted::Avro => {
            return Ok(nothing());
        }
        // `DATA-17` slice 4 (decision 2): the whole file is decoded once, by range and
        // never held, to count what it unpacks to against the cap: a bomb is refused (and
        // removed by the caller) before it can be loaded.
        Accepted::Gzip => {
            let reader = RangeReader::open(store, &row.storage_key).await?;
            blocking(move || upload_formats::check_gzip(reader.sequential(), cap)).await?;
            return Ok(nothing());
        }
        Accepted::Zip => {
            let reader = RangeReader::open(store, &row.storage_key).await?;
            blocking(move || upload_formats::check_zip(reader.seekable(), cap)).await?;
            return Ok(nothing());
        }
        Accepted::Parquet | Accepted::Workbook => {}
    }
    if accepted == Accepted::Parquet {
        // `DATA-17` T2.2: only the footer is read, by range. The file is never
        // held, so it has no size cap here (the job's row cap bounds a load).
        let reader = RangeReader::open(store, &row.storage_key).await?;
        let opened = blocking(move || upload_parquet::inspect(&reader)).await?;
        return Ok(WholeFacts {
            workbook: None,
            parquet: Some(opened),
        });
    }
    if declared > u64::try_from(MAX_UPLOAD_BYTES).unwrap_or(u64::MAX) {
        return Err(ApiError::BadRequest(TOO_LARGE.to_owned()));
    }
    let Some(_place) = state.upload_receive.try_acquire(&row.uploaded_by) else {
        // Not a refusal of the file: it stays, and the caller completes again.
        return Err(too_many_uploads());
    };
    let bytes = store.get_all(&row.storage_key).await?;
    match accepted {
        Accepted::Workbook => {
            let opened = blocking(move || upload_workbook::list_sheets(&bytes)).await?;
            Ok(WholeFacts {
                workbook: Some(opened),
                parquet: None,
            })
        }
        _ => Ok(nothing()),
    }
}

/// Abort the multipart upload of a `receiving` row and remove the object that
/// may sit at its key (the delete route's step for a session). A store with no
/// such upload is success.
///
/// # Errors
///
/// [`UploadStoreError`] when the store fails for any other reason.
pub(super) async fn abort_session(
    store: &UploadStore,
    row: &Upload,
) -> Result<(), UploadStoreError> {
    if let Some(multipart_id) = row.multipart_id.as_deref() {
        store
            .abort_multipart(&row.storage_key, multipart_id)
            .await?;
    }
    store.delete(&row.storage_key).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const MIB: u64 = 1024 * 1024;

    /// The cap of a session is the cap of a whole file until the measurement
    /// of slice 6 changes it; the 50 MiB of the sentence is this number.
    #[test]
    fn a_session_is_capped_at_the_same_50_mib_as_a_whole_file_for_now() {
        assert_eq!(MAX_SESSION_BYTES, u64::try_from(MAX_UPLOAD_BYTES).unwrap());
        assert!(TOO_LARGE.contains("50 MB"));
        let limit: usize = 8 * 1024 * 1024 + 64 * 1024;
        assert_eq!(MAX_PART_BODY_BYTES, limit);
        assert_eq!(PART_SIZE, 8 * MIB);
    }

    #[test]
    fn a_file_is_cut_into_whole_pieces_and_a_short_last_one() {
        assert_eq!(part_count(1), 1);
        assert_eq!(part_count(8 * MIB), 1);
        assert_eq!(part_count(8 * MIB + 1), 2);
        assert_eq!(part_count(50 * MIB), 7);
        // 2 GiB is 256 pieces, far below S3's 10,000.
        assert_eq!(part_count(2048 * MIB), 256);

        assert_eq!(expected_part_len(20 * MIB, 1), 8 * MIB);
        assert_eq!(expected_part_len(20 * MIB, 2), 8 * MIB);
        assert_eq!(expected_part_len(20 * MIB, 3), 4 * MIB);
        assert_eq!(expected_part_len(8 * MIB, 1), 8 * MIB);
        assert_eq!(expected_part_len(5, 1), 5);
        // The pieces add up to the file for any size.
        for size in [1, 7, 8 * MIB, 8 * MIB + 1, 50 * MIB, 123_456_789] {
            let sum: u64 = (1..=part_count(size))
                .map(|n| expected_part_len(size, n))
                .sum();
            assert_eq!(sum, size, "{size}");
        }
    }

    #[test]
    fn missing_pieces_are_listed_and_a_long_list_is_cut() {
        let have: BTreeSet<u64> = [1, 2, 4].into_iter().collect();
        assert_eq!(
            missing_pieces(4, &have),
            Some("Some pieces have not arrived: 3.".to_owned())
        );
        assert_eq!(missing_pieces(2, &have), None);
        let none = BTreeSet::new();
        let sentence = missing_pieces(30, &none).unwrap();
        assert!(sentence.ends_with("20, and 10 more."), "{sentence}");
    }

    fn part(number: i32, sha256: &str) -> UploadPart {
        UploadPart {
            part_number: number,
            size_bytes: 1,
            etag: "e".to_owned(),
            sha256: sha256.to_owned(),
        }
    }

    #[test]
    fn the_digest_is_a_sha256_over_the_raw_piece_hashes_in_order() {
        let a = sha256_hex(b"a");
        let b = sha256_hex(b"b");
        let digest = parts_digest(&[part(1, &a), part(2, &b)]).unwrap();
        let mut raw = Sha256::digest(b"a").to_vec();
        raw.extend_from_slice(&Sha256::digest(b"b"));
        assert_eq!(digest, Sha256::digest(&raw).to_vec());
        // Order matters.
        assert_ne!(digest, parts_digest(&[part(1, &b), part(2, &a)]).unwrap());
        // A hash that is not 64 hex digits is not guessed at.
        assert_eq!(parts_digest(&[part(1, "abc")]), None);
        assert_eq!(parts_digest(&[part(1, &"zz".repeat(32))]), None);
    }

    #[test]
    fn the_piece_size_sentence_names_the_size_the_check_uses() {
        assert!(piece_size_rule().contains("8388608 bytes"));
    }
}
