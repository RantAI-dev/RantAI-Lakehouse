-- The roles that may not download (`BI-1`, decision D7 of
-- `docs/superpowers/plans/2026-10-17-bi-1-export.md`, feature decision 9).
--
-- Why a column on `reporting_settings` and not a table: it is one more
-- deployment-wide list saved by the same screen (Admin > Settings) with the
-- same permission (`settings:write`) as the time zone and the fonts. Every
-- download (PDF, Excel, CSV, the assistant's links) stays governed by
-- `dashboard:read`; this list only takes the right away from people whose
-- every role is on it (a Platform Admin always may). The default is the empty
-- list, so nothing changes until an administrator chooses.
--
-- The API checks that each name is an existing role before it saves; the
-- CHECK is the backstop for shape (at most 100 names, none empty, 10,000
-- characters in all), so a row edited by hand cannot hold an unbounded
-- list. A role deleted later simply stops matching anyone. `ADD COLUMN ...
-- DEFAULT` keeps an existing row valid.
ALTER TABLE reporting_settings
    ADD COLUMN IF NOT EXISTS download_denied_roles TEXT[] NOT NULL DEFAULT '{}'
        CHECK (cardinality(download_denied_roles) <= 100
               AND array_position(download_denied_roles, '') IS NULL
               AND length(array_to_string(download_denied_roles, ',')) <= 10000);
