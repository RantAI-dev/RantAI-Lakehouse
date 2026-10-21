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
from datetime import timedelta
from urllib.parse import unquote_plus, urlparse

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


def _salesforce(instance_check: bool = True):  # noqa: ANN202
    from dispar_orchestrate.adapters import saas as saas_adapter
    from dispar_orchestrate.saas.vendors import salesforce as _module  # noqa: F401 -- imported for its registration

    dial = {"vendor": "salesforce", "account": "acme", "auth": {"type": "oauth2_authorization_code", "clientId": "stub-client"}}
    secrets = {"clientSecret": "stub-client-secret", "refreshToken": "stub-refresh-token"}
    module, vendor, parts, session = saas_adapter.open_session(dial, secrets)
    return module, vendor, parts, session, dial


def step_salesforce() -> None:
    """Salesforce (V-Salesforce.4): connect (browser sign-in; the returned host adopted
    through the .salesforce.com rule), discover (custom objects, describe), load (paged by
    nextRecordsUrl), reload (queryAll: a change, a new record, a delete), a new field, rate
    limit, forbidden, revoked sign-in, and a returned host outside the rule."""
    from dispar_orchestrate import ingest_errors

    admin("salesforce", action="reset")
    module, vendor, parts, session, dial = _salesforce()
    host = urlparse(STUB_URL).hostname
    _expect(session.origin == f"http://{host}:{saas_stub.VENDOR_PORTS['salesforce']}", f"salesforce connect: the session dials {session.origin}")
    who = session.get_json(vendor.whoami["path"])
    _expect("DailyApiRequests" in who, f"salesforce connect: the limits call answered {who!r}")
    _expect(any(line == "POST /services/oauth2/token" for line in mock_log("salesforce")), "salesforce connect: no refresh-token grant reached the mock")
    _log("salesforce connect: the refresh token was exchanged at the table's token path and the instance_url (a .salesforce.com host) adopted; the limits call is answered")

    declared = {d.name: d for d in module.list_objects(session, parts, dial)}
    _expect(set(declared) == {"Account", "Region__c"}, f"salesforce discover: the objects are {sorted(declared)}")
    _expect(declared["Account"].load_description() == "incremental", "salesforce discover: Account")
    _expect(declared["Region__c"].load_description().startswith("re-read in full: "), "salesforce discover: the custom object Region__c")
    fields = {f.name: f.type_name for f in module.describe(session, parts, dial, "Account")}
    _expect(fields.get("Id") and fields.get("IsDeleted") == "bool" and fields.get("AnnualRevenue") == "double", f"salesforce discover: the describe fields are {fields}")
    _log(f"salesforce discover: {len(declared)} objects (the custom Region__c re-read in full), Account described with {len(fields)} fields")

    admin("salesforce", action="clear_log")
    first = _read(module, parts, session, dial, "Account")
    _expect(sorted(r["Id"] for r in first) == ["001STUB001", "001STUB002", "001STUB003"], f"salesforce load: first run read {[r['Id'] for r in first]}")
    log = mock_log("salesforce")
    _expect(any("/query/01gSTUB" in line for line in log), f"salesforce load: no nextRecordsUrl was followed ({log})")
    _log("salesforce load: 3 accounts read over 2 batches, the second by nextRecordsUrl")

    cursor = max(r["SystemModstamp"] for r in first)
    admin("salesforce", action="put", id=2, fields={"Name": "account 2 changed"})
    admin("salesforce", action="put", id=4, fields={"Name": "account 4 new"})
    admin("salesforce", action="delete", id=3)
    admin("salesforce", action="clear_log")
    again = _read(module, parts, session, dial, "Account", since=cursor)
    got = {r["Id"]: r["IsDeleted"] for r in again}
    _expect(got == {"001STUB002": False, "001STUB003": True, "001STUB004": False}, f"salesforce reload: read {got}, expected the changed (2), the deleted (3) marked IsDeleted and the new (4)")
    _expect(any(line == "GET /services/data/v62.0/queryAll" for line in mock_log("salesforce")) and module.declaration(session, parts, dial, "Account").deletes, "salesforce reload: queryAll was not used")
    _log("salesforce reload: the changed, the new and the deleted record (IsDeleted true) came back through queryAll; the unchanged one did not")

    admin("salesforce", action="add_field", name="Rating__c")
    after = {f.name for f in module.describe(session, parts, dial, "Account")}
    _expect("Rating__c" in after and "Rating__c" not in fields, "salesforce schema change: the new custom field is not in describe")
    _log("salesforce schema change: the new custom field shows in describe before any load")

    admin("salesforce", action="rate_limit", count=2)
    started = time.time()
    _expect(len(_read(module, parts, session, dial, "Region__c")) == 3, "salesforce rate limit: Region__c did not come back after the waits")
    _expect(time.time() - started >= 1.5, "salesforce rate limit: the Retry-After was not waited")
    admin("salesforce", action="rate_limit", count=50)
    exc = _raises("SaasRateLimited", lambda: _read(module, parts, session, dial, "Region__c"))
    _expect(ingest_errors.classify(exc).error_class == "rate_limited", "salesforce rate limit: the run report class")
    admin("salesforce", action="rate_limit", count=0)
    _log("salesforce rate limit: 429 with Retry-After waited out; a limit that does not lift fails as rate_limited")

    admin("salesforce", action="quota", on=True)
    exc = _raises("SaasQuotaExhausted", lambda: _read(module, parts, session, dial, "Region__c"))
    classified = ingest_errors.classify(exc)
    _expect((classified.error_class, classified.retried) == ("rate_limited", False), f"salesforce quota: the run report class is {classified}")
    admin("salesforce", action="quota", on=False)
    _log("salesforce quota: the 403 REQUEST_LIMIT_EXCEEDED of a used-up API quota fails as rate_limited and is not retried in the run (BLOCKER 8)")

    admin("salesforce", action="forbid", on=True)
    exc = _raises("SaasPermissionMissing", lambda: _read(module, parts, session, dial, "Region__c"))
    _expect(ingest_errors.classify(exc).error_class == "permission_missing", "salesforce forbidden: the run report class")
    admin("salesforce", action="forbid", on=False)

    admin("salesforce", action="revoke", on=True)
    exc = _raises("OAuthSignInRefused", _salesforce)
    _expect(ingest_errors.classify(exc).error_class == "sign_in_refused", "salesforce revoked: the run report class")
    admin("salesforce", action="revoke", on=False)
    _log("salesforce forbidden and revoked sign-in: the fixed failures, none of the vendor's words")

    admin("salesforce", action="evil_instance", on=True)
    admin("salesforce", action="clear_log")
    _raises("SsrfBlocked", _salesforce)
    _expect(mock_log("salesforce") == ["POST /services/oauth2/token"], f"salesforce instance_url: something was asked of the mock after the token ({mock_log('salesforce')})")
    admin("salesforce", action="evil_instance", on=False)
    _log("salesforce instance_url: a returned host outside .salesforce.com / .force.com stops the run before any read")


