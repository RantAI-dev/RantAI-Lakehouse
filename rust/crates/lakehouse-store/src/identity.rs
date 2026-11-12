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
//! # Read/create, and the ADM-4 mutations
//!
//! `IdentityService` (the contract) declares four list methods, one
//! settings getter, and four creates — no update beyond roles and no
//! delete. The functions below cover exactly that surface plus `get_*`
//! (needed to re-read a row after insert) and `delete_*` (needed to make a
//! seeded or test-created row removable, and exercised by `tests/identity.rs`).
//! `update_role` and `delete_role`'s console route are `WEB-3` S3a-D2.
//!
//! ADM-4 adds the mutations the console needs to change what a user may
//! do — a user's role set, a user's tenant membership, and a user's
//! status — as `*_in_tx` functions: the CALLER opens the transaction,
//! because the rule that guards the change (ADM-4 D5: a change that would
//! leave no active user holding `*:*` is refused, in the same transaction
//! as the change) has to see the change and the count together. The
//! authorization rules (who may make the change) live in
//! `lakehouse-api`'s routes, like every other route's rules; nothing here
//! decides who may call it.
//!
//! # ADM-11: usernames, and a case-insensitive uniqueness rule
//!
//! Every user now carries a required `username` (1 to 254 characters from
//! `a-z 0-9 . _ @ + -`, stored lowercase), and `email` is optional. One
//! validator ([`normalize_username`]) and one clash rule (ADM-11 D5: a
//! username may not equal another user's username OR email, and the
//! reverse — so one sign-in box can never match two people) are enforced
//! HERE, inside the caller's transaction, so invite, rename and the OIDC
//! just-in-time provisioning in `lakehouse-auth` all go through the same
//! rule instead of three private copies of it. The schema-level backstops
//! are `app_user_username_lower_key` (unique on `lower(username)`) and the
//! `app_user_username_format` CHECK (`0132_usernames.sql`).

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use sqlx::{Postgres, Transaction};
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
    /// The unique sign-in handle, stored lowercase (ADM-11 D3).
    pub username: String,
    /// Email address; optional since ADM-11 (D2.4). `None` when the
    /// account has none — senders skip such an account and say so (D9).
    pub email: Option<String>,
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
    /// `app_user.last_activity_at`. Always `None`: the column defaults to
    /// `now()` at insert time (`0001_init.sql:68`) but nothing in this
    /// workspace — no login, session, or token use — ever updates it (J18),
    /// so serving the stored value would report a seeded user as "active"
    /// forever and a brand-new user as active at the moment of creation.
    /// Serializes as `lastActivity`.
    pub last_activity: Option<String>,
}

/// The raw row shape [`list_users`]/[`get_user`] select, before
/// ids are rendered as strings.
///
/// `last_activity_at` is deliberately not selected (see
/// [`User::last_activity`]'s doc comment), so there is no column here to
/// decode it into.
#[derive(Debug, FromRow)]
struct UserRow {
    id: Uuid,
    name: String,
    username: String,
    email: Option<String>,
    status: String,
    roles: Vec<String>,
    tenants: Vec<String>,
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        Self {
            id: row.id.to_string(),
            name: row.name,
            username: row.username,
            email: row.email,
            status: row.status,
            roles: row.roles,
            tenants: row.tenants,
            last_activity: None,
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
const USER_SELECT: &str = "SELECT u.id, u.name, u.username, u.email, u.status, \
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
    // safe: only the constant `USER_SELECT` is formatted in; values are bound.
    let rows: Vec<UserRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
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
    // safe: only the constant `USER_SELECT` is formatted in; values are bound.
    let row: UserRow = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(parse_id(id)?)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// Everything [`create_user`] needs. Mirrors the invite body of
/// `POST /api/identity/users` (ADM-11 D7).
#[derive(Debug, Clone)]
pub struct InviteUserInput {
    /// Display name.
    pub name: String,
    /// The unique sign-in handle. Lowercased and checked against the
    /// ADM-11 D3 rule here; refused with
    /// [`StoreError::FieldConflict`] when another user already holds it
    /// (or its email) as their own username or email (D5).
    pub username: String,
    /// Email address; optional since ADM-11 (D2.4). When present, refused
    /// with [`StoreError::FieldConflict`] if it equals (case-insensitively)
    /// another user's email or username (D5).
    pub email: Option<String>,
    /// Role *names* to grant. Every name must already exist in `role`.
    pub roles: Vec<String>,
    /// Tenant *names* to join. Every name must already exist in `tenant`.
    pub tenants: Vec<String>,
}

/// The one sentence the username validator answers with, naming the ADM-11
/// D3 rule. The [`StoreError::Validation`] message is safe to surface, so
/// an invite or rename form can show it verbatim.
pub(crate) const USERNAME_RULE: &str =
    "Username must be 1 to 254 characters from a-z, 0-9, dot, underscore, at, plus and hyphen.";

/// Lowercase, trim, and check `raw` against the ADM-11 D3 rule: 1 to 254
/// characters from `a-z 0-9 . _ @ + -`, stored lowercase.
///
/// This is THE validator (plan D3): the invite route, the rename route and
/// the OIDC just-in-time provisioning in `lakehouse-auth` all call it, so
/// there is no second copy of the character-set rule anywhere. The
/// 254-character bound mirrors `app_user_username_format`'s CHECK
/// (`0132_usernames.sql`), and counting characters (not bytes) matches
/// `char_length` on the database side.
///
/// # Errors
///
/// [`StoreError::Validation`] (HTTP 400) with [`USERNAME_RULE`] when the
/// value is empty, longer than 254 characters, or carries a character
/// outside the set.
#[must_use = "the normalized username is the value to store"]
pub fn normalize_username(raw: &str) -> Result<String, StoreError> {
    let lowered = raw.trim().to_lowercase();
    if lowered.is_empty()
        || lowered.chars().count() > 254
        || !lowered
            .chars()
            .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '.' | '_' | '@' | '+' | '-'))
    {
        return Err(StoreError::Validation(USERNAME_RULE.to_owned()));
    }
    Ok(lowered)
}

/// Trim the email a caller wants to store, keeping `None` as "no email"
/// (ADM-11 D2.4) and refusing an empty string: an empty address is a
/// typo, not an absence, and storing it would fabricate a value.
///
/// # Errors
///
/// [`StoreError::Validation`] (HTTP 400) when the value is present but
/// blank.
fn normalize_email(raw: Option<&str>) -> Result<Option<String>, StoreError> {
    match raw {
        None => Ok(None),
        Some(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Err(StoreError::Validation(
                    "Email must not be empty.".to_owned(),
                ));
            }
            Ok(Some(trimmed.to_owned()))
        }
    }
}

