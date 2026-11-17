"""dagster/dispar_orchestrate/run_reports.py -- what an ingest run tells
`lakehouse-api` about itself, for the connector's run history and run log
(`SRC-11` decisions D5 and D7).

The API keeps a product-owned record of each run of a connector (who started
it, each attempt at each table, why it ended) and a log of the product's OWN
words about it. It cannot see inside the orchestrator, so the orchestrator
reports, over the four routes under `POST /api/connectors/{id}/run-reports`
(`rust/crates/lakehouse-api/src/routes/connector_runs.rs`):

    start    the run began, and what began it
    attempt  one attempt at one table ended (rows, or an error class and the
             wait before the next attempt)
    log      one line of a closed list: `schema_decision_pending`,
             `columns_left_out`, `report_gap`
    end      the run ended (status and reason)

Every value sent is a number, a time, a table name as the spec spells it, or a
word of a closed list. Never an exception's text: the API's own run log is
built from fixed phrases for the same reason (AGENTS.md principle 4).

# A report never fails the load

The rows are the product; the report is about them. A report that cannot be
sent (the API is down, the token is unset, a 4xx) is logged as a warning that
names the kind of failure and nothing else, and the load goes on. It is not
hidden from the reader either (AGENTS.md principle 2): the next report that CAN
be sent is preceded by a `report_gap` line, so the run log says it may be
incomplete. A gap is remembered per reporter, that is per step; two steps of a
run do not know each other's gaps (they may be different processes).

The service token and the POST are `schema_observer`'s (`ObserverConfig`,
`_post_json`): the same `ingest:read` identity, the same 10-second `timeout=`
and `raise_for_status()`. This module adds no second HTTP helper.
"""

from __future__ import annotations

import logging
from datetime import datetime, timezone
from typing import Any

from dispar_orchestrate import schema_observer

logger = logging.getLogger(__name__)

# The closed lists the API accepts (`connector_run.rs`); a value outside one is
# a 400 there, so the constants here are the only spellings used.
TRIGGERS = ("schedule", "user", "refresh", "clear", "other")
RUN_STATUSES = ("succeeded", "failed", "paused")
END_REASONS = ("completed", "table_failed", "connector_paused", "other")
LOG_EVENTS = ("schema_decision_pending", "columns_left_out", "report_gap")


def now() -> str:
    """The current time as an RFC 3339 string in UTC."""
    return datetime.now(timezone.utc).isoformat()


class RunReporter:
    """Sends one run's reports. Every method returns whether the report was
    sent and never raises."""

    def __init__(self, cfg: schema_observer.ObserverConfig, connector_id: str, run_id: str) -> None:
        self._cfg = cfg
        self._base = f"/api/connectors/{connector_id}/run-reports"
        self._run_id = run_id
        # An earlier report could not be sent: the next one is preceded by a
        # `report_gap` line.
        self._gap = False

    @classmethod
    def for_run(cls, connector_id: str, run_id: str) -> "RunReporter":
        return cls(schema_observer.ObserverConfig.from_env(), connector_id, run_id)

    @property
    def has_gap(self) -> bool:
        return self._gap

    def start(self, trigger: str, started_at: str | None = None, table: str | None = None) -> bool:
        """The run began. `trigger` is one of `TRIGGERS`. `table` is the one
        source object a `refresh` or `clear` run is about (SRC-11 review 1
        BLOCKER 2: the run list names it, SRC-10 decision 5); an ordinary run
        of every table sends none."""
        if trigger not in TRIGGERS:
            trigger = "other"
        payload: dict[str, Any] = {"runId": self._run_id, "trigger": trigger}
        if table:
            payload["table"] = table[:256]
        if started_at:
            payload["startedAt"] = started_at
        return self._send(self._base, payload, "run start report")

    def attempt(
        self,
        *,
        table: str,
        attempt: int,
        started_at: str,
        ended_at: str,
        rows_loaded: int = 0,
        error_class: str | None = None,
        wait_seconds: int | None = None,
    ) -> bool:
        """One attempt at one table ended. `error_class` absent means it
        succeeded; `wait_seconds` only for a failure that will be retried."""
        payload: dict[str, Any] = {
            "table": table[:256],
            "attempt": attempt,
            "startedAt": started_at,
            "endedAt": ended_at,
            "rowsLoaded": max(0, rows_loaded),
        }
        if error_class is not None:
            payload["errorClass"] = error_class
            if wait_seconds is not None:
                payload["waitSeconds"] = wait_seconds
        return self._send(f"{self._base}/{self._run_id}/attempts", payload, "attempt report")

    def log(self, event: str, *, table: str | None = None, count: int | None = None) -> bool:
        """One line of the closed list `LOG_EVENTS`."""
        payload: dict[str, Any] = {"event": event}
        if table is not None:
            payload["table"] = table[:256]
        if count is not None:
            payload["count"] = count
        return self._send(f"{self._base}/{self._run_id}/log", payload, f"{event} log line", is_gap_line=event == "report_gap")

    def end(self, status: str, reason: str) -> bool:
        """The run ended. `status` is one of `RUN_STATUSES`, `reason` one of
        `END_REASONS`."""
        return self._send(
            f"{self._base}/{self._run_id}/end", {"status": status, "reason": reason, "endedAt": now()}, "run end report"
        )

    def _send(self, path: str, payload: dict[str, Any], what: str, *, is_gap_line: bool = False) -> bool:
        if self._gap and not is_gap_line:
            # Say so first. When even that cannot be sent the API is not
            # taking reports, and this one would meet the same answer.
            if not self._post(f"{self._base}/{self._run_id}/log", {"event": "report_gap"}, "report_gap log line"):
                return False
            self._gap = False
        sent = self._post(path, payload, what)
        if not sent:
            self._gap = True
        return sent

    def _post(self, path: str, payload: dict[str, Any], what: str) -> bool:
        try:
            schema_observer._post_json(self._cfg, path, payload, what)
        except (schema_observer.ObservationRefused, schema_observer.ObservationUnreachable) as exc:
            # Their messages are fixed: the kind of failure and an HTTP status.
            logger.warning("the %s of run %s was not sent (the load goes on): %s", what, self._run_id, exc)
            return False
        except Exception as exc:  # noqa: BLE001 -- a report never fails the load, whatever went wrong
            logger.warning(
                "the %s of run %s was not sent (the load goes on): %s", what, self._run_id, type(exc).__name__
            )
            return False
        return True
