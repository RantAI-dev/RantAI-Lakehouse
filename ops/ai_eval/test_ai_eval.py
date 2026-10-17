"""Unit tests for the Copilot eval harness (`ai_eval.py`).

No network: every test calls pure functions or a fake session. The seams are
the functions the harness runs in order: `run_case` (one case, its turns, its
setup and its cleanup), `pass_rule` and `verdict` (how runs become a case and
a set), `confirm_body` (the request the console sends), `cleanup_plan` (what
the `finally` deletes), `expect_state_problems` (the state check), and `main`
(the exit codes).

The 28 existing cases must score as they did before the dashboard features:
`test_existing_cases_rescore_to_the_numbers_recorded_before` replays a saved
run (`fixtures/existing_cases_saved.json`) through `--rescore`'s path and
compares every score with the values the code printed before the change
(`fixtures/existing_cases_expected.json`).
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import ai_eval as m  # noqa: E402

FIXTURES = HERE / "fixtures"


def make_cfg() -> m.Config:
    return m.Config(
        api_url="http://api.test",
        ch_url="http://ch.test",
        ch_user="u",
        ch_password="",
        email="e@example.com",
        password="p",
        timeout_s=5,
    )


class FakeResponse:
    def __init__(self, status: int = 200, body: Any = None) -> None:
        self.status_code = status
        self._body = body if body is not None else {}

    def json(self) -> Any:
        return self._body

    def raise_for_status(self) -> None:
        if self.status_code >= 400:
            raise m.requests.HTTPError(f"HTTP {self.status_code}")


class FakeSession:
    """Records every call; answers from a script keyed by (method, path)."""

    def __init__(self, replies: dict[tuple[str, str], Any] | None = None) -> None:
        self.replies = replies or {}
        self.calls: list[tuple[str, str, Any]] = []

    def _answer(self, method: str, url: str, body: Any) -> FakeResponse:
        path = url.split("http://api.test", 1)[-1].split("?", 1)[0]
        key = (method, path)
        self.calls.append((method, url, body))
        reply = self.replies.get(key)
        if isinstance(reply, FakeResponse):
            return reply
        return FakeResponse(200, reply if reply is not None else {"ok": True})

    def request(self, method: str, url: str, json: Any = None, **_: Any) -> FakeResponse:
        return self._answer(method.upper(), url, json)

    def get(self, url: str, **_: Any) -> FakeResponse:
        return self._answer("GET", url, None)

    def post(self, url: str, json: Any = None, **_: Any) -> FakeResponse:
        return self._answer("POST", url, json)

    def delete(self, url: str, **_: Any) -> FakeResponse:
        return self._answer("DELETE", url, None)


def confirm_reply(tool: str, result: dict[str, Any], outcome: str = "executed", status: int = 200) -> dict[str, Any]:
    """What `send_confirm` returns: the `POST /api/ai/tool` reply
    `{tool, outcome, result}` with the HTTP status added as `_status`."""
    return {"tool": tool, "outcome": outcome, "result": result, "_status": status}


# ── pass rule and set verdict ─────────────────────────────────────────────


class PassRuleTests(unittest.TestCase):
    def test_two_of_three_runs_pass_the_case(self) -> None:
        runs = [{"problems": [], "missing": []}, {"problems": ["x"], "missing": []}, {"problems": [], "missing": []}]
        case = m.case_verdict("c", runs, need=2)
        self.assertTrue(case["passed"])

    def test_one_of_three_runs_fails_the_case(self) -> None:
        runs = [{"problems": [], "missing": []}, {"problems": [], "missing": ["fact=1"]}, {"problems": ["x"], "missing": []}]
        case = m.case_verdict("c", runs, need=2)
        self.assertFalse(case["passed"])

    def test_a_run_passes_with_no_missing_and_no_problems(self) -> None:
        self.assertTrue(m.run_passed({"missing": [], "problems": []}))
        self.assertFalse(m.run_passed({"missing": ["a"], "problems": []}))
        self.assertFalse(m.run_passed({"missing": [], "problems": ["p"]}))

    def test_a_single_plain_run_is_one_run_and_passes_on_its_own(self) -> None:
        case = m.case_verdict("c", [{"problems": [], "missing": []}], need=1)
        self.assertTrue(case["passed"])
        self.assertEqual(len(case["runs"]), 1)


class SetVerdictTests(unittest.TestCase):
    def cases(self, passed: int, total: int) -> list[dict[str, Any]]:
        return [{"id": f"c{i}", "passed": i < passed} for i in range(total)]

    def test_the_set_passes_at_exactly_ninety_percent(self) -> None:
        verdict = m.set_verdict(self.cases(passed=9, total=10), baseline=None)
        self.assertTrue(verdict["pass"])
        self.assertEqual(verdict["passed"], 9)

    def test_the_set_fails_just_below_ninety_percent(self) -> None:
        verdict = m.set_verdict(self.cases(passed=8, total=9), baseline=None)
        self.assertFalse(verdict["pass"])

    def test_thirty_cases_pass_at_twenty_seven(self) -> None:
        self.assertTrue(m.set_verdict(self.cases(27, 30), baseline=None)["pass"])
        self.assertFalse(m.set_verdict(self.cases(26, 30), baseline=None)["pass"])

    def test_a_case_that_passed_in_the_baseline_and_fails_now_is_a_regression(self) -> None:
        # c0 passed in the baseline and fails now: a regression. The rest pass.
        cases = [{"id": "c0", "passed": False}] + [{"id": f"c{i}", "passed": True} for i in range(1, 10)]
        baseline = {"c0": True}
        verdict = m.set_verdict(cases, baseline=baseline)
        self.assertEqual(verdict["regressed"], ["c0"])
        self.assertFalse(verdict["pass"])

    def test_a_regression_fails_the_set_even_above_ninety_percent(self) -> None:
        cases = [{"id": f"c{i}", "passed": True} for i in range(20)]
        cases[3]["passed"] = False
        baseline = {"c3": True}
        verdict = m.set_verdict(cases, baseline=baseline)
        # 19 of 20 is 95%, above the bar, yet the set fails.
        self.assertGreaterEqual(verdict["passed"] / verdict["total"], 0.9)
        self.assertEqual(verdict["regressed"], ["c3"])
        self.assertFalse(verdict["pass"])

    def test_a_new_case_that_fails_is_not_a_regression(self) -> None:
        cases = [{"id": f"c{i}", "passed": True} for i in range(10)]
        cases.append({"id": "new", "passed": False})
        baseline = {f"c{i}": True for i in range(10)}
        verdict = m.set_verdict(cases, baseline=baseline)
        self.assertEqual(verdict["regressed"], [])
        self.assertTrue(verdict["pass"])

    def test_no_regression_when_the_baseline_case_still_passes(self) -> None:
        verdict = m.set_verdict(self.cases(10, 10), baseline={"c0": True})
        self.assertEqual(verdict["regressed"], [])
        self.assertTrue(verdict["pass"])

    def test_a_case_that_failed_in_the_baseline_is_not_a_regression_when_it_still_fails(self) -> None:
        cases = self.cases(10, 11)
        verdict = m.set_verdict(cases, baseline={"c10": False})
        self.assertEqual(verdict["regressed"], [])

    def test_the_summary_line_names_the_failed_and_regressed_cases(self) -> None:
        # a passed in the baseline and fails now; b fails and is new.
        cases = [{"id": "a", "passed": False}, {"id": "b", "passed": False}, {"id": "c", "passed": True}]
        line = m.summary_line(m.set_verdict(cases, baseline={"a": True}))
        self.assertTrue(line.startswith("FAIL"))
        self.assertIn("regressions: a", line)
        self.assertIn("failed: a, b", line)

    def test_the_summary_line_for_a_pass_says_no_regressions(self) -> None:
        cases = [{"id": f"c{i}", "passed": True} for i in range(10)]
        line = m.summary_line(m.set_verdict(cases, baseline=None))
        self.assertEqual(line, "PASS 10/10 (100%), regressions: none")


# ── confirm request, built as use-copilot.ts builds it ─────────────────────


class ConfirmBodyTests(unittest.TestCase):
    def test_the_title_is_copied_from_caption_when_missing(self) -> None:
        pending = {"tool": "create_chart", "args": {"kind": "bar", "caption": "Events per year"}}
        body = m.confirm_body(pending, mode="build", session_id=None)
        self.assertEqual(body["args"]["title"], "Events per year")

    def test_the_title_is_copied_from_name_then_text(self) -> None:
        by_name = m.confirm_body({"tool": "create_board", "args": {"name": "Sales"}}, "build", None)
        self.assertEqual(by_name["args"]["title"], "Sales")
        by_text = m.confirm_body({"tool": "create_chart", "args": {"text": "Note"}}, "build", None)
        self.assertEqual(by_text["args"]["title"], "Note")

    def test_an_existing_title_is_not_overwritten(self) -> None:
        pending = {"tool": "create_chart", "args": {"title": "Kept", "caption": "Other"}}
        body = m.confirm_body(pending, "build", None)
        self.assertEqual(body["args"]["title"], "Kept")

    def test_a_falsy_title_is_replaced_by_caption(self) -> None:
        # The console tests `!rawArgs.title`, so an empty title counts as missing.
        pending = {"tool": "create_chart", "args": {"title": "", "caption": "C"}}
        self.assertEqual(m.confirm_body(pending, "build", None)["args"]["title"], "C")

    def test_no_title_is_invented_when_there_is_no_source_for_one(self) -> None:
        body = m.confirm_body({"tool": "delete_chart", "args": {"id": "c1"}}, "build", None)
        self.assertNotIn("title", body["args"])

    def test_confirmed_true_is_always_sent(self) -> None:
        body = m.confirm_body({"tool": "create_chart", "args": {"kind": "bar"}}, "build", None)
        self.assertIs(body["args"]["confirmed"], True)

    def test_the_request_carries_tool_mode_and_session(self) -> None:
        body = m.confirm_body({"tool": "create_chart", "args": {}}, "ask", "ses1")
        self.assertEqual(body["tool"], "create_chart")
        self.assertEqual(body["mode"], "ask")
        self.assertEqual(body["sessionId"], "ses1")

    def test_session_id_is_null_when_absent(self) -> None:
        body = m.confirm_body({"tool": "create_chart", "args": {}}, "build", None)
        self.assertIsNone(body["sessionId"])

    def test_the_arguments_of_the_pending_call_are_not_mutated(self) -> None:
        args = {"caption": "C"}
        m.confirm_body({"tool": "create_chart", "args": args}, "build", None)
        self.assertEqual(args, {"caption": "C"})

    def test_the_tool_and_args_come_from_the_trace_result(self) -> None:
        # use-copilot.ts:380-382: pending = step.result; tool = pending.tool.
        step = {"tool": "create_chart", "args": {"x": 1}, "result": {"needs_confirmation": True, "tool": "update_chart", "args": {"id": "c9", "title": "T"}}}
        self.assertEqual(m.pending_of(step), {"tool": "update_chart", "args": {"id": "c9", "title": "T"}})

    def test_a_step_without_a_result_uses_its_own_tool_and_args(self) -> None:
        step = {"tool": "create_chart", "args": {"x": 1}}
        self.assertEqual(m.pending_of(step), {"tool": "create_chart", "args": {"x": 1}})


class ConfirmRunTests(unittest.TestCase):
    def test_confirm_posts_to_the_tool_route_and_records_the_response(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("POST", "/api/ai/tool"): {"tool": "create_chart", "outcome": "executed", "result": {"created": True, "id": "c77"}}})
        step = {"tool": "create_chart", "args": {}, "result": {"needs_confirmation": True, "tool": "create_chart", "args": {"caption": "E"}}}
        response = m.send_confirm(cfg, session, step, mode="build", session_id=None)
        self.assertEqual(response["result"]["id"], "c77")
        method, url, body = session.calls[-1]
        self.assertEqual((method, url), ("POST", "http://api.test/api/ai/tool"))
        self.assertEqual(body["args"]["title"], "E")
        self.assertIs(body["args"]["confirmed"], True)


# ── cleanup ───────────────────────────────────────────────────────────────


class CleanupPlanTests(unittest.TestCase):
    def test_the_plan_deletes_newest_first_by_kind(self) -> None:
        created = [
            {"kind": "board", "id": "b1"},
            {"kind": "chart", "id": "c1"},
            {"kind": "source", "id": "s1"},
            {"kind": "field", "id": "f1"},
            {"kind": "chart", "id": "c2"},
        ]
        order = [(s["kind"], s["id"]) for s in m.cleanup_plan(created)]
        # charts, then calculated fields, then SQL sources, then boards.
        self.assertEqual(order, [("chart", "c2"), ("chart", "c1"), ("field", "f1"), ("source", "s1"), ("board", "b1")])

    def test_within_a_kind_the_last_created_is_deleted_first(self) -> None:
        created = [{"kind": "chart", "id": "c1"}, {"kind": "chart", "id": "c2"}, {"kind": "chart", "id": "c3"}]
        self.assertEqual([s["id"] for s in m.cleanup_plan(created)], ["c3", "c2", "c1"])

    def test_each_kind_uses_its_own_delete_route(self) -> None:
        self.assertEqual(m.DELETE_ROUTES["chart"], "/api/dashboard/specs")
        self.assertEqual(m.DELETE_ROUTES["field"], "/api/dashboard/calc-fields")
        self.assertEqual(m.DELETE_ROUTES["source"], "/api/dashboard/sources")
        self.assertEqual(m.DELETE_ROUTES["board"], "/api/dashboard/boards")

    def test_cleanup_deletes_what_the_run_created_and_nothing_else(self) -> None:
        cfg = make_cfg()
        session = FakeSession()
        created = [{"kind": "chart", "id": "c1"}, {"kind": "board", "id": "b1"}]
        problems = m.run_cleanup(cfg, session, created)
        self.assertEqual(problems, [])
        deleted = [(method, url) for method, url, _ in session.calls]
        self.assertEqual(
            deleted,
            [
                ("DELETE", "http://api.test/api/dashboard/specs?id=c1"),
                ("DELETE", "http://api.test/api/dashboard/boards?id=b1"),
            ],
        )

    def test_a_failed_delete_is_a_problem_not_a_crash(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("DELETE", "/api/dashboard/specs"): FakeResponse(409, {"error": "in use"})})
        problems = m.run_cleanup(cfg, session, [{"kind": "chart", "id": "c1"}])
        self.assertEqual(len(problems), 1)
        self.assertIn("chart c1", problems[0])

    def test_a_connection_error_during_delete_is_a_problem(self) -> None:
        cfg = make_cfg()

        class Broken(FakeSession):
            def delete(self, url: str, **_: Any) -> FakeResponse:
                raise m.requests.ConnectionError("down")

        problems = m.run_cleanup(cfg, Broken(), [{"kind": "chart", "id": "c1"}])
        self.assertEqual(len(problems), 1)

    def test_a_run_that_raises_still_reports_a_failed_cleanup_as_a_problem(self) -> None:
        # The turn raises (the stack is gone mid-run) and the setup object
        # cannot be deleted. Both reasons belong on the run's `problems`, so a
        # failed cleanup is never only on stderr.
        cfg = make_cfg()
        case = {
            "id": "raises_then_cleanup_fails",
            "mode": "build",
            "turns": ["Make a chart of events."],
            "setup": [
                {"name": "board", "method": "POST", "path": "/api/dashboard/boards",
                 "body": {"name": "Eval Board"}, "id_at": "id"}
            ],
        }

        class Broken(FakeSession):
            def request(self, method: str, url: str, json: Any = None, **_: Any) -> FakeResponse:
                if "/api/ai/chat" in url:
                    raise m.requests.ConnectionError("stack gone")
                if "/api/dashboard/boards" in url:
                    return FakeResponse(200, {"id": "b1"})
                return self._answer(method.upper(), url, json)

            def post(self, url: str, json: Any = None, **_: Any) -> FakeResponse:
                if "/api/ai/chat" in url:
                    raise m.requests.ConnectionError("stack gone")
                return self._answer("POST", url, json)

            def delete(self, url: str, **_: Any) -> FakeResponse:
                return FakeResponse(409, {"error": "in use"})

        result = m.run_case(cfg, Broken(), case)
        self.assertEqual(result["id"], case["id"])
        self.assertFalse(result["passed"] if "passed" in result else False)
        joined = " ".join(result["problems"])
        self.assertIn("request failed", joined)
        self.assertIn("cleanup board b", joined)

    def test_created_objects_are_collected_from_setup_trace_and_confirm(self) -> None:
        setup_results = [{"kind": "board", "id": "b1"}]
        trace = [{"tool": "create_chart", "result": {"created": True, "id": "c1"}}, {"tool": "create_chart", "result": {"error": "no"}}]
        confirms = [{"tool": "create_calculated_field", "result": confirm_reply("create_calculated_field", {"ok": True, "id": "f1"})}]
        found = m.collect_created(setup_results, [trace], confirms)
        # The confirmed calculated field is collected too; the failed chart is not.
        self.assertEqual(
            found,
            [{"kind": "board", "id": "b1"}, {"kind": "chart", "id": "c1"}, {"kind": "field", "id": "f1"}],
        )

    def test_a_failed_calc_field_create_that_echoes_an_id_is_not_collected(self) -> None:
        # S2: a failed create must not make the run delete an object it did not
        # create. The real failure is {error} (ApiRejection); a stray id on it
        # is refused, as a chart and a board are refused without `created`.
        trace = [{"tool": "create_calculated_field", "ok": False, "result": {"error": "no", "id": "f-theirs"}}]
        self.assertEqual(m.collect_created([], [trace], []), [])

    def test_a_calc_field_with_no_ok_marker_is_not_collected(self) -> None:
        trace = [{"tool": "create_calculated_field", "result": {"id": "f-x", "field": {"id": "f-x"}}}]
        self.assertEqual(m.collect_created([], [trace], []), [])

    def test_a_calc_field_confirm_that_failed_is_not_collected(self) -> None:
        confirms = [{"tool": "create_calculated_field", "result": confirm_reply("create_calculated_field", {"error": "bad", "id": "f-theirs"}, outcome="failed")}]
        self.assertEqual(m.collect_created([], [], confirms), [])

    def test_a_confirmed_chart_and_board_are_collected_from_the_real_reply_shape(self) -> None:
        # `POST /api/ai/tool` answers {tool, outcome, result}; the created
        # object's id and `created` marker are in the inner `result`.
        confirms = [
            {"tool": "create_chart", "result": confirm_reply("create_chart", {"created": True, "id": "c77"})},
            {"tool": "create_board", "result": confirm_reply("create_board", {"created": True, "id": "b88"})},
        ]
        self.assertEqual(
            m.collect_created([], [], confirms),
            [{"kind": "chart", "id": "c77"}, {"kind": "board", "id": "b88"}],
        )

    def test_a_confirmed_chart_reaches_the_state_check_and_the_cleanup_of_a_run(self) -> None:
        # The seam: a case that confirms a chart. The chart the confirm made
        # counts in `expect_state`, and the run deletes it afterwards.
        cfg = make_cfg()
        session = FakeSession({
            ("POST", "/api/ai/chat"): {
                "answer": "Drafted.",
                "toolTrace": [{"tool": "create_chart", "args": {"caption": "C"},
                               "result": {"needs_confirmation": True, "tool": "create_chart", "args": {"caption": "C"}}}],
            },
            ("POST", "/api/ai/tool"): confirm_reply("create_chart", {"created": True, "id": "c77"}),
            ("GET", "/api/dashboard/specs"): {"specs": [{"id": "c77"}]},
        })
        case = {
            "id": "confirmed_chart",
            "mode": "build",
            "confirm": True,
            "turns": ["Draw it."],
            "expect_state": [{"path": "/api/dashboard/specs", "list": "specs", "where": {}, "count": 1}],
        }
        result = m.run_case(cfg, session, case)
        self.assertEqual(result["problems"], [])
        self.assertIn(("DELETE", "http://api.test/api/dashboard/specs?id=c77", None), session.calls)

    def test_a_calc_field_created_in_a_turn_is_collected_as_a_field(self) -> None:
        # The real success: `{ok: true, field: {id}}`, with `id` lifted to the top.
        trace = [{"tool": "create_calculated_field", "ok": True, "result": {"ok": True, "id": "f9", "field": {"id": "f9"}}}]
        self.assertEqual(m.collect_created([], [trace], []), [{"kind": "field", "id": "f9"}])

    def test_a_board_created_in_a_turn_is_collected_as_a_board(self) -> None:
        trace = [{"tool": "create_board", "result": {"created": True, "id": "b5", "name": "X"}}]
        self.assertEqual(m.collect_created([], [trace], []), [{"kind": "board", "id": "b5"}])

    def test_setup_results_are_collected_with_their_kind(self) -> None:
        self.assertEqual(
            m.collect_created([{"kind": "source", "id": "s9"}], [], []),
            [{"kind": "source", "id": "s9"}],
        )


# ── setup and {name} substitution ─────────────────────────────────────────


class SetupTests(unittest.TestCase):
    def test_a_setup_call_saves_the_returned_id_under_its_name(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("POST", "/api/dashboard/boards"): {"ok": True, "board": {"id": "b-123", "name": "Test"}}})
        setup = [{"name": "b", "method": "POST", "path": "/api/dashboard/boards", "body": {"name": "T"}, "id_at": "board.id"}]
        names, created = m.run_setup(cfg, session, setup)
        self.assertEqual(names, {"b": "b-123"})
        self.assertEqual(created, [{"kind": "board", "id": "b-123"}])

    def test_the_kind_of_a_setup_object_comes_from_its_route(self) -> None:
        self.assertEqual(m.kind_of_path("/api/dashboard/specs"), "chart")
        self.assertEqual(m.kind_of_path("/api/dashboard/boards"), "board")
        self.assertEqual(m.kind_of_path("/api/dashboard/sources"), "source")
        self.assertEqual(m.kind_of_path("/api/dashboard/calc-fields"), "field")
        self.assertIsNone(m.kind_of_path("/api/other"))

    def test_a_setup_name_substitutes_into_a_turn(self) -> None:
        self.assertEqual(m.fill_names("Rename chart {c} now", {"c": "c-9"}), "Rename chart c-9 now")

    def test_an_unknown_name_is_left_alone(self) -> None:
        self.assertEqual(m.fill_names("Hi {zz}", {}), "Hi {zz}")

    def test_a_setup_body_can_use_a_name_saved_by_an_earlier_setup_call(self) -> None:
        # A chart set up on a board sends the board's id, not the text "{board}".
        cfg = make_cfg()
        session = FakeSession({
            ("POST", "/api/dashboard/boards"): {"board": {"id": "b-7"}},
            ("POST", "/api/dashboard/specs"): {"chart": {"id": "c-1"}},
        })
        setup = [
            {"name": "board", "method": "POST", "path": "/api/dashboard/boards", "body": {"name": "T"}, "id_at": "board.id"},
            {"name": "chart", "method": "POST", "path": "/api/dashboard/specs", "body": {"board": "{board}"}, "id_at": "chart.id"},
        ]
        m.run_setup(cfg, session, setup)
        _, _, chart_body = session.calls[-1]
        self.assertEqual(chart_body, {"board": "b-7"})

    def test_an_expect_state_where_can_use_a_saved_name(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("GET", "/api/dashboard/specs"): {"charts": [{"id": "c-1", "title": "T"}]}})
        check = {"path": "/api/dashboard/specs", "list": "charts", "where": {"id": "{chart}"}, "count": 1}
        names = {"chart": "c-1"}
        filled = m.fill_check(check, names)
        self.assertEqual(filled["where"], {"id": "c-1"})
        self.assertEqual(m.expect_state_problems(cfg, session, filled, own_ids={"c-1"}), [])

    def test_a_failed_setup_call_raises_so_the_run_is_a_setup_error(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("POST", "/api/dashboard/specs"): FakeResponse(400, {"error": "bad"})})
        setup = [{"name": "c", "method": "POST", "path": "/api/dashboard/specs", "body": {}, "id_at": "chart.id"}]
        with self.assertRaises(m.SetupError):
            m.run_setup(cfg, session, setup)


# ── expect_state ──────────────────────────────────────────────────────────


class ExpectStateTests(unittest.TestCase):
    def test_a_state_check_matches_only_objects_this_run_created(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("GET", "/api/dashboard/specs"): {"charts": [
            {"id": "mine", "title": "Mine", "kind": "bar"},
            {"id": "theirs", "title": "Other", "kind": "bar"},
        ]}})
        check = {"path": "/api/dashboard/specs", "list": "charts", "where": {"kind": "bar"}, "count": 1}
        problems = m.expect_state_problems(cfg, session, check, own_ids={"mine"})
        self.assertEqual(problems, [])

    def test_a_state_check_fails_when_only_someone_elses_object_matches(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("GET", "/api/dashboard/specs"): {"charts": [{"id": "theirs", "kind": "bar"}]}})
        check = {"path": "/api/dashboard/specs", "list": "charts", "where": {"kind": "bar"}, "count": 1}
        problems = m.expect_state_problems(cfg, session, check, own_ids=set())
        self.assertTrue(problems)

    def test_scope_all_matches_every_object(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("GET", "/api/dashboard/specs"): {"charts": [{"id": "theirs", "kind": "bar"}]}})
        check = {"path": "/api/dashboard/specs", "list": "charts", "where": {"kind": "bar"}, "count": 1, "scope": "all"}
        self.assertEqual(m.expect_state_problems(cfg, session, check, own_ids=set()), [])

    def test_a_state_check_can_ask_for_at_least_a_number(self) -> None:
        # "At least two charts": three matching objects pass a `min` of two.
        cfg = make_cfg()
        session = FakeSession({("GET", "/api/dashboard/specs"): {"charts": [
            {"id": "a", "mart": "mart_event"}, {"id": "b", "mart": "mart_event"}, {"id": "c", "mart": "mart_event"}]}})
        check = {"path": "/api/dashboard/specs", "list": "charts", "where": {"mart": "mart_event"}, "min": 2}
        self.assertEqual(m.expect_state_problems(cfg, session, check, own_ids={"a", "b", "c"}), [])

    def test_a_state_check_with_min_fails_below_it(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("GET", "/api/dashboard/specs"): {"charts": [{"id": "a", "mart": "mart_event"}]}})
        check = {"path": "/api/dashboard/specs", "list": "charts", "where": {"mart": "mart_event"}, "min": 2}
        self.assertTrue(m.expect_state_problems(cfg, session, check, own_ids={"a"}))

    def test_a_state_check_can_require_an_absent_object(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("GET", "/api/dashboard/specs"): {"charts": []}})
        check = {"path": "/api/dashboard/specs", "list": "charts", "where": {"title": "X"}, "count": 0}
        self.assertEqual(m.expect_state_problems(cfg, session, check, own_ids=set()), [])

    def test_a_state_check_can_match_on_a_nested_board_field(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("GET", "/api/dashboard/specs"): {"charts": [
            {"id": "c1", "title": "T", "board": "b1"}]}})
        check = {"path": "/api/dashboard/specs", "list": "charts", "where": {"board": "b1"}, "count": 1}
        self.assertEqual(m.expect_state_problems(cfg, session, check, own_ids={"c1"}), [])


# ── `where` keys: flat, dotted into `def`, and list members (P1, P2) ──────


# The shape `GET /api/dashboard/specs` returns for one chart: `render_stored_chart`
# (dashboard.rs) puts the stored spec at the top, and `def` (a `ChartInput`)
# carries `grain`, `tableMode`, `measures`, `dimension`, `compare`. The three
# `def.*` keys below resolve against this, never against the top level.
REAL_SPEC_ITEM = {
    "id": "c-real",
    "title": "Line per month",
    "kind": "line",
    "mart": "mart_wisman",
    "x": "bulan",
    "y": "jumlah",
    "board": "b-1",
    "hasYear": False,
    "createdBy": "ai",
    "createdAt": "2026-10-15 00:00:00",
    "def": {
        "title": "Line per month",
        "mart": "mart_wisman",
        "kind": "line",
        "dimension": "bulan",
        "measures": ["jumlah"],
        "grain": "month",
    },
}


class WhereMatchTests(unittest.TestCase):
    def test_a_flat_key_still_matches_a_top_level_value(self) -> None:
        self.assertTrue(m.where_matches({"kind": "line"}, {"kind": "line"}))
        self.assertFalse(m.where_matches({"kind": "bar"}, {"kind": "line"}))

    def test_a_dotted_key_matches_a_nested_value(self) -> None:
        self.assertTrue(m.where_matches(REAL_SPEC_ITEM, {"def.grain": "month"}))
        self.assertFalse(m.where_matches(REAL_SPEC_ITEM, {"def.grain": "quarter"}))

    def test_a_dotted_key_that_is_absent_does_not_match(self) -> None:
        # No `tableMode` in this chart's def: a dotted check must not match
        # against None, nor against a missing parent.
        self.assertFalse(m.where_matches(REAL_SPEC_ITEM, {"def.tableMode": "rows"}))
        self.assertFalse(m.where_matches({"id": "x"}, {"def.grain": "month"}))
        # A `where` value of null must not turn "absent" into "matches either":
        # that would let a state check pass on a chart that lacks the field.
        self.assertFalse(m.where_matches(REAL_SPEC_ITEM, {"def.tableMode": None}))
        self.assertFalse(m.where_matches({"id": "x"}, {"def.grain": None}))

    def test_a_flat_grain_never_matches_because_grain_lives_under_def(self) -> None:
        # The bug this guards: the old flat matcher could never see `grain`.
        self.assertFalse(m.where_matches(REAL_SPEC_ITEM, {"grain": "month"}))

    def test_a_where_value_matches_a_member_of_a_list(self) -> None:
        # `measures` is a list in def; a where value names one member.
        self.assertTrue(m.where_matches(REAL_SPEC_ITEM, {"def.measures": "jumlah"}))
        self.assertFalse(m.where_matches(REAL_SPEC_ITEM, {"def.measures": "other"}))

    def test_a_state_check_with_a_dotted_key_counts_matching_charts(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("GET", "/api/dashboard/specs"): {"charts": [REAL_SPEC_ITEM]}})
        check = {"path": "/api/dashboard/specs", "list": "charts",
                 "where": {"kind": "line", "def.grain": "month"}, "count": 1}
        self.assertEqual(m.expect_state_problems(cfg, session, check, own_ids={"c-real"}), [])

    def test_the_three_grain_and_table_keys_resolve_against_a_real_spec_item(self) -> None:
        # The three cases that could never pass (P1) must now resolve against
        # the shape the route returns, so a perfect assistant can match them.
        cases = {c["id"]: c for c in json.loads((HERE / "dashboard_cases.json").read_text())}
        self.assertEqual(
            cases["chart_line_monthly"]["expect_state"][0]["where"], {"mart": "mart_wisman", "kind": "line", "def.grain": "month"}
        )
        self.assertEqual(
            cases["chart_quarterly_id"]["expect_state"][0]["where"], {"kind": "line", "def.grain": "quarter"}
        )
        table = {"id": "t", "kind": "table", "mart": "mart_event", "def": {"tableMode": "rows", "kind": "table"}}
        self.assertEqual(
            cases["chart_rows_table"]["expect_state"][0]["where"],
            {"mart": "mart_event", "kind": "table", "def.tableMode": "rows"},
        )
        self.assertTrue(m.where_matches(table, cases["chart_rows_table"]["expect_state"][0]["where"]))


# ── expect_approval, expect_no_tool, forbid_unless_tool ───────────────────


class TraceChecksTests(unittest.TestCase):
    def test_expect_approval_needs_a_needs_approval_result_for_that_tool(self) -> None:
        trace = [{"tool": "delete_chart", "ok": False, "result": {"needs_approval": True, "tool": "delete_chart"}}]
        self.assertEqual(m.approval_problems(trace, "delete_chart"), [])

    def test_expect_approval_is_missing_when_the_tool_ran_or_never_asked(self) -> None:
        self.assertTrue(m.approval_problems([], "delete_chart"))
        self.assertTrue(m.approval_problems([{"tool": "delete_chart", "ok": True, "result": {"deleted": True}}], "delete_chart"))

    def test_expect_no_tool_fails_when_the_tool_succeeded(self) -> None:
        trace = [{"tool": "create_chart", "ok": True, "result": {"created": True}}]
        self.assertTrue(m.no_tool_problems(trace, ["create_chart"]))

    def test_expect_no_tool_passes_when_the_tool_was_refused_or_absent(self) -> None:
        trace = [{"tool": "create_chart", "ok": False, "result": {"error": "no"}}]
        self.assertEqual(m.no_tool_problems(trace, ["create_chart"]), [])
        self.assertEqual(m.no_tool_problems([], ["create_chart"]), [])

    def test_forbid_unless_tool_is_a_problem_when_the_pattern_matches_and_the_key_is_absent(self) -> None:
        answer = "I have queued this in Approvals for you."
        trace = [{"tool": "delete_chart", "ok": True, "result": {"deleted": True}}]
        problems = m.forbid_unless_problems(answer, trace, pattern="Approvals", result_key="needs_approval")
        self.assertTrue(problems)

    def test_forbid_unless_tool_passes_when_the_key_is_in_the_trace(self) -> None:
        answer = "It is waiting in Approvals."
        trace = [{"tool": "delete_chart", "ok": False, "result": {"needs_approval": True}}]
        self.assertEqual(m.forbid_unless_problems(answer, trace, pattern="Approvals", result_key="needs_approval"), [])

    def test_forbid_unless_tool_passes_when_the_pattern_is_not_in_the_answer(self) -> None:
        self.assertEqual(m.forbid_unless_problems("No such chart.", [], pattern="Approvals", result_key="needs_approval"), [])


# ── per-turn modes and page context ───────────────────────────────────────


class TurnOptionsTests(unittest.TestCase):
    def test_without_modes_every_turn_uses_the_case_mode(self) -> None:
        self.assertEqual(m.turn_modes({"mode": "build", "turns": ["a", "b"]}), ["build", "build"])

    def test_a_case_without_mode_defaults_to_ask(self) -> None:
        self.assertEqual(m.turn_modes({"turns": ["a"]}), ["ask"])

    def test_modes_gives_one_mode_per_turn(self) -> None:
        self.assertEqual(m.turn_modes({"modes": ["ask", "build"], "turns": ["a", "b"]}), ["ask", "build"])

    def test_modes_must_match_the_turns(self) -> None:
        with self.assertRaises(m.CaseError):
            m.turn_modes({"modes": ["ask"], "turns": ["a", "b"]})

    def test_the_context_is_sent_with_every_turn(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("POST", "/api/ai/chat"): {"answer": "ok", "toolTrace": []}})
        m.ask(cfg, session, "ask", [{"role": "user", "content": "hi"}], context="On the page: two tiles")
        _, _, body = session.calls[-1]
        self.assertEqual(body["context"], "On the page: two tiles")

    def test_a_context_can_name_the_objects_the_setup_made(self) -> None:
        # page_context_tiles: the context names the two set-up tiles by their ids.
        cfg = make_cfg()
        session = FakeSession({
            ("POST", "/api/dashboard/specs"): {"chart": {"id": "t-1"}},
            ("POST", "/api/ai/chat"): {"answer": "ok", "toolTrace": []},
        })
        case = {
            "id": "ctx",
            "mode": "ask",
            "context": "The page shows tiles {tile_a}.",
            "setup": [{"name": "tile_a", "method": "POST", "path": "/api/dashboard/specs", "body": {}, "id_at": "chart.id"}],
            "turns": ["What is on this dashboard?"],
        }
        m.run_case(cfg, session, case)
        chat = [body for method, url, body in session.calls if url.endswith("/api/ai/chat")][0]
        self.assertEqual(chat["context"], "The page shows tiles t-1.")

    def test_no_context_key_when_the_case_has_none(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("POST", "/api/ai/chat"): {"answer": "ok", "toolTrace": []}})
        m.ask(cfg, session, "ask", [{"role": "user", "content": "hi"}], context=None)
        _, _, body = session.calls[-1]
        self.assertNotIn("context", body)


class RunCaseTests(unittest.TestCase):
    def test_a_turn_in_build_mode_is_sent_as_build(self) -> None:
        cfg = make_cfg()
        session = FakeSession({("POST", "/api/ai/chat"): {"answer": "ok", "toolTrace": []}})
        case = {"id": "x", "modes": ["ask", "build"], "turns": ["one", "two"]}
        m.run_case(cfg, session, case)
        modes = [body["mode"] for method, url, body in session.calls if url.endswith("/api/ai/chat")]
        self.assertEqual(modes, ["ask", "build"])


    def test_a_run_fills_the_setup_names_in_the_expected_tool_args(self) -> None:
        cfg = make_cfg()
        session = FakeSession({
            ("POST", "/api/dashboard/boards"): {"board": {"id": "b-9"}},
            ("POST", "/api/ai/chat"): {
                "answer": "Renamed.",
                "toolTrace": [{"tool": "update_board", "args": {"id": "b-9", "name": "N"}, "result": {"ok": True}}],
            },
        })
        case = {
            "id": "rename",
            "mode": "build",
            "setup": [{"name": "board", "method": "POST", "path": "/api/dashboard/boards", "body": {}, "id_at": "board.id"}],
            "turns": ["Rename it."],
            "expect_tool": {"name": "update_board", "args_contains": {"id": "{board}"}},
        }
        result = m.run_case(cfg, session, case)
        self.assertEqual(result["missing"], [])

    def run_confirming_case(self, reply: Any) -> dict[str, Any]:
        session = FakeSession({
            ("POST", "/api/ai/chat"): {
                "answer": "Drafted.",
                "toolTrace": [{"tool": "create_chart", "args": {"caption": "C"},
                               "result": {"needs_confirmation": True, "tool": "create_chart", "args": {"caption": "C"}}}],
            },
            ("POST", "/api/ai/tool"): reply,
        })
        case = {"id": "confirming", "mode": "build", "confirm": True, "turns": ["Draw it."]}
        return m.run_case(make_cfg(), session, case)

    def test_a_confirm_that_came_back_executed_adds_no_problem(self) -> None:
        result = self.run_confirming_case(confirm_reply("create_chart", {"created": True, "id": "c1"}))
        self.assertEqual(result["problems"], [])

    def test_a_confirm_with_http_500_is_a_problem_naming_the_status(self) -> None:
        reply = FakeResponse(500, {"error": "boom"})
        result = self.run_confirming_case(reply)
        problem = [p for p in result["problems"] if p.startswith("confirm create_chart")]
        self.assertEqual(len(problem), 1)
        self.assertIn("HTTP 500", problem[0])

    def test_a_confirm_with_an_error_status_is_a_problem_even_when_the_body_says_executed(self) -> None:
        reply = FakeResponse(502, confirm_reply("create_chart", {"created": True, "id": "c1"}))
        result = self.run_confirming_case(reply)
        problem = [p for p in result["problems"] if p.startswith("confirm create_chart")]
        self.assertEqual(len(problem), 1)
        self.assertIn("HTTP 502", problem[0])

    def test_a_confirm_with_an_outcome_other_than_executed_is_a_problem_naming_it(self) -> None:
        reply = confirm_reply("create_chart", {"error": "bad spec"}, outcome="failed")
        result = self.run_confirming_case(reply)
        problem = [p for p in result["problems"] if p.startswith("confirm create_chart")]
        self.assertEqual(len(problem), 1)
        self.assertIn("outcome failed", problem[0])
        self.assertIn("HTTP 200", problem[0])

    def test_a_confirm_reply_with_an_error_is_a_problem(self) -> None:
        reply = {"tool": "create_chart", "outcome": "executed", "error": "late failure", "result": {}}
        result = self.run_confirming_case(reply)
        self.assertTrue([p for p in result["problems"] if p.startswith("confirm create_chart")])


# ── the existing 28 cases score as before ─────────────────────────────────


class ExistingCasesTests(unittest.TestCase):
    """`--rescore` of the saved run must give the numbers recorded before the
    dashboard features (`fixtures/existing_cases_expected.json`)."""

    def test_existing_cases_rescore_to_the_numbers_recorded_before(self) -> None:
        cfg = make_cfg()
        cases = {c["id"]: c for c in json.loads((HERE / "cases.json").read_text())}
        saved = json.loads((FIXTURES / "existing_cases_saved.json").read_text())
        expected = json.loads((FIXTURES / "existing_cases_expected.json").read_text())

        # Ground truth is stubbed: the fixture was scored with these literals.
        def stub(_cfg: Any, _session: Any, fact: dict[str, Any]) -> list[Any]:
            if "sql" in fact:
                return [1234.0] if fact.get("kind") == "number" else ["Malaysia"]
            if "api" in fact:
                return ["Main"]
            return list(fact.get("values", []))

        with mock.patch.object(m, "resolve_fact", side_effect=stub):
            got = []
            for result in saved["results"]:
                scored = m.score_case(cfg, None, cases[result["id"]], m.bodies_from_saved(result))
                got.append({k: scored[k] for k in ("id", "completeness", "accuracy", "experience", "overall", "problems", "missing")})

        self.assertEqual(got, expected)
        self.assertEqual(len(got), 28)

    def test_run_fields_are_added_and_the_scored_keys_are_kept(self) -> None:
        # The plain run keeps today's keys; `runs` and `passed` are additive.
        scored = {"id": "x", "overall": 1.0, "problems": [], "missing": [], "completeness": 1.0}
        out = m.with_run_fields(scored, runs=[scored], need=1)
        self.assertEqual(out["id"], "x")
        self.assertEqual(out["overall"], 1.0)
        self.assertEqual(out["completeness"], 1.0)
        self.assertEqual(len(out["runs"]), 1)
        self.assertTrue(out["passed"])


class ScoreCaseTests(unittest.TestCase):
    """`score_case` on one scripted turn: no facts, so the tool check, the
    refusal cues and the confirm problems are the only things judged."""

    @staticmethod
    def body(answer: str, trace: list[dict[str, Any]] | None = None) -> dict[str, Any]:
        return {"answer": answer, "toolTrace": trace or [], "_status": 200, "_elapsed_s": 1}

    def test_a_name_in_args_contains_is_filled_before_it_is_compared(self) -> None:
        case = {"id": "rename", "expect_tool": {"name": "update_board", "args_contains": {"id": "{board}"}}}
        trace = [{"tool": "update_board", "args": {"id": "b-42", "name": "New"}}]
        scored = m.score_case(make_cfg(), None, case, [self.body("Done.", trace)], names={"board": "b-42"})
        self.assertEqual(scored["missing"], [])

    def test_a_name_that_is_not_the_called_id_is_still_missing(self) -> None:
        case = {"id": "rename", "expect_tool": {"name": "update_board", "args_contains": {"id": "{board}"}}}
        trace = [{"tool": "update_board", "args": {"id": "b-other"}}]
        scored = m.score_case(make_cfg(), None, case, [self.body("Done.", trace)], names={"board": "b-42"})
        self.assertEqual(len(scored["missing"]), 1)

    def test_names_are_optional_for_a_rescore(self) -> None:
        # `--rescore` has no setup names: an `args_contains` without a
        # placeholder scores as before.
        case = {"id": "t", "expect_tool": {"name": "create_chart", "args_contains": {"kind": "bar"}}}
        trace = [{"tool": "create_chart", "args": {"kind": "bar"}}]
        scored = m.score_case(make_cfg(), None, case, [self.body("Done.", trace)])
        self.assertEqual(scored["missing"], [])

    def refused(self, answer: str) -> bool:
        case = {"id": "r", "expect_refusal": True}
        scored = m.score_case(make_cfg(), None, case, [self.body(answer)])
        return "answered a question the data cannot answer" not in scored["problems"]

    def test_a_plain_cannot_is_a_refusal(self) -> None:
        self.assertTrue(self.refused("I can't create charts here."))
        self.assertTrue(self.refused("Charts can't read a Silver table directly."))
        self.assertTrue(self.refused("I cannot do that in ask mode."))
        self.assertTrue(self.refused("I can not draw that."))
        self.assertTrue(self.refused("I am not able to change the board."))

    def test_an_indonesian_cannot_is_a_refusal(self) -> None:
        self.assertTrue(self.refused("Saya tidak bisa membuat grafik di sini."))
        self.assertTrue(self.refused("Itu tidak dapat dilakukan."))

    def test_a_curly_apostrophe_still_counts(self) -> None:
        self.assertTrue(self.refused("I can\u2019t create charts here."))
        self.assertTrue(self.refused("That isn\u2019t available."))

    def test_an_answer_with_no_cue_is_not_a_refusal(self) -> None:
        self.assertFalse(self.refused("Here is the chart you asked for."))


# ── exit codes and the output file ────────────────────────────────────────


class ExitCodeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.cases_path = HERE / "cases.json"

    def test_exit_zero_on_pass(self) -> None:
        self.assertEqual(m.exit_code(setup_error=False, verdict={"pass": True}), 0)

    def test_exit_one_on_fail(self) -> None:
        self.assertEqual(m.exit_code(setup_error=False, verdict={"pass": False}), 1)

    def test_exit_two_on_a_setup_error_whatever_the_verdict(self) -> None:
        self.assertEqual(m.exit_code(setup_error=True, verdict=None), 2)

    def test_a_refused_login_is_a_setup_error(self) -> None:
        def login_refused(*_a: Any, **_k: Any) -> FakeResponse:
            return FakeResponse(401, {"error": "no"})

        argv = ["ai_eval.py", "--cases", str(HERE / "cases.json"), "--out", "/tmp/opencode/ai15/none.json"]
        env = {"AI_EVAL_EMAIL": "e@example.com", "AI_EVAL_PASSWORD": "p", "AI_EVAL_API_URL": "http://api.test"}
        with mock.patch.dict("os.environ", env, clear=False), mock.patch.object(sys, "argv", argv), \
                mock.patch.object(m.requests.Session, "post", login_refused):
            self.assertEqual(m.main(), 2)

    def test_an_unreachable_stack_is_a_setup_error(self) -> None:
        def down(*_a: Any, **_k: Any) -> FakeResponse:
            raise m.requests.ConnectionError("refused")

        argv = ["ai_eval.py", "--cases", str(HERE / "cases.json")]
        env = {"AI_EVAL_EMAIL": "e@example.com", "AI_EVAL_PASSWORD": "p", "AI_EVAL_API_URL": "http://api.test"}
        with mock.patch.dict("os.environ", env, clear=False), mock.patch.object(sys, "argv", argv), \
                mock.patch.object(m.requests.Session, "post", down):
            self.assertEqual(m.main(), 2)

    def run_main(self, argv_extra: list[str], run_case_results: list[dict[str, Any]]) -> tuple[int, dict[str, Any]]:
        """Drive `main` with a fake login and a scripted `run_case`; returns the
        exit code and the written results document."""
        out = Path(tempfile.mkdtemp()) / "out.json"
        argv = ["ai_eval.py", "--cases", str(self.cases_path), "--out", str(out), *argv_extra]
        env = {"AI_EVAL_EMAIL": "e@example.com", "AI_EVAL_PASSWORD": "p", "AI_EVAL_API_URL": "http://api.test"}
        script = iter(run_case_results)

        def fake_run_case(_cfg: Any, _session: Any, _case: Any) -> dict[str, Any]:
            return next(script)

        def login_ok(_self: Any, url: str, **_k: Any) -> FakeResponse:
            return FakeResponse(200, {"ok": True})

        with mock.patch.dict("os.environ", env, clear=False), mock.patch.object(sys, "argv", argv), \
                mock.patch.object(m.requests.Session, "post", login_ok), \
                mock.patch.object(m, "run_case", side_effect=fake_run_case), \
                mock.patch("sys.stdout"):
            code = m.main()
        return code, json.loads(out.read_text())

    def scored(self, passed_problem: bool) -> dict[str, Any]:
        base = {"id": "one", "completeness": 1.0, "accuracy": 1.0, "experience": 1.0, "overall": 1.0,
                "elapsed_s": 1.0, "tools": [], "problems": [], "missing": [], "answer": "", "trace": []}
        if passed_problem:
            base["problems"] = ["approval: delete_chart did not ask for approval"]
        return base

    def test_main_two_of_three_runs_pass_the_case_and_the_set_exits_zero(self) -> None:
        self.cases_path = self.write_cases([{"id": "one", "turns": ["t"]}])
        runs = [self.scored(False), self.scored(True), self.scored(False)]
        code, doc = self.run_main(["--repeat", "3", "--need", "2"], runs)
        self.assertEqual(code, 0)
        self.assertTrue(doc["results"][0]["passed"])
        self.assertEqual(len(doc["results"][0]["runs"]), 3)
        self.assertEqual(doc["verdict"]["pass"], True)

    def test_main_one_of_three_runs_pass_the_case_and_the_set_exits_one(self) -> None:
        self.cases_path = self.write_cases([{"id": "one", "turns": ["t"]}])
        runs = [self.scored(False), self.scored(True), self.scored(True)]
        code, doc = self.run_main(["--repeat", "3", "--need", "2"], runs)
        self.assertEqual(code, 1)
        self.assertFalse(doc["results"][0]["passed"])
        self.assertFalse(doc["verdict"]["pass"])

    def test_main_a_baseline_regression_exits_one_even_above_the_bar(self) -> None:
        cases = [{"id": f"c{i}", "turns": ["t"]} for i in range(10)]
        self.cases_path = self.write_cases(cases)
        baseline = Path(tempfile.mkdtemp()) / "base.json"
        baseline.write_text(json.dumps({"results": [{"id": "c0", "passed": True}]}))
        runs = [self.scored(False) | {"id": f"c{i}"} for i in range(10)]
        runs[0] = self.scored(True) | {"id": "c0"}
        code, doc = self.run_main(["--baseline", str(baseline)], runs)
        self.assertEqual(code, 1)
        self.assertEqual(doc["verdict"]["regressed"], ["c0"])

    def test_main_a_new_failing_case_is_not_a_regression_and_still_passes_the_set(self) -> None:
        cases = [{"id": f"c{i}", "turns": ["t"]} for i in range(10)]
        self.cases_path = self.write_cases(cases)
        baseline = Path(tempfile.mkdtemp()) / "base.json"
        baseline.write_text(json.dumps({"results": [{"id": f"c{i}", "passed": True} for i in range(9)]}))
        runs = [self.scored(False) | {"id": f"c{i}"} for i in range(10)]
        runs[9] = self.scored(True) | {"id": "c9"}
        code, doc = self.run_main(["--baseline", str(baseline)], runs)
        self.assertEqual(code, 0)
        self.assertEqual(doc["verdict"]["regressed"], [])

    def write_cases(self, cases: list[dict[str, Any]]) -> Path:
        path = Path(tempfile.mkdtemp()) / "cases.json"
        path.write_text(json.dumps(cases))
        return path

    def test_the_output_keeps_the_old_keys_and_adds_verdict_and_runs(self) -> None:
        cases = [{"id": "a", "runs": [{"problems": [], "missing": [], "overall": 1.0}], "passed": True,
                  "completeness": 1.0, "accuracy": 1.0, "experience": 1.0, "overall": 1.0}]
        doc = m.result_document("lbl", cases, verdict={"pass": True, "passed": 1, "total": 1})
        self.assertEqual(set(doc), {"label", "summary", "results", "verdict"})
        self.assertIn("runs", doc["results"][0])
        self.assertIn("passed", doc["results"][0])


# ── the dashboard case file ───────────────────────────────────────────────


class OutPathTests(unittest.TestCase):
    """S3: results never land in the repository. A relative `--out` is taken
    under the temp directory, the same place the default goes."""

    def test_a_relative_out_is_placed_in_the_temp_directory_not_the_cwd(self) -> None:
        out = m.resolve_out("results.json", label="x")
        self.assertEqual(out.parent, Path(tempfile.gettempdir()))
        self.assertEqual(out.name, "results.json")

    def test_a_relative_out_with_folders_stays_under_the_temp_directory(self) -> None:
        out = m.resolve_out("sub/results.json", label="x")
        self.assertTrue(str(out).startswith(tempfile.gettempdir()))

    def test_an_absolute_out_is_kept_as_given(self) -> None:
        target = Path(tempfile.gettempdir()) / "ai-eval-keep.json"
        self.assertEqual(m.resolve_out(str(target), label="x"), target)

    def test_the_default_is_a_timestamped_file_in_the_temp_directory(self) -> None:
        out = m.resolve_out("", label="dashboard")
        self.assertEqual(out.parent, Path(tempfile.gettempdir()))
        self.assertTrue(out.name.startswith("ai-eval-dashboard-"))


class DashboardRowsTests(unittest.TestCase):
    """P2: each strengthened row asserts what its plan row says, against the
    real field names (`ChartInput`: `compare`, `rows`, `columns`, `tableMode`,
    `measures`; `kind: pivot`)."""

    def cases(self) -> dict[str, dict[str, Any]]:
        return {c["id"]: c for c in json.loads((HERE / "dashboard_cases.json").read_text())}

    def test_kpi_compare_asserts_the_compare_is_set(self) -> None:
        # row 11: "KPI with `compare` set" -> def.compare.kind matches.
        where = self.cases()["chart_kpi_compare"]["expect_state"][0]["where"]
        self.assertEqual(where.get("def.compare.kind"), "previous")
        kpi = {"id": "k", "kind": "kpi", "mart": "mart_wisman", "def": {"kind": "kpi", "compare": {"kind": "previous", "period": "month"}}}
        self.assertTrue(m.where_matches(kpi, where))
        plain = {"id": "k", "kind": "kpi", "mart": "mart_wisman", "def": {"kind": "kpi"}}
        self.assertFalse(m.where_matches(plain, where))

    def test_pivot_asserts_its_kind_and_its_rows_and_columns(self) -> None:
        # row 13: "Pivot with `rows` and `columns`" -> kind pivot, rows and columns.
        where = self.cases()["chart_pivot"]["expect_state"][0]["where"]
        self.assertEqual(where["kind"], "pivot")
        self.assertEqual(where.get("def.rows"), "tahun")
        self.assertEqual(where.get("def.columns"), "bulan_no")
        pivot = {"id": "p", "kind": "pivot", "mart": "mart_wisman",
                 "def": {"kind": "pivot", "rows": ["tahun"], "columns": ["bulan_no"]}}
        self.assertTrue(m.where_matches(pivot, where))
        grouped = {"id": "g", "kind": "table", "mart": "mart_wisman", "def": {"kind": "table"}}
        self.assertFalse(m.where_matches(grouped, where))

    def test_chart_on_new_board_links_the_chart_to_the_board_it_created(self) -> None:
        # row 14: the chart's `board` is the id of the board this run created.
        case = self.cases()["chart_on_new_board"]
        chart = [c for c in case["expect_state"] if c["path"] == "/api/dashboard/specs"]
        self.assertEqual(chart[0]["where"].get("board"), "{created:board}")

    def test_a_where_value_naming_a_created_board_is_filled_from_the_trace(self) -> None:
        # The board is made by the assistant's create_board call; its id is
        # not known when the case is written. `{created:board}` names it.
        trace = [{"tool": "create_board", "ok": True, "result": {"created": True, "id": "b-new", "name": "X"}}]
        names: dict[str, str] = {}
        created = m.collect_created([], [trace], [])
        filled = m.fill_created({"where": {"board": "{created:board}"}}, created)
        self.assertEqual(filled["where"], {"board": "b-new"})

    def test_a_created_placeholder_with_no_such_object_is_left_unfilled(self) -> None:
        # No board was created: the placeholder stays, so nothing matches it.
        filled = m.fill_created({"where": {"board": "{created:board}"}}, [])
        self.assertEqual(filled["where"], {"board": "{created:board}"})

    def test_calc_create_asserts_the_formula_has_no_sql(self) -> None:
        # row 17: the formula, not only the answer, must not contain SQL.
        case = self.cases()["calc_create"]
        self.assertIn("formula_forbid", case)
        self.assertIn("SELECT", case["formula_forbid"])

    def test_a_formula_containing_sql_is_a_problem(self) -> None:
        # The model called the tool with `confirmed` itself: the result is in
        # the trace.
        trace = [{"tool": "create_calculated_field", "ok": True,
                  "result": {"id": "f1", "field": {"formula": "SELECT 1"}}}]
        self.assertTrue(m.formula_problems(trace, [], ["SELECT"]))
        # Whatever case the model writes it in: `select 1` is the same mistake.
        lower = [{"tool": "create_calculated_field", "ok": True,
                  "result": {"id": "f1", "field": {"formula": "select 1"}}}]
        self.assertTrue(m.formula_problems(lower, [], ["SELECT"]))

    def test_a_clean_formula_is_not_a_problem(self) -> None:
        trace = [{"tool": "create_calculated_field", "ok": True,
                  "result": {"id": "f1", "field": {"formula": "[jumlah_event] / 2"}}}]
        self.assertEqual(m.formula_problems(trace, [], ["SELECT"]), [])

    def test_a_formula_confirmed_by_the_person_is_still_checked(self) -> None:
        # The flow every writing case in the set runs: the trace step is only
        # the pending confirmation, and the executed result arrives through the
        # confirm. Reading the trace alone would find no formula here and let
        # the case pass unchecked.
        trace = [{"tool": "create_calculated_field",
                  "result": {"needs_confirmation": True, "tool": "create_calculated_field",
                             "args": {"formula": "SELECT 1"}}}]
        confirms = [{"tool": "create_calculated_field",
                     "result": confirm_reply("create_calculated_field", {"ok": True, "field": {"formula": "SELECT 1"}})}]
        self.assertTrue(m.formula_problems(trace, confirms, ["SELECT"]))

    def test_a_clean_confirmed_formula_is_not_a_problem(self) -> None:
        trace = [{"tool": "create_calculated_field",
                  "result": {"needs_confirmation": True, "tool": "create_calculated_field",
                             "args": {"formula": "[jumlah_event] / 2"}}}]
        confirms = [{"tool": "create_calculated_field",
                     "result": confirm_reply("create_calculated_field", {"ok": True, "field": {"formula": "[jumlah_event] / 2"}})}]
        self.assertEqual(m.formula_problems(trace, confirms, ["SELECT"]), [])

    def test_a_pending_confirmation_alone_is_not_a_created_formula(self) -> None:
        # Only a pending step, never confirmed: nothing was created, so there
        # is no formula to judge and no problem to report.
        trace = [{"tool": "create_calculated_field",
                  "result": {"needs_confirmation": True, "tool": "create_calculated_field",
                             "args": {"formula": "SELECT 1"}}}]
        self.assertEqual(m.formula_problems(trace, [], ["SELECT"]), [])

    def test_the_calc_create_case_confirms_so_its_formula_is_checked_in_confirms(self) -> None:
        # Row 17 is only enforced because the case confirms; if it stopped, the
        # shape of the result the checker reads would change with it.
        case = self.cases()["calc_create"]
        self.assertTrue(case.get("confirm"))
        self.assertIn("SELECT", case["formula_forbid"])

    def test_calc_validate_error_names_the_offending_token(self) -> None:
        # row 18: "the answer names the mistake". The formula's broken token is
        # the `?` (lex.rs: "unexpected character '?'"), and a text fact must
        # have the answer say it.
        case = self.cases()["calc_validate_error"]
        facts = [f for f in case.get("facts", []) if f.get("kind") == "text"]
        self.assertTrue(facts, "a text fact must name the mistake")
        self.assertIn("'?'", facts[0]["values"])

    def test_calc_then_chart_asserts_the_chart_uses_the_field(self) -> None:
        # row 19: a chart uses the calculated field by name (def.measures lists it).
        case = self.cases()["calc_then_chart"]
        chart_check = [c for c in case["expect_state"] if c["path"] == "/api/dashboard/specs"][0]
        self.assertEqual(chart_check["where"].get("def.measures"), "per_event")

    def test_silver_refused_asserts_the_answer_offers_a_mart(self) -> None:
        # row 25: "the answer offers a mart" -> the answer names a serving mart.
        case = self.cases()["silver_refused"]
        facts = [f for f in case.get("facts", []) if f.get("kind") == "text"]
        self.assertTrue(facts, "a text fact must name a mart the answer offers")
        self.assertTrue(any(v.startswith("mart_") for v in facts[0]["values"]))

    def test_kpi_number_backed_has_no_dead_forbid(self) -> None:
        # P4: forbid scans plain text, which strips the attribute, so the
        # `data-unverified` forbid can never match. It is removed.
        self.assertNotIn("forbid", self.cases()["kpi_number_backed"])


class DashboardCasesTests(unittest.TestCase):
    def test_the_dashboard_set_has_thirty_unique_cases(self) -> None:
        cases = json.loads((HERE / "dashboard_cases.json").read_text())
        self.assertGreaterEqual(len(cases), 30)
        self.assertEqual(len({c["id"] for c in cases}), len(cases))

    def test_every_write_case_confirms_and_checks_state_unless_it_tests_a_refusal_or_approval(self) -> None:
        # The plan's rule (step 2): a write case has `confirm` and `expect_state`,
        # except a case that tests a refusal (`expect_no_tool`, or a no-chart
        # `expect_state` with count 0) or an approval (`expect_approval`).
        cases = json.loads((HERE / "dashboard_cases.json").read_text())
        for case in cases:
            if "expect_approval" in case or "expect_no_tool" in case:
                continue
            if case["id"] in {"dash_ask_refuses_write", "silver_refused", "missing_column_refused",
                              "no_false_approval_claim"}:
                continue
            if case.get("mode") == "ask" and "modes" not in case:
                continue
            if case.get("confirm") is None and "expect_state" not in case:
                continue
            self.assertIs(case.get("confirm"), True, case["id"])
            self.assertIn("expect_state", case, case["id"])


if __name__ == "__main__":
    unittest.main()
