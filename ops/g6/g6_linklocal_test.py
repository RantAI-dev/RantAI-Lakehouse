#!/usr/bin/env python3
"""Runs WITHOUT ops/g6/docker-compose.g6.override.yml (INGEST_ALLOW_INTERNAL_HOSTS
stays unset): proves the DEFAULT compose file's dagster-code-location
still refuses a link-local target, even though the g6 gate's own override
(used by g6_ingest_matrix_test.py) would admit one.

Also proves the SAME guard against a Kafka connector whose BOOTSTRAP host
is real and reachable but whose cluster metadata ADVERTISES a private,
internal address (`kafka-g6-spoofed`, `internal-only-g6:9092` --
docker-compose.yml, `ssrf_guard_kafka.check_all_advertised_brokers`).

# Credential shape (ADR 0002 Addendum 3)

`POST /api/connectors` no longer accepts a client-chosen `secretRef` (see
g6_ingest_matrix_test.py's module docstring for the full ADR 0002
Addendum 3 shape). The REST connector below needs a REAL, resolvable
credential, not merely a syntactically valid one: `ingest_factory.py::
_resolve_object_secrets` resolves `secretRef` BEFORE the adapter's own
SSRF-guarded dial ever runs, so an unresolvable credential would produce
its OWN `status="rejected"` ingest_run row (`SecretRefRejected`) --
indistinguishable, by this script's own assertion, from the link-local
refusal this test exists to prove. A `file:`-scheme credential is the
only shape that can be provisioned AFTER this connector's server-generated
id exists (an `env:`-scheme name can never be pre-set, since it is
derived from that same unpredictable id) -- hence the SECOND, narrower
override this invocation adds on top of the default compose file,
`ops/g6/docker-compose.g6-linklocal-secrets.override.yml`, which does
nothing except make that one credential resolvable; see that file's own
header for why it does not weaken the property under test.

The Kafka connector's `auth.type = "none"` needs no credential value at
all (see g6_ingest_matrix_test.py's docstring), so it is registered with
a syntactically valid but never-written `credential`.
"""
from __future__ import annotations

import os
import sys
import time

import requests

API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "ci@example.com")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "ci-password-not-real-123")
API = requests.Session()

GATE_SECRETS_DIR = "/gate-secrets"
FILE_REF_PREFIX = "file:/run/secrets/connector_"


def _write_credential_file(ref_name: str, value: str) -> None:
    """Duplicated from g6_ingest_matrix_test.py -- see this repository's
    existing precedent (this module's own docstring) of not sharing code
    between standalone gate scripts."""
    if not ref_name.startswith(FILE_REF_PREFIX):
        raise SystemExit(f"[g6-linklocal] refusing to write a credential file for an unexpected ref shape: {ref_name!r}")
    basename = ref_name.rsplit("/", 1)[-1]
    path = os.path.join(GATE_SECRETS_DIR, basename)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(value)
    os.chmod(path, 0o644)


def _login() -> None:
    login = API.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)
    if not login.ok:
        raise SystemExit(f"[g6-linklocal] login failed: {login.status_code} {login.text}")
    if login.json().get("mustChangePassword"):
        API.post(f"{API_URL}/api/auth/change-password", json={"newPassword": AUTH_PASSWORD}, timeout=10)
        API.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)


def _assert_rejected(*, connector_id: str, label: str) -> None:
    """Polls `/api/governance/ingest-runs`, not a single immediate GET:
    `record_ingest_run` (`dagster/dispar_orchestrate/bronze_catalog.py`)
    writes `lake.bronze_meta.ingest_run` lazily, asynchronously, from
    inside the launched Dagster run -- a fresh stack's `lake` ClickHouse
    database does not exist until the FIRST such write anywhere in this
    stack completes. An immediate GET right after `POST .../ingest/run`
    returns before that write has happened, surfacing as `503 Database
    lake does not exist` (reproduced against this stack) -- a real race,
    not a "not rejected" result, so it is retried here rather than
    treated as a failure."""
    deadline = time.time() + 60
    last_rows = None
    while time.time() < deadline:
        runs_resp = API.get(f"{API_URL}/api/governance/ingest-runs", params={"connectorId": connector_id}, timeout=30)
        if runs_resp.ok:
            rows = runs_resp.json()
            last_rows = rows
            rejected = [r for r in rows if r.get("status") == "rejected"]
            if rejected:
                print(f"[g6-linklocal] PASS: {label} rejected by the DEFAULT (override-free) compose config")
                return
        time.sleep(3)
    raise SystemExit(f"[g6-linklocal] {label}: expected a rejected ingest_run row, got: {last_rows}")


