//! `POST/GET /api/uploads`, `GET /api/uploads/{id}`,
//! `GET /api/uploads/{id}/preview`, `POST /api/uploads/{id}/ingest`,
//! `DELETE /api/uploads/{id}` — phase 1 of the file-upload feature.
//!
//! Until this existed, raw data could only enter the lakehouse through a
//! database connection an operator configured in `.env`
//! (`dagster/dispar_orchestrate/dlt_pipeline.py`). A file exported from
//! SAP, Excel or a vendor portal had no way in: someone had to convert it
//! and `\copy` it into Postgres by hand, on the server.
//!
//! # The shape of the flow
//!
//! 1. `POST /api/uploads` stores the bytes and registers the file.
//! 2. `GET /api/uploads/{id}/preview` reports what the file LOOKS like —
//!    encoding, delimiter, candidate header row, first rows — as a
//!    proposal the user can correct.
//! 3. `POST /api/uploads/{id}/ingest` launches `file_ingest_job` with the
//!    options the user confirmed, through the run-config plumbing phase 0
//!    added.
//!
//! Step 2 exists because detection is not reliable enough to be silent.
//! The file that motivated this feature was named `.xls`, was actually
//! UTF-16 tab-separated text, and carried six lines of SAP report header
//! before its real column names. Any of those guessed wrongly and quietly
//! produces a Bronze table full of garbage that looks successful.

use axum::body::Bytes;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::uploads::{self, NewUpload, Upload};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::tenant::TENANT_ID;
use crate::upload_store::PREFIX;

/// Largest file this endpoint accepts, in bytes.
///
/// The body is buffered in memory to hash and store it, so this is a
/// memory bound as much as a product limit: ten concurrent uploads at the
/// cap is 500 MB of API process. Phase 3 replaces this path with a
/// presigned URL that lets the browser write to object storage directly,
/// at which point the API never sees the bytes and this cap can rise.
pub(super) const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;

/// How much of a file the preview reads.
///
/// Enough to see an encoding marker, a delimiter and a few dozen rows of
/// a wide export; small enough that previewing a 5 GB file costs the same
/// as previewing a 5 KB one.
const PREVIEW_BYTES: usize = 256 * 1024;

/// Rows the preview returns.
const PREVIEW_ROWS: usize = 20;

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "upload store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

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

fn actor_of(principal: Option<&Principal>) -> String {
    principal.map_or_else(|| "unknown".to_owned(), |p| p.display_name.clone())
}

/// `POST /api/uploads` — multipart form with one `file` part.
///
/// # Errors
///
/// 400 when no `file` part is present or the part cannot be read; 413
/// when the file exceeds [`MAX_UPLOAD_BYTES`]; 503 when Postgres or the
/// object store is unavailable.
pub async fn create(
    State(state): State<AppState>,
    principal: Option<axum::extract::Extension<Principal>>,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, ApiJson<Value>)> {
    let pool = pool(&state)?;
    let store = state
        .upload_store()
        .await
        .map_err(|err| ApiError::Unavailable(err.to_string()))?;

    let mut filename = String::new();
    let mut content_type = String::new();
    let mut bytes: Option<Bytes> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| ApiError::BadRequest(format!("multipart tidak valid: {err}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        filename = field.file_name().unwrap_or_default().to_owned();
        content_type = field.content_type().unwrap_or_default().to_owned();
        let data = field
            .bytes()
            .await
            .map_err(|err| ApiError::BadRequest(format!("gagal membaca file: {err}")))?;
        bytes = Some(data);
        break;
    }

    let Some(data) = bytes else {
        return Err(ApiError::BadRequest(
            "multipart harus memuat satu part bernama 'file'".to_owned(),
        )
        .into());
    };
    if data.is_empty() {
        return Err(ApiError::BadRequest("file kosong".to_owned()).into());
    }
    if data.len() > MAX_UPLOAD_BYTES {
        return Err(ApiError::BadRequest(format!(
            "file {} byte melebihi batas {MAX_UPLOAD_BYTES} byte",
            data.len()
        ))
        .into());
    }

    let id = format!("up-{}", Uuid::new_v4());
    let tenant = TENANT_ID.as_str();
    let key = storage_key(tenant, &id, &filename);
    let digest = format!("{:x}", Sha256::digest(&data));
    let size = i64::try_from(data.len()).unwrap_or(i64::MAX);

    // Bytes first, registry second: a row that exists means the object
    // exists. The reverse order leaves rows pointing at nothing whenever
    // the store write fails.
    store
        .put(&key, data)
        .await
        .map_err(|err| ApiError::Unavailable(err.to_string()))?;

    let row = uploads::insert(
        pool,
        &NewUpload {
            id: &id,
            original_filename: if filename.is_empty() {
                "unnamed"
            } else {
                &filename
            },
            storage_key: &key,
            content_type: &content_type,
            size_bytes: size,
            sha256: &digest,
            uploaded_by: &actor_of(principal.as_ref().map(|p| &p.0)),
            tenant,
        },
    )
    .await?;

    // A duplicate is reported, never refused: the same file legitimately
    // arrives twice (a corrected re-export, a scheduled drop). The user
    // decides, knowing that Bronze is append-only and ingesting it again
    // doubles every count downstream.
    let duplicate = uploads::find_by_sha256(pool, &digest, &id).await?;
    let mut body = serde_json::to_value(&row).unwrap_or_else(|_| json!({}));
    if let (Some(dup), Value::Object(map)) = (duplicate, &mut body) {
        map.insert(
            "duplicateOf".to_owned(),
            json!({ "id": dup.id, "originalFilename": dup.original_filename, "status": dup.status }),
        );
    }
    Ok((StatusCode::CREATED, ApiJson(body)))
}

/// `GET /api/uploads` — newest-first list for this tenant.
///
/// # Errors
///
/// 503 when Postgres is unavailable.
pub async fn list(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let rows = uploads::list(pool(&state)?, Some(TENANT_ID.as_str()), 100).await?;
    Ok(ApiJson(json!({ "uploads": rows })))
}

/// `GET /api/uploads/{id}`.
///
/// # Errors
///
/// 404 when no such upload exists; 503 when Postgres is unavailable.
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Upload>> {
    let row = uploads::get(pool(&state)?, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("upload {id} tidak ditemukan")))?;
    Ok(ApiJson(row))
}

