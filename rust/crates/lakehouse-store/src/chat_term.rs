//! A user's own words for the chat (migration `0060`): what a person meant
//! by a word the Copilot could not place, remembered per person so the
//! next chat does not ask again.
//!
//! One row per `(owner, term)`, `owner` being the same key Copilot sessions
//! are scoped to (the signed-in principal's id as a UUID string). Every
//! function takes the owner and binds it, so there is no query that reads
//! or writes across owners: that is the whole isolation story, and
//! `tests/chat_term.rs` pins it.
//!
//! The table's `CHECK` constraints hold every row rule, and [`upsert`]
//! applies the same rules first so a broken input is a
//! [`StoreError::Validation`] naming the field (safe to show a caller)
//! rather than a generic [`StoreError::Database`]. The cap of
//! [`MAX_TERMS_PER_OWNER`] words per owner is a count and not a row rule,
//! so it lives here, in [`upsert`], and not in the schema.

use serde::Serialize;
use sqlx::FromRow;
use time::OffsetDateTime;

use crate::{PgPool, StoreError};

/// Longest term, in characters, after trimming.
pub const MAX_TERM_CHARS: usize = 60;
/// Longest meaning, in characters, after trimming.
pub const MAX_MEANING_CHARS: usize = 200;
/// Longest remembered question, in characters, after trimming.
pub const MAX_QUESTION_CHARS: usize = 500;
/// Most terms one owner may keep. A new term past this is refused; an
/// existing term can still be updated.
pub const MAX_TERMS_PER_OWNER: usize = 100;

const COLUMNS: &str = "term, meaning, question, updated_at";

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

/// One remembered word, serialized straight to the console.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTerm {
    /// The word, trimmed and lower-cased.
    pub term: String,
    /// What the person said it means.
    pub meaning: String,
    /// The message that led to the question; `""` when none was kept.
    pub question: String,
    /// ISO 8601 (millisecond precision, UTC).
    pub updated_at: String,
}

#[derive(Debug, FromRow)]
struct TermRow {
    term: String,
    meaning: String,
    question: String,
    updated_at: OffsetDateTime,
}

impl From<TermRow> for ChatTerm {
    fn from(row: TermRow) -> Self {
        Self {
            term: row.term,
            meaning: row.meaning,
            question: row.question,
            updated_at: iso_millis(row.updated_at),
        }
    }
}

/// The key form of a term. Rust's `trim` is wider than Postgres's `btrim`
/// (it also strips tabs and non-breaking spaces), so the SQL below applies
/// `lower(btrim(..))` to this result again: the stored value then always
/// satisfies the table's normalisation `CHECK`.
fn normalise_term(term: &str) -> String {
    term.trim().to_lowercase()
}

fn check_length(field: &str, value: &str, max: usize) -> Result<(), StoreError> {
    let chars = value.chars().count();
    if chars == 0 || chars > max {
        return Err(StoreError::Validation(format!(
            "{field} must be 1 to {max} characters"
        )));
    }
    Ok(())
}

/// Read every term of `owner`, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_for_owner(pool: &PgPool, owner: &str) -> Result<Vec<ChatTerm>, StoreError> {
    let rows: Vec<TermRow> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM chat_term WHERE owner = $1 ORDER BY updated_at DESC, term"
    ))
    .bind(owner)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(ChatTerm::from).collect())
}

/// Save what `owner` means by `term`, replacing an earlier meaning of the
/// same term, and return what is now stored. The term is trimmed and
/// lower-cased; `meaning` and `question` are trimmed.
///
/// A term the owner does not have yet is refused once the owner has
/// [`MAX_TERMS_PER_OWNER`] terms; a term the owner already has can still be
/// updated. The count and the insert run in one transaction under a lock
/// on the owner, so two saves racing at 99 terms cannot both pass.
///
/// # Errors
///
/// Returns [`StoreError::Validation`] naming `term`, `meaning` or
/// `question` when its length is out of bounds (or `meaning` is blank), or
/// naming the limit when the owner is full; [`StoreError::Database`] if a
/// query fails.
pub async fn upsert(
    pool: &PgPool,
    owner: &str,
    term: &str,
    meaning: &str,
    question: &str,
) -> Result<ChatTerm, StoreError> {
    let term = normalise_term(term);
    let meaning = meaning.trim();
    let question = question.trim();
    check_length("term", &term, MAX_TERM_CHARS)?;
    check_length("meaning", meaning, MAX_MEANING_CHARS)?;
    if question.chars().count() > MAX_QUESTION_CHARS {
        return Err(StoreError::Validation(format!(
            "question must be at most {MAX_QUESTION_CHARS} characters"
        )));
    }

    let mut tx = pool.begin().await?;
    // Transaction-scoped, released on commit or rollback. The prefix keeps
    // the key apart from other advisory locks keyed on a bare string
    // (`overview::insert_from_fired_rule`).
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('chat_term:' || $1))")
        .bind(owner)
        .execute(&mut *tx)
        .await?;
    let row: Option<TermRow> = sqlx::query_as(&format!(
        "INSERT INTO chat_term (owner, term, meaning, question, updated_at) \
         SELECT $1, lower(btrim($2)), $3, $4, now() \
         WHERE (SELECT count(*) FROM chat_term WHERE owner = $1) < $5 \
            OR EXISTS (SELECT 1 FROM chat_term WHERE owner = $1 AND term = lower(btrim($2))) \
         ON CONFLICT (owner, term) DO UPDATE SET \
            meaning = EXCLUDED.meaning, \
            question = EXCLUDED.question, \
            updated_at = now() \
         RETURNING {COLUMNS}"
    ))
    .bind(owner)
    .bind(&term)
    .bind(meaning)
    .bind(question)
    .bind(i64::try_from(MAX_TERMS_PER_OWNER).unwrap_or(i64::MAX))
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    row.map(ChatTerm::from).ok_or_else(|| {
        StoreError::Validation(format!(
            "you can keep at most {MAX_TERMS_PER_OWNER} terms; delete one first"
        ))
    })
}

/// Remove `owner`'s `term` (trimmed and lower-cased like [`upsert`]).
/// Returns whether a row existed; deleting nothing is not an error, so the
/// route is idempotent.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the delete fails.
pub async fn delete(pool: &PgPool, owner: &str, term: &str) -> Result<bool, StoreError> {
    let result = sqlx::query("DELETE FROM chat_term WHERE owner = $1 AND term = lower(btrim($2))")
        .bind(owner)
        .bind(normalise_term(term))
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
