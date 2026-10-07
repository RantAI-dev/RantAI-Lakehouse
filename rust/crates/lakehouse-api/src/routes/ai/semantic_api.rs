//! The semantic layer's API (AI-16): read every description of a table and
//! its columns, and correct one.
//!
//! | Route | Permission | Does |
//! |---|---|---|
//! | `GET /api/semantic` | `catalog:read` | every entry |
//! | `GET /api/semantic/{asset}` | `catalog:read` | one table's entries, each marked `stale` when its column is gone |
//! | `PUT /api/semantic/{asset}` | `catalog:write` | confirms one entry |
//!
//! `{asset}` is the qualified name, `serving.<table>` or `silver.<table>`.
//!
//! # Same gate as the catalog
//!
//! The entries describe the deployment's one shared set of tables, so every
//! handler first asks [`catalog_tenant_refusal`], as the catalog routes do.
//! A refused caller gets `200` with `{"supported": false, "reason": …}`,
//! also on the `PUT`, which then writes nothing: the body says so, and no
//! status code claims a success.
//!
//! # The switch does not apply here
//!
//! `AI_SEMANTIC_LAYER` stops the background drafting and the chat's reading.
//! Reading and correcting do not depend on drafting, so these routes answer
//! either way.
//!
//! # What a `PUT` checks, and in what order
//!
//! The rules of the table's `CHECK`s come first, in the handler, so a person
//! gets a `400` naming the field and not a constraint failure. Then the live
//! tables are read from `ClickHouse` with the DATA MAP's own reader
//! ([`Live`]): a table that is not in `serving` or `silver` is a `404`, a
//! column the table does not have is a `400`. When `ClickHouse` answers with
//! no table at all, the answer is `503`, not `404`: nothing was learned about
//! this table. A database failure is answered as `"database error"` through
//! `ApiError::from(StoreError)`, which never carries upstream text.

use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::semantic::{self, SemanticEntry, SemanticInput};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::data_map::{
    COLUMN_TEXT_CHARS, Live, MAX_SYNONYMS, ROLES, SYNONYM_CHARS, TABLE_TEXT_CHARS, clear_cache,
};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::catalog::catalog_tenant_refusal;
use crate::state::AppState;

/// One entry as the console reads it. The table's own entry has
/// `column: null`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryView {
    asset: String,
    column: Option<String>,
    description: String,
    synonyms: Vec<String>,
    role: Option<String>,
    /// `draft` (the model wrote it) or `confirmed` (a person did).
    status: String,
    written_by: Option<Uuid>,
    model: Option<String>,
    updated_at: String,
    /// `true` when the table or the column is gone from `ClickHouse`.
    /// Absent when the live tables were not read, so "not known" is not
    /// reported as "still there".
    #[serde(skip_serializing_if = "Option::is_none")]
    stale: Option<bool>,
}

impl EntryView {
    fn new(entry: SemanticEntry, stale: Option<bool>) -> Self {
        Self {
            asset: entry.asset,
            column: (!entry.column_name.is_empty()).then_some(entry.column_name),
            description: entry.description,
            synonyms: entry.synonyms,
            role: entry.role,
            status: entry.status,
            written_by: entry.written_by,
            model: entry.model,
            updated_at: entry.updated_at,
            stale,
        }
    }
}

/// The body of a `PUT`.
#[derive(Debug, Deserialize)]
struct PutBody {
    /// The column's name; absent for the table itself.
    column: Option<String>,
    description: String,
    #[serde(default)]
    synonyms: Vec<String>,
    role: Option<String>,
}

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "semantic store unavailable: no Postgres pool is configured".to_owned(),
        )
    })
}

fn refused(reason: &str) -> ApiJson<Value> {
    ApiJson(json!({ "supported": false, "reason": reason }))
}

/// `GET /api/semantic` — every entry of every table.
///
/// # Errors
///
/// [`ApiError::Unavailable`] when no Postgres pool is configured, and a
/// classified [`lakehouse_store::StoreError`] (`"database error"`) when the
/// read fails.
pub async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Value>> {
    if let Some(reason) = catalog_tenant_refusal(&state, &principal, &headers).await? {
        return Ok(refused(reason));
    }
    let entries = semantic::list_all(pool(&state)?).await?;
    let entries: Vec<EntryView> = entries
        .into_iter()
        .map(|e| EntryView::new(e, None))
        .collect();
    Ok(ApiJson(json!({ "entries": entries })))
}

