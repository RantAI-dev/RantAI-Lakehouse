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
runs can be compared case by case.
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


def resolve_fact(cfg: Config, session: requests.Session, fact: dict[str, Any]) -> list[Any]:
    """The expected values of one fact: every row's first column for `sql`,
    every value at `path` of an API GET for `api`, or `values` as given."""
    if "sql" in fact:
        return [row[0] for row in ch_rows(cfg, fact["sql"])]
    if "api" in fact:
        items = dig(api_get(cfg, session, fact["api"]), fact["path"])
        where = fact.get("where", {})
        items = [i for i in items if all(isinstance(i, dict) and i.get(k) == v for k, v in where.items())]
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


def ask(cfg: Config, session: requests.Session, mode: str, messages: list[dict[str, str]]) -> dict[str, Any]:
    started = time.monotonic()
    resp = session.post(
        f"{cfg.api_url}/api/ai/chat",
        json={"mode": mode, "messages": messages, "stream": False},
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


def run_case(cfg: Config, session: requests.Session, case: dict[str, Any]) -> dict[str, Any]:
    mode = case.get("mode", "ask")
    messages: list[dict[str, str]] = []
    bodies: list[dict[str, Any]] = []
    for turn in case["turns"]:
        messages.append({"role": "user", "content": turn})
        body = ask(cfg, session, mode, messages)
        bodies.append(body)
        messages.append({"role": "assistant", "content": plain_text(body.get("answer", ""))})
    return score_case(cfg, session, case, bodies)


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
        if any(all(str(v).lower() in str(t.get("args", {}).get(k, "")).lower() for k, v in want.items()) for t in trace):
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
    args = parser.parse_args()

    cfg = Config.from_env()
    cases = json.loads(Path(args.cases).read_text())
    if args.only:
        wanted = set(args.only.split(","))
        cases = [c for c in cases if c["id"] in wanted]

    session = requests.Session()
    login = session.post(
        f"{cfg.api_url}/api/auth/login",
        json={"email": cfg.email, "password": cfg.password},
        timeout=30,
    )
    login.raise_for_status()

    saved = {}
    if args.rescore:
        saved = {r["id"]: r for r in json.loads(Path(args.rescore).read_text())["results"]}
        cases = [c for c in cases if c["id"] in saved]

    results = []
    for case in cases:
        try:
            if saved:
                result = score_case(cfg, session, case, bodies_from_saved(saved[case["id"]]))
            else:
                result = run_case(cfg, session, case)
        except requests.RequestException as err:
            result = {"id": case["id"], "completeness": 0, "accuracy": 0, "experience": 0,
                      "overall": 0, "problems": [f"request failed: {type(err).__name__}"]}
        results.append(result)
        print(
            f"{result['id']:<24} C={result['completeness']:.2f} A={result['accuracy']:.2f} "
            f"E={result['experience']:.2f} O={result['overall']:.2f} "
            f"{result.get('elapsed_s', 0):>5}s tools={','.join(t or '?' for t in result.get('tools', []))} "
            f"{'; '.join(result.get('problems', []) + ['missing ' + ', '.join(result.get('missing', []))] if result.get('missing') else result.get('problems', []))}",
            flush=True,
        )

    n = len(results) or 1
    summary = {
        key: round(sum(r[key] for r in results) / n, 3)
        for key in ("completeness", "accuracy", "experience", "overall")
    }
    print(f"\n{args.label}: " + "  ".join(f"{k}={v:.3f}" for k, v in summary.items()))
    out = Path(args.out) if args.out else Path(tempfile.gettempdir()) / f"ai-eval-{args.label}-{int(time.time())}.json"
    out.write_text(json.dumps({"label": args.label, "summary": summary, "results": results}, ensure_ascii=False, indent=2))
    print(f"saved {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
