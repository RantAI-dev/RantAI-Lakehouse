-- Public links that expire, with an optional password and an administrator
-- switch (`BI-19` part A, decision D1 of
-- `docs/superpowers/plans/2026-10-18-bi-19-sharing.md`; feature page
-- `docs/core/features/sharing-and-delivery.md`, decisions A1 to A8).
--
-- Why Postgres and not the dashboard row in ClickHouse: the link used to be a
-- column of `console.bi_board` (`public_token`), and every dashboard save is a
-- full-row INSERT that carries the token it read. A save that read the board
-- before someone pressed "switch off" therefore wrote the old token back and
-- revived the link (the same race `PR #105 review SHOULD-FIX 1` found for the
-- embed state). Here a link changes only through its own single statements,
-- and a dashboard save never touches it. The ClickHouse column stays, unread
-- (ClickHouse `ALTER ... DROP` is out of scope); `board_id` is the
-- dashboard's id there, so it carries no foreign key (a dashboard in the
-- trash or purged makes the link answer "does not work", it does not break
-- this table).
--
-- `dashboard_public_link`: at most one link per dashboard (feature A6, the
-- primary key); making a new one replaces the row, so the old token stops at
-- once. `expires_at` is required (feature A1): no link lasts forever.
--   * `token` is what the URL carries and is unique; it is a lookup key, not
--     a secret kept hashed, because the Share dialog shows the link again.
--   * `password_hash` is an `Argon2id` hash, never the password (feature A3).
--     NULL: no password. `password_version` is bumped whenever the password
--     changes or is removed, which ends every ticket issued before.
--   * `created_by` is the person who made it; it follows a deleted user to
--     NULL, the link stays.
--
-- `dashboard_public_link_ticket`: what a visitor holds after typing the
-- password once (feature item 2: once per browser session). Only the SHA-256
-- of the ticket is stored, as for session tokens. A ticket names the link's
-- `password_version`; the link's row disappearing (switch off, a new link)
-- deletes its tickets (cascade), and a password change makes them stale.
--
-- `dashboard_public_link_copy`: one row, written in the same transaction that
-- copies the links that existed before this migration out of ClickHouse
-- (feature A2: they expire 30 days after the upgrade). Without it the copy
-- would run again at every start and bring back a link its owner had ended
-- since, because the old column is never cleared. The plan did not name this
-- table; `ON CONFLICT DO NOTHING` alone is not enough for that reason.
--
-- `reporting_settings.public_links_enabled`: the administrator's switch
-- (feature A7). On at first, as before. Off stops every link at once without
-- deleting any, so switching it on again brings back the links that have not
-- expired. `ADD COLUMN ... DEFAULT` keeps an existing row valid.
CREATE TABLE IF NOT EXISTS dashboard_public_link (
    board_id         TEXT PRIMARY KEY,
    token            TEXT NOT NULL UNIQUE CHECK (char_length(token) BETWEEN 8 AND 200),
    expires_at       TIMESTAMPTZ NOT NULL,
    password_hash    TEXT,
    password_version INT NOT NULL DEFAULT 0,
    created_by       UUID REFERENCES app_user (id) ON DELETE SET NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS dashboard_public_link_ticket (
    ticket_hash      TEXT PRIMARY KEY,
    board_id         TEXT NOT NULL REFERENCES dashboard_public_link (board_id) ON DELETE CASCADE,
    password_version INT NOT NULL,
    expires_at       TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS dashboard_public_link_ticket_expires_idx
    ON dashboard_public_link_ticket (expires_at);

CREATE TABLE IF NOT EXISTS dashboard_public_link_copy (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    copied    INT NOT NULL CHECK (copied >= 0),
    done_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE reporting_settings
    ADD COLUMN IF NOT EXISTS public_links_enabled BOOLEAN NOT NULL DEFAULT TRUE;
