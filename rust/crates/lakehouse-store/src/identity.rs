//! Repository layer for the identity domain: tenants, users, roles, and
//! service identities.
//!
//! # What this module is (and is not)
//!
//! It is the Postgres-backed replacement for `src/services/mock/identity.ts`
//! — identity *as data the console manages*. It is emphatically NOT an
//! authentication system: there is no password, credential, session, or
//! secret material anywhere in this module or in the `0001_init` schema it
//! reads. `app_user` is a directory row; `service_identity` records that a
//! service credential exists (name/scopes/expiry), never the credential
//! itself.
//!
//! # Shapes are the contract, not an implementation detail
//!
//! The `Serialize` impls here are the wire format the browser consumes:
//! every struct below mirrors a type in `src/services/contracts/identity.ts`
//! field for field, with `#[serde(rename_all = "camelCase")]` producing the
//! exact `lastActivity` / `storageBytes` / `quotaCompute` / `rotationStatus`
//! / `lastUsedAt` keys that file declares. Renaming or reordering a field
//! here is a frontend-visible change.
//!
//! # Derived counts are computed, never stored
//!
//! `Tenant.users` and `Role.members` are `COUNT(*)` subqueries over
//! `app_user_tenant` / `app_user_role`, exactly as `0001_init.sql`'s header
//! comment requires — there is deliberately no column that could drift from
//! the join table that owns the count. `Tenant.agents` has no owning table
//! in any migration yet (the `agents` domain brings its own), so it is
//! reported as `0` rather than invented; see [`Tenant::agents`].
//!
//! # Read/create only
//!
//! `IdentityService` (the contract) declares four list methods, one
//! settings getter, and four creates — no update and no delete. The
//! functions below cover exactly that surface plus `get_*` (needed to
//! re-read a row after insert) and `delete_*` (needed to make a seeded or
//! test-created row removable, and exercised by
//! `tests/identity.rs`). No `update_*` exists because nothing can call one:
//! adding an unreachable mutation path to a service that has no
//! authorization layer would be strictly worse than not having it.

use serde::Serialize;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{PgPool, StoreError};

/// Render a timestamp the way JavaScript's `Date.prototype.toISOString`
/// does (UTC, exactly three fractional digits, `Z` suffix).
///
/// The contract types every timestamp as a bare `string`, and every value
/// the mock produced came from `toISOString()`. Matching that format keeps
/// the strings the console renders/parses byte-identical in shape to what
/// it received before this task, rather than switching it to
/// `time`'s default RFC 3339 rendering (which emits a `+00:00` offset and a
/// variable number of fractional digits). Mirrors
/// `lakehouse_dagster::iso_from_unix_seconds`, which exists for the same
/// reason on the `Dagster` side.
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

/// Parse a caller-supplied id string into a [`Uuid`].
///
/// Every id in this domain is a `UUID` column, but the contract types ids
/// as opaque `string`s — so a caller can hand us something that is not a
/// UUID at all. That is a "no such record", not a malformed-request error
/// or a 500: a well-formed request for an id that cannot exist gets the
/// same [`StoreError::NotFound`] as a well-formed UUID that happens to have
/// no row.
fn parse_id(id: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(id).map_err(|_| StoreError::NotFound)
}

// ── User ────────────────────────────────────────────────────────────────

/// A person in the workspace directory. Mirrors `User` in
/// `contracts/identity.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    /// `app_user.id`, rendered as a string (the contract types ids as
    /// opaque strings).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Email address; the table's natural key.
    pub email: String,
    /// `"active"` or `"inactive"` — the closed union the contract declares,
    /// enforced by `app_user_status_check`.
    pub status: String,
    /// Role *names* (not ids) this user holds, matching the contract's
    /// `roles: string[]`.
    pub roles: Vec<String>,
    /// Tenant *names* (not ids or slugs) this user belongs to, matching the
    /// contract's `tenants: string[]` and the mock fixtures, which use
    /// `"Meridian Group"` rather than `"meridian-group"`.
    pub tenants: Vec<String>,
    /// `app_user.last_activity_at`, ISO 8601. Serializes as `lastActivity`.
    pub last_activity: String,
}

