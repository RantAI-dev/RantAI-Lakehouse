-- Saved queries get a place, a history, a trash and a set of rules
-- (backlog `QS-3`, feature page `docs/core/features/saved-queries.md`).
--
-- WHY: every organisation has two spaces for its saved queries. The
-- **shared space** is the root: every member of the organisation sees
-- what is in it and may run it. The **personal space** is one folder
-- per person: only that person sees what is in it, unless somebody is
-- given a rule on it. A saved query also needs a history (so a change
-- can be undone), a way into the trash (so a deletion can be undone) and
-- rules that say who may do what with it (so a query can be shared
-- without being handed over). The table had no place for any of that.
--
-- # The tables
--
-- `saved_query_folder`: the tree the queries sit in. A folder belongs to
-- one tenant, sits under another folder (`parent_id`) or at the root
-- (`parent_id IS NULL`), and carries a name of 1 to 100 characters. A
-- **personal** folder (`is_personal`) belongs to one person
-- (`owner_id`); the partial unique index makes it one per tenant and
-- person, so a person's personal space is created once and found again
-- rather than made twice.
--
-- `saved_query_access_rule`: who may do what with one query or one
-- folder. Exactly one of `query_id`/`folder_id` names the item, and
-- exactly one of `user_id`/`role_id` names the grantee. The level is
-- `view` (see it and run it), `edit` (change its SQL, its title, its
-- tags and the folder it sits in) or `manage` (share it, delete it,
-- restore it, see its entry in the trash). A rule on a folder covers
-- everything inside it.
--
-- `saved_query_version`: the history of one query. Every change of the
-- SQL or of the title writes a version in the same transaction as the
-- change, so the history never disagrees with the row it describes.
--
-- # The columns `saved_query` gains
--
-- `folder_id`: the folder the query sits in; `NULL` is the shared
-- space. `deleted_at` and `deleted_by`: when the query went to the
-- trash and who put it there; `NULL` on both is a live query, and every
-- reader skips a trashed row.
--
-- # The backfill
--
-- Every row that existed before this migration stays where it is: the
-- shared space (`folder_id` stays `NULL`), because a row written before
-- the organisation had spaces belongs to the organisation and not to one
-- person. Each of them gets version 1, labelled `Saved before history`,
-- holding the title and the SQL it has today, so the history of an old
-- query starts where this migration found it instead of pretending the
-- query was written later. No rule rows are written: the author of a
-- query holds `manage` on it by the rule itself, not by a row.

CREATE TABLE saved_query_folder (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    parent_id   UUID REFERENCES saved_query_folder(id) ON DELETE RESTRICT,
    name        TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    is_personal BOOLEAN NOT NULL DEFAULT false,
    owner_id    UUID,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One personal space per tenant and person.
CREATE UNIQUE INDEX saved_query_folder_personal_uq
    ON saved_query_folder (tenant_id, owner_id) WHERE is_personal;
CREATE INDEX saved_query_folder_parent_idx ON saved_query_folder (parent_id);
CREATE INDEX saved_query_folder_tenant_idx ON saved_query_folder (tenant_id, parent_id);

ALTER TABLE saved_query
    ADD COLUMN folder_id UUID REFERENCES saved_query_folder(id) ON DELETE RESTRICT,
    ADD COLUMN deleted_at TIMESTAMPTZ,
    ADD COLUMN deleted_by UUID;

-- The live rows of one tenant, per folder, newest first.
CREATE INDEX saved_query_live_idx
    ON saved_query (tenant_id, folder_id, updated_at DESC) WHERE deleted_at IS NULL;
-- The rows waiting to be swept out of the trash.
CREATE INDEX saved_query_trash_idx
    ON saved_query (deleted_at) WHERE deleted_at IS NOT NULL;

CREATE TABLE saved_query_access_rule (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    query_id   UUID REFERENCES saved_query(id) ON DELETE CASCADE,
    folder_id  UUID REFERENCES saved_query_folder(id) ON DELETE CASCADE,
    user_id    UUID REFERENCES app_user(id) ON DELETE CASCADE,
    role_id    UUID REFERENCES role(id) ON DELETE CASCADE,
    level      TEXT NOT NULL CHECK (level IN ('view', 'edit', 'manage')),
    granted_by UUID REFERENCES app_user(id) ON DELETE SET NULL,
    granted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((query_id IS NOT NULL) <> (folder_id IS NOT NULL)),
    CHECK ((user_id IS NOT NULL) <> (role_id IS NOT NULL))
);

-- One rule per grantee per item, per kind of grantee.
CREATE UNIQUE INDEX saved_query_access_rule_user_q_uq
    ON saved_query_access_rule (query_id, user_id) WHERE user_id IS NOT NULL;
CREATE UNIQUE INDEX saved_query_access_rule_user_f_uq
    ON saved_query_access_rule (folder_id, user_id) WHERE user_id IS NOT NULL;
CREATE UNIQUE INDEX saved_query_access_rule_role_q_uq
    ON saved_query_access_rule (query_id, role_id) WHERE role_id IS NOT NULL;
CREATE UNIQUE INDEX saved_query_access_rule_role_f_uq
    ON saved_query_access_rule (folder_id, role_id) WHERE role_id IS NOT NULL;
CREATE INDEX saved_query_access_rule_user_idx
    ON saved_query_access_rule (user_id) WHERE user_id IS NOT NULL;
CREATE INDEX saved_query_access_rule_role_idx
    ON saved_query_access_rule (role_id) WHERE role_id IS NOT NULL;

CREATE TABLE saved_query_version (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    query_id    UUID NOT NULL REFERENCES saved_query(id) ON DELETE CASCADE,
    version     INT NOT NULL,
    title       TEXT NOT NULL,
    sql         TEXT NOT NULL,
    author_id   UUID,
    author_name TEXT NOT NULL,
    label       TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (query_id, version)
);

CREATE INDEX saved_query_version_query_idx ON saved_query_version (query_id, version DESC);

-- The first version of every row that existed before this migration.
INSERT INTO saved_query_version
    (query_id, version, title, sql, author_id, author_name, label, created_at)
SELECT id, 1, title, sql, owner_id, owner, 'Saved before history', created_at
  FROM saved_query;
