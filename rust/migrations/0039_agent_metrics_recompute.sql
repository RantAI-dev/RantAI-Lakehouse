-- WS7 plan (T11): re-enables the five DigitalEmployee metrics WS1 task
-- 1.11 made #[sqlx(skip)] `None` (rust/crates/lakehouse-store/src/
-- agents.rs) because nothing ever updated their insert-time-default
-- columns. This migration does NOT resurrect the old columns as write
-- targets -- it adds a function that computes each metric FRESH from real
-- agent_run/approval_item rows, called once per run's terminal transition
-- (WS7 item G3, routes::agents::run_headless_loop), and WRITES the result
-- into the existing columns so a read is a single indexed SELECT, not a
-- live aggregate query on every page load.
--
-- `budget_spent`/`approval_rate`/`success_rate`/`recent_runs` were `NOT
-- NULL DEFAULT 0` (`0017_agents.sql`) -- a column that can never be NULL
-- can never honestly say "not yet measured," only "0," which is exactly
-- the fabricated-zero problem WS1 task 1.11 stopped serving by dropping
-- these columns from every SELECT. Dropping NOT NULL/DEFAULT here lets
-- `recompute_employee_metrics` (below) write a real NULL for an employee
-- whose `success_rate`/`approval_rate` have no qualifying denominator
-- (zero terminal runs / zero decided approvals in the last 30 days) --
-- that is an honest "no ratio to report," never a fabricated "0%." Every
-- existing row's insert-time-default `0` is backfilled to NULL for the
-- same reason: those zeros were never a measurement either.
ALTER TABLE agent_employee
    ALTER COLUMN budget_spent DROP DEFAULT,
    ALTER COLUMN budget_spent DROP NOT NULL,
    ALTER COLUMN approval_rate DROP DEFAULT,
    ALTER COLUMN approval_rate DROP NOT NULL,
    ALTER COLUMN success_rate DROP DEFAULT,
    ALTER COLUMN success_rate DROP NOT NULL,
    ALTER COLUMN recent_runs DROP DEFAULT,
    ALTER COLUMN recent_runs DROP NOT NULL;

UPDATE agent_employee
SET budget_spent = NULL, approval_rate = NULL, success_rate = NULL, recent_runs = NULL;

-- Definitions (exact, not "approximately"):
--   budget_spent  = SUM(agent_run.budget_consumed) over every run for this
--                   employee (tokens -- see WS7 item G2's unit note); `0`
--                   when the employee has runs but none has consumed any
--                   tokens yet -- a real, COALESCEd zero, not a fabricated
--                   one, because "zero rows summed" and "rows summing to
--                   zero" are the same true answer here (unlike a ratio's
--                   empty denominator, below).
--   recent_runs   = COUNT(agent_run) in the last 30 days -- COUNT never
--                   returns NULL, so a real `0` here always means "zero
--                   runs," never "not measured."
--   success_rate  = COUNT(status = 'succeeded') / COUNT(status IN
--                   ('succeeded', 'failed', 'rejected', 'budget_exhausted'))
--                   over the last 30 days -- a run still 'running'/
--                   'waiting_approval' is excluded from BOTH numerator and
--                   denominator (it has no outcome yet, so it neither
--                   succeeds nor fails); 'budget_exhausted' (WS7 item G2)
--                   is a real terminal non-success outcome, counted in the
--                   denominator alongside 'failed'/'rejected'. NULL (not
--                   `0`) when the denominator is zero -- no terminal run
--                   in the window means no rate to report, not "0%."
--   approval_rate = COUNT(approval_item WHERE run_id IN (this employee's
--                   runs) AND status = 'approved') / COUNT(approval_item
--                   WHERE run_id IN (...) AND status IN ('approved',
--                   'rejected')) over the last 30 days -- a still-pending
--                   approval is excluded from both, same reasoning as
--                   success_rate; NULL when the denominator is zero, same
--                   reasoning as success_rate.
--
-- `budget_reserved` is NOT recomputed here: nothing in this plan or the
-- existing codebase ever reserves a budget ahead of a run (no multi-step
-- reservation/hold concept exists) -- it stays `#[sqlx(skip)] None`
-- permanently, disclosed in the WS7 phase G report, not silently dropped.
CREATE OR REPLACE FUNCTION recompute_employee_metrics(p_employee_id TEXT) RETURNS void AS $$
BEGIN
    UPDATE agent_employee e SET
        budget_spent = COALESCE((
            SELECT SUM(budget_consumed) FROM agent_run WHERE employee_id = p_employee_id
        ), 0),
        recent_runs = (
            SELECT COUNT(*) FROM agent_run
            WHERE employee_id = p_employee_id AND started_at > now() - interval '30 days'
        ),
        success_rate = (
            SELECT COUNT(*) FILTER (WHERE status = 'succeeded')::double precision
                   / NULLIF(COUNT(*) FILTER (
                       WHERE status IN ('succeeded', 'failed', 'rejected', 'budget_exhausted')
                     ), 0)
            FROM agent_run
            WHERE employee_id = p_employee_id AND started_at > now() - interval '30 days'
        ),
        approval_rate = (
            SELECT COUNT(*) FILTER (WHERE a.status = 'approved')::double precision
                   / NULLIF(COUNT(*) FILTER (WHERE a.status IN ('approved', 'rejected')), 0)
            FROM approval_item a
            JOIN agent_run r ON r.id = a.run_id
            WHERE r.employee_id = p_employee_id AND a.requested_at > now() - interval '30 days'
        )
    WHERE e.id = p_employee_id;
END;
$$ LANGUAGE plpgsql;
