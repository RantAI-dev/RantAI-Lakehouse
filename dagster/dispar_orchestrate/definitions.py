"""Code-location entrypoint. Module path is `dispar_orchestrate.definitions`
because `lakehouse_api::config::Config::dagster_location`'s existing
default (since before P3 — ported from `src/services/clients/dagster.ts`)
already names it; see `docs/adr/0005-dagster-code-location-ownership-and-packaging.md`.

Using `Definitions` (not the legacy `@repository` decorator) means Dagster
auto-names the one repository this code location exposes `__repository__`
— the other half of `DAGSTER_REPO`'s existing default.
"""

from dagster import Definitions

from dispar_orchestrate.assets import bronze_ingest_job
from dispar_orchestrate.gold_export import gold_export_job
from dispar_orchestrate.maintenance import bronze_maintenance_job, bronze_maintenance_schedule
from dispar_orchestrate.replication_metrics import (
    replication_slot_check_job,
    replication_slot_check_schedule,
)

# P4 adds `bronze_maintenance_job` (+ its daily schedule), P5 adds
# `replication_slot_check_job` (R5's slot-lag/WAL-retention metrics), and
# ADR 0010 adds `gold_export_job` (the scheduled trigger for Gold export to
# Iceberg, itself implemented in Rust — see `gold_export.py`'s module doc),
# all to the SAME code location as P3's `bronze_ingest_job` — one code
# location, one package, per ADR 0005 ("A future P4 [and P5] ... adds
# modules under the same `dispar_orchestrate` package and the same image,
# not new top-level directories").
defs = Definitions(
    jobs=[bronze_ingest_job, bronze_maintenance_job, replication_slot_check_job, gold_export_job],
    # `gold_export_job` is registered WITHOUT a schedule: it cannot
    # authenticate yet (`POST /api/gold/export/{mart}` is
    # `Policy::RequiresAuth`, enforced before the handler's own run-token
    # check), so a nightly schedule would 401 silently. See `gold_export.py`
    # for the full reasoning and how to restore the schedule once the job
    # has a service identity. The job stays registered so it remains
    # launchable on demand.
    schedules=[
        bronze_maintenance_schedule,
        replication_slot_check_schedule,
    ],
)
