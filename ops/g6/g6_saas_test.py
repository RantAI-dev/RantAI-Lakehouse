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
schema change, rate limit, revoked sign-in), and `0096` (slice 7) flips the types whose
steps pass. A vendor's steps do not need its type to be `supported`: they drive the
vendor module and the shared `SaasSession` in this process (the runner uses the
Dagster image, which carries `dispar_orchestrate`) against the mock vendor on the
vendor's own port. A vendor's hosts are fixed in the table and a person cannot point
a connector elsewhere, so the steps use the gate-only `SAAS_GATE_ORIGINS`
(`saas/vendors/__init__.py: rebase_for_gate`). This proves the vendor code over real
HTTP; it does not go through the API's connector routes (the Rust connection test
has wiremock tests) or the Iceberg sink (pytest `loader` tests). `--vendors-only`
runs the vendor steps with no API, for a local run against `python saas_stub.py`. The lines are printed with the prefix `[saas]` and copied to the job
summary; the job's time is the measurement against the signed 20 minutes.

Settings: `LAKEHOUSE_API_URL`, `AUTH_BOOTSTRAP_EMAIL`, `AUTH_BOOTSTRAP_PASSWORD`
(as `g6_ingest_matrix_test.py` reads them; its login helpers are imported),
`SAAS_STUB_URL`. Exit code 0 and `[saas] PASS`, or `[saas] FAILED: ...` on
stderr and exit code 1.
"""
from __future__ import annotations

import json
import os
import sys
import time
from urllib.parse import urlparse

import g6_ingest_matrix_test as matrix
import requests
import saas_stub

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


# ---- the vendors' steps ------------------------------------------------------------------------
#
# Words the mock puts in every error body. The product must repeat none of them
# (principle 4): each failure step checks the text it surfaces.
VENDOR_WORDS = "STUB-VENDOR-WORDS"


def _setup_vendor_environment() -> None:
    """Point the vendor modules at the mock (gate-only, see the module doc). The
    compose override sets the first two for the runner; they are set here too so a
    local run against `python saas_stub.py` works the same."""
    host = urlparse(STUB_URL).hostname
    os.environ.setdefault("CONNECTOR_OAUTH_ALLOW_HTTP", "true")
    os.environ.setdefault("INGEST_ALLOW_INTERNAL_HOSTS", "true")
    os.environ["SAAS_GATE_ORIGINS"] = json.dumps({vendor: f"http://{host}:{port}" for vendor, port in saas_stub.VENDOR_PORTS.items()})


def admin(vendor: str, **body: object) -> dict:
    resp = requests.post(f"{STUB_URL}/__admin/{vendor}", json=body, timeout=10)
    if not resp.ok:
        raise SaasGateFailure(f"the mock {vendor} refused {body.get('action')}: {resp.status_code}")
    return resp.json()


def mock_log(vendor: str) -> list[str]:
    return requests.get(f"{STUB_URL}/__admin/{vendor}", timeout=10).json()["log"]


def _expect(condition: bool, message: str) -> None:
    if not condition:
        raise SaasGateFailure(message)


def _raises(expected: str, call) -> Exception:  # noqa: ANN001
    """The exception `call` raises, which must be of the class named `expected`
    and must not hold any word the mock's error bodies carry."""
    try:
        call()
    except Exception as exc:  # noqa: BLE001 -- the class is checked right below
        _expect(type(exc).__name__ == expected, f"expected {expected}, got {type(exc).__name__}")
        _expect(VENDOR_WORDS not in str(exc), f"{expected} repeats the vendor's words: {exc}")
        return exc
    raise SaasGateFailure(f"expected {expected}, nothing was raised")


def _hubspot(auth: str = "integration"):  # noqa: ANN202
    from dispar_orchestrate.adapters import saas as saas_adapter
    from dispar_orchestrate.saas.vendors import hubspot as _module  # noqa: F401 -- imported for its registration

    dial = {"vendor": "hubspot", "account": "123456", "auth": {"type": "integration"}}
    secrets = {"token": "stub-private-app-token"}
    if auth == "oauth":
        dial = {"vendor": "hubspot", "account": "123456", "auth": {"type": "oauth2_authorization_code", "clientId": "stub-client"}}
        secrets = {"clientSecret": "stub-client-secret", "refreshToken": "stub-refresh-token"}
    module, vendor, parts, session = saas_adapter.open_session(dial, secrets)
    return module, vendor, parts, session, dial


def _read(module, parts, session, dial, name: str, since=None) -> list[dict]:  # noqa: ANN001
    decl = module.declaration(session, parts, dial, name)
    return list(module.read(session, parts, dial, decl, since=since))