/// Whether ANOTHER user (never `exclude`) already holds `username` as their
/// username or email, or `email` as their email or username — all compared
/// case-insensitively (ADM-11 D5). One statement, run inside the caller's
/// transaction, so a concurrent invite or rename that commits first makes
/// this one see it.
///
/// `username` must already be [`normalize_username`]'s output (lowercase);
/// `email` is compared through `lower()` here.
///
/// # Errors
///
/// [`StoreError::Database`] on any storage failure.
async fn username_email_taken_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    username: &str,
    email: Option<&str>,
    exclude: Option<Uuid>,
) -> Result<(bool, bool), StoreError> {
    let row: (bool, bool) = sqlx::query_as(
        "SELECT \
           EXISTS (SELECT 1 FROM app_user o \
                   WHERE ($1::uuid IS NULL OR o.id <> $1) \
                     AND (lower(o.username) = $2 \
                          OR (o.email IS NOT NULL AND lower(o.email) = $2))), \
           EXISTS (SELECT 1 FROM app_user o \
                   WHERE ($1::uuid IS NULL OR o.id <> $1) \
                     AND $3::text IS NOT NULL \
                     AND (lower(o.username) = lower($3) \
                          OR (o.email IS NOT NULL AND lower(o.email) = lower($3))))",
    )
    .bind(exclude)
    .bind(username)
    .bind(email)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row)
}

/// Whether ANOTHER user holds `candidate` (case-insensitively) as their
/// username or email — the single-question form of
/// [`username_email_taken_in_tx`] that [`next_free_username_in_tx`]'s
/// suffix loop asks. `candidate` must already be lowercase.
///
/// # Errors
///
/// [`StoreError::Database`] on any storage failure.
async fn identifier_taken_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    candidate: &str,
    exclude: Option<Uuid>,
) -> Result<bool, StoreError> {
    let (taken,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM app_user o \
         WHERE ($1::uuid IS NULL OR o.id <> $1) \
           AND (lower(o.username) = $2 \
                OR (o.email IS NOT NULL AND lower(o.email) = $2)))",
    )
    .bind(exclude)
    .bind(candidate)
    .fetch_one(&mut **tx)
    .await?;
    Ok(taken)
}

/// The first username built on `base` that no other user holds: `base`
/// itself when free, else `base-2`, `base-3`, … — the same suffix rule the
/// `0132_usernames.sql` backfill applies to case-variant emails (ADM-11
/// D10: an OIDC-provisioned user whose preferred handle clashes gets the
/// next free one instead of an error). The answer is guaranteed to pass
/// [`normalize_username`]'s length bound only when `base` does with room
/// for the suffix; a `base` at the 254-character limit can overflow it and
/// is refused like any other invalid username.
///
/// `base` must already be [`normalize_username`]'s output (lowercase).
///
/// # Errors
///
/// [`StoreError::Validation`] when a suffixed candidate would break the
/// D3 length bound, or [`StoreError::Database`] on any storage failure.
pub async fn next_free_username_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    base: &str,
    exclude: Option<Uuid>,
) -> Result<String, StoreError> {
    if !identifier_taken_in_tx(tx, base, exclude).await? {
        return Ok(base.to_owned());
    }
    // The first candidate is taken; the numbered ones start at -2, like the
    // backfill's `row_number` suffix. The loop always returns from inside:
    // either a free candidate is found, or the candidate outgrows the D3
    // length bound and is refused rather than stored broken.
    let mut suffix: u64 = 2;
    loop {
        let candidate = format!("{base}-{suffix}");
        if normalize_username(&candidate).is_err() {
            return Err(StoreError::Validation(USERNAME_RULE.to_owned()));
        }
        if !identifier_taken_in_tx(tx, &candidate, exclude).await? {
            return Ok(candidate);
        }
        suffix += 1;
    }
}

/// Create a user and its role/tenant memberships.
///
/// Runs in a transaction: a user whose role or tenant names cannot all be
/// resolved is not left half-created with a partial membership set.
///
/// # Errors
///
/// * [`StoreError::FieldConflict`] (409) — the username (or the email,
///   when one is given) equals another user's username or email,
///   case-insensitively (ADM-11 D5). The message names the field.
/// * [`StoreError::Validation`] (400) — the username breaks the D3 rule,
///   or a given email is blank.
/// * [`StoreError::ForeignKeyViolation`] (400) — a role or tenant name in
///   `input` does not exist. Membership rows are inserted by *name*
///   (`INSERT ... SELECT id FROM role WHERE name = $2`), so an unknown name
///   inserts zero rows rather than raising a database-level FK error; the
///   `rows_affected` check below is what turns that silent no-op into the
///   same 400 a genuine FK violation produces.
/// * [`StoreError::Database`] (500) — anything else.
pub async fn create_user(pool: &PgPool, input: &InviteUserInput) -> Result<User, StoreError> {
    let mut tx = pool.begin().await?;
    let id = create_user_in_tx(&mut tx, input).await?;
    tx.commit().await?;
    get_user(pool, &id.to_string()).await
}

