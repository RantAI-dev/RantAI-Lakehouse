//! `GET`/`PUT`/`DELETE /api/home/layout` — the signed-in user's Home page
//! layout: which cards and which "create" shortcuts show, in what order,
//! and which dashboard the preview card follows.
//!
//! # Ownership
//!
//! The layout is private to its owner, keyed exactly as Copilot sessions are
//! ([`super::ai::session_owner`]: the principal's id as a UUID string, and a
//! request with no principal is a 401). Sessions are stored in `ClickHouse`;
//! this layout is a Postgres row (`home_layout`, migration `0056`), but the
//! ownership model is the same one.
//!
//! # The server does not know the catalogue
//!
//! Validation here is about shape and size only: list lengths, the id
//! charset, no duplicates. Whether an id names a real card or shortcut is
//! deliberately NOT checked. The console owns the catalogue and drops ids
//! it does not recognise when it resolves a saved layout, so adding a card
//! (or retiring one) is a console change with no migration and no server
//! release, and an old saved layout keeps working after either.
//!
//! # `supported: false`, not a fabricated default
//!
//! With no Postgres pool the routes report `supported: false` with a reason
//! (the shape `routes::notifications` uses), never `layout: null`, which
//! would read as "this user has never customised" and invite a save that
//! cannot persist.
//!
//! # Error text
//!
//! Every message here is this module's own. Store failures go through
//! `StoreError` -> `ApiError` (`"database error"`), never upstream text.

use axum::body::Bytes;
use axum::extract::{Extension, State};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ai::session_owner;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// Most cards a layout may list. The console's catalogue is smaller; the
/// bound is headroom, not a count to keep in step with it.
const MAX_CARDS: usize = 12;
/// Most shortcuts a layout may list: the row under the composer is three
/// wide.
const MAX_SHORTCUTS: usize = 3;
/// Longest card or shortcut id.
const MAX_ID_LEN: usize = 40;
/// Longest dashboard id (`previewBoardId`). Board ids are generated
/// elsewhere, so this is a size bound and not a format check.
const MAX_BOARD_ID_LEN: usize = 100;

const UNSUPPORTED_REASON: &str = "no Postgres pool configured for this deployment";

/// The request body, and also what is stored: parsing into this struct and
/// storing it back means a stray field a client sends is dropped rather than
/// persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayoutBody {
    cards: Vec<String>,
    shortcuts: Vec<String>,
    /// The dashboard the preview card shows; `null` follows the one last
    /// opened in that browser.
    #[serde(default)]
    preview_board_id: Option<String>,
}

/// Check one list: at most `max` entries, each a 1-40 character id of
/// `[a-z0-9-]`, none repeated.
fn validate_ids(name: &str, ids: &[String], max: usize) -> Result<(), ApiError> {
    if ids.len() > max {
        return Err(ApiError::BadRequest(format!(
            "{name} may list at most {max} entries"
        )));
    }
    for (i, id) in ids.iter().enumerate() {
        let well_formed = !id.is_empty()
            && id.len() <= MAX_ID_LEN
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !well_formed {
            return Err(ApiError::BadRequest(format!(
                "{name} entries must be 1-{MAX_ID_LEN} characters of a-z, 0-9 and '-'"
            )));
        }
        if ids[..i].contains(id) {
            return Err(ApiError::BadRequest(format!(
                "{name} must not repeat an entry"
            )));
        }
    }
    Ok(())
}

/// Shape and size checks for a layout body; see the module doc for why the
/// ids are not checked against a catalogue.
fn validate(body: &LayoutBody) -> Result<(), ApiError> {
    validate_ids("cards", &body.cards, MAX_CARDS)?;
    validate_ids("shortcuts", &body.shortcuts, MAX_SHORTCUTS)?;
    if let Some(id) = &body.preview_board_id
        && (id.is_empty() || id.chars().count() > MAX_BOARD_ID_LEN)
    {
        return Err(ApiError::BadRequest(format!(
            "previewBoardId must be 1-{MAX_BOARD_ID_LEN} characters or null"
        )));
    }
    Ok(())
}

fn unsupported() -> ApiJson<Value> {
    ApiJson(json!({ "supported": false, "reason": UNSUPPORTED_REASON }))
}

fn supported(layout: Option<&Value>) -> ApiJson<Value> {
    ApiJson(json!({ "supported": true, "layout": layout }))
}

/// `GET /api/home/layout` — the saved layout, or `layout: null` when the
/// user never saved one.
///
/// # Errors
///
/// 401 without a signed-in user; 500 (`"database error"`) when the read
/// fails.
pub async fn get_layout(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let owner = session_owner(principal.as_ref())?;
    let Some(pool) = state.pg.as_deref() else {
        return Ok(unsupported());
    };
    let layout = lakehouse_store::home_layout::get(pool, &owner).await?;
    Ok(supported(layout.as_ref()))
}

