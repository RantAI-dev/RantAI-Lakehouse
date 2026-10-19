#!/usr/bin/env python3
"""SRC-3 gate, SaaS connectors (`g6-saas`, plan AD9, T1.12): the skeleton.

Slice 1 has no vendor behind the adapter, so the gate proves the parts that
exist and are shared:

1. the mock vendor server (`saas_stub.py`) is up and serves `/health`;
2. through the API, as the console does, `GET /api/connectors/types` lists the
   ten business-app types, every one `supported: false`, adapter `saas`, with a
   reason (honestly unsupported until its own gate step exists: AD10 and
   `AGENTS.md` principle 2);
3. no type that is not one of the ten has the `saas` adapter.

Each vendor adds its steps in its own slice (connect, discover, load, reload,
schema change, revoked sign-in), and `0096` (slice 7) flips the types whose
steps pass. The lines are printed with the prefix `[saas]` and copied to the job
summary; the job's time is the measurement against the signed 20 minutes.

Settings: `LAKEHOUSE_API_URL`, `AUTH_BOOTSTRAP_EMAIL`, `AUTH_BOOTSTRAP_PASSWORD`
(as `g6_ingest_matrix_test.py` reads them; its login helpers are imported),
`SAAS_STUB_URL`. Exit code 0 and `[saas] PASS`, or `[saas] FAILED: ...` on
stderr and exit code 1.
"""
from __future__ import annotations

import os
import sys
import time

import g6_ingest_matrix_test as matrix

STUB_URL = os.environ.get("SAAS_STUB_URL", "http://saas-stub-g6:8080")

# The names `0095_saas_connector_types.sql` inserts, and the vendor ids of
# `ops/fixtures/saas_vendors.json` they stand for.
SAAS_TYPES = (
    "Salesforce",
    "ServiceNow",
    "Workday Reports",
    "Google Analytics 4",
    "HubSpot",
    "Zendesk",
    "Confluence",
    "Dynamics 365",
    "NetSuite",
    "SharePoint",
)


class SaasGateFailure(Exception):
    pass


def _log(message: str) -> None:
    print(f"[saas] {message}", flush=True)


def step_stub_is_up() -> None:
    matrix._wait_for("saas-stub-g6", lambda: matrix.API.get(f"{STUB_URL}/health", timeout=5).status_code == 200, 60)
    _log("the mock vendor server answers /health")


def step_types_are_listed_unsupported() -> None:
    resp = matrix.API.get(f"{matrix.API_URL}/api/connectors/types", timeout=15)
    if not resp.ok:
        raise SaasGateFailure(f"GET /api/connectors/types answered {resp.status_code}")
    types = {t["name"]: t for t in resp.json()}
    missing = [name for name in SAAS_TYPES if name not in types]
    if missing:
        raise SaasGateFailure(f"the connector types do not list {missing}")
    for name in SAAS_TYPES:
        found = types[name]
        if found.get("adapter") != "saas":
            raise SaasGateFailure(f"{name}: adapter {found.get('adapter')!r}, expected 'saas'")
        if found.get("supported") is not False:
            raise SaasGateFailure(f"{name}: supported {found.get('supported')!r}; slice 1 has no gate step for it yet")
        if not (found.get("unsupportedReason") or "").strip():
            raise SaasGateFailure(f"{name}: unsupported with no reason")
    others = sorted(t["name"] for t in types.values() if t.get("adapter") == "saas" and t["name"] not in SAAS_TYPES)
    if others:
        raise SaasGateFailure(f"types outside the ten have the saas adapter: {others}")
    _log(f"the ten business-app types are listed, unsupported, with a reason: {', '.join(SAAS_TYPES)}")


def run() -> None:
    matrix._wait_for("lakehouse-api", lambda: matrix.API.get(f"{matrix.API_URL}/health", timeout=5).status_code == 200, 120)
    matrix.step_login()
    matrix.step_ensure_tenant()
    step_stub_is_up()
    step_types_are_listed_unsupported()


def main() -> int:
    sys.stdout.reconfigure(line_buffering=True)
    sys.stderr.reconfigure(line_buffering=True)
    started = time.time()
    try:
        run()
    except (SaasGateFailure, matrix.G6Failure) as exc:
        print(f"[saas] FAILED: {exc}", file=sys.stderr)
        return 1
    except Exception:  # noqa: BLE001 -- any other error is a failure of the gate and must be named as one
        import traceback

        print(f"[saas] FAILED: unexpected error:\n{traceback.format_exc()}", file=sys.stderr)
        return 1
    _log(f"PASS ({time.time() - started:.0f} s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
