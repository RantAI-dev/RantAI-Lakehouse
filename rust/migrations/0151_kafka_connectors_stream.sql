-- SRC-44 (QA LH-SRC-6-41): repair Kafka connectors saved as a batch.
--
-- The console saved every connector that is not change capture with
-- ingest_mode 'batch', Kafka included. The loader runs a `kafka` connector
-- only as 'stream' (dagster ingest_factory.py), so every run of such a row
-- stopped with UnknownAdapter. The console and the ingest-spec route now pair
-- the two (`validate_ingest_mode`); this moves the rows already saved wrong.
--
-- Targeted: only a kafka row still at the wrong value 'batch' is touched, so a
-- row already 'stream' is left alone and re-running it changes nothing.

UPDATE connector
   SET ingest_mode = 'stream'
 WHERE adapter = 'kafka'
   AND ingest_mode = 'batch';