GA4_REPORT = "traffic = country, date | sessions, purchaseRevenue"
GA4_BROWSER = {"vendor": "ga4", "account": "123", "objects": [GA4_REPORT], "auth": {"type": "oauth2_authorization_code", "clientId": "stub-client"}}
GA4_KEY = {}  # the gate's throwaway service-account key, made once per run


def _ga4_key_file() -> str:
    """A service-account key file whose private key was made for this run (never a real one);
    its public half is registered with the mock, which checks the assertion's signature."""
    if not GA4_KEY:
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric import rsa

        key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        numbers = key.public_key().public_numbers()
        admin("ga4", action="register_key", n=str(numbers.n), e=str(numbers.e))
        pem = key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()).decode()
        GA4_KEY["file"] = json.dumps({"type": "service_account", "client_email": "reader@example.invalid", "private_key": pem})
    return GA4_KEY["file"]


def _ga4(auth: str = "integration"):  # noqa: ANN202
    from dispar_orchestrate.adapters import saas as saas_adapter
    from dispar_orchestrate.saas.vendors import ga4 as _module  # noqa: F401 -- imported for its registration

    if auth == "oauth":
        dial, secrets = GA4_BROWSER, {"clientSecret": "stub-client-secret", "refreshToken": "stub-refresh-token"}
    else:
        dial, secrets = {"vendor": "ga4", "account": "123", "objects": [GA4_REPORT], "auth": {"type": "integration"}}, {"serviceAccountJson": _ga4_key_file()}
    module, vendor, parts, session = saas_adapter.open_session(dial, secrets)
    return module, vendor, parts, session, dial


