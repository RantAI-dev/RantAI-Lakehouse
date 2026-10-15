"""A run of ONE table with an action (`SRC-10` decision 5): `refresh_keep`,
`refresh_remove` and `clear`, from the run config's optional `object` and
`action` (`ingest_factory.run_ingest`, `_run_one_object`).

No network: the connector fetch, the sink and the record of the run are faked, as
in `test_ingest_factory.py`, whose fixtures and wiring helpers are reused. What
the sink does with the plan (forgetting the cursor and the marker, emptying the
table) is proved on a real local table in `test_sink_refresh.py` and
`test_sink_empty_table.py`.
"""

from __future__ import annotations

import pytest
from dagster import Failure, build_op_context

import dispar_orchestrate.ingest_factory as f
from dispar_orchestrate.test_ingest_factory import (  # noqa: F401 -- fixtures and wiring
    _batch_connector,
    _mysql_connector,
    _no_catalog_registration,
    _stub_env,
    _wire_mysql,
    observations,
)


def _fan_out(config: dict) -> list[dict]:
    return [dyn.value for dyn in f.run_ingest(build_op_context(op_config=config))]


def test_a_scheduled_run_config_with_only_a_connector_id_still_validates_and_runs_every_table(monkeypatch) -> None:
    _stub_env(monkeypatch)
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: _batch_connector(["a", "b"]))
    discovered: list = []
    monkeypatch.setattr(f, "_discover_new_tables", lambda connector, run_id: discovered.append(1) or False)
    seen: list = []
    monkeypatch.setattr(f, "_run_one_object", lambda connector, obj, run_id=None: seen.append(obj["name"]) or 1)

    result = f.ingest_job.execute_in_process(
        raise_on_error=False,
        run_config={"ops": {"run_ingest": {"config": {"connector_id": "conn-pg"}}}},
    )

    assert result.success
    assert sorted(seen) == ["a", "b"]
    assert discovered, "an ordinary run still looks for new tables"
    assert all(p["action"] is None for p in _fan_out({"connector_id": "conn-pg"}))


@pytest.mark.parametrize("action", ["refresh_keep", "refresh_remove", "clear"])
def test_a_run_of_one_table_fans_out_over_that_table_only_and_carries_the_action(monkeypatch, action) -> None:
    _stub_env(monkeypatch)
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: _batch_connector(["a", "b", "c"]))
    discovered: list = []
    monkeypatch.setattr(f, "_discover_new_tables", lambda connector, run_id: discovered.append(1) or False)

    payloads = _fan_out({"connector_id": "conn-pg", "object": "b", "action": action})

    assert [(p["obj"]["name"], p["action"]) for p in payloads] == [("b", action)]
    assert discovered == [], "a run of one table does not go looking for new tables"


def test_the_run_config_schema_accepts_object_and_action_through_the_job(monkeypatch) -> None:
    _stub_env(monkeypatch)
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: _batch_connector(["a", "b"]))
    seen: list = []
    monkeypatch.setattr(f, "_run_one_object", lambda connector, obj, run_id=None, action=None: seen.append((obj["name"], action)) or 7)

    result = f.ingest_job.execute_in_process(
        raise_on_error=False,
        run_config={"ops": {"run_ingest": {"config": {"connector_id": "conn-pg", "object": "b", "action": "clear"}}}},
    )

    assert result.success
    assert seen == [("b", "clear")]


@pytest.mark.parametrize(
    "config",
    [
        {"object": "a"},
        {"action": "clear"},
        {"object": "a", "action": "drop_everything"},
        {"object": "  ", "action": "clear"},
        {"object": "not_selected", "action": "clear"},
    ],
)
def test_a_half_a_request_a_bad_action_or_an_unselected_table_fails_closed_without_retries(monkeypatch, config) -> None:
    _stub_env(monkeypatch)
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: _batch_connector(["a", "b"]))
    with pytest.raises(Failure) as raised:
        _fan_out({"connector_id": "conn-pg", **config})
    assert raised.value.allow_retries is False


@pytest.mark.parametrize("connector", [{"adapter": "cdc"}, {"adapter": "kafka", "ingestMode": "stream"}])
def test_a_streaming_connector_does_not_run_one_table_with_an_action(monkeypatch, connector) -> None:
    _stub_env(monkeypatch)
    full = {**_batch_connector(["a"]), **connector}
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: full)
    with pytest.raises(Failure) as raised:
        _fan_out({"connector_id": "conn-pg", "object": "a", "action": "clear"})
    assert raised.value.allow_retries is False


