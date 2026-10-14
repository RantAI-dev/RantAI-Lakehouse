#!/usr/bin/env python3
"""Catalog search benchmark (DATA-11, decision 1): how long a search takes on a
catalog of 10,000 tables with 20 columns each, against the target of 500 ms
in `docs/core/features/catalog-search.md`.

Plan: `docs/superpowers/plans/2026-10-09-data-11-catalog-search.md`, task T4.
The planner writes `docs/plans/DATA-11-RESULT.md` from this script's output;
this file measures, it states no verdict.

# Never point this at a shared stack

`seed` WRITES synthetic rows into the two Bronze registries
(`bronze_meta` and `bronze_meta_sec`: `dataset_catalog`, `dataset_sync`,
`dataset_column`) of the ClickHouse named by the environment, and they are
read by every user of that catalog. Run it only against a throwaway compose
project that is deleted afterwards. It refuses to run unless
`BENCH_THROWAWAY_CATALOG=true` is set, and that variable is a promise by the
person running it, not a check the script can make. There is no cleanup
command: the stack is the cleanup.

# Commands

- `seed [--datasets N] [--columns M]`: N datasets (default 10,000) of M columns
  (default 20), half in each registry. Dataset `i` is `bench-dataset-<i>`,
  titled "Benchmark revenue table <i>", with columns such as
  `bench_col_07_amount`. Their `table_name` is empty, so nothing reads a
  Bronze table that does not exist. The rows are written through ClickHouse's
  HTTP interface into the Iceberg registries; that this INSERT is accepted on
  every deployment was NOT verified when the script was written -- a refusal
  surfaces as the HTTP error and nothing is retried or faked.
- `run [--rounds 50]`: signs in, then sends `--rounds` searches (default 50) to
  `GET /api/catalog?q=` and as many to `GET /api/catalog/query?search=`,
  rotating over four kinds: a table name, a column name, a one-letter typo and
  two words in reverse order. For each route it prints the first search's time
  (it builds the search copy, so `run` first waits `--cold-wait` seconds, 35
  by default, for any copy to pass its 30-second age; the wait is printed)
  and the p50 / p95 / max of the rest. Each time includes reading the whole
  response body. Output is one JSON object on stdout so the raw numbers go
  into the result document unedited; progress goes to stderr.

# Settings (environment)

- `BENCH_THROWAWAY_CATALOG`: `true` to let `seed` write. Required by `seed`.
- `BENCH_CLICKHOUSE_URL`: ClickHouse HTTP endpoint for `seed`. Required.
- `CLICKHOUSE_USER`, `CLICKHOUSE_PASSWORD`: its credentials. User defaults to
  `default`; the password may be empty only if the server allows it.
- `LAKEHOUSE_API_URL`: the API's base URL for `run`. Required.
- `AUTH_BOOTSTRAP_EMAIL`, `AUTH_BOOTSTRAP_PASSWORD`: an account that holds
  `catalog:read`. Required by `run`; there is no default credential.
- `BENCH_TENANT_ID`: a tenant the account belongs to, sent as `X-Tenant`.
  Optional.

Exit code 0 on success; on any failure a message on stderr and exit code 1.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import statistics
import sys
import time
from dataclasses import dataclass

import requests

REQUEST_TIMEOUT_S = 60
# Rows per INSERT: keeps one request body to a few MB.
INSERT_CHUNK_ROWS = 20_000
# One fixed seed, so two runs search for the same terms.
TERM_SEED = 11
WORDS = ["amount", "region", "customer", "status", "created", "total", "code", "price"]
REGISTRIES = ["bronze_meta", "bronze_meta_sec"]


class BenchFailure(Exception):
    """A setting is missing or the stack answered something unusable."""


@dataclass(frozen=True)
class Settings:
    throwaway: bool
    clickhouse_url: str
    clickhouse_user: str
    clickhouse_password: str
    api_url: str
    email: str
    password: str
    tenant_id: str

    @classmethod
    def from_env(cls) -> Settings:
        env = os.environ
        return cls(
            throwaway=env.get("BENCH_THROWAWAY_CATALOG", "") == "true",
            clickhouse_url=env.get("BENCH_CLICKHOUSE_URL", "").rstrip("/"),
            clickhouse_user=env.get("CLICKHOUSE_USER", "default"),
            clickhouse_password=env.get("CLICKHOUSE_PASSWORD", ""),
            api_url=env.get("LAKEHOUSE_API_URL", "").rstrip("/"),
            email=env.get("AUTH_BOOTSTRAP_EMAIL", ""),
            password=env.get("AUTH_BOOTSTRAP_PASSWORD", ""),
            tenant_id=env.get("BENCH_TENANT_ID", ""),
        )


def column_name(j: int) -> str:
    return f"bench_col_{j:02d}_{WORDS[j % len(WORDS)]}"


def _insert(settings: Settings, session: requests.Session, table: str, columns: str, rows: list[dict]) -> None:
    """One `INSERT ... FORMAT JSONEachRow`. `table` and `columns` are constants
    of this file, never input; the values travel in the body as JSON."""
    for start in range(0, len(rows), INSERT_CHUNK_ROWS):
        chunk = rows[start : start + INSERT_CHUNK_ROWS]
        body = "\n".join(json.dumps(r) for r in chunk)
        resp = session.post(
            settings.clickhouse_url,
            params={"query": f"INSERT INTO lake.`{table}` ({columns}) FORMAT JSONEachRow"},
            data=body.encode(),
            auth=(settings.clickhouse_user, settings.clickhouse_password),
            timeout=REQUEST_TIMEOUT_S,
        )
        if not resp.ok:
            raise BenchFailure(f"insert into {table} failed: {resp.status_code} {resp.text[:300]}")


def seed(settings: Settings, datasets: int, columns: int) -> None:
    if not settings.throwaway:
        raise BenchFailure(
            "seed writes into the catalog registries; set BENCH_THROWAWAY_CATALOG=true only "
            "for a throwaway compose project, never a shared stack"
        )
    if not settings.clickhouse_url:
        raise BenchFailure("BENCH_CLICKHOUSE_URL is not set")
    session = requests.Session()
    now = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    for index, registry in enumerate(REGISTRIES):
        mine = range(index, datasets, len(REGISTRIES))
        catalog, sync, cols = [], [], []
        for i in mine:
            slug = f"bench-dataset-{i}"
            catalog.append(
                {
                    "slug": slug,
                    "title": f"Benchmark revenue table {i}",
                    "description": f"Synthetic table {i} for the catalog search benchmark",
                    "tier": "public",
                    "updated_at": now,
                    "table_name": "",
                }
            )
            sync.append({"slug": slug, "total": 1000 + i, "author": "bench", "frekuensi": "daily"})
            for j in range(columns):
                cols.append(
                    {
                        "slug": slug,
                        "key_asli": column_name(j),
                        "tipe": "String",
                        "deskripsi": f"{WORDS[j % len(WORDS)]} of row",
                    }
                )
        _insert(settings, session, f"{registry}.dataset_catalog", "slug, title, description, tier, updated_at, table_name", catalog)
        _insert(settings, session, f"{registry}.dataset_sync", "slug, total, author, frekuensi", sync)
        _insert(settings, session, f"{registry}.dataset_column", "slug, key_asli, tipe, deskripsi", cols)
        print(f"[bench] {registry}: {len(catalog)} datasets, {len(cols)} columns", file=sys.stderr)


def terms(rounds: int, datasets: int) -> list[str]:
    """`rounds` terms, rotating name / column name / typo / two words reversed."""
    rng = random.Random(TERM_SEED)
    out: list[str] = []
    for n in range(rounds):
        kind = n % 4
        i = rng.randrange(datasets)
        if kind == 0:
            out.append(f"benchmark revenue table {i}")
        elif kind == 1:
            out.append(column_name(rng.randrange(20)))
        elif kind == 2:
            # One letter dropped from "revenue": inside the one-edit rule.
            out.append("revnue")
        else:
            out.append(f"table {WORDS[rng.randrange(len(WORDS))]}")
    return out


def _timed_get(api: requests.Session, url: str, params: dict) -> float:
    start = time.perf_counter()
    resp = api.get(url, params=params, timeout=REQUEST_TIMEOUT_S)
    body = resp.content  # the whole body is part of what a user waits for
    elapsed = time.perf_counter() - start
    resp.raise_for_status()
    if b'"supported":false' in body.replace(b" ", b""):
        raise BenchFailure(f"{url} refused the caller (supported: false); check the tenant and permissions")
    return elapsed * 1000.0


def _summary(first_ms: float, rest_ms: list[float]) -> dict:
    ordered = sorted(rest_ms)
    p95_index = max(0, min(len(ordered) - 1, round(0.95 * len(ordered)) - 1))
    return {
        "firstMs": round(first_ms, 1),
        "searches": len(rest_ms),
        "p50Ms": round(statistics.median(ordered), 1),
        "p95Ms": round(ordered[p95_index], 1),
        "maxMs": round(ordered[-1], 1),
    }


def run(settings: Settings, rounds: int, datasets: int, cold_wait: float) -> dict:
    if not (settings.api_url and settings.email and settings.password):
        raise BenchFailure("LAKEHOUSE_API_URL, AUTH_BOOTSTRAP_EMAIL and AUTH_BOOTSTRAP_PASSWORD must be set")
    if rounds < 2:
        raise BenchFailure("--rounds must be at least 2 (one first search and one more)")
    api = requests.Session()
    login = api.post(
        f"{settings.api_url}/api/auth/login",
        json={"email": settings.email, "password": settings.password},
        timeout=10,
    )
    if not login.ok:
        raise BenchFailure(f"login failed: {login.status_code}")
    if settings.tenant_id:
        api.headers["X-Tenant"] = settings.tenant_id

    routes = {
        "catalog": (f"{settings.api_url}/api/catalog", "q", {}),
        "catalogQuery": (
            f"{settings.api_url}/api/catalog/query",
            "search",
            {"page": 1, "pageSize": 50, "skipListMeta": "true"},
        ),
    }
    searches = terms(rounds, datasets)
    print(f"[bench] waiting {cold_wait:.0f}s so the search copy is older than its age", file=sys.stderr)
    time.sleep(cold_wait)
    result: dict = {"rounds": rounds, "datasetsAssumed": datasets, "coldWaitSeconds": cold_wait}
    for name, (url, key, extra) in routes.items():
        times = [_timed_get(api, url, {key: term, **extra}) for term in searches]
        result[name] = _summary(times[0], times[1:])
        print(f"[bench] {name}: {result[name]}", file=sys.stderr)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p_seed = sub.add_parser("seed", help="write synthetic datasets into a THROWAWAY catalog")
    p_seed.add_argument("--datasets", type=int, default=10_000)
    p_seed.add_argument("--columns", type=int, default=20)
    p_run = sub.add_parser("run", help="time searches against the API")
    p_run.add_argument("--rounds", type=int, default=50)
    p_run.add_argument("--datasets", type=int, default=10_000, help="how many were seeded; only picks table numbers")
    p_run.add_argument("--cold-wait", type=float, default=35.0)
    args = parser.parse_args()
    settings = Settings.from_env()
    try:
        if args.command == "seed":
            seed(settings, args.datasets, args.columns)
        else:
            print(json.dumps(run(settings, args.rounds, args.datasets, args.cold_wait), indent=2))
    except (BenchFailure, requests.RequestException) as err:
        print(f"[bench] FAILED: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
