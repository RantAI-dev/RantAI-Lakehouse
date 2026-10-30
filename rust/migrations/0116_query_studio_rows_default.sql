-- The Query Studio row default moves from 2,000 to 64,000 (`QS-2`; the
-- limit page `docs/core/features/query-limits-and-stop.md`, decision 2, and
-- the feature page `docs/core/features/query-studio-results.md`, decision 1).
--
-- # Why the number moves
--
-- 2,000 rows is a number for a chart. Query Studio is a workbench: a person
-- runs a statement, reads the answer and downloads it, and a cut at 2,000
-- rows hides most of what the query returned. 64,000 is the number the
-- owner signed, and it sits inside the range the column's CHECK allows
-- (100 to 100,000), so no range moves with it.
--
-- # What moves and what does not
--
-- The column's default moves, so a row written without a value gets 64,000.
-- The row migration `0101` seeded carries the old default and moves with
-- it, through a targeted UPDATE: `WHERE studio_rows = 2000` and nothing
-- else, so every other value stays as it is.
--
-- An administrator who chose exactly 2,000 by hand moves too, and that is
-- accepted: 2,000 was the code's own default, and nobody could set a value
-- at all before `BI-5` shipped on 2026-10-26, so a stored 2,000 is the old
-- default rather than a choice. An administrator who wants another number
-- sets it again in Admin > Settings > Query limits.
--
-- Number choice: 0116, the next free number above every number taken on
-- `main` (0112) and above the numbers the open pull requests hold (0103,
-- 0113, 0114 and 0115); a gap costs nothing.
ALTER TABLE query_limits
    ALTER COLUMN studio_rows SET DEFAULT 64000;

UPDATE query_limits SET studio_rows = 64000 WHERE studio_rows = 2000;
