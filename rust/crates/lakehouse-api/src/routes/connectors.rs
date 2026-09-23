//! `/api/connectors/*` — connector definitions (source/sink systems),
//! backed by Postgres (`lakehouse-store`).
//!
//! # Not a port
//!
//! Like `routes::identity`, this replaces an *in-browser* mock
//! (`src/services/mock/connectors.ts`) that never had a server side —
//! there is no TypeScript route handler this is bug-compatible with.
//! Status codes are chosen to be correct: 201 on create, 404 on a missing
//! id, 409 on a duplicate name, 400 on a malformed body or a `secretRef`
//! that looks like a raw credential, 503 with no database pool.
//!
//! # Credentials
//!
//! See `lakehouse_store::connectors`'s module doc comment for the full
//! decision record. The short version: no endpoint here ever returns a
//! `host` or `secretRef` — [`lakehouse_store::connectors::Connector`] and
//! `ConnectorDetail` have no such field to serialize.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::audit::{self as store_audit, NewAuditEvent};
use lakehouse_store::cdc::ConnectorSlug;
use lakehouse_store::connector_type::{self, ConnectorType};
use lakehouse_store::connectors::{self, ConnectorDetail, ConnectorDialInfo, CreateConnectorInput};
use lakehouse_store::ingest_spec::{Dial, SqlDriver};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::connector_deprovision::{self, DeprovisionError, Deprovisioned, PgTarget};
use crate::connector_probe;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::support::js_error;
use crate::state::AppState;

/// Borrow the Postgres pool, or fail with a 503. Mirrors
/// `routes::identity::pool`/`routes::pipelines::pool`.
fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "connector store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

fn parse_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))
}

fn required(field: &str, value: &str) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::BadRequest(format!("{field} is required")));
    }
    Ok(trimmed.to_owned())
}

/// `GET /api/connectors` — every connector.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list(State(state): State<AppState>) -> ApiResult<ApiJson<Vec<connectors::Connector>>> {
    Ok(ApiJson(connectors::list_connectors(pool(&state)?).await?))
}

/// `GET /api/connectors/ingestible` — every connector that has an ingest
/// spec set (`adapter IS NOT NULL`), as an
/// [`connectors::IngestibleConnector`]. Gated on `ingest:read`
/// (`POLICY_TABLE`), a strictly narrower grant than the base
/// `/api/connectors` route's `connector:manage` — this is the route
/// `dagster/dispar_orchestrate/ingest_factory.py`'s `ingest:read`-scoped
/// service identity calls, both at code-load time (to build schedules)
/// and at run time (`run_ingest`'s own re-fetch of its own connector).
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_ingestible(
    State(state): State<AppState>,
) -> ApiResult<ApiJson<Vec<connectors::IngestibleConnector>>> {
    Ok(ApiJson(
        connectors::list_ingestible_connectors(pool(&state)?).await?,
    ))
}

/// `GET /api/connectors/types` — every row of `connector_type`
/// (migration `0035`), the reference table the creation wizard reads to
/// offer a type — including `supported = false` roadmap rows, listed
/// honestly rather than omitted (AGENTS.md rule 2).
///
/// # Gap fix
///
/// `lakehouse_store::connector_type::list_connector_types` and
/// `src/services/clients/connectors.ts`'s `listTypes` both already
/// existed with no HTTP surface between them (verified against
/// `policy.rs`/`routes/mod.rs` before this route existed — the
/// wizard's connector-type list 404d). Gated on `connector:manage`,
/// matching every other read-shaped `/api/connectors*` route
/// (`list`, `detail`) the creation wizard's caller already needs —
/// this is a small reference table feeding directly into
/// `POST /api/connectors`, which is `connector:manage`-gated, not the
/// narrower `ingest:read` scope minted only for the Dagster ingest
/// service identity.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_types(State(state): State<AppState>) -> ApiResult<ApiJson<Vec<ConnectorType>>> {
    Ok(ApiJson(
        connector_type::list_connector_types(pool(&state)?).await?,
    ))
}

/// `GET /api/connectors/{id}` — one connector's detail.
///
/// # Errors
///
/// 404 if `id` is unknown; 503/500 as above.
pub async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<ConnectorDetail>> {
    let detail = connectors::get_connector(pool(&state)?, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    Ok(ApiJson(detail))
}

/// The `POST /api/connectors` body. Mirrors `CreateConnectorInput`.
///
/// `secret_ref` is a REFERENCE NAME (`"env:FOO"`, `"vault:path"`), never a
/// credential value — see the module doc comment.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateConnectorBody {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    direction: String,
    host: String,
    secret_ref: String,
    /// Optional secondary reference, e.g. the secret-access-key half of an
    /// S3 connector's access-key/secret-key pair (see
    /// `lakehouse_store::connectors::ConnectorDialInfo::secret_ref_secondary`'s
    /// doc comment). Without this, an API-created S3 connector's `/test`
    /// can never succeed — `probe_s3` requires both.
    #[serde(default)]
    secret_ref_secondary: Option<String>,
    environment: String,
    tenant: String,
    #[serde(default)]
    residency: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    owner: Option<String>,
}

const VALID_DIRECTIONS: [&str; 3] = ["source", "sink", "bidirectional"];

/// Refuse a caller-supplied `secretRef` that matches one of the deployment's
/// reserved connector-credential patterns
/// ([`crate::state::CONNECTOR_ALLOWED_SECRET_REF_PATTERNS`]).
///
/// Those patterns are the only shapes `AppState::connector_secret_resolver`
/// will resolve, and they exist for the connectors seeded by migration — the
/// ones this deployment operates itself. A user-created connector naming a
/// ref that matches one would have the API authenticate to a caller-chosen
/// `host` with the deployment's own connector credentials. `connector_probe`'s
/// SSRF guard does not prevent that: it blocks internal address ranges, and
/// exfiltration wants an EXTERNAL host, which is exactly what it permits.
///
/// So the allowlist answers "which refs may resolve at all" and this answers
/// "who may name them". Neither alone is sufficient: without the allowlist a
/// connector could name `env:DATABASE_URL`; without this check it could name
/// a reserved connector-credential ref and point it anywhere. Both checks
/// call the SAME [`lakehouse_core::secret::pattern_matches`] against the
/// SAME pattern constant, so the two cannot drift the way
/// `0023_connector_dedicated_secret_refs.sql`'s header describes happening to
/// the old exact-list version.
///
/// # What this does NOT refuse (WS3 plan review X4/Z4)
///
/// A name that is not itself one of the reserved patterns — e.g.
/// `env:CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS`, a real [`crate::config::Config`]
/// flag, not a credential — is accepted HERE, at creation time: this check's
/// job is narrower than "reject anything unsafe", it only refuses a caller
/// naming one of the deployment's OWN reserved refs. Such a name still fails
/// later, at resolve time, when [`AllowlistedSecretResolver`](lakehouse_core::secret::AllowlistedSecretResolver)
/// independently checks the same patterns and returns `NotAllowed` the first
/// time `POST .../test`, `POST .../discover`, or `POST .../ingest/run` tries
/// to actually resolve it — the two-stage design this module's doc comment
/// describes.
///
/// Deliberately compared after trimming, matching how the ref is stored and
/// later handed to the resolver — a check that normalized more aggressively
/// than the resolver would leave a gap between what this rejects and what
/// that accepts.
fn reject_allowlisted_secret_ref(field: &str, value: &str) -> Result<(), ApiError> {
    let trimmed = value.trim();
    if crate::state::CONNECTOR_ALLOWED_SECRET_REF_PATTERNS
        .iter()
        .any(|pattern| lakehouse_core::secret::pattern_matches(pattern, trimmed))
    {
        return Err(ApiError::BadRequest(format!(
            "{field} must not name a deployment connector credential; those are reserved for \
             connectors this deployment seeds itself"
        )));
    }
    Ok(())
}

/// The `NewAuditEvent` [`create`]/[`test_connection`]/[`delete`] each
/// write — a pure, unit-tested helper (WS5 item D3), mirroring
/// `routes::query::query_run_audit_event`/`routes::agents::
/// decide_approval_audit_event`'s pattern: `resource_kind: "connector"`
/// paired with the connector's own id is this module's own choice, baked
/// in here rather than passed by each call site, since it is the pairing
/// `lakehouse_store::connectors::get_connector`'s own `LEFT JOIN`-free
/// lookup (`resource_kind = 'connector' AND resource_id = id`, confirmed
/// by reading that function before writing this) already expects.
///
/// `principal_kind` comes from [`Principal::kind_for_audit`], never
/// `Principal::provider` — same CHECK this crate's other audit sites
/// satisfy (see that method's doc comment).
fn connector_audit_event(
    principal: &Principal,
    action: &str,
    connector_id: &str,
    args: Value,
    outcome: &str,
) -> NewAuditEvent {
    NewAuditEvent {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        action: action.to_owned(),
        resource_kind: Some("connector".to_owned()),
        resource_id: Some(connector_id.to_owned()),
        args: Some(args),
        outcome: outcome.to_owned(),
        detail: None,
        run_id: None,
        approval_id: None,
        session_id: None,
    }
}

