-- Dashboard templates: saved copies of a dashboard's definition to start new
-- dashboards from (`BI-18` part D, plan
-- `docs/superpowers/plans/2026-11-13-bi-18d-reports-templates.md` T3; feature
-- page `docs/core/features/reports-and-templates.md`, decisions T1 to T4). Read
-- by `lakehouse_store::dashboard_templates` and served by `lakehouse-api`'s
-- `routes::dashboard_templates`.
--
-- `definition` holds the dashboard file of `BI-19` part D
-- (`lakehouse_bi::dashboard_file::export`: settings, filters, tabs, layout,
-- chart definitions, the tables they read) and each chart's stored document
-- (`{ spec, def, hasYear }`, the chart row's own `spec_json`), so a dashboard
-- started from it is a copy of the charts as they were. It never holds a row of
-- data, a snapshot, a version history or any sharing state (T1). The CHECK
-- keeps it an object; the API builds and reads it.
--
-- Why Postgres: a template is deployment-wide (T4: everyone with
-- `dashboard:write` uses them) and is written once and deleted, never edited,
-- so it is an ordinary row; it belongs to no ClickHouse folder.
-- `source_board` names the `bi_board` it was saved from and has no foreign key:
-- that dashboard may be deleted later and the template stays, with the name it
-- had (`source_board_name`). `created_by` is a display name, as on a board.
--
-- No built-in templates are seeded (T3).
CREATE TABLE IF NOT EXISTS dashboard_template (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name              TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 200),
    source_board      TEXT NOT NULL DEFAULT '' CHECK (char_length(source_board) <= 200),
    source_board_name TEXT NOT NULL DEFAULT '',
    chart_count       INTEGER NOT NULL DEFAULT 0 CHECK (chart_count >= 0),
    definition        JSONB NOT NULL CHECK (jsonb_typeof(definition) = 'object'),
    created_by        TEXT NOT NULL DEFAULT '',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS dashboard_template_created_idx ON dashboard_template (created_at DESC);
