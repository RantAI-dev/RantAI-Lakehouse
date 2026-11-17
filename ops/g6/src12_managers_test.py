"""SRC-12 part B gate: a connector whose credential is a REFERENCE to a secret
in the installation's secret manager (Vault, or AWS Secrets Manager through a
`moto` stand-in) tests and loads, follows a change made in the manager at the
next run, and leaves no copy on the credentials volume.

What it proves (feature page `credentials-done-properly.md`, decisions 1, 2,
3, 9, 12; spec `SRC-12-AC1`, `SRC-12-AC4`), for one manager per run of this
file (the installation has one manager at a time; the CI job recreates the API
and the code location between the two):

  1. A reference to a secret that does not exist is refused (422) with the
     fixed text "does not exist in the secret manager" and none of the
     manager's own words; a reference of another manager's scheme is refused
     (400) as not allowed here.
     A reference outside `SECRET_MANAGER_ALLOWED_PREFIXES` (and one with `..`)
     is refused (400, `SRC-12 review B1 BLOCKER 1`).
  2. A connector is created with nothing stored, its password is put in the
     manager, and `PUT .../credential` with the reference is accepted
     (verified: the API read the reference once and the connection worked).
     The connector reports its credential as a reference.
  3. The volume holds no copy: no file for this connector on the credentials
     volume, and the plain password is in no file there.
  4. The connection test passes and a run loads 3 rows (the orchestrator read
     the reference itself).
  5. The secret is changed in the manager to a wrong password: the next test
     fails and the next run fails (nothing was kept); changed back, the next
     run succeeds and appends (6 rows). Decision 9: a reference is read fresh
     each time.

Stack: `ops/g6/docker-compose.g6.override.yml`, `ops/g6/docker-compose.src12.override.yml`
and `ops/g6/docker-compose.src12-managers.override.yml` (Vault's official
development server `vault-g6`, a `moto` server `moto-g6`, and the managers'
settings for the API and the code location). The writes to the manager are
the gate's own, through Vault's HTTP interface and `boto3`.

Usage: `python src12_managers_test.py vault|aws`. Exit 0 and
`[src12m] PASS (<manager>)`, or `[src12m] FAILED: ...` and exit 1.
"""
from __future__ import annotations

import json
import os
import secrets
import sys

import requests

import g6_ingest_matrix_test as matrix
import src11_connector_operations_test as ops

GATE = "src12m"
MANAGED_DIR = "/managed-secrets"
SECRET_FIELD = "password"
# Word for word `refs::OUTSIDE_PREFIXES_TEXT` (and `secret_manager.POLICY_TEXTS`).
OUTSIDE_PREFIXES_TEXT = "That reference is outside the secrets this installation allows connectors to reference"

# Words a manager uses in its own error bodies; none may reach the user.
MANAGER_WORDS = ("ResourceNotFoundException", "Secrets Manager can", "permission denied", "no handler for route", "vault")


class ManagerGateFailure(Exception):
    """A step of this gate failed."""


def say(line: str) -> None:
    print(f"[{GATE}] {line}", flush=True)


class VaultStore:
    """Vault's development server, KV version 2 at `secret/`."""

    kind = "vault"

    def __init__(self) -> None:
        self.addr = os.environ["VAULT_ADDR"].rstrip("/")
        self.token = os.environ["VAULT_TOKEN"]

    def wait_ready(self) -> None:
        ops.poll("Vault to answer", lambda: requests.get(f"{self.addr}/v1/sys/health", timeout=5).status_code == 200, 120)

    def put(self, name: str, value: str) -> None:
        resp = requests.put(
            f"{self.addr}/v1/secret/data/{name}",
            headers={"X-Vault-Token": self.token},
            json={"data": {SECRET_FIELD: value}},
            timeout=10,
        )
        if not resp.ok:
            raise ManagerGateFailure(f"writing to Vault failed: {resp.status_code}")

    def ref(self, name: str) -> str:
        return f"vault:secret/{name}#{SECRET_FIELD}"

    def other_scheme_ref(self) -> str:
        return "aws-sm:g12/not-this-manager"

    def outside_ref(self) -> str:
        return f"vault:secret/outside-g12/x#{SECRET_FIELD}"