/// Query parameters for [`preview`].
#[derive(Debug, Deserialize)]
pub struct PreviewQuery {
    /// Override the detected encoding (`utf-8` or `utf-16`).
    #[serde(default)]
    encoding: Option<String>,
    /// Override the detected delimiter (a single character).
    #[serde(default)]
    delimiter: Option<String>,
    /// Zero-based index of the header row within the decoded lines.
    #[serde(default, rename = "headerRow")]
    header_row: Option<usize>,
}

/// `GET /api/uploads/{id}/preview` — what this file appears to contain.
///
/// Returns detection results AND the rows they produce, so the console
/// can show a user the consequence of each guess before anything is
/// ingested. Every detected value can be overridden through the query
/// parameters, and whatever the user settles on is what
/// [`ingest`] is handed.
///
/// # Errors
///
/// 404 when no such upload exists; 503 when Postgres or the object store
/// is unavailable.
pub async fn preview(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<PreviewQuery>,
) -> ApiResult<ApiJson<Value>> {
    let row = uploads::get(pool(&state)?, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("upload {id} tidak ditemukan")))?;
    let store = state
        .upload_store()
        .await
        .map_err(|err| ApiError::Unavailable(err.to_string()))?;
    let head = store
        .head_bytes(&row.storage_key, PREVIEW_BYTES)
        .await
        .map_err(|err| ApiError::Unavailable(err.to_string()))?;

    let detected_encoding = detect_encoding(&head);
    let encoding = q.encoding.as_deref().unwrap_or(detected_encoding);
    let text = decode(&head, encoding);
    let lines: Vec<&str> = text.lines().collect();

    let detected_delimiter = detect_delimiter(&lines);
    let delimiter = q
        .delimiter
        .as_deref()
        .and_then(|d| d.chars().next())
        .unwrap_or(detected_delimiter);

    let detected_header_row = detect_header_row(&lines, delimiter);
    let header_row = q.header_row.unwrap_or(detected_header_row);

    let columns: Vec<String> = lines
        .get(header_row)
        .map(|line| split_row(line, delimiter))
        .unwrap_or_default();
    let rows: Vec<Vec<String>> = lines
        .iter()
        .skip(header_row + 1)
        .filter(|line| !line.trim().is_empty())
        .take(PREVIEW_ROWS)
        .map(|line| split_row(line, delimiter))
        .collect();

    Ok(ApiJson(json!({
        "uploadId": row.id,
        "originalFilename": row.original_filename,
        "sizeBytes": row.size_bytes,
        // Both the guess and the value in force, so the console can show
        // "detected UTF-16, using UTF-8 (your choice)" rather than hiding
        // that an override is active.
        "detected": {
            "encoding": detected_encoding,
            "delimiter": detected_delimiter.to_string(),
            "headerRow": detected_header_row,
        },
        "using": {
            "encoding": encoding,
            "delimiter": delimiter.to_string(),
            "headerRow": header_row,
        },
        "columns": columns,
        "rows": rows,
        // A preview reads only the first chunk, so "20 rows" here never
        // implies the file has only 20 — the console must not present this
        // count as the dataset's size.
        "truncated": row.size_bytes > i64::try_from(PREVIEW_BYTES).unwrap_or(i64::MAX),
    })))
}