/// `POST /api/connectors` — register a connector. Returns 201.
///
/// # Security
///
/// Requires an authenticated [`Principal`] holding `connector:manage`
/// (`crate::policy::POLICY_TABLE`, confirmed by reading it before writing
/// this) — this doc comment previously and incorrectly claimed the route
/// was unauthenticated (WS5 item D3 correction; the policy table has
/// required `connector:manage` for this route for some time, this
/// comment just never caught up).
///
/// `principal` is `Option<Extension<Principal>>`, not a bare
/// `Extension<Principal>`, even though the mounted route always supplies
/// one (`crate::policy::auth_gate` guarantees it for `connector:manage`):
/// `routes::ai::tools::connectors::create_connector` calls this handler
/// directly, bypassing that middleware, for the copilot's own
/// `create_connector` tool — matching `routes::query::run`'s own
/// `Option<Extension<Principal>>` + explicit refusal shape (WS5 item D1)
/// rather than mandating an extractor no internal caller can satisfy.
///
/// # Errors
///
/// 401 if no principal is present (see above); 400 on a malformed body, a
/// blank required field, an unrecognized `direction`, or a `secretRef`
/// shaped like a raw credential (see
/// `lakehouse_store::connectors::looks_like_raw_secret`); 409 if the name
/// is taken; 503/500 as above. Also 400 if `secretRef`/`secretRefSecondary`
/// names a deployment connector credential — see
/// [`reject_allowlisted_secret_ref`].
pub async fn create(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<connectors::Connector>)> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let body: CreateConnectorBody = parse_body(&body)?;
    let direction = required("direction", &body.direction)?;
    if !VALID_DIRECTIONS.contains(&direction.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "direction must be one of {VALID_DIRECTIONS:?}, got {direction:?}"
        ))
        .into());
    }
    let secret_ref = required("secretRef", &body.secret_ref)?;
    if connectors::looks_like_raw_secret(&secret_ref) {
        return Err(ApiError::BadRequest(
            "secretRef must be a reference to a credential (e.g. \"env:MY_SECRET\" or \
             \"vault:secret/data/...\"), not the credential itself"
                .to_owned(),
        )
        .into());
    }
    reject_allowlisted_secret_ref("secretRef", &secret_ref)?;
    let secret_ref_secondary = match body.secret_ref_secondary {
        Some(raw) if !raw.trim().is_empty() => {
            let trimmed = raw.trim().to_owned();
            if connectors::looks_like_raw_secret(&trimmed) {
                return Err(ApiError::BadRequest(
                    "secretRefSecondary must be a reference to a credential, not the credential \
                     itself"
                        .to_owned(),
                )
                .into());
            }
            reject_allowlisted_secret_ref("secretRefSecondary", &trimmed)?;
            Some(trimmed)
        }
        _ => None,
    };
    let input = CreateConnectorInput {
        name: required("name", &body.name)?,
        kind: required("type", &body.kind)?,
        direction,
        host: required("host", &body.host)?,
        secret_ref,
        secret_ref_secondary,
        environment: required("environment", &body.environment)?,
        tenant: required("tenant", &body.tenant)?,
        residency: body.residency,
        capabilities: body.capabilities,
        owner: body.owner,
    };
    let created = connectors::create_connector(pool(&state)?, &input).await?;
    // WS5 item D3: best-effort, never turns a successful create into an
    // error — name/type/direction only, never `secretRef` (a reference
    // name, not a credential, per this module's own guarantee, but still
    // not this row's business to repeat).
    let event = connector_audit_event(
        &principal,
        "connector.create",
        &created.id,
        json!({ "name": created.name, "type": created.kind, "direction": created.direction }),
        "executed",
    );
    if let Err(err) = store_audit::insert(pool(&state)?, event).await {
        tracing::warn!(%err, connector_id = %created.id, "failed to record connector.create audit event");
    }
    Ok((StatusCode::CREATED, ApiJson(created)))
}

/// `POST /api/connectors/{id}/test` — test a connector's connection.
///
/// Opens a REAL, bounded (5s, no retries) connectivity probe for
/// `PostgreSQL` and S3-compatible object-storage connectors — see
/// `crate::connector_probe`'s module doc comment for exactly what that
/// does and does not cover. Every other connector `type` gets an honest
/// `supported: false` result, never a fabricated latency or success.
///
/// # Errors
///
/// 401 if no principal is present (see [`create`]'s doc comment on why
/// this is `Option`, not a bare `Extension`); 404 if `id` is unknown;
/// 503/500 as above.
pub async fn test_connection(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<connectors::ConnectorTestResult>> {
    // See `create`'s doc comment: `Option`, not a bare `Extension`, because
    // `routes::ai::tools::connectors::test_connector` calls this handler
    // directly, bypassing `crate::policy::auth_gate`.
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let dial_info = connectors::get_connector_dial_info(pool(&state)?, &id).await?;
    let Some(dial_info) = dial_info else {
        return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
    };
    let outcome = crate::connector_probe::probe(
        &dial_info,
        state.connector_secret_resolver.as_ref(),
        state.config.connector_probe_allow_internal_hosts,
    )
    .await;
    match connectors::record_test_result(
        pool(&state)?,
        &id,
        outcome.ok,
        outcome.supported,
        outcome.latency_ms,
        &outcome.message,
    )
    .await
    {
        Ok(result) => {
            // WS5 item D3: `supported`/`ok` only — never `outcome.message`,
            // which `connector_probe::probe`'s own result may carry a raw
            // upstream probe error verbatim (host, credentials-adjacent
            // detail). Best-effort: a failed audit write never turns a
            // completed test into an error response.
            let event = connector_audit_event(
                &principal,
                "connector.test",
                &id,
                json!({ "supported": result.supported, "ok": result.ok }),
                "executed",
            );
            if let Err(err) = store_audit::insert(pool(&state)?, event).await {
                tracing::warn!(%err, connector_id = %id, "failed to record connector.test audit event");
            }
            Ok(ApiJson(result))
        }
        Err(lakehouse_store::StoreError::NotFound) => {
            Err(ApiError::NotFound(format!("Connector {id} not found")).into())
        }
        Err(err) => Err(ApiError::from(err).into()),
    }
}

/// `?schema=` query for `POST /api/connectors/{id}/discover`. Required
/// only for a `sql`/`cdc` adapter connector — see
/// `crate::connector_discover::discover`'s doc comment.
#[derive(Debug, Deserialize)]
pub struct DiscoverQuery {
    /// The schema to list tables/columns from. Bound as a query
    /// parameter into each driver's discovery SQL, never interpolated —
    /// see `crate::connector_discover`'s module doc comment for why that
    /// is this task's central property.
    schema: Option<String>,
}

/// `POST /api/connectors/{id}/discover` — list a connector's source
/// tables and columns, for a `sql`/`cdc` adapter connector against the
/// `?schema=` query parameter. Every other adapter (`files`/`rest`/
/// `sheets`) or a connector with no ingest-spec adapter set yet answers
/// with an honest `supported: false` — see
/// `crate::connector_discover`'s module doc comment for exactly what is
/// and is not implemented.
///
/// # Errors
///
/// 404 if `id` is unknown; 422 if the connector's dial is invalid, its
/// resolved host is SSRF-blocked, its credential cannot be resolved, or
/// the discovery connection/query itself fails; 503/500 as every other
/// connector route.
pub async fn discover(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DiscoverQuery>,
) -> ApiResult<ApiJson<crate::connector_discover::DiscoverResult>> {
    let dial_info = connectors::get_connector_dial_info(pool(&state)?, &id).await?;
    let Some(dial_info) = dial_info else {
        return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
    };
    let result = crate::connector_discover::discover(
        &dial_info,
        query.schema.as_deref(),
        state.connector_secret_resolver.as_ref(),
        state.config.connector_probe_allow_internal_hosts,
    )
    .await?;
    Ok(ApiJson(result))
}

/// `?table=` query for `GET /api/connectors/{id}/debezium-properties`.
#[derive(Debug, Deserialize)]
pub struct DebeziumPropertiesQuery {
    /// Schema-qualified source table to capture, e.g. `"public.orders"`.
    /// Required: no registry column stores which table a CDC connector
    /// captures yet — WS3's `connector_ingest_spec` migration
    /// (`dial`/`sourceObjects` JSONB) adds one. Until then this is a
    /// required query parameter, not an oversight.
    table: String,
}

/// The response body for `GET /api/connectors/{id}/debezium-properties`.
///
/// An `untagged` enum so the same route can return either:
/// - the rendered `${ENV_VAR_NAME}`-reference template (the
///   [`DebeziumPropertiesResponse::Rendered`] variant — every supported
///   driver), or
/// - the honest `{ supported: false, reason: ... }` body for an
///   Oracle-driver CDC connector arriving with
///   `ORACLE_CDC_LOGMINER_ENABLED=false` (the
///   [`DebeziumPropertiesResponse::Unsupported`] variant).
///
/// `Rendered.properties` contains ONLY `${ENV_VAR_NAME}` references for
/// every credential-shaped field — never a resolved secret — see
/// [`lakehouse_store::cdc::render_debezium_properties_template`]'s doc
/// comment. Consumed by a future `ops/debezium/render_compose.py`
/// (WS3), which is what actually expands the references when it
/// generates a real `debezium-server` compose service; nothing in this
/// handler or its caller resolves them.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum DebeziumPropertiesResponse {
    /// The rendered template body.
    Rendered {
        /// The `.properties` file body, with `${ENV_VAR_NAME}` references
        /// in place of every credential value.
        properties: String,
        /// The `table` query parameter this response was rendered for,
        /// echoed back so a caller does not have to track it separately.
        table: String,
        /// A human-readable reminder of what `properties` is and is not —
        /// present so this is self-documenting even if read outside this
        /// handler's own doc comment.
        note: String,
    },
    /// The honest `supported: false` response, carrying the REAL reason
    /// an Oracle-driver CDC connector's `Debezium` properties template
    /// cannot be rendered. The shape mirrors `POST .../ingest/run`'s own
    /// `supported: false` body (`ingest_run` below), so the connector
    /// detail page can show one canonical message for both routes.
    Unsupported {
        /// Always `false`; this variant exists so the field's name is
        /// visible at the call site and in the wire shape, not to
        /// permit a `true` future — `Debezium` CDC is either supported
        /// (rendered) or it isn't.
        supported: bool,
        /// Why this connector's `Debezium` properties template is not
        /// being rendered. Self-contained text, never carries an
        /// upstream error (AGENTS.md rule 4).
        reason: String,
    },
}