/// `GET /api/semantic/{asset}` — one table's entries, the table's own first.
/// Each carries `stale: true` when its column (or its table) is not in the
/// live `ClickHouse` tables, and `stale: false` when it is. `stale` is absent
/// when `ClickHouse` did not answer.
///
/// # Errors
///
/// As [`list`].
pub async fn get_asset(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Path(asset): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    if let Some(reason) = catalog_tenant_refusal(&state, &principal, &headers).await? {
        return Ok(refused(reason));
    }
    let entries = semantic::list_for_asset(pool(&state)?, &asset).await?;
    // No entry, nothing to mark: skip the `ClickHouse` read.
    let live = if entries.is_empty() {
        Vec::new()
    } else {
        Live::load(&state.clickhouse).await.tables()
    };
    let found = live.iter().find(|t| t.asset == asset);
    let entries: Vec<EntryView> = entries
        .into_iter()
        .map(|e| {
            let gone = if live.is_empty() {
                None
            } else {
                Some(found.is_none_or(|t| {
                    !e.column_name.is_empty() && !t.columns.contains(&e.column_name)
                }))
            };
            EntryView::new(e, gone)
        })
        .collect();
    Ok(ApiJson(json!({ "entries": entries })))
}

/// `PUT /api/semantic/{asset}` — a person's text for the table itself or for
/// one of its columns. It replaces a draft or an earlier confirmation and is
/// marked `confirmed`, written by the caller. The audit event is
/// `semantic.confirm`, with the names of the fields written and never their
/// text.
///
/// # Errors
///
/// `400` if the body is not JSON or breaks a rule (the message names the
/// field), or the column does not exist; `404` if `asset` is not a table in
/// `serving` or `silver`; [`ApiError::Unavailable`] when no Postgres pool is
/// configured or `ClickHouse` listed no table; a classified
/// [`lakehouse_store::StoreError`] (`"database error"`) when the write fails.
pub async fn put_entry(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Path(asset): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    if let Some(reason) = catalog_tenant_refusal(&state, &principal, &headers).await? {
        return Ok(refused(reason));
    }
    let parsed: PutBody = serde_json::from_slice(&body)
        .map_err(|_| ApiError::BadRequest("body must be JSON with a `description`".to_owned()))?;
    let (input, fields) = validate(&asset, parsed)?;
    let pool = pool(&state)?;
    check_live(&state, &input).await?;
    semantic::confirm(pool, &input, principal.id.uuid()).await?;
    // The chat's DATA MAP is cached; show the person's text at once.
    clear_cache().await;
    // Best-effort, like every other audit write: it never fails the edit it
    // records.
    let _ = lakehouse_store::audit::insert(
        pool,
        lakehouse_store::audit::NewAuditEvent {
            principal_id: Some(principal.id.uuid().to_string()),
            principal_kind: Some("user".to_owned()),
            actor_label: Some(principal.display_name.clone()),
            action: "semantic.confirm".to_owned(),
            resource_kind: Some("semantic".to_owned()),
            resource_id: Some(asset),
            args: Some(json!({ "fields": fields })),
            outcome: "executed".to_owned(),
            ..Default::default()
        },
    )
    .await;
    Ok(ApiJson(json!({ "ok": true })))
}