/// `PUT /api/home/layout` — save the caller's layout and return it in the
/// `GET` shape.
///
/// # Errors
///
/// 400 when the body is not `{cards, shortcuts, previewBoardId}` or breaks
/// a limit in the module doc; 401 without a signed-in user; 500
/// (`"database error"`) when the write fails.
pub async fn put_layout(
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
    let parsed: LayoutBody = serde_json::from_slice(&body).map_err(|_| {
        ApiError::BadRequest(
            "body must be {\"cards\": [...], \"shortcuts\": [...], \"previewBoardId\": string|null}"
                .to_owned(),
        )
    })?;
    validate(&parsed)?;
    let value = serde_json::to_value(&parsed)
        .map_err(|_| ApiError::BadRequest("layout could not be encoded".to_owned()))?;
    let stored = lakehouse_store::home_layout::upsert(pool, &owner, &value).await?;
    Ok(supported(Some(&stored)))
}

/// `DELETE /api/home/layout` — back to the default layout. Idempotent:
/// resetting a user with nothing saved is not an error.
///
/// # Errors
///
/// 401 without a signed-in user; 500 (`"database error"`) when the delete
/// fails.
pub async fn delete_layout(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let owner = session_owner(principal.as_ref())?;
    let Some(pool) = state.pg.as_deref() else {
        return Ok(unsupported());
    };
    lakehouse_store::home_layout::delete(pool, &owner).await?;
    Ok(supported(None))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn body(cards: &[&str], shortcuts: &[&str], board: Option<&str>) -> LayoutBody {
        LayoutBody {
            cards: cards.iter().map(|s| (*s).to_owned()).collect(),
            shortcuts: shortcuts.iter().map(|s| (*s).to_owned()).collect(),
            preview_board_id: board.map(str::to_owned),
        }
    }

    #[test]
    fn a_well_formed_layout_passes_and_empty_lists_are_allowed() {
        assert!(
            validate(&body(
                &["recent", "pipeline-runs", "x1"],
                &["new-query"],
                None
            ))
            .is_ok()
        );
        assert!(validate(&body(&[], &[], Some("board-1"))).is_ok());
    }

    #[test]
    fn more_than_twelve_cards_or_three_shortcuts_is_refused() {
        let thirteen: Vec<String> = (0..13).map(|i| format!("card-{i}")).collect();
        let refs: Vec<&str> = thirteen.iter().map(String::as_str).collect();
        assert!(validate(&body(&refs[..12], &[], None)).is_ok());
        assert!(validate(&body(&refs, &[], None)).is_err());
        assert!(validate(&body(&[], &["a", "b", "c"], None)).is_ok());
        assert!(validate(&body(&[], &["a", "b", "c", "d"], None)).is_err());
    }

    #[test]
    fn ids_must_be_one_to_forty_characters_of_lowercase_digits_and_hyphen() {
        let forty = "a".repeat(40);
        let forty_one = "a".repeat(41);
        assert!(validate(&body(&[&forty], &[], None)).is_ok());
        for bad in [
            &forty_one[..],
            "",
            "Recent",
            "a_b",
            "a b",
            "a.b",
            "é",
            "a/b",
        ] {
            assert!(
                validate(&body(&[bad], &[], None)).is_err(),
                "{bad:?} should be refused"
            );
            assert!(
                validate(&body(&[], &[bad], None)).is_err(),
                "{bad:?} should be refused in shortcuts"
            );
        }
    }

    #[test]
    fn a_repeated_id_within_one_list_is_refused_but_across_lists_is_fine() {
        assert!(validate(&body(&["recent", "recent"], &[], None)).is_err());
        assert!(validate(&body(&[], &["a", "b", "a"], None)).is_err());
        assert!(validate(&body(&["recent"], &["recent"], None)).is_ok());
    }

    #[test]
    fn the_preview_board_id_is_bounded_and_may_be_null() {
        assert!(validate(&body(&[], &[], Some(&"b".repeat(100)))).is_ok());
        assert!(validate(&body(&[], &[], Some(&"b".repeat(101)))).is_err());
        assert!(validate(&body(&[], &[], Some(""))).is_err());
    }

    #[test]
    fn refusals_use_this_modules_message_and_are_bad_requests() {
        let err = validate(&body(&["Bad Id"], &[], None)).unwrap_err();
        assert_eq!(err.status(), 400);
        assert!(err.to_string().contains("cards entries must be"));
    }

    #[test]
    fn the_body_requires_both_lists_and_defaults_the_board_to_null() {
        let ok: LayoutBody = serde_json::from_str(r#"{"cards":[],"shortcuts":[]}"#).unwrap();
        assert_eq!(ok.preview_board_id, None);
        assert!(serde_json::from_str::<LayoutBody>(r#"{"cards":[]}"#).is_err());
        assert!(
            serde_json::from_str::<LayoutBody>(r#"{"cards":"recent","shortcuts":[]}"#).is_err()
        );
    }
}
