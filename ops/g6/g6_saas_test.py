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


def _zendesk(auth: str = "integration"):  # noqa: ANN202
    from dispar_orchestrate.adapters import saas as saas_adapter
    from dispar_orchestrate.saas.vendors import zendesk as _module  # noqa: F401 -- imported for its registration

    if auth == "oauth":
        dial = {"vendor": "zendesk", "account": "acme", "auth": {"type": "oauth2_authorization_code", "clientId": "stub-client"}}
        # Zendesk rotates the refresh token on every refresh: ask the mock which one is current.
        current = admin("zendesk", action="refresh_token")["refresh_token"]
        secrets = {"clientSecret": "stub-client-secret", "refreshToken": current}
    else:
        dial = {"vendor": "zendesk", "account": "acme", "auth": {"type": "integration"}}
        secrets = {"email": "agent@example.invalid", "apiToken": "stub-api-token"}
    module, vendor, parts, session = saas_adapter.open_session(dial, secrets)
    return module, vendor, parts, session, dial


def step_zendesk() -> None:
    """Zendesk (V-Zendesk.4): connect (email + API token), discover, load (no ticket twice
    at a page boundary: upstream #659), reload (cursor export; a change, a new ticket, a
    deleted one), a new field, rate limit, revoked and forbidden sign-in."""
    from dispar_orchestrate import ingest_errors

    admin("zendesk", action="reset")

    module, vendor, parts, session, dial = _zendesk()
    who = session.get_json(vendor.whoami["path"])
    _expect(who["user"]["id"] == 7, f"zendesk connect: the account it reached is {who!r}")
    _log("zendesk connect: the email and API token reach user 7")
    _b_module, b_vendor, _b_parts, b_session, _b_dial = _zendesk("oauth")
    _expect(b_session.get_json(b_vendor.whoami["path"])["user"]["id"] == 7, "zendesk connect (browser sign-in): not the account")
    _log("zendesk connect (browser sign-in): the refresh grant with the client in a JSON body reaches user 7 (the refresh token rotated)")

    declared = {d.name: d for d in module.list_objects(session, parts, dial)}
    _expect({"tickets", "users", "ticket_events", "organizations", "groups"} <= set(declared), "zendesk discover: the objects are not listed")
    _expect(declared["tickets"].deletes and declared["tickets"].load_description() == "incremental", "zendesk discover: tickets")
    _expect(declared["groups"].load_description().startswith("re-read in full: "), "zendesk discover: groups")
    _log(f"zendesk discover: {len(declared)} objects; tickets incremental and reporting deletes, groups re-read in full")

    # The mock must still send the duplicate of upstream #659, or the check below proves nothing.
    raw = []
    link = f"/api/v2/incremental/tickets.json?start_time={int(time.time()) - 2 * 86_400}&per_page=3"
    while link:
        page = session.get_json(link)
        raw += [t["id"] for t in page["tickets"]]
        link = None if page["end_of_stream"] else page["next_page"]
    _expect(len(raw) > len(set(raw)), f"zendesk load: the mock's time-based export repeated nothing ({raw}); it must, to model Zendesk")
    first = _read(module, parts, session, dial, "tickets")
    _expect(sorted(t["id"] for t in first) == [1, 2, 3, 4, 5, 6], f"zendesk load: read {[t['id'] for t in first]}, expected each of 6 tickets once")
    _log(f"zendesk load: the time-based export sends {raw} (duplicates at the page boundaries); the connector reads each of the 6 tickets once")

    cursor = max(t["updated_at"] for t in first)
    admin("zendesk", action="put", id=2, fields={"subject": "ticket 2 changed"})
    admin("zendesk", action="put", id=7, fields={"subject": "ticket 7 new"})
    admin("zendesk", action="delete", id=3)
    admin("zendesk", action="clear_log")
    again = _read(module, parts, session, dial, "tickets", since=cursor)
    got = {t["id"]: t["status"] for t in again}
    _expect(set(got) == {2, 3, 6, 7}, f"zendesk reload: read {sorted(got)}, expected the changed (2), the deleted (3), the new (7) and the cursor's own (6)")
    _expect(got[3] == "deleted" and declared["tickets"].deletes, "zendesk reload: the deleted ticket does not say deleted")
    _expect(all("/cursor.json" in line for line in mock_log("zendesk") if "incremental" in line), "zendesk reload: a time-based export was used for tickets")
    _log("zendesk reload: the changed, the new and the deleted ticket (status deleted) came back, and the cursor's own; nothing older, nothing twice")

    admin("zendesk", action="add_field", name="priority_note")
    changed = _read(module, parts, session, dial, "tickets", since=cursor)
    _expect(any("priority_note" in t for t in changed), "zendesk schema change: the new field is not in the records")
    _log("zendesk schema change: the new field arrives in the records; Zendesk has no describe, so the observer reports it after the load (decision 9)")

    admin("zendesk", action="rate_limit", count=2)
    started = time.time()
    _expect(len(_read(module, parts, session, dial, "groups")) == 7, "zendesk rate limit: the groups did not come back after the waits")
    _expect(time.time() - started >= 1.5, "zendesk rate limit: the Retry-After was not waited")
    admin("zendesk", action="rate_limit", count=50)
    exc = _raises("SaasRateLimited", lambda: _read(module, parts, session, dial, "groups"))
    _expect(ingest_errors.classify(exc).error_class == "rate_limited", "zendesk rate limit: the run report class")
    admin("zendesk", action="rate_limit", count=0)
    _log("zendesk rate limit: 429 with Retry-After waited out; a limit that does not lift fails as rate_limited")

    admin("zendesk", action="forbid", on=True)
    exc = _raises("SaasPermissionMissing", lambda: _read(module, parts, session, dial, "groups"))
    _expect(ingest_errors.classify(exc).error_class == "permission_missing", "zendesk forbidden: the run report class")
    admin("zendesk", action="forbid", on=False)
    admin("zendesk", action="revoke", on=True)
    exc = _raises("SaasSignInRefused", lambda: _read(module, parts, session, dial, "groups"))
    _expect(ingest_errors.classify(exc).error_class == "sign_in_refused", "zendesk revoked: the run report class")

    def browser_read() -> None:
        r_module, _v, r_parts, r_session, r_dial = _zendesk("oauth")
        _read(r_module, r_parts, r_session, r_dial, "groups")

    exc = _raises("OAuthSignInRefused", browser_read)
    _expect(ingest_errors.classify(exc).error_class == "sign_in_refused", "zendesk revoked: the browser sign-in's run report class")
    admin("zendesk", action="revoke", on=False)
    _log("zendesk forbidden and revoked sign-in: the fixed failures, none of the vendor's words")