/// [`create_user`], inside a transaction the CALLER owns, returning the
/// new user's id instead of the whole [`User`] row.
///
/// ADM-3 D1: an invite writes the user, its memberships AND its
/// credential as one unit of work. The credential lives in `auth_identity`,
/// which `lakehouse-auth` owns; the user and its memberships live here.
/// Neither crate can open a transaction the other joins, so the CALLER
/// (the API route) opens it and hands it to both — this function is the
/// store's half of that.
///
/// Dropping `tx` without committing rolls the whole thing back (sqlx's
/// `Transaction` starts a rollback on drop), so an unknown role or tenant
/// name leaves no user behind.
///
/// # Errors
///
/// * [`StoreError::FieldConflict`] (409) — the username (or the email,
///   when one is given) equals another user's username or email,
///   case-insensitively (ADM-11 D5). Serialized by
///   `pg_advisory_xact_lock(USER_IDENTIFIER_LOCK_KEY)` (ADM-11 review 1 S5),
///   so two concurrent invites cannot both pass the check.
/// * [`StoreError::Validation`] (400) — the username breaks the ADM-11 D3
///   rule, or a given email is blank.
/// * [`StoreError::ForeignKeyViolation`] (400) — a role or tenant name in
///   `input` does not exist.
/// * [`StoreError::Database`] (500) — anything else.
pub async fn create_user_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: &InviteUserInput,
) -> Result<Uuid, StoreError> {
    // ADM-11 review 1 S5: serialize identifier checks and writes under USER_IDENTIFIER_LOCK_KEY
    // so concurrent invites cannot both pass the check.
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(USER_IDENTIFIER_LOCK_KEY)
        .execute(&mut **tx)
        .await?;

    let username = normalize_username(&input.username)?;
    let email = normalize_email(input.email.as_deref())?;
    // ADM-11 D5: one sign-in box must never match two people, so the
    // username is refused when it equals another user's username OR email,
    // and the reverse for the email. Inside this transaction, so a
    // concurrent invite or rename that commits first is seen.
    let (username_taken, email_taken) =
        username_email_taken_in_tx(tx, &username, email.as_deref(), None).await?;
    if username_taken {
        return Err(StoreError::FieldConflict(
            "Username already in use".to_owned(),
        ));
    }
    if email_taken {
        return Err(StoreError::FieldConflict("Email already in use".to_owned()));
    }
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO app_user (name, username, email) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&input.name)
    .bind(&username)
    .bind(&email)
    .fetch_one(&mut **tx)
    .await?;

    for role in &input.roles {
        let affected = sqlx::query(
            "INSERT INTO app_user_role (user_id, role_id) \
             SELECT $1, id FROM role WHERE name = $2 ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(role)
        .execute(&mut **tx)
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
        .execute(&mut **tx)
        .await?
        .rows_affected();
        if affected == 0 {
            return Err(StoreError::ForeignKeyViolation);
        }
    }

    Ok(id)
}

/// Add an existing user to a tenant: one `app_user_tenant` row, the same
/// insert [`create_user`] makes for each tenant named at invite time.
/// Idempotent: a user already in the tenant is left as it was.
///
/// Returns `true` when a row was added and `false` when the membership
/// already existed, so a caller can record the change only when there was
/// one. Membership is read from `app_user_tenant` on every request
/// (`lakehouse_auth::repository`), so the user's next request, on a session
/// they already hold, sees the tenant.
///
/// # Errors
///
/// Returns [`StoreError::ForeignKeyViolation`] if the user or the tenant
/// does not exist (callers check both first so a client sees a 404; this
/// is the answer when one is deleted between the check and the insert),
/// or [`StoreError::Database`] on any other failure.
pub async fn add_user_to_tenant(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Uuid,
) -> Result<bool, StoreError> {
    let added = sqlx::query(
        "INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(tenant_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(added > 0)
}

/// Whether an `app_user` row with this id exists.
///
/// Mirrors [`tenant_exists`] so BLD-1's `PUT /api/pipelines/{id}/run-as`
/// can answer an unknown author with the same 404 it gives an unknown
/// tenant, rather than leaking a foreign-key violation as a 400.
///
/// # Errors
///
/// [`StoreError::Database`] on any database failure.
pub async fn user_exists(pool: &PgPool, id: Uuid) -> Result<bool, StoreError> {
    let present: Option<(bool,)> =
        sqlx::query_as("SELECT EXISTS (SELECT 1 FROM app_user WHERE id = $1)")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(present.is_some_and(|(present,)| present))
}

// ── ADM-4: changing what a user may do ──────────────────────────────────
//
// The functions below are the store's half of ADM-4: they move a user's
// role memberships and tenant memberships, and they do it INSIDE a
// transaction the caller owns. The caller owns it because the rule the
// change has to satisfy (ADM-4 D5: "any change that would leave no active
// user holding `*:*` is refused with a 409, in the same transaction as the
// change") needs the change and the check that guards it to be one unit of
// work — a change committed here and checked afterwards would let two
// concurrent changes each see the other one still holding the grant and
// both be allowed, leaving nobody.
//
// The authorization half of ADM-4 (who may make the change at all) lives
// in `lakehouse-api`'s routes, next to every other route's rules; nothing
// in this module decides who may call it.

/// One role, as the identity routes need it: the name a caller uses, the
/// id `app_user_role.role_id` points at, and the raw `role.permissions`
/// text the role's grant is made of.
///
/// The permissions are handed over VERBATIM on purpose. `role.permissions`
/// is free text, and the ONE parser for it is `lakehouse_auth::PermissionSet`
/// — the parser every principal's permissions go through. A second
/// token matcher in this crate could disagree with it about what a grant
/// means, and a disagreement about a grant is a privilege-escalation hole
/// (AGENTS.md rule 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleGrant {
    /// `role.name` — what `User.roles` holds and what a caller names.
    pub name: String,
    /// `role.id` — what `app_user_role.role_id` points at.
    pub id: Uuid,
    /// `role.permissions`, verbatim: the free-text grant list.
    pub permissions: String,
}

/// Resolve `names` to the roles they name, inside the CALLER's
/// transaction.
///
/// Returns the roles that EXIST, sorted by name. A name that names no
/// role is simply absent from the answer: the caller compares the names it
/// asked for with the names it got back and answers 400 for the
/// difference (ADM-4 D1: "an unknown role name is a 400").
///
/// Duplicate names collapse — the answer is a SET of roles, and
/// `app_user_role`'s composite primary key would refuse the same role
/// twice.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any storage failure.
pub async fn resolve_roles_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    names: &[String],
) -> Result<Vec<RoleGrant>, StoreError> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(String, Uuid, String)> =
        sqlx::query_as("SELECT name, id, permissions FROM role WHERE name = ANY($1)")
            .bind(names)
            .fetch_all(&mut **tx)
            .await?;
    let mut found: Vec<RoleGrant> = rows
        .into_iter()
        .map(|(name, id, permissions)| RoleGrant {
            name,
            id,
            permissions,
        })
        .collect();
    found.sort_by(|left, right| left.name.cmp(&right.name));
    found.dedup_by(|left, right| left.id == right.id);
    Ok(found)
}

