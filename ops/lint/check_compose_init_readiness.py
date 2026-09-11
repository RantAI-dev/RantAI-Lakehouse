#!/usr/bin/env python3
"""WS0 items 8-9 (docs/superpowers/plans/2026-09-10-ws0-delta-audit.md):
two compose init/readiness gaps this repo has hit before.

1. The `postgres` service's healthcheck must probe TCP, not the local
   Unix socket. `pg_isready` with no `-h` can report healthy against the
   `postgres:16` image entrypoint's TEMPORARY init-phase server
   (`docker-entrypoint.sh`'s `docker_temp_server_start()`, started with
   `-c listen_addresses=''` — Unix socket only, no TCP), which stops and
   is replaced by the real server (the one every other compose service's
   `postgres:5432` connection actually dials) afterward.
2. `openfga-db-init` must retry its check-then-create `psql` command
   rather than run it exactly once — `depends_on: condition:
   service_healthy` proves `pg_isready` passed, not that the server is
   ready to accept `CREATE DATABASE` from a fresh connection the instant
   that healthcheck first succeeds.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

COMPOSE_PATH = Path(__file__).resolve().parents[2] / "docker-compose.yml"


def check_postgres_healthcheck_uses_tcp(text: str) -> bool:
    match = re.search(r"pg_isready[^\"]*", text)
    if match is None:
        print("check_compose_init_readiness: no pg_isready healthcheck found at all")
        return False
    if "-h" not in match.group(0):
        print(
            "check_compose_init_readiness: pg_isready healthcheck has no -h flag, so it "
            f"can pass against the image's temporary init-phase server: {match.group(0)!r}"
        )
        return False
    return True


def check_openfga_db_init_retries(text: str) -> bool:
    block_match = re.search(r"\n  openfga-db-init:.*?(?=\n  \S)", text, re.S)
    if block_match is None:
        print("check_compose_init_readiness: openfga-db-init service not found")
        return False
    block = block_match.group(0)
    if "for attempt in" not in block:
        print(
            "check_compose_init_readiness: openfga-db-init has no retry loop around its "
            "psql check-then-create command"
        )
        return False
    return True


def main() -> int:
    text = COMPOSE_PATH.read_text(encoding="utf-8")
    ok = check_postgres_healthcheck_uses_tcp(text)
    ok = check_openfga_db_init_retries(text) and ok
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
