-- Phase 2, Task 2.7: connector definitions (source/sink systems the
-- lakehouse pulls from or pushes to).
--
-- CREDENTIAL DECISION -- read before touching this table.
--
-- `CreateConnectorInput` (contracts/connectors.ts) already only ever
-- accepts a `secretRef: string`, never a credential value. That is not an
-- oversight this migration works around; it is the design this table
-- implements: `secret_ref` names WHERE a credential lives (an environment
-- variable, a secret-manager path) so a real client can resolve it at
-- connection time. The credential's actual value is never sent to this
-- service, never appears in a request body this table's writer accepts,
-- and therefore CANNOT be stored here -- there is no column for it, and
-- there never will be one. This is reference-only credential handling
-- (option 1 of the task brief), chosen over envelope encryption because:
--
--   * it needs no crypto dependency, no KEK, no key-rotation story -- the
--     value that would need rotating never enters this service at all;
--   * "the console can show which secret is referenced, but never the
--     secret" is true by construction, not by a redaction step that could
--     be forgotten on one code path (see `Connector`/`ConnectorDetail`
--     below, which do not even have a field to leak);
--   * the repository already had one secret-leak incident (see
--     `rust/tests/parity/README.md`) from a value that DID flow through
--     the service -- the cheapest fix to "never leaks" is "never holds it".
--
-- Product consequence (told straight, not buried): this console cannot
-- issue, rotate, or validate a credential's actual bytes. `testConnection`
-- cannot attempt a real network probe with the referenced secret (nothing
-- here can decrypt or fetch it), and the UI cannot offer "reveal secret" or
-- "test with these exact bytes" -- only "which secret is this connector
-- configured to use" and "is the connector's last known health status
-- healthy". A real connectivity test has to happen in whatever runtime
-- actually holds the resolved credential (a Dagster resource, a connector
-- worker), not in this console's API tier.
CREATE TABLE connector (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    type                TEXT NOT NULL,
    direction           TEXT NOT NULL,
    health              TEXT NOT NULL DEFAULT 'healthy',
    environment         TEXT NOT NULL,
    tenant              TEXT NOT NULL,
    -- Connection target (hostname/endpoint label), NOT a credential by
    -- itself, but still never rendered by any GET handler in this domain
    -- (see `routes::connectors`) -- it can still describe internal
    -- topology a console shouldn't broadcast.
    host                TEXT NOT NULL,
    -- A REFERENCE to where a credential lives (e.g. "env:PG_OMS_PASSWORD",
    -- "vault:secret/data/connectors/pg-oms"). Never a credential value --
    -- see the migration header comment. `create_connector` additionally
    -- rejects any value that is *shaped* like a raw secret (long hex/
    -- base64 blob, JWT, PEM block, ...) as defense in depth against a
    -- caller mistake, even though nothing here could do anything harmful
    -- with one beyond storing it.
    secret_ref          TEXT NOT NULL,
    residency            TEXT NOT NULL DEFAULT '',
    last_test_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_activity_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    capabilities        TEXT[] NOT NULL DEFAULT '{}',
    owner               TEXT NOT NULL DEFAULT 'Current user',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT connector_direction_check
        CHECK (direction IN ('source', 'sink', 'bidirectional')),
    CONSTRAINT connector_health_check
        CHECK (health IN ('healthy', 'degraded', 'unhealthy', 'unknown')),
    -- Every fixture in mock/connectors.ts uses a distinct connector name;
    -- same rationale as `pipeline_definition_name_unique`.
    CONSTRAINT connector_name_unique UNIQUE (name)
);
