"""Tests for the pure re-upload chain graph and the polling chain sensor
(`BLD-14`, DEC-22, D3-D9). No Dagster instance and no network: the graph is
derived from plain wire payloads, and the sensor's poster is patched.

The graph tests are sentences about D3's order rule and P1/P2's exclusions.
The sensor tests pin each branch of `decide` and the cursor's "settle only
after the record was accepted" rule (D8, P4, P5, P10).
"""

from __future__ import annotations

from dispar_orchestrate.authored_factory import (
    FILE_INGEST_JOB,
    ChainEntry,
    build_authored_dependency_sensors,
    reupload_chain_graph,
)


def pipeline(
    pipeline_id: str,
    *,
    target: str | None = None,
    inputs: list[tuple[str, str]] | None = None,
    tenant: str | None = "t1",
    status: str = "ready",
    depends_on: list[str] | None = None,
    target_lag: dict | None = None,
) -> dict:
    """One `GET /api/pipelines/runnable` row in the shape D3 reads."""
    definition: dict = {
        "inputs": [{"zone": zone, "table": table} for zone, table in (inputs or [])],
    }
    if target:
        zone, table = target.split(".", 1)
        definition["targetZone"] = zone
        definition["targetTable"] = table
    result: dict = {
        "id": pipeline_id,
        "tenantId": tenant,
        "status": status,
        "definition": definition,
        "dependsOn": depends_on or [],
    }
    if target_lag is not None:
        result["targetLag"] = target_lag
    return result


def first_pipeline(
    pipeline_id: str, table: str = "orders", *, target: str = "silver.orders_clean", **kwargs
) -> dict:
    fields: dict = {
        "target": target,
        "inputs": [("bronze", table)],
        "depends_on": [FILE_INGEST_JOB],
    }
    fields.update(kwargs)
    result = pipeline(pipeline_id, **fields)
    result["definition"]["sourceZone"] = "bronze"
    result["definition"]["sourceTable"] = table
    return result


def graph_for(*pipelines: dict) -> dict:
    return reupload_chain_graph(list(pipelines))


def test_a_gold_pipeline_reading_a_silver_output_is_after_it() -> None:
    silver = first_pipeline("pl-silver")
    gold = pipeline("pl-gold", target="gold.orders_daily", inputs=[("silver", "orders_clean")])

    graph = graph_for(gold, silver)

    entry = graph["pl-gold"]["orders"]
    assert entry.parents == ("pl-silver",)
    assert entry.ancestors == ("pl-silver",)
    assert "pl-silver" not in graph, "the first pipeline has no chain parent of its own"


def test_a_second_input_the_reupload_does_not_reach_is_not_a_parent() -> None:
    silver = first_pipeline("pl-silver")
    unrelated = pipeline("pl-unrelated", target="silver.other_clean")
    gold = pipeline(
        "pl-gold",
        target="gold.orders_daily",
        inputs=[("silver", "orders_clean"), ("silver", "other_clean")],
    )

    graph = graph_for(silver, unrelated, gold)

    entry = graph["pl-gold"]["orders"]
    assert entry.parents == ("pl-silver",)
    assert entry.ancestors == ("pl-silver",)


def test_a_diamond_waits_for_both_branches() -> None:
    first = first_pipeline("pl-first")
    left = pipeline("pl-left", target="silver.left", inputs=[("silver", "orders_clean")])
    right = pipeline("pl-right", target="silver.right", inputs=[("silver", "orders_clean")])
    join = pipeline("pl-join", target="gold.join", inputs=[("silver", "left"), ("silver", "right")])

    graph = graph_for(join, right, left, first)

    entry = graph["pl-join"]["orders"]
    assert entry.parents == ("pl-left", "pl-right")
    assert entry.ancestors == ("pl-first", "pl-left", "pl-right")


def test_another_tenants_pipeline_is_never_in_the_chain() -> None:
    silver = first_pipeline("pl-silver", tenant="t1")
    foreign_gold = pipeline(
        "pl-foreign-gold", target="gold.orders_daily", inputs=[("silver", "orders_clean")], tenant="t2"
    )
    own_gold = pipeline(
        "pl-own-gold", target="gold.orders_daily", inputs=[("silver", "orders_clean")], tenant="t1"
    )

    graph = graph_for(silver, foreign_gold, own_gold)

    assert "pl-foreign-gold" not in graph
    assert graph["pl-own-gold"]["orders"].parents == ("pl-silver",)


