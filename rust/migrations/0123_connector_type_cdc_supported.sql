-- SRC-4 slice 3 (decision S3-g): MySQL CDC and SQL Server CDC are supported
-- again. `0066` (SRC-9 decision 8) listed both as not supported because the
-- Debezium Server template could not run them. Change capture now runs per
-- connector on Kafka Connect (ADR 0016): one renderer builds the MySQL and the
-- SQL Server connector config (`lakehouse-store/src/cdc.rs`), and the gates
-- `g4-mysql` and `g4-mssql` prove each against a real source (`ops/gates/
-- coverage.json`), so the wizard may offer them.
--
-- Each UPDATE is targeted like `0066`'s, and names the exact reason text it
-- replaces (`unsupported_reason = '<0066's text>'`): a deployment that has
-- already edited a row keeps its edit, and a re-run changes nothing. Both rows
-- keep `adapter = 'cdc'`.

UPDATE connector_type
SET supported = true,
    unsupported_reason = NULL
WHERE name = 'MySQL CDC'
  AND supported = false
  AND adapter = 'cdc'
  AND unsupported_reason = 'Change capture for MySQL is not built yet: this build can only prepare it for PostgreSQL.';

UPDATE connector_type
SET supported = true,
    unsupported_reason = NULL
WHERE name = 'SQL Server CDC'
  AND supported = false
  AND adapter = 'cdc'
  AND unsupported_reason = 'Change capture for SQL Server is not built yet: this build can only prepare it for PostgreSQL.';
