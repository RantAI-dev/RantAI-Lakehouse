"""Code-location entrypoint. Module path is `dispar_orchestrate.definitions`
because `lakehouse_api::config::Config::dagster_location`'s existing
default (since before P3 — ported from `src/services/clients/dagster.ts`)
already names it; see `docs/adr/0005-dagster-code-location-ownership-and-packaging.md`.

Using `Definitions` (not the legacy `@repository` decorator) means Dagster
auto-names the one repository this code location exposes `__repository__`
— the other half of `DAGSTER_REPO`'s existing default.
"""

from dagster import Definitions
from dispar_orchestrate.agent_runs import agent_run_job, agent_run_schedules
from dispar_orchestrate.assets import bronze_ingest_job
from dispar_orchestrate.gold_export import gold_export_job
from dispar_orchestrate.maintenance import (
    bronze_maintenance_job,
    bronze_maintenance_schedule,
)
from dispar_orchestrate.replication_metrics import (
    replication_slot_check_job,
    replication_slot_check_schedule,
)

# P4 adds `bronze_maintenance_job` (+ its daily schedule), P5 adds
# `replication_slot_check_job` (R5's slot-lag/WAL-retention metrics), ADR
# 0010 adds `gold_export_job` (the scheduled trigger for Gold export to
# Iceberg, itself implemented in Rust — see `gold_export.py`'s module doc),
# and the copilot-operations-handover plan's T3.3 adds `agent_run_job` +
# `agent_run_schedules` (one `ScheduleDefinition` per digital employee that
# has a `schedule_cron` set, built from `GET /api/agents/employees` at
# code-load time — see `agent_runs.py`'s module doc for the full design and
# why that HTTP call can never crash this code location), all to the SAME
# code location as P3's `bronze_ingest_job` — one code location, one
# package, per ADR 0005 ("A future P4 [and P5] ... adds modules under the
# same `dispar_orchestrate` package and the same image, not new top-level
# directories").
#
# `gold_export_job` is STILL registered without a schedule (a separate,
# larger change would be needed to give it the same service-credential
# treatment `agent_run_job` got here — see `gold_export.py`'s module doc),
# so it remains launchable on demand from the Dagster UI, same as before.
# `agent_run_job`'s own schedules are no longer permanently empty: with
# `AGENT_RUN_TOKEN` set (see `lakehouse-api::main::bootstrap_agent_run_service`
# and this repo's `.env.example`), `agent_run_schedules` holds one entry
# per scheduled employee; with it unset, `agent_run_schedules` is `[]` and
# `agent_run_job` stays launchable on demand only, exactly like
# `gold_export_job`.
defs = Definitions(
    jobs=[
        bronze_ingest_job,
        bronze_maintenance_job,
        replication_slot_check_job,
        gold_export_job,
        agent_run_job,
    ],
    schedules=[
        bronze_maintenance_schedule,
        replication_slot_check_schedule,
        *agent_run_schedules,
    ],
)
