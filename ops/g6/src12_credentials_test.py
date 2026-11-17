"""SRC-12 part A gate: a credential typed in the console is stored encrypted,
the connector still tests and loads with it, and it survives a key change.

What it proves (feature page `credentials-done-properly.md`, decisions 4, 5;
spec `SRC-12-AC2`):

before  (the stack runs with the CI key)
  1. A MySQL connector is created with a typed (managed) password. The file
     the API wrote on the credentials volume is read from the gate's own
     read-only mount: it is not the plain value, does not contain it, starts
     with the format's version byte, and is owner-only.
  2. `POST .../test` passes and a run loads the table into Bronze: the
     orchestrator opens the sealed file with the same key.
(the CI job then runs `lakehouse-api rotate-secrets-key` with a new key and
the old one as `CONNECTOR_SECRETS_KEY_PREVIOUS`, removes the previous key and
recreates the API and the code location)
after
  3. The stored file changed (it is re-sealed), is still not the plain value,
     the connector tests again and a second run loads again with the new key
     alone: the rows double (`append`).

The refusal to start without the key is checked by the CI job itself
(`docker compose config` with the variable empty must fail and name it).

The stack is `ops/g6/docker-compose.g6.override.yml` plus
`ops/g6/docker-compose.src12.override.yml`, which gives `lakehouse-api` the
WRITABLE credentials volume the other g6 gates replace with a read-only one,
and this runner a read-only view of it at `/managed-secrets`. State between
the two phases is kept in `/gate-secrets/src12-state.json` (the g6 volume, a
name no credential ref can have).

Usage: `python src12_credentials_test.py before|after`. Exit 0 and
`[src12] PASS (<phase>)`, or `[src12] FAILED: ...` and exit 1.
"""
from __future__ import annotations

import hashlib
import json
import os
import secrets
import stat
import sys

import requests

import g6_ingest_matrix_test as matrix
import src11_connector_operations_test as ops

GATE = "src12"
MANAGED_DIR = "/managed-secrets"
STATE_PATH = "/gate-secrets/src12-state.json"
FORMAT_VERSION = 1
MIN_SEALED_LEN = 1 + 12 + 16


class CredentialGateFailure(Exception):
    """A step of this gate failed."""


def say(line: str) -> None:
    print(f"[{GATE}] {line}", flush=True)


def stored_file(connector_ref: str) -> tuple[str, bytes, int]:
    """The managed file behind the derived ref `connector_ref`: (name, bytes,
    mode). The name comes from the server's own response and must be a
    `file:/run/secrets/connector_managed_*` ref, never a path built from
    anything else."""
    prefix = "file:/run/secrets/connector_managed_"
    if not connector_ref.startswith(prefix):
        raise CredentialGateFailure(f"the connector's credential is not a managed file ref: {connector_ref!r}")
    name = connector_ref.rsplit("/", 1)[-1]
    path = os.path.join(MANAGED_DIR, name)
    if not os.path.isfile(path):
        raise CredentialGateFailure(f"{name} is not on the credentials volume")
    with open(path, "rb") as fh:
        data = fh.read()
    return name, data, stat.S_IMODE(os.stat(path).st_mode)


def check_sealed(data: bytes, plain: str, what: str) -> None:
    """The file is a seal, not the credential."""
    if data == plain.encode() or plain.encode() in data:
        raise CredentialGateFailure(f"{what}: the stored file holds the plain credential")
    if len(data) < MIN_SEALED_LEN + len(plain) or data[0] != FORMAT_VERSION:
        raise CredentialGateFailure(f"{what}: the stored file is not in the sealed format (version byte, nonce, ciphertext)")


def connection_test_passes(connector_id: str, what: str) -> None:
    resp = matrix.API.post(f"{matrix.API_URL}/api/connectors/{connector_id}/test", timeout=30)
    body = ops.expect_ok(resp, f"POST test ({what})")
    if body.get("supported") is not True or body.get("ok") is not True:
        raise CredentialGateFailure(f"{what}: the connection test should pass, got {body}")


