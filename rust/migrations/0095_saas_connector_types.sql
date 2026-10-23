-- SRC-3 T1.1 (docs/superpowers/plans/2026-10-19-data-phase-4-connectors.md,
-- AD1 and AD10): admit the `saas` adapter and list the ten business-app
-- connector types, all `supported = false` until the last slice flips them.
--
-- ── Why one adapter, not ten ─────────────────────────────────────────
--
-- AD1: the adapter string is switched on in about fifteen places in Rust,
-- Python and the console. Ten adapters would be ten arms in each; one `saas`
-- adapter with a `vendor` field in its dial adds one arm each and dispatches
-- on the vendor inside the SaaS code. The ten `connector_type` rows below all
-- carry `adapter = 'saas'`; the vendor is chosen by the type's name in the
-- console and stored in the connector's dial.
--
-- ── Why DROP+ADD the CHECKs ──────────────────────────────────────────
--
-- Postgres cannot extend a CHECK's set in place (see `0043`). Both
-- `connector_adapter_check` and `connector_type_adapter_check` are dropped
-- and re-added with `saas` added to the set `0043` left. Every value already
-- allowed stays allowed, so no row is invalidated. The statements are in
-- the shapes `ops/lint/check_connector_gate_coverage.py` replays (ALTER TABLE
-- ... DROP/ADD CONSTRAINT change no row), so SRC-9's lint needs no change
-- (plan section 8, item 5).
--
-- ── Why `supported = false` now (AD10, AGENTS.md principle 2) ────────
--
-- The types are listed so the picker can show them, but nothing loads yet.
-- Each carries `adapter = 'saas'` (not NULL) so a connector created later
-- opens in the edit page, as `0062` and `0066` do for their rows, and an
-- `unsupported_reason` the picker shows on the disabled tile. The reason is
-- set by a targeted UPDATE per type (`supported = false AND adapter =
-- 'saas'`) because the lint's INSERT shape has no reason column; a
-- deployment that has already edited a row keeps its edit. `0096` (slice 7)
-- flips the eight that pass a gate; Workday and NetSuite stay unsupported
-- (decision 10 of docs/core/features/business-app-connectors.md).
ALTER TABLE connector DROP CONSTRAINT connector_adapter_check;
ALTER TABLE connector
    ADD CONSTRAINT connector_adapter_check
    CHECK (adapter IS NULL OR adapter IN ('sql','cdc','files','rest','sheets','mongodb','kafka','sftp','saas'));

ALTER TABLE connector_type DROP CONSTRAINT connector_type_adapter_check;
ALTER TABLE connector_type
    ADD CONSTRAINT connector_type_adapter_check
    CHECK (adapter IS NULL OR adapter IN ('sql','cdc','files','rest','sheets','mongodb','kafka','sftp','saas'));

INSERT INTO connector_type (name, adapter, supported, docs_url)
VALUES
    ('Salesforce', 'saas', false, NULL),
    ('ServiceNow', 'saas', false, NULL),
    ('Workday Reports', 'saas', false, NULL),
    ('Google Analytics 4', 'saas', false, NULL),
    ('HubSpot', 'saas', false, NULL),
    ('Zendesk', 'saas', false, NULL),
    ('Confluence', 'saas', false, NULL),
    ('Dynamics 365', 'saas', false, NULL),
    ('NetSuite', 'saas', false, NULL),
    ('SharePoint', 'saas', false, NULL)
ON CONFLICT DO NOTHING;

UPDATE connector_type
SET unsupported_reason = 'Being built (SRC-3).'
WHERE name = 'Salesforce'
  AND supported = false
  AND adapter = 'saas';

UPDATE connector_type
SET unsupported_reason = 'Being built (SRC-3).'
WHERE name = 'ServiceNow'
  AND supported = false
  AND adapter = 'saas';

UPDATE connector_type
SET unsupported_reason = 'Being built (SRC-3).'
WHERE name = 'Workday Reports'
  AND supported = false
  AND adapter = 'saas';

UPDATE connector_type
SET unsupported_reason = 'Being built (SRC-3).'
WHERE name = 'Google Analytics 4'
  AND supported = false
  AND adapter = 'saas';

UPDATE connector_type
SET unsupported_reason = 'Being built (SRC-3).'
WHERE name = 'HubSpot'
  AND supported = false
  AND adapter = 'saas';

UPDATE connector_type
SET unsupported_reason = 'Being built (SRC-3).'
WHERE name = 'Zendesk'
  AND supported = false
  AND adapter = 'saas';

UPDATE connector_type
SET unsupported_reason = 'Being built (SRC-3).'
WHERE name = 'Confluence'
  AND supported = false
  AND adapter = 'saas';

UPDATE connector_type
SET unsupported_reason = 'Being built (SRC-3).'
WHERE name = 'Dynamics 365'
  AND supported = false
  AND adapter = 'saas';

UPDATE connector_type
SET unsupported_reason = 'Being built (SRC-3).'
WHERE name = 'NetSuite'
  AND supported = false
  AND adapter = 'saas';

UPDATE connector_type
SET unsupported_reason = 'Being built (SRC-3).'
WHERE name = 'SharePoint'
  AND supported = false
  AND adapter = 'saas';