def step_rest_link_local() -> None:
    created = API.post(
        f"{API_URL}/api/connectors",
        json={"name": "g6-linklocal", "type": "REST API", "direction": "source", "host": "169.254.169.254",
              "credential": {"source": "file", "primary": "token"},
              "environment": "production", "tenant": "g6", "residency": "", "capabilities": []},
        timeout=10,
    )
    if not created.ok:
        raise SystemExit(f"[g6-linklocal] create connector failed: {created.status_code} {created.text}")
    body = created.json()
    connector_id = body["id"]
    derived = (body.get("credential") or {}).get("primary")
    if not derived:
        raise SystemExit(f"[g6-linklocal] create response carried no derived credential name: {body}")
    # A real, resolvable value -- see this module's docstring for why a
    # credential that cannot resolve would make this test pass for the
    # wrong reason.
    _write_credential_file(derived, "g6-linklocal-bearer-token-not-real")

    # No "recordsPath" -- this step never needs nested extraction at
    # all, since the link-local target is refused before any response
    # body is parsed.
    #
    # sourceObjects has ONE entry, not `[]` -- verification correction,
    # same bug and same fix as g6_ingest_matrix_test.py's g6-rest
    # connector: `ingest_factory.py::run_ingest` never even calls
    # `_run_one_object` for an empty `sourceObjects` list, so with `[]`
    # here this run "succeeded" in under 3 seconds WITHOUT EVER DIALING
    # 169.254.169.254 -- the SSRF guard was never exercised at all
    # (reproduced against this stack: `RUN_SUCCESS` logged immediately
    # after `RESOURCE_INIT_SUCCESS`, no extract/load step activity). The
    # entry's content doesn't matter to the REST adapter (it ignores
    # `source_objects`); this only needs to make the loop iterate once.
    spec_resp = API.put(
        f"{API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={"adapter": "rest", "ingestMode": "batch",
              "dial": {"baseUrl": "http://169.254.169.254", "auth": {"type": "bearer"},
                       "pagination": {"type": "none"}, "endpoints": [{"path": "/"}]},
              "sourceObjects": [{"name": "root", "target": "g6_linklocal_items"}]},
        timeout=10,
    )
    if not spec_resp.ok:
        raise SystemExit(f"[g6-linklocal] set ingest-spec failed: {spec_resp.status_code} {spec_resp.text}")
    run_resp = API.post(f"{API_URL}/api/connectors/{connector_id}/ingest/run", timeout=10)
    run_id = run_resp.json().get("runId")
    if not run_id:
        raise SystemExit(f"[g6-linklocal] no runId launched: {run_resp.text}")
    # The launched run itself fails at dial time
    # (dagster/dispar_orchestrate/ssrf_guard.py, INGEST_ALLOW_INTERNAL_HOSTS
    # unset by default) -- check the recorded ingest_run row, not the
    # launch response (which only proves the JOB was accepted, not that
    # the DIAL was refused).
    _assert_rejected(connector_id=connector_id, label="link-local REST target")


def step_kafka_spoofed_advertised_broker() -> None:
    """Bootstraps at `kafka-g6-spoofed:9092` -- a real, legitimately
    resolvable compose service, never itself checked by
    `ssrf_guard_kafka` (the BOOTSTRAP host is operator-supplied; only the
    cluster's ADVERTISED broker list is server-supplied and therefore
    guarded, see that module's doc comment). That broker's cluster
    metadata advertises `internal-only-g6:9092`, a network alias
    resolving to this SAME container's own (private, RFC1918) address --
    `check_all_advertised_brokers` refuses it before `adapters/kafka.py`
    ever calls `.poll()`."""
    created = API.post(
        f"{API_URL}/api/connectors",
        json={"name": "g6-kafka-spoofed", "type": "Kafka", "direction": "source", "host": "kafka-g6-spoofed:9092",
              "credential": {"source": "file", "primary": "token"},
              "environment": "production", "tenant": "g6", "residency": "", "capabilities": []},
        timeout=10,
    )
    if not created.ok:
        raise SystemExit(f"[g6-linklocal] create kafka-spoofed connector failed: {created.status_code} {created.text}")
    connector_id = created.json()["id"]
    # auth.type="none" needs no credential value -- see this module's
    # docstring; nothing is written for the derived name here.

    spec_resp = API.put(
        f"{API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={"adapter": "kafka", "ingestMode": "stream",
              "dial": {"bootstrapServers": ["kafka-g6-spoofed:9092"], "topic": "g6-spoofed-events",
                       "auth": {"type": "none"}, "groupId": "g6-gate-kafka-spoofed", "microBatchSeconds": 10},
              "sourceObjects": [{"name": "g6-spoofed-events", "target": "g6_kafka_spoofed_events"}]},
        timeout=10,
    )
    if not spec_resp.ok:
        raise SystemExit(f"[g6-linklocal] set ingest-spec for kafka-spoofed failed: {spec_resp.status_code} {spec_resp.text}")
    run_resp = API.post(f"{API_URL}/api/connectors/{connector_id}/ingest/run", timeout=10)
    run_id = run_resp.json().get("runId")
    if not run_id:
        raise SystemExit(f"[g6-linklocal] no runId launched for kafka-spoofed: {run_resp.text}")
    # `run_kafka_stream_batch` now records an ingest_run row for this
    # outcome (`4e9eddd` -- `check_all_advertised_brokers` raising
    # `ssrf_guard.SsrfBlocked` from inside the micro-batch poll records
    # `status="rejected"`, mirroring every batch adapter's own
    # `_run_one_object` rejection path), so this asserts the SAME way
    # `step_rest_link_local` does, not by polling the Dagster run's own
    # status.
    _assert_rejected(connector_id=connector_id, label="kafka advertised-broker spoof")


def main() -> int:
    _login()
    try:
        step_rest_link_local()
        step_kafka_spoofed_advertised_broker()
    except SystemExit as exc:
        print(exc, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
