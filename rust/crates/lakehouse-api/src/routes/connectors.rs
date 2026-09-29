//! `/api/connectors/*` — connector definitions (source/sink systems),
//! backed by Postgres (`lakehouse-store`).
//!
//! # Not a port
//!
//! Like `routes::identity`, this replaces an *in-browser* mock
//! (`src/services/mock/connectors.ts`) that never had a server side —
//! there is no TypeScript route handler this is bug-compatible with.
//! Status codes are chosen to be correct: 201 on create, 404 on a missing
//! id, 409 on a duplicate name, 400 on a malformed body or a body that
//! still names the removed `secretRef`/`secretRefSecondary` fields, 503
//! with no database pool.
//!
//! # Credentials
//!
//! See `lakehouse_store::connectors`'s module doc comment for the full
//! decision record. The short version: no endpoint here ever returns a
//! `host` — [`lakehouse_store::connectors::Connector`] and
//! `ConnectorDetail` have no such field to serialize, and no `secretRef`
//! EXCEPT [`create`]'s response, once, at creation (ADR 0002 Addendum 3
//! — a user-created connector no longer chooses a ref at all, only a
//! source/kind; the server derives the name from the id it generates and
//! returns it so the operator knows what to provision).

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_core::secret::SecretValue;
use lakehouse_dagster::{DgConfiguredRun, iso_from_unix_seconds, map_run_status};
use lakehouse_store::PgPool;
use lakehouse_store::audit::{self as store_audit, NewAuditEvent};
use lakehouse_store::cdc::ConnectorSlug;
use lakehouse_store::connector_probe_result::{self, ConnectorProbeResult};
use lakehouse_store::connector_type::{self, ConnectorType};
use lakehouse_store::connectors::{self, ConnectorDetail, ConnectorDialInfo, CreateConnectorInput};
use lakehouse_store::ingest_spec::{Dial, SqlDriver};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

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

/// `GET /api/connectors` — every connector visible to the caller's active
/// tenant.
///
/// # Tenant scoping
///
/// `tenant_scope::resolve` runs first, fail closed: a principal belonging
/// to zero tenants (`Ok(None)`) gets an EMPTY list, returned before the
/// store is ever queried — never "unscoped, show every tenant's
/// connectors." A connector whose own `tenant_id` column is `NULL`
/// (unassigned — `0042_tenant_provisioning.sql`) never matches a real
/// tenant filter (SQL `NULL = $1` is never `true`), so it is invisible to
/// every tenant-scoped caller, not merely the ones who happen not to ask
/// for it.
///
/// # Errors
///
/// 401 if unauthenticated (should not happen — this route requires
/// `connector:manage` in `POLICY_TABLE`, which already extracts a
/// principal before this handler runs; the check here only covers the
/// internal, non-HTTP call path `routes::ai::tools::connectors` uses).
/// 404 if `X-Tenant` names a tenant the caller does not belong to. 503 if
/// no pool is configured; 500 on a database failure.
pub async fn list(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Vec<connectors::Connector>>> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let Some(tenant_id) = crate::tenant_scope::resolve(&principal, &headers)? else {
        return Ok(ApiJson(Vec::new()));
    };
    let filter = connectors::ConnectorFilter {
        tenant_id: Some(tenant_id),
    };
    Ok(ApiJson(
        connectors::list_connectors(pool(&state)?, &filter).await?,
    ))
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

/// The `PATCH /api/connectors/{id}` body. Mirrors `UpdateConnectorInput`
/// in `contracts/connectors.ts`. Every field is optional; an absent one is
/// left unchanged.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateConnectorBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    direction: Option<String>,
    #[serde(default)]
    environment: Option<String>,
    #[serde(default)]
    residency: Option<String>,
    #[serde(default)]
    host: Option<String>,
}

/// Validate an [`UpdateConnectorBody`] into the store's input. Pure —
/// unit-tested below.
///
/// # Errors
///
/// 400 for a blank field, an unknown `direction`, or an update that
/// changes nothing.
fn update_input(body: UpdateConnectorBody) -> Result<connectors::UpdateConnectorInput, ApiError> {
    let non_blank = |field: &str, value: Option<String>| -> Result<Option<String>, ApiError> {
        value.map(|v| required(field, &v)).transpose()
    };
    let direction = non_blank("direction", body.direction)?;
    if let Some(direction) = &direction
        && !VALID_DIRECTIONS.contains(&direction.as_str())
    {
        return Err(ApiError::BadRequest(format!(
            "direction must be one of {VALID_DIRECTIONS:?}, got {direction:?}"
        )));
    }
    let input = connectors::UpdateConnectorInput {
        name: non_blank("name", body.name)?,
        direction,
        environment: non_blank("environment", body.environment)?,
        residency: non_blank("residency", body.residency)?,
        host: non_blank("host", body.host)?,
    };
    if input.is_empty() {
        return Err(ApiError::BadRequest("nothing to update".to_owned()));
    }
    Ok(input)
}

/// Fields `PATCH` deliberately does not take, each with the route that
/// does — refused with that pointer instead of `deny_unknown_fields`'
/// generic "unknown field".
fn reject_non_patchable_fields(body: &Bytes) -> Result<(), ApiError> {
    let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(body) else {
        return Ok(()); // Malformed JSON is `parse_body`'s job to report.
    };
    let refusal = if map.contains_key("type") {
        Some(
            "a connector's type cannot be changed: it fixes the adapter, the connection shape \
             and the credential names — create a new connector instead",
        )
    } else if map.contains_key("tenant") || map.contains_key("tenantId") {
        Some("a connector's tenant is changed through PUT /api/connectors/{id}/tenant")
    } else if map.contains_key("credential") || map.contains_key("secretRef") {
        Some("a connector's credential is changed through PUT /api/connectors/{id}/credential")
    } else if map.contains_key("dial") {
        Some(
            "a connector's connection settings are changed through PUT /api/connectors/{id}/ingest-spec",
        )
    } else {
        None
    };
    refusal.map_or(Ok(()), |message| {
        Err(ApiError::BadRequest(message.to_owned()))
    })
}

/// `PATCH /api/connectors/{id}` — edit a connector's name, direction,
/// environment, residency and connection-target label. The console's edit
/// page calls this alongside the ingest-spec, tenant and credential routes,
/// each of which owns its own part of a connector.
///
/// # Errors
///
/// 401 without a principal (see [`create`] on why it is `Option`); 400 per
/// [`update_input`] / [`reject_non_patchable_fields`]; 404 if `id` is
/// unknown; 409 if the new name is taken; 503/500 as every other route.
pub async fn update(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<connectors::Connector>> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    reject_non_patchable_fields(&body)?;
    let input = update_input(parse_body(&body)?)?;
    let fields: Vec<&str> = [
        ("name", input.name.is_some()),
        ("direction", input.direction.is_some()),
        ("environment", input.environment.is_some()),
        ("residency", input.residency.is_some()),
        ("host", input.host.is_some()),
    ]
    .into_iter()
    .filter_map(|(field, changed)| changed.then_some(field))
    .collect();
    let updated = match connectors::update_connector(pool(&state)?, &id, &input).await {
        Ok(updated) => updated,
        Err(lakehouse_store::StoreError::NotFound) => {
            return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
        }
        Err(lakehouse_store::StoreError::Conflict) => {
            return Err(
                ApiError::Conflict("a connector with that name already exists".to_owned()).into(),
            );
        }
        Err(err) => return Err(ApiError::from(err).into()),
    };
    // Which fields changed, never their values: `host` is handled with the
    // same care as a credential-adjacent field everywhere in this module.
    let event = connector_audit_event(
        &principal,
        "connector.update",
        &id,
        json!({ "fields": fields }),
        "executed",
    );
    if let Err(err) = store_audit::insert(pool(&state)?, event).await {
        tracing::warn!(%err, connector_id = %id, "failed to record connector.update audit event");
    }
    Ok(ApiJson(updated))
}

/// The `POST /api/connectors` body's `credential` field. Mirrors
/// `CreateConnectorInput["credential"]` in `contracts/connectors.ts`.
///
/// There is no `secretRef`/`secretRefSecondary` field anywhere on this
/// body (ADR 0002 Addendum 3): the client does not know the connector's
/// id yet (the server generates it in [`connectors::create_connector`]),
/// so it cannot name a reference itself. It chooses a source scheme and a
/// kind per slot instead, and the server derives the actual names —
/// returned once, in [`CreateConnectorResponse`].
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialSpecBody {
    source: connectors::CredentialSource,
    primary: connectors::CredentialKind,
    /// `None` for a connector type that needs only one credential (e.g.
    /// `PostgreSQL`). `Some` for e.g. an S3 connector's access-key/
    /// secret-key pair — without this, an API-created S3 connector's
    /// `/test` can never succeed, since `probe_s3` requires both.
    #[serde(default)]
    secondary: Option<connectors::CredentialKind>,
    /// The credential VALUES, write-only — accepted only with `source:
    /// "managed"` (ADR 0002 Addendum 4). Written to the managed store by
    /// [`create`], never persisted in Postgres, never echoed back.
    #[serde(default)]
    values: Option<CredentialValuesBody>,
}

/// A credential value as it arrives in a request body. Deserializes like a
/// plain string, but its `Debug` never prints the content — so a
/// `#[derive(Debug)]` on any body that holds one (every body here has one)
/// cannot leak it into a log line. Converted to a [`SecretValue`] at the
/// first use.
#[derive(Deserialize)]
#[serde(transparent)]
pub struct SecretInput(String);