# ---- the product steps (slice 2 review BLOCKER 4) ------------------------------------------------
#
# AD9 and the SRC-9 registry want connect, discover, load and reload to be `real`, which means
# THROUGH THE PRODUCT, not through the vendor module alone: the connector is created through the API
# (a type that is `supported = false` is not refused), the API's connection test dials the mock, the
# discovery routes run on the orchestrator, and `ingest_job` loads into Bronze Iceberg, which the
# gate reads back through ClickHouse. Built once here for every vendor: a vendor's slice adds an
# entry to `PRODUCT_VENDORS` and its mock. The browser sign-in cannot be driven without a browser, so
# it stays a vendor-level step above; these steps sign in with the vendor's integration credential.

PRODUCT_VENDORS: dict[str, dict] = {
    "hubspot": {
        "type": "HubSpot", "host": "api.hubapi.com", "account": "123456", "label": "HubSpot",
        "credential": {"source": "file", "primary": "token"}, "values": {"primary": "stub-private-app-token"},
        "object": "deals", "target": "saas_gate_hubspot_deals", "cursor": "hs_lastmodifieddate", "text": "dealname",
        # discovery: the properties API describes deals before a load (decision 9)
        "preview_columns": {"id", "dealname", "amount"},
        "first_ids": {"1", "2", "3"}, "reload_ids": {"1", "2", "3", "4"},
    },
    "zendesk": {
        "type": "Zendesk", "host": "acme.zendesk.com", "account": "acme", "label": "Zendesk",
        "credential": {"source": "file", "primary": "access_key", "secondary": "token"},
        "values": {"primary": "agent@example.invalid", "secondary": "stub-api-token"},
        "object": "tickets", "target": "saas_gate_zendesk_tickets", "cursor": "updated_at", "text": "subject",
        # Zendesk does not describe its fields before a read, so its preview is honestly unsupported
        "preview_columns": None,
        "first_ids": {"1", "2", "3", "4", "5", "6"}, "reload_ids": {"1", "2", "3", "4", "5", "6", "7"},
    },
}