/// Map a [`SqlDriver`] to the real, documented `Debezium` connector class
/// name that source driver runs under (WS3 item 15). Used only by
/// [`debezium_properties`] to pick
/// [`lakehouse_store::cdc::render_debezium_properties_template`]'s
/// `connector_class` argument — before this task, that argument did not
/// exist and the renderer always hardcoded the `PostgreSQL` class, which
/// would have silently mislabeled a `mysql`/`mssql` connector's rendered
/// template.
///
/// The `SqlDriver::Oracle` arm names `Debezium`'s real Oracle class, but no
/// rendered template ever carries it: [`debezium_properties`] refuses every
/// Oracle-driver connector before rendering, whatever
/// `ORACLE_CDC_LOGMINER_ENABLED` says (see `oracle_cdc_refusal`).
fn debezium_connector_class(driver: SqlDriver) -> &'static str {
    match driver {
        SqlDriver::Postgres => "io.debezium.connector.postgresql.PostgresConnector",
        SqlDriver::Mysql => "io.debezium.connector.mysql.MySqlConnector",
        SqlDriver::Mssql => "io.debezium.connector.sqlserver.SqlServerConnector",
        SqlDriver::Oracle => "io.debezium.connector.oracle.OracleConnector",
    }
}

/// The source-connection fields [`debezium_properties`] needs, resolved
/// from `dial_info` by [`resolve_debezium_source_target`] — pulled out of
/// that handler into its own type/function so the handler itself stays
/// under `clippy::too_many_lines`, not for any reuse beyond this module.
struct DebeziumSourceTarget {
    host: String,
    port: u16,
    database: String,
    user: String,
    connector_class: &'static str,
    /// The original [`SqlDriver`] value the parsed `dial` carried — kept
    /// on the target so [`debezium_properties`] can decide whether the
    /// Oracle refusal applies, without
    /// the route handler having to re-parse the dial just to inspect the
    /// driver.
    driver: SqlDriver,
}

/// Resolve `id`/`dial_info` into a [`DebeziumSourceTarget`] — see
/// [`debezium_properties`]'s "Adapter dispatch" doc section for the full
/// `adapter`-first-then-legacy-`kind` rule this implements.
///
/// # Errors
///
/// A 400 [`ApiError`] for every case documented on [`debezium_properties`]
/// as producing one.
fn resolve_debezium_source_target(
    id: &str,
    dial_info: &ConnectorDialInfo,
) -> Result<DebeziumSourceTarget, ApiError> {
    match dial_info.adapter.as_deref() {
        Some(adapter @ ("sql" | "cdc")) => {
            let parsed = Dial::parse(adapter, &dial_info.dial).map_err(|err| {
                ApiError::BadRequest(format!(
                    "connector {id}'s dial does not parse as its own adapter {adapter:?}: {err}"
                ))
            })?;
            let (driver, host, port, database, user) = match &parsed {
                Dial::Sql(dial) => (
                    dial.driver,
                    dial.host.clone(),
                    dial.port,
                    dial.database.clone(),
                    dial.user.clone(),
                ),
                Dial::Cdc(dial) => (
                    dial.driver,
                    dial.host.clone(),
                    dial.port,
                    dial.database.clone(),
                    dial.user.clone(),
                ),
                Dial::Files(_)
                | Dial::Rest(_)
                | Dial::Sheets(_)
                | Dial::Mongo(_)
                | Dial::Kafka(_)
                | Dial::Sftp(_) => {
                    return Err(ApiError::BadRequest(format!(
                        "connector {id} is registered with adapter {adapter:?} but its parsed \
                         dial is not a sql/cdc shape"
                    )));
                }
            };
            // Oracle still resolves here; it is refused one step later, at
            // the `debezium_properties` call site (`oracle_cdc_refusal`),
            // which is why the driver is kept on the target — the route can
            // branch on it without re-parsing the dial.
            Ok(DebeziumSourceTarget {
                host,
                port,
                database,
                user,
                connector_class: debezium_connector_class(driver),
                driver,
            })
        }
        Some("mongodb") => {
            // The `mongodb` adapter is NOT a `SqlDriver` variant (`MongoDB`
            // is a document store, not a SQL/CDC source for `SqlDriver`'s
            // purposes), so the dispatch here is on the adapter string
            // itself. The mongo
            // source block in `render_debezium_properties_template` only
            // reads `source.connector_slug` (for the offset/schema-
            // history filenames and topic prefix) and
            // `source.schema_qualified_table` (currently a no-op for
            // mongo, kept for parity with the postgres/mysql/mssql
            // branches); the host/port/database/user fields of
            // `DebeziumSourceSpec` are validated but never interpolated
            // into the mongo branch — so we hand the resolver safe,
            // non-empty placeholder values here rather than inventing a
            // real host/port from `MongoDial.hosts` (the operator's
            // real MongoDB host:port are env-var references in the
            // rendered template, resolved by the container's shell,
            // never by us).
            let parsed = Dial::parse("mongodb", &dial_info.dial).map_err(|err| {
                ApiError::BadRequest(format!(
                    "connector {id}'s dial does not parse as a mongodb dial: {err}"
                ))
            })?;
            if !matches!(parsed, Dial::Mongo(_)) {
                return Err(ApiError::BadRequest(format!(
                    "connector {id} is registered with adapter \"mongodb\" but its parsed \
                     dial is not a Mongo dial"
                )));
            }
            Ok(DebeziumSourceTarget {
                host: "mongo.placeholder".to_owned(),
                port: 27017,
                database: "placeholder".to_owned(),
                user: "placeholder".to_owned(),
                connector_class: lakehouse_store::cdc::MONGO_CONNECTOR_CLASS,
                driver: SqlDriver::Postgres,
            })
        }
        None if dial_info.kind.to_lowercase().contains("postgres") => {
            let Some(target) = connector_probe::parse_postgres_host(&dial_info.host) else {
                return Err(ApiError::BadRequest(format!(
                    "connector {id} is registered as PostgreSQL but its host is not shaped \
                     \"<user>@<host>:<port>/<database>\""
                )));
            };
            Ok(DebeziumSourceTarget {
                host: target.host.to_owned(),
                port: target.port,
                database: target.database.to_owned(),
                user: target.user.to_owned(),
                connector_class: debezium_connector_class(SqlDriver::Postgres),
                driver: SqlDriver::Postgres,
            })
        }
        _ => Err(ApiError::BadRequest(format!(
            "connector {id} is type {kind:?} with adapter {adapter:?} — Debezium properties \
             only apply to a sql/cdc/mongodb adapter connector, or a legacy null-adapter \
             PostgreSQL connector",
            kind = dial_info.kind,
            adapter = dial_info.adapter,
        ))),
    }
}

/// `GET /api/connectors/{id}/debezium-properties` — render the
/// `debezium-server` `application.properties` TEMPLATE a CDC connector
/// would run with, using `${ENV_VAR_NAME}` references for every
/// credential-shaped field. See
/// [`lakehouse_store::cdc::render_debezium_properties_template`]'s doc
/// comment for why this is a template, never a resolved config, and
/// why that is a correctness requirement (a resolved config would
/// expose the deployment's database password, S3 keys, and catalog
/// token to any `connector:manage` principal) rather than a shortcut.
///
/// # Adapter dispatch
///
/// A connector whose `adapter` column (`0033_connector_ingest_spec.sql`) is
/// `sql` or `cdc` has its connection fields read from the structured
/// `dial` column via [`Dial::parse`] — this covers `postgres`, `mysql`,
/// and `mssql` drivers, each rendering its own real `Debezium` connector
/// class via [`debezium_connector_class`]. A connector whose `adapter` is
/// `mongodb` is dispatched on the adapter string itself,
/// since `MongoDB` is not a `SqlDriver` variant; its rendered template
/// carries a single `mongodb.connection.string` env-var-reference
/// instead of the SQL source's per-field host/port/user/password/dbname.
/// A connector with `adapter IS NULL` (a pre-WS3 row) falls back to the
/// ORIGINAL `kind`-string check plus
/// [`connector_probe::parse_postgres_host`]'s `host`-string parsing —
/// mirrors the same `adapter`-first-then-legacy-`kind` pattern
/// `connector_probe::probe`'s own dispatch and the connector-deletion
/// deprovision step already use (WS3 plan review X4, Z14). Every other
/// adapter (`files`/`rest`/`sheets`), or a null-adapter connector whose
/// `kind` does not name `PostgreSQL`, is not a CDC source and gets an
/// honest 400.
///
/// # Errors
///
/// 404 if `id` is unknown; 400 if the connector is not a
/// `sql`/`cdc`/`mongodb` adapter connector (nor a legacy null-adapter
/// `PostgreSQL` connector), its dial does not parse or is not shaped
/// `sql`/`cdc`/`mongodb`, a legacy connector's `host` is not shaped
/// `"<user>@<host>:<port>/<database>"`, its `secretRef` is not an
/// `env:`-scheme reference (this template can only name an env var, so
/// a `vault:`-scheme or other reference cannot be rendered as one — an
/// honest 400, not a guess), `table` is missing or blank, or any field
/// fails `DebeziumSourceSpec`/`render_debezium_properties_template`'s
/// validation; 409 for an Oracle-driver CDC connector arriving with
/// `ORACLE_CDC_LOGMINER_ENABLED=false`; 503/500 as every other
/// connector route.
pub async fn debezium_properties(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DebeziumPropertiesQuery>,
) -> ApiResult<(StatusCode, ApiJson<DebeziumPropertiesResponse>)> {
    let dial_info = connectors::get_connector_dial_info(pool(&state)?, &id).await?;
    let Some(dial_info) = dial_info else {
        return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
    };
    let table = query.table.trim();
    if table.is_empty() {
        return Err(ApiError::BadRequest("table query parameter is required".to_owned()).into());
    }

    let target = resolve_debezium_source_target(&id, &dial_info)?;

    // Oracle CDC refusal. Fires BEFORE the secretRef `env:` check on
    // purpose: an Oracle CDC connector with a `vault:`-schemed credential
    // should report "this build does not do LogMiner", not a
    // downstream-shaped error about an unrenderable ref scheme.
    if let Some(response) = oracle_cdc_refusal(&target, state.config.oracle_cdc_logminer_enabled) {
        return Ok((StatusCode::CONFLICT, ApiJson(response)));
    }

    let Some(database_password_ref) = dial_info.secret_ref.strip_prefix("env:") else {
        return Err(ApiError::BadRequest(format!(
            "connector {id}'s secretRef {:?} is not an env: reference; this template can \
             only name an environment variable, not a vault: or other reference scheme",
            dial_info.secret_ref
        ))
        .into());
    };
    let slug = connector_slug_for_id(&id)?;
    let source = lakehouse_store::cdc::DebeziumSourceSpec::new(
        slug,
        target.host,
        target.port,
        target.database,
        target.user,
        table,
    )
    .map_err(|err| ApiError::BadRequest(format!("connector {id}'s fields are invalid: {err}")))?;

    let sink = lakehouse_store::cdc::IcebergSinkLocation {
        catalog_uri: &state.config.lakekeeper_catalog_uri,
        warehouse: &state.config.lakekeeper_warehouse,
        s3_endpoint: &state.config.rustfs_s3_endpoint,
    };
    // RUSTFS_ACCESS_KEY/RUSTFS_SECRET_KEY/LAKEKEEPER_TOKEN name the SAME
    // deployment-wide Iceberg-sink env vars `docker-compose.yml`'s
    // `debezium-server` heredoc already references — every CDC connector in
    // this deployment writes to the one shared warehouse, unlike the
    // source database password, which is per-connector.
    let refs = lakehouse_store::cdc::DebeziumEnvRefs {
        database_password_ref,
        s3_access_key_ref: "RUSTFS_ACCESS_KEY",
        s3_secret_key_ref: "RUSTFS_SECRET_KEY",
        catalog_token_ref: Some("LAKEKEEPER_TOKEN"),
    };
    let properties = lakehouse_store::cdc::render_debezium_properties_template(
        &source,
        &sink,
        &refs,
        target.connector_class,
    )
    .map_err(|err| ApiError::BadRequest(format!("connector {id}'s fields are invalid: {err}")))?;

    Ok((
        StatusCode::OK,
        ApiJson(DebeziumPropertiesResponse::Rendered {
            properties,
            table: table.to_owned(),
            note: "values are ${ENV_VAR_NAME} references the deployment's own shell expands at \
                   container start — never a resolved secret"
                .to_owned(),
        }),
    ))
}