def test_a_paused_pipeline_and_what_only_it_feeds_are_left_out() -> None:
    silver = first_pipeline("pl-silver", status="paused")
    gold = pipeline("pl-gold", target="gold.orders_daily", inputs=[("silver", "orders_clean")])

    assert graph_for(silver, gold) == {}


def test_a_target_lag_pipeline_is_left_out() -> None:
    silver = first_pipeline("pl-silver", target_lag={"kind": "lag", "minutes": 5})
    gold = pipeline("pl-gold", target="gold.orders_daily", inputs=[("silver", "orders_clean")])

    assert graph_for(silver, gold) == {}


def test_a_cycle_is_left_out() -> None:
    first = first_pipeline("pl-first")
    a = pipeline("pl-a", target="silver.a", inputs=[("bronze", "orders"), ("gold", "b")])
    b = pipeline("pl-b", target="gold.b", inputs=[("silver", "a")])
    tail = pipeline("pl-tail", target="gold.tail", inputs=[("gold", "b")])

    graph = graph_for(first, a, b, tail)

    assert "pl-a" not in graph
    assert "pl-b" not in graph
    assert "pl-tail" not in graph


def test_with_the_switch_off_nothing_below_the_first_pipeline_is_in_the_chain() -> None:
    silver = first_pipeline("pl-silver", depends_on=[])
    gold = pipeline("pl-gold", target="gold.orders_daily", inputs=[("silver", "orders_clean")])

    assert graph_for(silver, gold) == {}


def test_a_first_pipeline_that_also_reads_another_first_pipeline_has_parents() -> None:
    first = first_pipeline("pl-first")
    second = first_pipeline("pl-second", target="silver.second", inputs=None)
    second["definition"]["inputs"] = [
        {"zone": "bronze", "table": "orders"},
        {"zone": "silver", "table": "orders_clean"},
    ]

    graph = graph_for(first, second)

    assert graph["pl-second"]["orders"].parents == ("pl-first",)
    assert graph["pl-second"]["orders"].ancestors == ("pl-first",)


# ── BLD-14 T7: the polling sensor ──────────────────────────────────────────

import json
import time
from types import SimpleNamespace
from typing import Any
from unittest.mock import patch

from dagster import DagsterRunStatus, RunRequest, SkipReason, job

from dispar_orchestrate.authored_factory import AuthoredPipelineConfig
from dispar_orchestrate.reupload_chain import (
    CHAIN_WAIT_LIMIT_SECONDS,
    REUPLOAD_ROOT_TAG,
    REUPLOAD_TABLE_TAG,
    Lookups,
    Root,
    RunInfo,
    build_reupload_chain_sensor,
    build_reupload_chain_sensors,
    chain_sensor_name,
    decide,
)


def lookups(
    pipeline_id: str,
    runs: dict[tuple[str, str], RunInfo] | None = None,
    *,
    newer: bool = False,
    in_flight: bool = False,
) -> Lookups:
    runs = runs or {}
    return Lookups(
        pipeline_id=pipeline_id,
        run_for_tag=lambda pid, root: runs.get((pid, root)),
        newer_root=lambda table, root: newer,
        in_flight=lambda pid: in_flight,
    )


ROOT = Root(run_id="root-1", table="orders", end_time=1_000.0)
NOW = 1_000.0 + 60.0


def test_a_pipeline_starts_when_every_parent_succeeded_for_this_reupload() -> None:
    entry = ChainEntry(parents=("pl-silver",), ancestors=("pl-silver",))
    runs = {("pl-silver", "root-1"): RunInfo("silver-run", "SUCCESS")}

    decision = decide(ROOT, entry, lookups("pl-gold", runs), NOW)

    assert decision.action == "run"
    assert decision.run_key == "reupload-chain:root-1:pl-gold"
    assert decision.tags == {REUPLOAD_ROOT_TAG: "root-1", REUPLOAD_TABLE_TAG: "orders"}


