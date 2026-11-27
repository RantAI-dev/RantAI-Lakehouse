"""`DATA-66` — one Iceberg sync per table: the job, the watcher, and the
authored op's own ask.

Before this module, `gold_export_job` exported **every** switched-on
publication after any authored pipeline succeeded and nightly at 04:00,
as one serial run: one table waited behind all the others, and a change
made outside a pipeline waited for the night. `DATA-66` gives each
`(zone, mart)` its own run and its own trigger:

- **`iceberg_sync_job`** exports and trims ONE publication, named by its
  run config. It is not on `SERIAL_JOB_NAMES`; `dagster.yaml` instead
  limits the tag `lakehouse/iceberg-sync-target` to one run per unique
  value, so two tables sync in parallel and two syncs of one table queue
  instead of being dropped (`D6`).
- **`iceberg_sync_sensor`** asks `GET /api/gold/sync-due` every 15
  seconds (`D8`) and turns each due item into a `RunRequest` with the
  API's deterministic `runKey`, so Dagster starts one run per signal.
  The API decides; this sensor only asks. That keeps the rules (and the
  cheap `system.parts` signal) in one place, unit-tested without a
  server.
- **`request_iceberg_sync`** is what an authored pipeline calls right
  after its write (`D7`). It never raises (`D3`): the ClickHouse write
  already happened, and if the ask is lost the watcher's `change` key
  picks the table up on its next tick.

Every path ends at the ONE export route (`POST /api/gold/export/{mart}`
with `ifChanged=true`) followed by `trim_gold_mart`, so the content
fingerprint, the single-flight lock, the single-commit export and the
latest copy are exactly the code `DATA-2`/`DATA-6`/`DATA-10`/`DATA-63`
already own.

Automatic syncs need the operator's export run token: without it the
API would refuse the export (`401`), so the sensor says so in one skip
reason instead of failing 1,440 times a day.
"""

from __future__ import annotations

from typing import Any

import requests
from dagster import (
    DefaultSensorStatus,
    Failure,
    RunRequest,
    SkipReason,
    job,
    op,
    sensor,
)

from dispar_orchestrate.gold_export import (
    ZONE_GOLD,
    ZONE_SILVER,
    GoldExportConfig,
    MartExportTarget,
    _headers,
    export_gold_mart,
    trim_gold_mart,
)
from dispar_orchestrate.ingest_factory import job_tags
from dispar_orchestrate.op_metadata import DEFAULT_RETRY_POLICY, source_metadata

# `DATA-66` D12: the job name, mirrored in Rust as
# `lakehouse_api::iceberg_sync::ICEBERG_SYNC_JOB` (the API launches it).
ICEBERG_SYNC_JOB = "iceberg_sync_job"
# The one op a launch addresses; also the Rust run-config builder's key.
ICEBERG_SYNC_TARGET_OP = "iceberg_sync_target"
# `dagster.yaml` limits this tag to one run per unique value (D6).
ICEBERG_SYNC_TARGET_TAG = "lakehouse/iceberg-sync-target"
# How the run was asked for: pipeline | change | retry | schedule (D12).
ICEBERG_SYNC_TRIGGER_TAG = "lakehouse/iceberg-sync-trigger"

TRIGGER_PIPELINE = "pipeline"
TRIGGER_CHANGE = "change"
TRIGGER_RETRY = "retry"
TRIGGER_SCHEDULE = "schedule"
TRIGGERS = (TRIGGER_PIPELINE, TRIGGER_CHANGE, TRIGGER_RETRY, TRIGGER_SCHEDULE)

# Feature page decision 8: how often the watcher asks the API.
WATCHER_INTERVAL_SECONDS = 15

RUN_LIMIT_ENV = "DAGSTER_RUN_LIMIT_ICEBERG_SYNC_JOB_SECONDS"
# A sync is a full copy plus a trim; an hour leaves room for a large
# table without leaving a dead run holding the per-table queue all day.
DEFAULT_RUN_LIMIT_SECONDS = 3_600