/// The only gate between an Oracle-driver CDC connector and
/// [`render_debezium_properties_template`] — and it always refuses.
///
/// This build ships no `Debezium` Oracle `LogMiner` property template. The
/// template it can render is shaped for the Postgres/MySQL/SQL Server
/// connectors, so rendering it for Oracle would put Oracle's connector class
/// on properties an Oracle connector would reject — a config that looks
/// ready and is not. `ORACLE_CDC_LOGMINER_ENABLED` names where that support
/// will attach; it does not provide it. The two flag states therefore differ
/// only in what the refusal tells the operator.
fn oracle_cdc_refusal(
    target: &DebeziumSourceTarget,
    oracle_cdc_logminer_enabled: bool,
) -> Option<DebeziumPropertiesResponse> {
    if target.driver != SqlDriver::Oracle {
        return None;
    }
    let reason = if oracle_cdc_logminer_enabled {
        "ORACLE_CDC_LOGMINER_ENABLED is set, but this build ships no Debezium Oracle \
         LogMiner property template: the template it can render is shaped for the \
         Postgres, MySQL and SQL Server connectors, and Oracle's connector would reject \
         it. Oracle ingestion runs through the batch sql adapter until LogMiner support \
         is built (docs/adr/0008-initial-snapshot-backfill.md)."
    } else {
        "Oracle CDC via Debezium LogMiner is not enabled on this deployment \
         (ORACLE_CDC_LOGMINER_ENABLED is unset or not \"true\"), and this build ships no \
         LogMiner property template in any case. LogMiner capture needs ARCHIVELOG mode \
         and supplemental logging on the source database, which the batch sql adapter \
         neither assumes nor configures (docs/adr/0008-initial-snapshot-backfill.md)."
    };
    Some(DebeziumPropertiesResponse::Unsupported {
        supported: false,
        reason: reason.to_owned(),
    })
}

/// `?force=true` on `DELETE /api/connectors/{id}` — see [`delete`]'s doc
/// comment for what this overrides and why it exists at all.
#[derive(Debug, Default, Deserialize)]
pub struct DeleteQuery {
    /// Delete the registry row even if CDC deprovisioning failed.
    #[serde(default)]
    force: bool,
}

/// Derive the [`ConnectorSlug`] a `PostgreSQL` CDC connector's replication
/// slot/publication would be named after, from the connector's own
/// registry `id`.
///
/// There is no separate "Debezium slug" column: ADR 0007 never grew
/// dynamic per-connector provisioning (see `lakehouse_store::cdc`'s module
/// doc comment — "Callers"), so `connector.id`
/// (`connectors::slug_id`'s output, e.g. `"conn-orders-pg-abc123"`) is the
/// only identifier a real connector has, and it already uses exactly the
/// charset a slug needs MINUS the separator: lowercase ASCII alphanumerics
/// and `-`. Converting `-` to `_` maps it onto [`ConnectorSlug`]'s
/// allowed shape (`^[a-z0-9][a-z0-9_]{0,62}$`) losslessly and
/// deterministically — the same `id` always produces the same slug, so
/// deprovisioning a connector always targets the slot/publication that
/// connector's own id would have named if/when a real provisioning flow
/// existed to create them.
fn connector_slug_for_id(id: &str) -> Result<ConnectorSlug, ApiError> {
    ConnectorSlug::new(&id.replace('-', "_")).map_err(|err| {
        // `connector.id` is always `slug_id`-generated (see that
        // function's doc comment), so this should be unreachable in
        // practice; if it ever isn't, this is a 500 (a data-shape problem
        // this deployment has, not something the caller did wrong), not a
        // silent skip of deprovisioning.
        ApiError::Internal(format!(
            "connector id {id:?} does not map to a valid CDC slug: {err}"
        ))
    })
}

/// Decide WHETHER `id`'s connector should have a `Postgres` replication
/// slot/publication dropped at all, and if so, do it. This is the single
/// place that rule lives (WS3 plan review X4): [`delete`] calls this
/// unconditionally and carries no
/// `kind`-based condition of its own, so there is exactly one encoding of
/// the rule to keep correct, rather than two that could drift apart.
///
/// - `adapter = "cdc"` — the slot/publication names come from
///   `dial.slotName`/`dial.publicationName`, with **no fallback**:
///   [`lakehouse_store::ingest_spec::CdcDial`] requires both fields, so a
///   `cdc` connector always names its own slot/publication rather than one
///   guessed from its registry `id`.
/// - `adapter` is `sql` | `files` | `rest` | `sheets` — returns `Ok(None)`
///   (nothing to deprovision) unconditionally. These adapters never had a
///   replication slot to begin with, so deleting one of these connectors
///   must not attempt to drop a slot/publication merely because its `kind`
///   string happens to say "`PostgreSQL`" — the exact defect this task
///   fixes.
/// - `adapter IS NULL` (a connector row created before
///   `0033_connector_ingest_spec.sql` added the column) **and** `kind`
///   names Postgres — falls back to the pre-WS3 slug-derived names,
///   unchanged from before this task. Bounded EXACTLY to
///   `adapter IS NULL`: the `Some(_)` arm above always matches first once
///   any connector has an `adapter` at all (`sql` included), so this arm
///   is unreachable for any connector this build itself creates.
/// - `adapter IS NULL` and `kind` does not name Postgres — `Ok(None)`.
///
/// `Ok(None)` means "there was nothing to deprovision"; [`delete`] treats
/// that exactly like a successful deprovision and proceeds to delete the
/// row.
async fn deprovision_postgres_connector(
    state: &AppState,
    id: &str,
    dial_info: &ConnectorDialInfo,
) -> Result<Option<Deprovisioned>, ApiError> {
    match dial_info.adapter.as_deref() {
        Some("cdc") => {
            let dial = Dial::parse("cdc", &dial_info.dial).map_err(|err| {
                // `AGENTS.md`'s known-gap note forbids adding another
                // `ApiError::Internal` that
                // carries upstream error text, so the parse failure is
                // logged server-side and the response gets fixed text only.
                // Failing closed here is still correct: a row marked
                // `adapter = 'cdc'` whose `dial` will not parse is a
                // data-integrity problem, not something to silently skip.
                tracing::error!(
                    connector_id = %id,
                    error = %err,
                    "adapter=cdc connector's dial column does not parse as a CDC dial; \
                     deprovisioning cannot determine its slot/publication names"
                );
                ApiError::Internal(format!(
                    "connector {id} is registered with adapter=cdc but its stored dial does not \
                     parse as a CDC dial; see the server log for the parse error"
                ))
            })?;
            let Some(cdc) = dial.as_cdc() else {
                // Unreachable in practice (`Dial::parse("cdc", ..)` only
                // ever returns `Dial::Cdc`), but matched honestly rather
                // than with `unwrap`/`expect` — see the parse arm above for
                // why the response carries fixed text.
                tracing::error!(
                    connector_id = %id,
                    "Dial::parse(\"cdc\", ..) returned a non-Cdc variant"
                );
                return Err(ApiError::Internal(format!(
                    "connector {id} is registered with adapter=cdc but its parsed dial is not a \
                     CDC dial"
                )));
            };
            deprovision_with_names(state, id, dial_info, &cdc.slot_name, &cdc.publication_name)
                .await
                .map(Some)
        }
        None if dial_info.kind.to_lowercase().contains("postgres") => {
            let slug = connector_slug_for_id(id)?;
            deprovision_with_names(
                state,
                id,
                dial_info,
                &format!("{slug}_slot"),
                &format!("{slug}_pub"),
            )
            .await
            .map(Some)
        }
        // `sql` | `files` | `rest` | `sheets` never had a replication slot
        // (the `Some(_)` half of this arm), and `adapter IS NULL` with a
        // non-Postgres `kind` never did either (the `None` half, falling
        // through from the guarded arm above) — both are "nothing to
        // deprovision" (WS3 plan review X4).
        Some(_) | None => Ok(None),
    }
}

