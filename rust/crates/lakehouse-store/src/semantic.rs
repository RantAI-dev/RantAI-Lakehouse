//! The semantic layer's storage (`semantic_entry`, migration `0059`): one
//! plain-words description per table and per column, with synonyms and a
//! column's role, that the Copilot's DATA MAP reads and a person can
//! correct.
//!
//! Two writers share the table. [`insert_draft`] is the background drafting
//! pass: it never replaces a row that exists, whatever that row's status,
//! so a person's text always wins. [`confirm`] is a person's write: it
//! replaces the row, marks it `confirmed`, records the author and clears
//! the model's name. There is no delete function; nothing in the product
//! removes an entry.
//!
//! The table's own `CHECK` constraints hold every length and enum rule, and
//! the routes are expected to validate the same rules first and answer 400
//! naming the field, so a violation here surfaces as the generic
//! [`StoreError::Database`].

use serde::Serialize;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{PgPool, StoreError};

fn iso_millis(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    )
}

/// What a draft or a confirmation writes for one table or column. The
/// status, author, model and timestamp are set by [`insert_draft`] and
/// [`confirm`], not by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticInput {
    /// The qualified table name, `serving.<table>` or `silver.<table>`. At
    /// most 200 characters.
    pub asset: String,
    /// The column's name, or `""` for the table itself. At most 200
    /// characters.
    pub column_name: String,
    /// At most 400 characters for a table, 200 for a column.
    pub description: String,
    /// At most 6, each 1 to 40 characters.
    pub synonyms: Vec<String>,
    /// `measure`, `dimension`, `time` or `key` for a column; `None` for the
    /// table itself and for a column whose role is unknown.
    pub role: Option<String>,
}

/// One row of `semantic_entry`, serialized straight to the console.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticEntry {
    /// The qualified table name.
    pub asset: String,
    /// The column's name, `""` for the table itself.
    pub column_name: String,
    /// The description in plain words.
    pub description: String,
    /// Other names people use for it.
    pub synonyms: Vec<String>,
    /// `measure`, `dimension`, `time`, `key`, or `None`.
    pub role: Option<String>,
    /// `draft` (written by the model) or `confirmed` (written by a person).
    pub status: String,
    /// The person who confirmed it; `None` for a draft.
    pub written_by: Option<Uuid>,
    /// The model that drafted it; `None` once a person wrote it.
    pub model: Option<String>,
    /// ISO 8601 (millisecond precision, UTC).
    pub updated_at: String,
}

const COLUMNS: &str =
    "asset, column_name, description, synonyms, role, status, written_by, model, updated_at";

/// Read every entry of every table, ordered by table then column. The
/// table's own entry (`column_name = ''`) sorts first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn list_all(pool: &PgPool) -> Result<Vec<SemanticEntry>, StoreError> {
    let rows: Vec<EntryRow> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM semantic_entry ORDER BY asset, column_name"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(SemanticEntry::from).collect())
}

/// Read the entries of one table, its own entry first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn list_for_asset(pool: &PgPool, asset: &str) -> Result<Vec<SemanticEntry>, StoreError> {
    let rows: Vec<EntryRow> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM semantic_entry WHERE asset = $1 ORDER BY column_name"
    ))
    .bind(asset)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(SemanticEntry::from).collect())
}

/// Write the model's draft for one table or column, unless an entry of any
/// status already exists for it. Returns whether a row was written.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure, including a
/// `CHECK` violation when `input` breaks a rule of the table.
pub async fn insert_draft(
    pool: &PgPool,
    input: &SemanticInput,
    model: &str,
) -> Result<bool, StoreError> {
    let result = sqlx::query(
        "INSERT INTO semantic_entry \
            (asset, column_name, description, synonyms, role, status, model) \
         VALUES ($1, $2, $3, $4, $5, 'draft', $6) \
         ON CONFLICT (asset, column_name) DO NOTHING",
    )
    .bind(&input.asset)
    .bind(&input.column_name)
    .bind(&input.description)
    .bind(&input.synonyms)
    .bind(&input.role)
    .bind(model)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Write a person's text for one table or column, replacing any entry that
/// exists. The row becomes `confirmed`, `written_by` is the person and
/// `model` is cleared.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure, including a
/// `CHECK` violation when `input` breaks a rule of the table.
pub async fn confirm(
    pool: &PgPool,
    input: &SemanticInput,
    written_by: Uuid,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO semantic_entry \
            (asset, column_name, description, synonyms, role, status, written_by, model) \
         VALUES ($1, $2, $3, $4, $5, 'confirmed', $6, NULL) \
         ON CONFLICT (asset, column_name) DO UPDATE SET \
            description = EXCLUDED.description, \
            synonyms    = EXCLUDED.synonyms, \
            role        = EXCLUDED.role, \
            status      = 'confirmed', \
            written_by  = EXCLUDED.written_by, \
            model       = NULL, \
            updated_at  = now()",
    )
    .bind(&input.asset)
    .bind(&input.column_name)
    .bind(&input.description)
    .bind(&input.synonyms)
    .bind(&input.role)
    .bind(written_by)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, FromRow)]
struct EntryRow {
    asset: String,
    column_name: String,
    description: String,
    synonyms: Vec<String>,
    role: Option<String>,
    status: String,
    written_by: Option<Uuid>,
    model: Option<String>,
    updated_at: OffsetDateTime,
}

impl From<EntryRow> for SemanticEntry {
    fn from(row: EntryRow) -> Self {
        Self {
            asset: row.asset,
            column_name: row.column_name,
            description: row.description,
            synonyms: row.synonyms,
            role: row.role,
            status: row.status,
            written_by: row.written_by,
            model: row.model,
            updated_at: iso_millis(row.updated_at),
        }
    }
}
