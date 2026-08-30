-- Phase 2, Task 2.6: alert INSTANCES (fired occurrences with ack/resolve
-- lifecycle state) for `OverviewService.listAlerts`/`acknowledgeAlert`/
-- `resolveAlert`.
--
-- WHY POSTGRES, NOT ClickHouse (`console.alert_rule`'s database):
--
-- `lakehouse-alerts` already owns *rule definitions* in `console.alert_rule`
-- (ClickHouse) -- threshold config that's written rarely and evaluated in
-- batch by `run_rules`. An `AlertItem` (this table) is a different kind of
-- object: one row per *fired occurrence*, whose `status` field is mutated
-- by a human clicking "acknowledge" or "resolve" at an arbitrary later
-- time, from a different request, possibly a different operator, than
-- whatever fired it. That is exactly the OLTP write pattern
-- `lakehouse-store`'s crate doc comment describes ClickHouse as "a poor fit
-- for": ClickHouse's MergeTree family has no cheap in-place row UPDATE --
-- doing this in ClickHouse would mean either async `ALTER TABLE ... UPDATE`
-- mutations (slow, eventually consistent -- a user clicking "acknowledge"
-- would not reliably see it reflected on the next read) or a
-- ReplacingMergeTree/CollapsingMergeTree versioned-row dance to fake
-- updates. Postgres gives a plain, immediately-consistent `UPDATE ... WHERE
-- id = $1` instead, which is what an ack/resolve click actually needs.
--
-- WHAT THIS IS NOT: a live pipeline from `alert_rule` evaluation into this
-- table. `run_rules` (lakehouse-alerts) only evaluates rules and delivers
-- webhook/email notifications today -- it does not yet write a row here
-- when a rule fires. Wiring that up is future work (it would mean
-- `run_rules` gaining a Postgres dependency it doesn't have today); this
-- migration only backs the read/ack/resolve surface `OverviewService`
-- exposes, seeded with the `mock/overview.ts` fixtures so the Alerts panel
-- isn't empty, same convention as every other Task 2.x seed migration.
CREATE TABLE alert_instance (
    id                TEXT PRIMARY KEY,
    title             TEXT NOT NULL,
    severity          TEXT NOT NULL,
    source            TEXT NOT NULL,
    affected          TEXT NOT NULL,
    status            TEXT NOT NULL DEFAULT 'open',
    assignee          TEXT,
    at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    detail            TEXT NOT NULL DEFAULT '',
    resolution_note   TEXT,
    href              TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT alert_instance_severity_check
        CHECK (severity IN ('critical', 'high', 'medium', 'low', 'info')),
    CONSTRAINT alert_instance_status_check
        CHECK (status IN ('open', 'acknowledged', 'resolved'))
);
