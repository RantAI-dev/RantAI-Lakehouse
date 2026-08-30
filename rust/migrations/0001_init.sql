-- Phase 2, Task 2.1: OLTP identity schema.
--
-- Derived from `src/services/contracts/identity.ts` (the spec) and
-- `src/services/mock/identity.ts` (fixture cardinality/optionality). Covers
-- `Tenant`, `User`, `Role`, and `ServiceIdentity`. `WorkspaceSettings` is not
-- modeled here: it is a single piece of workspace-wide app configuration,
-- not an identity entity with its own lifecycle, and is deferred to
-- whichever later Phase 2 task actually needs to persist it.
--
-- Fields the contracts model as *derived aggregates* (not raw stored facts)
-- are deliberately NOT stored as columns, to avoid a value that can silently
-- go stale next to the join tables that actually own the count:
--   * `Tenant.users` / `Tenant.agents`   -> COUNT(*) over `app_user_tenant`
--     (agents has no owning table yet in this migration; left for the
--     `agents` domain's own migration in a later task).
--   * `Role.members`                     -> COUNT(*) over `app_user_role`.
-- Every other contract field maps 1:1 to a column below; see the per-table
-- comments for exact field -> column correspondence.

-- ── tenant ──────────────────────────────────────────────────────────────
-- Maps `Tenant` (identity.ts:18-28). `storageBytes`/`quotaCompute`/
-- `usedCompute` map straight across; `users`/`agents` are derived (see
-- above) and intentionally absent as columns.
CREATE TABLE tenant (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name           TEXT NOT NULL,
    slug           TEXT NOT NULL,
    plan           TEXT NOT NULL,
    residency      TEXT NOT NULL,
    storage_bytes  BIGINT NOT NULL DEFAULT 0,
    quota_compute  BIGINT NOT NULL DEFAULT 0,
    used_compute   BIGINT NOT NULL DEFAULT 0,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- `slug` is what a tenant is addressed by outside its opaque id
    -- (`CreateTenantInput.slug`); the domain requires it be unique the same
    -- way a URL path segment or subdomain would be.
    CONSTRAINT tenant_slug_unique UNIQUE (slug)
);

-- ── role ────────────────────────────────────────────────────────────────
-- Maps `Role` (identity.ts:11-17). `members` is derived (see above) and
-- absent as a column.
CREATE TABLE role (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name         TEXT NOT NULL,
    permissions  TEXT NOT NULL DEFAULT '',
    description  TEXT NOT NULL DEFAULT '',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Every fixture in mock/identity.ts uses a distinct role name, and
    -- `User.roles: string[]` addresses roles by that name -- two roles
    -- with the same name would be ambiguous to every caller building that
    -- array, so name uniqueness is enforced here.
    CONSTRAINT role_name_unique UNIQUE (name)
);

-- ── app_user ────────────────────────────────────────────────────────────
-- Maps `User` (identity.ts:1-9). `roles`/`tenants` (string[] of names) are
-- structured many-to-many relationships to first-class entities (`role`,
-- `tenant`), so they are modeled as join tables below, never flattened into
-- an array/CSV column on this table.
CREATE TABLE app_user (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name             TEXT NOT NULL,
    email            TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'active',
    last_activity_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- `User.status` is a closed `"active" | "inactive"` union in the
    -- contract; enforce it at the schema boundary rather than trusting
    -- every future caller to only ever write one of the two strings.
    CONSTRAINT app_user_status_check CHECK (status IN ('active', 'inactive')),
    -- Every fixture in mock/identity.ts uses a distinct email, and it is
    -- how a human user is identified/invited (`InviteUserInput.email`) --
    -- the standard real-world invariant for a user table.
    CONSTRAINT app_user_email_unique UNIQUE (email)
);

-- ── app_user_role ───────────────────────────────────────────────────────
-- Backs `User.roles: string[]`. Composite PK doubles as the uniqueness
-- constraint (a user cannot hold the same role twice) and the FK index.
CREATE TABLE app_user_role (
    user_id  UUID NOT NULL REFERENCES app_user(id) ON DELETE CASCADE,
    role_id  UUID NOT NULL REFERENCES role(id) ON DELETE RESTRICT,
    PRIMARY KEY (user_id, role_id)
);

-- ── app_user_tenant ─────────────────────────────────────────────────────
-- Backs `User.tenants: string[]` (every mock fixture belongs to >= 1
-- tenant). Composite PK doubles as the uniqueness constraint and FK index.
CREATE TABLE app_user_tenant (
    user_id    UUID NOT NULL REFERENCES app_user(id) ON DELETE CASCADE,
    tenant_id  UUID NOT NULL REFERENCES tenant(id) ON DELETE RESTRICT,
    PRIMARY KEY (user_id, tenant_id)
);

-- ── service_identity ────────────────────────────────────────────────────
-- Maps `ServiceIdentity` (identity.ts:31-38). `scopes: string[]` has no
-- matching first-class entity in the contract (unlike `User.roles`, there
-- is no `Scope` type with its own id/description) -- a native Postgres
-- array is the faithful structured mapping, not a flattening, since the
-- contract itself models `scopes` as nothing more than a list of strings.
--
-- `rotationStatus` ("current" | "due" | "expired") IS stored, not derived
-- from `expires_at`. The `mock/identity.ts` fixtures are directionally
-- consistent with a now()-vs-`expires_at` rule (`si-6`'s `expiresAt` is in
-- the past and is "expired"; `si-1..si-5` are all in the future and are
-- "current" or "due"), which suggests "due" is "expires within N days" for
-- some N -- but that N is never stated anywhere in the 91-line contract or
-- the mock, only implied by two data points (7 days -> due, 30 days ->
-- current). Hardcoding a guessed threshold into this migration would be
-- inventing a field the contract does not actually specify; storing the
-- status explicitly instead lets whichever later Phase 2 task owns
-- rotation policy define (and change) that threshold in application code
-- without a schema migration.
CREATE TABLE service_identity (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name             TEXT NOT NULL,
    scopes           TEXT[] NOT NULL DEFAULT '{}',
    environment      TEXT NOT NULL,
    rotation_status  TEXT NOT NULL DEFAULT 'current',
    expires_at       TIMESTAMPTZ NOT NULL,
    last_used_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT service_identity_rotation_status_check
        CHECK (rotation_status IN ('current', 'due', 'expired')),
    -- Every fixture in mock/identity.ts uses a distinct name, and it is how
    -- an operator identifies a service credential in the UI/CLI.
    CONSTRAINT service_identity_name_unique UNIQUE (name)
);
