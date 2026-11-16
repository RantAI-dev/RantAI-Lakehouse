-- Reports: written pages of text blocks with live charts between them
-- (`BI-18` part D, plan `docs/superpowers/plans/2026-11-13-bi-18d-reports-templates.md`
-- T1; feature page `docs/core/features/reports-and-templates.md`, decisions R1
-- to R4). Read by `lakehouse_store::dashboard_reports` and served by
-- `lakehouse-api`'s `routes::dashboard_reports`.
--
-- Why Postgres and not ClickHouse beside `bi_board`: a report is edited as one
-- document by people, not appended to as a log of versions, so it wants an
-- ordinary row with an `UPDATE`, and its trash is a nullable column rather
-- than a third row state. Like `dashboard_access_rule` (0103), `folder_id`
-- names a `bi_folder` row in ClickHouse and so has no foreign key; the API
-- refuses to delete a folder that still holds a report (live or in the trash),
-- so a report never outlives its folder.
--
-- `blocks` is the ordered list of blocks as JSON: heading, paragraph, list,
-- link and chart (`lakehouse_bi::reports::Block`). Text is stored as typed,
-- never as HTML; a chart block is a reference to an existing chart (its id and
-- the dashboard it is on), never its data, so every reader reads the chart as
-- themselves. The API checks the shape and the limits (at most 30 charts,
-- R4) before a row is written; the CHECK here only keeps the column a list.
--
-- `created_by`, `updated_by` and `deleted_by` are display names, as
-- `bi_board.created_by` is: a service identity may write a report too, and it
-- has no `app_user` row to reference.
--
-- `deleted_at` set means "in the trash" (`BI-18` part C's 30 days): every live
-- read asks for `deleted_at IS NULL`, and the API's hourly trash sweep removes
-- a row 30 days after it was deleted.
CREATE TABLE IF NOT EXISTS dashboard_report (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    folder_id  TEXT NOT NULL DEFAULT '' CHECK (char_length(folder_id) <= 200),
    title      TEXT NOT NULL CHECK (char_length(title) BETWEEN 1 AND 200),
    blocks     JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(blocks) = 'array'),
    created_by TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by TEXT NOT NULL DEFAULT '',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ,
    deleted_by TEXT NOT NULL DEFAULT ''
);

-- The list, and the folder delete's "is anything still in here" count.
CREATE INDEX IF NOT EXISTS dashboard_report_folder_idx ON dashboard_report (folder_id);
-- The trash list and the sweep.
CREATE INDEX IF NOT EXISTS dashboard_report_deleted_idx
    ON dashboard_report (deleted_at) WHERE deleted_at IS NOT NULL;
