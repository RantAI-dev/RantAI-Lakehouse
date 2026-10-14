"""Unit tests for `run_reports.py` -- what an ingest run tells `lakehouse-api`
about itself (`SRC-11` D5 and D7). `requests.post` is replaced: no network.
"""

from __future__ import annotations

import logging

import pytest
import requests

from dispar_orchestrate import run_reports, schema_observer


class _Api:
    """A fake `requests.post`: records every call, answers per `plan`."""

    def __init__(self) -> None:
        self.calls: list[dict] = []
        self.fail_next = 0  # how many of the next calls raise a connection error
        self.status = 200
        self.body: object = {"recorded": True}

    def post(self, url, json=None, headers=None, timeout=None):
        self.calls.append({"url": url, "json": json, "headers": headers, "timeout": timeout})
        if self.fail_next > 0:
            self.fail_next -= 1
            raise requests.ConnectionError("lakehouse-api:8080 unreachable")
        api = self

        class _Response:
            status_code = api.status

            def raise_for_status(self) -> None:
                if api.status >= 400:
                    raise requests.HTTPError(response=self)

            def json(self):
                if api.body is None:
                    raise ValueError("not json")
                return api.body

        return _Response()


@pytest.fixture
def api(monkeypatch) -> _Api:
    fake = _Api()
    monkeypatch.setattr(schema_observer.requests, "post", fake.post)
    return fake


def _reporter(token: str = "svc-token") -> run_reports.RunReporter:
    cfg = schema_observer.ObserverConfig(api_url="http://api:8080", service_token=token)
    return run_reports.RunReporter(cfg, "conn-1", "run-1")


def test_the_start_report_names_the_run_and_what_started_it(api) -> None:
    assert _reporter().start("schedule", "2026-10-14T10:00:00+00:00")
    [call] = api.calls
    assert call["url"] == "http://api:8080/api/connectors/conn-1/run-reports"
    assert call["json"] == {"runId": "run-1", "trigger": "schedule", "startedAt": "2026-10-14T10:00:00+00:00"}


def test_a_refresh_start_report_names_its_table_and_an_ordinary_one_sends_none(api) -> None:
    # SRC-11 review 1 BLOCKER 2: the run list says "Refresh · <table>".
    _reporter().start("refresh", table="public.orders")
    _reporter().start("schedule")
    assert api.calls[0]["json"] == {"runId": "run-1", "trigger": "refresh", "table": "public.orders"}
    assert "table" not in api.calls[1]["json"]


def test_every_report_carries_the_service_token_and_a_timeout(api) -> None:
    reporter = _reporter()
    reporter.start("user")
    reporter.end("succeeded", "completed")
    assert [c["headers"] for c in api.calls] == [{"Authorization": "Bearer svc-token"}] * 2
    assert all(isinstance(c["timeout"], (int, float)) and c["timeout"] > 0 for c in api.calls)


def test_a_trigger_outside_the_closed_list_is_reported_as_other(api) -> None:
    _reporter().start("a person pressed a button")
    assert api.calls[0]["json"]["trigger"] == "other"


def test_a_successful_attempt_carries_rows_and_no_error_class(api) -> None:
    _reporter().attempt(table="shop.orders", attempt=1, started_at="a", ended_at="b", rows_loaded=42)
    [call] = api.calls
    assert call["url"].endswith("/api/connectors/conn-1/run-reports/run-1/attempts")
    assert call["json"] == {"table": "shop.orders", "attempt": 1, "startedAt": "a", "endedAt": "b", "rowsLoaded": 42}


def test_a_failed_attempt_carries_its_class_and_the_wait_before_the_next(api) -> None:
    _reporter().attempt(
        table="t", attempt=2, started_at="a", ended_at="b", error_class="unreachable", wait_seconds=300
    )
    assert api.calls[0]["json"] == {
        "table": "t", "attempt": 2, "startedAt": "a", "endedAt": "b", "rowsLoaded": 0,
        "errorClass": "unreachable", "waitSeconds": 300,
    }


