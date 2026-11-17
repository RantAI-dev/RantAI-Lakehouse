-- Scheduled reports by email (`RPT-1`, decision D1 of
-- `docs/superpowers/plans/2026-10-17-rpt-1-scheduled-reports.md`, feature page
-- `docs/core/features/scheduled-reports.md`).
--
-- Why Postgres and not the dashboards' ClickHouse tables: a subscription is
-- operational state that the runner must claim with a row lock
-- (`FOR UPDATE SKIP LOCKED`, feature decision 8) so two API processes never
-- send the same report, and a delivery record must be written exactly once
-- per send. ClickHouse has neither a lock nor a transaction. The dashboard
-- itself stays where it is; `board` is its id there, so it carries no foreign
-- key (a board in the trash or deleted makes the next run fail with a plain
-- reason, it does not break this table).
--
-- `report_subscription`: one schedule for one dashboard, owned by one person.
-- The report is made AS the owner (feature decision 1): their masking and row
-- filters, every recipient sees what the owner sees. `owner_id` therefore
-- cascades with the user: a deleted person's subscriptions go with them. A
-- person who is merely disabled keeps their rows; the run stops and records
-- why (it never sends as someone who can no longer sign in).
--   * The schedule is `kind` plus the fields that kind uses, in
--     `time_zone` (an IANA name; the API checks it against the zone database,
--     the CHECK is only the shape, as in `0070`). `weekday` is ISO (1 = Monday
--     .. 7 = Sunday). `month_day` is 1..28, or -1 for "the last day"; days 29
--     to 31 do not exist because they skip months. Fields a kind does not use
--     are NULL, so a row cannot say two things.
--   * `attachments` is JSON: the PDF (paper, orientation) or null, and the
--     chosen charts with their file format. `filters` and `params` are the
--     subscription's own dashboard values, in the shapes `GET /api/dashboard`
--     takes (`BI-1`); `grain` the same.
--   * `next_run_at` is the next due time. The runner moves it forward in the
--     same transaction that claims the row, computed from the claim time, so a
--     send that was missed while the API was down runs once, not once per
--     missed time (feature decision 8).
--   * `consecutive_failures` reaches 5 and the row is paused with
--     `paused_reason` (feature decision 7). A success resets it.
--
-- `report_recipient`: one row per recipient, either a console user or an
-- outside address, never both (CHECK). An address is stored lower-cased so the
-- unique index sees `A@x.com` and `a@x.com` as one. The 50-recipient limit and
-- the outside-domain list are the API's checks (the list changes without a
-- migration); a user recipient cascades with the user.
--
-- `report_delivery`: what each send did, kept 90 days by the runner (feature
-- decision 7). `reason` is always one of the product's own sentences, never
-- text from the mail server or the PDF service (SEC-11); the CHECK on length
-- only bounds it. `origin` says whether the schedule or "Send now" made it.
--
-- `reporting_settings.report_allowed_domains`: the outside domains reports may
-- go to (feature decision 3), saved by Admin > Settings with `settings:write`
-- like every other deployment-wide list there. Empty by default, so no report
-- leaves the company until an administrator adds a domain. The API validates
-- the names; the CHECK bounds the list (at most 50, none empty).
CREATE TABLE IF NOT EXISTS report_subscription (
    id                   UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    board                TEXT        NOT NULL CHECK (length(board) BETWEEN 1 AND 200),
    owner_id             UUID        NOT NULL REFERENCES app_user (id) ON DELETE CASCADE,
    kind                 TEXT        NOT NULL
        CHECK (kind IN ('hourly', 'daily', 'weekly', 'monthly')),
    minute               SMALLINT    NOT NULL CHECK (minute BETWEEN 0 AND 59),
    hour                 SMALLINT    CHECK (hour BETWEEN 0 AND 23),
    weekday              SMALLINT    CHECK (weekday BETWEEN 1 AND 7),
    month_day            SMALLINT    CHECK (month_day = -1 OR month_day BETWEEN 1 AND 28),
    time_zone            TEXT        NOT NULL
        CHECK (length(time_zone) BETWEEN 1 AND 64
               AND time_zone ~ '^[A-Za-z][A-Za-z0-9_+-]*(/[A-Za-z0-9_+-]+)*$'),
    attachments          JSONB       NOT NULL,
    filters              JSONB,
    params               JSONB,
    grain                TEXT        CHECK (length(grain) <= 40),
    skip_when_empty      BOOLEAN     NOT NULL DEFAULT FALSE,
    paused               BOOLEAN     NOT NULL DEFAULT FALSE,
    paused_reason        TEXT        CHECK (length(paused_reason) <= 300),
    consecutive_failures INTEGER     NOT NULL DEFAULT 0 CHECK (consecutive_failures >= 0),
    next_run_at          TIMESTAMPTZ NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The fields a kind uses, and only those.
    CONSTRAINT report_subscription_fields CHECK (
        (kind = 'hourly'  AND hour IS NULL     AND weekday IS NULL     AND month_day IS NULL)
     OR (kind = 'daily'   AND hour IS NOT NULL AND weekday IS NULL     AND month_day IS NULL)
     OR (kind = 'weekly'  AND hour IS NOT NULL AND weekday IS NOT NULL AND month_day IS NULL)
     OR (kind = 'monthly' AND hour IS NOT NULL AND weekday IS NULL     AND month_day IS NOT NULL)
    )
);

