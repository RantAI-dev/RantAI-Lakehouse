#!/usr/bin/env python3
"""Dashboard open-time benchmark (BI-4 part A, plan task T2): how long
`GET /api/dashboard?board=<id>` takes, N times in a row, against a running
API. It reads only; it writes nothing and needs no throwaway stack, but the
numbers mean something only on a quiet one (other people's requests share the
engine).

Plan: `docs/superpowers/plans/2026-10-24-bi-4-caching-parallel-tiles.md`.
The planner reads `docs/plans/BI-4-RESULT.md`, which quotes this script's
output unedited; this file measures and states no verdict.

Environment (nothing is read from the command line, so a token never shows in
the process list):

- `API_URL`: base URL of the API, for example `http://localhost:8080`.
- `BENCH_COOKIE`: the whole `Cookie` header of a signed-in session, or
- `BENCH_BEARER`: a bearer token. Exactly one of the two is required.

Usage: `dashboard_open_bench.py --board <id> [--rounds 20]`

Output is one JSON object on stdout: the board, the tile count of the first
response, the time of the first request (reported apart: it may warm the
engine's own caches) and the p50 / p95 / max in milliseconds of the rest. Each
time includes reading the whole response body. Progress goes to stderr.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request


def _headers() -> dict[str, str]:
    cookie = os.environ.get("BENCH_COOKIE", "").strip()
    bearer = os.environ.get("BENCH_BEARER", "").strip()
    if bool(cookie) == bool(bearer):
        sys.exit("set exactly one of BENCH_COOKIE and BENCH_BEARER")
    if cookie:
        return {"Cookie": cookie}
    return {"Authorization": f"Bearer {bearer}"}


def _percentile(sorted_ms: list[float], p: float) -> float:
    """Nearest-rank percentile of an ascending list."""
    rank = max(1, math.ceil(p / 100 * len(sorted_ms)))
    return sorted_ms[rank - 1]


def _read(url: str, headers: dict[str, str]) -> tuple[float, dict]:
    request = urllib.request.Request(url, headers=headers)
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(request, timeout=90) as response:
            body = response.read()
    except urllib.error.HTTPError as err:
        sys.exit(f"{url} answered HTTP {err.code}")
    elapsed_ms = (time.perf_counter() - started) * 1000
    return elapsed_ms, json.loads(body)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--board", required=True)
    parser.add_argument("--rounds", type=int, default=20)
    args = parser.parse_args()
    if args.rounds < 2:
        sys.exit("--rounds must be at least 2 (one first request, then the rest)")

    base = os.environ.get("API_URL", "").rstrip("/")
    if not base:
        sys.exit("set API_URL")
    url = f"{base}/api/dashboard?{urllib.parse.urlencode({'board': args.board})}"
    headers = _headers()

    first_ms, first = _read(url, headers)
    results = first.get("results", {})
    failed = sorted(k for k, v in results.items() if isinstance(v, dict) and "error" in v)
    print(f"first request {first_ms:.0f} ms, {len(results)} tiles", file=sys.stderr)

    times: list[float] = []
    for i in range(args.rounds - 1):
        elapsed_ms, _ = _read(url, headers)
        times.append(elapsed_ms)
        print(f"round {i + 2}: {elapsed_ms:.0f} ms", file=sys.stderr)
    times.sort()
    print(
        json.dumps(
            {
                "board": args.board,
                "tiles": len(results),
                "tilesWithError": failed,
                "firstMs": round(first_ms),
                "rounds": len(times),
                "p50Ms": round(_percentile(times, 50)),
                "p95Ms": round(_percentile(times, 95)),
                "maxMs": round(times[-1]),
            }
        )
    )


if __name__ == "__main__":
    main()