/// Replace `user_id`'s local role set with exactly `role_ids`, inside the
/// CALLER's transaction.
///
/// ADM-4 D1: the request carries the user's COMPLETE role set, not a
/// delta, so the user's roles after the call are exactly the roles the
/// request named. The rows are therefore deleted and re-inserted rather
/// than diffed: a diff would need a second implementation of "which roles
/// changed" (here and in the audit row that names the old and the new
/// set), and the two could disagree about what the user's role set is.
///
/// The delete and the inserts share the caller's transaction, so a
/// failure anywhere in it leaves the user's roles exactly as they were.
///
/// # Errors
///
/// Returns [`StoreError::ForeignKeyViolation`] if a role id names no
/// `role` row (the route resolves the names to ids first, so this can
/// only happen if the role was deleted between the resolution and this
/// write), or [`StoreError::Database`] on any other failure.
pub async fn set_user_roles_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    role_ids: &[Uuid],
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM app_user_role WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    for role_id in role_ids {
        sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
            .bind(user_id)
            .bind(role_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// The names of the roles `user_id` holds, inside the CALLER's
/// transaction.
///
/// Read for the audit row's "before" half: ADM-4 D6 asks the audit to
/// name the old AND the new role names, so an operator can see what the
/// change took away — not only what it gave.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any storage failure.
pub async fn user_role_names_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<Vec<String>, StoreError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT r.name FROM app_user_role ur JOIN role r ON r.id = ur.role_id \
         WHERE ur.user_id = $1 ORDER BY r.name",
    )
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(|(name,)| name).collect())
}

/// Remove `user_id` from `tenant_id` — the counterpart of
/// [`add_user_to_tenant`], inside the CALLER's transaction.
///
/// Returns `true` when a membership row was removed and `false` when the
/// user was not in that tenant (there was nothing to remove). Idempotent,
/// exactly like the add: removing a member who is not a member changes
/// nothing and says so.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any storage failure.
pub async fn remove_user_from_tenant_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    tenant_id: Uuid,
) -> Result<bool, StoreError> {
    let removed = sqlx::query("DELETE FROM app_user_tenant WHERE user_id = $1 AND tenant_id = $2")
        .bind(user_id)
        .bind(tenant_id)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    Ok(removed > 0)
}

/// Set `user_id`'s `app_user.status` to `status` (`"active"` or
/// `"inactive"`), inside the CALLER's transaction.
///
/// ADM-4 D3: switching a user off is a status change AND the end of every
/// session that user holds — ONE unit of work, so a user can never be
/// deactivated with a live session left behind. The caller therefore owns
/// the transaction and writes the session revocations into it too
/// (`lakehouse_auth::session::revoke_all_sessions_for_user_in_tx`).
///
/// The user's data, dashboards and audit history are NOT touched: only
/// the status changes. Deactivation is the answer to "this person has
/// left"; deleting the account would lose who authored what.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if no such user exists, or
/// [`StoreError::Database`] on any other failure. A `status` the schema
/// does not allow is refused by `app_user_status_check` — a database
/// error, not a silent write.
pub async fn set_user_status_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    status: &str,
) -> Result<(), StoreError> {
    let affected = sqlx::query("UPDATE app_user SET status = $2, updated_at = now() WHERE id = $1")
        .bind(user_id)
        .bind(status)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// The `app_user.status` of `id` — `"active"` or `"inactive"`, the two
/// values `app_user_status_check` allows — or `None` when there is no such
/// user.
///
/// A **deactivated user is not a dead user**, and `lakehouse_auth::
/// repository::load_principal_for_user` does not distinguish them: it
/// reads `app_user.name` and hands back every role the user has, so a
/// pipeline authored by a deactivated user would otherwise be enforced as
/// if the user were still working. Callers that need the distinction read
/// it here.
///
/// # Errors
///
/// [`StoreError::Database`] on any database failure.
pub async fn user_status(pool: &PgPool, id: Uuid) -> Result<Option<String>, StoreError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT status FROM app_user WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(status,)| status))
}

/// Whether `user_id` is a member of `tenant_id`, read from the same
/// `app_user_tenant` row every request's tenant list is read from
/// (`lakehouse_auth::repository::load_principal_for_user`), so a caller's
/// own scope check and this one can never disagree about membership.
///
/// Used by BLD-1's `PUT /api/pipelines/{id}/run-as` to hold the author to
/// the pipeline's own tenant: a pipeline may only read as a user who is
/// inside the same scope the pipeline itself lives in.
///
/// # Errors
///
/// [`StoreError::Database`] on any database failure.
pub async fn user_in_tenant(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Uuid,
) -> Result<bool, StoreError> {
    let present: Option<(bool,)> = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM app_user_tenant \
         WHERE user_id = $1 AND tenant_id = $2)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(present.is_some_and(|(present,)| present))
}

/// The tenants a user belongs to (`app_user_tenant`), in no particular order.
/// An empty list means the user belongs to no tenant.
///
/// Read from the same rows `lakehouse_auth::repository::load_principal_for_user`
/// puts in `Principal::tenant_ids`, so this list and the tenant list on a
/// request can never disagree about membership. Used by `RPT-1`'s recipient
/// rules (review fix (SHOULD-FIX) 1): the picker and the save-time refusal
/// ask whether a person shares a tenant with the report's owner.
///
/// # Errors
///
/// [`StoreError::Database`] on any database failure.
pub async fn user_tenant_ids(pool: &PgPool, user_id: Uuid) -> Result<Vec<Uuid>, StoreError> {
    let rows: Vec<(Uuid,)> =
        sqlx::query_as("SELECT tenant_id FROM app_user_tenant WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// The display name of `id`, or `None` when there is no such row.
///
/// Read from `app_user.name` — the same column
/// [`lakehouse_auth::repository::load_principal_for_user`] puts in
/// [`lakehouse_auth::Principal::display_name`], so a pipeline's page and its
/// own enforcement agree on who the author is. Used by BLD-1's pipeline
/// detail read: `GET /api/identity/users` needs `identity:read`, which no
/// ordinary role holds, so the name has to travel with the pipeline or the
/// people who operate pipelines cannot see whose permissions govern them.
///
/// # Errors
///
/// [`StoreError::Database`] on any database failure.
pub async fn display_name(pool: &PgPool, id: Uuid) -> Result<Option<String>, StoreError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT name FROM app_user WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(name,)| name))
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
    // safe: only the constant `ROLE_SELECT` is formatted in; values are bound.
    let rows: Vec<RoleRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .fetch_all(pool)
        .await?;
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
    // safe: only the constant `ROLE_SELECT` is formatted in; values are bound.
    let row: RoleRow = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
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

