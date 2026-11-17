-- Whether an uploaded map's region lookup exists on the data engine
-- (`BI-16` part C3, decision D8 of
-- `docs/superpowers/plans/2026-10-24-bi-16c-advanced-charts-maps.md`).
--
-- `RegionOf(map, lat, lon)` reads a ClickHouse polygon dictionary that the API
-- creates after an upload (`console.geo_region` holds the full boundaries,
-- `console.geo_dict_<id>` is the dictionary over them). Creating it can fail
-- for reasons outside Postgres (a grant, a dictionary the engine cannot load),
-- and the map must stay usable for drawing when it does. This flag records the
-- outcome, so the formula compiler can say "RegionOf is not available for this
-- map" instead of emitting a call that would fail on the engine. It is set only
-- after the dictionary has been created and loaded; maps uploaded before this
-- migration have no lookup (false) until they are uploaded again.
--
-- Not edited once applied; a later change is a new migration.

ALTER TABLE geo_map ADD COLUMN region_lookup BOOLEAN NOT NULL DEFAULT FALSE;