def _poll(what: str, read, ok, timeout_s: int = 90):  # noqa: ANN001, ANN202
    """Read until `ok(observed)`, else fail naming what was last seen (ClickHouse may lag a load)."""
    deadline = time.time() + timeout_s
    observed = None
    while True:
        try:
            observed = read()
            if ok(observed):
                return observed
        except matrix.G6Failure as exc:
            observed = f"read error: {str(exc)[:200]}"
        if time.time() >= deadline:
            raise SaasGateFailure(f"{what}; last saw {observed!r}")
        time.sleep(3.0)


def _bronze_rows(target: str, columns: str) -> list[dict]:
    """The rows of `bronze.<target>` through ClickHouse, ordered by key, under a `WHERE` (R11)."""
    matrix._bronze_row_count(target)  # creates the catalog database
    text = matrix.ch_query(
        f"SELECT {columns} FROM {matrix.CATALOG_DB}.`bronze.{target}` WHERE 1 "
        "ORDER BY toString(id) SETTINGS allow_database_iceberg = 1 FORMAT JSONEachRow"
    )
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def _run_and_wait(connector_id: str, label: str) -> None:
    run_id = matrix._post_ingest_run(connector_id).json().get("runId")
    if not run_id:
        raise SaasGateFailure(f"{label}: ingest/run returned no runId")
    matrix.step_wait_for_run_success(run_id)