impl std::fmt::Debug for SecretInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl SecretInput {
    fn into_secret(self) -> SecretValue {
        SecretValue::new(self.0)
    }
}

/// `credential.values` on `POST /api/connectors`: one value per slot the
/// connector declares. `deny_unknown_fields` so a misspelled slot is an
/// error, not a silently unset credential.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialValuesBody {
    primary: SecretInput,
    #[serde(default)]
    secondary: Option<SecretInput>,
}

/// The `POST /api/connectors` body. Mirrors `CreateConnectorInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateConnectorBody {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    direction: String,
    host: String,
    credential: CredentialSpecBody,
    environment: String,
    /// The tenant's display name. Ignored when a tenant id resolves (the
    /// stored name then comes from the tenant row itself); required only
    /// for a caller that belongs to no tenant at all.
    #[serde(default)]
    tenant: String,
    /// The tenant this connector belongs to — must be one the caller is a
    /// member of. Absent: the caller's active tenant (`X-Tenant`, else
    /// their first), the same one `GET /api/connectors` lists — so a
    /// connector is visible to the person who just created it.
    #[serde(default)]
    tenant_id: Option<Uuid>,
    #[serde(default)]
    residency: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    owner: Option<String>,
}

/// The `POST /api/connectors` response: the created
/// [`connectors::Connector`] plus its credential reference NAMES. For an
/// `env`/`file` source those are what an operator must provision; for a
/// `managed` source with values the API already stored them
/// (`credential_stored`), and the names are informational only. Mirrors
/// `CreateConnectorResponse` in `contracts/connectors.ts`.
///
/// # Returned ONCE (ADR 0002 Addendum 3)
///
/// This is the only place these names are ever handed back. No GET
/// response for this connector repeats them — `Connector`/`ConnectorDetail`
/// gain no field (the module doc comment's guarantees 1/2 still hold
/// exactly as written); this is a distinct, create-only response shape.
/// An operator who loses these names can still recover them: they are a
/// PURE function of the connector's own id
/// (`lakehouse_store::connectors::derive_secret_ref`), so re-deriving them
/// from `id` (visible on every GET) reproduces the identical string. Only
/// the VALUE behind the name is secret; the name itself is not sensitive,
/// which is why re-deriving it is safe even though this route does not
/// offer a "show me again" endpoint.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateConnectorResponse {
    #[serde(flatten)]
    connector: connectors::Connector,
    credential: connectors::ConnectorCredentialNames,
    /// Whether [`create`] stored the credential value(s) itself (`managed`
    /// source with `values`) — i.e. whether there is anything left for an
    /// operator to provision. Never the value.
    credential_stored: bool,
}

const VALID_DIRECTIONS: [&str; 3] = ["source", "sink", "bidirectional"];

/// Refuse a body that still names `secretRef`/`secretRefSecondary` — the
/// pre-ADR-0002-Addendum-3 shape — instead of silently ignoring those
/// fields (AGENTS.md principle 2: never silently drop what a caller sent).
/// [`CreateConnectorBody`] has no field to deserialize either name into,
/// so a caller sending them would otherwise get no error and no
/// indication their `secretRef` was never used.
///
/// Runs on the raw JSON, before [`CreateConnectorBody`] deserialization,
/// so the message can name the ADR and the new shape explicitly, rather
/// than a generic "unknown field" a `#[serde(deny_unknown_fields)]` would
/// produce (which also cannot single out these two names for a
/// specialized message without a second copy of the field list already
/// implied by the struct definition).
fn reject_legacy_secret_ref_fields(body: &Bytes) -> Result<(), ApiError> {
    let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(body) else {
        return Ok(()); // Malformed JSON is `parse_body`'s job to report.
    };
    if map.contains_key("secretRef") || map.contains_key("secretRefSecondary") {
        return Err(ApiError::BadRequest(
            "secretRef/secretRefSecondary are no longer accepted here: a connector's credential \
             reference name is now assigned by the server, derived from its own id (ADR 0002 \
             Addendum 3, docs/adr/0002-secretref-resolution.md). Send `credential: { source, \
             primary, secondary? }` instead, and read the derived names off this route's \
             response"
                .to_owned(),
        ));
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
/// blank required field, an unrecognized `direction`, or a body that still
/// names `secretRef`/`secretRefSecondary` (see
/// [`reject_legacy_secret_ref_fields`] — ADR 0002 Addendum 3 removed both
/// fields); 409 if the name is taken; 503/500 as above.
pub async fn create(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<CreateConnectorResponse>)> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    reject_legacy_secret_ref_fields(&body)?;
    let body: CreateConnectorBody = parse_body(&body)?;
    let direction = required("direction", &body.direction)?;
    if !VALID_DIRECTIONS.contains(&direction.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "direction must be one of {VALID_DIRECTIONS:?}, got {direction:?}"
        ))
        .into());
    }
    let source = body.credential.source;
    let values = credential_values(
        source,
        body.credential.secondary.is_some(),
        body.credential.values,
    )?;
    let (tenant_id, tenant_name) =
        resolve_create_tenant(&state, &principal, &headers, body.tenant_id, &body.tenant).await?;
    let input = CreateConnectorInput {
        name: required("name", &body.name)?,
        kind: required("type", &body.kind)?,
        direction,
        host: required("host", &body.host)?,
        credential: connectors::CredentialSpec {
            source,
            primary: body.credential.primary,
            secondary: body.credential.secondary,
        },
        environment: required("environment", &body.environment)?,
        tenant: tenant_name,
        residency: body.residency,
        capabilities: body.capabilities,
        owner: body.owner,
    };
    let (created, credential) = connectors::create_connector(pool(&state)?, &input).await?;

    // ADR 0002 Addendum 4: the row exists, so its id — and therefore the
    // managed file names — are known. Store the value(s) now. A connector
    // whose credential could not be stored is removed again rather than
    // left behind looking configured.
    let credential_stored = values.is_some();
    let finished = async {
        if let Some(values) = values {
            store_credential_values(&state, &credential, values).await?;
        }
        if let Some(tenant_id) = tenant_id {
            connectors::assign_tenant(pool(&state)?, &created.id, tenant_id).await?;
        }
        Ok::<(), ApiError>(())
    }
    .await;
    if let Err(err) = finished {
        remove_managed_credentials(
            &state,
            &created.id,
            Some(&credential.primary),
            credential.secondary.as_deref(),
        )
        .await;
        if let Err(rollback) = connectors::delete_connector(pool(&state)?, &created.id).await {
            tracing::error!(
                %rollback,
                connector_id = %created.id,
                "connector could not be finished (credential or tenant) and its row could \
                 not be removed either; delete it manually"
            );
        }
        return Err(err.into());
    }

    // WS5 item D3: best-effort, never turns a successful create into an
    // error — name/type/direction and where the credential lives, never a
    // credential reference name (not this row's business to repeat, even
    // though a name alone is not a secret value) and never a value.
    let event = connector_audit_event(
        &principal,
        "connector.create",
        &created.id,
        json!({
            "name": created.name,
            "type": created.kind,
            "direction": created.direction,
            "credentialSource": source,
            "credentialStored": credential_stored,
        }),
        "executed",
    );
    if let Err(err) = store_audit::insert(pool(&state)?, event).await {
        tracing::warn!(%err, connector_id = %created.id, "failed to record connector.create audit event");
    }
    Ok((
        StatusCode::CREATED,
        ApiJson(CreateConnectorResponse {
            connector: created,
            credential,
            credential_stored,
        }),
    ))
}

/// Which tenant a new connector belongs to, and the name stored with it.
///
/// Resolved BEFORE the row exists, so a tenant the caller may not use never
/// leaves a connector behind. Before this, every connector created through
/// the console had `tenant_id = NULL` and was invisible to `list`, including
/// to the person who created it.
///
/// An explicit `requested` tenant must be one of the caller's (else 404 —
/// the same no-discovery rule as `tenant_scope::resolve`); absent, the
/// caller's active tenant is used. The stored name comes from the tenant
/// row, not from what the client typed; `typed_name` is used only for a
/// caller that belongs to no tenant at all.
async fn resolve_create_tenant(
    state: &AppState,
    principal: &Principal,
    headers: &HeaderMap,
    requested: Option<Uuid>,
    typed_name: &str,
) -> Result<(Option<Uuid>, String), ApiError> {
    let tenant_id = match requested {
        Some(requested) if principal.in_tenant(requested) => Some(requested),
        Some(_) => return Err(ApiError::NotFound("tenant not found".to_owned())),
        None => crate::tenant_scope::resolve(principal, headers)?,
    };
    let name = match tenant_id {
        Some(tenant_id) => {
            lakehouse_store::identity::get_tenant(pool(state)?, &tenant_id.to_string())
                .await?
                .name
        }
        None => required("tenant", typed_name)?,
    };
    Ok((tenant_id, name))
}

/// The validated credential values [`create`] will store: the primary, and
/// the secondary when the connector declares a secondary slot.
struct CredentialValues {
    primary: SecretValue,
    secondary: Option<SecretValue>,
}