def test_a_pipeline_waits_while_one_branch_of_a_diamond_is_running() -> None:
    entry = ChainEntry(
        parents=("pl-left", "pl-right"), ancestors=("pl-first", "pl-left", "pl-right")
    )
    runs = {
        ("pl-left", "root-1"): RunInfo("left-run", "SUCCESS"),
        ("pl-right", "root-1"): RunInfo("right-run", "STARTED", in_flight=True),
    }

    decision = decide(ROOT, entry, lookups("pl-join", runs), NOW)

    assert decision.action == "wait"
    assert decision.run_key is None


def test_a_failed_parent_stops_the_chain_and_records_the_cause() -> None:
    entry = ChainEntry(parents=("pl-silver",), ancestors=("pl-silver",))
    runs = {("pl-silver", "root-1"): RunInfo("silver-run", "FAILURE")}

    decision = decide(ROOT, entry, lookups("pl-gold", runs), NOW)

    assert decision.action == "record"
    assert (decision.outcome, decision.reason, decision.cause) == (
        "not_run",
        "upstream_failed",
        "pl-silver",
    )


def test_a_failure_two_steps_up_is_named_as_the_cause() -> None:
    entry = ChainEntry(
        parents=("pl-silver",), ancestors=("pl-first", "pl-silver")
    )
    runs = {
        ("pl-first", "root-1"): RunInfo("first-run", "FAILURE"),
        ("pl-silver", "root-1"): RunInfo("silver-run", "SUCCESS"),
    }

    decision = decide(ROOT, entry, lookups("pl-gold", runs), NOW)

    assert (decision.reason, decision.cause) == ("upstream_failed", "pl-first")


def test_a_cancelled_parent_records_upstream_cancelled() -> None:
    entry = ChainEntry(parents=("pl-silver",), ancestors=("pl-silver",))
    runs = {("pl-silver", "root-1"): RunInfo("silver-run", "CANCELED")}

    decision = decide(ROOT, entry, lookups("pl-gold", runs), NOW)

    assert (decision.outcome, decision.reason, decision.cause) == (
        "not_run",
        "upstream_cancelled",
        "pl-silver",
    )


def test_the_run_key_is_the_same_for_the_same_reupload_so_it_runs_once() -> None:
    entry = ChainEntry(parents=("pl-silver",), ancestors=("pl-silver",))
    runs = {("pl-silver", "root-1"): RunInfo("silver-run", "SUCCESS")}
    answer = lookups("pl-gold", runs)

    first = decide(ROOT, entry, answer, NOW)
    second = decide(ROOT, entry, answer, NOW + 1)

    assert first.run_key == second.run_key == "reupload-chain:root-1:pl-gold"


def test_a_newer_reupload_supersedes_an_older_one_that_has_not_launched() -> None:
    entry = ChainEntry(parents=("pl-silver",), ancestors=("pl-silver",))

    decision = decide(ROOT, entry, lookups("pl-gold", newer=True), NOW)

    assert decision.action == "record"
    assert decision.outcome == "superseded"


def test_a_pipeline_with_a_run_in_flight_is_not_launched_again() -> None:
    entry = ChainEntry(parents=("pl-silver",), ancestors=("pl-silver",))
    runs = {("pl-silver", "root-1"): RunInfo("silver-run", "SUCCESS")}

    decision = decide(ROOT, entry, lookups("pl-gold", runs, in_flight=True), NOW)

    assert decision.action == "wait"


def test_waiting_past_six_hours_records_wait_expired() -> None:
    entry = ChainEntry(parents=("pl-silver", "pl-other"), ancestors=("pl-silver", "pl-other"))

    decision = decide(
        ROOT,
        entry,
        lookups("pl-gold"),
        ROOT.end_time + CHAIN_WAIT_LIMIT_SECONDS + 1,
    )

    assert (decision.outcome, decision.reason, decision.cause) == (
        "not_run",
        "wait_expired",
        "pl-silver",
    )


