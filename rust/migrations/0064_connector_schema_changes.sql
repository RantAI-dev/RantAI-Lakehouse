-- SRC-8 (F1-F6): a source table that gains, loses or retypes a column changed
-- the Bronze table silently. Nothing remembered what a table looked like at
-- the last run, a connector had no setting for what to do about a change, and
-- a connector or a table could not be held back from loading. This migration
-- holds the state the comparison and the policy need (decisions D1-D8 on
-- `docs/core/features/source-schema-changes.md`). All additive; nothing here
-- alters a Bronze table. Never edited once applied.
--
-- 1. `connector.schema_change_policy` (D1): what a non-breaking change does.
--    A breaking change waits under every value (decision 3), so the column
--    holds only the four choices. Existing connectors start on the default,
--    which is also what a new connector gets.
--
-- 2. `connector.paused_reason` / `paused_at` (F6): the connector-level pause
--    that the "pause" policy sets and an approval lifts. Both NULL means not
--    paused; the pair is always written together by `schema_change.rs`.
--    `SRC-11`'s auto-disable reuses it, hence a free-text reason rather than
--    a column per cause. The schedule query and `POST .../ingest/run` honour it.
--
-- 3. `connector_source_schema` (F1, F3): the last ACCEPTED columns and primary
--    key of each source table, i.e. what the Bronze table was last told to
--    hold. A first observation writes it as the baseline and reports no
--    change. `waiting_columns` / `waiting_primary_key` hold the last OBSERVED
--    shape of a table that waits for a decision, because a pending change does
--    not move the baseline and approval must accept exactly what was seen
--    (not whatever the source looks like later). Both are NULL when nothing
--    waits.
--
-- 4. `connector_schema_change` (F5): one row per change. `status` is
--    `applied` (took effect when detected), `pending` (waits for a person) or
--    `approved` (a person accepted it). The partial unique index is the
--    identity of a pending change - connector, table, kind and column (the
--    empty string for the table-level kinds) - so seeing the same change on
--    the next run neither adds a row nor raises a second alert. Values are
--    type names and column names only, never source data (principle 4).
--
-- 5. `connector_inactive_column` (D8): a stored fact rather than a view over
--    approved `column_removed` rows. The Bronze table's Schema tab asks "which
--    columns of this table are inactive" for a whole table at once, and a
--    column that comes back at the source must stop being inactive, which is
--    one DELETE here (`schema_change.rs` does it when the column is accepted
--    again) instead of an ordering rule over history rows with equal times.
--
-- Rows of 3-5 are removed with their connector (ON DELETE CASCADE), as
-- `connector_probe_result` (0044) is; `delete_connector` does not clean them
-- itself.

ALTER TABLE connector
    ADD COLUMN IF NOT EXISTS schema_change_policy TEXT NOT NULL DEFAULT 'apply_non_breaking'
    CHECK (schema_change_policy IN ('apply_non_breaking', 'apply_all', 'ask_first', 'pause'));
ALTER TABLE connector ADD COLUMN IF NOT EXISTS paused_reason TEXT;
ALTER TABLE connector ADD COLUMN IF NOT EXISTS paused_at TIMESTAMPTZ;

CREATE TABLE IF NOT EXISTS connector_source_schema (
    connector_id        TEXT NOT NULL REFERENCES connector(id) ON DELETE CASCADE,
    object_name         TEXT NOT NULL,
    columns             JSONB NOT NULL,
    primary_key         JSONB NOT NULL,
    observed_at         TIMESTAMPTZ NOT NULL,
    waiting_columns     JSONB,
    waiting_primary_key JSONB,
    PRIMARY KEY (connector_id, object_name)
);

CREATE TABLE IF NOT EXISTS connector_schema_change (
    id           TEXT PRIMARY KEY,
    connector_id TEXT NOT NULL REFERENCES connector(id) ON DELETE CASCADE,
    object_name  TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN (
        'column_added', 'column_removed', 'type_changed', 'primary_key_changed', 'table_added'
    )),
    column_name  TEXT NOT NULL DEFAULT '',
    before_value TEXT,
    after_value  TEXT,
    breaking     BOOLEAN NOT NULL,
    status       TEXT NOT NULL CHECK (status IN ('applied', 'pending', 'approved')),
    run_id       TEXT,
    detected_at  TIMESTAMPTZ NOT NULL,
    decided_by   TEXT,
    decided_at   TIMESTAMPTZ
);

-- "What waits on this connector" and the "pause lifts when nothing waits" check.
CREATE INDEX IF NOT EXISTS connector_schema_change_pending_idx
    ON connector_schema_change (connector_id) WHERE status = 'pending';
-- The recent list, newest first.
CREATE INDEX IF NOT EXISTS connector_schema_change_recent_idx
    ON connector_schema_change (connector_id, detected_at DESC);
-- The identity of a pending change (see header, point 4).
CREATE UNIQUE INDEX IF NOT EXISTS connector_schema_change_pending_identity_idx
    ON connector_schema_change (connector_id, object_name, kind, column_name)
    WHERE status = 'pending';

CREATE TABLE IF NOT EXISTS connector_inactive_column (
    connector_id   TEXT NOT NULL REFERENCES connector(id) ON DELETE CASCADE,
    object_name    TEXT NOT NULL,
    column_name    TEXT NOT NULL,
    inactive_since TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (connector_id, object_name, column_name)
);
