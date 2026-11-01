-- Who may see (and later edit and manage) which dashboard or folder
-- (`BI-19` part B, decision D6 of
-- `docs/superpowers/plans/2026-10-18-bi-19-sharing.md`; feature page
-- `docs/core/features/sharing-and-delivery.md`, decisions B1 to B9).
--
-- Why Postgres and not ClickHouse beside `bi_board`: the grantees (users and
-- roles) live here, so a deleted user's rules go with them by ON DELETE
-- CASCADE instead of lingering as ids nobody can resolve; and a rule is
-- changed by its own statements, never carried through a full-row dashboard
-- save (the race that moved the public link out of the board row in `0098`).
-- `item_id` names a `bi_board` or `bi_folder` row in ClickHouse, so it has no
-- foreign key: a dashboard that is deleted or purged leaves its rules inert,
-- and the API removes them when the dashboard is purged for good.
--
-- `dashboard_access_rule`: one level for one grantee on one item. Exactly one
-- of `user_id` and `role_id` is set (B5: a person or a role, never "everyone
-- outside"), enforced by a CHECK so no code path can store both or neither.
-- `level` is the closed set of B1. Uniqueness is per item and grantee, written
-- as two partial unique indexes because a plain UNIQUE treats NULLs as
-- distinct and would allow the same user twice.
--
-- `dashboard_restriction`: the item is marked "only the people listed" (B3):
-- the starting point from `dashboard:read` / `dashboard:write` no longer
-- applies to it or anything below it. Present means restricted. Kept apart
-- from the rules so a restriction with nobody listed is still expressible
-- (only administrators then see the item).
--
-- Nothing here changes who sees what until a row is written (B2).
CREATE TABLE IF NOT EXISTS dashboard_access_rule (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    item_kind  TEXT NOT NULL CHECK (item_kind IN ('board', 'folder')),
    item_id    TEXT NOT NULL CHECK (char_length(item_id) BETWEEN 1 AND 200),
    user_id    UUID REFERENCES app_user (id) ON DELETE CASCADE,
    role_id    UUID REFERENCES role (id) ON DELETE CASCADE,
    level      TEXT NOT NULL CHECK (level IN ('view', 'edit', 'manage')),
    granted_by UUID REFERENCES app_user (id) ON DELETE SET NULL,
    granted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((user_id IS NOT NULL) <> (role_id IS NOT NULL))
);

CREATE UNIQUE INDEX IF NOT EXISTS dashboard_access_rule_user_uq
    ON dashboard_access_rule (item_kind, item_id, user_id) WHERE user_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS dashboard_access_rule_role_uq
    ON dashboard_access_rule (item_kind, item_id, role_id) WHERE role_id IS NOT NULL;
-- "What can this person see": the lookup every dashboard request makes.
CREATE INDEX IF NOT EXISTS dashboard_access_rule_user_idx
    ON dashboard_access_rule (user_id) WHERE user_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS dashboard_access_rule_role_idx
    ON dashboard_access_rule (role_id) WHERE role_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS dashboard_restriction (
    item_kind     TEXT NOT NULL CHECK (item_kind IN ('board', 'folder')),
    item_id       TEXT NOT NULL CHECK (char_length(item_id) BETWEEN 1 AND 200),
    restricted_by UUID REFERENCES app_user (id) ON DELETE SET NULL,
    restricted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (item_kind, item_id)
);