/// The raw row shape [`list_users`]/[`get_user`] select, before
/// timestamps and ids are rendered as strings.
#[derive(Debug, FromRow)]
struct UserRow {
    id: Uuid,
    name: String,
    email: String,
    status: String,
    roles: Vec<String>,
    tenants: Vec<String>,
    last_activity_at: OffsetDateTime,
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        Self {
            id: row.id.to_string(),
            name: row.name,
            email: row.email,
            status: row.status,
            roles: row.roles,
            tenants: row.tenants,
            last_activity: iso_millis(row.last_activity_at),
        }
    }
}

/// Optional narrowing for [`list_users`]. Every field defaults to `None`
/// ("no filter"), so `UserFilter::default()` lists everyone.
#[derive(Debug, Clone, Default)]
pub struct UserFilter {
    /// Keep only users with this `status` (`"active"` / `"inactive"`).
    pub status: Option<String>,
    /// Keep only users who belong to the tenant with this slug.
    pub tenant_slug: Option<String>,
}

/// The columns and correlated subqueries every user read shares.
///
/// `roles`/`tenants` are built with correlated `ARRAY(SELECT ...)`
/// subqueries rather than a `LEFT JOIN ... GROUP BY`: a user with no roles
/// yields an empty array (not a `[null]` from `array_agg`), and the two
/// independent many-to-many relationships cannot multiply each other's row
/// counts the way two joins in one query would.
const USER_SELECT: &str = "SELECT u.id, u.name, u.email, u.status, u.last_activity_at, \
     ARRAY(SELECT r.name FROM app_user_role ur JOIN role r ON r.id = ur.role_id \
           WHERE ur.user_id = u.id ORDER BY r.name) AS roles, \
     ARRAY(SELECT t.name FROM app_user_tenant ut JOIN tenant t ON t.id = ut.tenant_id \
           WHERE ut.user_id = u.id ORDER BY t.name) AS tenants \
     FROM app_user u";

/// List users, newest first, optionally narrowed by [`UserFilter`].
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails (including Postgres
/// being unreachable — see [`crate::connect_lazy`]).
pub async fn list_users(pool: &PgPool, filter: &UserFilter) -> Result<Vec<User>, StoreError> {
    let sql = format!(
        "{USER_SELECT} \
         WHERE ($1::text IS NULL OR u.status = $1) \
           AND ($2::text IS NULL OR EXISTS ( \
                 SELECT 1 FROM app_user_tenant ut JOIN tenant t ON t.id = ut.tenant_id \
                 WHERE ut.user_id = u.id AND t.slug = $2)) \
         ORDER BY u.created_at DESC, u.name"
    );
    let rows: Vec<UserRow> = sqlx::query_as(&sql)
        .bind(filter.status.as_deref())
        .bind(filter.tenant_slug.as_deref())
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(User::from).collect())
}