/// `POST /api/uploads/{id}/ingest` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestBody {
    /// Bronze table to create, e.g. `sap_material_master`.
    bronze_table: String,
    /// Encoding to decode with (`utf-8` | `utf-16`).
    #[serde(default)]
    encoding: Option<String>,
    /// Field delimiter.
    #[serde(default)]
    delimiter: Option<String>,
    /// Zero-based header row index.
    #[serde(default)]
    header_row: Option<usize>,
}

/// `POST /api/uploads/{id}/ingest` — hand this file to `file_ingest_job`.
///
/// # Errors
///
/// 400 when `bronzeTable` is not a plain identifier; 404 when no such
/// upload exists; 422 when Dagster rejects the launch; 503 when Postgres
/// or Dagster is unavailable.
pub async fn ingest(
    State(state): State<AppState>,
    Path(id): Path<String>,
    raw_body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: IngestBody = serde_json::from_slice(&raw_body)
        .map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))?;
    let pool = pool(&state)?;
    let row = uploads::get(pool, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("upload {id} tidak ditemukan")))?;

    // The target name becomes a ClickHouse/Iceberg table identifier and a
    // catalog slug, so it is constrained here rather than escaped later.
    let table = body.bronze_table.trim().to_ascii_lowercase();
    if table.is_empty()
        || !table
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        || table.starts_with(|c: char| c.is_ascii_digit())
    {
        return Err(ApiError::BadRequest(
            "bronzeTable harus huruf kecil, angka, atau '_' dan tidak diawali angka".to_owned(),
        )
        .into());
    }

    let parse_options = json!({
        "encoding": body.encoding.unwrap_or_else(|| "utf-8".to_owned()),
        "delimiter": body.delimiter.unwrap_or_else(|| ",".to_owned()),
        "header_row": body.header_row.unwrap_or(0),
    });

    let run_config = json!({
        "ops": {
            "ingest_uploaded_file": {
                "config": {
                    "upload_id": row.id,
                    "storage_key": row.storage_key,
                    "bronze_table_name": table,
                    "encoding": parse_options["encoding"],
                    "delimiter": parse_options["delimiter"],
                    "header_row": parse_options["header_row"],
                }
            }
        }
    });

    let outcome = state
        .dagster
        .launch_run_with_config("file_ingest_job", Some(&run_config))
        .await
        .map_err(|err| ApiError::Unavailable(err.to_string()))?;
    if let Some(error) = outcome.error {
        return Err(ApiError::BadRequest(error).into());
    }

    let updated = uploads::mark_ingesting(
        pool,
        &row.id,
        &parse_options,
        &table,
        outcome.run_id.as_deref(),
    )
    .await?;
    Ok(ApiJson(json!({
        "upload": updated,
        "runId": outcome.run_id,
    })))
}

/// `DELETE /api/uploads/{id}` — remove the object, then its registry row.
///
/// # Errors
///
/// 404 when no such upload exists; 503 when Postgres or the object store
/// is unavailable.
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let pool = pool(&state)?;
    let row = uploads::get(pool, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("upload {id} tidak ditemukan")))?;
    let store = state
        .upload_store()
        .await
        .map_err(|err| ApiError::Unavailable(err.to_string()))?;
    // Object first: a failed delete here leaves the row, and the file is
    // still reachable and still listed. Deleting the row first would make
    // the object invisible to the console and findable only by a storage
    // sweep.
    if let Err(err) = store.delete(&row.storage_key).await {
        tracing::warn!(%err, key = %row.storage_key, "upload object delete failed");
    }
    let deleted = uploads::delete(pool, &row.id).await?;
    Ok(ApiJson(json!({ "deleted": deleted, "id": row.id })))
}

// ── detection ───────────────────────────────────────────────────────────

/// `utf-16` when the bytes carry a UTF-16 BOM or look like UTF-16LE
/// (ASCII text interleaved with NUL), `utf-8` otherwise.
///
/// The SAP export that motivated this feature is UTF-16LE with a BOM and
/// a `.xls` name: neither its extension nor its declared content type
/// says so.
fn detect_encoding(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return "utf-16";
    }
    let sample = &bytes[..bytes.len().min(512)];
    // A plain count, not `bytecount`: this runs once per preview over at
    // most 512 bytes, and adding a dependency for that would cost more
    // than it saves.
    #[allow(clippy::naive_bytecount, reason = "512-byte sample, once per preview")]
    let nuls = sample.iter().filter(|b| **b == 0).count();
    if sample.len() > 16 && nuls * 3 > sample.len() {
        return "utf-16";
    }
    "utf-8"
}

