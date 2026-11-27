-- DATA-66: how closely a switched-on publication follows ClickHouse, per
-- (zone, mart). `on_update` syncs after each pipeline write and after a
-- change the watcher notices; `scheduled` syncs at one hourly/daily slot in
-- an IANA zone; `manual` syncs only when a person presses "Publish now".
--
-- Every row that existed before this migration was exported after any
-- authored pipeline and nightly at 04:00, which is the nearest thing this
-- schema has to `on_update` (feature page decision 4), so the NOT NULL
-- DEFAULT fills them in without a backfill statement. A row created by
-- switching a mart on gets the same default (decision 5).
--
-- The table CHECK is the storage half of the API's refusal: a `scheduled`
-- row always has its kind, minute and zone, a `daily` row has an hour, and
-- an `hourly` row (or a non-scheduled one) has none. Fail closed at both
-- layers, like `0157`'s zone CHECK.
ALTER TABLE gold_publication
    ADD COLUMN sync_mode TEXT NOT NULL DEFAULT 'on_update'
        CHECK (sync_mode IN ('on_update', 'scheduled', 'manual')),
    ADD COLUMN sync_schedule_kind TEXT NULL
        CHECK (sync_schedule_kind IN ('hourly', 'daily')),
    ADD COLUMN sync_hour SMALLINT NULL
        CHECK (sync_hour BETWEEN 0 AND 23),
    ADD COLUMN sync_minute SMALLINT NULL
        CHECK (sync_minute BETWEEN 0 AND 59),
    ADD COLUMN sync_time_zone TEXT NULL;

ALTER TABLE gold_publication
    ADD CONSTRAINT gold_publication_sync_schedule_consistent CHECK (
        (sync_mode = 'scheduled')
            = (sync_schedule_kind IS NOT NULL
               AND sync_minute IS NOT NULL
               AND sync_time_zone IS NOT NULL)
        AND (sync_schedule_kind IS DISTINCT FROM 'daily' OR sync_hour IS NOT NULL)
        AND (sync_schedule_kind IS DISTINCT FROM 'hourly' OR sync_hour IS NULL)
        AND (sync_schedule_kind IS NOT NULL OR sync_hour IS NULL)
    );