/// Fetch one user by id.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if no such user exists (or `id` is not
/// a UUID at all), or [`StoreError::Database`] on any other failure.
pub async fn get_user(pool: &PgPool, id: &str) -> Result<User, StoreError> {
    let sql = format!("{USER_SELECT} WHERE u.id = $1");
    let row: UserRow = sqlx::query_as(&sql)
        .bind(parse_id(id)?)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// Everything [`create_user`] needs. Mirrors `InviteUserInput` in
/// `contracts/identity.ts`.
#[derive(Debug, Clone)]
pub struct InviteUserInput {
    /// Display name.
    pub name: String,
    /// Email address; must not collide with an existing user.
    pub email: String,
    /// Role *names* to grant. Every name must already exist in `role`.
    pub roles: Vec<String>,
    /// Tenant *names* to join. Every name must already exist in `tenant`.
    pub tenants: Vec<String>,
}

/// Create a user and its role/tenant memberships.
///
/// Runs in a transaction: a user whose role or tenant names cannot all be
/// resolved is not left half-created with a partial membership set.
///
/// # Errors
///
/// * [`StoreError::Conflict`] (409) — the email is already taken.
/// * [`StoreError::ForeignKeyViolation`] (400) — a role or tenant name in
///   `input` does not exist. Membership rows are inserted by *name*
///   (`INSERT ... SELECT id FROM role WHERE name = $2`), so an unknown name
///   inserts zero rows rather than raising a database-level FK error; the
///   `rows_affected` check below is what turns that silent no-op into the
///   same 400 a genuine FK violation produces.
/// * [`StoreError::Database`] (500) — anything else.
pub async fn create_user(pool: &PgPool, input: &InviteUserInput) -> Result<User, StoreError> {
    let mut tx = pool.begin().await?;

    let (id,): (Uuid,) =
        sqlx::query_as("INSERT INTO app_user (name, email) VALUES ($1, $2) RETURNING id")
            .bind(&input.name)
            .bind(&input.email)
            .fetch_one(&mut *tx)
            .await?;

    for role in &input.roles {
        let affected = sqlx::query(
            "INSERT INTO app_user_role (user_id, role_id) \
             SELECT $1, id FROM role WHERE name = $2 ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(role)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected == 0 {
            return Err(StoreError::ForeignKeyViolation);
        }
    }

    for tenant in &input.tenants {
        let affected = sqlx::query(
            "INSERT INTO app_user_tenant (user_id, tenant_id) \
             SELECT $1, id FROM tenant WHERE name = $2 ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(tenant)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected == 0 {
            return Err(StoreError::ForeignKeyViolation);
        }
    }

    tx.commit().await?;
    get_user(pool, &id.to_string()).await
}

/// Delete a user; its membership rows cascade away with it.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if no such user exists, or
/// [`StoreError::Database`] on any other failure.
pub async fn delete_user(pool: &PgPool, id: &str) -> Result<(), StoreError> {
    let affected = sqlx::query("DELETE FROM app_user WHERE id = $1")
        .bind(parse_id(id)?)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

// ── Role ────────────────────────────────────────────────────────────────

/// A named permission bundle. Mirrors `Role` in `contracts/identity.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Role {
    /// `role.id`, as a string.
    pub id: String,
    /// Role name; the table's natural key, and what `User.roles` holds.
    pub name: String,
    /// How many users hold this role. Computed as `COUNT(*)` over
    /// `app_user_role` — never a stored column.
    pub members: i64,
    /// Free-text permission list (e.g. `"query:read, catalog:read"`), typed
    /// as a single `string` by the contract.
    pub permissions: String,
    /// Human-readable description.
    pub description: String,
}

/// The raw row shape role reads select.
#[derive(Debug, FromRow)]
struct RoleRow {
    id: Uuid,
    name: String,
    members: i64,
    permissions: String,
    description: String,
}

impl From<RoleRow> for Role {
    fn from(row: RoleRow) -> Self {
        Self {
            id: row.id.to_string(),
            name: row.name,
            members: row.members,
            permissions: row.permissions,
            description: row.description,
        }
    }
}

/// The columns every role read shares, including the derived `members`
/// count.
const ROLE_SELECT: &str = "SELECT r.id, r.name, r.permissions, r.description, \
     (SELECT COUNT(*) FROM app_user_role ur WHERE ur.role_id = r.id) AS members \
     FROM role r";

/// List roles, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_roles(pool: &PgPool) -> Result<Vec<Role>, StoreError> {
    let sql = format!("{ROLE_SELECT} ORDER BY r.created_at DESC, r.name");
    let rows: Vec<RoleRow> = sqlx::query_as(&sql).fetch_all(pool).await?;
    Ok(rows.into_iter().map(Role::from).collect())
}

