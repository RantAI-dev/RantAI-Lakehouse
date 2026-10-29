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
//!
//!   The answer is `lakehouse_store::connector_discovery::Answer`. A SQL source,
//!   object storage and a REST list answer in this request (`state: "done"` or
//!   `"failed"`); the types only the orchestrator reads answer
//!   `state: "running"` and the console polls.
//! - `GET /api/connectors/{id}/discovery/{kind}` -- the latest answer of that
//!   kind (404 when none was ever requested). The same shape; a `running`
//!   request older than five minutes reads as a `timeout` failure.
//!
//! The orchestrator (decisions D1, D4, D5, D6), on its `ingest:read` service
//! identity, like the `SRC-11` run reports:
//!
//! - `POST /api/connectors/{id}/discovery-results` -- the answer to a request this
//!   API launched. The body is exactly one of
//!   `{ "requestId": "...", "kind": "list", "state": "done", "objects": [..],
//!   "truncated": false }`,
//!   `{ "requestId": "...", "kind": "preview", "state": "done", "columns": [..],
//!   "primaryKey": [..] }` or
//!   `{ "requestId": "...", "kind": "list", "state": "failed", "errorClass":
//!   "unreachable" }`. An unknown field, an unknown class, a name that is blank,
//!   too long or holds a control character, more than 10,000 objects or 2,000
//!   columns, or a result past 3 MiB is a 400, and nothing is stored. The answer
//!   is `{ "recorded": true }`, or `false` for a repeat or a request that was
//!   replaced by a newer one (`lakehouse_store::connector_discovery`, "Latest
//!   wins"). Outside the tenant gate (a service identity belongs to no tenant)
//!   and `ingest:read` is also held by a user, so the handler adds
//!   `schema_changes::require_orchestrator`: a service identity or an
//!   unrestricted administrator only.
//!
//! A request the API cannot answer itself is launched as the orchestrator's
//! `discover_job` (run config in [`launch`]); the row of the request is written
//! BEFORE the launch so the job's answer always has a request to land on, and
//! a launch that fails ends the request at once instead of leaving it to time out.
//!
//! # Principle 4, the whole route
//!
//! Every answer is HTTP 200 with a `state`; a failure is an `errorClass` of the
//! closed list and the fixed `reason` for it. Nothing from a driver, a source or
//! the orchestrator reaches the response. A refusal that is the caller's own
//! mistake (a blank object, an unknown field, a too-long string) is a 400 with
//! this module's own words. The request takes up to 130 seconds
//! (`routes::route_timeout`): the listing call's limit is 120 seconds (D9).

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::connector_discovery::{
    self, Answer, ErrorClass, Kind, ListedColumn, ListedObject, Listing, MAX_NAME_CHARS,
    MAX_OBJECTS, Outcome, Preview, State as RequestState,
};
use lakehouse_store::connectors;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;

use super::schema_changes::{check_name, require_orchestrator};
use crate::connector_listing::{self, Dispatch, Request};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::connectors::{parse_body, pool};
use crate::state::AppState;

/// The orchestrator's job (`dagster/dispar_orchestrate/discover.py`).
const DISCOVER_JOB: &str = "discover_job";

/// The longest request id accepted: a UUID is 36 characters.
const MAX_REQUEST_ID_LEN: usize = 128;

/// The body limit of the orchestrator's result route: the 3 MiB the stored
/// result may take, and the envelope around it.
pub(crate) const MAX_RESULT_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Fixed 422 text: the orchestrator answered the launch with a refusal. What it
/// said went to the log at the `lakehouse-dagster` boundary.
const LAUNCH_REFUSED: &str = "The orchestrator did not accept the listing job.";

/// Fixed 503 text: the orchestrator could not be reached to start the job.
const LAUNCH_UNAVAILABLE: &str = "The orchestrator could not be reached to start the listing.";

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
        state.config.connector_tls_ca(),
    )
    .await;
    let answer = match dispatched {
        Dispatch::Done(answer) => answer,
        Dispatch::Orchestrator => launch(&state, &id, kind, &request).await?,
    };
    Ok(ApiJson(answer))
}

