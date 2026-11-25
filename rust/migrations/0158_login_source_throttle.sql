-- Per-source sign-in counter (SEC-75, plan adm-130, decision 2).
--
-- The per-account throttle (`login_throttle`, SEC-2) cannot stop an
-- anonymous caller from growing the audit trail: every attempt with a
-- new identifier is a new key, so the account lock never fires. This
-- table counts the FAILED sign-ins per SOURCE (the address a request
-- came from -- see `client_source`), so one caller cannot spend the
-- whole installation's sign-in budget.
--
-- `source` is the address in plain text: it is not a credential, it is
-- the thing an operator reads to find out who is hammering the sign-in
-- form, and the plan (decision 5) says the counter table holds it in
-- plain text. It is purged with the row (decision 3's sweep).
--
-- The key is (source, window) and the window is a fixed wall-clock
-- boundary computed IN SQL (`to_timestamp(floor(extract(epoch from
-- now()) / 900) * 900)`), so every process of the API counts into the
-- same window and no caller can slip a failure into a window of its
-- own. `failures` is the count within that window, CHECK (>= 0).
--
-- `first_row_written` says whether the FIRST refused row of this source
-- and window has been written to the audit trail. The audit-compaction
-- step (decision 3) writes that first row and sets the flag; later
-- failures in the same window only count, and the sweep writes one
-- summary row for the window. The step that counts the failures (step 2)
-- leaves the flag alone: it changes no audit behaviour.
--
-- Not append-only: the counter is updated in place, and the sweep
-- deletes the rows it has summarised. No entry in `db_roles.rs` (the
-- list there holds the tables with a REFUSING trigger).
CREATE TABLE IF NOT EXISTS login_source_throttle (
    source            TEXT NOT NULL,
    window_started_at TIMESTAMPTZ NOT NULL,
    failures          INT NOT NULL CHECK (failures >= 0),
    first_row_written BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (source, window_started_at)
);

-- The sweep reads the windows that have closed; without this index it
-- would read the whole table once a minute.
CREATE INDEX IF NOT EXISTS login_source_throttle_window_idx
    ON login_source_throttle (window_started_at);