/// Fetch one role by id.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if no such role exists (or `id` is not
/// a UUID), or [`StoreError::Database`] on any other failure.
pub async fn get_role(pool: &PgPool, id: &str) -> Result<Role, StoreError> {
    let sql = format!("{ROLE_SELECT} WHERE r.id = $1");
    let row: RoleRow = sqlx::query_as(&sql)
        .bind(parse_id(id)?)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// Everything [`create_role`] needs. Mirrors `CreateRoleInput`.
#[derive(Debug, Clone)]
pub struct CreateRoleInput {
    /// Role name; must not collide with an existing role.
    pub name: String,
    /// Free-text permission list.
    pub permissions: String,
    /// Human-readable description.
    pub description: String,
}

/// Create a role. A freshly created role has no members, so the derived
/// `members` count comes back `0` without needing a special case.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken, or
/// [`StoreError::Database`] on any other failure.
pub async fn create_role(pool: &PgPool, input: &CreateRoleInput) -> Result<Role, StoreError> {
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO role (name, permissions, description) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&input.name)
    .bind(&input.permissions)
    .bind(&input.description)
    .fetch_one(pool)
    .await?;
    get_role(pool, &id.to_string()).await
}

/// Delete a role.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if no such role exists,
/// [`StoreError::ForeignKeyViolation`] if a user still holds it
/// (`app_user_role.role_id` is `ON DELETE RESTRICT`), or
/// [`StoreError::Database`] on any other failure.
pub async fn delete_role(pool: &PgPool, id: &str) -> Result<(), StoreError> {
    let affected = sqlx::query("DELETE FROM role WHERE id = $1")
        .bind(parse_id(id)?)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

// ── Tenant ──────────────────────────────────────────────────────────────

/// A customer workspace. Mirrors `Tenant` in `contracts/identity.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tenant {
    /// `tenant.id`, as a string.
    pub id: String,
    /// Display name, and what `User.tenants` holds.
    pub name: String,
    /// URL-safe identifier; unique.
    pub slug: String,
    /// Plan name (e.g. `"Enterprise"`), free text in the contract.
    pub plan: String,
    /// Residency policy description.
    pub residency: String,
    /// How many users belong to this tenant. Computed as `COUNT(*)` over
    /// `app_user_tenant` — never a stored column.
    pub users: i64,
    /// How many agents this tenant runs.
    ///
    /// Always `0` today, and deliberately so: no migration has created an
    /// agents table yet (`0001_init.sql` says the `agents` domain brings
    /// its own), so there is nothing to `COUNT(*)`. Reporting a real zero
    /// is honest; storing a hand-maintained number on `tenant` would be the
    /// exact stale-derived-count mistake the schema was designed to avoid.
    /// When the agents domain lands, this becomes a subquery like `users`
    /// and nothing else in this file changes.
    pub agents: i64,
    /// Bytes stored. Serializes as `storageBytes`.
    pub storage_bytes: i64,
    /// Compute quota. Serializes as `quotaCompute`.
    pub quota_compute: i64,
    /// Compute consumed. Serializes as `usedCompute`.
    pub used_compute: i64,
}

/// The raw row shape tenant reads select.
#[derive(Debug, FromRow)]
struct TenantRow {
    id: Uuid,
    name: String,
    slug: String,
    plan: String,
    residency: String,
    users: i64,
    storage_bytes: i64,
    quota_compute: i64,
    used_compute: i64,
}

impl From<TenantRow> for Tenant {
    fn from(row: TenantRow) -> Self {
        Self {
            id: row.id.to_string(),
            name: row.name,
            slug: row.slug,
            plan: row.plan,
            residency: row.residency,
            users: row.users,
            agents: 0,
            storage_bytes: row.storage_bytes,
            quota_compute: row.quota_compute,
            used_compute: row.used_compute,
        }
    }
}

/// The columns every tenant read shares, including the derived `users`
/// count.
const TENANT_SELECT: &str = "SELECT t.id, t.name, t.slug, t.plan, t.residency, \
     t.storage_bytes, t.quota_compute, t.used_compute, \
     (SELECT COUNT(*) FROM app_user_tenant ut WHERE ut.tenant_id = t.id) AS users \
     FROM tenant t";

/// Optional narrowing for [`list_tenants`].
#[derive(Debug, Clone, Default)]
pub struct TenantFilter {
    /// Keep only tenants on this plan.
    pub plan: Option<String>,
}