/// Launch the orchestrator's `discover_job` for a request the API cannot answer
/// itself (`SRC-13` decision D1) and answer `running`.
///
/// The request row is written first (so the job's answer has a request to land
/// on, whatever its speed) and carries a random id the job echoes. The run
/// config holds the connector's id and what is asked, never a credential: the
/// job fetches the connector, with its secret REFERENCES, from `ingestible` as an
/// ingest run does, and resolves them through the allowlisted resolver.
///
/// # Errors
///
/// 422 with a fixed text when the orchestrator refuses the job, 503 with a fixed
/// text when it cannot be reached; in both the request is ended at once as a
/// failure, so the console's next poll does not wait for a job that never ran.
async fn launch(
    state: &AppState,
    id: &str,
    kind: Kind,
    request: &Request,
) -> Result<Answer, ApiError> {
    let pool = pool(state)?;
    // What the list is narrowed by, for the answer to echo: a SQL source's
    // schema (Oracle), else a file glob (SFTP).
    let scope = if request.schema.is_empty() {
        request.glob.as_str()
    } else {
        request.schema.as_str()
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let now = OffsetDateTime::now_utc();
    connector_discovery::begin(pool, id, kind, &request_id, &request.object, scope, now).await?;

    let run_config = json!({"ops": {"run_discover": {"config": {
        "connector_id": id,
        "request_id": request_id,
        "kind": kind.as_str(),
        "object": request.object,
        "schema": request.schema,
        "glob": request.glob,
        "search": request.search,
        "limit": request.limit,
    }}}});
    let failure = match state
        .dagster
        .launch_run_with_config(DISCOVER_JOB, &run_config)
        .await
    {
        Ok(outcome) if outcome.failure.is_none() => None,
        Ok(_) => Some(ApiError::Unprocessable(LAUNCH_REFUSED.to_owned())),
        Err(_) => Some(ApiError::Unavailable(LAUNCH_UNAVAILABLE.to_owned())),
    };
    if let Some(failure) = failure {
        tracing::warn!(connector_id = %id, "SRC-13: the discovery job was not launched");
        let ended = Outcome::Failed(ErrorClass::Failed);
        if let Err(err) =
            connector_discovery::finish(pool, id, kind, &request_id, &ended, now).await
        {
            tracing::warn!(%err, connector_id = %id, "SRC-13: could not end a request that did not launch");
        }
        return Err(failure);
    }
    let mut answer = Answer::running(kind);
    answer.scope = (!scope.is_empty()).then(|| scope.to_owned());
    answer.object = (!request.object.is_empty()).then(|| request.object.clone());
    Ok(answer)
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

// ── The orchestrator's answer ─────────────────────────────────────────────

/// The `POST .../discovery-results` body (module docs).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResultBody {
    request_id: String,
    kind: Kind,
    state: RequestState,
    #[serde(default)]
    error_class: Option<ErrorClass>,
    #[serde(default)]
    objects: Option<Vec<ListedObject>>,
    #[serde(default)]
    truncated: Option<bool>,
    #[serde(default)]
    columns: Option<Vec<ListedColumn>>,
    #[serde(default)]
    primary_key: Option<Vec<String>>,
}

/// What the result route answers: whether this call stored the answer (`false`
/// for a repeat, or for a request a newer one replaced).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recorded {
    recorded: bool,
}

/// The result a body carries, or the 400 for a body that contradicts itself: a
/// failure with a result, a finish with a class, a list with columns.
fn outcome_of(body: ResultBody) -> Result<Outcome, ApiError> {
    let bad = |what: &str| ApiError::BadRequest(what.to_owned());
    match body.state {
        RequestState::Running => Err(bad("state must be \"done\" or \"failed\"")),
        RequestState::Failed => {
            let class = body
                .error_class
                .ok_or_else(|| bad("a failure names its errorClass"))?;
            if body.objects.is_some()
                || body.truncated.is_some()
                || body.columns.is_some()
                || body.primary_key.is_some()
            {
                return Err(bad("a failure carries no result"));
            }
            Ok(Outcome::Failed(class))
        }
        RequestState::Done => {
            if body.error_class.is_some() {
                return Err(bad("a finished request carries no errorClass"));
            }
            match body.kind {
                Kind::List => {
                    let objects = body
                        .objects
                        .ok_or_else(|| bad("a list carries its objects"))?;
                    if body.columns.is_some() || body.primary_key.is_some() {
                        return Err(bad("a list carries no columns"));
                    }
                    Ok(Outcome::Listing(Listing {
                        objects,
                        truncated: body.truncated.unwrap_or(false),
                    }))
                }
                Kind::Preview => {
                    let columns = body
                        .columns
                        .ok_or_else(|| bad("a preview carries its columns"))?;
                    if body.objects.is_some() || body.truncated.is_some() {
                        return Err(bad("a preview carries no objects"));
                    }
                    Ok(Outcome::Preview(Preview {
                        columns,
                        primary_key: body.primary_key,
                    }))
                }
            }
        }
    }
}

