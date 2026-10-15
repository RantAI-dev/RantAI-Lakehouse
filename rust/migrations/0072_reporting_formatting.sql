-- Deployment-wide number and font settings (`BI-17` part A, decision D5 of
-- `docs/superpowers/plans/2026-10-15-bi-17a-chart-formatting.md`): the number
-- locale, the company palette and the fonts dashboards may use.
--
-- Why columns on `reporting_settings` and not a table: they are three more
-- deployment-wide choices saved by the same screen (Admin > Settings) with the
-- same permission (`settings:write`) and read together with the time zone into
-- the `reporting` object of every dashboard payload, so a public link and a
-- signed embed format numbers the way the console does. A deployment that
-- never saves keeps the defaults below and no chart changes: `en-US`
-- separators, no company palette, every shipped font allowed.
--
-- The API validates before it saves (locale in the two values, palette of 2 to
-- 12 `#rrggbb` colours or empty, fonts a subset of the console's shipped
-- ids); the CHECKs are the backstop for shape, so a row edited by hand cannot
-- put free text into a page's CSS. `ADD COLUMN ... DEFAULT` keeps an existing
-- row valid. The default font list must match `lakehouse_bi::formatting::SHIPPED_FONTS`.
ALTER TABLE reporting_settings
    ADD COLUMN IF NOT EXISTS number_locale TEXT NOT NULL DEFAULT 'en-US'
        CHECK (number_locale IN ('en-US', 'id-ID')),
    ADD COLUMN IF NOT EXISTS company_palette TEXT[] NOT NULL DEFAULT '{}'
        CHECK (cardinality(company_palette) = 0
               OR (cardinality(company_palette) BETWEEN 2 AND 12
                   AND array_to_string(company_palette, ',')
                       ~ '^#[0-9a-fA-F]{6}(,#[0-9a-fA-F]{6})*$')),
    ADD COLUMN IF NOT EXISTS fonts TEXT[] NOT NULL
        DEFAULT '{inter,ibm-plex-sans,source-serif,jetbrains-mono,system}'
        CHECK (fonts <@ ARRAY['inter','ibm-plex-sans','source-serif','jetbrains-mono','system']::text[]);
