-- WS3: a reference table naming every connector type the console's wizard
-- can offer, whether or not it is dialable yet. `supported = true` marks a
-- type this build's adapters (`lakehouse_store::ingest_spec::Dial`) can
-- actually parse a `dial` for; `supported = false` types are listed so the
-- wizard can show them (and their eventual roadmap) without pretending
-- they work today (AGENTS.md rule 2: unsupported -> honestly unsupported,
-- never a fabricated success). `adapter` names the `dial`/`Dial::parse`
-- shape a supported type maps to; it is NULL for every unsupported type,
-- since there is no shape yet to name.
CREATE TABLE connector_type (
    name       TEXT PRIMARY KEY,
    adapter    TEXT,
    supported  BOOLEAN NOT NULL DEFAULT false,
    docs_url   TEXT,
    CONSTRAINT connector_type_adapter_check
        CHECK (adapter IS NULL OR adapter IN ('sql', 'cdc', 'files', 'rest', 'sheets'))
);

-- docs_url stays NULL for every row -- no fabricated URL (AGENTS.md rule
-- 2: never invent a link nobody has published).
INSERT INTO connector_type (name, adapter, supported, docs_url)
VALUES
    ('PostgreSQL', 'sql', true, NULL),
    ('PostgreSQL CDC', 'cdc', true, NULL),
    ('MySQL', 'sql', true, NULL),
    ('MySQL CDC', 'cdc', true, NULL),
    ('MariaDB', 'sql', true, NULL),
    ('SQL Server', 'sql', true, NULL),
    ('SQL Server CDC', 'cdc', true, NULL),
    ('Object storage', 'files', true, NULL),
    ('REST API', 'rest', true, NULL),
    ('Google Sheets', 'sheets', true, NULL),
    ('Kafka', NULL, false, NULL),
    ('MongoDB', NULL, false, NULL),
    ('Oracle', NULL, false, NULL),
    ('SAP / ERP', NULL, false, NULL),
    ('SFTP', NULL, false, NULL),
    ('MQTT', NULL, false, NULL)
ON CONFLICT DO NOTHING;
