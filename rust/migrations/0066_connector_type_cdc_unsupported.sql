-- SRC-9 (decision 8): MySQL CDC and SQL Server CDC were seeded
-- `supported = true` in `0035`, but this build cannot run either of them.
-- `GET /api/connectors/{id}/debezium-properties` renders one
-- PostgreSQL-shaped Debezium template and only swaps the connector class
-- (`lakehouse-store/src/cdc.rs`). Debezium 3.6.3 rejects it for MySQL (no
-- `database.server.id`, PostgreSQL-only `pgoutput`/slot/publication
-- properties) and for SQL Server (no `database.names`), so a Debezium Server
-- started from it fails configuration validation. The wizard offered two
-- types that cannot work (AGENTS.md rule 2, never fabricate).
--
-- Both rows are listed as not supported with an `unsupported_reason` that
-- the wizard shows on the disabled tile (the column came with `0062`).
--
-- Both keep `adapter = 'cdc'` on purpose, for the reason `0062` gives for
-- Google Sheets: connectors already created with these types must still open
-- in the edit page, which resolves the dial form from the adapter. Only the
-- listing says "not supported"; the Debezium settings route refuses them.
--
-- Each UPDATE is targeted (`supported = true AND adapter = 'cdc'`), like
-- `0043` and `0062`, so a deployment that has already edited a row keeps its
-- edit and a re-run changes nothing.

UPDATE connector_type
SET supported = false,
    unsupported_reason = 'Change capture for MySQL is not built yet: this build can only prepare it for PostgreSQL.'
WHERE name = 'MySQL CDC'
  AND supported = true
  AND adapter = 'cdc';

UPDATE connector_type
SET supported = false,
    unsupported_reason = 'Change capture for SQL Server is not built yet: this build can only prepare it for PostgreSQL.'
WHERE name = 'SQL Server CDC'
  AND supported = true
  AND adapter = 'cdc';