def step_ga4() -> None:
    """Google Analytics 4 (V-GA4.4): connect (browser sign-in and a service-account key signed
    with a throwaway RSA key the mock verifies), discover (the report the connector defines, its
    columns from the property's metadata), load (paged by offset), reload (the last days are read
    again, Google having corrected them), a new custom dimension, rate limit, forbidden, revoked."""
    from dispar_orchestrate import ingest_errors
    from dispar_orchestrate.saas.vendors import ga4 as ga4_module

    admin("ga4", action="reset")
    GA4_KEY.clear()
    for auth in ("integration", "oauth"):
        _module, vendor, parts, session, _dial = _ga4(auth)
        meta = session.get_json(vendor.whoami["path"].replace("{account}", parts.account))
        _expect({"sessions", "purchaseRevenue"} <= {m["apiName"] for m in meta["metrics"]}, f"ga4 connect ({auth}): the metadata is {meta!r}")
    log = mock_log("ga4")
    _expect("POST /token urn:ietf:params:oauth:grant-type:jwt-bearer" in log and "POST /token refresh_token" in log, f"ga4 connect: the token grants the mock saw are {log}")
    _log("ga4 connect: the browser sign-in (refresh grant) and the service-account key (an RS256 assertion whose signature the mock verified) both reach the property")

    module, vendor, parts, session, dial = _ga4("integration")
    declared = {d.name: d for d in module.list_objects(session, parts, dial)}
    _expect(list(declared) == [GA4_REPORT] and declared[GA4_REPORT].load_description() == "incremental", f"ga4 discover: the objects are {list(declared)}")
    _expect(declared[GA4_REPORT].primary_key == ("country", "date") and declared[GA4_REPORT].cursor == "date", "ga4 discover: the key and the cursor")
    fields = {f.name: f.type_name for f in module.describe(session, parts, dial, GA4_REPORT)}
    _expect(fields == {"country": "text", "date": "timestamp", "sessions_INTEGER": "bigint", "purchaseRevenue_CURRENCY": "double"}, f"ga4 discover: the described columns are {fields}")
    _raises("SaasSetupRefused", lambda: module.describe(session, parts, dial, "t = country | nosuchmetric"))
    _log("ga4 discover: the connector's report is the one object, incremental on date, keyed by its dimensions; its columns come from getMetadata and a metric the property lacks is refused")

    days = admin("ga4", action="days")["days"]
    ga4_module.ROWS_PER_PAGE = 4  # the mock has 10 rows: show the offset paging
    try:
        admin("ga4", action="clear_log")
        decl = module.declaration(session, parts, dial, GA4_REPORT)
        first = list(module.read(session, parts, dial, decl, since=None))
    finally:
        ga4_module.ROWS_PER_PAGE = 10_000
    pages = [line for line in mock_log("ga4") if "runReport" in line]
    _expect(len(first) == 10 and len(pages) == 3 and "offset=8" in pages[-1], f"ga4 load: {len(first)} rows over {len(pages)} pages ({pages})")
    _expect(first[0]["sessions_INTEGER"] in range(10, 20) and isinstance(first[0]["purchaseRevenue_CURRENCY"], float), "ga4 load: the metrics are not typed")
    _expect(pages[0].split(" ")[2].startswith("2015-08-14.."), f"ga4 load: the first run asked from {pages[0]}")
    _log("ga4 load: 10 rows (5 days x 2 countries) over 3 pages of 4 by offset, metrics typed, the first run from the start of Analytics")

    cursor = max(r["date"] for r in first)
    admin("ga4", action="put", days_ago=0, country="ID", metrics={"sessions": 99})
    admin("ga4", action="put", days_ago=2, country="US", metrics={"sessions": 55})
    admin("ga4", action="clear_log")
    lagged = cursor.subtract(seconds=decl.lag_seconds)
    again = list(module.read(session, parts, dial, decl, since=lagged))
    got = {(r["country"], r["date"].strftime("%Y%m%d")): r["sessions_INTEGER"] for r in again}
    _expect(got[("ID", days[-1])] == 99 and got[("US", days[-3])] == 55, f"ga4 reload: the corrected numbers are not in {got}")
    _expect(len(got) == 8 and min(d for _c, d in got) == days[-4], f"ga4 reload: read {sorted(got)}; expected the last four days, nothing older")
    _expect(any(f" {lagged.to_date_string()}.." in line for line in mock_log("ga4") if "runReport" in line), "ga4 reload: the request did not start at the lagged cursor")
    _log("ga4 reload: the last four days (the cursor day and the three before it) were read again with Google's corrected numbers; nothing older; merge on country + date")

    admin("ga4", action="add_dimension", name="customEvent:plan")
    both = module.describe(session, parts, dial, "t2 = country, customEvent:plan | sessions")
    _expect("customEvent:plan" in {f.name for f in both}, "ga4 schema change: the new custom dimension is not accepted")
    _log("ga4 schema change: a custom dimension added at the property is accepted by describe before any load")

    admin("ga4", action="rate_limit", count=2)
    started = time.time()
    _expect(len(list(module.read(session, parts, dial, decl, since=None))) == 10, "ga4 rate limit: the report did not come back after the waits")
    _expect(time.time() - started >= 1.5, "ga4 rate limit: the Retry-After was not waited")
    admin("ga4", action="rate_limit", count=50)
    exc = _raises("SaasRateLimited", lambda: list(module.read(session, parts, dial, decl, since=None)))
    _expect(ingest_errors.classify(exc).error_class == "rate_limited", "ga4 rate limit: the run report class")
    admin("ga4", action="rate_limit", count=0)
    _log("ga4 rate limit: 429 with Retry-After waited out; a limit that does not lift fails as rate_limited")

    admin("ga4", action="forbid", on=True)
    exc = _raises("SaasPermissionMissing", lambda: list(module.read(session, parts, dial, decl, since=None)))
    _expect(ingest_errors.classify(exc).error_class == "permission_missing", "ga4 forbidden: the run report class")
    admin("ga4", action="forbid", on=False)

    def key_read() -> None:
        k_module, _v, k_parts, k_session, k_dial = _ga4("integration")
        list(k_module.read(k_session, k_parts, k_dial, k_module.declaration(k_session, k_parts, k_dial, GA4_REPORT), since=None))

    # The mock checks the signature for real: another key's public half must refuse the assertion.
    from cryptography.hazmat.primitives.asymmetric import rsa

    other = rsa.generate_private_key(public_exponent=65537, key_size=2048).public_key().public_numbers()
    admin("ga4", action="register_key", n=str(other.n), e=str(other.e))
    _raises("OAuthSignInRefused", key_read)
    GA4_KEY.clear()  # a new throwaway key, registered again
    _ga4_key_file()
    _log("ga4 service-account key: an assertion signed by a key the property does not know is refused (the mock verifies the RS256 signature)")
    admin("ga4", action="revoke", on=True)

    def browser_read() -> None:
        b_module, _v, b_parts, b_session, b_dial = _ga4("oauth")
        list(b_module.read(b_session, b_parts, b_dial, b_module.declaration(b_session, b_parts, b_dial, GA4_REPORT), since=None))

    for name, call in (("browser sign-in", browser_read), ("service-account key", key_read)):
        exc = _raises("OAuthSignInRefused", call)
        _expect(ingest_errors.classify(exc).error_class == "sign_in_refused", f"ga4 revoked ({name}): the run report class")
    admin("ga4", action="revoke", on=False)
    _log("ga4 forbidden and revoked sign-in (the refresh token and the service-account key): the fixed failures, none of the vendor's words")


# ---- the product steps (slice 2 review BLOCKER 4) ------------------------------------------------
#
# AD9 and the SRC-9 registry want connect, discover, load and reload to be `real`, which means
# THROUGH THE PRODUCT, not through the vendor module alone: the connector is created through the API
# (a type that is `supported = false` is not refused), the API's connection test dials the mock, the
# discovery routes run on the orchestrator, and `ingest_job` loads into Bronze Iceberg, which the
# gate reads back through ClickHouse. Built once here for every vendor: a vendor's slice adds an
# entry to `PRODUCT_VENDORS` and its mock. The browser sign-in cannot be driven without a browser, so
# it stays a vendor-level step above; these steps sign in with the vendor's integration credential.