def step_hubspot() -> None:
    """HubSpot (V-HubSpot.4): connect (both sign-ins), discover, load, reload
    (incremental, with a delete), schema change, rate limit, revoked sign-in."""
    from dispar_orchestrate import ingest_errors
    admin("hubspot", action="reset")

    for auth in ("integration", "oauth"):
        _module, vendor, _parts, session, _dial = _hubspot(auth)
        who = session.get_json(vendor.whoami["path"])
        _expect(who.get("portalId") == 123456, f"hubspot connect ({auth}): the account it reached is {who.get('portalId')!r}")
    _log("hubspot connect: the private-app token and the browser sign-in both reach account 123456")

    module, _vendor, parts, session, dial = _hubspot()
    declared = {d.name: d for d in module.list_objects(session, parts, dial)}
    _expect({"deals", "contacts", "owners"} <= set(declared), "hubspot discover: the objects are not listed")
    _expect(declared["deals"].load_description() == "incremental" and declared["owners"].load_description().startswith("re-read in full: "), "hubspot discover: how each object loads")
    fields = {f.name: f.type_name for f in module.describe(session, parts, dial, "deals")}
    _expect(fields.get("id") and fields.get("amount") == "double" and fields.get("is_renewal") == "bool", f"hubspot discover: the properties API fields are {fields}")
    _log(f"hubspot discover: {len(declared)} objects, deals described with {len(fields)} fields")

    admin("hubspot", action="clear_log")
    first = _read(module, parts, session, dial, "deals")
    _expect([r["id"] for r in first] == ["1", "2", "3"], f"hubspot load: first run read {[r['id'] for r in first]}")
    _expect(first[0]["amount"] == 100.0 and first[0]["is_renewal"] is False, "hubspot load: the values are not typed")
    _expect(not any("/search" in line for line in mock_log("hubspot")), "hubspot load: the first run searched instead of listing")
    _log("hubspot load: 3 deals listed, numbers and booleans typed")

    cursor = max(r["hs_lastmodifieddate"] for r in first)
    admin("hubspot", action="put", object="deals", id=2, properties={"dealname": "deal 2 changed", "amount": 250})
    admin("hubspot", action="put", object="deals", id=4, properties={"dealname": "deal 4 new", "amount": 400})
    admin("hubspot", action="archive", object="deals", id=1)
    admin("hubspot", action="clear_log")
    again = _read(module, parts, session, dial, "deals", since=cursor)
    ids = sorted(r["id"] for r in again)
    _expect(ids == ["2", "3", "4"], f"hubspot reload: read {ids}, expected the changed (2), the new (4) and the cursor's own record (3)")
    log = mock_log("hubspot")
    _expect(any("POST" in line and line.endswith("/search") for line in log), "hubspot reload: no search request")
    _expect(not any(line == "GET /crm/v3/objects/deals" for line in log), "hubspot reload: it listed every deal")
    _expect(not declared["deals"].deletes and "1" not in ids, "hubspot reload: a delete is claimed, or the archived deal came back")
    _log("hubspot reload: only the changed, the new and the cursor record came back; the archived deal is not reported (HubSpot's search omits it, so no delete is claimed)")

    before = set(fields)
    admin("hubspot", action="add_property", object="deals", name="region", type="string")
    after = {f.name for f in module.describe(session, parts, dial, "deals")}
    _expect(after - before == {"region"}, f"hubspot schema change: describe differs by {after - before}")
    _log("hubspot schema change: the new property shows in describe before any load")

    admin("hubspot", action="rate_limit", count=2)
    started = time.time()
    _expect(len(_read(module, parts, session, dial, "owners")) == 1, "hubspot rate limit: the read did not come back after the waits")
    _expect(time.time() - started >= 1.5, "hubspot rate limit: the Retry-After was not waited")
    admin("hubspot", action="rate_limit", count=50)
    exc = _raises("SaasRateLimited", lambda: _read(module, parts, session, dial, "owners"))
    _expect(ingest_errors.classify(exc).error_class == "rate_limited", "hubspot rate limit: the run report class")
    admin("hubspot", action="rate_limit", count=0)
    _log("hubspot rate limit: 429 with Retry-After waited out; a limit that does not lift fails as rate_limited")

    admin("hubspot", action="revoke", on=True)
    exc = _raises("SaasSignInRefused", lambda: _read(module, parts, session, dial, "owners"))
    _expect(ingest_errors.classify(exc).error_class == "sign_in_refused", "hubspot revoked: the run report class")

    def browser_sign_in_read() -> None:
        # The token request happens at the first header, i.e. in this read.
        b_module, _v, b_parts, b_session, b_dial = _hubspot("oauth")
        _read(b_module, b_parts, b_session, b_dial, "owners")

    exc = _raises("OAuthSignInRefused", browser_sign_in_read)
    _expect(ingest_errors.classify(exc).error_class == "sign_in_refused", "hubspot revoked: the browser sign-in's run report class")
    _log("hubspot revoked sign-in: the token and the refresh token are refused with the fixed text, none of the vendor's words")
    admin("hubspot", action="revoke", on=False)


def run_vendors() -> None:
    _setup_vendor_environment()
    step_hubspot()


def run() -> None:
    matrix._wait_for("lakehouse-api", lambda: matrix.API.get(f"{matrix.API_URL}/health", timeout=5).status_code == 200, 120)
    matrix.step_login()
    matrix.step_ensure_tenant()
    step_stub_is_up()
    step_types_are_listed_unsupported()
    run_vendors()


def main() -> int:
    sys.stdout.reconfigure(line_buffering=True)
    sys.stderr.reconfigure(line_buffering=True)
    started = time.time()
    try:
        if "--vendors-only" in sys.argv:
            run_vendors()
        else:
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