/// List tenants, newest first, optionally narrowed by [`TenantFilter`].
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_tenants(pool: &PgPool, filter: &TenantFilter) -> Result<Vec<Tenant>, StoreError> {
    let sql = format!(
        "{TENANT_SELECT} WHERE ($1::text IS NULL OR t.plan = $1) \
         ORDER BY t.created_at DESC, t.name"
    );
    let rows: Vec<TenantRow> = sqlx::query_as(&sql)
        .bind(filter.plan.as_deref())
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Tenant::from).collect())
}

/// Fetch one tenant by id.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if no such tenant exists (or `id` is
/// not a UUID), or [`StoreError::Database`] on any other failure.
pub async fn get_tenant(pool: &PgPool, id: &str) -> Result<Tenant, StoreError> {
    let sql = format!("{TENANT_SELECT} WHERE t.id = $1");
    let row: TenantRow = sqlx::query_as(&sql)
        .bind(parse_id(id)?)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// Everything [`create_tenant`] needs. Mirrors `CreateTenantInput`.
#[derive(Debug, Clone)]
pub struct CreateTenantInput {
    /// Display name.
    pub name: String,
    /// URL-safe identifier; must not collide with an existing tenant.
    pub slug: String,
    /// Plan name.
    pub plan: String,
    /// Residency policy description.
    pub residency: String,
}

/// The compute quota a brand-new tenant starts with.
///
/// Taken from `mock/identity.ts`'s `createTenant`, which seeded
/// `quotaCompute: 5000` (and zeroes for storage/usage, which are the
/// column defaults in `0001_init.sql`). Preserved so a tenant created
/// through the real backend lands on the same starting quota the console
/// has always shown after a create.
const DEFAULT_QUOTA_COMPUTE: i64 = 5000;

/// Create a tenant.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the slug is taken, or
/// [`StoreError::Database`] on any other failure.
pub async fn create_tenant(pool: &PgPool, input: &CreateTenantInput) -> Result<Tenant, StoreError> {
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO tenant (name, slug, plan, residency, quota_compute) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(&input.name)
    .bind(&input.slug)
    .bind(&input.plan)
    .bind(&input.residency)
    .bind(DEFAULT_QUOTA_COMPUTE)
    .fetch_one(pool)
    .await?;
    get_tenant(pool, &id.to_string()).await
}

/// Delete a tenant.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if no such tenant exists,
/// [`StoreError::ForeignKeyViolation`] if a user still belongs to it
/// (`app_user_tenant.tenant_id` is `ON DELETE RESTRICT`), or
/// [`StoreError::Database`] on any other failure.
pub async fn delete_tenant(pool: &PgPool, id: &str) -> Result<(), StoreError> {
    let affected = sqlx::query("DELETE FROM tenant WHERE id = $1")
        .bind(parse_id(id)?)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

// ── Service identity ────────────────────────────────────────────────────

/// A non-human principal (a service credential's *metadata*, never the
/// credential). Mirrors `ServiceIdentity` in `contracts/identity.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceIdentity {
    /// `service_identity.id`, as a string.
    pub id: String,
    /// Credential name; the table's natural key.
    pub name: String,
    /// Granted scopes (e.g. `["query:read", "catalog:read"]`).
    pub scopes: Vec<String>,
    /// Deployment environment (e.g. `"production"`).
    pub environment: String,
    /// When the credential expires, ISO 8601. Serializes as `expiresAt`.
    pub expires_at: String,
    /// `"current"`, `"due"`, or `"expired"` — the closed union the contract
    /// declares. Serializes as `rotationStatus`.
    pub rotation_status: String,
    /// Last time the credential was seen in use, ISO 8601. Serializes as
    /// `lastUsedAt`.
    pub last_used_at: String,
}

/// The raw row shape service-identity reads select.
#[derive(Debug, FromRow)]
struct ServiceIdentityRow {
    id: Uuid,
    name: String,
    scopes: Vec<String>,
    environment: String,
    expires_at: OffsetDateTime,
    rotation_status: String,
    last_used_at: OffsetDateTime,
}

impl From<ServiceIdentityRow> for ServiceIdentity {
    fn from(row: ServiceIdentityRow) -> Self {
        Self {
            id: row.id.to_string(),
            name: row.name,
            scopes: row.scopes,
            environment: row.environment,
            expires_at: iso_millis(row.expires_at),
            rotation_status: row.rotation_status,
            last_used_at: iso_millis(row.last_used_at),
        }
    }
}

/// The columns every service-identity read shares.
const SERVICE_IDENTITY_SELECT: &str = "SELECT s.id, s.name, s.scopes, s.environment, s.expires_at, s.rotation_status, \
     s.last_used_at FROM service_identity s";

/// Optional narrowing for [`list_service_identities`].
#[derive(Debug, Clone, Default)]
pub struct ServiceIdentityFilter {
    /// Keep only identities in this environment.
    pub environment: Option<String>,
}

/// List service identities, newest first, optionally narrowed by
/// [`ServiceIdentityFilter`].
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_service_identities(
    pool: &PgPool,
    filter: &ServiceIdentityFilter,
) -> Result<Vec<ServiceIdentity>, StoreError> {
    let sql = format!(
        "{SERVICE_IDENTITY_SELECT} WHERE ($1::text IS NULL OR s.environment = $1) \
         ORDER BY s.created_at DESC, s.name"
    );
    let rows: Vec<ServiceIdentityRow> = sqlx::query_as(&sql)
        .bind(filter.environment.as_deref())
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(ServiceIdentity::from).collect())
}