def _servicenow(auth: str = "integration"):  # noqa: ANN202
    from dispar_orchestrate.adapters import saas as saas_adapter
    from dispar_orchestrate.saas.vendors import servicenow as _module  # noqa: F401 -- imported for its registration

    if auth == "oauth":
        dial = {"vendor": "servicenow", "account": "acme", "auth": {"type": "oauth2_authorization_code", "clientId": "stub-client"}}
        secrets = {"clientSecret": "stub-client-secret", "refreshToken": "stub-refresh-token"}
    else:
        dial = {"vendor": "servicenow", "account": "acme", "auth": {"type": "integration"}}
        secrets = {"clientId": "snow-client", "clientSecret": "snow-secret"}
    module, vendor, parts, session = saas_adapter.open_session(dial, secrets)
    return module, vendor, parts, session, dial


def step_servicenow() -> None:
    """ServiceNow (V-ServiceNow.4): connect (client credentials and the browser sign-in), discover
    (suggested tables, how a table loads, the dictionary's fields), load (paged by the Link header),
    reload (incremental by sys_updated_on), a new field, rate limit, forbidden and revoked sign-in."""
    from dispar_orchestrate import ingest_errors

    admin("servicenow", action="reset")

    for auth in ("integration", "oauth"):
        _m, vendor, _parts, session, _dial = _servicenow(auth)
        who = session.get_json(vendor.whoami["path"])
        _expect(len(who.get("result") or []) == 1, f"servicenow connect ({auth}): who-am-i answered {who!r}")
    _log("servicenow connect: the client-credentials grant and the browser sign-in (refresh grant) both reach the instance")

    module, _vendor, parts, session, dial = _servicenow()
    declared = {d.name: d for d in module.list_objects(session, parts, dial)}
    _expect({"incident", "change_request", "problem", "sys_user", "cmdb_ci"} <= set(declared), "servicenow discover: the suggested tables are not listed")
    _expect(module.declaration(session, parts, dial, "incident").load_description() == "incremental", "servicenow discover: incident should load incrementally")
    fields = {f.name: f.type_name for f in module.describe(session, parts, dial, "incident")}
    _expect({"number", "short_description", "sys_id", "state"} <= set(fields) and fields["sys_updated_on"] == "timestamp", f"servicenow discover: the dictionary fields are {fields}")
    _log(f"servicenow discover: {len(declared)} suggested tables, incident described with {len(fields)} fields (its parent table's included)")

    admin("servicenow", action="clear_log")
    first = _read(module, parts, session, dial, "incident")
    _expect([r["number"] for r in first] == ["INC0000001", "INC0000002", "INC0000003"], f"servicenow load: first run read {[r['number'] for r in first]}")
    _expect(sum(1 for line in mock_log("servicenow") if line.startswith("GET /api/now/table/incident")) >= 3, "servicenow load: the Link header was not followed (the sample plus two pages)")
    _log("servicenow load: 3 incidents read over 2 pages followed by the Link header")

    cursor = max(r["sys_updated_on"] for r in first) - timedelta(seconds=1)  # the lag is applied by dlt; here the cursor is given as is
    admin("servicenow", action="put", id=2, fields={"short_description": "incident 2 changed"})
    admin("servicenow", action="put", id=4)
    admin("servicenow", action="clear_log")
    again = _read(module, parts, session, dial, "incident", since=cursor)
    ids = sorted(r["number"] for r in again)
    _expect(ids == ["INC0000002", "INC0000003", "INC0000004"], f"servicenow reload: read {ids}, expected the changed (2), the cursor's own (3) and the new (4)")
    _expect(any("sys_updated_on>=" in unquote_plus(line) for line in mock_log("servicenow")), "servicenow reload: no filtered request")
    _expect(not declared["incident"].deletes, "servicenow reload: a delete is claimed")
    _log("servicenow reload: only the changed, the new and the cursor record came back (sys_updated_on filter); no delete is claimed")

    before = set(fields)
    admin("servicenow", action="add_field", name="u_region")
    after = {f.name for f in module.describe(session, parts, dial, "incident")}
    _expect(after - before == {"u_region"}, f"servicenow schema change: describe differs by {after - before}")
    _log("servicenow schema change: the new field shows in describe before any load (the dictionary)")

    admin("servicenow", action="rate_limit", count=2)
    started = time.time()
    _expect(len(_read(module, parts, session, dial, "sys_user")) == 1, "servicenow rate limit: the read did not come back after the waits")
    _expect(time.time() - started >= 1.5, "servicenow rate limit: the Retry-After was not waited")
    admin("servicenow", action="rate_limit", count=50)
    exc = _raises("SaasRateLimited", lambda: _read(module, parts, session, dial, "sys_user"))
    _expect(ingest_errors.classify(exc).error_class == "rate_limited", "servicenow rate limit: the run report class")
    admin("servicenow", action="rate_limit", count=0)
    _log("servicenow rate limit: 429 with Retry-After waited out; a limit that does not lift fails as rate_limited")

    admin("servicenow", action="forbid", on=True)
    exc = _raises("SaasPermissionMissing", lambda: _read(module, parts, session, dial, "sys_user"))
    _expect(ingest_errors.classify(exc).error_class == "permission_missing", "servicenow forbidden: the run report class")
    _expect(module.describe(session, parts, dial, "incident") is None, "servicenow forbidden: describe should fall back to the after-load path")
    admin("servicenow", action="forbid", on=False)
    admin("servicenow", action="revoke", on=True)
    exc = _raises("OAuthSignInRefused", lambda: _read(module, parts, session, dial, "sys_user"))
    _expect(ingest_errors.classify(exc).error_class == "sign_in_refused", "servicenow revoked: the run report class")
    admin("servicenow", action="revoke", on=False)
    _log("servicenow forbidden and revoked sign-in: the fixed failures, none of the vendor's words")


