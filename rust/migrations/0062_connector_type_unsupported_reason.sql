-- SRC-6 (F5): Google Sheets was seeded `supported = true` in `0035`, but no
-- build has ever been able to test or load it: `probe_sheets` and
-- `adapters/sheets.py` both answer "unsupported" because the build has no
-- verified Google sign-in. The wizard therefore offered a type that cannot
-- work (AGENTS.md rule 2, never fabricate).
--
-- This adds a nullable `unsupported_reason` that the wizard shows on a
-- disabled tile, so the reason comes from the API and not from a string
-- hard-coded in the console. It is NULL for every other row: the roadmap
-- rows keep the generic "Not available yet".
--
-- Google Sheets keeps `adapter = 'sheets'` on purpose: connectors already
-- created with that type must still open in the edit page, which resolves
-- the dial form from the adapter. Only the listing says "not supported".
--
-- The UPDATE is targeted (`supported = true AND adapter = 'sheets'`), like
-- `0043`, so a deployment that has already edited the row keeps its edit
-- and a re-run changes nothing.

ALTER TABLE connector_type ADD COLUMN IF NOT EXISTS unsupported_reason TEXT;

UPDATE connector_type
SET supported = false,
    unsupported_reason = 'No verified Google sign-in exists in this build, so a Google Sheets connector cannot be tested or loaded.'
WHERE name = 'Google Sheets'
  AND supported = true
  AND adapter = 'sheets';