def test_a_settled_newer_reupload_supersedes_an_older_open_one() -> None:
    """BLD-14 review R4: R2 was settled on an earlier tick and is no longer
    in the open set, but it is still a newer re-upload of the same table.
    R1 must be superseded, not launched."""
    base = time.time() - 400
    roots = [
        _root_record("R2", "orders", base + 200),
        _root_record("R1", "orders", base),
    ]
    parent = SimpleNamespace(
        run_id="silver-r1",
        job_name="authored__pl_silver",
        status=DagsterRunStatus.SUCCESS,
        tags={REUPLOAD_ROOT_TAG: "R1", REUPLOAD_TABLE_TAG: "orders"},
    )
    instance = _StubInstance(roots, [SimpleNamespace(dagster_run=parent, end_time=base)])
    context = _StubContext(
        json.dumps({"floor": 0, "settled": ["R2"]}), instance
    )
    body = _sensor(
        {"orders": ChainEntry(parents=("pl-silver",), ancestors=("pl-silver",))}
    )._raw_fn

    with patch("dispar_orchestrate.reupload_chain._post_chain_step", return_value=True) as post:
        out = list(body(context))

    assert not [item for item in out if isinstance(item, RunRequest)], (
        f"an older open re-upload must not launch: {out}"
    )
    bodies = [call.args[2] for call in post.call_args_list]
    assert any(
        b.get("rootRunId") == "R1" and b.get("outcome") == "superseded" for b in bodies
    ), f"the older root must be superseded: {bodies}"


def test_a_second_reupload_waits_for_the_next_tick_after_one_launch() -> None:
    """BLD-14 review R3: the instance does not yet hold a request yielded
    earlier in this tick, so `in_flight` cannot stop a second launch for
    another root. At most one run per pipeline per tick; the older root
    waits (and is superseded on a later tick)."""
    base = time.time() - 400
    # Equal end times: neither root supersedes the other (R4 is a separate
    # finding), so both would launch unless R3 limits the tick.
    roots = [
        _root_record("R2", "orders", base + 200),
        _root_record("R1", "orders", base + 200),
    ]

    def parent(root_id: str):
        run = SimpleNamespace(
            run_id=f"silver-{root_id}",
            job_name="authored__pl_silver",
            status=DagsterRunStatus.SUCCESS,
            tags={REUPLOAD_ROOT_TAG: root_id, REUPLOAD_TABLE_TAG: "orders"},
        )
        return SimpleNamespace(dagster_run=run, end_time=base + 200)

    instance = _StubInstance(roots, [parent("R1"), parent("R2")])
    context = _StubContext('{"floor": 0, "settled": []}', instance)
    body = _sensor(
        {"orders": ChainEntry(parents=("pl-silver",), ancestors=("pl-silver",))}
    )._raw_fn

    out = list(body(context))
    requests = [item for item in out if isinstance(item, RunRequest)]
    assert len(requests) == 1, f"one run per pipeline per tick: {out}"
    assert requests[0].run_key == "reupload-chain:R2:pl-gold"