def load_and_count(connector_id: str, bronze: str, want_rows: int, what: str) -> None:
    run_id = ops.launch_run(connector_id)
    ops.wait_dagster(run_id, "SUCCESS", what)
    rows = ops.bronze_rows(bronze)
    if rows != want_rows:
        raise CredentialGateFailure(f"{what}: bronze.{bronze} has {rows} rows, expected {want_rows}")


def phase_before() -> None:
    ops.matrix.step_wait_for_services()
    matrix.step_login()
    matrix.step_ensure_tenant()
    sfx = secrets.token_hex(3)
    table, bronze = f"g12_cred_{sfx}", f"g6_src12_{sfx}"
    matrix._src8_mysql(
        f"DROP TABLE IF EXISTS {table}",
        f"CREATE TABLE {table} (id INT PRIMARY KEY, name VARCHAR(50))",
        f"INSERT INTO {table} (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'c')",
    )
    plain = matrix.CONNECTOR_MYSQL_PASSWORD
    body = matrix._create_connector(
        name=f"src12-cred-{sfx}", kind="MySQL", host="mysql-g6:3306",
        credential={"source": "managed", "primary": "password", "values": {"primary": plain}},
    )
    connector_id = body["id"]
    ref = (body.get("credential") or {}).get("primary")
    if not ref:
        raise CredentialGateFailure(f"the create response carried no credential name: {body}")
    ops.expect_ok(ops.put_mysql_spec(connector_id, "mysql-g6", 3306, table, bronze), "PUT ingest-spec")

    name, data, mode = stored_file(ref)
    check_sealed(data, plain, "after create")
    if mode & 0o077:
        raise CredentialGateFailure(f"the stored file is readable by others (mode {mode:o})")
    say(f"1. {name} is sealed ({len(data)} bytes, mode {mode:o}); the plain value is not in it")

    connection_test_passes(connector_id, "before the key change")
    load_and_count(connector_id, bronze, 3, "2. first load")
    say("2. the connection test passes and the orchestrator loaded 3 rows through the sealed file")

    state = {"connectorId": connector_id, "ref": ref, "bronze": bronze, "sha256": hashlib.sha256(data).hexdigest(), "rows": 3}
    with open(STATE_PATH, "w", encoding="utf-8") as fh:
        json.dump(state, fh)


def phase_after() -> None:
    with open(STATE_PATH, encoding="utf-8") as fh:
        state = json.load(fh)
    ops.matrix.step_wait_for_services()
    matrix.step_login()
    plain = matrix.CONNECTOR_MYSQL_PASSWORD
    name, data, _ = stored_file(state["ref"])
    check_sealed(data, plain, "after the key change")
    if hashlib.sha256(data).hexdigest() == state["sha256"]:
        raise CredentialGateFailure(f"{name} is byte-for-byte what it was before the key change: it was not re-sealed")
    say(f"3. {name} was re-sealed (different bytes), still not the plain value")
    connection_test_passes(state["connectorId"], "after the key change")
    load_and_count(state["connectorId"], state["bronze"], state["rows"] * 2, "3. load with the new key alone")
    say("3. the connection test passes and a second load (append) doubled the rows under the new key alone")


def main() -> int:
    phase = sys.argv[1] if len(sys.argv) > 1 else ""
    if phase not in ("before", "after"):
        print(f"[{GATE}] FAILED: usage: src12_credentials_test.py before|after", file=sys.stderr)
        return 1
    try:
        (phase_before if phase == "before" else phase_after)()
    except (CredentialGateFailure, ops.OpsGateFailure, matrix.G6Failure) as exc:
        print(f"[{GATE}] FAILED: {phase}: {exc}", file=sys.stderr)
        return 1
    except requests.RequestException as exc:
        print(f"[{GATE}] FAILED: {phase}: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1
    except Exception:  # noqa: BLE001 -- any other error is a failure of the gate and must be named as one
        import traceback
        print(f"[{GATE}] FAILED: {phase}: unexpected error:\n{traceback.format_exc()}", file=sys.stderr)
        return 1
    print(f"[{GATE}] PASS ({phase})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