/// Advisory lock key used to serialize identifier (username / email) checks and writes
/// across concurrent invites, user identity updates, and OIDC JIT provisioning
/// (ADM-11 review 1 S5).
pub const USER_IDENTIFIER_LOCK_KEY: i64 = 0x4c48_5f55_5345_5253; // "LH_USERS"

/// The roles `0002_seed_identity.sql` creates: the product's built-in
/// permission bundles, which a deployment starts with and which
/// `DELETE /api/identity/roles/{id}` refuses (`WEB-3` S3a-D2).
///
/// The seed's fixed ids are the only marker it writes — there is no
/// `system` column — and they are what this list pins. Ids, not names, so a
/// rename (`update_role`) never turns a built-in role into a removable one.
pub const BUILT_IN_ROLE_IDS: [&str; 7] = [
    "22222222-2222-4222-8222-000000000001",
    "22222222-2222-4222-8222-000000000002",
    "22222222-2222-4222-8222-000000000003",
    "22222222-2222-4222-8222-000000000004",
    "22222222-2222-4222-8222-000000000005",
    "22222222-2222-4222-8222-000000000006",
    "22222222-2222-4222-8222-000000000007",
];

/// Whether `id` is one of [`BUILT_IN_ROLE_IDS`] (compared lower-cased, as
/// [`Role::id`] renders a UUID).
#[must_use]
pub fn is_built_in_role(id: &str) -> bool {
    let id = id.trim().to_lowercase();
    BUILT_IN_ROLE_IDS.contains(&id.as_str())
}

/// Everything [`update_role`] needs. Mirrors `UpdateRoleInput`.
#[derive(Debug, Clone)]
pub struct UpdateRoleInput {
    /// The role's new name; must not collide with another role's.
    pub name: String,
    /// Free-text permission list.
    pub permissions: String,
    /// Human-readable description.
    pub description: String,
}

/// Rewrite a role's name, permissions and description, returning it as the
/// list shows it (with its current member count).
///
/// Memberships join by id, so a rename reaches every user who holds the
/// role at their next request. A policy's free-text `subjects` is NOT
/// rewritten: the console's Access rules page shows the stored text, so
/// the old name stays visible. A policy's structured `conditions` is the
/// dangerous case — a clause names roles by name and carries that role's
/// masks and row filters — so `routes::identity::update_role` refuses a
/// rename while one names this role (`WEB-3` review R2) and this function
/// is never reached with obligations still attached. It rewrites no
/// `policy` row itself.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if no role has that id (including an id
/// that is not a UUID), [`StoreError::Conflict`] (409) if another role has
/// the name, or [`StoreError::Database`] on any other failure.
pub async fn update_role(
    pool: &PgPool,
    id: &str,
    input: &UpdateRoleInput,
) -> Result<Role, StoreError> {
    let affected = sqlx::query(
        "UPDATE role SET name = $2, permissions = $3, description = $4, updated_at = now() \
         WHERE id = $1",
    )
    .bind(parse_id(id)?)
    .bind(&input.name)
    .bind(&input.permissions)
    .bind(&input.description)
    .execute(pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(StoreError::NotFound);
    }
    get_role(pool, id).await
}

// ── Table access (`GOV-7`) ──────────────────────────────────────────────

/// One table (or whole schema, with `table` `"*"`) a role may read.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, sqlx::FromRow)]
pub struct TableGrant {
    /// The schema name.
    pub schema: String,
    /// The table name, or `"*"` for the whole schema.
    pub table: String,
}

/// A role's read scope (`GOV-7` D3): `'all'`, or `'listed'` with the tables
/// and schemas its grants name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleTableAccess {
    /// `role.read_scope`: `'all'` or `'listed'`.
    pub scope: String,
    /// The granted tables and schemas, sorted, for `'listed'` roles.
    pub grants: Vec<TableGrant>,
}

/// A role's table access. The grants are sorted, so the same role answers
/// the same body every time.
///
/// # Errors
///
/// [`StoreError::NotFound`] if no role has that id (or `id` is not a UUID);
/// [`StoreError::Database`] on any other failure.
pub async fn get_role_table_access(pool: &PgPool, id: &str) -> Result<RoleTableAccess, StoreError> {
    let role_id = parse_id(id)?;
    let scope: Option<String> = sqlx::query_scalar("SELECT read_scope FROM role WHERE id = $1")
        .bind(role_id)
        .fetch_optional(pool)
        .await?;
    let Some(scope) = scope else {
        return Err(StoreError::NotFound);
    };
    let grants: Vec<(String, String)> = sqlx::query_as(
        "SELECT schema_name, table_name FROM role_read_grant WHERE role_id = $1          ORDER BY schema_name, table_name",
    )
    .bind(role_id)
    .fetch_all(pool)
    .await?;
    Ok(RoleTableAccess {
        scope,
        grants: grants
            .into_iter()
            .map(|(schema, table)| TableGrant { schema, table })
            .collect(),
    })
}

/// Replace a role's scope and grants in ONE transaction (`GOV-7` D7): the
/// scope and the rows that give it meaning are one unit of work, so a reader
/// never sees a `'listed'` role with the previous scope's grants (or the
/// other way round).
///
/// A scope of `'all'` stores no grants: keeping stale rows for a role that
/// reads everything would silently resurrect them the next time somebody
/// narrows it.
///
/// # Errors
///
/// [`StoreError::NotFound`] if no role has that id (or `id` is not a UUID);
/// [`StoreError::Database`] on any other failure, including a duplicate
/// grant (the caller deduplicates first).
pub async fn set_role_table_access(
    pool: &PgPool,
    id: &str,
    scope: &str,
    grants: &[TableGrant],
    created_by: Option<Uuid>,
) -> Result<RoleTableAccess, StoreError> {
    let role_id = parse_id(id)?;
    let mut tx = pool.begin().await?;
    let affected = sqlx::query("UPDATE role SET read_scope = $2, updated_at = now() WHERE id = $1")
        .bind(role_id)
        .bind(scope)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(StoreError::NotFound);
    }
    sqlx::query("DELETE FROM role_read_grant WHERE role_id = $1")
        .bind(role_id)
        .execute(&mut *tx)
        .await?;
    for grant in grants {
        sqlx::query(
            "INSERT INTO role_read_grant (role_id, schema_name, table_name, created_by)              VALUES ($1, $2, $3, $4)",
        )
        .bind(role_id)
        .bind(&grant.schema)
        .bind(&grant.table)
        .bind(created_by)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    get_role_table_access(pool, id).await
}

// ── Tenant ──────────────────────────────────────────────────────────────

