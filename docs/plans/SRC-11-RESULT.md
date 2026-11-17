# `SRC-11` result: what the connector-operations gate proved

Measured on pull request #144, commit `a2199d4b`, CI run 38676641937, job
"Rust · SRC-11 connector operations" (`ops/g6/src11_connector_operations_test.py`),
on 2026-10-15. One run on a GitHub-hosted `ubuntu-latest` runner; the steps
took 183 seconds, the job 5m23s.

**Two settings were lowered for this job only**, so the gate does not wait
hours: `INGEST_RETRY_DELAYS_SECONDS=5,10,20` (the product's default is
`60,300,900`, the spec's 1, 5 and 15 minutes) and
`CONNECTOR_AUTO_DISABLE_AFTER=3` (default 30, Airbyte's threshold). The
feature page's checklist row 6 allows it. The behaviour proven is the same;
the waits are shorter.

| Step | What happened |
| --- | --- |
| a. A source that is down, then back | The gate closed the only path to the MySQL source. Attempt 1 failed as `unreachable`, with the next try announced in 5 s; the gate reopened the path; attempt 2 loaded 3 rows. The run succeeded with nobody acting. |
| b. A refused sign-in (PostgreSQL, `SRC-19`) | One attempt, class `sign_in_refused`, no retry. |
| f. The run log | The downloaded text names attempt 1 and its class and holds no password, no driver text, no traceback. |
| c. Auto-disable | Three failed runs in a row paused the connector with "Paused after 3 failed runs in a row"; the "connector disabled" webhook arrived once and named the connector; the schedule's due list left it out. |
| d. A paused connector does not load (`SRC-18`, first half) | A run started straight through the orchestrator ended `paused` (`connector_paused`) and loaded nothing; no second alert. |
| c, continued. Resume | The pause lifted, the failure streak went to 0, the next run loaded. |
| e. A left-out column | The column was listed inactive with the reason "excluded"; the next load carried no value for it; nothing waited for a decision. |
| g. Run history | All 9 runs the gate started through "Run now" are listed, each with the trigger `user`. |

## Not measured

- The real waits (1, 5 and 15 minutes) and the real threshold (30 runs).
- That the orchestrator's executor honours each retry's wait in production
  the same way as in the gate's stack (it is the same image and settings).
- The 30-day log removal: covered by a store test, not by the gate.
- History of more than 50 runs: covered by a store test.
- More than one run of the gate: one measurement.