class AwsStore:
    """AWS Secrets Manager, answered by a `moto` server."""

    kind = "aws"

    def __init__(self) -> None:
        import boto3  # installed by the CI step that runs this gate

        self.client = boto3.client(
            "secretsmanager",
            endpoint_url=os.environ["AWS_ENDPOINT_URL_SECRETS_MANAGER"],
            region_name=os.environ["AWS_REGION"],
            aws_access_key_id=os.environ["AWS_ACCESS_KEY_ID"],
            aws_secret_access_key=os.environ["AWS_SECRET_ACCESS_KEY"],
        )
        self.url = os.environ["AWS_ENDPOINT_URL_SECRETS_MANAGER"].rstrip("/")
        self._created: set[str] = set()

    def wait_ready(self) -> None:
        ops.poll("moto to answer", lambda: requests.get(f"{self.url}/moto-api/", timeout=5).ok, 120)

    def put(self, name: str, value: str) -> None:
        text = json.dumps({SECRET_FIELD: value})
        if name in self._created:
            self.client.put_secret_value(SecretId=name, SecretString=text)
        else:
            self.client.create_secret(Name=name, SecretString=text)
            self._created.add(name)

    def ref(self, name: str) -> str:
        return f"aws-sm:{name}#{SECRET_FIELD}"

    def other_scheme_ref(self) -> str:
        return "vault:secret/g12/not-this-manager#password"

    def outside_ref(self) -> str:
        return f"aws-sm:outside-g12/x#{SECRET_FIELD}"


def store_for(kind: str):
    return VaultStore() if kind == "vault" else AwsStore()


def set_reference(connector_id: str, reference: str) -> requests.Response:
    return matrix.API.put(
        f"{matrix.API_URL}/api/connectors/{connector_id}/credential",
        json={"primary": {"reference": reference}},
        timeout=60,
    )


def volume_files_for(connector_id: str) -> list[str]:
    """Names on the credentials volume that belong to this connector (a
    managed file's name embeds the connector id with `-` as `_`)."""
    key = connector_id.replace("-", "_")
    return sorted(name for name in os.listdir(MANAGED_DIR) if key in name)


def volume_holds(plain: str) -> list[str]:
    """Files on the credentials volume with `plain` in their bytes."""
    found = []
    for name in os.listdir(MANAGED_DIR):
        path = os.path.join(MANAGED_DIR, name)
        if os.path.isfile(path):
            with open(path, "rb") as fh:
                if plain.encode() in fh.read():
                    found.append(name)
    return found


