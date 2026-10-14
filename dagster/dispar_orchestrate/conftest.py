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
    test creates: `("start", trigger)`, `("attempt", {...})`,
    `("log", event, table, count)`, `("end", status, reason)`."""

    def __init__(self, reports: list, connector_id: str, run_id: str) -> None:
        self.reports, self.connector_id, self.run_id = reports, connector_id, run_id

    def start(self, trigger, started_at=None) -> bool:
        self.reports.append(("start", trigger))
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