def _confluence(auth: str = "integration"):  # noqa: ANN202
    from dispar_orchestrate.adapters import saas as saas_adapter
    from dispar_orchestrate.saas.vendors import confluence as _module  # noqa: F401 -- imported for its registration

    if auth == "oauth":
        dial = {"vendor": "confluence", "account": "acme", "auth": {"type": "oauth2_authorization_code", "clientId": "stub-client"}}
        # Atlassian rotates the refresh token on every refresh: ask the mock which one is current.
        secrets = {"clientSecret": "stub-client-secret", "refreshToken": admin("confluence", action="refresh_token")["refresh_token"]}
    else:
        dial = {"vendor": "confluence", "account": "acme", "auth": {"type": "integration"}}
        secrets = {"email": "agent@example.invalid", "apiToken": "stub-api-token"}
    module, vendor, parts, session = saas_adapter.open_session(dial, secrets)
    return module, vendor, parts, session, dial


def step_confluence() -> None:
    """Confluence (V-Confluence.4): connect (email + API token at the site; the browser sign-in behind
    Atlassian's gateway, the cloud id taken from accessible-resources), discover, load (cursor paging by
    the relative _links.next), reload (newest first, stopping at the cursor), a new property, rate limit,
    forbidden and revoked sign-in."""
    from dispar_orchestrate import ingest_errors

    admin("confluence", action="reset")

    module, vendor, parts, session, dial = _confluence()
    _expect(len(session.get_json(parts.base_path + vendor.whoami["path"])["results"]) == 1, "confluence connect: the spaces call at the site")
    _b_module, b_vendor, b_parts, b_session, _b_dial = _confluence("oauth")
    _expect(b_parts.base_path == "/ex/confluence/0f1e2d3c-4b5a-4968-8776-655443322110", f"confluence connect (browser sign-in): the gateway prefix is {b_parts.base_path!r}")
    _expect(len(b_session.get_json(b_parts.base_path + b_vendor.whoami["path"])["results"]) == 1, "confluence connect (browser sign-in): the spaces call behind the gateway")
    _log("confluence connect: the email and API token reach the site; the browser sign-in (JSON refresh grant, the refresh token rotated) finds its cloud id and reaches the gateway")

    declared = {d.name: d for d in module.list_objects(session, parts, dial)}
    _expect(set(declared) == {"spaces", "pages", "blogposts", "comments"}, f"confluence discover: {sorted(declared)}")
    _expect(declared["pages"].load_description() == "incremental" and declared["spaces"].load_description().startswith("re-read in full: "), "confluence discover: how each object loads")
    _log("confluence discover: spaces (re-read in full), pages, blogposts and comments (incremental)")

    admin("confluence", action="clear_log")
    first = _read(module, parts, session, dial, "pages")
    _expect([r["id"] for r in first] == ["1005", "1004", "1003", "1002", "1001"], f"confluence load: first run read {[r['id'] for r in first]}")
    _expect(len([line for line in mock_log("confluence") if "/pages" in line]) == 3, "confluence load: the relative _links.next was not followed to the third page")
    comments = _read(module, parts, session, dial, "comments")
    _expect([(c["id"], c["comment_type"]) for c in comments] == [("5001", "footer")], f"confluence load: comments {comments!r}")
    _log("confluence load: 5 pages over 3 cursor pages, newest first, with their bodies; the comment told apart as footer")

    cursor = max(r["modified_at"] for r in first)  # the lag is dlt's; the cursor is given as is
    admin("confluence", action="put", id=2, fields={"title": "page 2 changed"})
    admin("confluence", action="put", id=6)
    admin("confluence", action="clear_log")
    again = _read(module, parts, session, dial, "pages", since=cursor - timedelta(seconds=1))
    _expect(sorted(r["id"] for r in again) == ["1002", "1005", "1006"], f"confluence reload: read {[r['id'] for r in again]}, expected the new (6), the changed (2) and the cursor's own (5)")
    _expect(len([line for line in mock_log("confluence") if "/pages" in line]) == 2, "confluence reload: it did not stop near the cursor")
    _expect(not declared["pages"].deletes, "confluence reload: a delete is claimed")
    _log("confluence reload: newest first and stopped at the cursor after 2 of 3 pages; the changed, the new and the cursor record; no delete is claimed")

    admin("confluence", action="add_field", name="labelCount")
    _expect(any("labelCount" in r for r in _read(module, parts, session, dial, "pages")), "confluence schema change: the new property is not in the records")
    _log("confluence schema change: the new property arrives in the records; Confluence has no describe, so the observer reports it after the load (decision 9)")

    admin("confluence", action="rate_limit", count=2)
    started = time.time()
    _expect(len(_read(module, parts, session, dial, "spaces")) == 1, "confluence rate limit: the read did not come back after the waits")
    _expect(time.time() - started >= 1.5, "confluence rate limit: the Retry-After was not waited")
    admin("confluence", action="rate_limit", count=50)
    exc = _raises("SaasRateLimited", lambda: _read(module, parts, session, dial, "spaces"))
    _expect(ingest_errors.classify(exc).error_class == "rate_limited", "confluence rate limit: the run report class")
    admin("confluence", action="rate_limit", count=0)
    _log("confluence rate limit: 429 with Retry-After waited out; a limit that does not lift fails as rate_limited")

    admin("confluence", action="forbid", on=True)
    exc = _raises("SaasPermissionMissing", lambda: _read(module, parts, session, dial, "spaces"))
    _expect(ingest_errors.classify(exc).error_class == "permission_missing", "confluence forbidden: the run report class")
    admin("confluence", action="forbid", on=False)
    admin("confluence", action="revoke", on=True)
    exc = _raises("SaasSignInRefused", lambda: _read(module, parts, session, dial, "spaces"))
    _expect(ingest_errors.classify(exc).error_class == "sign_in_refused", "confluence revoked: the run report class")

    def browser_open() -> None:
        _confluence("oauth")

    exc = _raises("OAuthSignInRefused", browser_open)
    _expect(ingest_errors.classify(exc).error_class == "sign_in_refused", "confluence revoked: the browser sign-in's run report class")
    admin("confluence", action="revoke", on=False)
    _log("confluence forbidden and revoked sign-in: the fixed failures, none of the vendor's words")