def test_two_reuploads_with_the_newer_load_queued_start_gold_once_for_r2() -> None:
    """BLD-14 review R2: R2 ends while R1's first-pipeline run is still
    queued. Option A in the review: the chain sensor owns the first
    pipeline's launch too, so R2 gets its own tagged run; `_after` steps
    aside for the file load. Gold then supersedes R1 and runs once for R2,
    with no false wait_expired sentence."""
    base = time.time() - 400
    roots = [
        _root_record("R2", "orders", base + 200),
        _root_record("R1", "orders", base),
    ]

    def tagged(run_id: str, root_id: str, status: DagsterRunStatus, job: str = "authored__pl_first"):
        run = SimpleNamespace(
            run_id=run_id,
            job_name=job,
            status=status,
            tags={REUPLOAD_ROOT_TAG: root_id, REUPLOAD_TABLE_TAG: "orders"},
        )
        return SimpleNamespace(dagster_run=run, end_time=base + 200)

    @job(name="authored__pl_first")
    def first_job() -> None:
        return None

    @job(name="authored__pl_gold")
    def gold_job() -> None:
        return None

    first = first_pipeline("pl-first")
    gold = pipeline("pl-gold", target="gold.orders_daily", inputs=[("silver", "orders_clean")])
    jobs = [(first, first_job), (gold, gold_job)]
    cfg = AuthoredPipelineConfig(api_url="http://api.invalid", run_token="token")

    # `_after` must not also launch the first pipeline for the file load.
    after = build_authored_dependency_sensors(jobs)[0]
    after_out = list(
        after._run_status_sensor_fn(
            _StubContext(None, _StubInstance(roots), dagster_run=roots[0].dagster_run)
        )
    )
    assert len(after_out) == 1 and isinstance(after_out[0], SkipReason), (
        f"the chain owns the first pipeline's file-load launch: {after_out}"
    )
    assert "BLD-14" in after_out[0].skip_message

    chain_sensors = {
        s.name: s for s in build_reupload_chain_sensors(jobs, cfg)
    }
    first_sensor = chain_sensors["authored__pl_first_chain"]
    first_instance = _StubInstance(roots, [tagged("first-r1", "R1", DagsterRunStatus.QUEUED)])
    first_out = list(
        first_sensor._raw_fn(
            _StubContext('{"floor": 0, "settled": []}', first_instance)
        )
    )
    first_requests = [item for item in first_out if isinstance(item, RunRequest)]
    assert any(
        r.tags and r.tags.get(REUPLOAD_ROOT_TAG) == "R2" for r in first_requests
    ), f"the chain must launch R2's first run itself: {first_out}"

    # On the tick when that run has succeeded, Gold runs once for R2.
    gold_instance = _StubInstance(
        roots,
        [
            tagged("first-r1", "R1", DagsterRunStatus.QUEUED),
            tagged("first-r2", "R2", DagsterRunStatus.SUCCESS),
        ],
    )
    gold_context = _StubContext('{"floor": 0, "settled": []}', gold_instance)
    with patch("dispar_orchestrate.reupload_chain._post_chain_step", return_value=True) as post:
        out = list(chain_sensors["authored__pl_gold_chain"]._raw_fn(gold_context))

    run_requests = [item for item in out if isinstance(item, RunRequest)]
    assert len(run_requests) == 1, f"Gold must run once, for R2: {out}"
    assert run_requests[0].run_key == "reupload-chain:R2:pl-gold"
    bodies = [call.args[2] for call in post.call_args_list]
    assert any(
        b.get("rootRunId") == "R1" and b.get("outcome") == "superseded" for b in bodies
    ), f"R1 must be superseded: {bodies}"
    assert not any(b.get("reason") == "wait_expired" for b in bodies), (
        f"no false expiry while R2 loaded the table: {bodies}"
    )


def test_a_reupload_is_not_lost_behind_twenty_one_loads_of_other_tables() -> None:
    """BLD-14 review R1: a chain waits up to six hours. Twenty-one loads of
    other tables after the re-upload must not push it out of the sensor's
    view and leave it lost with no record. The target load is the OLDEST
    root here, so a recency-capped scan drops it and the test fails."""
    base = time.time() - 200
    roots = [_root_record(f"other-{i}", "other", base + i) for i in range(21)]
    roots.append(_root_record("root-target", "orders", base - 100))
    parent = SimpleNamespace(
        run_id="silver-run",
        job_name="authored__pl_silver",
        status=DagsterRunStatus.SUCCESS,
        tags={REUPLOAD_ROOT_TAG: "root-target", REUPLOAD_TABLE_TAG: "orders"},
    )
    instance = _StubInstance(
        roots,
        [SimpleNamespace(dagster_run=parent, end_time=NOW)],
    )
    context = _StubContext('{"floor": 0, "settled": []}', instance)
    body = _sensor({"orders": ChainEntry(parents=("pl-silver",), ancestors=("pl-silver",))})._raw_fn

    out = list(body(context))
    requests = [item for item in out if isinstance(item, RunRequest)]
    assert len(requests) == 1, f"the re-upload must still start Gold: {out}"
    assert requests[0].run_key == "reupload-chain:root-target:pl-gold"


def _root_record(run_id: str, table: str, end_time: float) -> SimpleNamespace:
    run = SimpleNamespace(
        run_id=run_id,
        status=DagsterRunStatus.SUCCESS,
        tags={},
        run_config={
            "ops": {"ingest_uploaded_file": {"config": {"bronze_table_name": table}}}
        },
    )
    return SimpleNamespace(dagster_run=run, end_time=end_time, start_time=end_time - 10)


