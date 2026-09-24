-- Query Studio: per-user history, and saved queries the console can write.
--
-- WHY: `query_history` recorded every run under the literal string
-- "anonymous" and `list_history` returned the whole table, so every user
-- read every other user's SQL. Ownership is recorded from here on;
-- `owner_id` stays nullable because the rows written before this migration
-- belong to nobody, and guessing an owner for them would be worse than
-- leaving them out (the same treatment `chat_session.owner_id` got).
ALTER TABLE query_history ADD COLUMN owner_id UUID;

-- Every history read is "my rows, newest first".
CREATE INDEX query_history_owner_at_idx ON query_history (owner_id, at DESC);

-- Saved queries are shared team assets, but a row still has an author:
-- `owner` is the display name shown in the table, `owner_id` is who wrote
-- it. Seeded rows keep a NULL `owner_id` — they were written by nobody.
ALTER TABLE saved_query ADD COLUMN owner_id UUID;

-- The two seeded saved queries (0006_seed_queries.sql) point at tables
-- that do not exist in this lakehouse (`gold.revenue`, `hot.customers`,
-- `lake.orders_history`), so the only two examples a new user could open
-- both failed on Run. Point them at marts that are actually served.
-- Targeted on id AND the seed file's original title, per AGENTS.md: a
-- deployment that reused one of these ids for its own row keeps that row.
UPDATE saved_query
   SET title = 'Top destinations by visitors',
       sql = 'SELECT destinasi, wisnus, wisman, total
FROM serving.mart_kunjungan_dtw
ORDER BY total DESC
LIMIT 10'
 WHERE id = '99999999-9999-4999-8999-000000000001'
   AND title = 'Revenue by region';

UPDATE saved_query
   SET title = 'Foreign arrivals by region',
       sql = 'SELECT kawasan, sum(jumlah) AS jumlah
FROM serving.mart_wisman
GROUP BY kawasan
ORDER BY jumlah DESC',
       tags = ARRAY['tourism']
 WHERE id = '99999999-9999-4999-8999-000000000002'
   AND title = 'Hot + cold customer join';
