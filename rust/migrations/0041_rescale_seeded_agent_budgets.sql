-- WS7 item G2 redefined `agent_employee.budget_limit` from a currency unit
-- to a token unit (see `agents.rs`'s `run_headless_loop`, and the plan's
-- Task G2: "budget_limit`/`budget_consumed` are measured in TOKENS, not
-- currency"). The two rows `0018_seed_agents.sql` inserted were authored
-- under the OLD, currency meaning — `emp-inventory` at 1000, `emp-risk`
-- at 500 — and were never revisited when the unit changed. Under the new
-- token unit those numbers are a rounding error: a single LLM call's
-- `usage.total_tokens` commonly exceeds 1000 on its own, so both seeded
-- employees would exhaust a "real" budget on or before their first call.
--
-- This rescales both seeded values to token-scale by a factor of 100,
-- preserving the 2:1 ratio the original seed intended (`emp-inventory`
-- stays the larger of the two): 1000 -> 100000, 500 -> 50000.
--
-- Targeted by id AND the exact old value, so this can never clobber a
-- budget_limit an operator has since set through the API.
UPDATE agent_employee
SET budget_limit = 100000
WHERE id = 'emp-inventory' AND budget_limit = 1000;

UPDATE agent_employee
SET budget_limit = 50000
WHERE id = 'emp-risk' AND budget_limit = 500;
