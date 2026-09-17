#!/usr/bin/env python3
"""Runs WITHOUT ops/g6/docker-compose.g6.override.yml: proves the
DEFAULT compose file's dagster-code-location (INGEST_ALLOW_INTERNAL_HOSTS
unset) still refuses a link-local target, even though the g6 gate's own
override (used by g6_ingest_matrix_test.py) would admit one."""
from __future__ import annotations

import os
import sys

import requests

API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "ci@example.com")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "ci-password-not-real-123")
API = requests.Session()


def main() -> int:
    login = API.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)
    if not login.ok:
        print(f"[g6-linklocal] login failed: {login.status_code} {login.text}", file=sys.stderr)
        return 1
    if login.json().get("mustChangePassword"):
        API.post(f"{API_URL}/api/auth/change-password", json={"newPassword": AUTH_PASSWORD}, timeout=10)
        API.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)

    created = API.post(
        f"{API_URL}/api/connectors",
        json={"name": "g6-linklocal", "type": "REST API", "direction": "source", "host": "169.254.169.254",
              "secretRef": "env:CONNECTOR_REST_API_KEY", "environment": "production", "tenant": "g6",
              "residency": "", "capabilities": []},
        timeout=10,
    )
    connector_id = created.json()["id"]
    API.put(
        f"{API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={"adapter": "rest", "ingestMode": "batch",
              "dial": {"baseUrl": "http://169.254.169.254", "auth": {"type": "bearer"},
                       "pagination": {"type": "none"}, "endpoints": [{"path": "/", "dataPath": None}]},
              "sourceObjects": []},
        timeout=10,
    )
    run_resp = API.post(f"{API_URL}/api/connectors/{connector_id}/ingest/run", timeout=10)
    run_id = run_resp.json().get("runId")
    if not run_id:
        print(f"[g6-linklocal] no runId launched: {run_resp.text}", file=sys.stderr)
        return 1
    # The launched run itself fails at dial time
    # (dagster/dispar_orchestrate/ssrf_guard.py, INGEST_ALLOW_INTERNAL_HOSTS
    # unset by default) -- check the recorded ingest_run row, not the
    # launch response (which only proves the JOB was accepted, not that
    # the DIAL was refused).
    runs_resp = API.get(f"{API_URL}/api/governance/ingest-runs", params={"connectorId": connector_id}, timeout=30)
    rows = runs_resp.json()
    rejected = [r for r in rows if r.get("status") == "rejected"]
    if not rejected:
        print(f"[g6-linklocal] expected a rejected ingest_run row for a link-local target, got: {rows}", file=sys.stderr)
        return 1
    print("[g6-linklocal] PASS: link-local target rejected by the DEFAULT (override-free) compose config")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