/// Slim subset of [`Tenant`] (just `id`, `name`, `slug`) used only by
/// `GET /api/auth/me` to render a caller's own tenant memberships, never a
/// general tenant-listing endpoint (that is [`Tenant`], unchanged). Lives
/// here rather than in `routes/auth.rs` because [`list_tenants_by_ids`]
/// returns it: a tuple of three bare columns would force every caller to
/// re-shape the rows into the same `{id, name, slug}` triple, and the only
/// such caller (`routes::auth::me`) is the one place that wants this exact
/// shape — so the store gives back the typed value directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TenantSummary {
    /// `tenant.id`, rendered as a string.
    pub id: String,
    /// Display name.
    pub name: String,
    /// URL-safe identifier.
    pub slug: String,
}

/// Load `{id, name, slug}` for exactly `tenant_ids`, in no particular
/// order. Short-circuits to `Ok(Vec::new())` on an empty input, so a caller
/// that already knows it has nothing to look up never pays a round trip —
/// `routes::auth::me` relies on this to keep its service-principal /
/// `tenant_ids.is_empty()` paths at zero queries.
///
/// Used only by `GET /api/auth/me` to render the caller's own tenant
/// memberships; the general tenant-listing endpoint is [`list_tenants`],
/// unchanged.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails (including Postgres
/// being unreachable).
pub async fn list_tenants_by_ids(
    pool: &PgPool,
    tenant_ids: &[Uuid],
) -> Result<Vec<TenantSummary>, StoreError> {
    if tenant_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(Uuid, String, String)> =
        sqlx::query_as("SELECT id, name, slug FROM tenant WHERE id = ANY($1)")
            .bind(tenant_ids)
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, slug)| TenantSummary {
            id: id.to_string(),
            name,
            slug,
        })
        .collect())
}

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
    /// Lakekeeper's opaque warehouse id backing this tenant's Iceberg data,
    /// or `None` if provisioning has not created one (see migration
    /// `0042_tenant_provisioning.sql`). Serializes as
    /// `warehouseId`. Text, not a UUID: it stores exactly what
    /// Lakekeeper's `management/v1/warehouse` response returns.
    pub warehouse_id: Option<String>,
    /// Resumable checkpoint for tenant provisioning
    /// (`pending`/`warehouse_ready`/`grants_ready`/`namespace_ready`/
    /// `complete`/`not_applicable`/`failed`), enforced by a `CHECK`
    /// constraint at the database. `not_applicable` marks a tenant seeded
    /// before provisioning existed (`0002_seed_identity.sql`) — never
    /// `complete`, since nothing was actually provisioned for it.
    /// Serializes as `provisioningStatus`.
    pub provisioning_status: String,
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
    warehouse_id: Option<String>,
    provisioning_status: String,
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
            warehouse_id: row.warehouse_id,
            provisioning_status: row.provisioning_status,
        }
    }
}

/// The columns every tenant read shares, including the derived `users`
/// count.
const TENANT_SELECT: &str = "SELECT t.id, t.name, t.slug, t.plan, t.residency, \
     t.storage_bytes, t.quota_compute, t.used_compute, \
     t.warehouse_id, t.provisioning_status, \
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
    // safe: only the constant `TENANT_SELECT` is formatted in; values are bound.
    let rows: Vec<TenantRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
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
    // safe: only the constant `TENANT_SELECT` is formatted in; values are bound.
    let row: TenantRow = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(parse_id(id)?)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// Whether a tenant with `id` exists — the existence check
/// `routes::connectors::assign_connector_tenant`/
/// `routes::pipelines::assign_pipeline_tenant` each
/// run before writing a `connector`/`pipeline_definition` row's
/// `tenant_id`, so an unknown tenant id 404s instead of silently creating a
/// foreign-key violation the caller would see as a generic 500. Extracted
/// here (rather than duplicated as an inline `EXISTS(...)` query in both
/// route files) per AGENTS.md rule 4 — one query, one place, shared by both
/// assignment routes.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn tenant_exists(pool: &PgPool, id: Uuid) -> Result<bool, StoreError> {
    let (exists,): (bool,) = sqlx::query_as("SELECT EXISTS(SELECT 1 FROM tenant WHERE id = $1)")
        .bind(id)
        .fetch_one(pool)
        .await?;
    Ok(exists)
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

/// Find a tenant by its slug, or `None` if no such tenant exists.
///
/// Unlike [`get_tenant`], a missing row is not an error: `POST
/// /api/identity/tenants` uses this to
/// decide whether to resume an existing, still-in-progress row or create a
/// brand-new one — `None` is exactly the "create a new tenant" case, not a
/// failure.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn find_tenant_by_slug(pool: &PgPool, slug: &str) -> Result<Option<Tenant>, StoreError> {
    let sql = format!("{TENANT_SELECT} WHERE t.slug = $1");
    // safe: only the constant `TENANT_SELECT` is formatted in; values are bound.
    let row: Option<TenantRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(slug)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(Tenant::from))
}