def _ga4_row_ids() -> set[str]:
    """`country|date` of every row the mock holds, read from the mock when the step needs it."""
    days = admin("ga4", action="days")["days"]
    return {f"{c}|{d[:4]}-{d[4:6]}-{d[6:]}" for d in days for c in ("ID", "US")}


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
    # ServiceNow: the client-credentials way (the API's connection test exchanges them for a token); the browser
    # sign-in is covered by the vendor-level step. Incident ids are ServiceNow's 32-character sys_id.
    "servicenow": {
        "type": "ServiceNow", "host": "acme.service-now.com", "account": "acme", "label": "ServiceNow",
        "credential": {"source": "file", "primary": "access_key", "secondary": "token"},
        "values": {"primary": "snow-client", "secondary": "snow-secret"},
        "object": "incident", "target": "saas_gate_servicenow_incident", "cursor": "sys_updated_on", "text": "short_description", "key": "sys_id",
        "columns": "sys_id, short_description, sys_updated_on", "order": "sys_id",
        "row_id": lambda r: str(r["sys_id"]),
        # the dictionary describes the table (parents included) before the load (decision 9)
        "preview_columns": {"sys_id", "number", "short_description"},
        "first_ids": {f"snow{i:028d}" for i in (1, 2, 3)}, "reload_ids": {f"snow{i:028d}" for i in (1, 2, 3, 4)},
        "changed_id": f"snow{2:028d}",
        "changes": [
            {"action": "put", "id": 2, "fields": {"short_description": "incident 2 changed"}},
            {"action": "put", "id": 4},
        ],
        "reads_marker": ("%3E%3D",),  # the url-encoded `>=` of the sys_updated_on filter
    },
    # Confluence: the email + API token way (the API's connection test sends Basic with the plain email); the
    # browser sign-in behind Atlassian's gateway is covered by the vendor-level step (its connection test is
    # honestly "not testable"). Pages have no describe, so the preview is honestly unsupported.
    "confluence": {
        "type": "Confluence", "host": "acme.atlassian.net", "account": "acme", "label": "Confluence",
        "credential": {"source": "file", "primary": "access_key", "secondary": "token"},
        "values": {"primary": "agent@example.invalid", "secondary": "stub-api-token"},
        "object": "pages", "target": "saas_gate_confluence_pages", "cursor": "modified_at", "text": "title", "key": "id",
        "columns": "id, title, modified_at", "preview_columns": None,
        "first_ids": {str(1000 + i) for i in range(1, 6)}, "reload_ids": {str(1000 + i) for i in range(1, 7)},
        "changed_id": "1002",
        "changes": [
            {"action": "put", "id": 2, "fields": {"title": "page 2 changed"}},
            {"action": "put", "id": 6},
        ],
        "reads_marker": ("/pages",), "max_reads": 2,  # newest first, stopped at the cursor: 2 of 3 pages
    },
    # Salesforce offers the browser sign-in only (decisions 3 and 4), so the connector holds the app's client
    # secret and a refresh token as files, as `g6-oauth` does, and the product refreshes it at the mock.
    "salesforce": {
        "type": "Salesforce", "host": "acme.my.salesforce.com", "account": "acme", "label": "Salesforce",
        "auth": {"type": "oauth2_authorization_code", "clientId": "stub-client"},
        "credential": {"source": "file", "primary": "secret_key", "secondary": "token"},
        "values": {"primary": "stub-client-secret", "secondary": "stub-refresh-token"},
        "object": "Account", "target": "saas_gate_salesforce_account", "cursor": "SystemModstamp", "text": "name", "key": "Id",
        "columns": "id, name, is_deleted",
        "preview_columns": {"Id", "Name", "IsDeleted"},
        "first_ids": {"001STUB001", "001STUB002", "001STUB003"},
        "reload_ids": {"001STUB001", "001STUB002", "001STUB003", "001STUB004"},
        "changes": [
            {"action": "put", "id": 2, "fields": {"Name": "account 2 changed"}},
            {"action": "put", "id": 4, "fields": {"Name": "account 4 new"}},
            {"action": "delete", "id": 3},
        ],
        "reads_marker": ("queryAll",),
    },
    # Google Analytics 4: the object is a report the connector defines; the sign-in is the browser's (the API's
    # connection test cannot present a service-account key yet, and says so), with the app's secret and a
    # refresh token as files. A report has no id column: its key is country + date.
    "ga4": {
        "type": "Google Analytics 4", "host": "analyticsdata.googleapis.com", "account": "123", "label": "Google Analytics 4",
        "auth": {"type": "oauth2_authorization_code", "clientId": "stub-client"},
        "dial_extra": {"objects": [GA4_REPORT]},
        "credential": {"source": "file", "primary": "secret_key", "secondary": "token"},
        "values": {"primary": "stub-client-secret", "secondary": "stub-refresh-token"},
        "object": GA4_REPORT, "target": "saas_gate_ga4_traffic", "cursor": "date", "text": "country", "primary_key": ["country", "date"],
        "columns": "country, date, sessions_integer", "order": "country, date",
        "row_id": lambda r: f"{r['country']}|{str(r['date'])[:10]}",
        "preview_columns": {"country", "date", "sessions_INTEGER", "purchaseRevenue_CURRENCY"},
        "first_ids": lambda: _ga4_row_ids(), "reload_ids": lambda: _ga4_row_ids(),
        "changes": [
            {"action": "put", "days_ago": 0, "country": "ID", "metrics": {"sessions": 99}},
            {"action": "put", "days_ago": 2, "country": "US", "metrics": {"sessions": 55}},
        ],
        "reload_ready": lambda rows: any(r["sessions_integer"] == 99 for r in rows),
        "reads_marker": ("runReport",),
    },
    # Slice 3 review BLOCKER 10: the other signed way into GA4, the service-account key. The API's connection test
    # exchanges an RS256 assertion for a token (the mock checks the signature against the public half of the key made
    # for this step), and a load does the same in the orchestrator.
    "ga4-key": {
        "vendor": "ga4",
        "type": "Google Analytics 4", "host": "analyticsdata.googleapis.com", "account": "123", "label": "Google Analytics 4 (service-account key)",
        "auth": {"type": "integration"},
        "dial_extra": {"objects": [GA4_REPORT]},
        "credential": {"source": "file", "primary": "secret_key"},
        "values": {"primary": lambda: (GA4_KEY.clear(), _ga4_key_file())[1]},
        "object": GA4_REPORT, "target": "saas_gate_ga4_key_traffic", "cursor": "date", "text": "country", "primary_key": ["country", "date"],
        "columns": "country, date, sessions_integer", "order": "country, date",
        "row_id": lambda r: f"{r['country']}|{str(r['date'])[:10]}",
        "preview_columns": {"country", "date", "sessions_INTEGER", "purchaseRevenue_CURRENCY"},
        "first_ids": lambda: _ga4_row_ids(), "reload_ids": lambda: _ga4_row_ids(),
        "changes": [
            {"action": "put", "days_ago": 0, "country": "ID", "metrics": {"sessions": 99}},
            {"action": "put", "days_ago": 2, "country": "US", "metrics": {"sessions": 55}},
        ],
        "reload_ready": lambda rows: any(r["sessions_integer"] == 99 for r in rows),
        "reads_marker": ("runReport",),
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


def _bronze_rows(target: str, columns: str, order: str = "toString(id)") -> list[dict]:
    """The rows of `bronze.<target>` through ClickHouse, ordered by key, under a `WHERE` (R11)."""
    matrix._bronze_row_count(target)  # creates the catalog database
    text = matrix.ch_query(
        f"SELECT {columns} FROM {matrix.CATALOG_DB}.`bronze.{target}` WHERE 1 "
        f"ORDER BY {order} SETTINGS allow_database_iceberg = 1 FORMAT JSONEachRow"
    )
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def _run_and_wait(connector_id: str, label: str) -> None:
    run_id = matrix._post_ingest_run(connector_id).json().get("runId")
    if not run_id:
        raise SaasGateFailure(f"{label}: ingest/run returned no runId")
    matrix.step_wait_for_run_success(run_id)


def step_product(vendor_id: str) -> None:
    """connect, discover, load, reload and a revoked sign-in for one vendor, through the product."""
    product_key = vendor_id
    spec = PRODUCT_VENDORS[product_key]
    vendor_id = spec.get("vendor", product_key)  # a second way of signing in to one vendor is its own entry
    label, obj, target, text = spec["label"], spec["object"], spec["target"], spec["text"]
    started = time.time()

    created = matrix._create_connector(name=f"g6-saas-{product_key}", kind=spec["type"], host=spec["host"], credential=spec["credential"])
    cid = created["id"]
    derived = created.get("credential") or {}
    for slot in ("primary", "secondary"):
        value = spec["values"].get(slot)
        if value:
            matrix._write_credential_file(derived[slot], value() if callable(value) else value)
    resp = matrix.API.put(
        f"{matrix.API_URL}/api/connectors/{cid}/ingest-spec",
        json={
            "adapter": "saas", "ingestMode": "batch",
            "dial": {"vendor": vendor_id, "account": spec["account"], "auth": spec.get("auth", {"type": "integration"}), **spec.get("dial_extra", {})},
            "sourceObjects": [{"name": obj, "target": target, "loadMode": "incremental_dedup", "incrementalKey": spec["cursor"], "primaryKey": spec.get("primary_key") or [spec.get("key", "id")]}],
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
    if vendor_id == "ga4":
        # Slice 3 review BLOCKER 9: the console's report editor lists the property's own dimensions and metrics.
        fields = matrix.discovery_answer(cid, "list", {"schema": "fields"})
        kinds = {(o.get("name"), o.get("kind")) for o in fields.get("objects") or []}
        _expect(fields.get("state") == "done" and {("country", "dimension"), ("date", "dimension"), ("sessions", "metric")} <= kinds, f"ga4 discover: the fields list is {fields.get('state')!r} {sorted(kinds)[:8]}")
        _log(f"ga4 discover: {len(kinds)} dimensions and metrics of the property listed through discovery for the report editor")

    # load: ingest_job into Bronze Iceberg, read back.
    _run_and_wait(cid, f"{vendor_id} load")
    columns = spec.get("columns") or f"id, {text}" + (", status" if vendor_id == "zendesk" else "")
    order = spec.get("order", "toString(id)")
    row_id = spec.get("row_id", lambda r: str(r["id"]))  # a report has no id column: its key is its dimensions
    wanted_first = spec["first_ids"]() if callable(spec["first_ids"]) else spec["first_ids"]
    rows = _poll(f"{vendor_id} load: Bronze {target}", lambda: _bronze_rows(target, columns, order), lambda r: len(r) == len(wanted_first))
    _expect({row_id(r) for r in rows} == wanted_first, f"{vendor_id} load: Bronze holds ids {sorted(row_id(r) for r in rows)}")
    _log(f"{vendor_id} load: {len(rows)} rows in Bronze {target}, read back through ClickHouse, each once")

    # reload: the vendor changes; only the changed and new records are read, merged on the key.
    if vendor_id == "hubspot":
        admin("hubspot", action="put", object="deals", id=2, properties={"dealname": "deal 2 changed"})
        admin("hubspot", action="put", object="deals", id=4, properties={"dealname": "deal 4 new"})
        admin("hubspot", action="archive", object="deals", id=1)
    elif "changes" in spec:
        for change in spec["changes"]:
            admin(vendor_id, **change)
    else:
        admin("zendesk", action="put", id=2, fields={"subject": "ticket 2 changed"})
        admin("zendesk", action="put", id=7, fields={"subject": "ticket 7 new"})
        admin("zendesk", action="delete", id=3)
    admin(vendor_id, action="clear_log")
    _run_and_wait(cid, f"{vendor_id} reload")
    wanted_reload = spec["reload_ids"]() if callable(spec["reload_ids"]) else spec["reload_ids"]
    ready = spec.get("reload_ready") or (lambda r: any("changed" in str(x[text]) for x in r))
    after = _poll(
        f"{vendor_id} reload: Bronze {target}", lambda: _bronze_rows(target, columns, order),
        lambda r: {row_id(x) for x in r} == wanted_reload and ready(r),
    )
    ids = [row_id(r) for r in after]
    _expect(len(ids) == len(set(ids)), f"{vendor_id} reload: a key is in Bronze twice: {ids}")
    by_id = {row_id(r): r for r in after}
    if vendor_id != "ga4":
        changed_id = spec.get("changed_id") or ("001STUB002" if vendor_id == "salesforce" else "2")
        _expect("changed" in str(by_id[changed_id][text]), f"{vendor_id} reload: record {changed_id} was not updated")
    if vendor_id == "ga4":
        days = admin("ga4", action="days")["days"]
        key = lambda country, back: f"{country}|{days[-1 - back][:4]}-{days[-1 - back][4:6]}-{days[-1 - back][6:]}"  # noqa: E731
        _expect(by_id[key("ID", 0)]["sessions_integer"] == 99 and by_id[key("US", 2)]["sessions_integer"] == 55, "ga4 reload: Google's corrected numbers are not in Bronze")
        _expect(len(by_id) == 10, "ga4 reload: a row was lost or stored twice")
        starts = [line.split(" ")[2].split("..")[0] for line in mock_log("ga4") if "runReport" in line]
        _expect(starts and all(start != "2015-08-14" for start in starts), f"ga4 reload: the run read from the start of Analytics instead of the lagged cursor ({starts})")
    elif vendor_id == "salesforce":
        _expect(by_id["001STUB003"]["is_deleted"] in (True, 1, "true"), "salesforce reload: the deleted account does not say is_deleted in Bronze")
        _expect(by_id["001STUB001"]["is_deleted"] in (False, 0, "false") and len(by_id) == 4, "salesforce reload: an account was lost or marked")
    elif vendor_id == "confluence":
        _expect(len(by_id) == 6, "confluence reload: a page was lost or stored twice")
    elif vendor_id == "servicenow":
        _expect(len(by_id) == 4, "servicenow reload: an incident was lost or stored twice")
    elif vendor_id == "zendesk":
        _expect(by_id["3"]["status"] == "deleted", "zendesk reload: the deleted ticket does not say status = deleted in Bronze")
        _expect(len(by_id) == 7, "zendesk reload: a ticket was lost")
    else:
        _expect("1" in by_id, "hubspot reload: the archived deal vanished; no delete is claimed, so it must stay")
    seen = mock_log(vendor_id)
    reads = [line for line in seen if any(m in line for m in spec.get("reads_marker", ("/search", "cursor", "incremental")))]
    _expect(reads, f"{vendor_id} reload: the mock saw no incremental request ({seen})")
    if spec.get("max_reads"):
        _expect(len(reads) <= spec["max_reads"], f"{vendor_id} reload: {len(reads)} list requests; an incremental read stops near the cursor ({reads})")
    _expect(not any(line == "GET /crm/v3/objects/deals" for line in seen), f"{vendor_id} reload: it listed every deal again")
    _log(f"{vendor_id} reload: "
         + {
             "zendesk": "the changed and the new record merged on the key, no duplicate; the deleted ticket says status = deleted",
             "salesforce": "the changed and the new record merged on the key, no duplicate; the deleted account says is_deleted (queryAll)",
             "confluence": "the changed and the new page merged on id, no duplicate; the read stopped near the cursor (no delete is claimed)",
             "servicenow": "the changed and the new incident merged on sys_id, no duplicate (no delete is claimed)",
             "ga4": "the corrected last days merged on country + date, no row twice, the run starting at the lagged cursor",
         }.get(vendor_id, "the changed and the new record merged on the key, no duplicate; the archived deal stays (HubSpot reports no deletes)"))

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
    step_salesforce()
    step_ga4()
    step_servicenow()
    step_confluence()


def run() -> None:
    matrix.step_wait_for_services()  # ClickHouse, the API, Dagster and its code location
    matrix.step_login()
    matrix.step_ensure_tenant()
    step_stub_is_up()
    step_types_are_listed_unsupported()
    _setup_vendor_environment()
    for product_key, spec in PRODUCT_VENDORS.items():
        admin(spec.get("vendor", product_key), action="reset")
        step_product(product_key)
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
