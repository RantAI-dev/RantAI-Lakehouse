-- Per-user Home layout: which cards and which "create" shortcuts the Home
-- page shows, and in what order, so the choice follows a person across
-- browsers the way dashboards do.
--
-- `owner` is the same key Copilot sessions are scoped to
-- (`routes::ai::session_owner`: the signed-in principal's id rendered as a
-- UUID string), as TEXT and not a foreign key to `app_user`. Sessions live
-- in `ClickHouse` and key on the same string; mirroring that keeps one
-- ownership model for per-person console state, and a service identity
-- or a deleted user needs no cascade for what is only a display preference.
-- A request with no principal never reaches this table (401).
--
-- `layout` is JSONB, validated by the API on write (list lengths, id
-- charset, no duplicates; see `routes::home`) and deliberately NOT by the
-- schema: the console owns the catalogue of card and shortcut ids and drops
-- ids it does not recognise when it reads, so adding a card later needs no
-- migration and no data fix. No row means "the default layout", which is
-- what `DELETE /api/home/layout` returns a user to.
CREATE TABLE IF NOT EXISTS home_layout (
    owner      TEXT        PRIMARY KEY,
    layout     JSONB       NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
