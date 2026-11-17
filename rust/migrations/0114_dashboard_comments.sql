-- Comments on dashboards and the mentions in them (`BI-19` part E, decision
-- D15 of `docs/superpowers/plans/2026-10-18-bi-19-sharing.md`; feature page
-- `docs/core/features/sharing-and-delivery.md`, E1 to E3). The plan names this
-- file `0098_...`; that number went to the public links, and `0113` is the
-- newest on any branch, so this is the next free one.
--
-- Why Postgres: authors and mentioned people live here, so a deleted account
-- is handled by a foreign key instead of an id nobody can resolve. `board_id`
-- names a `bi_board` row in ClickHouse and so has no foreign key (as in
-- `0103`): a dashboard that is purged for good leaves its comments inert, and
-- the API's trash sweep removes them. A comment on a dashboard in the trash is
-- unreachable by every route (they all resolve the dashboard first), so the
-- comments follow the board and come back with it on restore.
--
-- `dashboard_comment.body` is plain text, 1 to 2000 characters, enforced here
-- as well as in the route so no code path can store an empty or oversized one.
-- A deleted comment keeps its row (`deleted_at`, `deleted_by`) as the record of
-- who removed what; no route returns its body again.
--
-- `dashboard_comment_mention`: who was mentioned in a comment, and whether they
-- have seen it (`read_at`). The bell reads this table directly, instead of a
-- separate notification store holding a title, because a stored title would
-- keep a dashboard's name after the reader lost access to it; the bell builds
-- its line from the live dashboard and re-checks access on every read. The
-- notification email and the bell never carry the comment's text (E3).
CREATE TABLE IF NOT EXISTS dashboard_comment (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    board_id   TEXT NOT NULL CHECK (char_length(board_id) BETWEEN 1 AND 200),
    author_id  UUID REFERENCES app_user (id) ON DELETE SET NULL,
    body       TEXT NOT NULL CHECK (char_length(body) BETWEEN 1 AND 2000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    edited_at  TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    deleted_by UUID REFERENCES app_user (id) ON DELETE SET NULL
);

-- "The live comments of this dashboard, oldest first": the panel's read.
CREATE INDEX IF NOT EXISTS dashboard_comment_board_idx
    ON dashboard_comment (board_id, created_at) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS dashboard_comment_mention (
    comment_id UUID NOT NULL REFERENCES dashboard_comment (id) ON DELETE CASCADE,
    user_id    UUID NOT NULL REFERENCES app_user (id) ON DELETE CASCADE,
    read_at    TIMESTAMPTZ,
    PRIMARY KEY (comment_id, user_id)
);

-- "What mentioned me": the bell's read.
CREATE INDEX IF NOT EXISTS dashboard_comment_mention_user_idx
    ON dashboard_comment_mention (user_id) WHERE read_at IS NULL;