/// `POST /api/connectors/{id}/discovery-results` -- the orchestrator's answer to
/// a request this API launched (module docs).
///
/// # Errors
///
/// 403 for a caller that is neither the orchestrator's service identity nor an
/// unrestricted administrator; 400 for an invalid or self-contradicting body or
/// one past a bound of the contract; 404 for an unknown connector; 503/500 as
/// the store reports.
pub async fn report_result(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<Recorded>> {
    require_orchestrator(&principal)?;
    let parsed: ResultBody = parse_body(&body)?;
    check_name("requestId", &parsed.request_id, MAX_REQUEST_ID_LEN)?;
    let (request_id, kind) = (parsed.request_id.clone(), parsed.kind);
    let outcome = outcome_of(parsed)?;
    let pool = pool(&state)?;
    if connectors::get_connector(pool, &id).await?.is_none() {
        return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
    }
    let recorded = connector_discovery::finish(
        pool,
        &id,
        kind,
        &request_id,
        &outcome,
        OffsetDateTime::now_utc(),
    )
    .await?;
    Ok(ApiJson(Recorded { recorded }))
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

    fn result(json: serde_json::Value) -> Result<Outcome, ApiError> {
        outcome_of(serde_json::from_value(json).unwrap())
    }

    /// The three documented bodies become the three outcomes.
    #[test]
    fn the_documented_result_bodies_become_outcomes() {
        let listing = result(json!({
            "requestId": "r", "kind": "list", "state": "done",
            "objects": [{"name": "orders", "kind": "topic"}], "truncated": true,
        }))
        .unwrap();
        let Outcome::Listing(listing) = listing else {
            panic!("a listing");
        };
        assert!(listing.truncated);
        assert_eq!(listing.objects[0].name, "orders");

        let preview = result(json!({
            "requestId": "r", "kind": "preview", "state": "done",
            "columns": [{"name": "id", "typeName": "integer", "inferred": true}],
            "primaryKey": ["id"],
        }))
        .unwrap();
        assert!(matches!(preview, Outcome::Preview(p) if p.primary_key.is_some()));

        let failed = result(json!({
            "requestId": "r", "kind": "list", "state": "failed", "errorClass": "unreachable",
        }))
        .unwrap();
        assert_eq!(failed, Outcome::Failed(ErrorClass::Unreachable));
    }

    /// A body that contradicts itself, or names a state or class outside the
    /// contract, stores nothing (400).
    #[test]
    fn a_self_contradicting_result_body_is_refused() {
        let refused = [
            json!({"requestId": "r", "kind": "list", "state": "running"}),
            json!({"requestId": "r", "kind": "list", "state": "failed"}),
            json!({"requestId": "r", "kind": "list", "state": "failed",
                   "errorClass": "failed", "objects": []}),
            json!({"requestId": "r", "kind": "list", "state": "done"}),
            json!({"requestId": "r", "kind": "list", "state": "done", "objects": [],
                   "errorClass": "failed"}),
            json!({"requestId": "r", "kind": "list", "state": "done", "objects": [],
                   "columns": []}),
            json!({"requestId": "r", "kind": "preview", "state": "done"}),
            json!({"requestId": "r", "kind": "preview", "state": "done", "columns": [],
                   "objects": []}),
        ];
        for body in refused {
            assert!(result(body.clone()).is_err(), "{body}");
        }
        // Unknown fields, unknown states and unknown classes do not even parse:
        // the orchestrator cannot smuggle free text through any of them.
        for body in [
            json!({"requestId": "r", "kind": "list", "state": "failed",
                   "errorClass": "failed", "message": "password authentication failed"}),
            json!({"requestId": "r", "kind": "list", "state": "failed",
                   "errorClass": "password authentication failed"}),
            json!({"requestId": "r", "kind": "list", "state": "exploded"}),
            json!({"requestId": "r", "kind": "tables", "state": "done"}),
        ] {
            assert!(
                serde_json::from_value::<ResultBody>(body.clone()).is_err(),
                "{body}"
            );
        }
    }

    #[test]
    fn only_list_and_preview_are_kinds() {
        assert_eq!(kind_of("list").unwrap(), Kind::List);
        assert_eq!(kind_of("preview").unwrap(), Kind::Preview);
        assert!(kind_of("LIST").is_err());
        assert!(kind_of("columns").is_err());
    }
}