/// The rules of `semantic_entry`'s `CHECK`s, as a `400` naming the field,
/// and the entry to write. Also returns the names of the fields the body
/// set, for the audit event. Text is trimmed and counted in characters.
fn validate(asset: &str, body: PutBody) -> Result<(SemanticInput, Vec<&'static str>), ApiError> {
    let mut fields = Vec::new();
    let column_name = match body.column {
        None => String::new(),
        Some(c) if c.is_empty() => {
            return Err(bad(
                "column",
                "must not be empty; leave it out for the table",
            ));
        }
        Some(c) => {
            fields.push("column");
            c
        }
    };
    let is_table = column_name.is_empty();

    let description = body.description.trim().to_owned();
    let max = if is_table {
        TABLE_TEXT_CHARS
    } else {
        COLUMN_TEXT_CHARS
    };
    if description.chars().count() > max {
        return Err(bad(
            "description",
            &format!("must be at most {max} characters"),
        ));
    }
    fields.push("description");

    let synonyms: Vec<String> = body.synonyms.iter().map(|s| s.trim().to_owned()).collect();
    if synonyms.len() > MAX_SYNONYMS {
        return Err(bad("synonyms", &format!("must be at most {MAX_SYNONYMS}")));
    }
    if synonyms
        .iter()
        .any(|s| s.is_empty() || s.chars().count() > SYNONYM_CHARS)
    {
        return Err(bad(
            "synonyms",
            &format!("each must be 1 to {SYNONYM_CHARS} characters"),
        ));
    }
    fields.push("synonyms");

    let role = match body.role {
        None => None,
        Some(_) if is_table => {
            return Err(bad("role", "applies to a column, not to the table"));
        }
        Some(r) if !ROLES.contains(&r.as_str()) => {
            return Err(bad("role", &format!("must be one of {}", ROLES.join(", "))));
        }
        Some(r) => {
            fields.push("role");
            Some(r)
        }
    };

    Ok((
        SemanticInput {
            asset: asset.to_owned(),
            column_name,
            description,
            synonyms,
            role,
        },
        fields,
    ))
}

fn bad(field: &str, rule: &str) -> ApiError {
    ApiError::BadRequest(format!("`{field}` {rule}"))
}

/// The table must be live in `serving` or `silver`, and so must the column.
async fn check_live(state: &AppState, input: &SemanticInput) -> Result<(), ApiError> {
    let in_scope = ["serving.", "silver."]
        .iter()
        .any(|db| input.asset.starts_with(db));
    if !in_scope {
        return Err(not_a_table(&input.asset));
    }
    let live = Live::load(&state.clickhouse).await.tables();
    if live.is_empty() {
        return Err(ApiError::Unavailable(
            "the warehouse's tables could not be read".to_owned(),
        ));
    }
    let table = live
        .iter()
        .find(|t| t.asset == input.asset)
        .ok_or_else(|| not_a_table(&input.asset))?;
    if !input.column_name.is_empty() && !table.columns.contains(&input.column_name) {
        return Err(bad("column", "is not a column of this table"));
    }
    Ok(())
}

fn not_a_table(asset: &str) -> ApiError {
    ApiError::NotFound(format!("`{asset}` is not a table in serving or silver"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_auth::{PermissionSet, PrincipalId};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::super::data_map::{CACHE_TEST_LOCK, cache_is_empty_for_test, seed_cache_for_test};
    use super::*;
    use crate::config::Config;

    /// A `ClickHouse` that lists `serving.orders` with one column, `id`.
    async fn clickhouse() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM system.tables"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [
                { "database": "serving", "name": "orders", "total_rows": "3" },
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [
                { "database": "serving", "table": "orders", "name": "id", "type": "UInt32" },
            ]})))
            .mount(&server)
            .await;
        server
    }

    fn state_for(pool: &PgPool, ch: &MockServer) -> AppState {
        let options = pool.connect_options();
        let url = format!(
            "postgres://{}:postgres@{}:{}/{}",
            options.get_username(),
            options.get_host(),
            options.get_port(),
            options
                .get_database()
                .expect("#[sqlx::test] always targets a named database"),
        );
        let env = HashMap::from([
            ("DATABASE_URL".to_owned(), url),
            ("CH_URL".to_owned(), ch.uri()),
        ]);
        AppState::new(Config::from_map(&env).expect("a valid test Config"))
    }

    fn admin() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "Fajar Nugroho".to_owned(),
            permissions: PermissionSet::parse("*:*"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// The chat's cached DATA MAP would otherwise keep the old text for up
    /// to its time to live.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_confirmation_drops_the_cached_data_map(pool: PgPool) {
        let _serial = CACHE_TEST_LOCK.lock().await;
        let ch = clickhouse().await;
        let state = state_for(&pool, &ch);
        seed_cache_for_test().await;
        let body = Bytes::from(json!({ "description": "Orders placed online." }).to_string());

        let reply = put_entry(
            State(state),
            Extension(admin()),
            HeaderMap::new(),
            Path("serving.orders".to_owned()),
            body,
        )
        .await
        .expect("the confirmation is accepted");

        assert!(
            reply.0.get("supported").is_none(),
            "the write must not be refused: {}",
            reply.0
        );
        assert!(cache_is_empty_for_test().await);
    }
}