def step_product(vendor_id: str) -> None:
    """connect, discover, load, reload and a revoked sign-in for one vendor, through the product."""
    spec = PRODUCT_VENDORS[vendor_id]
    label, obj, target, text = spec["label"], spec["object"], spec["target"], spec["text"]
    started = time.time()

    created = matrix._create_connector(name=f"g6-saas-{vendor_id}", kind=spec["type"], host=spec["host"], credential=spec["credential"])
    cid = created["id"]
    derived = created.get("credential") or {}
    for slot in ("primary", "secondary"):
        if spec["values"].get(slot):
            matrix._write_credential_file(derived[slot], spec["values"][slot])
    resp = matrix.API.put(
        f"{matrix.API_URL}/api/connectors/{cid}/ingest-spec",
        json={
            "adapter": "saas", "ingestMode": "batch",
            "dial": {"vendor": vendor_id, "account": spec["account"], "auth": {"type": "integration"}},
            "sourceObjects": [{"name": obj, "target": target, "loadMode": "incremental_dedup", "incrementalKey": spec["cursor"], "primaryKey": ["id"]}],
        },
        timeout=15,
    )
    if not resp.ok:
        raise SaasGateFailure(f"{label}: PUT ingest-spec answered {resp.status_code} {resp.text[:300]}")
    _log(f"{vendor_id} create: the {spec['type']} connector (a type that is not supported yet) was created and its spec saved through the API")

    # connect: the API's connection test dials the mock vendor with the saved credential.
    test = matrix.API.post(f"{matrix.API_URL}/api/connectors/{cid}/test", timeout=30)
    body = test.json() if test.ok else {}
    _expect(test.ok and body.get("supported") is True and body.get("ok") is True, f"{vendor_id} connect: POST .../test answered {test.status_code} {str(body)[:300]}")
    _expect(spec["account"] in str(body.get("message")) and VENDOR_WORDS not in str(body), f"{vendor_id} connect: the message does not name the account, or repeats the vendor: {body.get('message')!r}")
    _log(f"{vendor_id} connect: the API's connection test passed and named the account ({body.get('latencyMs')} ms)")

    # discover: through the orchestrator.
    listing = matrix.discovery_answer(cid, "list", {})
    found = next((o for o in listing.get("objects") or [] if o.get("name") == obj), None)
    _expect(listing.get("state") == "done" and found is not None, f"{vendor_id} discover: the list is {listing.get('state')!r} {listing.get('errorClass')!r}, without {obj!r}")
    _expect(found.get("load") == "incremental", f"{vendor_id} discover: {obj} should say it loads incrementally, says {found.get('load')!r}")
    preview = matrix.discovery_answer(cid, "preview", {"object": obj})
    if spec["preview_columns"] is None:
        _expect(preview.get("state") == "failed" and preview.get("errorClass") == "not_supported", f"{vendor_id} discover: expected an honest not_supported preview, got {preview.get('state')!r} {preview.get('errorClass')!r}")
        detail = "its preview is honestly unsupported (no describe)"
    else:
        names = {c.get("name") for c in preview.get("columns") or []}
        _expect(preview.get("state") == "done" and spec["preview_columns"] <= names, f"{vendor_id} discover: the preview shows {sorted(names)[:12]}")
        detail = f"previewed {len(names)} columns"
    _log(f"{vendor_id} discover: {len(listing['objects'])} objects listed by the orchestrator, {obj} loads incrementally; {detail}")

    # load: ingest_job into Bronze Iceberg, read back.
    _run_and_wait(cid, f"{vendor_id} load")
    columns = f"id, {text}" + (", status" if vendor_id == "zendesk" else "")
    rows = _poll(f"{vendor_id} load: Bronze {target}", lambda: _bronze_rows(target, columns), lambda r: len(r) == len(spec["first_ids"]))
    _expect({str(r["id"]) for r in rows} == spec["first_ids"], f"{vendor_id} load: Bronze holds ids {[r['id'] for r in rows]}")
    _log(f"{vendor_id} load: {len(rows)} rows in Bronze {target}, read back through ClickHouse, each once")

    # reload: the vendor changes; only the changed and new records are read, merged on the key.
    if vendor_id == "hubspot":
        admin("hubspot", action="put", object="deals", id=2, properties={"dealname": "deal 2 changed"})
        admin("hubspot", action="put", object="deals", id=4, properties={"dealname": "deal 4 new"})
        admin("hubspot", action="archive", object="deals", id=1)
    else:
        admin("zendesk", action="put", id=2, fields={"subject": "ticket 2 changed"})
        admin("zendesk", action="put", id=7, fields={"subject": "ticket 7 new"})
        admin("zendesk", action="delete", id=3)
    admin(vendor_id, action="clear_log")
    _run_and_wait(cid, f"{vendor_id} reload")
    after = _poll(
        f"{vendor_id} reload: Bronze {target}", lambda: _bronze_rows(target, columns),
        lambda r: {str(x["id"]) for x in r} == spec["reload_ids"] and any("changed" in str(x[text]) for x in r),
    )
    ids = [str(r["id"]) for r in after]
    _expect(len(ids) == len(set(ids)), f"{vendor_id} reload: a key is in Bronze twice: {ids}")
    by_id = {str(r["id"]): r for r in after}
    _expect("changed" in str(by_id["2"][text]), f"{vendor_id} reload: ticket/deal 2 was not updated")
    if vendor_id == "zendesk":
        _expect(by_id["3"]["status"] == "deleted", "zendesk reload: the deleted ticket does not say status = deleted in Bronze")
        _expect(len(by_id) == 7, "zendesk reload: a ticket was lost")
    else:
        _expect("1" in by_id, "hubspot reload: the archived deal vanished; no delete is claimed, so it must stay")
    seen = mock_log(vendor_id)
    reads = [line for line in seen if "/search" in line or "cursor" in line or "incremental" in line]
    _expect(reads, f"{vendor_id} reload: the mock saw no incremental request ({seen})")
    _expect(not any(line == "GET /crm/v3/objects/deals" for line in seen), f"{vendor_id} reload: it listed every deal again")
    _log(f"{vendor_id} reload: the changed and the new record merged on the key, no duplicate; "
         + ("the deleted ticket says status = deleted" if vendor_id == "zendesk" else "the archived deal stays (HubSpot reports no deletes)"))

    # revoked sign-in, through the API's test and a run.
    admin(vendor_id, action="revoke", on=True)
    try:
        test = matrix.API.post(f"{matrix.API_URL}/api/connectors/{cid}/test", timeout=30)
        body = test.json() if test.ok else {}
        _expect(test.ok and body.get("ok") is False and VENDOR_WORDS not in str(body), f"{vendor_id} revoked: the connection test said {str(body)[:300]}")
        run_id = matrix._post_ingest_run(cid).json().get("runId")
        matrix.step_wait_for_run_failure(run_id)
    finally:
        admin(vendor_id, action="revoke", on=False)
    _log(f"{vendor_id} revoked sign-in: the API's test fails in plain words and the run fails; none of the vendor's words appear in the test")
    _log(f"{vendor_id} product steps took {time.time() - started:.0f} s")


def run_vendors() -> None:
    _setup_vendor_environment()
    step_hubspot()
    step_zendesk()


def run() -> None:
    matrix.step_wait_for_services()  # ClickHouse, the API, Dagster and its code location
    matrix.step_login()
    matrix.step_ensure_tenant()
    step_stub_is_up()
    step_types_are_listed_unsupported()
    _setup_vendor_environment()
    for vendor_id in PRODUCT_VENDORS:
        admin(vendor_id, action="reset")
        step_product(vendor_id)
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