/// Advance a tenant's provisioning checkpoint, optionally recording the
/// Lakekeeper warehouse id.
///
/// Called once per step of the provisioning state machine
/// (`routes::identity::provision_tenant`), and only
/// after that step has genuinely succeeded — the caller is responsible for
/// never calling this to advance past a step that failed, so a crash or
/// error mid-way leaves the last truthful `provisioning_status` rather
/// than a fabricated one. `warehouse_id: None` leaves the column
/// untouched (`COALESCE`) instead of clearing it, so a later step that
/// does not itself know the warehouse id (grants/namespace) cannot
/// accidentally erase the id an earlier step recorded.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if no such tenant exists (or
/// `tenant_id` is not a UUID), or [`StoreError::Database`] on any other
/// failure.
pub async fn update_tenant_provisioning_status(
    pool: &PgPool,
    tenant_id: &str,
    status: &str,
    warehouse_id: Option<&str>,
) -> Result<Tenant, StoreError> {
    let id = parse_id(tenant_id)?;
    let affected = sqlx::query(
        "UPDATE tenant SET provisioning_status = $2, \
         warehouse_id = COALESCE($3, warehouse_id) WHERE id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(warehouse_id)
    .execute(pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(StoreError::NotFound);
    }
    get_tenant(pool, tenant_id).await
}

/// Delete a tenant.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if no such tenant exists,
/// [`StoreError::ForeignKeyViolation`] if a user still belongs to it
/// (`app_user_tenant.tenant_id` is `ON DELETE RESTRICT`), a file of the
/// tenant still sits in a folder (`file_upload.folder_id` is
/// `ON DELETE RESTRICT`, `DATA-35` migration `0112`; `PR #259 review
/// SHOULD-FIX 2`), or a folder still holds a sub-folder or a query in a
/// folder (`saved_query.folder_id` and `saved_query_folder.parent_id` are
/// `ON DELETE RESTRICT` in `0122_saved_query_access.sql`; `RESTRICT` is
/// not deferrable, so the database refuses the delete and it fails
/// closed, no behaviour change: plan 55 D8), or
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
    ///
    /// Not simply the stored `service_identity.rotation_status` column: that
    /// column is never updated as time passes (`0001_init.sql:107-121`),
    /// while service-token authentication already refuses a credential once
    /// `expires_at <= now()` (`lakehouse-auth::service_token`, line ~135).
    /// Left alone, an identity created `"current"` would keep reporting
    /// `"current"` after authentication had started rejecting it. So
    /// [`SERVICE_IDENTITY_SELECT`] derives `"expired"` in SQL whenever
    /// `expires_at <= now()`, matching authentication exactly, and only
    /// falls back to the stored value otherwise. No `"due"` threshold is
    /// derived because none is specified (`0001_init.sql`'s header comment
    /// for this column says so explicitly) — inventing one here would be a
    /// fabricated policy, not an honest read.
    pub rotation_status: String,
    /// When this identity's credential was last actually used, or `None` if
    /// it has never been used. Serializes as `lastUsedAt`.
    ///
    /// `None` is the honest answer for a never-used identity, and the
    /// column alone cannot give it: `last_used_at` is `NOT NULL DEFAULT
    /// now()` (`0001_init.sql`), so a freshly created identity's column
    /// holds its creation time. Serving that would report a use that never
    /// happened. [`SERVICE_IDENTITY_SELECT`] therefore returns the column
    /// only when it has moved past `created_at` — which only
    /// `lakehouse-auth::service_token::verify_service_token`'s write can do
    /// — and NULL otherwise. That write is throttled to once per 300 s per
    /// identity but always lets the FIRST use through, so a used identity
    /// is never reported as unused for want of a second call.
    pub last_used_at: Option<String>,
}

/// The raw row shape service-identity reads select.
///
/// `rotation_status` here holds the *derived* value
/// [`SERVICE_IDENTITY_SELECT`] computes, not the bare column.
#[derive(Debug, FromRow)]
struct ServiceIdentityRow {
    id: Uuid,
    name: String,
    scopes: Vec<String>,
    environment: String,
    expires_at: OffsetDateTime,
    rotation_status: String,
    last_used_at: Option<OffsetDateTime>,
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
            last_used_at: row.last_used_at.map(iso_millis),
        }
    }
}

/// The columns every service-identity read shares.
///
/// `rotation_status` is a `CASE` expression, not the bare column: it derives
/// `"expired"` whenever `expires_at <= now()` — the exact condition
/// `lakehouse-auth::service_token` uses to refuse a credential — so a filter
/// and the list can never disagree with each other or with authentication.
/// See [`ServiceIdentity::rotation_status`] for the full rationale.
/// `last_used_at` is NULL unless the column has moved past `created_at`:
/// the column defaults to the insert time, so equal-to-creation means "never
/// used", not "used at creation" — see [`ServiceIdentity::last_used_at`].
const SERVICE_IDENTITY_SELECT: &str = "SELECT s.id, s.name, s.scopes, s.environment, s.expires_at, \
     CASE WHEN s.expires_at <= now() THEN 'expired' ELSE s.rotation_status END AS rotation_status, \
     CASE WHEN s.last_used_at > s.created_at THEN s.last_used_at END AS last_used_at \
     FROM service_identity s";

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
    // safe: only the constant `SERVICE_IDENTITY_SELECT` is formatted in; values are bound.
    let rows: Vec<ServiceIdentityRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
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
    // safe: only the constant `SERVICE_IDENTITY_SELECT` is formatted in; values are bound.
    let row: ServiceIdentityRow = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
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

/// What [`rotate_service_identity`] returns: the fresh raw token, returned
/// to the caller exactly once, alongside the re-read identity.
///
/// The `secret` is the raw opaque token whose SHA-256 hex digest was just
/// persisted to `service_credential.token_hash`. It is the only place in
/// the codebase this raw value is ever returned — every list/get route on
/// `service_identity` returns credential *metadata*, never a hash's input.
/// Mirrors how a session's raw token is returned only at login/rotate time
/// and never again from any subsequent `/api/auth/sessions`/`/me`/etc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotateServiceIdentityResponse {
    /// The newly minted raw token, returned exactly once on this response.
    /// Invariant: never persisted in plaintext, never re-served by any list
    /// or get route (see [`rotate_service_identity`]'s doc comment for why
    /// the only honest read-back is the freshly-issued one). Serializes as
    /// `secret` to match the contract's `secret: string` field.
    pub secret: String,
    /// The service identity after the rotation's three writes have landed:
    /// `expires_at` reset to a fresh `NEW_IDENTITY_VALIDITY_DAYS` window and
    /// `rotation_status` reset to `"current"`. Serializes as `identity`.
    pub identity: ServiceIdentity,
}