# The one sentence a tick says when no run token is configured. Without
# it the export would 401, and the export page already tells the
# operator the same thing.
NO_TOKEN_SKIP_REASON = (
    "automatic publishing needs the export run token (GOLD_EXPORT_RUN_TOKEN); "
    "only Publish now works until it is set"
)


@op(
    config_schema={"zone": str, "mart": str, "trigger": str},
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/iceberg_sync.py::iceberg_sync_target",
        reads=["run config (zone, mart, trigger)"],
        writes=[
            "Iceberg <zone>.<mart> (via POST /api/gold/export)",
            "ClickHouse lake.bronze_meta.maintenance_run",
        ],
    ),
)
def iceberg_sync_target(context) -> MartExportTarget:
    """Read this run's one publication out of the run config.

    An unknown zone is a protocol error, not a guess: publishing into the
    wrong namespace would be worse than failing the run, so it is a
    non-retryable `Failure` (`DATA-66` D6). The trigger is carried for the
    log and the run report; it does not change what is exported.
    """
    config = context.op_config
    zone = config["zone"]
    mart = config["mart"]
    trigger = config["trigger"]
    if zone not in (ZONE_SILVER, ZONE_GOLD):
        raise Failure(
            f"unknown publication zone {zone!r}: expected silver or gold",
            allow_retries=False,
        )
    if trigger not in TRIGGERS:
        raise Failure(
            f"unknown sync trigger {trigger!r}: expected one of {', '.join(TRIGGERS)}",
            allow_retries=False,
        )
    target = MartExportTarget(mart, zone)
    context.log.info(f"syncing {target.iceberg_table} ({trigger})")
    return target


@job(
    tags=job_tags(
        ICEBERG_SYNC_JOB, RUN_LIMIT_ENV, DEFAULT_RUN_LIMIT_SECONDS
    )
)
def iceberg_sync_job() -> None:
    """One publication, one run (`D6`): the export route followed by the
    trim, exactly as `gold_export_job` does per mart — but for only the
    table this run names, and without the serial tag, so two tables sync
    at once while two runs of one table queue on the per-table tag."""
    target = iceberg_sync_target()
    trim_gold_mart(target, export_gold_mart(target))


def evaluate_sync_due(cfg: GoldExportConfig):
    """The sensor body, split out of the decorator so tests can drive it
    with a config and a patched `requests` (the same split
    `pipeline_events.py::evaluate_finished_run` uses).

    Yields one `RunRequest` per due item with the API's own `runKey` (so
    Dagster drops a key it has seen) and both D6 tags; a malformed body
    is a non-retryable `Failure` rather than "nothing is due", which would
    silently stop every sync.
    """
    if not cfg.run_token:
        yield SkipReason(NO_TOKEN_SKIP_REASON)
        return
    resp = requests.get(
        f"{cfg.api_url}/api/gold/sync-due", headers=_headers(cfg), timeout=30
    )
    resp.raise_for_status()
    try:
        body = resp.json()
    except ValueError:
        raise Failure(
            f"GET {cfg.api_url}/api/gold/sync-due returned a body that is not "
            "JSON -- expected {\"due\": [...]}",
            allow_retries=False,
        ) from None
    due = body.get("due") if isinstance(body, dict) else None
    if not isinstance(due, list):
        raise Failure(
            f"GET {cfg.api_url}/api/gold/sync-due returned no due list -- "
            "expected {\"due\": [...]}",
            allow_retries=False,
        )
    if not due:
        yield SkipReason("no publication is due now")
        return
    for item in due:
        if not isinstance(item, dict):
            raise Failure(
                f"GET {cfg.api_url}/api/gold/sync-due returned a due item that "
                f"is not an object: {item!r}",
                allow_retries=False,
            )
        zone = item.get("zone")
        mart = item.get("mart")
        trigger = item.get("trigger")
        run_key = item.get("runKey")
        if not all(
            isinstance(value, str) and value
            for value in (zone, mart, trigger, run_key)
        ):
            raise Failure(
                f"GET {cfg.api_url}/api/gold/sync-due returned a due item "
                f"without zone, mart, trigger and runKey: {item!r}",
                allow_retries=False,
            )
        if zone not in (ZONE_SILVER, ZONE_GOLD):
            raise Failure(
                f"GET {cfg.api_url}/api/gold/sync-due returned an unknown zone "
                f"{zone!r}",
                allow_retries=False,
            )
        if trigger not in TRIGGERS:
            raise Failure(
                f"GET {cfg.api_url}/api/gold/sync-due returned an unknown "
                f"trigger {trigger!r}",
                allow_retries=False,
            )
        yield RunRequest(
            run_key=run_key,
            run_config={
                "ops": {
                    ICEBERG_SYNC_TARGET_OP: {
                        "config": {"zone": zone, "mart": mart, "trigger": trigger}
                    }
                }
            },
            tags={
                ICEBERG_SYNC_TARGET_TAG: f"{zone}.{mart}",
                ICEBERG_SYNC_TRIGGER_TAG: trigger,
            },
        )