/// Fetch one service identity by id.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if no such identity exists (or `id` is
/// not a UUID), or [`StoreError::Database`] on any other failure.
pub async fn get_service_identity(pool: &PgPool, id: &str) -> Result<ServiceIdentity, StoreError> {
    let sql = format!("{SERVICE_IDENTITY_SELECT} WHERE s.id = $1");
    let row: ServiceIdentityRow = sqlx::query_as(&sql)
        .bind(parse_id(id)?)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// Everything [`create_service_identity`] needs. Mirrors
/// `CreateServiceIdentityInput` — note it carries no secret, because the
/// console never had one to send.
#[derive(Debug, Clone)]
pub struct CreateServiceIdentityInput {
    /// Credential name; must not collide with an existing identity.
    pub name: String,
    /// Granted scopes.
    pub scopes: Vec<String>,
    /// Deployment environment.
    pub environment: String,
}

/// How long a newly created service identity is valid for, in days.
///
/// `mock/identity.ts`'s `createServiceIdentity` stamped `expiresAt` 90 days
/// out (`agoIso(-60 * 24 * 90)`) with `rotationStatus: "current"`; this
/// preserves that. The value lives here, in application code, precisely as
/// `0001_init.sql`'s header comment anticipated — the schema stores
/// `rotation_status` rather than deriving it, so rotation policy can change
/// without a migration.
const NEW_IDENTITY_VALIDITY_DAYS: i64 = 90;

/// Create a service identity, valid for [`NEW_IDENTITY_VALIDITY_DAYS`] and
/// marked `"current"`.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken, or
/// [`StoreError::Database`] on any other failure.
pub async fn create_service_identity(
    pool: &PgPool,
    input: &CreateServiceIdentityInput,
) -> Result<ServiceIdentity, StoreError> {
    let expires_at = OffsetDateTime::now_utc() + time::Duration::days(NEW_IDENTITY_VALIDITY_DAYS);
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO service_identity (name, scopes, environment, rotation_status, expires_at) \
         VALUES ($1, $2, $3, 'current', $4) RETURNING id",
    )
    .bind(&input.name)
    .bind(&input.scopes)
    .bind(&input.environment)
    .bind(expires_at)
    .fetch_one(pool)
    .await?;
    get_service_identity(pool, &id.to_string()).await
}