/// Revoke every currently-unrevoked credential for `identity_id`, mint a
/// fresh credential, and reset `expires_at` to a fresh
/// `NEW_IDENTITY_VALIDITY_DAYS` window and `rotation_status` to
/// `"current"`, all in one transaction.
///
/// # Why the `mint_credential` callback rather than minting inline
///
/// `lakehouse-auth` already owns the CSPRNG (`generate_opaque_token`) +
/// SHA-256 (`hash_token`) pair that turns a credential into a row in
/// `service_credential`. Inlining those two functions here would be a
/// second copy of the same guard — the very thing AGENTS.md rule 4 calls a
/// "finding". But `lakehouse-auth` already depends on `lakehouse-store`
/// (`lakehouse_auth::repository`'s re-export of `lakehouse_store::PgPool`),
/// so importing `lakehouse-auth::token::*` from `lakehouse-store` would be a
/// Cargo cycle Cargo refuses to build. A callback makes the contract
/// explicit: this function owns the SQL transaction shape (revoke, reset,
/// insert, commit), and a caller-owned closure owns "where the (raw token,
/// hash) pair comes from". The route handler at
/// `lakehouse-api/src/routes/identity::rotate_service_identity` provides
/// the closure, using `lakehouse_auth::token::generate_opaque_token` +
/// `lakehouse_auth::token::hash_token` — no duplication of either.
///
/// # The old credential's fate
///
/// Every currently-unrevoked `service_credential` row for `identity_id` is
/// marked `revoked_at = now()` BEFORE the new row is inserted, and the
/// transaction holds both writes — so a leak window between the two is
/// closed: the moment rotation succeeds, every previously-valid token for
/// this identity is rejected by `lakehouse_auth::service_token::
/// verify_service_token` (it filters on `revoked_at IS NULL` and the
/// never-reissued token's hash no longer matches any unrevoked row).
/// `rotate_revokes_the_old_credential_immediately` is the load-bearing
/// test for this property; it lives in `lakehouse-auth`'s test suite (it
/// needs `verify_service_token`, which `lakehouse-store`'s tests cannot
/// import — same cycle).
///
/// # Errors
///
/// * [`StoreError::NotFound`] if `identity_id` does not exist (the final
///   `get_service_identity` re-read sees zero rows after the `UPDATE`
///   touched zero rows, so the same `NotFound` covers both "no such id"
///   and a non-UUID `id` — the parser rejects the latter before this
///   function runs).
/// * [`StoreError::Database`] on any storage failure, including a FK
///   violation if `identity_id` was deleted between the UPDATE and the
///   subsequent INSERT — surface as a classified `ApiError::Internal`,
///   never an upstream-text interpolation.
pub async fn rotate_service_identity(
    pool: &PgPool,
    identity_id: Uuid,
    mint_credential: impl FnOnce() -> (String, String),
) -> Result<RotateServiceIdentityResponse, StoreError> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE service_credential SET revoked_at = now() \
         WHERE service_identity_id = $1 AND revoked_at IS NULL",
    )
    .bind(identity_id)
    .execute(&mut *tx)
    .await?;
    let new_expires_at =
        OffsetDateTime::now_utc() + time::Duration::days(NEW_IDENTITY_VALIDITY_DAYS);
    let rotated = sqlx::query(
        "UPDATE service_identity SET rotation_status = 'current', expires_at = $2, updated_at = now() \
         WHERE id = $1",
    )
    .bind(identity_id)
    .bind(new_expires_at)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    // Detect "no such identity" before the INSERT below — otherwise the
    // missing FK raises `ForeignKeyViolation` (a 400-mapped class), which
    // would be the wrong response shape for "this id does not exist"
    // (404-mapped `NotFound`). Raising here drops the transaction so the
    // revocation write is also rolled back, leaving every previously-valid
    // credential unchanged.
    if rotated == 0 {
        return Err(StoreError::NotFound);
    }
    let (raw_token, token_hash) = mint_credential();
    sqlx::query("INSERT INTO service_credential (service_identity_id, token_hash) VALUES ($1, $2)")
        .bind(identity_id)
        .bind(&token_hash)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let identity = get_service_identity(pool, &identity_id.to_string()).await?;
    Ok(RotateServiceIdentityResponse {
        secret: raw_token,
        identity,
    })
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

/// Describes an update to a user's email address (ADM-11 D8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailUpdate<'a> {
    /// Leave the existing email address unchanged.
    Unchanged,
    /// Clear the email address (set to NULL).
    Clear,
    /// Set a new email address.
    Set(&'a str),
}

/// Update a user's username and/or email address (ADM-11 D8).
///
/// Refuses with [`StoreError::FieldConflict`] if the new username or email clashes
/// with another user's username or email (case-insensitively, ADM-11 D5).
/// Refuses with [`StoreError::Validation`] if the username breaks ADM-11 D3 rules.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if `user_id` does not exist,
/// [`StoreError::FieldConflict`] on unique clash, [`StoreError::Validation`] on
/// invalid format, or [`StoreError::Database`] on query failure.
pub async fn update_user_identity(
    pool: &PgPool,
    user_id: Uuid,
    new_username: Option<&str>,
    new_email: EmailUpdate<'_>,
) -> Result<User, StoreError> {
    let mut tx = pool.begin().await?;
    let user = update_user_identity_in_tx(&mut tx, user_id, new_username, new_email).await?;
    tx.commit().await?;
    Ok(user)
}

/// [`update_user_identity`], inside a transaction the caller owns.
///
/// # Errors
///
/// Same as [`update_user_identity`].
pub async fn update_user_identity_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    new_username: Option<&str>,
    new_email: EmailUpdate<'_>,
) -> Result<User, StoreError> {
    // ADM-11 review 1 S5: serialize identifier checks and writes under USER_IDENTIFIER_LOCK_KEY.
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(USER_IDENTIFIER_LOCK_KEY)
        .execute(&mut **tx)
        .await?;

    let current = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT username, email FROM app_user WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StoreError::NotFound)?;

    let target_username = match new_username {
        Some(u) => normalize_username(u)?,
        None => current.0,
    };

    let target_email = match new_email {
        EmailUpdate::Set(e) => normalize_email(Some(e))?,
        EmailUpdate::Clear => None,
        EmailUpdate::Unchanged => current.1,
    };

    let (username_taken, email_taken) =
        username_email_taken_in_tx(tx, &target_username, target_email.as_deref(), Some(user_id))
            .await?;
    if username_taken {
        return Err(StoreError::FieldConflict(
            "Username already in use".to_owned(),
        ));
    }
    if email_taken {
        return Err(StoreError::FieldConflict("Email already in use".to_owned()));
    }

    sqlx::query("UPDATE app_user SET username = $1, email = $2, updated_at = now() WHERE id = $3")
        .bind(&target_username)
        .bind(&target_email)
        .bind(user_id)
        .execute(&mut **tx)
        .await?;

    let sql = format!("{USER_SELECT} WHERE u.id = $1");
    let row: UserRow = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await?;
    Ok(row.into())
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
            username: "rina".to_owned(),
            email: Some("rina@meridian.example".to_owned()),
            status: "active".to_owned(),
            roles: vec!["Analyst".to_owned()],
            tenants: vec!["Meridian Group".to_owned()],
            last_activity: None,
        };
        let value = serde_json::to_value(&user).unwrap();
        for key in [
            "id",
            "name",
            "username",
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
            warehouse_id: None,
            provisioning_status: "not_applicable".to_owned(),
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
            last_used_at: Some("2026-01-01T00:00:00.000Z".to_owned()),
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
