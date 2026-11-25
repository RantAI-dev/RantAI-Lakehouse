-- DATA-65: publish Silver (`silver.*`) as well as Gold (`serving.*`) ClickHouse
-- MergeTree tables to Apache Iceberg through Lakekeeper. A publication is now
-- keyed by (zone, mart): `silver.orders` and `serving.orders` are two distinct
-- switches writing to two distinct Iceberg namespaces (`silver` and `gold`),
-- and must be able to be toggled independently without one overwriting the
-- other.
--
-- Additive and backward compatible: every row that existed before this
-- migration is a Gold publication, so the NOT DEFAULT `gold` fills it in and
-- the recreated primary key keeps one row per (gold, mart), exactly the old
-- uniqueness guarantee. The CHECK means a caller can never write a zone the
-- Rust `PublishZone` parser would refuse (fail closed at both layers).
ALTER TABLE gold_publication ADD COLUMN zone TEXT NOT NULL DEFAULT 'gold' CHECK (zone IN ('silver', 'gold'));

ALTER TABLE gold_publication DROP CONSTRAINT IF EXISTS gold_publication_pkey;
ALTER TABLE gold_publication ADD CONSTRAINT gold_publication_pkey PRIMARY KEY (zone, mart);
