-- SRC-4 slice 6 (decision S6-a): `Oracle CDC` is a connector type the wizard
-- offers. It is a NEW row, not a change to the batch `Oracle` type: the batch
-- adapter (`adapter = 'sql'`) reads a table in full on every run, while change
-- capture reads the redo log through Debezium's `LogMiner` on Kafka Connect
-- (ADR 0016) and keeps Bronze current. They are two connector types with two
-- dials (`CdcDial` with `driver = "oracle"`, `lakehouse-store/src/
-- ingest_spec.rs`), so a person picks the one they mean.
--
-- `adapter = 'cdc'`, as `PostgreSQL CDC`, `MySQL CDC`, `SQL Server CDC` and
-- `MongoDB CDC`: the dial's `driver` tells the shapes apart, not the adapter
-- column. `supported = true` because the gate `g4-oracle`
-- (`ops/g4/g4_oracle_test.py`) proves the type against a real Oracle Free
-- (`ops/gates/coverage.json`); `docs_url` stays NULL (no invented link,
-- AGENTS.md rule 2).
--
-- `ON CONFLICT DO NOTHING`: a deployment that already added a row of this name
-- by hand keeps its row, and a re-run changes nothing. The statement has the
-- shape `ops/lint/check_connector_gate_coverage.py` replays.

INSERT INTO connector_type (name, adapter, supported, docs_url)
VALUES
    ('Oracle CDC', 'cdc', true, NULL)
ON CONFLICT DO NOTHING;
