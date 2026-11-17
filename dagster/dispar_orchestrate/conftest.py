"""Shared pytest fixtures for the `dispar_orchestrate` unit tests.

`reports` replaces `run_reports.RunReporter.for_run` for EVERY test (autouse):
`run_ingest` and `ingest_source_object` report to `lakehouse-api` (SRC-11 D5),
and a unit test must not reach a network, nor wait on a refused connection.
The fake records each report in order, so a test that cares can assert on what
the ops would have sent; the reporter's own behaviour is tested in
`test_run_reports.py`, which builds `RunReporter` directly and never calls
`for_run`.
"""

from __future__ import annotations

import pytest

from dispar_orchestrate import run_reports


class RecordingReporter:
    """Stands in for `RunReporter`. `reports` is shared by every reporter a
    test creates: `("start", trigger[, table])`, `("attempt", {...})`,
    `("log", event, table, count)`, `("end", status, reason)`."""

    def __init__(self, reports: list, connector_id: str, run_id: str) -> None:
        self.reports, self.connector_id, self.run_id = reports, connector_id, run_id

    def start(self, trigger, started_at=None, table=None) -> bool:
        # `("start", trigger)`, or `("start", trigger, table)` for a refresh or
        # clear, which names its table (SRC-11 review 1 BLOCKER 2).
        self.reports.append(("start", trigger) if table is None else ("start", trigger, table))
        return True

    def attempt(
        self, *, table, attempt, started_at, ended_at, rows_loaded=0, error_class=None, wait_seconds=None
    ) -> bool:
        # The real signature, spelled out: a keyword the ops send that
        # `RunReporter.attempt` does not take must fail here too.
        self.reports.append(
            (
                "attempt",
                {
                    "table": table,
                    "attempt": attempt,
                    "started_at": started_at,
                    "ended_at": ended_at,
                    "rows_loaded": rows_loaded,
                    "error_class": error_class,
                    "wait_seconds": wait_seconds,
                },
            )
        )
        return True

    def log(self, event, *, table=None, count=None) -> bool:
        self.reports.append(("log", event, table, count))
        return True

    def end(self, status, reason) -> bool:
        self.reports.append(("end", status, reason))
        return True


@pytest.fixture(autouse=True)
def reports(monkeypatch) -> list:
    sent: list = []
    monkeypatch.setattr(
        run_reports.RunReporter,
        "for_run",
        classmethod(lambda cls, connector_id, run_id: RecordingReporter(sent, connector_id, run_id)),
    )
    return sent


@pytest.fixture(autouse=True)
def connector_s3_test_credentials(monkeypatch) -> None:
    """`SEC-37`: `BronzeIngestConfig.from_env` (and `GoldExportConfig`'s)
    REQUIRE the two connector-scoped S3 credentials — the `rustfsadmin`
    fallback is gone. Most unit tests never reach S3 but still build the
    config (the factory, the sink adapter), so the suite supplies throwaway
    values here; a test that asserts the missing-variable error removes one
    itself, and so does `mock.patch.dict(..., clear=True)`."""
    monkeypatch.setenv("CONNECTOR_S3_ACCESS_KEY", "test-access-key")
    monkeypatch.setenv("CONNECTOR_S3_SECRET_KEY", "test-secret-key")


class FakeManagerHttp:
    """Replaces `secret_manager.requests.request` (`SRC-12` part B): canned
    answers by method and URL, every call recorded, no network. A request no
    route matches raises a connection error, so a test that forgot a route
    sees "unreachable", never a real call."""

    def __init__(self) -> None:
        self.calls: list[dict] = []
        self._routes: list[dict] = []

    def add(self, method: str, url: str, *, status: int = 200, json=None, text: str | None = None, times=None) -> None:
        self._routes.append({"method": method, "url": url, "status": status, "json": json, "text": text, "times": times})

    def __call__(self, method, url, **kwargs):
        import json as jsonlib

        import requests

        self.calls.append({"method": method, "url": url, **kwargs})
        for route in self._routes:
            if route["method"] == method and url.split("?")[0] == route["url"].split("?")[0]:
                if route["times"] is not None:
                    if route["times"] <= 0:
                        continue
                    route["times"] -= 1
                response = requests.Response()
                response.status_code = route["status"]
                if route["text"] is not None:
                    response._content = route["text"].encode()
                else:
                    response._content = jsonlib.dumps(route["json"]).encode()
                response.url = url
                return response
        raise requests.ConnectionError("no route in the fake")


@pytest.fixture
def manager_http(monkeypatch: pytest.MonkeyPatch) -> FakeManagerHttp:
    from dispar_orchestrate import secret_manager

    fake = FakeManagerHttp()
    monkeypatch.setattr(secret_manager.requests, "request", fake)
    return fake