-- The runner's claim reads "not paused and due", oldest first.
CREATE INDEX IF NOT EXISTS report_subscription_due
    ON report_subscription (next_run_at) WHERE NOT paused;
CREATE INDEX IF NOT EXISTS report_subscription_board
    ON report_subscription (board);
CREATE INDEX IF NOT EXISTS report_subscription_owner
    ON report_subscription (owner_id);

CREATE TABLE IF NOT EXISTS report_recipient (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subscription_id UUID NOT NULL REFERENCES report_subscription (id) ON DELETE CASCADE,
    user_id         UUID REFERENCES app_user (id) ON DELETE CASCADE,
    email           TEXT CHECK (email = lower(email) AND length(email) BETWEEN 3 AND 254),
    -- Either a console user or an address, never both and never neither.
    CONSTRAINT report_recipient_one_kind CHECK (num_nonnulls(user_id, email) = 1)
);
CREATE UNIQUE INDEX IF NOT EXISTS report_recipient_user
    ON report_recipient (subscription_id, user_id) WHERE user_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS report_recipient_email
    ON report_recipient (subscription_id, email) WHERE email IS NOT NULL;

CREATE TABLE IF NOT EXISTS report_delivery (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subscription_id UUID        NOT NULL REFERENCES report_subscription (id) ON DELETE CASCADE,
    started_at      TIMESTAMPTZ NOT NULL,
    finished_at     TIMESTAMPTZ NOT NULL,
    status          TEXT        NOT NULL CHECK (status IN ('sent', 'failed', 'skipped')),
    origin          TEXT        NOT NULL DEFAULT 'schedule'
        CHECK (origin IN ('schedule', 'manual')),
    recipient_count INTEGER     NOT NULL CHECK (recipient_count >= 0),
    reason          TEXT        CHECK (length(reason) <= 500)
);
CREATE INDEX IF NOT EXISTS report_delivery_by_subscription
    ON report_delivery (subscription_id, started_at DESC);
-- The 90-day removal reads by age alone.
CREATE INDEX IF NOT EXISTS report_delivery_by_age ON report_delivery (started_at);

ALTER TABLE reporting_settings
    ADD COLUMN IF NOT EXISTS report_allowed_domains TEXT[] NOT NULL DEFAULT '{}'
        CHECK (cardinality(report_allowed_domains) <= 50
               AND array_position(report_allowed_domains, '') IS NULL
               AND length(array_to_string(report_allowed_domains, ',')) <= 10000);