/// Validate `credential.values` against the rest of the credential spec,
/// BEFORE any row is created, so a bad value never leaves a connector
/// behind. Pure — unit-tested below.
///
/// # Errors
///
/// 400 if values are sent with an operator-provisioned source (`env`/
/// `file`: the API does not write those, so accepting a value would be a
/// silent drop), if a secondary value is sent for a connector with no
/// secondary slot, or if a value fails
/// [`crate::connector_secret_store::validate_secret_value`].
fn credential_values(
    source: connectors::CredentialSource,
    has_secondary_slot: bool,
    values: Option<CredentialValuesBody>,
) -> Result<Option<CredentialValues>, ApiError> {
    let Some(values) = values else {
        return Ok(None);
    };
    if source != connectors::CredentialSource::Managed {
        return Err(ApiError::BadRequest(
            "credential.values is only accepted with credential.source \"managed\": an env/file \
             credential is provisioned by an operator, never through this API"
                .to_owned(),
        ));
    }
    if values.secondary.is_some() && !has_secondary_slot {
        return Err(ApiError::BadRequest(
            "credential.values.secondary was sent, but credential.secondary (its kind) was not"
                .to_owned(),
        ));
    }
    let primary = values.primary.into_secret();
    crate::connector_secret_store::validate_secret_value(primary.expose_secret())
        .map_err(|err| ApiError::BadRequest(format!("credential.values.primary: {err}")))?;
    let secondary = match values.secondary {
        Some(value) => {
            let value = value.into_secret();
            crate::connector_secret_store::validate_secret_value(value.expose_secret()).map_err(
                |err| ApiError::BadRequest(format!("credential.values.secondary: {err}")),
            )?;
            Some(value)
        }
        None => None,
    };
    Ok(Some(CredentialValues { primary, secondary }))
}

/// Map a store failure to the response a caller can act on. Every message
/// is already value-free (see `SecretStoreError`).
fn secret_store_error(err: &crate::connector_secret_store::SecretStoreError) -> ApiError {
    use crate::connector_secret_store::SecretStoreError;
    match err {
        SecretStoreError::InvalidValue(_) => ApiError::BadRequest(err.to_string()),
        SecretStoreError::NotManaged => ApiError::Internal(err.to_string()),
        SecretStoreError::Io(_) => ApiError::Unavailable(err.to_string()),
    }
}

/// Write [`CredentialValues`] under the managed names [`create`] derived.
async fn store_credential_values(
    state: &AppState,
    names: &connectors::ConnectorCredentialNames,
    values: CredentialValues,
) -> Result<(), ApiError> {
    let store = &state.connector_secret_store;
    store
        .write(&names.primary, &values.primary)
        .await
        .map_err(|err| secret_store_error(&err))?;
    if let (Some(name), Some(value)) = (names.secondary.as_deref(), values.secondary.as_ref()) {
        store
            .write(name, value)
            .await
            .map_err(|err| secret_store_error(&err))?;
    }
    Ok(())
}

/// Delete whichever of `refs` are managed (ADR 0002 Addendum 4) — an
/// operator's `env:`/`file:` credential is never touched. Best-effort: a
/// failure is logged, never surfaced, because every caller runs this after
/// (or instead of) a change that has already succeeded or failed on its
/// own terms.
async fn remove_managed_credentials(
    state: &AppState,
    connector_id: &str,
    primary: Option<&str>,
    secondary: Option<&str>,
) {
    for secret_ref in [primary, secondary].into_iter().flatten() {
        if !connectors::is_managed_secret_ref(secret_ref) {
            continue;
        }
        if let Err(err) = state.connector_secret_store.remove(secret_ref).await {
            tracing::warn!(
                %err,
                connector_id = %connector_id,
                "failed to remove a managed connector credential file"
            );
        }
    }
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
        &state.config.connector_internal_hosts(),
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

/// `?limit=` query for `GET /api/connectors/{id}/probe-history`.
#[derive(Debug, Deserialize)]
pub struct ProbeHistoryQuery {
    limit: Option<i64>,
}

/// The default number of history rows returned when `?limit=` is absent.
const DEFAULT_PROBE_HISTORY_LIMIT: i64 = 50;

/// The accepted range for `?limit=` -- matches
/// [`lakehouse_store::connector_probe_result`]'s own per-connector cap of
/// 200 rows: asking for more than this table could ever hold for one
/// connector is refused rather than silently clamped, so a caller relying
/// on an out-of-range `limit` learns that immediately instead of quietly
/// getting fewer rows than it asked for.
const PROBE_HISTORY_LIMIT_RANGE: std::ops::RangeInclusive<i64> = 1..=200;

/// `GET /api/connectors/{id}/probe-history` — the connector's most recent
/// connectivity-probe results, newest first. See
/// `lakehouse_store::connector_probe_result`'s module doc comment: this is
/// history distinct from `connector.health`/`lastTestAt` (current state,
/// unchanged by this route), and only ever contains SUPPORTED probes.
///
/// # Errors
///
/// 400 if `limit` is present and outside `1..=200` (never silently
/// clamped — a caller relying on an out-of-range value should learn that,
/// not get a quietly-truncated result); 404 if `id` is unknown; 503/500 as
/// every other connector route.
pub async fn probe_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ProbeHistoryQuery>,
) -> ApiResult<ApiJson<ProbeHistoryResponse>> {
    let limit = query.limit.unwrap_or(DEFAULT_PROBE_HISTORY_LIMIT);
    if !PROBE_HISTORY_LIMIT_RANGE.contains(&limit) {
        return Err(ApiError::BadRequest(format!(
            "limit must be between {} and {}, got {limit}",
            PROBE_HISTORY_LIMIT_RANGE.start(),
            PROBE_HISTORY_LIMIT_RANGE.end()
        ))
        .into());
    }
    let pool = pool(&state)?;
    // `get_connector` (rather than a bare existence check) so a 404 for an
    // unknown id matches every other `/api/connectors/{id}/*` route's
    // wording exactly -- see `detail`'s handler, above.
    connectors::get_connector(pool, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    let results = connector_probe_result::list_probe_results(pool, &id, limit).await?;
    Ok(ApiJson(ProbeHistoryResponse { results }))
}

/// The `GET /api/connectors/{id}/probe-history` response body. Mirrors
/// `ProbeHistoryResponse` in `contracts/connectors.ts`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeHistoryResponse {
    /// Newest first — see [`connector_probe_result::list_probe_results`].
    results: Vec<ConnectorProbeResult>,
}

/// The `PUT /api/connectors/{id}/secret` body. Mirrors
/// `RotateConnectorSecretRequest` in `contracts/connectors.ts`.
///
/// No `newSecretRef` free-text field any more (ADR 0002 Addendum 3): the
/// caller cannot name a ref, only a source scheme and a kind for the slot
/// being rotated — the server derives the new ref from the CONNECTOR'S
/// OWN id (`id` is a path parameter here, already fixed by the time this
/// body is read), the same way [`connectors::create_connector`] does.
/// Since a derived name always begins `CONNECTOR_CONN_`/`connector_conn_`
/// (every id begins `conn-`), a rotation can never target a reserved,
/// deployment-owned pattern (`CONNECTOR_PG_*`, `CONNECTOR_S3_*`) —
/// structurally, not by a runtime check — closing the same exfiltration
/// path a free-text `newSecretRef` would have reopened after `create`'s
/// own equivalent fix.
///
/// `deny_unknown_fields`: a caller-supplied field this shape does not
/// name (e.g. a lingering `newSecretRef`, or a typo) fails the request
/// rather than being silently ignored — this route's whole point is
/// precise control over which slot gets rewritten, so a misspelled or
/// stale field should never silently no-op.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RotateSecretBody {
    /// Which of the connector's two credential slots to rotate.
    slot: connectors::SecretSlot,
    /// `env:` or `file:` — see [`connectors::derive_secret_ref`].
    source: connectors::CredentialSource,
    /// The suffix the derived name ends in.
    kind: connectors::CredentialKind,
}

/// The `PUT /api/connectors/{id}/secret` response body.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotateSecretResponse {
    /// Always `true` — this route either rotates the slot or returns an
    /// error; there is no partial/pending state to report.
    rotated: bool,
    /// Echoes back which slot was rotated.
    slot: connectors::SecretSlot,
}

