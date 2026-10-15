//! Table discovery routes (`SRC-13`): what a person lists and previews from a
//! connector's source before choosing what to load.
//!
//! # The routes
//!
//! For a person with `connector:manage` on a connector of their own tenant
//! (decision D11; the tenant gate is the route layer):
//!
//! - `POST /api/connectors/{id}/discovery/{kind}` -- `{kind}` is `list` or
//!   `preview`. The body is one JSON object, every field optional except where
//!   said:
//!   `{ "schema": "public", "glob": "sales/*.csv", "search": "ord",
//!   "limit": 500, "object": "public.orders" }`.
//!   - `schema`: a SQL source's schema. Absent or empty on a list, the answer is the
//!     SCHEMAS (decision D7); with it, the base tables of that schema (D8).
//!   - `glob`: a file glob for object storage (D2), matched against the full key.
//!   - `search`: part of a name, case-insensitive, narrowing any list on the server
//!     (D9). `%` and `_` in it are ordinary characters.
//!   - `limit`: the most objects returned, 1 to 10,000 (the default).
//!   - `object`: required for `preview` and refused for `list`: the listed object
//!     (`schema.table`, a file key, an endpoint path, a topic, a collection).
//!   The answer is `lakehouse_store::connector_discovery::Answer`. A SQL source,
//!   object storage and a REST list answer in this request (`state: "done"` or
//!   `"failed"`); the types only the orchestrator reads answer
//!   `state: "running"` and the console polls.
//! - `GET /api/connectors/{id}/discovery/{kind}` -- the latest answer of that
//!   kind (404 when none was ever requested). The same shape; a `running`
//!   request older than five minutes reads as a `timeout` failure.
//!
//! # Principle 4, the whole route
//!
//! Every answer is HTTP 200 with a `state`; a failure is an `errorClass` of the
//! closed list and the fixed `reason` for it. Nothing from a driver, a source or
//! the orchestrator reaches the response. A refusal that is the caller's own
//! mistake (a blank object, an unknown field, a too-long string) is a 400 with
//! this module's own words. The request takes up to 130 seconds
//! (`routes::route_timeout`): the listing call's limit is 120 seconds (D9).

use axum::body::Bytes;
use axum::extract::{Path, State};
use lakehouse_core::ApiError;
use lakehouse_store::connector_discovery::{
    self, Answer, ErrorClass, Kind, MAX_NAME_CHARS, MAX_OBJECTS,
};
use lakehouse_store::connectors;
use serde::Deserialize;
use time::OffsetDateTime;

use crate::connector_listing::{self, Dispatch, Request};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::connectors::{parse_body, pool};
use crate::state::AppState;

/// The body of `POST .../discovery/{kind}`: every field optional.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartBody {
    schema: Option<String>,
    glob: Option<String>,
    search: Option<String>,
    limit: Option<usize>,
    object: Option<String>,
}

/// The `{kind}` path segment.
fn kind_of(segment: &str) -> Result<Kind, ApiError> {
    Kind::parse(segment)
        .ok_or_else(|| ApiError::BadRequest("kind must be \"list\" or \"preview\"".to_owned()))
}

/// A string field as accepted: at most 256 characters, no control character.
fn text_field(what: &str, value: Option<String>) -> Result<String, ApiError> {
    let value = value.unwrap_or_default();
    if value.chars().count() > MAX_NAME_CHARS || value.chars().any(char::is_control) {
        return Err(ApiError::BadRequest(format!(
            "{what} must be at most {MAX_NAME_CHARS} characters with no control character"
        )));
    }
    Ok(value)
}

/// Check a body against the kind it is sent for.
///
/// # Errors
///
/// 400 for a field that is too long or holds a control character, a `limit`
/// outside 1 to 10,000, a `preview` without an `object`, a `list` with one.
fn request_of(kind: Kind, body: StartBody) -> Result<Request, ApiError> {
    let limit = body.limit.unwrap_or(MAX_OBJECTS);
    if !(1..=MAX_OBJECTS).contains(&limit) {
        return Err(ApiError::BadRequest(format!(
            "limit must be between 1 and {MAX_OBJECTS}"
        )));
    }
    let request = Request {
        schema: text_field("schema", body.schema)?,
        glob: text_field("glob", body.glob)?,
        search: text_field("search", body.search)?,
        limit,
        object: text_field("object", body.object)?,
    };
    match (kind, request.object.trim().is_empty()) {
        (Kind::Preview, true) => Err(ApiError::BadRequest(
            "a preview needs the object to preview".to_owned(),
        )),
        (Kind::List, false) => Err(ApiError::BadRequest("a list names no object".to_owned())),
        _ => Ok(request),
    }
}

