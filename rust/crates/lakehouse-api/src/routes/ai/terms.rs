//! `GET`/`PUT`/`DELETE /api/ai/terms`: the words a person has told the chat
//! the meaning of, remembered for that person (`chat_term`, migration
//! `0060`).
//!
//! | Route | Does |
//! |---|---|
//! | `GET /api/ai/terms` | `{ "terms": [{term, meaning, question, updatedAt}] }`, newest first |
//! | `PUT /api/ai/terms` | body `{term, meaning, question?}`; answers the stored term |
//! | `DELETE /api/ai/terms?term=` | `{ "deleted": bool }`; 200 whether or not the row existed |
//!
//! # Ownership
//!
//! Every handler acts on the caller's own rows only. The owner is
//! [`session_owner`], the key Copilot sessions and the Home layout use, and
//! it is the only identity the store is given: no body field or query
//! parameter names an owner. A request with no principal is a 401.
//!
//! # Who writes
//!
//! These routes are the only writer. The model has no tool that stores a
//! term; the console calls `PUT` when a person clicks one of the options
//! the chat offered.
//!
//! # The AI switches do not apply here
//!
//! Reading, correcting and deleting a person's own words do not depend on
//! whether the chat uses them, so these routes answer whatever the AI
//! switches are.
//!
//! # Rules and error text
//!
//! The rules (term 1 to 60 characters, meaning 1 to 200, question at most
//! 500, at most 100 terms per person) are checked in
//! `lakehouse_store::chat_term::upsert`, and its `StoreError::Validation`
//! becomes a 400 naming the field. A store failure becomes `"database
//! error"` through `ApiError::from(StoreError)`; no upstream text reaches a
//! response. With no Postgres pool the routes report `supported: false`, as
//! `routes::home` does.

use axum::body::Bytes;
use axum::extract::{Extension, Query, State};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::chat_term;
use serde::Deserialize;
use serde_json::{Value, json};

use super::session_owner;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

const UNSUPPORTED_REASON: &str = "no Postgres pool configured for this deployment";

/// The `PUT` body. `question` is the message that led to the question; a
/// client that has none leaves it out.
#[derive(Debug, Deserialize)]
struct TermBody {
    term: String,
    meaning: String,
    #[serde(default)]
    question: String,
}

/// The `DELETE` query. A missing `term` is a 400 from the handler, with this
/// module's message and not the framework's.
#[derive(Debug, Deserialize)]
pub struct TermQuery {
    #[serde(default)]
    term: Option<String>,
}

fn unsupported() -> ApiJson<Value> {
    ApiJson(json!({ "supported": false, "reason": UNSUPPORTED_REASON }))
}

/// `GET /api/ai/terms`: the caller's terms, newest first.
///
/// # Errors
///
/// 401 without a signed-in user; 500 (`"database error"`) when the read
/// fails.
pub async fn list_terms(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let owner = session_owner(principal.as_ref())?;
    let Some(pool) = state.pg.as_deref() else {
        return Ok(unsupported());
    };
    let terms = chat_term::list_for_owner(pool, &owner).await?;
    Ok(ApiJson(json!({ "terms": terms })))
}

/// `PUT /api/ai/terms`: remember what the caller means by a word, and
/// answer the stored term.
///
/// # Errors
///
/// 400 when the body is not `{term, meaning, question?}`, when a rule in the
/// module doc is broken (the message names the field), or when the caller
/// already has 100 terms and this one is new; 401 without a signed-in user;
/// 500 (`"database error"`) when the write fails.
pub async fn put_term(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let owner = session_owner(principal.as_ref())?;
    let Some(pool) = state.pg.as_deref() else {
        return Ok(unsupported());
    };
    // Parsed by hand rather than with the `Json` extractor so a malformed
    // body gets this module's message and not the framework's.
    let parsed: TermBody = serde_json::from_slice(&body).map_err(|_| {
        ApiError::BadRequest(
            "body must be {\"term\": string, \"meaning\": string, \"question\": string}".to_owned(),
        )
    })?;
    let stored = chat_term::upsert(
        pool,
        &owner,
        &parsed.term,
        &parsed.meaning,
        &parsed.question,
    )
    .await?;
    Ok(ApiJson(json!(stored)))
}

/// `DELETE /api/ai/terms?term=`: forget one of the caller's terms.
/// Idempotent: a term the caller does not have answers `deleted: false`.
///
/// # Errors
///
/// 400 without a `term`; 401 without a signed-in user; 500 (`"database
/// error"`) when the delete fails.
pub async fn delete_term(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Query(query): Query<TermQuery>,
) -> ApiResult<ApiJson<Value>> {
    let owner = session_owner(principal.as_ref())?;
    let Some(pool) = state.pg.as_deref() else {
        return Ok(unsupported());
    };
    let term = query
        .term
        .ok_or_else(|| ApiError::BadRequest("term is required".to_owned()))?;
    let deleted = chat_term::delete(pool, &owner, &term).await?;
    Ok(ApiJson(json!({ "deleted": deleted })))
}