/// `PUT /api/connectors/{id}/secret` — rotate one of a connector's two
/// credential REFERENCE NAMES (never a credential value), PROBE-FIRST.
///
/// # Why probe before swap
///
/// The candidate credential reference (derived from this connector's own
/// id, per [`RotateSecretBody`]'s doc comment) must already be
/// provisioned (an env var, a secret-manager path) — this route never
/// carries or accepts a secret value itself. Writing a derived name
/// straight onto the connector row without checking it first would let a
/// not-yet-provisioned ref silently break every future dial of this
/// connector, discovered only the next time something tries to use it. So
/// this handler builds an in-memory COPY of the connector's dial info
/// with the derived ref swapped into the requested slot, runs a REAL
/// connectivity probe against that copy using the SAME
/// [`crate::connector_probe::probe`], the same
/// [`AppState::connector_secret_resolver`], and the same
/// `connector_probe_allow_internal_hosts` flag [`test_connection`]
/// uses — nothing is written to the database until that probe reports
/// `supported: true, ok: true`.
///
/// Using the SAME allowlisted resolver as `POST .../test` is deliberate,
/// not incidental: every derived name already matches
/// [`crate::state::CONNECTOR_ALLOWED_SECRET_REF_PATTERNS`] (ADR 0002
/// Addendum 3), so this only ever fails as an honest "not provisioned
/// yet" — never a resolver refusal, since a derived name can never be one
/// of the reserved, deployment-owned patterns (`RotateSecretBody`'s doc
/// comment explains why that is structural, not a runtime check).
///
/// # Why the probe result is never persisted as history
///
/// [`connectors::record_test_result`] (and the
/// [`connector_probe_result`] history row it writes) exist to record
/// what happened when the connector's CURRENT, in-use credential was
/// dialed. The probe this handler runs dials a credential the connector
/// is NOT using yet — recording that as connector history would make
/// `GET .../probe-history` show a "successful test" against a ref the
/// connector had never actually been configured with at the time,
/// which is not what that endpoint promises its readers. So this
/// handler calls [`crate::connector_probe::probe`] directly and never
/// [`connectors::record_test_result`]; `connector.health`/`lastTestAt`
/// and the probe-history table are both left exactly as they were.
///
/// # Errors
///
/// 400 on a malformed body, or one that still names a field this shape
/// does not have (`deny_unknown_fields`, e.g. a lingering `newSecretRef`);
/// 404 if `id` is unknown; 422 if the connector's type is either unsupported by
/// this build's probe (a rotation can never be verified, so it is never
/// applied) or the probe genuinely fails against the candidate credential;
/// 409 if the connector's current ref in that slot changed between this
/// handler's read and its write (someone else rotated it first — reload and
/// retry); 503/500 as every other connector route.
pub async fn rotate_secret(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<RotateSecretResponse>> {
    let body: RotateSecretBody = parse_body(&body)?;
    // Derived from THIS connector's own id -- see `RotateSecretBody`'s doc
    // comment for why this can never target a reserved, deployment-owned
    // pattern.
    let new_secret_ref = connectors::derive_secret_ref(&id, body.source, body.kind);

    let dial_info = connectors::get_connector_dial_info(pool(&state)?, &id).await?;
    let Some(dial_info) = dial_info else {
        return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
    };

    // The value read here, BEFORE the probe, is what `swap_secret_ref`
    // compares against at write time — closing the race between this
    // read and that write, not just between two callers of this route.
    let expected_old = match body.slot {
        connectors::SecretSlot::Primary => Some(dial_info.secret_ref.clone()),
        connectors::SecretSlot::Secondary => dial_info.secret_ref_secondary.clone(),
    };

    let mut candidate = dial_info.clone();
    match body.slot {
        connectors::SecretSlot::Primary => candidate.secret_ref = new_secret_ref.clone(),
        connectors::SecretSlot::Secondary => {
            candidate.secret_ref_secondary = Some(new_secret_ref.clone());
        }
    }

    let outcome = crate::connector_probe::probe(
        &candidate,
        state.connector_secret_resolver.as_ref(),
        &state.config.connector_internal_hosts(),
    )
    .await;

    if !outcome.supported {
        // 422, not 409: nothing about this connector's STATE conflicts
        // with the request — this build simply cannot verify ANY
        // rotation for this connector type, so one is never applied
        // unverified. `test_connection` reports the identical
        // `supported: false` case as 200 (it is that route's whole
        // successful, honest response shape); here it must fail the
        // request instead, since the caller asked for a WRITE this
        // handler cannot safely perform.
        return Err(ApiError::Unprocessable(format!(
            "connector {id}'s type cannot be probed by this build, so a secret rotation \
             cannot be verified and was NOT applied: {}",
            outcome.message
        ))
        .into());
    }
    if !outcome.ok {
        // 422: the candidate credential itself does not work — the
        // probe's own classified message is already safe to surface
        // (see `connector_probe`'s module doc comment: never raw
        // upstream `Display` text).
        return Err(ApiError::Unprocessable(outcome.message).into());
    }

    match connectors::swap_secret_ref(
        pool(&state)?,
        &id,
        body.slot,
        expected_old.as_deref(),
        &new_secret_ref,
    )
    .await
    {
        Ok(()) => {}
        Err(lakehouse_store::StoreError::NotFound) => {
            return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
        }
        Err(lakehouse_store::StoreError::Conflict) => {
            return Err(ApiError::Conflict(
                "the secret ref changed since it was read; reload and retry".to_owned(),
            )
            .into());
        }
        Err(err) => return Err(ApiError::from(err).into()),
    }

    // Matches this file's established audit convention (`create`,
    // `test_connection`, `delete`): only a SUCCESSFUL mutation is
    // audited here, best-effort, never turning a completed rotation into
    // an error response. A refused attempt (bad body, unsupported type,
    // failed probe, stale expected_old) is reported to the caller via
    // its error response alone, exactly as every other refusal on this
    // file's routes is -- none of `create`/`test_connection`/`delete`
    // record a refusal either.
    //
    // `args` names the slot and that the probe succeeded — NEVER the old
    // or new ref name (WS5 item D3's rule for this whole file: a
    // reference name is not this row's business to repeat, even though
    // it is not itself a credential value).
    let event = connector_audit_event(
        &principal,
        "connector.secret_rotate",
        &id,
        json!({ "slot": body.slot.as_str(), "probeOk": true }),
        "executed",
    );
    if let Err(err) = store_audit::insert(pool(&state)?, event).await {
        tracing::warn!(%err, connector_id = %id, "failed to record connector.secret_rotate audit event");
    }

    Ok(ApiJson(RotateSecretResponse {
        rotated: true,
        slot: body.slot,
    }))
}

/// One slot's new credential in a `PUT /api/connectors/{id}/credential`
/// body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialSlotBody {
    /// The suffix the managed name ends in.
    kind: connectors::CredentialKind,
    /// The credential itself. Write-only; see [`SecretInput`].
    value: SecretInput,
}

/// The `PUT /api/connectors/{id}/credential` body. Mirrors
/// `SetConnectorCredentialRequest` in `contracts/connectors.ts`.
///
/// Unlike [`RotateSecretBody`], this carries credential VALUES — the write
/// path ADR 0002 Addendum 4 adds for a user who has no access to the
/// host's environment. The refs are still never caller-chosen: each is
/// derived from the connector's own id with
/// [`connectors::CredentialSource::Managed`].
///
/// Both slots in ONE request, not one request per slot: an S3 connector's
/// access key and secret key only work as a pair, so they must be probed
/// together — changing them one at a time would probe the new access key
/// against the old secret key and always fail.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetCredentialBody {
    #[serde(default)]
    primary: Option<CredentialSlotBody>,
    #[serde(default)]
    secondary: Option<CredentialSlotBody>,
}

/// The `PUT /api/connectors/{id}/credential` response body. Never carries
/// a value or a ref.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCredentialResponse {
    /// Always `true` — a refused request is an error response instead.
    saved: bool,
    /// Which slot(s) were set.
    slots: Vec<connectors::SecretSlot>,
    /// Whether a real probe dialed the source with the new credential(s)
    /// before they were saved. `false` only for a connector type this
    /// build cannot probe (see [`set_credential`]'s doc comment).
    verified: bool,
    /// The probe's own classified message — safe to show (see
    /// `connector_probe`'s module doc comment).
    message: String,
}

/// One slot a [`set_credential`] request changes, validated.
struct SlotChange {
    slot: connectors::SecretSlot,
    new_ref: String,
    value: SecretValue,
    old_ref: Option<String>,
}

/// Validate a [`SetCredentialBody`] into one [`SlotChange`] per slot sent,
/// each with its managed ref derived from THIS connector's id.
fn slot_changes(
    id: &str,
    dial_info: &ConnectorDialInfo,
    body: SetCredentialBody,
) -> Result<Vec<SlotChange>, ApiError> {
    let mut changes = Vec::new();
    for (slot, input) in [
        (connectors::SecretSlot::Primary, body.primary),
        (connectors::SecretSlot::Secondary, body.secondary),
    ] {
        let Some(input) = input else { continue };
        let value = input.value.into_secret();
        crate::connector_secret_store::validate_secret_value(value.expose_secret())
            .map_err(|err| ApiError::BadRequest(format!("{}: {err}", slot.as_str())))?;
        let old_ref = match slot {
            connectors::SecretSlot::Primary => Some(dial_info.secret_ref.clone()),
            connectors::SecretSlot::Secondary => dial_info.secret_ref_secondary.clone(),
        };
        changes.push(SlotChange {
            slot,
            new_ref: connectors::derive_secret_ref(
                id,
                connectors::CredentialSource::Managed,
                input.kind,
            ),
            value,
            old_ref,
        });
    }
    Ok(changes)
}

/// Point each changed slot at its managed ref (optimistic-concurrency
/// guarded, like `rotate_secret`), and delete a managed file the slot no
/// longer references (a kind change).
async fn apply_ref_swaps(
    state: &AppState,
    id: &str,
    changes: &[SlotChange],
) -> Result<(), ApiError> {
    for (i, change) in changes.iter().enumerate() {
        if change.old_ref.as_deref() == Some(change.new_ref.as_str()) {
            continue;
        }
        let swapped = match pool(state) {
            Ok(pool) => {
                connectors::swap_secret_ref(
                    pool,
                    id,
                    change.slot,
                    change.old_ref.as_deref(),
                    &change.new_ref,
                )
                .await
            }
            Err(err) => {
                discard_unswapped(state, id, &changes[i..]).await;
                return Err(err);
            }
        };
        let err = match swapped {
            Ok(()) => {
                remove_managed_credentials(state, id, change.old_ref.as_deref(), None).await;
                continue;
            }
            Err(lakehouse_store::StoreError::NotFound) => {
                ApiError::NotFound(format!("Connector {id} not found"))
            }
            Err(lakehouse_store::StoreError::Conflict) => ApiError::Conflict(
                "the connector's credential changed since it was read; reload and retry".to_owned(),
            ),
            Err(err) => ApiError::from(err),
        };
        discard_unswapped(state, id, &changes[i..]).await;
        return Err(err);
    }
    Ok(())
}

