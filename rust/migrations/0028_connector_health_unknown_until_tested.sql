-- WS1 finding J19: a brand-new connector claimed, at creation, to already be
-- `healthy` and to have been tested "just now" -- before any probe had ever
-- run. `0013_connectors.sql` defaulted `health` to `'healthy'` and
-- `last_test_at`/`last_activity_at` to `now()`, and `create_connector`
-- inserted the literal `'healthy'`. None of that was ever measured: it was a
-- fabricated starting state, exactly the "invented success" this workstream
-- exists to remove.
--
-- `health` now defaults to `'unknown'` (already a valid value under
-- `connector_health_check`) and `last_test_at`/`last_activity_at` become
-- nullable with no default, so "never probed" is representable and true:
-- `lastTestAt: null` until `record_test_result` actually runs, and
-- `lastActivityAt` stays nullable because no writer for it exists anywhere
-- in this codebase yet -- WS3's ingest-run tracking is the planned source
-- (see the store module doc comment). Dropping the column instead of making
-- it nullable was considered and rejected: the contract
-- (`contracts/connectors.ts`) already has a `lastActivityAt` field, and a
-- future WS3 writer needs the column to exist to fill it in.
--
-- The two connectors `0022_prune_connector_seed.sql` seeds
-- (`conn-pg-lakehouse`, `conn-s3-warehouse`) carry the same fabrication:
-- that migration's INSERT stamps `last_test_at = now() - interval '30
-- minutes'` against `created_at = now()` in the same statement, i.e.
-- `last_test_at` thirty minutes BEFORE `created_at` -- nothing had tested
-- either connector at seed time. A real probe, run by `record_test_result`
-- (see that function, this task), can only ever stamp `last_test_at` AFTER
-- the row's `created_at`. So the predicate below --
-- `last_test_at < created_at` -- targets exactly the fabricated seed
-- stamp and never touches a row where a deployment has since run a genuine
-- test (whose `last_test_at` would be >= `created_at`), matching this
-- migration file set's established "match the seed's own fabricated values,
-- never a blanket UPDATE" convention (see `0027_prune_seeded_activity.sql`).
ALTER TABLE connector ALTER COLUMN health SET DEFAULT 'unknown';
ALTER TABLE connector ALTER COLUMN last_test_at DROP NOT NULL, ALTER COLUMN last_test_at DROP DEFAULT;
ALTER TABLE connector ALTER COLUMN last_activity_at DROP NOT NULL, ALTER COLUMN last_activity_at DROP DEFAULT;

UPDATE connector SET health = 'unknown', last_test_at = NULL
 WHERE id IN ('conn-pg-lakehouse', 'conn-s3-warehouse')
   AND health = 'healthy' AND last_test_at < created_at;