/// Delete a service identity.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if no such identity exists, or
/// [`StoreError::Database`] on any other failure.
pub async fn delete_service_identity(pool: &PgPool, id: &str) -> Result<(), StoreError> {
    let affected = sqlx::query("DELETE FROM service_identity WHERE id = $1")
        .bind(parse_id(id)?)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// The wire format is the contract: every key the browser reads must be
    /// the camelCase name `contracts/identity.ts` declares, not the
    /// `snake_case` Rust field name. This is the regression test for a
    /// `rename_all` attribute being dropped from any struct above — a
    /// change that compiles cleanly and silently breaks the console.
    #[test]
    fn serialized_field_names_match_the_typescript_contract() {
        let user = User {
            id: "u".to_owned(),
            name: "Rina".to_owned(),
            email: "rina@meridian.example".to_owned(),
            status: "active".to_owned(),
            roles: vec!["Analyst".to_owned()],
            tenants: vec!["Meridian Group".to_owned()],
            last_activity: "2026-01-01T00:00:00.000Z".to_owned(),
        };
        let value = serde_json::to_value(&user).unwrap();
        for key in [
            "id",
            "name",
            "email",
            "status",
            "roles",
            "tenants",
            "lastActivity",
        ] {
            assert!(value.get(key).is_some(), "User is missing `{key}`");
        }

        let tenant = Tenant {
            id: "t".to_owned(),
            name: "Meridian Group".to_owned(),
            slug: "meridian-group".to_owned(),
            plan: "Enterprise".to_owned(),
            residency: "Jakarta (ID)".to_owned(),
            users: 3,
            agents: 0,
            storage_bytes: 1,
            quota_compute: 2,
            used_compute: 3,
        };
        let value = serde_json::to_value(&tenant).unwrap();
        for key in [
            "id",
            "name",
            "slug",
            "plan",
            "residency",
            "users",
            "agents",
            "storageBytes",
            "quotaCompute",
            "usedCompute",
        ] {
            assert!(value.get(key).is_some(), "Tenant is missing `{key}`");
        }

        let identity = ServiceIdentity {
            id: "s".to_owned(),
            name: "bi-dashboard-reader".to_owned(),
            scopes: vec!["query:read".to_owned()],
            environment: "production".to_owned(),
            expires_at: "2026-01-01T00:00:00.000Z".to_owned(),
            rotation_status: "current".to_owned(),
            last_used_at: "2026-01-01T00:00:00.000Z".to_owned(),
        };
        let value = serde_json::to_value(&identity).unwrap();
        for key in [
            "id",
            "name",
            "scopes",
            "environment",
            "expiresAt",
            "rotationStatus",
            "lastUsedAt",
        ] {
            assert!(
                value.get(key).is_some(),
                "ServiceIdentity is missing `{key}`"
            );
        }

        let role = Role {
            id: "r".to_owned(),
            name: "Analyst".to_owned(),
            members: 24,
            permissions: "query:read".to_owned(),
            description: "Read governed data.".to_owned(),
        };
        let value = serde_json::to_value(&role).unwrap();
        for key in ["id", "name", "members", "permissions", "description"] {
            assert!(value.get(key).is_some(), "Role is missing `{key}`");
        }
    }

    /// Timestamps must render the way `Date.prototype.toISOString` does —
    /// three fractional digits and a `Z`, not `time`'s default `+00:00`
    /// RFC 3339 form — because that is the shape the console has always
    /// parsed.
    #[test]
    fn iso_millis_matches_javascript_to_iso_string() {
        let at = OffsetDateTime::from_unix_timestamp(1_787_803_210)
            .unwrap()
            .replace_millisecond(75)
            .unwrap();
        assert_eq!(iso_millis(at), "2026-08-27T04:00:10.075Z");
    }

    /// A non-UUID id is a "no such record", not a 500: `parse_id` is what
    /// keeps a junk id from ever reaching Postgres as a type error.
    #[test]
    fn a_non_uuid_id_is_not_found_rather_than_a_database_error() {
        let err = parse_id("not-a-uuid").expect_err("must reject a non-UUID id");
        assert!(matches!(err, StoreError::NotFound), "got {err:?}");
    }
}
