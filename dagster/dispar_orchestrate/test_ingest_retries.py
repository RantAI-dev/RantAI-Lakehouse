"""The ingest ops' retries (`SRC-11` D1 and D2): which failure is retried, after
how long, and that only the failing table's step runs again.

No network: `_run_one_object` and the connector fetch are replaced. The
classification itself is tested in `test_ingest_errors.py`; this file checks
what the two ops DO with a verdict.
"""

from __future__ import annotations

from types import SimpleNamespace

import pytest
from dagster import Failure, RetryRequested, build_op_context

import dispar_orchestrate.ingest_factory as f
from dispar_orchestrate import ingest_errors, schema_observer


@pytest.fixture(autouse=True)
def _api_and_delays(monkeypatch):
    monkeypatch.delenv(ingest_errors.RETRY_DELAYS_ENV, raising=False)
    monkeypatch.setattr(
        f.IngestFactoryConfig,
        "from_env",
        staticmethod(lambda: f.IngestFactoryConfig(api_url="http://x", service_token="t")),
    )


def _attempt(retry_number: int):
    """A context of the `retry_number`th attempt of a step (the one thing a
    direct `build_op_context()` call cannot say)."""
    return SimpleNamespace(retry_number=retry_number, run_id="run-1", log_event=lambda *_: None)


def _step(context, payload):
    """The op's own function, called with a context that carries an attempt
    count (Dagster's direct invocation insists on a real context)."""
    return f.ingest_source_object.compute_fn.decorated_fn(context, payload)


def _payload(name: str = "orders"):
    return {"connector": {"id": "c"}, "obj": {"name": name, "target": name}}


def _raising(exc):
    def run(_connector, _obj, run_id=None):
        raise exc

    return run


def test_a_transient_failure_waits_one_then_five_then_fifteen_minutes(monkeypatch) -> None:
    monkeypatch.setattr(f, "_run_one_object", _raising(schema_observer.ObservationUnreachable("down")))
    waits = []
    for attempt in range(3):
        with pytest.raises(RetryRequested) as retry:
            _step(_attempt(attempt), _payload())
        waits.append(retry.value.seconds_to_wait)
        assert retry.value.max_retries == 3
    assert waits == [60, 300, 900]


def test_after_the_third_retry_the_step_fails_with_a_fixed_text_and_is_not_retried_again(monkeypatch) -> None:
    secret_text = "host db.internal.example refused"
    monkeypatch.setattr(f, "_run_one_object", _raising(ConnectionRefusedError(secret_text)))
    with pytest.raises(Failure) as failed:
        _step(_attempt(3), _payload())
    assert failed.value.allow_retries is False
    assert "gave up after 4 attempts" in str(failed.value.description)
    assert secret_text not in str(failed.value.description)


def test_the_delays_come_from_the_setting(monkeypatch) -> None:
    monkeypatch.setenv(ingest_errors.RETRY_DELAYS_ENV, "1,2")
    monkeypatch.setattr(f, "_run_one_object", _raising(TimeoutError()))
    with pytest.raises(RetryRequested) as second:
        _step(_attempt(1), _payload())
    assert (second.value.seconds_to_wait, second.value.max_retries) == (2, 2)
    with pytest.raises(Failure) as used_up:
        _step(_attempt(2), _payload())
    assert used_up.value.allow_retries is False


def test_a_bad_delay_setting_falls_back_to_the_default_waits(monkeypatch) -> None:
    monkeypatch.setenv(ingest_errors.RETRY_DELAYS_ENV, "soon")
    monkeypatch.setattr(f, "_run_one_object", _raising(TimeoutError()))
    with pytest.raises(RetryRequested) as retry:
        _step(_attempt(0), _payload())
    assert retry.value.seconds_to_wait == 60


