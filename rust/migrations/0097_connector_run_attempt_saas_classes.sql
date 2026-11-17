-- SRC-3 slice 1 review, BLOCKER 1 (docs/superpowers/plans/2026-10-19-data-phase-4-connectors.md):
-- two more words for why an attempt failed, `permission_missing` and
-- `rate_limited`.
--
-- ── Why ───────────────────────────────────────────────────────────────
--
-- Decision 13 of docs/core/features/business-app-connectors.md asks a run to
-- say "the vendor limited the rate", and item 7 of the page's list asks for a
-- plain message when a permission is missing at the vendor. The closed list
-- of `0072` (`SRC-11` decision D2) had neither, so slice 1 could only file
-- them under `source_busy` and `sign_in_refused`, which do not say that.
--
-- `permission_missing` is not retried (the same answer would come back until
-- someone changes the account at the vendor); `rate_limited` is retried as
-- `source_busy` is, by the run's own delays. The Rust side is
-- `connector_run::ErrorClass` (the fixed texts and `is_transient`), the
-- Python side `ingest_errors` (`ERROR_CLASSES`, `RETRIED_CLASSES`).
--
-- ── Why DROP+ADD the CHECK ────────────────────────────────────────────
--
-- Postgres cannot extend a CHECK's set in place (see `0043`). Every value
-- already allowed stays allowed, so no stored row is invalidated. The name is
-- `0072`'s own.
ALTER TABLE connector_run_attempt DROP CONSTRAINT connector_run_attempt_class_check;
ALTER TABLE connector_run_attempt
    ADD CONSTRAINT connector_run_attempt_class_check
    CHECK (error_class IS NULL OR error_class IN (
        'unreachable', 'timeout', 'source_busy', 'api_unreachable',
        'sign_in_refused', 'address_refused', 'unsupported', 'setting_rejected',
        'schema_decision_pending', 'other', 'permission_missing', 'rate_limited'));