def run_phase(kind: str) -> None:
    store = store_for(kind)
    store.wait_ready()
    matrix.step_wait_for_services()
    matrix.step_login()
    matrix.step_ensure_tenant()
    sfx = secrets.token_hex(3)
    table, bronze = f"g12m_{sfx}", f"g6_src12m_{kind}_{sfx}"
    secret_name = f"g12/{kind}-{sfx}"
    plain = matrix.CONNECTOR_MYSQL_PASSWORD
    matrix._src8_mysql(
        f"DROP TABLE IF EXISTS {table}",
        f"CREATE TABLE {table} (id INT PRIMARY KEY, name VARCHAR(50))",
        f"INSERT INTO {table} (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'c')",
    )
    store.put(secret_name, plain)

    # Nothing is stored for the connector: the credential will be a reference.
    body = matrix._create_connector(
        name=f"src12m-{kind}-{sfx}", kind="MySQL", host="mysql-g6:3306",
        credential={"source": "managed", "primary": "password"},
    )
    connector_id = body["id"]
    ops.expect_ok(ops.put_mysql_spec(connector_id, "mysql-g6", 3306, table, bronze), "PUT ingest-spec")

    # 1. Refusals, in fixed words.
    absent = set_reference(connector_id, store.ref(f"g12/absent-{sfx}"))
    if absent.status_code != 422:
        raise ManagerGateFailure(f"a reference to nothing should be refused with 422, got {absent.status_code} {absent.text[:200]}")
    text = absent.text
    if "does not exist in the secret manager" not in text:
        raise ManagerGateFailure(f"the refusal should say the secret does not exist, got: {text[:300]}")
    leaked = [w for w in MANAGER_WORDS if w.lower() in text.lower()]
    if leaked:
        raise ManagerGateFailure(f"the refusal carries a manager's own words {leaked}: {text[:300]}")
    other = set_reference(connector_id, store.other_scheme_ref())
    if other.status_code != 400:
        raise ManagerGateFailure(f"another manager's scheme should be refused with 400, got {other.status_code} {other.text[:200]}")
    say(f"1. a reference to nothing is refused with the fixed text; another manager's scheme is refused ({other.status_code})")

    # SRC-12 review B1 BLOCKER 1: the operator's prefixes (`.env`:
    # SECRET_MANAGER_ALLOWED_PREFIXES, `.../g12/`) are the only places a
    # reference may point, whatever the manager's identity could read.
    # `SRC-12 review B2 BLOCKER 3`: the first reference is VALID under the
    # grammar and outside the prefixes, so the refusal must be the prefix
    # rule's fixed text, exactly; the second is not valid at all ("..") and is
    # refused by the grammar, which is a different sentence.
    refused = set_reference(connector_id, store.outside_ref())
    if refused.status_code != 400 or OUTSIDE_PREFIXES_TEXT not in refused.text:
        raise ManagerGateFailure(
            f"a valid reference outside the allowed prefixes should be refused by the prefix rule with 400 and {OUTSIDE_PREFIXES_TEXT!r}, got {refused.status_code} {refused.text[:200]}"
        )
    dotdot = set_reference(connector_id, store.ref(f"g12/../outside-g12/{sfx}"))
    if dotdot.status_code != 400 or "not a valid" not in dotdot.text or OUTSIDE_PREFIXES_TEXT in dotdot.text:
        raise ManagerGateFailure(
            f"a reference with '..' should be refused by the grammar (400, 'not a valid ...'), got {dotdot.status_code} {dotdot.text[:200]}"
        )
    say("1b. a valid reference outside the operator's allowed prefixes is refused by the prefix rule; one with '..' by the grammar (400 each)")

    # 2. The reference is accepted: read once, connection tested.
    reference = store.ref(secret_name)
    saved = ops.expect_ok(set_reference(connector_id, reference), "PUT credential (reference)")
    if saved.get("saved") is not True or saved.get("verified") is not True:
        raise ManagerGateFailure(f"the reference should be saved and verified, got {saved}")
    source = ops.connector_detail(connector_id).get("credentialSource")
    if source != "reference":
        raise ManagerGateFailure(f"the connector should report its credential as a reference, got {source!r}")
    say("2. the reference was read, the connection worked, and the connector reports a reference")

    # 3. No copy on the credentials volume.
    files = volume_files_for(connector_id)
    holding = volume_holds(plain)
    if files or holding:
        raise ManagerGateFailure(f"the credentials volume holds a copy: files {files}, files with the value {holding}")
    say("3. the credentials volume holds no file for the connector and no file with the value")

    # 4. Test and load through the reference (the orchestrator reads it itself).
    ops_test = matrix.API.post(f"{matrix.API_URL}/api/connectors/{connector_id}/test", timeout=30)
    result = ops.expect_ok(ops_test, "POST test")
    if result.get("ok") is not True:
        raise ManagerGateFailure(f"the connection test should pass, got {result}")
    ops.wait_dagster(ops.launch_run(connector_id), "SUCCESS", "first load")
    rows = ops.bronze_rows(bronze)
    if rows != 3:
        raise ManagerGateFailure(f"bronze.{bronze} has {rows} rows after the first load, expected 3")
    say("4. the test passes and the orchestrator loaded 3 rows through the reference")

    # 5. Change the secret in the manager: the next use sees it.
    wrong = f"wrong-{secrets.token_hex(4)}"
    store.put(secret_name, wrong)
    failing = ops.expect_ok(
        matrix.API.post(f"{matrix.API_URL}/api/connectors/{connector_id}/test", timeout=30), "POST test (changed secret)"
    )
    if failing.get("ok") is not False:
        raise ManagerGateFailure(f"after the secret changed to a wrong value the test should fail, got {failing}")
    ops.wait_dagster(ops.launch_run(connector_id), "FAILURE", "load with the changed secret")
    if ops.bronze_rows(bronze) != 3:
        raise ManagerGateFailure("the run with the wrong secret should have loaded nothing")
    store.put(secret_name, plain)
    ops.wait_dagster(ops.launch_run(connector_id), "SUCCESS", "load with the secret changed back")
    rows = ops.bronze_rows(bronze)
    if rows != 6:
        raise ManagerGateFailure(f"bronze.{bronze} has {rows} rows after the last load, expected 6 (append)")
    say("5. a change in the manager was used by the very next test and run, and nothing was kept")


def main() -> int:
    kind = sys.argv[1] if len(sys.argv) > 1 else ""
    if kind not in ("vault", "aws"):
        print(f"[{GATE}] FAILED: usage: src12_managers_test.py vault|aws", file=sys.stderr)
        return 1
    try:
        run_phase(kind)
    except (ManagerGateFailure, ops.OpsGateFailure, matrix.G6Failure) as exc:
        print(f"[{GATE}] FAILED: {kind}: {exc}", file=sys.stderr)
        return 1
    except requests.RequestException as exc:
        print(f"[{GATE}] FAILED: {kind}: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1
    except Exception:  # noqa: BLE001 -- any other error is a failure of the gate and must be named as one
        import traceback

        print(f"[{GATE}] FAILED: {kind}: unexpected error:\n{traceback.format_exc()}", file=sys.stderr)
        return 1
    print(f"[{GATE}] PASS ({kind})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