/// Removes the files [`set_credential`] wrote for `changes` whose ref swap
/// never happened, so a failed swap leaves no managed file that no
/// connector names. Skipped: a change whose new ref IS its old ref (that
/// file was replaced in place and is still the live credential), and any
/// ref the connector names right now — a concurrent `set_credential` may
/// have swapped to the very same derived name, and its file must survive.
/// When the current refs cannot be read, nothing is removed: an orphaned
/// file is harmless, a deleted live one breaks the connector.
async fn discard_unswapped(state: &AppState, id: &str, changes: &[SlotChange]) {
    let live = match pool(state) {
        Ok(pool) => connectors::get_connector_dial_info(pool, id).await.ok(),
        Err(_) => None,
    };
    let Some(live) = live else {
        return;
    };
    let in_use = |r: &str| {
        live.as_ref().is_some_and(|info| {
            info.secret_ref == r || info.secret_ref_secondary.as_deref() == Some(r)
        })
    };
    for change in changes {
        if change.old_ref.as_deref() != Some(change.new_ref.as_str()) && !in_use(&change.new_ref) {
            remove_managed_credentials(state, id, Some(&change.new_ref), None).await;
        }
    }
}

/// `PUT /api/connectors/{id}/credential` — the user supplies credential
/// values, and the API stores them as this connector's managed credentials
/// (ADR 0002 Addendum 4). Also how an operator-provisioned connector
/// (`env:`/`file:`) is switched to a managed one.
///
/// # Probe first, like `rotate_secret`
///
/// The candidate values are tested BEFORE anything is written: the probe
/// runs against an in-memory copy of the connector whose changed slots name
/// the managed refs, resolved by a [`CandidateSecretResolver`] that answers
/// those refs from memory and delegates every other ref to the usual
/// allowlisted resolver. A credential the source rejects is refused (422)
/// and never stored — the connector keeps dialing with whatever it had.
///
/// # One deliberate difference from `rotate_secret`
///
/// `rotate_secret` refuses a connector type this build cannot probe,
/// because the credential it would swap to already exists somewhere and
/// can be verified later. Here the user holds the ONLY copy of the value;
/// refusing an unprobeable type (Kafka, SFTP, `MongoDB`, Oracle, …) would
/// make its credential impossible to set at all. So an unsupported probe
/// saves the value with `verified: false` and says so — never a fabricated
/// success.
///
/// # Errors
///
/// 400 on a malformed body, no slot at all, or an unusable value; 404 if
/// `id` is unknown; 422 if the probe dialed and the source rejected the
/// credential; 409 if a slot's ref changed between this handler's read and
/// its write; 503 if the credential store is not mounted; 500 as every
/// other route.
///
/// [`CandidateSecretResolver`]: crate::connector_secret_store::CandidateSecretResolver
pub async fn set_credential(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<SetCredentialResponse>> {
    let body: SetCredentialBody = parse_body(&body)?;
    if body.primary.is_none() && body.secondary.is_none() {
        return Err(ApiError::BadRequest(
            "at least one of primary/secondary is required".to_owned(),
        )
        .into());
    }

    let dial_info = connectors::get_connector_dial_info(pool(&state)?, &id).await?;
    let Some(dial_info) = dial_info else {
        return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
    };

    let changes = slot_changes(&id, &dial_info, body)?;
    let mut candidate = dial_info.clone();
    for change in &changes {
        match change.slot {
            connectors::SecretSlot::Primary => candidate.secret_ref = change.new_ref.clone(),
            connectors::SecretSlot::Secondary => {
                candidate.secret_ref_secondary = Some(change.new_ref.clone());
            }
        }
    }
    let resolver = crate::connector_secret_store::CandidateSecretResolver::new(
        changes
            .iter()
            .map(|c| (c.new_ref.clone(), c.value.clone()))
            .collect(),
        state.connector_secret_resolver.clone(),
    );
    let outcome = crate::connector_probe::probe(
        &candidate,
        &resolver,
        &state.config.connector_internal_hosts(),
    )
    .await;
    if outcome.supported && !outcome.ok {
        return Err(ApiError::Unprocessable(format!(
            "the credential was NOT saved: {}",
            outcome.message
        ))
        .into());
    }

    for change in &changes {
        state
            .connector_secret_store
            .write(&change.new_ref, &change.value)
            .await
            .map_err(|err| secret_store_error(&err))?;
    }
    apply_ref_swaps(&state, &id, &changes).await?;

    let slots: Vec<connectors::SecretSlot> = changes.iter().map(|c| c.slot).collect();
    // Same audit convention as `rotate_secret`: successful mutations only,
    // best-effort, the slots and whether they were verified — never a ref
    // name, never a value.
    let event = connector_audit_event(
        &principal,
        "connector.credential_set",
        &id,
        json!({
            "slots": slots.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "verified": outcome.supported,
        }),
        "executed",
    );
    if let Err(err) = store_audit::insert(pool(&state)?, event).await {
        tracing::warn!(%err, connector_id = %id, "failed to record connector.credential_set audit event");
    }

    Ok(ApiJson(SetCredentialResponse {
        saved: true,
        slots,
        verified: outcome.supported,
        message: outcome.message,
    }))
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
        &state.config.connector_internal_hosts(),
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
            // Replication slots and publications are Postgres objects.
            // A MySQL (binlog) or SQL Server (per-table CDC) source keeps
            // no server-side state for this connector to drop.
            if cdc.driver != SqlDriver::Postgres {
                return Ok(None);
            }
            // The wizard stores `host` as a bare hostname; user, port and
            // database live only in `dial` (same as `connector_probe`).
            let target = PgHost {
                host: cdc.host.clone(),
                port: cdc.port,
                user: cdc.user.clone(),
                database: cdc.database.clone(),
            };
            deprovision_with_names(
                state,
                id,
                dial_info,
                &target,
                &cdc.slot_name,
                &cdc.publication_name,
            )
            .await
            .map(Some)
        }
        None if dial_info.kind.to_lowercase().contains("postgres") => {
            let slug = connector_slug_for_id(id)?;
            // A pre-WS3 row: its only record of the target is the legacy
            // `<user>@<host>:<port>/<database>` host string.
            let Some(parsed) = connector_probe::parse_postgres_host(&dial_info.host) else {
                return Err(ApiError::Internal(format!(
                    "connector {id} is registered as PostgreSQL but its host is not shaped \
                     \"<user>@<host>:<port>/<database>\", so CDC deprovisioning cannot even \
                     attempt to dial it"
                )));
            };
            let target = PgHost {
                host: parsed.host.to_owned(),
                port: parsed.port,
                user: parsed.user.to_owned(),
                database: parsed.database.to_owned(),
            };
            deprovision_with_names(
                state,
                id,
                dial_info,
                &target,
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

/// Where a Postgres CDC source lives, resolved by
/// [`deprovision_postgres_connector`] from whichever record the connector
/// has: its parsed `dial` (every connector this build creates) or the
/// legacy host string (a pre-WS3 row).
struct PgHost {
    host: String,
    port: u16,
    user: String,
    database: String,
}

/// The shared body behind every [`deprovision_postgres_connector`] arm that
/// actually attempts a drop: resolve `dial_info`'s credential through the
/// same allowlisted resolver [`crate::connector_probe::probe`] uses for
/// `POST /api/connectors/{id}/test`, then drop
/// `slot_name`/`publication_name` on `target`.
async fn deprovision_with_names(
    state: &AppState,
    id: &str,
    dial_info: &ConnectorDialInfo,
    target: &PgHost,
    slot_name: &str,
    publication_name: &str,
) -> Result<Deprovisioned, ApiError> {
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
        host: target.host.clone(),
        port: target.port,
        user: target.user.clone(),
        password,
        database: target.database.clone(),
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
/// # Pipelines that read from the connector block the delete
///
/// Checked first, before any deprovisioning: while a pipeline still names
/// this connector the delete is a 409 listing them, `force` or not.
///
/// # Errors
///
/// 401 if no principal is present (see [`create`]'s doc comment on why
/// this is `Option`, not a bare `Extension`); 404 if `id` is unknown; 409
/// if a pipeline still reads from it, or if CDC deprovisioning failed and
/// `force` was not given; 503/500 as above.
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

    // Before anything irreversible: a pipeline still reading from this
    // connector would be left pointing at nothing (no foreign key guards
    // `pipeline_definition.connector_id`). `force` does not override this
    // — it exists for an unreachable CDC source, not for orphaning
    // pipelines.
    let dependents = connectors::dependent_pipelines(pool(&state)?, &id).await?;
    if !dependents.is_empty() {
        let names = dependents
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ApiError::Conflict(format!(
            "connector {id} was NOT deleted: {} pipeline(s) still read from it ({names}). \
             Delete those pipelines or point them at another connector first.",
            dependents.len()
        ))
        .into());
    }

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
    // ADR 0002 Addendum 4: a managed credential belongs to this connector
    // alone, so it goes with it. An operator's env/file credential is left
    // exactly where the operator put it.
    remove_managed_credentials(
        &state,
        &id,
        Some(&dial_info.secret_ref),
        dial_info.secret_ref_secondary.as_deref(),
    )
    .await;
    // WS5 item D3: best-effort, after the row is already gone — a failed
    // audit write here must never resurrect the 404/409 branches above or
    // undo a delete that already succeeded.
    let event = connector_audit_event(&principal, "connector.delete", &id, json!({}), "executed");
    if let Err(err) = store_audit::insert(pool(&state)?, event).await {
        tracing::warn!(%err, connector_id = %id, "failed to record connector.delete audit event");
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Body for `PUT /api/connectors/{id}/tenant`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignTenantBody {
    tenant_id: Uuid,
}

/// `PUT /api/connectors/{id}/tenant` — assign (or reassign) a connector to
/// a tenant.
///
/// # Why a distinct route, not a field on `POST`/an update body
///
/// A connector's tenant is a governance decision (who is allowed to see
/// and manage this row), made independently of whoever defined its dial
/// config, and worth its own explicit audit trail entry rather than being
/// folded into an unrelated create/update diff.
///
/// # Why this exists at all
///
/// `0042_tenant_provisioning.sql` backfills `tenant_id` on only the two
/// connector rows `0022_prune_connector_seed.sql` seeds — every connector a
/// real deployment creates afterward starts `tenant_id = NULL` and is
/// invisible to `GET /api/connectors`'s tenant-scoped read until
/// assigned. This route is how an operator closes that gap, rather than
/// leaving it disclosed only in a migration comment nobody reading the API
/// ever sees.
///
/// # Errors
///
/// 404 if either the connector id or the tenant id does not exist — the
/// same status for both, deliberately: this route must not become a
/// discovery oracle for which tenant ids exist (mirrors `tenant_scope::
/// resolve`'s own 404-not-403 rule). 400 for a malformed body. 503 if no
/// pool is configured; 500 on any other database failure.
pub async fn assign_connector_tenant(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let AssignTenantBody { tenant_id } = parse_body(&body)?;
    let pool = pool(&state)?;
    if !lakehouse_store::identity::tenant_exists(pool, tenant_id).await? {
        return Err(ApiError::NotFound("tenant not found".to_owned()).into());
    }
    connectors::assign_tenant(pool, &id, tenant_id)
        .await
        .map_err(|err| match err {
            lakehouse_store::StoreError::NotFound => {
                ApiError::NotFound(format!("Connector {id} not found"))
            }
            other => other.into(),
        })?;
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
/// `&state.config.connector_internal_hosts()`. Never checks
/// `sheets`: its dial names a spreadsheet id, not a caller-chosen host —
/// every `sheets` connector targets Google's own fixed hosts.
///
/// # Errors
///
/// Returns `ApiError::BadRequest` naming the offending resolved address
/// (the same message [`connector_probe::resolve_checked`] already produces
/// for `POST .../test`) if a checked host resolves to a private/internal
/// address that `internal_hosts` does not permit.
async fn check_dial_ssrf(
    dial: &Dial,
    internal_hosts: &crate::internal_hosts::InternalHosts,
) -> Result<(), ApiError> {
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
    connector_probe::resolve_checked(host, port, internal_hosts)
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
    check_dial_ssrf(&dial, &state.config.connector_internal_hosts()).await?;
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
/// 409 if a run for this connector is still queued or running (see
/// [`ingest_run_history`]); 422 if `Dagster` reports a launch-time
/// failure; 503 on a `Dagster` transport failure; 503/500 as above for the
/// database pool.
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

    // One run at a time per connector. Bronze is append-only, so a second
    // run started while the first is still going copies every table into
    // Bronze a second time — a double click was enough to do it. Checked
    // against Dagster's own run list, so a run started by the schedule
    // counts too. Not a lock: two requests in the same instant can both
    // pass, which the console's disabled button covers.
    let recent = state
        .dagster
        .list_runs_for_job_with_config(INGEST_JOB, RECENT_INGEST_RUNS)
        .await
        .map_err(|err| ApiError::Unavailable(js_error(err)))?;
    if let Some(active) = connector_runs(&recent, &id)
        .into_iter()
        .find(ConnectorIngestRun::is_active)
    {
        return Err(ApiError::Conflict(format!(
            "a run for this connector is already {} (run {}); wait for it to finish before \
             starting another",
            active.status,
            active.run_id.get(..8).unwrap_or(&active.run_id),
        ))
        .into());
    }

    // Mirrors `agent_runs.py`'s `_employee_run_config` shape: one static
    // job (`ingest_job`), the launched run's only per-run input is this
    // connector's id, which `run_ingest`'s Dagster op re-fetches the
    // connector row by (`dagster/dispar_orchestrate/ingest_factory.py`).
    let run_config = json!({"ops": {"run_ingest": {"config": {"connector_id": id}}}});
    match state
        .dagster
        .launch_run_with_config(INGEST_JOB, &run_config)
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

/// The one Dagster job every connector's ingest runs as
/// (`dagster/dispar_orchestrate/ingest_factory.py`).
const INGEST_JOB: &str = "ingest_job";

/// How many of `ingest_job`'s most recent runs, across every connector,
/// are searched for one connector's runs.
const RECENT_INGEST_RUNS: u32 = 100;

/// How many of one connector's runs [`ingest_run_history`] returns.
const CONNECTOR_RUN_HISTORY: usize = 20;

/// One `ingest_job` run of one connector.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorIngestRun {
    /// `Dagster`'s run id.
    pub run_id: String,
    /// `queued | running | completed | failed | cancelled | unknown`
    /// ([`map_run_status`], the vocabulary the pipelines pages use).
    pub status: &'static str,
    /// ISO-8601, `None` until the run starts.
    pub started_at: Option<String>,
    /// ISO-8601, `None` until the run ends.
    pub ended_at: Option<String>,
}

impl ConnectorIngestRun {
    /// Whether the run has not finished yet.
    fn is_active(&self) -> bool {
        matches!(self.status, "queued" | "running")
    }
}

/// `connector_id`'s runs among `runs`, keeping `Dagster`'s newest-first
/// order. Every connector runs the same `ingest_job`; only the run config
/// (`ops.run_ingest.config.connector_id`) tells whose run it is.
fn connector_runs(runs: &[DgConfiguredRun], connector_id: &str) -> Vec<ConnectorIngestRun> {
    runs.iter()
        .filter(|run| {
            run.run_config
                .pointer("/ops/run_ingest/config/connector_id")
                .and_then(Value::as_str)
                == Some(connector_id)
        })
        .take(CONNECTOR_RUN_HISTORY)
        .map(|run| ConnectorIngestRun {
            run_id: run.run_id.clone(),
            status: map_run_status(&run.status),
            started_at: run.start_time.map(iso_from_unix_seconds),
            ended_at: run.end_time.map(iso_from_unix_seconds),
        })
        .collect()
}

/// `GET /api/connectors/{id}/ingest/runs` — this connector's recent
/// `ingest_job` runs, newest first, with their status.
///
/// The per-table results (`GET /api/governance/ingest-runs`) are written
/// only once a run reaches a table, so a run that fails before that — an
/// unreachable API, a refused credential — leaves no result at all. This
/// is the list that still shows it, and the one the console watches to
/// know a run is still going.
///
/// # Errors
///
/// 503 when `Dagster` cannot be reached.
pub async fn ingest_run_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Vec<ConnectorIngestRun>>> {
    let runs = state
        .dagster
        .list_runs_for_job_with_config(INGEST_JOB, RECENT_INGEST_RUNS)
        .await
        .map_err(|err| ApiError::Unavailable(js_error(err)))?;
    Ok(ApiJson(connector_runs(&runs, &id)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    fn dg_run(id: &str, status: &str, connector: &str) -> DgConfiguredRun {
        DgConfiguredRun {
            run_id: id.to_owned(),
            status: status.to_owned(),
            start_time: Some(1_790_670_111.0),
            end_time: (status == "SUCCESS").then_some(1_790_670_153.0),
            run_config: json!({ "ops": { "run_ingest": { "config": { "connector_id": connector } } } }),
        }
    }

    #[test]
    fn connector_runs_keeps_only_this_connectors_runs_newest_first() {
        let runs = [
            dg_run("run-3", "STARTED", "conn-a"),
            dg_run("run-2", "SUCCESS", "conn-b"),
            dg_run("run-1", "SUCCESS", "conn-a"),
        ];
        let mine = connector_runs(&runs, "conn-a");
        assert_eq!(
            mine.iter().map(|r| r.run_id.as_str()).collect::<Vec<_>>(),
            ["run-3", "run-1"]
        );
        assert_eq!(mine[0].status, "running");
        assert!(mine[0].is_active());
        assert_eq!(mine[0].ended_at, None);
        assert_eq!(mine[1].status, "completed");
        assert!(!mine[1].is_active());
        assert_eq!(
            mine[1].started_at.as_deref(),
            Some("2026-09-29T08:21:51.000Z")
        );
    }

    #[test]
    fn connector_runs_ignores_a_run_without_a_connector_id() {
        let mut run = dg_run("run-1", "SUCCESS", "conn-a");
        run.run_config = json!({});
        assert!(connector_runs(&[run], "conn-a").is_empty());
    }

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
            credential: CredentialSpecBody {
                source: connectors::CredentialSource::Env,
                primary: connectors::CredentialKind::Token,
                secondary: None,
                values: None,
            },
            environment: "production".to_owned(),
            tenant: "t".to_owned(),
            tenant_id: None,
            residency: String::new(),
            capabilities: vec![],
            owner: None,
        };
        assert!(!VALID_DIRECTIONS.contains(&body.direction.as_str()));
    }

    /// `credential.secondary` must parse through the request body
    /// (camelCase, per the struct's `rename_all`) and reach
    /// `CreateConnectorInput` — otherwise an API-created S3 connector can
    /// never be tested, since `probe_s3` requires both refs.
    #[test]
    fn credential_secondary_round_trips_through_the_request_body() {
        let json = serde_json::json!({
            "name": "n",
            "type": "Object storage",
            "direction": "sink",
            "host": "http://rustfs:9000|bucket",
            "credential": { "source": "env", "primary": "access_key", "secondary": "secret_key" },
            "environment": "production",
            "tenant": "t",
        });
        let body: CreateConnectorBody = serde_json::from_value(json).unwrap();
        assert_eq!(
            body.credential.secondary,
            Some(connectors::CredentialKind::SecretKey)
        );
    }

    /// Absent `credential.secondary` (e.g. a `PostgreSQL` connector, which
    /// only ever needs one credential) must still parse.
    #[test]
    fn credential_secondary_is_optional() {
        let json = serde_json::json!({
            "name": "n",
            "type": "PostgreSQL",
            "direction": "bidirectional",
            "host": "u@host:5432/db",
            "credential": { "source": "env", "primary": "password" },
            "environment": "production",
            "tenant": "t",
        });
        let body: CreateConnectorBody = serde_json::from_value(json).unwrap();
        assert_eq!(body.credential.secondary, None);
    }

    /// The regression guard this check exists for (ADR 0002 Addendum 3):
    /// a body still naming the pre-addendum `secretRef`/`secretRefSecondary`
    /// fields is refused, citing the ADR, rather than silently ignored
    /// (`CreateConnectorBody` has no field to deserialize either name
    /// into, so without this check they would simply vanish).
    #[test]
    fn create_refuses_a_body_still_naming_legacy_secret_ref_fields() {
        let body = Bytes::from(
            serde_json::json!({
                "name": "n",
                "type": "PostgreSQL",
                "direction": "source",
                "host": "h",
                "secretRef": "env:MY_SECRET",
                "environment": "production",
                "tenant": "t",
            })
            .to_string(),
        );
        let err = reject_legacy_secret_ref_fields(&body).unwrap_err();
        assert!(err.to_string().contains("ADR 0002 Addendum 3"), "{err}");
    }

    /// Same refusal for the secondary field alone.
    #[test]
    fn create_refuses_a_body_still_naming_legacy_secret_ref_secondary_field() {
        let body = Bytes::from(
            serde_json::json!({
                "name": "n",
                "type": "Object storage",
                "direction": "sink",
                "host": "h",
                "credential": { "source": "env", "primary": "access_key" },
                "secretRefSecondary": "env:MY_SECRET_2",
                "environment": "production",
                "tenant": "t",
            })
            .to_string(),
        );
        assert!(reject_legacy_secret_ref_fields(&body).is_err());
    }

    /// An ordinary body with no legacy field passes this check.
    #[test]
    fn create_accepts_a_body_with_no_legacy_secret_ref_fields() {
        let body = Bytes::from(
            serde_json::json!({
                "name": "n",
                "type": "PostgreSQL",
                "direction": "source",
                "host": "h",
                "credential": { "source": "env", "primary": "password" },
                "environment": "production",
                "tenant": "t",
            })
            .to_string(),
        );
        assert!(reject_legacy_secret_ref_fields(&body).is_ok());
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

    // ── `GET /api/connectors` tenant scoping ────────────────────────────

    fn principal_with_tenants(tenant_ids: &[Uuid]) -> Principal {
        Principal {
            tenant_ids: tenant_ids.to_vec(),
            ..fixture_user_principal()
        }
    }

    fn headers_with_x_tenant(tenant_id: Uuid) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-tenant",
            tenant_id
                .to_string()
                .parse()
                .expect("uuid renders as a valid header value"),
        );
        headers
    }

    /// A principal that belongs to zero tenants gets
    /// an EMPTY list — never `403`/`404`, and never every tenant's rows.
    /// `state_without_pool()` proves this is returned BEFORE the store is
    /// ever queried: reaching `pool(&state)?` here would 503, not 200 with
    /// an empty body.
    #[tokio::test]
    async fn list_with_a_tenantless_principal_returns_an_empty_list_before_touching_the_store() {
        let state = state_without_pool();
        let principal = principal_with_tenants(&[]);
        let result = list(
            State(state),
            Some(Extension(principal)),
            axum::http::HeaderMap::new(),
        )
        .await
        .expect("a tenantless principal must get Ok(empty list), not an error");
        assert!(
            result.0.is_empty(),
            "Ok(None) from tenant_scope::resolve must render as an empty list, never every \
             tenant's connectors"
        );
    }

    /// No `Extension<Principal>` at all (the internal, non-HTTP call path
    /// `routes::ai::tools::connectors` used to take before this task) is
    /// refused with 401, not a panic or an unscoped read.
    #[tokio::test]
    async fn list_with_no_principal_extension_is_unauthorized() {
        let state = state_without_pool();
        let err = list(State(state), None, axum::http::HeaderMap::new())
            .await
            .expect_err("no principal must be refused");
        assert_eq!(err.0.status(), 401);
    }

    /// `X-Tenant` naming a tenant the principal does not belong to is a
    /// 404 (via `tenant_scope::resolve`'s own contract), propagated
    /// through this route's own `?` rather than swallowed.
    #[tokio::test]
    async fn list_with_a_foreign_x_tenant_header_is_not_found() {
        let state = state_without_pool();
        let principal = principal_with_tenants(&[Uuid::from_u128(1)]);
        let headers = headers_with_x_tenant(Uuid::from_u128(2)); // not a member
        let err = list(State(state), Some(Extension(principal)), headers)
            .await
            .expect_err("a foreign X-Tenant must be refused");
        assert_eq!(err.0.status(), 404);
    }

    // ── PATCH validation (pure) ──────────────────────────────────────────

    fn update_body(json: serde_json::Value) -> UpdateConnectorBody {
        serde_json::from_value(json).expect("valid PATCH body")
    }

    #[test]
    fn update_input_keeps_only_the_fields_sent() {
        let input = update_input(update_body(
            json!({ "name": " renamed ", "residency": "id" }),
        ))
        .unwrap();
        assert_eq!(input.name.as_deref(), Some("renamed"));
        assert_eq!(input.residency.as_deref(), Some("id"));
        assert!(input.direction.is_none() && input.environment.is_none() && input.host.is_none());
    }

    #[test]
    fn update_input_refuses_blank_fields_bad_directions_and_empty_updates() {
        for body in [
            json!({ "name": "  " }),
            json!({ "direction": "sideways" }),
            json!({}),
        ] {
            assert!(update_input(update_body(body.clone())).is_err(), "{body}");
        }
    }

    /// Type, tenant, credential and dial each have their own route; naming
    /// one in a PATCH gets a pointer to it, not a silent drop.
    #[test]
    fn patch_refuses_fields_owned_by_other_routes_with_a_pointer() {
        for (body, route) in [
            (json!({ "type": "MySQL" }), "create a new connector"),
            (json!({ "tenantId": "x" }), "/tenant"),
            (json!({ "credential": {} }), "/credential"),
            (json!({ "dial": {} }), "/ingest-spec"),
        ] {
            let bytes = Bytes::from(serde_json::to_vec(&body).unwrap());
            let Err(ApiError::BadRequest(message)) = reject_non_patchable_fields(&bytes) else {
                panic!("{body} must be refused");
            };
            assert!(message.contains(route), "{message}");
        }
        let fine = Bytes::from(serde_json::to_vec(&json!({ "name": "n" })).unwrap());
        assert!(reject_non_patchable_fields(&fine).is_ok());
    }

    mod create_and_update_routes {
        use lakehouse_auth::{PermissionSet, PrincipalId};
        use lakehouse_store::identity::{self, CreateTenantInput};

        use super::super::*;
        use crate::config::Config;

        fn database_url_for(pool: &sqlx::PgPool) -> String {
            let options = pool.connect_options();
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database"),
            )
        }

        fn state_for(pool: &sqlx::PgPool) -> AppState {
            let mut env = std::collections::HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
            AppState::new(Config::from_map(&env).expect("a valid test Config"))
        }

        async fn seed_tenant(pool: &sqlx::PgPool, name: &str, slug: &str) -> uuid::Uuid {
            identity::create_tenant(
                pool,
                &CreateTenantInput {
                    name: name.to_owned(),
                    slug: slug.to_owned(),
                    plan: "Standard".to_owned(),
                    residency: "ID".to_owned(),
                },
            )
            .await
            .expect("create a test tenant")
            .id
            .parse()
            .expect("create_tenant returns a UUID-shaped id")
        }

        fn principal_in(tenant_ids: &[uuid::Uuid]) -> Extension<Principal> {
            Extension(Principal {
                id: PrincipalId::User(uuid::Uuid::nil()),
                tenant_ids: tenant_ids.to_vec(),
                display_name: "Creator".to_owned(),
                permissions: PermissionSet::parse("connector:manage"),
                provider: "local".to_owned(),
                must_change_password: false,
                role_names: Vec::new(),
            })
        }

        /// An operator-provisioned (`env`) credential, so these tests never
        /// touch the managed store's fixed `/run/secrets` directory.
        fn create_body(name: &str, tenant_id: Option<uuid::Uuid>) -> Bytes {
            let mut body = json!({
                "name": name, "type": "PostgreSQL", "direction": "source",
                "host": "db.internal",
                "credential": { "source": "env", "primary": "password" },
                "environment": "staging", "tenant": "typed by the user", "residency": "ID",
            });
            if let Some(tenant_id) = tenant_id {
                body["tenantId"] = json!(tenant_id.to_string());
            }
            Bytes::from(serde_json::to_vec(&body).expect("serialize"))
        }

        /// The bug this closes: a console-created connector used to have
        /// `tenant_id = NULL` and was invisible to everyone, its creator
        /// included. It now lands in the caller's active tenant, named
        /// after the tenant row — not whatever text was typed.
        #[sqlx::test(migrations = "../../migrations")]
        async fn create_lands_in_the_callers_active_tenant(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let tenant = seed_tenant(&pool, "Acme Co", "acme-create").await;

            let (status, ApiJson(created)) = create(
                State(state),
                Some(principal_in(&[tenant])),
                HeaderMap::new(),
                create_body("visible to its creator", None),
            )
            .await
            .expect("create should succeed");
            assert_eq!(status, StatusCode::CREATED);
            assert_eq!(created.connector.tenant, "Acme Co");

            let listed = connectors::list_connectors(
                &pool,
                &connectors::ConnectorFilter {
                    tenant_id: Some(tenant),
                },
            )
            .await
            .expect("list");
            assert!(listed.iter().any(|c| c.id == created.connector.id));
        }

        /// A tenant the caller does not belong to is refused as "not
        /// found", and no connector row is left behind.
        #[sqlx::test(migrations = "../../migrations")]
        async fn create_refuses_a_tenant_the_caller_is_not_in(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let mine = seed_tenant(&pool, "Mine", "mine").await;
            let theirs = seed_tenant(&pool, "Theirs", "theirs").await;

            let err = create(
                State(state),
                Some(principal_in(&[mine])),
                HeaderMap::new(),
                create_body("smuggled", Some(theirs)),
            )
            .await
            .expect_err("a foreign tenant must be refused");
            assert_eq!(err.0.status(), 404);
            let all = connectors::list_connectors(&pool, &connectors::ConnectorFilter::default())
                .await
                .expect("list");
            assert!(all.iter().all(|c| c.name != "smuggled"));
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn patch_renames_and_the_detail_reads_the_new_values_back(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let tenant = seed_tenant(&pool, "Acme Co", "acme-patch").await;
            let (_, ApiJson(created)) = create(
                State(state.clone()),
                Some(principal_in(&[tenant])),
                HeaderMap::new(),
                create_body("before rename", None),
            )
            .await
            .expect("create");
            let id = created.connector.id.clone();

            let ApiJson(updated) = update(
                State(state.clone()),
                Some(principal_in(&[tenant])),
                Path(id.clone()),
                Bytes::from_static(br#"{"name":"after rename","residency":"id-jakarta"}"#),
            )
            .await
            .expect("patch should succeed");
            assert_eq!(updated.name, "after rename");

            let ApiJson(detail) = super::super::detail(State(state), Path(id))
                .await
                .expect("detail");
            assert_eq!(detail.residency, "id-jakarta");
            assert_eq!(detail.tenant_id, Some(tenant.to_string()));
        }
    }

    /// `PUT /api/connectors/{id}/tenant` route tests.
    mod assign_tenant_route {
        use lakehouse_store::identity::{self, CreateTenantInput};

        use super::super::*;
        use crate::config::Config;

        fn database_url_for(pool: &sqlx::PgPool) -> String {
            let options = pool.connect_options();
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database"),
            )
        }

        fn state_for(pool: &sqlx::PgPool) -> AppState {
            let mut env = std::collections::HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        async fn seed_tenant(pool: &sqlx::PgPool, slug: &str) -> uuid::Uuid {
            let tenant = identity::create_tenant(
                pool,
                &CreateTenantInput {
                    name: "Acme Co".to_owned(),
                    slug: slug.to_owned(),
                    plan: "Standard".to_owned(),
                    residency: "US".to_owned(),
                },
            )
            .await
            .expect("create a test tenant");
            tenant
                .id
                .parse()
                .expect("create_tenant returns a UUID-shaped id")
        }

        /// A connector with `tenant_id = NULL` — `create_connector` never
        /// sets it, matching every connector a real deployment creates
        /// after `0042_tenant_provisioning.sql` (whose own backfill only
        /// touches the two seeded rows).
        async fn seed_connector_with_null_tenant(pool: &sqlx::PgPool, name: &str) -> String {
            let created = connectors::create_connector(
                pool,
                &connectors::CreateConnectorInput {
                    name: name.to_owned(),
                    kind: "PostgreSQL CDC".to_owned(),
                    direction: "source".to_owned(),
                    host: "db.internal:5432".to_owned(),
                    credential: connectors::CredentialSpec {
                        source: connectors::CredentialSource::Env,
                        primary: connectors::CredentialKind::Password,
                        secondary: None,
                    },
                    environment: "staging".to_owned(),
                    tenant: "Acme Co".to_owned(),
                    residency: "US".to_owned(),
                    capabilities: Vec::new(),
                    owner: None,
                },
            )
            .await
            .expect("create a test connector");
            let (connector, _credential_names) = created;
            connector.id
        }

        fn assign_body(tenant_id: uuid::Uuid) -> Bytes {
            Bytes::from(
                serde_json::to_vec(&json!({ "tenantId": tenant_id.to_string() }))
                    .expect("serialize"),
            )
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn assigns_and_is_then_visible_in_the_scoped_list(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let tenant_id = seed_tenant(&pool, "c6-assign").await;
            let connector_id = seed_connector_with_null_tenant(&pool, "c6-connector").await;

            let status = assign_connector_tenant(
                State(state),
                Path(connector_id.clone()),
                assign_body(tenant_id),
            )
            .await
            .expect("assignment should succeed");
            assert_eq!(status, StatusCode::NO_CONTENT);

            let rows = connectors::list_connectors(
                &pool,
                &connectors::ConnectorFilter {
                    tenant_id: Some(tenant_id),
                },
            )
            .await
            .expect("list_connectors should succeed");
            assert!(
                rows.iter().any(|r| r.id == connector_id),
                "the assigned connector must now be visible in its tenant's scoped list"
            );
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn refuses_an_unknown_tenant_id_with_404(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let connector_id = seed_connector_with_null_tenant(&pool, "c6-orphan").await;
            let unknown_tenant = uuid::Uuid::new_v4();

            let err = assign_connector_tenant(
                State(state),
                Path(connector_id),
                assign_body(unknown_tenant),
            )
            .await
            .expect_err("an unknown tenant id must be refused");
            assert_eq!(err.0.status(), 404);
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn refuses_an_unknown_connector_id_with_404(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let tenant_id = seed_tenant(&pool, "c6-no-connector").await;

            let err = assign_connector_tenant(
                State(state),
                Path("does-not-exist".to_owned()),
                assign_body(tenant_id),
            )
            .await
            .expect_err("an unknown connector id must be refused");
            assert_eq!(err.0.status(), 404);
        }
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

    // ── ADR 0002 Addendum 4: user-supplied (managed) credentials ────────

    fn values_body(json: serde_json::Value) -> CredentialValuesBody {
        serde_json::from_value(json).expect("valid credential.values body")
    }

    #[test]
    fn credential_values_are_optional() {
        let result = credential_values(connectors::CredentialSource::Managed, false, None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn managed_credential_values_are_accepted() {
        let result = credential_values(
            connectors::CredentialSource::Managed,
            true,
            Some(values_body(
                json!({ "primary": "access", "secondary": "secret" }),
            )),
        )
        .unwrap()
        .expect("values present");
        assert_eq!(result.primary.expose_secret(), "access");
        assert_eq!(result.secondary.unwrap().expose_secret(), "secret");
    }

    /// A value sent with an operator-provisioned source would be silently
    /// dropped (the API never writes env/file credentials) — refused.
    #[test]
    fn values_with_an_operator_source_are_refused() {
        for source in [
            connectors::CredentialSource::Env,
            connectors::CredentialSource::File,
        ] {
            let err =
                credential_values(source, false, Some(values_body(json!({ "primary": "pw" }))))
                    .err()
                    .expect("must be refused");
            assert!(matches!(err, ApiError::BadRequest(_)), "{source:?}");
        }
    }

    #[test]
    fn a_secondary_value_without_a_secondary_slot_is_refused() {
        let err = credential_values(
            connectors::CredentialSource::Managed,
            false,
            Some(values_body(json!({ "primary": "pw", "secondary": "x" }))),
        )
        .err()
        .expect("must be refused");
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    /// A value the resolvers would read back trimmed is refused before any
    /// row is created, and the error names the slot but never the value.
    #[test]
    fn a_padded_value_is_refused_without_echoing_it() {
        let err = credential_values(
            connectors::CredentialSource::Managed,
            false,
            Some(values_body(json!({ "primary": " hunter2 " }))),
        )
        .err()
        .expect("must be refused");
        let ApiError::BadRequest(message) = err else {
            panic!("expected 400");
        };
        assert!(message.contains("credential.values.primary"), "{message}");
        assert!(!message.contains("hunter2"), "{message}");
    }

    #[test]
    fn an_unknown_values_slot_is_refused() {
        let parsed: Result<CredentialValuesBody, _> =
            serde_json::from_value(json!({ "primary": "pw", "tertiary": "x" }));
        assert!(parsed.is_err());
    }

    /// Every request body that carries a credential derives `Debug`; none
    /// of them may print the value.
    #[test]
    fn debug_of_a_body_carrying_a_credential_never_prints_it() {
        let create: CreateConnectorBody = serde_json::from_value(json!({
            "name": "n", "type": "PostgreSQL", "direction": "source", "host": "db.internal",
            "credential": {
                "source": "managed", "primary": "password",
                "values": { "primary": "hunter2-create" }
            },
            "environment": "production", "tenant": "t"
        }))
        .unwrap();
        let set: SetCredentialBody = serde_json::from_value(json!({
            "primary": { "kind": "password", "value": "hunter2-set" }
        }))
        .unwrap();
        let rendered = format!("{create:?} {set:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn set_credential_body_refuses_unknown_fields() {
        for body in [
            json!({ "primary": { "kind": "password", "value": "pw" }, "source": "env" }),
            json!({ "primary": { "kind": "password", "value": "pw", "slot": "primary" } }),
        ] {
            let parsed: Result<SetCredentialBody, _> = serde_json::from_value(body.clone());
            assert!(parsed.is_err(), "{body}");
        }
    }
}