@pytest.mark.parametrize(
    "exc",
    [
        ingest_errors.ApiRefused("no"),
        f.ssrf_guard.SsrfBlocked("blocked"),
        ValueError("bad setting"),
        RuntimeError("unexplained"),
    ],
)
def test_a_failure_that_is_not_transient_is_not_retried(monkeypatch, exc) -> None:
    monkeypatch.setattr(f, "_run_one_object", _raising(exc))
    with pytest.raises((Failure, RuntimeError)) as ended:
        _step(_attempt(0), _payload())
    assert not isinstance(ended.value, RetryRequested)
    if isinstance(ended.value, Failure):
        assert ended.value.allow_retries is False


def test_a_refused_postgres_password_ends_the_step_at_once_with_a_fixed_text(monkeypatch) -> None:
    import psycopg2

    refused = psycopg2.OperationalError('FATAL:  password authentication failed for user "g6_reader"')
    monkeypatch.setattr(f, "_run_one_object", _raising(refused))
    with pytest.raises(Failure) as ended:
        _step(_attempt(0), _payload())
    assert ended.value.allow_retries is False
    assert "g6_reader" not in str(ended.value.description)
    assert ended.value.__cause__ is refused


def test_the_fetch_of_the_connector_is_retried_when_the_api_is_down_and_not_when_it_refuses(monkeypatch) -> None:
    import requests

    def down(_cfg, _cid):
        raise requests.ConnectionError("lakehouse-api:8080 is down")

    monkeypatch.setattr(f, "_fetch_one_connector", down)
    with pytest.raises(RetryRequested) as retry:
        f._fetch_connector_for_run(f.IngestFactoryConfig.from_env(), "c", 0)
    assert retry.value.seconds_to_wait == 60

    def forbidden(_cfg, _cid):
        response = requests.Response()
        response.status_code = 403
        raise requests.HTTPError(response=response)

    monkeypatch.setattr(f, "_fetch_one_connector", forbidden)
    with pytest.raises(Failure) as refused:
        f._fetch_connector_for_run(f.IngestFactoryConfig.from_env(), "c", 0)
    assert refused.value.allow_retries is False


def test_the_kafka_path_is_retried_by_the_same_rule(monkeypatch) -> None:
    monkeypatch.setattr(
        f, "_fetch_one_connector", lambda cfg, cid: {"id": cid, "adapter": "kafka", "ingestMode": "stream"}
    )

    def broker_down(connector, run_id=None):
        raise ConnectionRefusedError("broker")

    monkeypatch.setattr(f, "_run_stream_connector", broker_down)
    context = SimpleNamespace(
        op_config={"connector_id": "k"}, retry_number=1, run_id="run-1", log=SimpleNamespace(info=lambda *_: None)
    )
    with pytest.raises(RetryRequested) as retry:
        list(f.run_ingest.compute_fn.decorated_fn(context))
    assert retry.value.seconds_to_wait == 300


def test_only_the_failing_table_is_run_again(monkeypatch) -> None:
    """Three tables, `customers` fails twice with a transient error: the job
    runs `customers` three times and `orders` and `invoices` once each."""
    monkeypatch.setenv(ingest_errors.RETRY_DELAYS_ENV, "0,0,0")
    calls: dict[str, int] = {}

    def fake(_connector, obj, run_id=None):
        calls[obj["name"]] = calls.get(obj["name"], 0) + 1
        if obj["name"] == "customers" and calls["customers"] < 3:
            raise schema_observer.ObservationUnreachable("down")
        return 5

    monkeypatch.setattr(f, "_run_one_object", fake)
    connector = {
        "id": "conn-pg",
        "adapter": "sql",
        "dial": {},
        "sourceObjects": [{"name": t, "target": t} for t in ("orders", "customers", "invoices")],
    }
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: connector)

    result = f.ingest_job.execute_in_process(
        raise_on_error=False,
        run_config={"ops": {"run_ingest": {"config": {"connector_id": "conn-pg"}}}},
    )
    assert result.success
    assert calls == {"orders": 1, "customers": 3, "invoices": 1}
    restarted = {e.step_key for e in result.all_events if e.event_type_value == "STEP_RESTARTED"}
    assert restarted == {"ingest_source_object[customers]"}
