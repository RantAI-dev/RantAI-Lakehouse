-- Per-principal counter of the refusals at the gate (SEC-75, plan
-- adm-130, decision 4).
--
-- The gate writes one `refused` row for every 403 it answers, so a
-- signed-in caller (or a session that has been revoked but is still
-- replayed) can grow `audit_event` without limit. Since #546 the table
-- is append-only, so nothing trims it.
--
-- This table counts the refusals per PRINCIPAL, per METHOD and per
-- ROUTE PATTERN, per window: the first refusal of a principal and a
-- route writes the row, the later ones only count, and the sweep
-- writes one summary row for the window when it has closed. The method
-- is part of the key because the same pattern answers different
-- methods with different permissions, and a caller refused on a `POST`
-- and on a `DELETE` of the same route is two refusals.
--
-- `window_started_at` is a fixed wall-clock boundary computed IN SQL
-- (`to_timestamp(floor(extract(epoch from now()) / 900) * 900)`), the
-- same expression the per-source counter uses, so every process of the
-- API counts into the same window.
--
-- `action` and `resource_kind` are the route's own audit action and the
-- kind of the object it changes, kept here so the summary row the sweep
-- writes names the same things the first row did.
--
-- `actor_label` is the principal's display name, kept for the same
-- reason: the summary row is written later, by a job that does not
-- look the principal up again.
--
-- Not append-only: the counter is updated in place, and the sweep
-- deletes the rows it has summarised. No entry in `db_roles.rs` (the
-- list there holds the tables with a REFUSING trigger).
CREATE TABLE IF NOT EXISTS gate_refusal_throttle (
    principal_id      TEXT NOT NULL,
    principal_kind    TEXT,
    actor_label       TEXT NOT NULL,
    method            TEXT NOT NULL,
    route_pattern     TEXT NOT NULL,
    action            TEXT NOT NULL,
    resource_kind     TEXT,
    window_started_at TIMESTAMPTZ NOT NULL,
    refusals          INT NOT NULL CHECK (refusals >= 0),
    first_row_written BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (principal_id, method, route_pattern, window_started_at)
);

-- The sweep reads the windows that have closed; without this index it
-- would read the whole table once a minute.
CREATE INDEX IF NOT EXISTS gate_refusal_throttle_window_idx
    ON gate_refusal_throttle (window_started_at);