class _StubInstance:
    def __init__(self, roots: list, tagged: list | None = None) -> None:
        self.roots = roots
        self.tagged = tagged or []

    def get_run_records(self, filters, limit=None):
        if filters.job_name == "file_ingest_job":
            roots = self.roots
            # The R1 fix finds open re-uploads by the six-hour window, not
            # by a recency limit: the stub must answer `updated_after`.
            if getattr(filters, "updated_after", None) is not None:
                cutoff = filters.updated_after.timestamp()
                roots = [r for r in roots if (r.end_time or 0.0) >= cutoff]
            return roots[:limit] if limit else roots
        if filters.tags and filters.tags.get(REUPLOAD_ROOT_TAG):
            root_id = filters.tags[REUPLOAD_ROOT_TAG]
            return [
                record
                for record in self.tagged
                if record.dagster_run.job_name == filters.job_name
                and (getattr(record.dagster_run, "tags", None) or {}).get(REUPLOAD_ROOT_TAG) == root_id
            ]
        return []


class _StubContext:
    def __init__(self, cursor: str | None, instance: _StubInstance, dagster_run: Any = None) -> None:
        self.cursor = cursor
        self.instance = instance
        # The `_after` run-status sensor reads the triggering run here.
        self.dagster_run = dagster_run
        self.updated: list[str] = []

    def update_cursor(self, value: str) -> None:
        self.updated.append(value)
        self.cursor = value


def _sensor(entries: dict[str, ChainEntry]) -> Any:
    @job(name="authored__pl_gold")
    def _placeholder_job() -> None:
        return None

    return build_reupload_chain_sensor(
        {"id": "pl-gold", "definition": {}, "status": "ready"},
        _placeholder_job,
        entries,
        AuthoredPipelineConfig(api_url="http://api.invalid", run_token="token"),
    )


def test_the_first_tick_sets_the_floor_and_launches_nothing() -> None:
    instance = _StubInstance([_root_record("root-1", "orders", NOW)])
    context = _StubContext(None, instance)
    body = _sensor({"orders": ChainEntry(parents=("pl-silver",), ancestors=("pl-silver",))})._raw_fn
    out = list(body(context))

    assert len(out) == 1
    assert isinstance(out[0], SkipReason)
    assert "floor" in out[0].skip_message
    cursor = json.loads(context.updated[-1])
    assert cursor["settled"] == []
    assert cursor["floor"] > 0


def test_a_root_is_settled_only_after_its_record_was_accepted() -> None:
    tagged_run = SimpleNamespace(
        run_id="gold-run",
        job_name="authored__pl_gold",
        status=DagsterRunStatus.SUCCESS,
        tags={REUPLOAD_ROOT_TAG: "root-1", REUPLOAD_TABLE_TAG: "orders"},
    )
    ended = time.time() - 10
    tagged = [SimpleNamespace(dagster_run=tagged_run, end_time=ended)]
    instance = _StubInstance([_root_record("root-1", "orders", ended)], tagged)
    context = _StubContext('{"floor": 0, "settled": []}', instance)
    body = _sensor({"orders": ChainEntry(parents=("pl-silver",), ancestors=("pl-silver",))})._raw_fn

    with patch(
        "dispar_orchestrate.reupload_chain._post_chain_step", return_value=False
    ):
        list(body(context))
    cursor = json.loads(context.updated[-1])
    assert cursor["settled"] == [], "a refused post must be retried next tick"

    context2 = _StubContext('{"floor": 0, "settled": []}', instance)
    with patch("dispar_orchestrate.reupload_chain._post_chain_step", return_value=True) as post:
        list(body(context2))
    assert post.call_count == 1
    cursor2 = json.loads(context2.updated[-1])
    assert cursor2["settled"] == ["root-1"]


def test_the_sensor_name_matches_the_api_chain_sensor_name() -> None:
    assert chain_sensor_name("pl-x") == "authored__pl_x_chain"


def test_no_token_builds_no_chain_sensors() -> None:
    cfg = AuthoredPipelineConfig(api_url="http://api.invalid", run_token="")
    assert build_reupload_chain_sensors([], cfg) == []
