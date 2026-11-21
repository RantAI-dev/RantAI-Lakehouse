"""Self-test of BLD-14 G9's chain-run wait in `upload_test.py`.

The G9 run 44812988896 failed 0.15 s after seeing a new Silver run because
the gate read it while it was still QUEUED and called that a failure. There
is no stack here: the runs route is a stub that answers QUEUED, RUNNING and
then COMPLETED, and the gate's poll sleep is patched out.
"""
from __future__ import annotations

import sys
from pathlib import Path
from unittest.mock import patch

import pytest

sys.path.insert(0, str(Path(__file__).parent))
import upload_test as g9  # noqa: E402


class FakeApi:
    def __init__(self, answers: list[list[dict]]) -> None:
        self.answers = answers
        self.calls = 0

    def get(self, url, timeout=None):
        assert "/runs" in url
        answer = self.answers[min(self.calls, len(self.answers) - 1)]
        self.calls += 1
        return _Response({"runs": answer})


class _Response:
    status_code = 200

    def __init__(self, body: dict) -> None:
        self.body = body

    def json(self) -> dict:
        return self.body


def _run(status: str) -> dict:
    return {"id": "abe2633b", "status": status, "startedAt": "", "endedAt": None}


def test_a_queued_then_running_chain_run_is_waited_for_until_it_finishes() -> None:
    api = FakeApi([[_run("queued")], [_run("running")], [_run("completed")]])
    with patch.object(g9.time, "sleep", lambda _seconds: None):
        run = g9.wait_for_terminal_chain_run(
            api, "pl-silver", set(), "the Silver chain run", 30
        )

    assert run["status"] == "completed"
    assert api.calls == 3, "a queued or running read must not end the wait"


def test_a_failed_chain_run_is_returned_as_a_failure_not_a_success() -> None:
    api = FakeApi([[_run("queued")], [_run("failed")]])
    with patch.object(g9.time, "sleep", lambda _seconds: None):
        run = g9.wait_for_terminal_chain_run(
            api, "pl-silver", set(), "the Silver chain run", 30
        )

    assert run["status"] == "failed"


def test_the_timeout_says_did_not_finish_within_n_seconds() -> None:
    api = FakeApi([[_run("queued")]])
    with patch.object(
        g9,
        "_wait_for",
        side_effect=g9.G9Failure("timed out waiting for 'the Silver chain run to finish'"),
    ):
        with pytest.raises(g9.G9Failure, match=r"did not finish within 30 s"):
            g9.wait_for_terminal_chain_run(
                api, "pl-silver", set(), "the Silver chain run", 30
            )
