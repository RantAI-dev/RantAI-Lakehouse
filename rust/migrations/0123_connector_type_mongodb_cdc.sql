-- SRC-4 slice 5 (decision S5-g): `MongoDB CDC` is a connector type the wizard
-- offers. It is a NEW row, not a change to the batch `MongoDB` type: the batch
-- adapter (`adapter = 'mongodb'`, `0043`) reads a collection in full on every
-- run, while change capture reads the replica set's change stream through
-- Debezium on Kafka Connect (ADR 0016) and keeps Bronze current. They are two
-- connector types with two dials (`Dial::CdcMongo`, `lakehouse-store/src/
-- ingest_spec.rs`), so a person picks the one they mean.
--
-- `adapter = 'cdc'`, as `PostgreSQL CDC`, `MySQL CDC` and `SQL Server CDC`: the
-- dial's `driver` (`mongodb`) tells the shapes apart, not the adapter column.
-- `supported = true` because the gate `g4-mongo` (`ops/g4/g4_mongo_test.py`)
-- proves the type against a real replica set (`ops/gates/coverage.json`);
-- `docs_url` stays NULL (no invented link, AGENTS.md rule 2).
--
-- `ON CONFLICT DO NOTHING`: a deployment that already added a row of this name
-- by hand keeps its row, and a re-run changes nothing. The statement has the
-- shape `ops/lint/check_connector_gate_coverage.py` replays.

INSERT INTO connector_type (name, adapter, supported, docs_url)
VALUES
    ('MongoDB CDC', 'cdc', true, NULL)
ON CONFLICT DO NOTHING;
