-- The sites image and web-page cards may load from (`BI-16` part B, decision
-- D4 of `docs/superpowers/plans/2026-10-15-bi-16b-simple-types.md`).
--
-- Why a column on `reporting_settings` and not a table: it is one more
-- deployment-wide list saved by the same screen (Admin > Settings) with the
-- same permission (`settings:write`) as the time zone, the number locale and
-- the fonts, and it is read together with them into the `reporting` object of
-- every dashboard payload, so a public link and a signed embed apply the same
-- list as the console. A deployment that never saves keeps the empty list:
-- no image or web card can be saved until an administrator allows a site
-- (fail closed).
--
-- The API validates before it saves (`lakehouse_bi::embed_access::
-- validate_content_sites`: `https://host[:port]`, no wildcard, at most 50);
-- the CHECKs are the backstop for shape, so a row edited by hand cannot put
-- a path, a quote or a scheme other than `https` into the list. `ADD COLUMN
-- ... DEFAULT` keeps an existing row valid.
ALTER TABLE reporting_settings
    ADD COLUMN IF NOT EXISTS content_sites TEXT[] NOT NULL DEFAULT '{}'
        CHECK (cardinality(content_sites) <= 50
               AND array_to_string(content_sites, ',')
                   ~ '^(https://[a-z0-9.-]+(:[0-9]{1,5})?(,https://[a-z0-9.-]+(:[0-9]{1,5})?)*)?$');