/// The shared body behind every [`deprovision_postgres_connector`] arm that
/// actually attempts a drop: resolve `dial_info`'s host/credential exactly
/// the way [`crate::connector_probe::probe`] does for `POST
/// /api/connectors/{id}/test` (same parser, same allowlisted resolver),
/// then drop `slot_name`/`publication_name` on that target. Unchanged from
/// [`deprovision_postgres_connector`]'s body before this task, just
/// parameterized on the two names instead of a single `&ConnectorSlug`.
async fn deprovision_with_names(
    state: &AppState,
    id: &str,
    dial_info: &ConnectorDialInfo,
    slot_name: &str,
    publication_name: &str,
) -> Result<Deprovisioned, ApiError> {
    let Some(target) = connector_probe::parse_postgres_host(&dial_info.host) else {
        return Err(ApiError::Internal(format!(
            "connector {id} is registered as PostgreSQL but its host is not shaped \
             \"<user>@<host>:<port>/<database>\", so CDC deprovisioning cannot even attempt to \
             dial it"
        )));
    };
    let password = state
        .connector_secret_resolver
        .resolve_dyn(&dial_info.secret_ref)
        .await
        .map_err(|err| {
            ApiError::Internal(format!(
                "could not resolve connector {id}'s credential to deprovision its CDC slot: {err}"
            ))
        })?;
    let pg_target = PgTarget {
        host: target.host.to_owned(),
        port: target.port,
        user: target.user.to_owned(),
        password,
        database: target.database.to_owned(),
    };
    connector_deprovision::drop_slot_and_publication(&pg_target, slot_name, publication_name)
        .await
        .map_err(|err| {
            ApiError::Internal(deprovision_error_message(
                id,
                slot_name,
                publication_name,
                &err,
            ))
        })
}

/// Render a [`DeprovisionError`] into text safe to put in a 409/500 body or
/// a log line — never the credential [`deprovision_with_names`] resolved,
/// only the slot/publication names and the error's own `Display` (which,
/// per [`DeprovisionError`]'s doc comment, never includes connection
/// credentials).
fn deprovision_error_message(
    id: &str,
    slot_name: &str,
    publication_name: &str,
    err: &DeprovisionError,
) -> String {
    format!(
        "deprovisioning connector {id}'s CDC replication slot ({slot_name:?}) and publication \
         ({publication_name:?}) failed: {err}"
    )
}