def test_a_wait_is_never_sent_with_a_success(api) -> None:
    _reporter().attempt(table="t", attempt=1, started_at="a", ended_at="b", wait_seconds=60)
    assert "waitSeconds" not in api.calls[0]["json"] and "errorClass" not in api.calls[0]["json"]


def test_a_log_line_carries_only_its_event_table_and_count(api) -> None:
    reporter = _reporter()
    reporter.log("columns_left_out", table="shop.orders", count=3)
    reporter.log("report_gap")
    assert [c["json"] for c in api.calls] == [
        {"event": "columns_left_out", "table": "shop.orders", "count": 3},
        {"event": "report_gap"},
    ]
    assert api.calls[0]["url"].endswith("/run-reports/run-1/log")


def test_the_end_report_names_status_reason_and_time(api) -> None:
    _reporter().end("paused", "connector_paused")
    [call] = api.calls
    assert call["url"].endswith("/run-reports/run-1/end")
    assert call["json"]["status"] == "paused" and call["json"]["reason"] == "connector_paused"
    assert call["json"]["endedAt"]


def test_a_report_that_cannot_be_sent_does_not_raise_and_is_logged_without_the_error_text(api, caplog) -> None:
    api.fail_next = 1
    with caplog.at_level(logging.WARNING, logger=run_reports.logger.name):
        sent = _reporter().start("schedule")
    assert sent is False
    assert "lakehouse-api:8080" not in caplog.text and "svc-token" not in caplog.text
    assert "run 1" not in caplog.text and "run-1" in caplog.text


@pytest.mark.parametrize("status", [400, 404, 403, 500, 503])
def test_an_error_answer_is_a_report_not_sent_never_an_exception(api, status) -> None:
    api.status = status
    assert _reporter().end("succeeded", "completed") is False


def test_an_unset_token_sends_nothing_and_does_not_raise(api) -> None:
    assert _reporter(token="").start("schedule") is False
    assert api.calls == []


def test_an_answer_that_is_not_json_is_a_report_not_sent(api) -> None:
    api.body = None  # the fake raises `ValueError` from `.json()` for a `None` body
    assert _reporter().start("schedule") is False


def test_the_next_report_that_can_be_sent_says_the_log_may_be_incomplete(api) -> None:
    reporter = _reporter()
    api.fail_next = 1
    assert reporter.attempt(table="t", attempt=1, started_at="a", ended_at="b") is False
    assert reporter.has_gap
    assert reporter.end("failed", "table_failed") is True
    assert [c["json"].get("event") or c["json"].get("status") for c in api.calls[1:]] == ["report_gap", "failed"]
    assert not reporter.has_gap
    # And only once.
    reporter.end("failed", "table_failed")
    assert [c["json"].get("event") for c in api.calls].count("report_gap") == 1


def test_while_the_api_still_refuses_reports_the_gap_stays_and_nothing_else_is_tried(api) -> None:
    reporter = _reporter()
    api.fail_next = 2
    reporter.start("schedule")  # fails
    assert reporter.end("succeeded", "completed") is False  # the gap line fails, so the end is not tried
    assert reporter.has_gap
    assert len(api.calls) == 2


def test_the_gap_line_itself_does_not_trigger_another_gap_line(api) -> None:
    reporter = _reporter()
    api.fail_next = 1
    reporter.start("schedule")
    assert reporter.log("report_gap") is True
    assert [c["json"].get("event") for c in api.calls[1:]] == ["report_gap"]


def test_for_run_reads_the_environment_once_through_the_observers_config(monkeypatch) -> None:
    monkeypatch.setenv("LAKEHOUSE_API_URL", "http://api.example:9")
    monkeypatch.setenv("INGEST_SERVICE_TOKEN", "tok")
    cfg = schema_observer.ObserverConfig.from_env()
    assert (cfg.api_url, cfg.service_token) == ("http://api.example:9", "tok")
    # `conftest.py` replaces `for_run` for every test; the class is still the real one.
    assert run_reports.RunReporter(cfg, "c", "r").has_gap is False