def test_clear_empties_the_table_loads_nothing_and_records_cleared_with_zero_rows(monkeypatch, observations) -> None:
    recorded: list = []
    cleared: list = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.sink_adapter, "clear_table", lambda target, config: cleared.append(target))
    monkeypatch.setattr(f.sink_adapter.SinkConfig, "from_bronze_ingest_config", classmethod(lambda cls, c: object()))
    monkeypatch.setattr(f.dlt_pipeline.BronzeIngestConfig, "from_env", staticmethod(lambda: object()))
    monkeypatch.setattr(f.sink_adapter, "load_via_sink", lambda *a, **k: pytest.fail("a clear loads nothing"))
    # No secret is resolved and no source is dialled for a clear.
    monkeypatch.setattr(f, "_resolve_object_secrets", lambda *a: pytest.fail("a clear resolves no secret"))

    rows = f._run_one_object(
        _mysql_connector(), {"name": "shop.orders", "target": "orders"}, run_id="run-1", action="clear"
    )

    assert rows == 0
    assert cleared == ["orders"]
    assert [(r["status"], r["rows"], r["object_name"], r["error"]) for r in recorded] == [
        ("cleared", 0, "shop.orders", "")
    ]
    assert observations.calls == [], "a clear makes no schema observation: the baseline is about the source"


def test_a_clear_that_fails_records_failed_with_the_exception_type_only_and_reraises(monkeypatch) -> None:
    recorded: list = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.sink_adapter.SinkConfig, "from_bronze_ingest_config", classmethod(lambda cls, c: object()))
    monkeypatch.setattr(f.dlt_pipeline.BronzeIngestConfig, "from_env", staticmethod(lambda: object()))

    def boom(target, config):
        raise RuntimeError("SECRET-HOST-NAME unreachable")

    monkeypatch.setattr(f.sink_adapter, "clear_table", boom)

    with pytest.raises(RuntimeError):
        f._run_one_object(_mysql_connector(), {"name": "shop.orders", "target": "orders"}, action="clear")

    assert [(r["status"], r["rows"]) for r in recorded] == [("failed", None)]
    assert "SECRET-HOST-NAME" not in recorded[0]["error"]


@pytest.mark.parametrize("action, refresh", [("refresh_keep", "keep"), ("refresh_remove", "remove"), (None, None)])
def test_a_refresh_loads_by_the_tables_mode_under_a_plan_that_says_how_to_refresh(
    monkeypatch, observations, action, refresh
) -> None:
    plans: list = []
    recorded = _wire_mysql(monkeypatch, loads=[], builds=[])

    def capture(source, target, sink_config, plan):
        plans.append(plan)
        return f.sink_adapter.SinkResult(rows=5, has_failed_jobs=False, load_info_str="")

    monkeypatch.setattr(f.sink_adapter, "load_via_sink", capture)
    obj = {"name": "shop.orders", "target": "orders", "loadMode": "incremental_dedup", "incrementalKey": "id",
           "primaryKey": ["id"]}

    rows = f._run_one_object(_mysql_connector(), obj, run_id="r", **({"action": action} if action else {}))

    assert rows == 5
    [plan] = plans
    assert (plan.mode, plan.cursor, plan.key, plan.refresh) == ("incremental_dedup", "id", ("id",), refresh)
    assert [r["status"] for r in recorded] == ["succeeded"]
    assert len(observations.calls) == 1, "a refresh is an ordinary load: the schema check runs"


def test_a_refresh_of_a_table_the_api_holds_back_loads_nothing_and_records_waiting(monkeypatch, observations) -> None:
    loads: list = []
    recorded = _wire_mysql(monkeypatch, loads=loads, builds=[])
    observations.answer = f.schema_observer.Decision(action="wait", columns=None, changes=[{"kind": "type_changed"}])

    rows = f._run_one_object(
        _mysql_connector(), {"name": "shop.orders", "target": "orders"}, action="refresh_remove"
    )

    assert rows is None and loads == []
    assert [r["status"] for r in recorded] == ["waiting"]


def test_the_refresh_of_a_load_plan_must_be_keep_or_remove() -> None:
    from dispar_orchestrate.adapters.sink import LoadPlan, UnsupportedLoadMode

    assert LoadPlan(mode="replace", refresh="keep").refresh == "keep"
    with pytest.raises(UnsupportedLoadMode):
        LoadPlan(mode="replace", refresh="clear")