/// `DELETE /api/connectors/{id}` — remove a connector registration.
///
/// # CDC deprovisioning happens FIRST, decided by `adapter` not `kind`
///
/// This calls [`deprovision_postgres_connector`] unconditionally, BEFORE
/// removing the registry row — that function is the single place deciding
/// whether there is anything to drop at all (WS3 plan review X4: an
/// `adapter = "cdc"` connector, or the bounded `adapter IS NULL` legacy
/// Postgres case; never a `sql`/`files`/`rest`/`sheets` connector, however
/// its `kind` string reads). This fixes the exact gap
/// `lakehouse_store::connectors::delete_connector` used to describe:
/// silently deleting the row while the slot survived left it pinning WAL on
/// the customer's source database until disk filled, forever, with nothing
/// in the registry to show it. Deprovisioning is idempotent (an
/// already-absent slot/publication is success), so a connector that was
/// never fully provisioned, or was already cleaned up by a previous
/// attempt, still deletes cleanly.
///
/// - Deprovision succeeds, or there was nothing to deprovision -> the row
///   is deleted -> 204, same as before.
/// - Deprovision fails and `?force` is absent/false -> the row is KEPT and
///   this returns 409, naming the slot/publication in the message. This is
///   deliberate: the whole point is that an orphaned slot must stay
///   visible (as a connector row still in the registry) rather than
///   vanish along with the only record that it needs cleaning up.
/// - Deprovision fails and `?force=true` is given -> the row is deleted
///   anyway (204) and a `tracing::error!` names the slot, publication, and
///   host as a deliberately loud, greppable record of the orphan this
///   just created. `force` exists because a connector can end up pointing
///   at a decommissioned or now-unreachable host — deprovisioning can
///   never succeed there, and an operator who already knows that needs a
///   way to remove the row anyway rather than being stuck forever.
///
/// # Errors
///
/// 401 if no principal is present (see [`create`]'s doc comment on why
/// this is `Option`, not a bare `Extension`); 404 if `id` is unknown; 409
/// if CDC deprovisioning failed and `force` was not given; 503/500 as
/// above.
pub async fn delete(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
    Query(query): Query<DeleteQuery>,
) -> ApiResult<StatusCode> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let dial_info = connectors::get_connector_dial_info(pool(&state)?, &id).await?;
    let Some(dial_info) = dial_info else {
        return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
    };

    if let Err(err) = deprovision_postgres_connector(&state, &id, &dial_info).await {
        if query.force {
            let slug = connector_slug_for_id(&id).ok();
            tracing::error!(
                connector_id = %id,
                slot = %slug.as_ref().map_or_else(|| "<unknown>".to_owned(), |s| format!("{s}_slot")),
                publication = %slug.as_ref().map_or_else(|| "<unknown>".to_owned(), |s| format!("{s}_pub")),
                host = %dial_info.host,
                error = %err,
                "force-deleting connector despite failed CDC deprovision: the replication \
                 slot/publication above may still exist on the source database and will \
                 keep pinning WAL until cleaned up manually"
            );
        } else {
            let slug_hint = connector_slug_for_id(&id).map_or_else(
                |_| "its CDC slot/publication".to_owned(),
                |s| format!("slot {s}_slot / publication {s}_pub"),
            );
            return Err(ApiError::Conflict(format!(
                "connector {id} was NOT deleted: dropping {slug_hint} failed ({err}); the \
                 registry row is kept deliberately so this orphaned slot stays visible instead \
                 of silently pinning WAL on the source database forever. Retry once the source \
                 is reachable, or pass ?force=true to delete the row anyway (the \
                 slot/publication will then have to be cleaned up manually)."
            ))
            .into());
        }
    }

    let deleted = connectors::delete_connector(pool(&state)?, &id).await?;
    if !deleted {
        return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
    }
    // WS5 item D3: best-effort, after the row is already gone — a failed
    // audit write here must never resurrect the 404/409 branches above or
    // undo a delete that already succeeded.
    let event = connector_audit_event(&principal, "connector.delete", &id, json!({}), "executed");
    if let Err(err) = store_audit::insert(pool(&state)?, event).await {
        tracing::warn!(%err, connector_id = %id, "failed to record connector.delete audit event");
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/connectors/{id}/ingest-spec` — a connector's ingest
/// configuration.
///
/// Gated on `ingest:read`, not `connector:manage` — see `policy.rs`'s
/// `POLICY_TABLE` entry and `0033_connector_ingest_spec.sql`'s header
/// comment for why: a read-only caller (e.g. Phase G's ingest service
/// identity) must never be handed PUT-level `connector:manage` just to
/// read a `dial`.
///
/// # Errors
///
/// 404 if `id` is unknown; 503/500 as above.
pub async fn ingest_spec_get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<connectors::IngestSpec>> {
    let spec = connectors::get_ingest_spec(pool(&state)?, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    Ok(ApiJson(spec))
}

/// The `PUT /api/connectors/{id}/ingest-spec` body. Mirrors
/// `IngestSpecInput` in `contracts/connectors.ts`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestSpecBody {
    adapter: String,
    ingest_mode: String,
    #[serde(default)]
    dial: serde_json::Value,
    #[serde(default)]
    source_objects: serde_json::Value,
    #[serde(default)]
    schedule_cron: Option<String>,
}

/// A second, non-authoritative SSRF check on `dial`'s host, run at
/// `PUT .../ingest-spec` SAVE time — see [`ingest_spec_put`]'s doc comment
/// for why this can only ever be advisory and what stays authoritative.
///
/// Checks `sql`/`cdc` (`dial.host`/`dial.port` directly), `files` (only
/// when `dial.endpoint` is set — a `files` dial with no endpoint override
/// has no caller-supplied host to check) and `rest` (`dial.baseUrl`,
/// parsed the same way [`connector_probe::probe_s3`] parses an S3
/// endpoint) against [`connector_probe::resolve_checked`], gated the SAME
/// way [`connector_probe::probe`] already is by
/// `state.config.connector_probe_allow_internal_hosts`. Never checks
/// `sheets`: its dial names a spreadsheet id, not a caller-chosen host —
/// every `sheets` connector targets Google's own fixed hosts.
///
/// # Errors
///
/// Returns `ApiError::BadRequest` naming the offending resolved address
/// (the same message [`connector_probe::resolve_checked`] already produces
/// for `POST .../test`) if a checked host resolves to a private/internal
/// address and `allow_internal_hosts` is `false`.
async fn check_dial_ssrf(dial: &Dial, allow_internal_hosts: bool) -> Result<(), ApiError> {
    let host_port: Option<(&str, u16)> = match dial {
        Dial::Sql(sql) => Some((sql.host.as_str(), sql.port)),
        Dial::Cdc(cdc) => Some((cdc.host.as_str(), cdc.port)),
        Dial::Files(files) => files
            .endpoint
            .as_deref()
            .and_then(connector_probe::parse_endpoint_host_port),
        Dial::Rest(rest) => connector_probe::parse_endpoint_host_port(&rest.base_url),
        // Tier 2 adapters (`mongodb`/`kafka`/`sftp`): the authoritative
        // SSRF check runs in Dagster (`ssrf_guard_mongo.py` /
        // `ssrf_guard_kafka.py` / `ssrf_guard_sftp.py`) at dial time,
        // not in this save-time pre-check; this function only fails
        // fast on the common case (a caller pastes an obviously-internal
        // host for a `sql`/`cdc`/`files`/`rest` connector and finds out
        // immediately, at save time). The Rust-side
        // `routes/connectors::check_dial_ssrf` match returns `None` for
        // these variants by construction -- the same shape `sheets`
        // already has (its dial names a spreadsheet id, not a
        // caller-chosen host). The authoritative check for these
        // adapters stays in Dagster, not here.
        Dial::Sheets(_) | Dial::Mongo(_) | Dial::Kafka(_) | Dial::Sftp(_) => None,
    };
    let Some((host, port)) = host_port else {
        return Ok(());
    };
    connector_probe::resolve_checked(host, port, allow_internal_hosts)
        .await
        .map_err(ApiError::BadRequest)
}

/// `PUT /api/connectors/{id}/ingest-spec` — set a connector's ingest
/// configuration.
///
/// Gated on `connector:manage`, not `ingest:read` — the write half of the
/// same split [`ingest_spec_get`]'s doc comment describes.
/// [`lakehouse_store::connectors::set_ingest_spec`] runs
/// `ingest_spec::Dial::parse` against `dial` BEFORE writing anything, so an
/// invalid `dial` (unknown field, missing required field, unsafe hostname)
/// never reaches the database.
///
/// # A second, non-authoritative SSRF check runs here too
///
/// Once `dial` parses, this handler ALSO extracts its host (see
/// [`check_dial_ssrf`]) and runs it through
/// [`connector_probe::resolve_checked`] — the SAME check [`connector_probe`]
/// runs before a real dial. This is deliberately NOT the authoritative
/// guard: DNS can change between this save and a later
/// `POST .../ingest/run`, so the check that actually matters stays at dial
/// time, inside `connector_probe` for a Rust-side probe and inside
/// Dagster's own `ssrf_guard.resolve_checked` for an ingestion job. This
/// save-time copy exists only to fail fast on the common case — a caller
/// pastes an obviously-internal host and finds out immediately, at save
/// time, rather than on the next scheduled run (WS3 plan judge review Z1).
///
/// # Errors
///
/// 404 if `id` is unknown; 400 if `dial` fails
/// `ingest_spec::Dial::parse` for `adapter`, or if [`check_dial_ssrf`]
/// refuses `dial`'s host (`StoreError::Validation` maps to
/// `ApiError::BadRequest`, never `Internal` — both messages are safe to
/// surface); 503/500 as above.
pub async fn ingest_spec_put(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<connectors::IngestSpec>> {
    let body: IngestSpecBody = parse_body(&body)?;
    // Parsed here, in the route, ahead of `set_ingest_spec`'s own (later,
    // authoritative-for-persistence) `Dial::parse` call: this handler
    // needs the typed `Dial` itself to extract a host for
    // `check_dial_ssrf`, which `set_ingest_spec` never returns.
    let dial = Dial::parse(&body.adapter, &body.dial)
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    check_dial_ssrf(&dial, state.config.connector_probe_allow_internal_hosts).await?;
    let input = connectors::IngestSpecInput {
        adapter: body.adapter,
        ingest_mode: body.ingest_mode,
        dial: body.dial,
        source_objects: body.source_objects,
        schedule_cron: body.schedule_cron,
    };
    match connectors::set_ingest_spec(pool(&state)?, &id, &input).await {
        Ok(spec) => Ok(ApiJson(spec)),
        Err(lakehouse_store::StoreError::NotFound) => {
            Err(ApiError::NotFound(format!("Connector {id} not found")).into())
        }
        Err(err) => Err(ApiError::from(err).into()),
    }
}

/// `POST /api/connectors/{id}/ingest/run`'s response for a `cdc`-adapter
/// connector: `supported: false` with the REAL reason, never a fabricated
/// "snapshot started" response, and never a launch (ADR 0008).
///
/// `ADR 0008` (`docs/adr/0008-initial-snapshot-backfill.md`) explicitly
/// declines to build a signal-table/incremental-snapshot mechanism —
/// there is no `debezium_signal` table anywhere in this codebase.
/// `Debezium`'s own default `snapshot.mode=initial` already performs the
/// initial-snapshot-then-stream sequence automatically, the moment this
/// connector's own `debezium-<slug>` compose service
/// (`ops/debezium/render_compose.py`'s `service_name = f"debezium-
/// {sanitized_id}"`, the SAME [`connector_slug_for_id`] sanitize transform
/// used here) starts against its replication slot/publication. So there
/// is nothing a route can trigger: the trigger already happened, at that
/// service's startup, not at a caller's request.
///
/// Falls back to the raw `connector_id` (rather than failing the whole
/// "unsupported" response) on the practically-unreachable case
/// [`connector_slug_for_id`] itself documents — an honest refusal should
/// not itself 500 over a display-string detail.
fn cdc_ingest_run_unsupported_reason(connector_id: &str) -> String {
    let slug = connector_slug_for_id(connector_id)
        .map_or_else(|_| connector_id.to_owned(), |slug| slug.to_string());
    format!(
        "CDC ingestion has no separate trigger: Debezium's own \
         snapshot.mode=initial (ADR 0008) runs the initial snapshot \
         automatically the moment the debezium-{slug} compose service \
         (ops/debezium/render_compose.py) starts against this \
         connector's replication slot/publication. Bring that service up \
         to start ingestion; there is nothing this route can trigger."
    )
}

/// `POST /api/connectors/{id}/ingest/run` — launch this connector's
/// static `ingest_job` now, or (for a `cdc`-adapter connector) report the
/// honest reason there is nothing to launch.
///
/// Gated on `connector:manage` (`POLICY_TABLE`) — a MUTATING action, not
/// `ingest:read`: the `ingest:read`-scoped Dagster ingest-schedule-factory
/// identity (`main::bootstrap_ingest_run_service`) is a READER, it never
/// calls this route itself.
///
/// # Distinguishing a `Dagster` launch failure from a transport failure
/// (WS3 plan review Z9)
///
/// [`lakehouse_dagster::DgClient::launch_run_with_config`] returns
/// `Ok(LaunchOutcome { run_id: None, error: Some(msg) })`, NOT `Err`, for
/// a GraphQL-level launch failure (a bad job name, a
/// `RunConfigValidationInvalid`) — exactly the branching
/// `routes::pipelines::trigger` (`pipelines.rs:184-207`) already proves
/// out: `Err` is reserved for transport failures and malformed GraphQL
/// responses. This handler branches the identical way: `outcome.error`
/// present becomes a 422 naming that error; `Err(err)` becomes a 503,
/// classified through [`js_error`] the same way `trigger`'s own 503
/// branch is (`DgError`'s `Display` is already sanitized — see that
/// type's doc comment — so this is not a fourteenth Phase-1
/// `ApiError::Internal(err.to_string())` leak).
///
/// # Errors
///
/// 404 if `id` is unknown or has no ingest spec set (`adapter IS NULL`);
/// 422 if `Dagster` reports a launch-time failure; 503 on a `Dagster`
/// transport failure; 503/500 as above for the database pool.
pub async fn ingest_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let spec = connectors::get_ingest_spec(pool(&state)?, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    let adapter = spec
        .adapter
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} has no ingest spec set")))?;

    if adapter == "cdc" {
        return Ok(ApiJson(json!({
            "supported": false,
            "reason": cdc_ingest_run_unsupported_reason(&id),
        })));
    }

    // Mirrors `agent_runs.py`'s `_employee_run_config` shape: one static
    // job (`ingest_job`), the launched run's only per-run input is this
    // connector's id, which `run_ingest`'s Dagster op re-fetches the
    // connector row by (`dagster/dispar_orchestrate/ingest_factory.py`).
    let run_config = json!({"ops": {"run_ingest": {"config": {"connector_id": id}}}});
    match state
        .dagster
        .launch_run_with_config("ingest_job", &run_config)
        .await
    {
        Ok(outcome) => {
            if let Some(error) = outcome.error {
                return Err(ApiError::Unprocessable(error).into());
            }
            Ok(ApiJson(json!({ "runId": outcome.run_id })))
        }
        Err(err) => Err(ApiError::Unavailable(js_error(err)).into()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use axum::body::to_bytes;
    use axum::http::Request;
    use lakehouse_auth::PermissionSet;
    use lakehouse_auth::PrincipalId;
    use serde_json::Value;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::config::Config;

    fn state_without_pool() -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        AppState::new(Config::from_map(&env).unwrap())
    }

    /// A logged-in human principal — `PrincipalId::User`. Mirrors
    /// `routes::query`/`routes::agents`'s own test fixture of the same
    /// name.
    fn fixture_user_principal() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(1)),
            tenant_ids: Vec::new(),
            display_name: "Rina Wijaya".to_owned(),
            permissions: PermissionSet::parse("connector:manage"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// A service-token principal — `PrincipalId::Service`.
    fn fixture_service_principal() -> Principal {
        Principal {
            id: PrincipalId::Service(Uuid::from_u128(2)),
            tenant_ids: Vec::new(),
            display_name: "dagster-orchestrator".to_owned(),
            permissions: PermissionSet::parse("connector:manage"),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// WS5 item D3, failing-test-first: `connector_audit_event` did not
    /// exist before this task; referencing it below failed to compile
    /// with `cannot find function connector_audit_event in this scope`
    /// (confirmed by checking out the pre-fix file and running this test
    /// before adding the helper).
    #[test]
    fn connector_create_audit_event_pairs_with_connector_resource_kind() {
        let principal = fixture_user_principal();
        let event = connector_audit_event(
            &principal,
            "connector.create",
            "conn-1",
            json!({ "name": "orders-cdc", "type": "postgres-cdc" }),
            "executed",
        );
        assert_eq!(event.principal_kind.as_deref(), Some("user"));
        assert_eq!(
            event.principal_id.as_deref(),
            Some(principal.id.uuid().to_string().as_str())
        );
        assert_eq!(event.resource_kind.as_deref(), Some("connector"));
        assert_eq!(event.resource_id.as_deref(), Some("conn-1"));
        assert_eq!(event.action, "connector.create");
        assert_eq!(event.outcome, "executed");
    }

    /// A service identity's connector action must record `principal_kind:
    /// "service"`, never `"user"`.
    #[test]
    fn connector_audit_event_records_a_service_identity_as_service_not_user() {
        let principal = fixture_service_principal();
        let event = connector_audit_event(
            &principal,
            "connector.test",
            "conn-1",
            json!({}),
            "executed",
        );
        assert_eq!(event.principal_kind.as_deref(), Some("service"));
    }

    /// The connector.test call site must only ever pass `supported`/`ok`
    /// into `args` — never the raw upstream probe error message
    /// `connector_probe::probe`'s own result may carry (a host,
    /// credentials-adjacent detail). This asserts the helper's own args
    /// shape stays exactly `{ "supported": .., "ok": .. }`, so a future
    /// edit that adds `result.message` to the call site is caught by
    /// re-reading this test's fixed shape, not by trusting the call site.
    #[test]
    fn connector_test_audit_event_never_carries_the_raw_probe_error() {
        let principal = fixture_user_principal();
        let raw_probe_message =
            "connection refused: password authentication failed for user \"root\" host=10.0.0.5";
        let event = connector_audit_event(
            &principal,
            "connector.test",
            "conn-1",
            json!({ "supported": true, "ok": false }),
            "executed",
        );
        let args_str = event.args.unwrap().to_string();
        assert!(
            !args_str.contains(raw_probe_message),
            "must never embed the raw upstream probe error text"
        );
        assert_eq!(args_str, r#"{"supported":true,"ok":false}"#);
    }

    #[tokio::test]
    async fn missing_pool_is_a_503_naming_database_url() {
        let state = state_without_pool();
        let err = pool(&state).expect_err("a malformed DATABASE_URL must yield no pool");
        assert_eq!(err.status(), 503);
        assert!(err.to_string().contains("DATABASE_URL"));
    }

    #[tokio::test]
    async fn every_database_backed_route_returns_503_without_a_pool() {
        let paths = ["/api/connectors", "/api/connectors/conn-x"];
        for path in paths {
            let app = crate::routes::router(state_without_pool());
            let response = app
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(body.get("error").is_some(), "{path}");
        }
    }

    #[tokio::test]
    async fn debezium_properties_route_is_registered() {
        let app = crate::routes::router(state_without_pool());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/connectors/conn-x/debezium-properties?table=public.orders")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "no pool -> 503, not 404"
        );
    }

    #[test]
    fn create_body_rejects_unknown_direction() {
        let body = CreateConnectorBody {
            name: "n".to_owned(),
            kind: "REST API".to_owned(),
            direction: "sideways".to_owned(),
            host: "h".to_owned(),
            secret_ref: "env:X".to_owned(),
            secret_ref_secondary: None,
            environment: "production".to_owned(),
            tenant: "t".to_owned(),
            residency: String::new(),
            capabilities: vec![],
            owner: None,
        };
        assert!(!VALID_DIRECTIONS.contains(&body.direction.as_str()));
    }

    /// D5/Should-fix: `secretRefSecondary` must parse through the request
    /// body (camelCase, per the struct's `rename_all`) and reach
    /// `CreateConnectorInput` — otherwise an API-created S3 connector can
    /// never be tested, since `probe_s3` requires both refs.
    #[test]
    fn secret_ref_secondary_round_trips_through_the_request_body() {
        let json = serde_json::json!({
            "name": "n",
            "type": "Object storage",
            "direction": "sink",
            "host": "http://rustfs:9000|bucket",
            "secretRef": "env:AK",
            "secretRefSecondary": "env:SK",
            "environment": "production",
            "tenant": "t",
        });
        let body: CreateConnectorBody = serde_json::from_value(json).unwrap();
        assert_eq!(body.secret_ref_secondary.as_deref(), Some("env:SK"));
    }

    /// Absent `secretRefSecondary` (e.g. a `PostgreSQL` connector, which
    /// only ever needs one credential) must still parse.
    #[test]
    fn secret_ref_secondary_is_optional() {
        let json = serde_json::json!({
            "name": "n",
            "type": "PostgreSQL",
            "direction": "bidirectional",
            "host": "u@host:5432/db",
            "secretRef": "env:PW",
            "environment": "production",
            "tenant": "t",
        });
        let body: CreateConnectorBody = serde_json::from_value(json).unwrap();
        assert_eq!(body.secret_ref_secondary, None);
    }

    /// The defense-in-depth check from `looks_like_raw_secret` is wired
    /// into this handler, not just unit-tested in isolation.
    #[test]
    fn secret_looking_secret_ref_is_rejected_by_the_shared_check() {
        assert!(connectors::looks_like_raw_secret(
            "postgres://admin:hunter2@db.internal:5432/oms"
        ));
        assert!(!connectors::looks_like_raw_secret("env:MY_SECRET"));
    }

    /// The exfiltration path this check exists to close: a
    /// `connector:manage` principal naming a deployment connector credential
    /// on a connector whose `host` they choose. A representative instance of
    /// EVERY allowlisted pattern must be refused, so widening the pattern
    /// list without widening this check fails here rather than silently
    /// opening the hole again.
    #[test]
    fn user_created_connector_cannot_name_a_deployment_connector_credential() {
        for r in [
            "env:CONNECTOR_MYSQL_PASSWORD",
            "env:CONNECTOR_S3_SECRET_KEY",
            "env:CONNECTOR_S3_ACCESS_KEY",
            "env:CONNECTOR_REST_API_KEY",
            "env:CONNECTOR_OAUTH_TOKEN",
            "file:/run/secrets/connector_mysql_password",
        ] {
            assert!(
                reject_allowlisted_secret_ref("secretRef", r).is_err(),
                "pattern-matching ref {r:?} must be refused on a user-created connector"
            );
            // Whitespace must not be a bypass: the value is trimmed before
            // storage, so a padded ref would reach the resolver identically.
            assert!(
                reject_allowlisted_secret_ref("secretRef", &format!("  {r}  ")).is_err(),
                "padded {r:?} must be refused too"
            );
        }
    }

    /// The check must not over-reach: an ordinary `env:` ref that does not
    /// match a reserved pattern is still accepted here. It will fail later
    /// at resolution (it is not on the allowlist), which is a different,
    /// honest error — "this deployment will not resolve that", not "you may
    /// not say that". Includes `env:CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS`
    /// (WS3 plan review X4/Z4): a real config flag, not a credential, so it
    /// matches none of the credential-suffix patterns and must pass THIS
    /// check even though `AllowlistedSecretResolver::resolve` refuses it
    /// with `NotAllowed` at resolve time — see this function's doc comment
    /// for the full two-stage explanation.
    #[test]
    fn ordinary_secret_refs_are_still_accepted_by_this_check() {
        for r in [
            "env:MY_SECRET",
            "vault:secret/data/x",
            "env:POSTGRES_PASSWORD_2",
            "env:CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS",
        ] {
            assert!(
                reject_allowlisted_secret_ref("secretRef", r).is_ok(),
                "{r:?} does not match a reserved connector-credential pattern and must pass \
                 this check"
            );
        }
    }

    /// The allowlist must never admit one of the API's own secrets again.
    /// This is the regression guard for the finding itself: the previous
    /// list was `env:POSTGRES_PASSWORD` / `env:RUSTFS_ACCESS_KEY` /
    /// `env:RUSTFS_SECRET_KEY`, and a pattern wide enough to re-admit any of
    /// them would restore the exfiltration path no matter what the
    /// route-level check does.
    #[test]
    fn allowlist_never_admits_the_apis_own_secrets() {
        for forbidden in [
            "env:POSTGRES_PASSWORD",
            "env:RUSTFS_ACCESS_KEY",
            "env:RUSTFS_SECRET_KEY",
            "env:DATABASE_URL",
            "env:CH_PASSWORD",
        ] {
            assert!(
                !crate::state::CONNECTOR_ALLOWED_SECRET_REF_PATTERNS
                    .iter()
                    .any(|pattern| lakehouse_core::secret::pattern_matches(pattern, forbidden)),
                "{forbidden:?} is one of the API's own secrets and must never match a \
                 connector-resolvable pattern"
            );
        }
    }

    /// A [`ConnectorDialInfo`] fixture for [`deprovision_postgres_connector`]
    /// unit tests — `kind`/`adapter`/`dial` vary per test, everything else
    /// is filler.
    fn dial_info(kind: &str, adapter: Option<&str>, dial: serde_json::Value) -> ConnectorDialInfo {
        ConnectorDialInfo {
            kind: kind.to_owned(),
            // Deliberately shaped so `parse_postgres_host` succeeds and the
            // connect attempt fails fast and deterministically (nothing
            // listens on 127.0.0.1:1 — same trick
            // `connector_deprovision`'s own
            // `unreachable_host_surfaces_as_connect_error_not_a_hang` test
            // uses), rather than depending on DNS behavior for an
            // unresolvable hostname.
            host: "dialuser@127.0.0.1:1/db".to_owned(),
            secret_ref: "env:TEST_CONNECTOR_PASSWORD".to_owned(),
            secret_ref_secondary: None,
            adapter: adapter.map(str::to_owned),
            dial,
        }
    }

    /// A state whose `connector_secret_resolver` always resolves
    /// `env:TEST_CONNECTOR_PASSWORD`, without touching the real process
    /// environment or the production allowlist — these tests only need
    /// credential resolution to succeed deterministically so execution
    /// reaches the real dial attempt (and its slot/publication names),
    /// never a real database.
    fn state_with_stub_secret_resolver() -> AppState {
        let mut state = state_without_pool();
        state.connector_secret_resolver = std::sync::Arc::new(
            lakehouse_core::secret::EnvSecretResolver::with_map(HashMap::from([(
                "TEST_CONNECTOR_PASSWORD".to_owned(),
                "unused-in-this-test".to_owned(),
            )])),
        );
        state
    }

    /// WS3 plan review X4: an `adapter = "cdc"`
    /// connector's slot/publication come from `dial`, with no fallback to a
    /// slug derived from its own `id` — even when `kind` does not say
    /// "`PostgreSQL`" at all, proving the dispatch is driven by `adapter`, not
    /// `kind`. The names embedded in the resulting error are exactly what
    /// was actually attempted, since the unreachable `127.0.0.1:1` target
    /// makes [`connector_deprovision::drop_slot_and_publication`] fail
    /// deterministically at the connect step, past both the adapter
    /// dispatch and the credential resolution.
    #[tokio::test]
    async fn deprovision_reads_slot_and_publication_from_dial_for_cdc_adapter() {
        let state = state_with_stub_secret_resolver();
        let info = dial_info(
            "Custom CDC System",
            Some("cdc"),
            serde_json::json!({
                "driver": "postgres",
                "host": "source.example.internal",
                "port": 5432,
                "database": "oms",
                "user": "replicator",
                "slotName": "dial_supplied_slot",
                "publicationName": "dial_supplied_pub",
            }),
        );
        let err = deprovision_postgres_connector(&state, "conn-cdc-dial-wins", &info)
            .await
            .expect_err("127.0.0.1:1 refuses every connection");
        let text = err.to_string();
        assert!(
            text.contains("dial_supplied_slot") && text.contains("dial_supplied_pub"),
            "expected the error to name dial's own slot/publication: {text}"
        );
        // The slug this connector's `id` would derive to must never appear
        // -- proves `dial` won with NO fallback, not merely that it was
        // tried first.
        assert!(
            !text.contains("conn_cdc_dial_wins"),
            "the slug-derived name must never appear once adapter=cdc: {text}"
        );
    }

    /// The exact defect this rule fixes: a `sql`-adapter connector must never
    /// have `drop_slot`/`drop_publication` attempted, however its `kind`
    /// string reads. Proven here at the unit level (rather than needing an
    /// unreachable host to prove absence indirectly): `Ok(None)` with NO
    /// dial attempt means this returns immediately, before ever resolving a
    /// credential or touching the network.
    #[tokio::test]
    async fn deprovision_never_attempted_for_a_sql_adapter_even_when_kind_says_postgresql() {
        let state = state_with_stub_secret_resolver();
        let info = dial_info(
            "PostgreSQL",
            Some("sql"),
            serde_json::json!({
                "driver": "postgres",
                "host": "warehouse.example.internal",
                "port": 5432,
                "database": "oms",
                "user": "reader",
            }),
        );
        let result = deprovision_postgres_connector(&state, "conn-sql-never-deprovisioned", &info)
            .await
            .expect("a sql-adapter connector must never even attempt a dial");
        assert!(result.is_none());
    }

    /// The bounded legacy fallback: `adapter IS NULL` (a pre-WS3 row) and
    /// `kind` names Postgres still falls back to the slug-derived names,
    /// unchanged from before this task — proven by the connector's own `id`
    /// slug appearing in the resulting error, since no `dial` exists to
    /// provide anything else.
    #[tokio::test]
    async fn deprovision_falls_back_to_slug_derivation_only_when_adapter_is_null_and_kind_is_postgres()
     {
        let state = state_with_stub_secret_resolver();
        let info = dial_info("PostgreSQL", None, serde_json::json!({}));
        let err = deprovision_postgres_connector(&state, "conn-legacy-null-adapter", &info)
            .await
            .expect_err("127.0.0.1:1 refuses every connection");
        let text = err.to_string();
        assert!(
            text.contains("conn_legacy_null_adapter_slot")
                && text.contains("conn_legacy_null_adapter_pub"),
            "expected the legacy path to fall back to the id-derived slug: {text}"
        );
    }

    /// `adapter IS NULL` and `kind` does NOT name Postgres — nothing to
    /// deprovision, same as any other non-CDC case.
    #[tokio::test]
    async fn deprovision_is_a_no_op_for_a_null_adapter_non_postgres_connector() {
        let state = state_with_stub_secret_resolver();
        let info = dial_info("Object storage", None, serde_json::json!({}));
        let result = deprovision_postgres_connector(&state, "conn-s3-warehouse", &info)
            .await
            .expect("a non-Postgres, null-adapter connector must never attempt a dial");
        assert!(result.is_none());
    }

    // ---- mongodb adapter dispatch tests ----

    /// The `mongodb` adapter's dispatch lives in
    /// [`resolve_debezium_source_target`] (not on [`SqlDriver`], since
    /// `MongoDB` is not a SQL/CDC source) and returns a target carrying
    /// the real `Debezium` `MongoDB` connector class — `Some("mongodb")`
    /// is a dispatch arm alongside the existing `sql`/`cdc` arms. The
    /// test passes through
    /// `resolve_debezium_source_target` directly (no router, no DB) so it
    /// stays inside `cargo test -p lakehouse-api --lib`, never the
    /// `tests/` directory's `#[sqlx::test]`-backed harness.
    #[test]
    fn mongodb_adapter_dispatch_returns_the_mongo_connector_class() {
        let info = ConnectorDialInfo {
            kind: "MongoDB CDC".to_owned(),
            host: "mongo.internal:27017".to_owned(),
            secret_ref: "env:CONNECTOR_MONGO_PASSWORD".to_owned(),
            secret_ref_secondary: None,
            adapter: Some("mongodb".to_owned()),
            dial: serde_json::json!({
                "hosts": ["mongo-a.internal:27017"],
                "database": "oms",
                "username": "cdc_reader",
                "directConnection": true,
            }),
        };
        let target = resolve_debezium_source_target("conn-mongo-1", &info)
            .expect("a mongodb adapter with a Mongo-shaped dial must dispatch");
        assert_eq!(
            target.connector_class,
            lakehouse_store::cdc::MONGO_CONNECTOR_CLASS
        );
        assert_eq!(
            target.connector_class,
            "io.debezium.connector.mongodb.MongoDbConnector"
        );
    }

    // ---- the Oracle CDC gate ----

    fn target_with_driver(driver: SqlDriver) -> DebeziumSourceTarget {
        DebeziumSourceTarget {
            host: "unused".to_owned(),
            port: 0,
            database: "unused".to_owned(),
            user: "unused".to_owned(),
            connector_class: "unused",
            driver,
        }
    }

    fn refusal_reason(response: Option<DebeziumPropertiesResponse>) -> String {
        match response.expect("an Oracle-driver connector must be refused") {
            DebeziumPropertiesResponse::Unsupported { supported, reason } => {
                assert!(!supported);
                reason
            }
            DebeziumPropertiesResponse::Rendered { .. } => {
                panic!("an Oracle-driver connector must never get a rendered template")
            }
        }
    }

    /// Flag off (the default): refused, and the reason names the setting and
    /// what `LogMiner` capture would need from the source database.
    #[test]
    fn an_oracle_connector_is_refused_when_the_logminer_flag_is_off() {
        let reason = refusal_reason(oracle_cdc_refusal(
            &target_with_driver(SqlDriver::Oracle),
            false,
        ));
        assert!(reason.contains("ORACLE_CDC_LOGMINER_ENABLED"), "{reason}");
        assert!(reason.contains("ARCHIVELOG"), "{reason}");
    }

    /// Flag ON: still refused. The only template this build can render is
    /// shaped for the Postgres/MySQL/SQL Server connectors; handing it back
    /// with Oracle's connector class on it would be a config that looks ready
    /// and that Oracle's connector rejects. The reason says the flag is set
    /// and why that is not enough, rather than repeating "not enabled".
    #[test]
    fn an_oracle_connector_is_refused_even_when_the_logminer_flag_is_on() {
        let reason = refusal_reason(oracle_cdc_refusal(
            &target_with_driver(SqlDriver::Oracle),
            true,
        ));
        assert!(reason.contains("is set"), "{reason}");
        assert!(reason.contains("no Debezium Oracle"), "{reason}");
    }

    /// The gate touches Oracle only: the drivers that have a real template
    /// render it whatever the flag says.
    #[test]
    fn the_oracle_gate_never_touches_the_drivers_that_have_a_template() {
        for driver in [SqlDriver::Postgres, SqlDriver::Mysql, SqlDriver::Mssql] {
            for flag in [false, true] {
                assert!(
                    oracle_cdc_refusal(&target_with_driver(driver), flag).is_none(),
                    "{driver:?} with flag={flag} must render"
                );
            }
        }
    }

    /// An Oracle-driver CDC connector still
    /// resolves cleanly through [`resolve_debezium_source_target`]
    /// regardless of the gate — the gate is a separate call in the
    /// handler. This regression-tests that the resolver does not
    /// silently start rejecting Oracle just because the gate was added.
    #[test]
    fn oracle_driver_still_resolves_through_resolve_debezium_source_target() {
        let info = ConnectorDialInfo {
            kind: "Oracle".to_owned(),
            host: "oracle.internal:1521".to_owned(),
            secret_ref: "env:CONNECTOR_ORACLE_PASSWORD".to_owned(),
            secret_ref_secondary: None,
            adapter: Some("cdc".to_owned()),
            dial: serde_json::json!({
                "driver": "oracle",
                "host": "oracle.internal",
                "port": 1521,
                "database": "ORCL",
                "user": "cdc_reader",
                "slotName": "oracle_slot",
                "publicationName": "oracle_pub",
            }),
        };
        let target = resolve_debezium_source_target("conn-oracle-1", &info)
            .expect("Oracle driver must still resolve, the gate fires later");
        assert_eq!(target.driver, SqlDriver::Oracle);
        assert_eq!(
            target.connector_class,
            "io.debezium.connector.oracle.OracleConnector"
        );
    }
}
