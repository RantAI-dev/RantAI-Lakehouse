-- SRC-3 T7.1 (docs/superpowers/plans/2026-10-19-data-phase-4-connectors.md,
-- slice 7, AD10): the eight business-app types whose gate passes become
-- `supported = true`; Workday Reports and NetSuite stay unsupported.
--
-- ── Why these eight (decision 10 of docs/core/features/business-app-connectors.md)
--
-- `0095` listed all ten types with `supported = false` so that nothing was
-- offered before a gate could prove it. Since then each of Salesforce,
-- ServiceNow, Google Analytics 4, HubSpot, Zendesk, Confluence, Dynamics 365
-- and SharePoint has a gate step set (connect, discover, load, reload) in the
-- `g6-saas` gate, run by two CI jobs (`g6-saas-a`, `g6-saas-b`, AD9 as amended),
-- and a `coverage.json` entry saying which. The same commit adds those entries,
-- because SRC-9's lint (`ops/lint/check_connector_gate_coverage.py`) fails a
-- supported type without one. The gates run against mock vendors; no real
-- account was used, which the feature page and `docs/plans/SRC-3-RESULT.md` say.
--
-- ── Why Workday Reports and NetSuite stay unsupported
--
-- Both are built and pass the same gate against their mocks, but decision 10
-- keeps them off the list until a test on a real account shows the wire
-- details that no readable documentation confirmed (token requests, report
-- and SuiteQL shapes). Their reason changes from "Being built" to say so,
-- because "being built" would now be false (principle 2).
--
-- ── Shape
--
-- Each statement is targeted (`name`, `adapter = 'saas'`, `supported = false`)
-- so a deployment that already changed a row keeps its change, and is in the
-- form the SRC-9 lint replays. Nothing here is edited once applied; a later
-- change is a new migration.

UPDATE connector_type
SET supported = true,
    unsupported_reason = NULL
WHERE name = 'Salesforce'
  AND adapter = 'saas'
  AND supported = false;

UPDATE connector_type
SET supported = true,
    unsupported_reason = NULL
WHERE name = 'ServiceNow'
  AND adapter = 'saas'
  AND supported = false;

UPDATE connector_type
SET supported = true,
    unsupported_reason = NULL
WHERE name = 'Google Analytics 4'
  AND adapter = 'saas'
  AND supported = false;

UPDATE connector_type
SET supported = true,
    unsupported_reason = NULL
WHERE name = 'HubSpot'
  AND adapter = 'saas'
  AND supported = false;

UPDATE connector_type
SET supported = true,
    unsupported_reason = NULL
WHERE name = 'Zendesk'
  AND adapter = 'saas'
  AND supported = false;

UPDATE connector_type
SET supported = true,
    unsupported_reason = NULL
WHERE name = 'Confluence'
  AND adapter = 'saas'
  AND supported = false;

UPDATE connector_type
SET supported = true,
    unsupported_reason = NULL
WHERE name = 'Dynamics 365'
  AND adapter = 'saas'
  AND supported = false;

UPDATE connector_type
SET supported = true,
    unsupported_reason = NULL
WHERE name = 'SharePoint'
  AND adapter = 'saas'
  AND supported = false;

UPDATE connector_type
SET unsupported_reason = 'Built and tested against a mock; waits for a test on a real account (SRC-3).'
WHERE name = 'Workday Reports'
  AND adapter = 'saas'
  AND supported = false;

UPDATE connector_type
SET unsupported_reason = 'Built and tested against a mock; waits for a test on a real account (SRC-3).'
WHERE name = 'NetSuite'
  AND adapter = 'saas'
  AND supported = false;