# `default_status=RUNNING`, matching every other sensor this code location
# registers: a sensor created stopped never fires until someone notices in
# the Dagster UI, and from outside a stopped sensor looks identical to a
# healthy one. `minimum_interval_seconds=15` is feature page decision 8:
# "On every update" notices a change made outside a pipeline within 15 s.
# The tick is cheap — one read of the enabled publications and two small
# queries on the API side — and a run starts only when one is due.
@sensor(
    job=iceberg_sync_job,
    minimum_interval_seconds=WATCHER_INTERVAL_SECONDS,
    default_status=DefaultSensorStatus.RUNNING,
)
def iceberg_sync_sensor(context):
    """Ask the API which publications are due, and start one run each.

    The rules live in Rust (`lakehouse_api::iceberg_sync::due_syncs`), so
    this sensor cannot disagree with the API's own answer; the run key it
    passes makes each answer start at most one run.
    """
    yield from evaluate_sync_due(GoldExportConfig.from_env())


def request_iceberg_sync(target_zone: str, target_table: str, log: Any) -> None:
    """Ask the API to sync this table now (`D7`), right after a pipeline
    wrote it.

    **Never raises** (`D3`): the ClickHouse write has already happened, and
    a sync is a copy, so no exception, status or timeout here may fail the
    pipeline. Every failure is a `warning` (the fixed reason and the
    status only, never a body's text) and the watcher's `change` key picks
    the table up on its next tick. A missing run token is an `info` line
    and a skip: the API could only answer 401.

    `target_zone` is the ClickHouse zone name the authored op wrote
    (`serving`/`silver`); the API accepts both spellings of Gold.
    """
    try:
        cfg = GoldExportConfig.from_env()
    except Exception as exc:  # noqa: BLE001 - D3: nothing here may fail the pipeline
        if log:
            log.warning(
                f"after-write sync for {target_zone}.{target_table} skipped: "
                f"the gold export config could not be read ({exc})"
            )
        return
    if not cfg.run_token:
        if log:
            log.info(
                f"after-write sync for {target_zone}.{target_table} skipped: "
                "GOLD_EXPORT_RUN_TOKEN is not set"
            )
        return
    try:
        resp = requests.post(
            f"{cfg.api_url}/api/gold/export/{target_table}/after-write",
            params={"zone": target_zone},
            headers=_headers(cfg),
            timeout=10,
        )
        if resp.ok:
            return
        status = resp.status_code
        try:
            answered = resp.json()
        except ValueError:
            answered = {}
        reason = answered.get("reason") if isinstance(answered, dict) else None
        # Only this repository's own fixed reasons are logged; an
        # upstream answer's text is not.
        fixed_reason = reason if reason in TRIGGERS + (
            "publishing_off",
            "not_started",
        ) else "unknown"
        if log:
            log.warning(
                f"after-write sync for {target_zone}.{target_table} was not "
                f"started: status {status}, reason {fixed_reason}"
            )
    except Exception as exc:  # noqa: BLE001 - D3: nothing here may fail the pipeline
        if log:
            log.warning(
                f"after-write sync for {target_zone}.{target_table} failed to "
                f"reach the API: {exc}"
            )