fn decode(bytes: &[u8], encoding: &str) -> String {
    if encoding.eq_ignore_ascii_case("utf-16") {
        let (bytes, big_endian) = match bytes {
            [0xFF, 0xFE, rest @ ..] => (rest, false),
            [0xFE, 0xFF, rest @ ..] => (rest, true),
            other => (other, false),
        };
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| {
                if big_endian {
                    u16::from_be_bytes([pair[0], pair[1]])
                } else {
                    u16::from_le_bytes([pair[0], pair[1]])
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// The delimiter that splits the sampled lines most consistently.
///
/// Consistency, not frequency: a description column full of commas makes
/// `,` the most COMMON character in a tab-separated export, but only the
/// real delimiter yields the same field count on line after line.
fn detect_delimiter(lines: &[&str]) -> char {
    const CANDIDATES: [char; 4] = ['\t', ',', ';', '|'];
    let sample: Vec<&&str> = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .take(50)
        .collect();
    let mut best = (',', 0usize);
    for candidate in CANDIDATES {
        let counts: Vec<usize> = sample
            .iter()
            .map(|line| line.matches(candidate).count())
            .filter(|n| *n > 0)
            .collect();
        if counts.is_empty() {
            continue;
        }
        let modal = counts
            .iter()
            .copied()
            .max_by_key(|n| counts.iter().filter(|m| *m == n).count())
            .unwrap_or(0);
        let agreeing = counts.iter().filter(|n| **n == modal).count();
        let score = agreeing * modal;
        if score > best.1 {
            best = (candidate, score);
        }
    }
    best.0
}

/// Index of the line that most plausibly holds column names.
///
/// The first line with the file's modal field count and no empty leading
/// cell. Report exports (SAP's "Dynamic List Display" among them) open
/// with title and date lines that have one field or a handful of stray
/// tabs; taking line 0 as the header there produces a table whose columns
/// are a report title.
fn detect_header_row(lines: &[&str], delimiter: char) -> usize {
    let counts: Vec<(usize, usize)> = lines
        .iter()
        .enumerate()
        .take(50)
        .map(|(i, line)| (i, split_row(line, delimiter).len()))
        .filter(|(_, n)| *n > 1)
        .collect();
    if counts.is_empty() {
        return 0;
    }
    let modal = counts
        .iter()
        .map(|(_, n)| *n)
        .max_by_key(|n| counts.iter().filter(|(_, m)| m == n).count())
        .unwrap_or(0);
    counts
        .iter()
        .find(|(i, n)| {
            *n == modal
                && lines
                    .get(*i)
                    .map(|line| split_row(line, delimiter))
                    .is_some_and(|cells| cells.iter().filter(|c| !c.trim().is_empty()).count() > 1)
        })
        .map_or(0, |(i, _)| *i)
}

fn split_row(line: &str, delimiter: char) -> Vec<String> {
    line.split(delimiter)
        .map(|cell| cell.trim_matches('"').trim().to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

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

    #[test]
    fn detects_utf16_from_a_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend("a\tb".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(detect_encoding(&bytes), "utf-16");
        assert_eq!(decode(&bytes, "utf-16"), "a\tb");
    }

    #[test]
    fn detects_utf8_for_plain_ascii() {
        assert_eq!(detect_encoding(b"id,name\n1,two\n"), "utf-8");
    }

    #[test]
    fn delimiter_is_the_consistent_one_not_the_common_one() {
        // Every description holds commas; only the tab count is stable.
        let lines = vec![
            "id\tdescription\tqty",
            "1\tBOLT, HEX, M6\t10",
            "2\tTAPE, WHITE, 50MM\t4",
            "3\tGLUE, FAST, 20G\t7",
        ];
        assert_eq!(detect_delimiter(&lines), '\t');
    }

    #[test]
    fn header_row_skips_a_report_preamble() {
        // The shape of the SAP export this feature was built for.
        let lines = vec![
            "24.09.2025",
            "",
            "Material master Interface for MES",
            "",
            "Plnt\tMaterial\tDescription",
            "8250\t0250161\tADHESIVE",
            "8250\t0483912\tPP BAND",
        ];
        assert_eq!(detect_header_row(&lines, '\t'), 4);
    }
}
