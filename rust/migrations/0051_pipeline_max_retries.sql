-- Plan 1c (R2, day-1): `pipeline_definition.max_retries` — a per-authored-
-- pipeline retry cap applied by `dagster/dispar_orchestrate/
-- authored_factory.py::_op_for_pipeline` when it builds the authored op's
-- `RetryPolicy`. The store-wide default lives in
-- `op_metadata.DEFAULT_RETRY_POLICY` (`max_retries=2`, exponential
-- backoff, jitter); this column overrides ONLY the count, never the
-- delay/backoff/jitter — the plan deliberately keeps those 1b defaults so
-- a synchronised retry storm cannot return.
--
-- Numbered 0051 (not 0050 or 0049): both are already taken by the
-- unmerged `feat/pipeline-alerts-sla-volume` branch's
-- `0049_*.sql` / `0050_*.sql`. R2's PR body records this fact, so a
-- branch-order audit can confirm the gap and a rebase lands 0049/0050
-- between this migration and any later one without renumbering.
--
-- 0..5 inclusive, 0 meaning "never retry" and 5 the highest count
-- `dagster.RetryPolicy` accepts as a non-negative small integer (the
-- column is smallint, not int, to keep the row narrow). The CHECK
-- constraint is the real safety boundary; the route layer additionally
-- rejects out-of-range bodies with 400 so the client never gets a 500
-- from the constraint violation.
ALTER TABLE pipeline_definition
    ADD COLUMN max_retries SMALLINT NOT NULL DEFAULT 2
        CHECK (max_retries BETWEEN 0 AND 5);

COMMENT ON COLUMN pipeline_definition.max_retries IS
    'Per-authored-pipeline retry cap passed as RetryPolicy.max_retries '
    'to the dagster op built by authored_factory._op_for_pipeline. '
    'Overrides only the count of DEFAULT_RETRY_POLICY (op_metadata.py); '
    'delay, backoff and jitter stay at the store-wide defaults.';