/// `POST /api/connectors/{id}/discovery/{kind}` -- see the module docs.
///
/// # Errors
///
/// 400 for a kind other than `list`/`preview`, a body that is not the documented
/// JSON, or a field outside its bounds; 404 for an unknown connector (the route
/// layer); 503/500 as the store reports. A source that cannot be read is NOT an
/// error here: it is a 200 answer with `state: "failed"` and an `errorClass`.
pub async fn start(
    State(state): State<AppState>,
    Path((id, kind)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult<ApiJson<Answer>> {
    let kind = kind_of(&kind)?;
    let parsed: StartBody = if body.iter().all(u8::is_ascii_whitespace) {
        StartBody::default()
    } else {
        parse_body(&body)?
    };
    let request = request_of(kind, parsed)?;
    let Some(info) = connectors::get_connector_dial_info(pool(&state)?, &id).await? else {
        return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
    };
    let dispatched = connector_listing::dispatch(
        kind,
        &info,
        &request,
        state.connector_secret_resolver.as_ref(),
        &state.config.connector_internal_hosts(),
    )
    .await;
    let answer = match dispatched {
        Dispatch::Done(answer) => answer,
        // `SRC-13` task 3 launches the orchestrator's `discover_job` here.
        Dispatch::Orchestrator => Answer::failed(kind, ErrorClass::NotSupported),
    };
    Ok(ApiJson(answer))
}

/// `GET /api/connectors/{id}/discovery/{kind}` -- the latest answer of that
/// kind, as the orchestrator's job posted it.
///
/// # Errors
///
/// 400 for a kind other than `list`/`preview`; 404 when no request of that kind
/// was ever made for this connector; 503/500 as the store reports.
pub async fn poll(
    State(state): State<AppState>,
    Path((id, kind)): Path<(String, String)>,
) -> ApiResult<ApiJson<Answer>> {
    let kind = kind_of(&kind)?;
    let stored = connector_discovery::get(pool(&state)?, &id, kind)
        .await?
        .ok_or_else(|| {
            ApiError::NotFound("No discovery was requested for this connector".to_owned())
        })?;
    Ok(ApiJson(stored.into_answer(OffsetDateTime::now_utc())))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn body(json: &str) -> StartBody {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_list_takes_defaults_and_a_preview_needs_its_object() {
        let list = request_of(Kind::List, body("{}")).unwrap();
        assert_eq!(list.limit, MAX_OBJECTS);
        assert!(list.schema.is_empty() && list.object.is_empty());

        assert!(request_of(Kind::Preview, body("{}")).is_err());
        assert!(request_of(Kind::Preview, body(r#"{"object": "  "}"#)).is_err());
        assert!(request_of(Kind::List, body(r#"{"object": "a.b"}"#)).is_err());
        let preview = request_of(Kind::Preview, body(r#"{"object": "public.a"}"#)).unwrap();
        assert_eq!(preview.object, "public.a");
    }

    #[test]
    fn bounds_are_enforced_before_anything_is_dialled() {
        assert!(request_of(Kind::List, body(r#"{"limit": 0}"#)).is_err());
        assert!(request_of(Kind::List, body(r#"{"limit": 10001}"#)).is_err());
        assert!(request_of(Kind::List, body(r#"{"limit": 10000}"#)).is_ok());
        let long = "x".repeat(MAX_NAME_CHARS + 1);
        for field in ["schema", "glob", "search"] {
            let json = format!(r#"{{"{field}": "{long}"}}"#);
            assert!(request_of(Kind::List, body(&json)).is_err(), "{field}");
        }
        assert!(request_of(Kind::List, body(r#"{"search": "a\nb"}"#)).is_err());
        assert!(serde_json::from_str::<StartBody>(r#"{"surprise": 1}"#).is_err());
    }

    #[test]
    fn only_list_and_preview_are_kinds() {
        assert_eq!(kind_of("list").unwrap(), Kind::List);
        assert_eq!(kind_of("preview").unwrap(), Kind::Preview);
        assert!(kind_of("LIST").is_err());
        assert!(kind_of("columns").is_err());
    }
}
