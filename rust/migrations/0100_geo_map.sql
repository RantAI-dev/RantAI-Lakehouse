-- Uploaded maps (`BI-16` part C2b, decision D5 of
-- `docs/superpowers/plans/2026-10-24-bi-16c-advanced-charts-maps.md`).
--
-- An administrator uploads a GeoJSON boundary file; the API checks it
-- (`lakehouse_bi::geo`), simplifies it for drawing and stores the result here.
-- A chart then refers to the map by `id` in its `map` field, the same field
-- the bundled maps use, and the console loads the file through
-- `GET /api/maps/{id}/geojson`.
--
-- Why Postgres: a map is configuration an administrator owns, the
-- delete-in-use check needs a single owner of the id, and the console's chart
-- store (ClickHouse) is for charts and boards. Why `bytea` and not a bucket
-- object: the file is at most 5 MB after simplification (the CHECK below is the
-- backstop for the API's cap), Postgres compresses large values on its own
-- (TOAST), and one store means one backup and no object to orphan.
--
-- `id` is made by the server (`m_` and 12 lower-case hex digits) so it is safe
-- to put in an identifier later (the spatial functions of part C3 build a
-- dictionary name from it), and the CHECK pins that shape. `name` is the label
-- people see in the map picker; two maps may not share one (case ignored) so
-- the picker never shows two entries that look the same. `name_property` is
-- the property of the uploaded file that held each region's name: kept so the
-- admin page can say what was chosen. There is no tenant column: a map is
-- visible to everyone who can make charts (feature decision 4), as the bundled
-- maps are; only `settings:write` uploads, renames or deletes.
--
-- Not edited once applied; a later change is a new migration.

CREATE TABLE geo_map (
    id            TEXT        PRIMARY KEY CHECK (id ~ '^m_[0-9a-f]{12}$'),
    name          TEXT        NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    name_property TEXT        NOT NULL CHECK (char_length(name_property) BETWEEN 1 AND 64),
    region_count  INTEGER     NOT NULL CHECK (region_count BETWEEN 1 AND 20000),
    geojson       BYTEA       NOT NULL CHECK (octet_length(geojson) BETWEEN 1 AND 5242880),
    created_by    TEXT        NOT NULL DEFAULT '' CHECK (char_length(created_by) <= 200),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX geo_map_name_key ON geo_map (lower(name));
