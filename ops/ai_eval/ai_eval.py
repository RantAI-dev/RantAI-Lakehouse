#!/usr/bin/env python3
"""Copilot evaluation harness: asks `POST /api/ai/chat` a fixed set of
questions and scores the answers for completeness, accuracy and experience.

# Why this exists

Copilot quality was judged by eye in the console, one question at a time.
That cannot tell a prompt change that helps from one that only moved the
failure somewhere else, and it cannot compare models. This harness runs
the same cases every time and prints one scored row per case, so a change
to the system prompt, a tool, or `LLM_MODEL` can be measured, not guessed.

# Ground truth is computed, never written down

Every expected number or name comes from a SQL query run against the same
ClickHouse the API reads (or from an API list call) at evaluation time.
Nothing in `cases.json` is a hardcoded data value: the cases hold the
question and the query that answers it. So the harness stays correct when
the seed data changes, and no tenant data is copied into this repository.

# Scoring

- **Completeness**: the share of expected facts (numbers within a
  tolerance, names case-insensitively) that appear in the answer.
- **Accuracy**: starts at 1. It drops to 0 when a case that has no answer
  in the data is answered anyway (no refusal wording), or when a forbidden
  pattern (a raw database error, an invented currency amount) appears. Each
  number the API's own citation checker marked unverified costs 0.1, down
  to 0.5.
- **Experience**: latency, answering in the question's language, and no
  raw upstream error text.
- **Overall** = 0.4 completeness + 0.4 accuracy + 0.2 experience, the
  weighting the goal asked for.

A number that matches is only evidence the model printed the right value
somewhere in its answer, not proof the sentence around it is right. Read
the saved answers for any case whose score moves.

# Running it

    AI_EVAL_EMAIL=… AI_EVAL_PASSWORD=… CH_USER=… CH_PASSWORD=… \
        python3 ops/ai_eval/ai_eval.py --label baseline

`--only id1,id2` runs a subset. Results go to `--out` (default: a JSON file
in the system temp directory), with every answer and tool trace, so two
runs can be compared case by case. A relative `--out` is also taken under the
temp directory, so a run from the repository never writes results into it.

# The dashboard set

`dashboard_cases.json` is the standard set of dashboard requests. One command
brings up a throwaway stack, seeds it, runs the set and takes the stack away
again:

    LLM_KEY=… ops/ai_eval/run_dashboard_eval.sh --out /tmp/baseline.json

The set passes at 90% of its cases or more. `--repeat N --need K` runs each
case N times and needs K of them to pass, so one flaky answer does not decide
a case; the run above uses `--repeat 3 --need 2`. `--baseline <results.json>`
adds a second rule: a case that passed in the baseline and fails now fails the
set however good the percentage is. A case the baseline never saw is a new
case, not a regression.

Every case sets up the objects it needs, confirms what the assistant drafts
the way the console does, checks the state it created, and deletes it again,
so the set does not depend on what the stack happens to hold and does not
leave anything behind. A run that could not finish is a failed case, not a
crash, and a delete that fails is a problem of that case.

Exit code 0 when the set passes, 1 when it fails, 2 when the run could not
start (no stack, no login, a port already in use).

This run belongs to a person, not to CI: it needs a model key, it builds an
image, and it answers are read by whoever runs it. Put the numbers in the
review of the PR that changed the tool or prompt. Results are never kept in
the repository.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import requests

HERE = Path(__file__).resolve().parent

REFUSAL_CUES = (
    "no data",
    "not available",
    "isn't available",
    "is not available",
    "unavailable",
    "don't have",
    "do not have",
    "doesn't have",
    "does not have",
    "doesn't contain",
    "does not contain",
    "not found",
    "no record",
    "not recorded",
    "not tracked",
    "no information",
    "not in the",
    "only covers",
    "only has data",
    "there are no",
    "there is no",
    "there aren't any",
    "are no ",
    "none ",
    "not configured",
    "no alert",
    "no connector",
    "not covered",
    "isn't covered",
    "is not covered",
    "outside the",
    "does not cover",
    "doesn't cover",
    "tidak tercakup",
    "di luar",
    "does not include",
    "doesn't include",
    "not included",
    "tidak termasuk",
    "tidak ada",
    "tidak tersedia",
    "belum ada",
    "belum tersedia",
    "tidak ditemukan",
    "tidak memiliki",
    "tidak mencakup",
    "hanya mencakup",
    "hanya tersedia",
)

# Words that appear in almost any Indonesian answer and almost never in an
# English one; three or more distinct hits reads as Indonesian.
INDONESIAN_MARKERS = (
    "yang",
    "dan",
    "dengan",
    "adalah",
    "untuk",
    "dari",
    "pada",
    "tidak",
    "ini",
    "jumlah",
    "tahun",
    "terbanyak",
    "sebesar",
    "yaitu",
    "punya",
    "paling",
    "banyak",
    "sumber",
    "atau",
    "juga",
    "ada",
    "di",
    "ke",
    "usaha",
    "berdasarkan",
)

RAW_ERROR_PATTERNS = (r"DB::Exception", r"Code: \d+", r"\bstack trace\b", r"SQLSTATE")

MULTIPLIERS = {
    "million": 1e6,
    "juta": 1e6,
    "jt": 1e6,
    "m": 1e6,
    "billion": 1e9,
    "miliar": 1e9,
    "b": 1e9,
    "thousand": 1e3,
    "ribu": 1e3,
    "rb": 1e3,
    "k": 1e3,
}


@dataclass(frozen=True)
class Config:
    api_url: str
    ch_url: str
    ch_user: str
    ch_password: str
    email: str
    password: str
    timeout_s: int

    @classmethod
    def from_env(cls) -> Config:
        email = os.environ.get("AI_EVAL_EMAIL", "")
        password = os.environ.get("AI_EVAL_PASSWORD", "")
        if not email or not password:
            raise SystemExit("AI_EVAL_EMAIL and AI_EVAL_PASSWORD must be set")
        return cls(
            api_url=os.environ.get("AI_EVAL_API_URL", "http://127.0.0.1:27080").rstrip("/"),
            ch_url=os.environ.get("AI_EVAL_CH_URL", "http://127.0.0.1:27123").rstrip("/"),
            ch_user=os.environ.get("CH_USER", "default"),
            ch_password=os.environ.get("CH_PASSWORD", ""),
            email=email,
            password=password,
            timeout_s=int(os.environ.get("AI_EVAL_TIMEOUT_S", "240")),
        )


# ── ground truth ─────────────────────────────────────────────────────────


def ch_rows(cfg: Config, sql: str) -> list[list[Any]]:
    """Rows of `sql` as lists, first column first (`JSONCompact`)."""
    resp = requests.post(
        cfg.ch_url,
        params={"default_format": "JSONCompact"},
        data=sql.encode(),
        auth=(cfg.ch_user, cfg.ch_password),
        timeout=30,
    )
    resp.raise_for_status()
    return resp.json()["data"]


def api_get(cfg: Config, session: requests.Session, path: str) -> Any:
    resp = session.get(f"{cfg.api_url}{path}", timeout=60)
    resp.raise_for_status()
    return resp.json()


def dig(value: Any, path: str) -> list[Any]:
    """`a.b[].c` style lookup; `[]` fans out over a list."""
    current = [value]
    for part in path.split("."):
        fan = part.endswith("[]")
        key = part[:-2] if fan else part
        nxt: list[Any] = []
        for item in current:
            if key:
                item = item.get(key) if isinstance(item, dict) else None
            if item is None:
                continue
            if fan and isinstance(item, list):
                nxt.extend(item)
            elif not fan:
                nxt.append(item)
        current = nxt
    return current


def where_matches(item: Any, where: dict[str, Any]) -> bool:
    """Whether `item` satisfies every `where` key. A flat key reads the item's
    own field; a dotted key reads into it (`def.grain` is the `grain` of the
    item's `def`, as `GET /api/dashboard/specs` nests it). A key that is absent
    does not match. A field that holds a list matches when the value is one of
    its members (`def.measures` names a measure the chart uses)."""
    if not isinstance(item, dict):
        return False
    for key, want in where.items():
        found = dig(item, key)
        if not found:
            return False
        if not any(_is_or_holds(got, want) for got in found):
            return False
    return True


def _is_or_holds(got: Any, want: Any) -> bool:
    if isinstance(got, list):
        return want in got
    return got == want


def resolve_fact(cfg: Config, session: requests.Session, fact: dict[str, Any]) -> list[Any]:
    """The expected values of one fact: every row's first column for `sql`,
    every value at `path` of an API GET for `api`, or `values` as given."""
    if "sql" in fact:
        return [row[0] for row in ch_rows(cfg, fact["sql"])]
    if "api" in fact:
        items = dig(api_get(cfg, session, fact["api"]), fact["path"])
        where = fact.get("where", {})
        items = [i for i in items if where_matches(i, where)]
        if "field" in fact:
            items = [i.get(fact["field"]) for i in items if isinstance(i, dict)]
        return items[: fact.get("limit", 50)]
    return list(fact.get("values", []))


# ── answer parsing ───────────────────────────────────────────────────────


def plain_text(answer: str) -> str:
    """The answer without the citation checker's HTML markers."""
    return re.sub(r"<[^>]+>", "", answer)


NUMBER_RE = re.compile(r"(?<![\w.])(\d[\d.,]*\d|\d)\s*(%|[A-Za-z]+)?")


def number_readings(token: str) -> set[float]:
    """Every value `token` could mean, reading `.`/`,` either as thousands
    separators or as the decimal point: `2.643.888`, `2,643,888`, `12,09`
    and `12.09` all parse; an ambiguous `2.643` yields both 2.643 and 2643."""
    readings: set[float] = set()
    for thousands, decimal in ((",", "."), (".", ",")):
        body = token
        if body.count(decimal) > 1:
            continue
        body = body.replace(thousands, "").replace(decimal, ".")
        try:
            readings.add(float(body))
        except ValueError:
            continue
    return readings


def numbers_in(text: str) -> set[float]:
    found: set[float] = set()
    for match in NUMBER_RE.finditer(text):
        token, suffix = match.group(1), (match.group(2) or "").lower()
        mult = MULTIPLIERS.get(suffix, 1.0)
        for value in number_readings(token):
            found.add(value)
            found.add(value * mult)
    return found


def has_number(found: set[float], expected: float, tol: float) -> bool:
    if expected == 0:
        return any(abs(v) < 1e-9 for v in found)
    return any(abs(v - expected) <= abs(expected) * tol for v in found)


def looks_indonesian(text: str) -> bool:
    words = set(re.findall(r"[a-z]+", text.lower()))
    return sum(1 for w in INDONESIAN_MARKERS if w in words) >= 3


# ── one case ─────────────────────────────────────────────────────────────


def ask(
    cfg: Config,
    session: requests.Session,
    mode: str,
    messages: list[dict[str, str]],
    context: str | None = None,
) -> dict[str, Any]:
    started = time.monotonic()
    payload: dict[str, Any] = {"mode": mode, "messages": messages, "stream": False}
    if context:
        # `ChatBody.context` (routes/ai/mod.rs): the page the person is looking at.
        payload["context"] = context
    resp = session.post(
        f"{cfg.api_url}/api/ai/chat",
        json=payload,
        timeout=cfg.timeout_s,
    )
    elapsed = time.monotonic() - started
    body: dict[str, Any]
    try:
        body = resp.json()
    except ValueError:
        body = {"error": f"non-JSON response, HTTP {resp.status_code}"}
    body["_status"] = resp.status_code
    body["_elapsed_s"] = round(elapsed, 1)
    return body


class CaseError(Exception):
    """A case file entry the harness cannot run (a bad `modes` list)."""


class SetupError(Exception):
    """A case's `setup` call was refused: the run is not scored."""


# The collection a created object is in, by its API route; the cleanup and
# `expect_state` both name objects by these kinds.
DELETE_ROUTES = {
    "chart": "/api/dashboard/specs",
    "field": "/api/dashboard/calc-fields",
    "source": "/api/dashboard/sources",
    "board": "/api/dashboard/boards",
}
# Newest first: a chart may use a field, a field a source, and a board holds
# charts, so the dependants go before what they depend on.
CLEANUP_ORDER = ("chart", "field", "source", "board")


def kind_of_path(path: str) -> str | None:
    for kind, route in DELETE_ROUTES.items():
        if path == route or path.startswith(route + "/"):
            return kind
    return None


def turn_modes(case: dict[str, Any]) -> list[str]:
    """One mode per turn: `modes` when given, else the case's `mode`."""
    turns = case["turns"]
    if "modes" in case:
        modes = case["modes"]
        if len(modes) != len(turns):
            raise CaseError(f"{case.get('id', '?')}: modes has {len(modes)} entries for {len(turns)} turns")
        return list(modes)
    return [case.get("mode", "ask")] * len(turns)


def fill_names(text: str, names: dict[str, str]) -> str:
    """Replace `{name}` in a turn with the id a setup call saved under it."""
    return re.sub(r"\{(\w+)\}", lambda match: names.get(match.group(1), match.group(0)), text)


def fill_value(value: Any, names: dict[str, str]) -> Any:
    """`fill_names` through a JSON value: strings, and the strings in lists and
    objects. A setup body and an `expect_state` `where` use the same names."""
    if isinstance(value, str):
        return fill_names(value, names)
    if isinstance(value, list):
        return [fill_value(v, names) for v in value]
    if isinstance(value, dict):
        return {k: fill_value(v, names) for k, v in value.items()}
    return value


def fill_check(check: dict[str, Any], names: dict[str, str]) -> dict[str, Any]:
    """An `expect_state` with its `{name}` values replaced by the run's ids."""
    filled = dict(check)
    filled["where"] = fill_value(check.get("where", {}), names)
    filled["path"] = fill_names(check["path"], names)
    return filled


CREATED_RE = re.compile(r"\{created:(\w+)\}")


def fill_created(check: dict[str, Any], created: list[dict[str, str]]) -> dict[str, Any]:
    """Replace `{created:<kind>}` in a `where` value with the id of the first
    object of that kind this run created (from the trace or a confirm). The
    assistant makes such an object itself, so its id is not known when the case
    is written. A kind this run did not create is left as written, so it
    matches nothing."""
    first: dict[str, str] = {}
    for obj in created:
        first.setdefault(obj["kind"], obj["id"])

    def sub(value: Any) -> Any:
        if isinstance(value, str):
            return CREATED_RE.sub(lambda mt: first.get(mt.group(1), mt.group(0)), value)
        if isinstance(value, dict):
            return {k: sub(v) for k, v in value.items()}
        return value

    filled = dict(check)
    filled["where"] = sub(check.get("where", {}))
    return filled


def formula_problems(
    trace: list[dict[str, Any]], confirms: list[dict[str, Any]], forbidden: list[str]
) -> list[str]:
    """`formula_forbid`: the formula of every calculated field the run created
    must not contain any of `forbidden`. The answer is scored as text; the
    formula is a field of the tool's result and is checked here.

    Both places a created field's result can arrive are read. A tool the
    model called with `confirmed` itself answers in the trace. A tool the
    person confirmed -- which is the flow every writing case in the set
    uses -- returns the pending `{needs_confirmation, tool, args}` in the
    trace and the executed `{ok, field}` only in `confirms`, so reading the
    trace alone would find no formula and let the case pass unchecked."""
    formulas: list[str] = []
    for step in trace:
        if step.get("tool") == "create_calculated_field":
            formulas.append(_created_formula(step.get("result")))
    for confirm in confirms:
        if confirm.get("tool") == "create_calculated_field":
            formulas.append(_created_formula(confirm.get("result")))
    problems: list[str] = []
    for formula in formulas:
        for word in forbidden:
            if re.search(word, formula, re.IGNORECASE):
                problems.append(f"formula contains {word}")
    return problems


def _created_formula(result: Any) -> str:
    """The formula of a `create_calculated_field` result, or `""` when the
    result is the pending confirmation rather than the executed call."""
    field = result.get("field") if isinstance(result, dict) else None
    return str((field or {}).get("formula", "")) if isinstance(field, dict) else ""


def pending_of(step: dict[str, Any]) -> dict[str, Any]:
    """The call a trace step asks the person to confirm (use-copilot.ts:380-382):
    the step's result when it holds one, else the step's own tool and args."""
    result = step.get("result") or {}
    if isinstance(result, dict) and result.get("needs_confirmation"):
        return {"tool": result.get("tool", step.get("tool")), "args": result.get("args", {})}
    return {"tool": step.get("tool"), "args": step.get("args", {})}


def confirm_body(pending: dict[str, Any], mode: str, session_id: str | None) -> dict[str, Any]:
    """The `POST /api/ai/tool` body the console sends to confirm a draft
    (use-copilot.ts:384-401): `title` copied from `caption`, `name` or `text`
    when it is missing or empty, then `confirmed: true`."""
    args = dict(pending.get("args") or {})
    if not args.get("title") and (args.get("caption") or args.get("name") or args.get("text")):
        args["title"] = str(args.get("caption") or args.get("name") or args.get("text"))
    args["confirmed"] = True
    return {"tool": pending.get("tool"), "args": args, "mode": mode, "sessionId": session_id}


def send_confirm(
    cfg: Config, session: requests.Session, step: dict[str, Any], mode: str, session_id: str | None
) -> dict[str, Any]:
    resp = session.post(
        f"{cfg.api_url}/api/ai/tool",
        json=confirm_body(pending_of(step), mode, session_id),
        timeout=cfg.timeout_s,
    )
    try:
        body = resp.json()
    except ValueError:
        body = {"error": f"non-JSON response, HTTP {resp.status_code}"}
    body["_status"] = resp.status_code
    return body


def run_setup(
    cfg: Config, session: requests.Session, setup: list[dict[str, Any]]
) -> tuple[dict[str, str], list[dict[str, str]]]:
    """Make the case's own objects through the API. Returns the ids saved under
    each name and the objects created, for cleanup."""
    names: dict[str, str] = {}
    created: list[dict[str, str]] = []
    for call in setup:
        resp = session.request(
            call["method"],
            f"{cfg.api_url}{fill_names(call['path'], names)}",
            json=fill_value(call.get("body"), names),
            timeout=60,
        )
        if resp.status_code >= 400:
            raise SetupError(f"setup {call.get('name', call['path'])}: HTTP {resp.status_code}")
        body = resp.json()
        found = dig(body, call["id_at"])
        if not found:
            raise SetupError(f"setup {call.get('name', call['path'])}: no {call['id_at']} in the reply")
        obj_id = str(found[0])
        if "name" in call:
            names[call["name"]] = obj_id
        kind = kind_of_path(call["path"])
        if kind:
            created.append({"kind": kind, "id": obj_id})
    return names, created


def collect_created(
    setup_created: list[dict[str, str]],
    traces: list[list[dict[str, Any]]],
    confirms: list[dict[str, Any]],
) -> list[dict[str, str]]:
    """Every object this run created: setup calls, trace results marked
    created with an id, and the calculated fields and boards the tools made."""
    found = list(setup_created)
    for trace in traces:
        for step in trace:
            result = step.get("result") or {}
            if not isinstance(result, dict) or not result.get("id"):
                continue
            tool = step.get("tool")
            if tool == "create_chart" and result.get("created"):
                found.append({"kind": "chart", "id": str(result["id"])})
            elif tool == "create_board" and result.get("created"):
                found.append({"kind": "board", "id": str(result["id"])})
            elif tool == "create_calculated_field" and result.get("ok") is True:
                found.append({"kind": "field", "id": str(result["id"])})
    for confirm in confirms:
        tool = confirm.get("tool")
        result = confirm.get("result") or {}
        if isinstance(result, dict) and result.get("id") and result.get("created"):
            kind = {"create_chart": "chart", "create_board": "board"}.get(tool or "")
            if kind:
                found.append({"kind": kind, "id": str(result["id"])})
        if tool == "create_calculated_field" and isinstance(result, dict) and result.get("id") and result.get("ok") is True:
            found.append({"kind": "field", "id": str(result["id"])})
    return found


def cleanup_plan(created: list[dict[str, str]]) -> list[dict[str, str]]:
    """The deletes, in the order they must run: charts, calculated fields, SQL
    sources, boards, and within a kind the last created first."""
    ordered: list[dict[str, str]] = []
    for kind in CLEANUP_ORDER:
        ordered.extend(reversed([c for c in created if c["kind"] == kind]))
    return ordered


def run_cleanup(cfg: Config, session: requests.Session, created: list[dict[str, str]]) -> list[str]:
    """Delete what the run created. A failed delete is returned as a problem."""
    problems: list[str] = []
    for obj in cleanup_plan(created):
        route = DELETE_ROUTES[obj["kind"]]
        try:
            resp = session.delete(f"{cfg.api_url}{route}?id={obj['id']}", timeout=60)
        except requests.RequestException as err:
            problems.append(f"cleanup {obj['kind']} {obj['id']}: {type(err).__name__}")
            continue
        if resp.status_code >= 400:
            problems.append(f"cleanup {obj['kind']} {obj['id']}: HTTP {resp.status_code}")
    return problems


def expect_state_problems(
    cfg: Config,
    session: requests.Session,
    check: dict[str, Any],
    own_ids: set[str],
) -> list[str]:
    """`expect_state`: list a collection and count the objects matching `where`.
    Only objects this run created count, unless `scope` is `all`."""
    resp = session.get(f"{cfg.api_url}{check['path']}", timeout=60)
    resp.raise_for_status()
    items = resp.json().get(check["list"], [])
    where = check.get("where", {})
    matches = [
        i
        for i in items
        if where_matches(i, where)
        and (check.get("scope") == "all" or str(i.get("id")) in own_ids)
    ]
    if "min" in check:
        if len(matches) < check["min"]:
            return [f"state {check['path']} {where}: {len(matches)} found, at least {check['min']} expected"]
        return []
    want = check.get("count", 1)
    if len(matches) != want:
        return [f"state {check['path']} {where}: {len(matches)} found, {want} expected"]
    return []


def approval_problems(trace: list[dict[str, Any]], tool: str) -> list[str]:
    """`expect_approval`: the tool's step must hold `needs_approval: true`."""
    for step in trace:
        result = step.get("result") or {}
        if step.get("tool") == tool and isinstance(result, dict) and result.get("needs_approval") is True:
            return []
    return [f"approval: {tool} did not ask for approval"]


def no_tool_problems(trace: list[dict[str, Any]], names: list[str]) -> list[str]:
    """`expect_no_tool`: none of the named tools may appear with `ok: true`."""
    return [
        f"ran {step.get('tool')} (ok)"
        for step in trace
        if step.get("tool") in names and step.get("ok") is True
    ]


def forbid_unless_problems(
    answer: str, trace: list[dict[str, Any]], pattern: str, result_key: str
) -> list[str]:
    """`forbid_unless_tool`: the answer may match `pattern` only when some trace
    result carries `result_key`. It catches a claim such as "waiting in
    Approvals" that the server never backed with a `needs_approval` result."""
    if not re.search(pattern, answer, re.IGNORECASE):
        return []
    for step in trace:
        result = step.get("result") or {}
        if isinstance(result, dict) and result_key in result:
            return []
    return [f"forbidden: {pattern} without {result_key}"]


def with_run_fields(scored: dict[str, Any], runs: list[dict[str, Any]], need: int) -> dict[str, Any]:
    """Add `runs` and `passed` to a case's scored result; every existing key is kept."""
    out = dict(scored)
    out["runs"] = runs
    out["passed"] = sum(1 for r in runs if run_passed(r)) >= need
    return out


def run_passed(run: dict[str, Any]) -> bool:
    """A run passes when it has no missing item and no problem item."""
    return not run.get("missing") and not run.get("problems")


def case_verdict(case_id: str, runs: list[dict[str, Any]], need: int) -> dict[str, Any]:
    passed = sum(1 for r in runs if run_passed(r)) >= need
    return {"id": case_id, "runs": runs, "passed": passed}


def set_verdict(cases: list[dict[str, Any]], baseline: dict[str, bool] | None) -> dict[str, Any]:
    """The set passes at 90% of its cases or more, and with a baseline no case
    that passed there may fail. A case not in the baseline is not a regression."""
    total = len(cases)
    passed = sum(1 for c in cases if c["passed"])
    regressed = []
    if baseline is not None:
        regressed = [c["id"] for c in cases if baseline.get(c["id"]) and not c["passed"]]
    failed = [c["id"] for c in cases if not c["passed"]]
    enough = total > 0 and passed * 10 >= total * 9
    return {
        "pass": enough and not regressed,
        "passed": passed,
        "total": total,
        "failed": failed,
        "regressed": regressed,
    }


def summary_line(verdict: dict[str, Any]) -> str:
    pct = round(100 * verdict["passed"] / verdict["total"]) if verdict["total"] else 0
    head = "PASS" if verdict["pass"] else "FAIL"
    text = f"{head} {verdict['passed']}/{verdict['total']} ({pct}%), "
    if verdict["regressed"]:
        text += "regressions: " + ", ".join(verdict["regressed"])
    else:
        text += "regressions: none"
    if verdict["failed"]:
        text += "; failed: " + ", ".join(verdict["failed"])
    return text


def exit_code(setup_error: bool, verdict: dict[str, Any] | None) -> int:
    if setup_error:
        return 2
    return 0 if verdict and verdict["pass"] else 1


def result_document(label: str, cases: list[dict[str, Any]], verdict: dict[str, Any] | None) -> dict[str, Any]:
    return {"label": label, "summary": {}, "results": cases, "verdict": verdict}


def run_case(cfg: Config, session: requests.Session, case: dict[str, Any]) -> dict[str, Any]:
    """One run of one case: setup, the turns (with confirms), the checks, then
    cleanup in a `finally`. A failed cleanup is a problem of this run's result."""
    modes = turn_modes(case)
    messages: list[dict[str, str]] = []
    bodies: list[dict[str, Any]] = []
    confirms: list[dict[str, Any]] = []
    setup_created: list[dict[str, str]] = []
    names: dict[str, str] = {}
    scored: dict[str, Any] | None = None
    _raised_run: BaseException | None = None
    try:
        names, setup_created = run_setup(cfg, session, case.get("setup", []))
        context = fill_names(case["context"], names) if case.get("context") else None
        for turn, mode in zip(case["turns"], modes):
            messages.append({"role": "user", "content": fill_names(turn, names)})
            body = ask(cfg, session, mode, messages, context=context)
            bodies.append(body)
            if case.get("confirm"):
                for step in body.get("toolTrace", []):
                    result = step.get("result") or {}
                    if isinstance(result, dict) and result.get("needs_confirmation"):
                        response = send_confirm(cfg, session, step, mode, body.get("sessionId"))
                        confirms.append({"tool": pending_of(step)["tool"], "result": response})
            messages.append({"role": "assistant", "content": plain_text(body.get("answer", ""))})
        scored = score_case(cfg, session, case, bodies)
        scored["confirms"] = confirms
        trace = [s for b in bodies for s in b.get("toolTrace", [])]
        created = collect_created(setup_created, [trace], confirms)
        own_ids = {c["id"] for c in created}
        for check in case.get("expect_state", []):
            try:
                filled = fill_created(fill_check(check, names), created)
                scored["problems"].extend(expect_state_problems(cfg, session, filled, own_ids))
            except requests.RequestException as err:
                scored["problems"].append(f"state check failed: {type(err).__name__}")
        if "expect_approval" in case:
            scored["problems"].extend(approval_problems(trace, case["expect_approval"]))
        if "expect_no_tool" in case:
            scored["problems"].extend(no_tool_problems(trace, case["expect_no_tool"]))
        if "forbid_unless_tool" in case:
            spec = case["forbid_unless_tool"]
            scored["problems"].extend(
                forbid_unless_problems(scored["answer"], trace, spec["pattern"], spec["result_key"])
            )
        if "formula_forbid" in case:
            scored["problems"].extend(formula_problems(trace, confirms, case["formula_forbid"]))
    except BaseException as raised:  # noqa: BLE001 - a failed run is a result, not a crash
        _raised_run = raised
    finally:
        # Cleanup runs on every exit. A run that scored keeps its problems on the
        # result; a run that raised becomes a failed result with the same problems,
        # so a failed cleanup is never only on stderr.
        cleanup_problems = _cleanup(cfg, session, setup_created, bodies, confirms)
        if scored is None:
            scored = _failed_run(case, _raised_run, cleanup_problems)
        else:
            scored["problems"].extend(cleanup_problems)
    return scored


def _failed_run(
    case: dict[str, Any], raised: BaseException | None, cleanup_problems: list[str]
) -> dict[str, Any]:
    """A run that could not finish is a failed run, not a crash: the reason it
    raised is its first problem, and a cleanup failure is a problem of the run."""
    if isinstance(raised, requests.RequestException):
        problems = [f"request failed: {type(raised).__name__}"]
    elif isinstance(raised, (SetupError, CaseError)):
        problems = [f"setup: {raised}"]
    elif raised is None:
        problems = ["run failed for an unknown reason"]
    else:
        problems = [f"run failed: {type(raised).__name__}: {raised}"]
    problems.extend(cleanup_problems)
    return {
        "id": case["id"],
        "completeness": 0,
        "accuracy": 0,
        "experience": 0,
        "overall": 0,
        "problems": problems,
        "missing": [],
    }


def _cleanup(
    cfg: Config,
    session: requests.Session,
    setup_created: list[dict[str, str]],
    bodies: list[dict[str, Any]],
    confirms: list[dict[str, Any]],
) -> list[str]:
    trace = [s for b in bodies for s in b.get("toolTrace", [])]
    return run_cleanup(cfg, session, collect_created(setup_created, [trace], confirms))


def bodies_from_saved(result: dict[str, Any]) -> list[dict[str, Any]]:
    """The response bodies a saved result was scored from, rebuilt for
    `--rescore`: the tool trace per turn and the final answer, status and
    latency. Earlier turns' answers are not saved and are not scored."""
    traces = result.get("trace") or [[]]
    status = 200
    for problem in result.get("problems", []):
        match = re.search(r"no answer \(HTTP (\d+)", problem)
        if match:
            status = int(match.group(1))
    bodies = [{"toolTrace": t, "answer": "", "_status": 200, "_elapsed_s": result.get("elapsed_s", 0)} for t in traces]
    bodies[-1].update({"answer": result.get("answer", ""), "_status": status})
    note = next((p[len("note: "):] for p in result.get("problems", []) if p.startswith("note: ")), None)
    if note:
        bodies[-1]["note"] = note
    return bodies


def score_case(
    cfg: Config, session: requests.Session, case: dict[str, Any], bodies: list[dict[str, Any]]
) -> dict[str, Any]:
    final = bodies[-1]
    raw_answer = final.get("answer", "") or ""
    answer = plain_text(raw_answer)
    lowered = answer.lower()
    found_numbers = numbers_in(answer)

    required = 0
    hit = 0
    missing: list[str] = []
    for fact in case.get("facts", []):
        values = resolve_fact(cfg, session, fact)
        kind = fact.get("kind", "text")
        for value in values:
            required += 1
            if kind == "number":
                ok = has_number(found_numbers, float(value), fact.get("tol", 0.015))
            else:
                needles = [str(value).lower()] + [a.lower() for a in fact.get("aliases", {}).get(str(value), [])]
                ok = any(n in lowered for n in needles)
            if ok:
                hit += 1
            else:
                missing.append(f"{fact.get('label', kind)}={value}")

    tools_called = [t.get("tool") for b in bodies for t in b.get("toolTrace", [])]
    expect_tool = case.get("expect_tool")
    if expect_tool:
        required += 1
        trace = [t for b in bodies for t in b.get("toolTrace", []) if t.get("tool") == expect_tool["name"]]
        want = expect_tool.get("args_contains", {})
        # BLD-2 review R2 SHOULD-FIX 9: dotted keys traverse nested args so matching is key-order independent
        def _get_arg(args: dict, key: str):
            curr = args
            for part in key.split("."):
                if isinstance(curr, dict):
                    curr = curr.get(part, "")
                else:
                    return ""
            return curr

        if any(all(str(v).lower() in str(_get_arg(t.get("args", {}), k)).lower() for k, v in want.items()) for t in trace):
            hit += 1
        else:
            missing.append(f"tool={expect_tool['name']}{want}")

    refused = any(cue in lowered for cue in REFUSAL_CUES)
    # No expected facts (e.g. no pipeline is failing right now): completeness
    # is about declining only when the case expects that.
    if required:
        completeness = hit / required
    elif case.get("expect_refusal"):
        completeness = 1.0 if refused else 0.0
    else:
        completeness = 1.0

    accuracy = 1.0
    problems: list[str] = []
    if case.get("expect_refusal") and not refused:
        accuracy = 0.0
        problems.append("answered a question the data cannot answer")
    for pattern in case.get("forbid", []) + list(RAW_ERROR_PATTERNS):
        if re.search(pattern, answer, re.IGNORECASE):
            accuracy = 0.0
            problems.append(f"forbidden: {pattern}")
    unverified = raw_answer.count('data-unverified="true"')
    accuracy = max(0.0, accuracy - min(0.5, 0.1 * unverified)) if accuracy > 0 else 0.0

    elapsed = sum(b.get("_elapsed_s", 0) for b in bodies) / len(bodies)
    experience = 1.0 if elapsed <= 20 else 0.8 if elapsed <= 40 else 0.5 if elapsed <= 90 else 0.2
    if case.get("lang") == "id" and not looks_indonesian(answer):
        experience -= 0.4
        problems.append("did not answer in Indonesian")
    if case.get("lang", "en") == "en" and looks_indonesian(answer):
        experience -= 0.4
        problems.append("answered an English question in Indonesian")
    if not answer.strip() or final.get("_status") != 200:
        experience = 0.0
        completeness = 0.0
        problems.append(f"no answer (HTTP {final.get('_status')}: {final.get('error', '')})")
    if final.get("note"):
        experience -= 0.2
        problems.append(f"note: {final['note']}")
    experience = max(0.0, experience)

    overall = 0.4 * completeness + 0.4 * accuracy + 0.2 * experience
    return {
        "id": case["id"],
        "completeness": round(completeness, 3),
        "accuracy": round(accuracy, 3),
        "experience": round(experience, 3),
        "overall": round(overall, 3),
        "elapsed_s": round(elapsed, 1),
        "tools": tools_called,
        "unverified": unverified,
        "missing": missing,
        "problems": problems,
        "answer": raw_answer,
        "trace": [b.get("toolTrace", []) for b in bodies],
    }


def print_row(result: dict[str, Any]) -> None:
    print(
        f"{result['id']:<24} C={result['completeness']:.2f} A={result['accuracy']:.2f} "
        f"E={result['experience']:.2f} O={result['overall']:.2f} "
        f"{result.get('elapsed_s', 0):>5}s tools={','.join(t or '?' for t in result.get('tools', []))} "
        f"{'; '.join(result.get('problems', []) + ['missing ' + ', '.join(result.get('missing', []))] if result.get('missing') else result.get('problems', []))}",
        flush=True,
    )


def resolve_out(out: str, label: str) -> Path:
    """Where the results go. An absolute `--out` is kept as given; a relative
    one, and the default when none is given, goes under the temp directory, so
    a run from the repository never writes its results into it (S3)."""
    if out and Path(out).is_absolute():
        return Path(out)
    tmp = Path(tempfile.gettempdir())
    if not out:
        return tmp / f"ai-eval-{label}-{int(time.time())}.json"
    return tmp / out


def load_baseline(path: str) -> dict[str, bool]:
    """Which cases passed in a saved run's verdict (a run with `passed`)."""
    doc = json.loads(Path(path).read_text())
    return {r["id"]: bool(r.get("passed")) for r in doc.get("results", []) if "passed" in r}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--label", default="run")
    parser.add_argument("--cases", default=str(HERE / "cases.json"))
    parser.add_argument("--only", default="")
    parser.add_argument("--out", default="")
    parser.add_argument(
        "--rescore",
        default="",
        help="score the answers saved in this results file again, without asking the API",
    )
    parser.add_argument("--repeat", type=int, default=1, help="runs per case")
    parser.add_argument("--need", type=int, default=1, help="runs that must pass for a case to pass")
    parser.add_argument(
        "--baseline",
        default="",
        help="a saved results file; a case that passed there and fails now fails the set",
    )
    args = parser.parse_args()
    if args.need < 1 or args.need > args.repeat:
        parser.error("--need must be between 1 and --repeat")

    try:
        cfg = Config.from_env()
        cases = json.loads(Path(args.cases).read_text())
        if args.only:
            wanted = set(args.only.split(","))
            cases = [c for c in cases if c["id"] in wanted]
        baseline = load_baseline(args.baseline) if args.baseline else None

        session = requests.Session()
        login = session.post(
            f"{cfg.api_url}/api/auth/login",
            json={"email": cfg.email, "password": cfg.password},
            timeout=30,
        )
        login.raise_for_status()
    except (SystemExit, OSError, ValueError, requests.RequestException) as err:
        print(f"setup error: {err}", file=sys.stderr)
        return exit_code(setup_error=True, verdict=None)

    saved = {}
    if args.rescore:
        saved = {r["id"]: r for r in json.loads(Path(args.rescore).read_text())["results"]}
        cases = [c for c in cases if c["id"] in saved]

    results = []
    verdict_cases = []
    for case in cases:
        runs: list[dict[str, Any]] = []
        for _ in range(args.repeat):
            try:
                if saved:
                    runs.append(score_case(cfg, session, case, bodies_from_saved(saved[case["id"]])))
                else:
                    runs.append(run_case(cfg, session, case))
            except requests.RequestException as err:
                runs.append({"id": case["id"], "completeness": 0, "accuracy": 0, "experience": 0,
                             "overall": 0, "problems": [f"request failed: {type(err).__name__}"]})
            except (SetupError, CaseError) as err:
                runs.append({"id": case["id"], "completeness": 0, "accuracy": 0, "experience": 0,
                             "overall": 0, "problems": [f"setup: {err}"]})
        # The run whose own problems are fewest is the case's shown result;
        # the verdict counts every run.
        shown = min(runs, key=lambda r: (len(r.get("problems", [])) + len(r.get("missing", []))))
        result = with_run_fields(shown, runs=runs, need=args.need)
        results.append(result)
        verdict_cases.append({"id": case["id"], "passed": result["passed"], "runs": runs})
        print_row(shown)

    n = len(results) or 1
    summary = {
        key: round(sum(r[key] for r in results) / n, 3)
        for key in ("completeness", "accuracy", "experience", "overall")
    }
    print(f"\n{args.label}: " + "  ".join(f"{k}={v:.3f}" for k, v in summary.items()))

    verdict = set_verdict(verdict_cases, baseline=baseline) if verdict_cases else None
    if verdict:
        print(summary_line(verdict))
    out = resolve_out(args.out, args.label)
    out.write_text(json.dumps(result_document(args.label, results, verdict) | {"summary": summary},
                              ensure_ascii=False, indent=2))
    print(f"saved {out}")
    return exit_code(setup_error=False, verdict=verdict)


if __name__ == "__main__":
    sys.exit(main())
