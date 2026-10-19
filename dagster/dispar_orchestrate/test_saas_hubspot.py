"""`saas/vendors/hubspot.py` (`SRC-3` V-HubSpot.2): objects, the properties-API
describe, full and incremental reads, through the real `SaasSession` over a
FAKE HubSpot (the properties, list, search and owners endpoints as HubSpot
documents them). No network. The loader test runs `load_via_sink` for real on
`test_load_modes_measurement.Lab`; CI runs it in `dagster-loader-tests`."""

from __future__ import annotations

import time
from datetime import datetime, timezone
from typing import Any
from urllib.parse import parse_qs, urlparse

import pytest

from dispar_orchestrate import schema_observer, ssrf_guard
from dispar_orchestrate import ingest_factory as f
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters.sink import load_via_sink
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.vendors import hubspot
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture

ORIGIN = "https://api.hubapi.com"
DIAL = {"vendor": "hubspot", "account": "123456", "auth": {"type": "integration"}}
SECRETS = {"token": "not-a-real-token"}


class Reply:
    def __init__(self, status: int = 200, body: Any = None, headers: dict | None = None) -> None:
        self.status_code = status
        self._body = body if body is not None else {}
        self.headers = headers or {}

    def json(self) -> Any:
        return self._body


def iso(ms: int) -> str:
    return datetime.fromtimestamp(ms / 1000, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.") + f"{ms % 1000:03d}Z"


DEAL_PROPS = {
    "amount": "number",
    "closedate": "datetime",
    "createdate": "datetime",
    "dealname": "string",
    "dealstage": "enumeration",
    "hs_lastmodifieddate": "datetime",
    "hs_object_id": "string",
    "pipeline": "string",
    "is_renewal": "bool",  # a custom property
}
CONTACT_PROPS = {
    "createdate": "datetime", "email": "string", "firstname": "string", "hs_object_id": "string",
    "lastmodifieddate": "datetime", "lastname": "string",
}  # fmt: skip


class FakeHubspot:
    """HubSpot as documented: every property is text, an empty text for no value;
    list pages carry `paging.next.link`; search takes a GTE filter."""

    def __init__(self) -> None:
        self.deals: dict[str, dict[str, Any]] = {}
        self.contacts: dict[str, dict[str, Any]] = {}
        self.owners = [{"id": "1", "email": "a@example.invalid", "updatedAt": iso(5_000)}]
        self.log: list[tuple[str, str]] = []
        self.headers: list[dict] = []
        self.status_for: dict[str, int] = {}

    def put(self, store: dict[str, dict[str, Any]], rec_id: str, ms: int, cursor: str = "hs_lastmodifieddate", **props: Any) -> None:
        store[rec_id] = {"hs_object_id": rec_id, cursor: iso(ms), **{k: str(v) if v is not None else "" for k, v in props.items()}}

    def _store(self, plural: str) -> dict[str, dict[str, Any]]:
        return {"deals": self.deals, "contacts": self.contacts}[plural]

    def _result(self, rec_id: str, row: dict[str, Any], wanted: list[str]) -> dict[str, Any]:
        return {"id": rec_id, "properties": {k: row.get(k, "") for k in wanted if k in row or k.startswith("hs_")}, "archived": False}

    def get(self, url: str, *, headers: dict, params: dict | None, timeout: float) -> Reply:
        self.headers.append(headers)
        parsed = urlparse(url)
        query = {**{k: v[0] for k, v in parse_qs(parsed.query).items()}, **(params or {})}
        path = parsed.path
        self.log.append(("GET", path))
        if path in self.status_for:
            return Reply(self.status_for[path], {"message": "VENDOR-WORDS"})
        if path.startswith("/crm/v3/properties/"):
            table = {"deals": DEAL_PROPS, "contacts": CONTACT_PROPS}[path.rsplit("/", 1)[1]]
            return Reply(body={"results": [{"name": n, "type": t} for n, t in table.items()]})
        if path == "/crm/v3/owners/":
            return Reply(body={"results": self.owners})
        if path.startswith("/crm/v3/objects/"):
            store = self._store(path.rsplit("/", 1)[1])
            wanted = query["properties"].split(",")
            ids = sorted(store)
            start = int(query.get("after", 0))
            limit = int(query["limit"])
            body: dict[str, Any] = {"results": [self._result(i, store[i], wanted) for i in ids[start : start + limit]]}
            if start + limit < len(ids):
                body["paging"] = {"next": {"after": str(start + limit), "link": f"{ORIGIN}{path}?after={start + limit}&properties={query['properties']}&limit={limit}"}}
            return Reply(body=body)
        return Reply(404)

    def post(self, url: str, *, headers: dict, json: Any, timeout: float) -> Reply:
        self.headers.append(headers)
        path = urlparse(url).path
        self.log.append(("POST", path))
        assert path.startswith("/crm/v3/objects/") and path.endswith("/search"), path
        store = self._store(path.split("/")[4])
        flt = json["filterGroups"][0]["filters"][0]
        floor = int(flt["value"])
        hits = sorted(
            (i for i, r in store.items() if int(datetime.fromisoformat(r[flt["propertyName"]].replace("Z", "+00:00")).timestamp() * 1000) >= floor),
            key=lambda i: store[i][flt["propertyName"]],
        )
        self.searched = hits
        return Reply(body={"total": len(hits), "results": [self._result(i, store[i], json["properties"]) for i in hits]})


def resolver(host: str, port: int, **_kw: object) -> ssrf_guard.ResolvedAddress:
    return ssrf_guard.ResolvedAddress(ip="93.184.216.34", port=port, family=2)


@pytest.fixture
def api() -> FakeHubspot:
    return FakeHubspot()


def open_(api: FakeHubspot):  # noqa: ANN202
    return saas_adapter.open_session(DIAL, SECRETS, resolve_checked=resolver, http_get=api.get, http_post=api.post, sleep=lambda _s: None)


def read(api: FakeHubspot, name: str, since: Any = None) -> list[dict]:
    module, _vendor, parts, session = open_(api)
    decl = module.declaration(session, parts, DIAL, name)
    return list(module.read(session, parts, DIAL, decl, since=since))


def test_the_vendor_module_is_found_by_the_registry() -> None:
    assert vendors.module_for("hubspot") is hubspot


def test_six_crm_objects_are_incremental_and_owners_and_pipelines_say_why_they_are_not() -> None:
    declared = {d.name: d for d in hubspot.list_objects(None, None, DIAL)}
    assert set(declared) == {"companies", "contacts", "deals", "tickets", "products", "quotes", "owners", "pipelines_deals", "pipelines_tickets"}
    assert declared["deals"].cursor == "hs_lastmodifieddate" and declared["deals"].load_description() == "incremental"
    assert declared["contacts"].cursor == "lastmodifieddate"
    for name in ("owners", "pipelines_deals", "pipelines_tickets"):
        assert declared[name].cursor is None and declared[name].load_description().startswith("re-read in full: HubSpot cannot filter")
    assert all(d.primary_key == ("id",) and d.deletes is False for d in declared.values()), "decision 7: HubSpot reports no deletes"


def test_an_unknown_object_is_a_rejected_setting_and_never_echoed() -> None:
    with pytest.raises(saas_adapter.SaasSetupRefused) as raised:
        hubspot.declaration(None, None, DIAL, "../../etc/passwd")
    assert "passwd" not in str(raised.value)


def test_describe_lists_the_properties_api_fields_with_dlt_types_and_the_id(api: FakeHubspot) -> None:
    module, _v, parts, session = open_(api)
    fields = {f_.name: f_ for f_ in module.describe(session, parts, DIAL, "deals")}
    assert fields["id"].nullable is False
    assert fields["amount"].type_name == "double" and fields["is_renewal"].type_name == "bool" and fields["closedate"].type_name == "timestamp"
    assert module.describe(session, parts, DIAL, "owners") is None, "no properties API for owners: seen after the load"
    assert all(h == {"Authorization": "Bearer not-a-real-token"} for h in api.headers), "the integration token, nothing else"


def test_a_new_hubspot_property_shows_up_in_the_next_describe(api: FakeHubspot) -> None:
    module, _v, parts, session = open_(api)
    before = {f_.name for f_ in module.describe(session, parts, DIAL, "deals")}
    DEAL_PROPS["region"] = "string"
    try:
        after = {f_.name for f_ in module.describe(session, parts, DIAL, "deals")}
    finally:
        del DEAL_PROPS["region"]
    assert after - before == {"region"}


def test_a_first_run_lists_everything_and_types_the_values(api: FakeHubspot) -> None:
    api.put(api.deals, "1", 1_000_000, dealname="Big", amount="10.5", is_renewal="true", closedate="")
    api.put(api.deals, "2", 2_000_000, dealname="Small", amount="3", is_renewal="false")
    records = read(api, "deals")
    assert [r["id"] for r in records] == ["1", "2"] and ("POST", "/crm/v3/objects/deals/search") not in api.log
    one = records[0]
    assert one["amount"] == 10.5 and one["is_renewal"] is True and one["closedate"] is None
    assert records[1]["is_renewal"] is False


def test_a_later_run_searches_from_the_cursor_with_a_lag_and_reads_only_what_changed(api: FakeHubspot) -> None:
    for i in range(1, 6):
        api.put(api.deals, str(i), 10_000_000 + i * 600_000, dealname=f"d{i}", amount=i)
    cursor = iso(10_000_000 + 3 * 600_000)  # the last value of an earlier run: deal 3
    records = read(api, "deals", since=cursor)
    assert [r["id"] for r in records] == ["3", "4", "5"], "deal 3 again (merge removes it); 1 and 2 are not read"
    # 30 seconds before the cursor is inside the lag window and is read too.
    api.put(api.deals, "2", 10_000_000 + 3 * 600_000 - 30_000, dealname="late-visible")
    assert "2" in [r["id"] for r in read(api, "deals", since=cursor)]


def test_contacts_are_searched_on_lastmodifieddate(api: FakeHubspot) -> None:
    api.put(api.contacts, "7", 9_000_000, cursor="lastmodifieddate", email="x@example.invalid")
    got = read(api, "contacts", since=iso(8_000_000))
    assert [r["id"] for r in got] == ["7"] and got[0]["lastmodifieddate"] == iso(9_000_000)


def test_owners_are_read_in_full_with_no_properties_call(api: FakeHubspot) -> None:
    assert [r["id"] for r in read(api, "owners")] == ["1"]
    assert not any(p.startswith("/crm/v3/properties") for _m, p in api.log)


@pytest.mark.parametrize("status,error", [(401, "SaasSignInRefused"), (403, "SaasPermissionMissing")])
def test_a_refusal_is_the_sessions_fixed_failure_without_hubspots_words(api: FakeHubspot, status: int, error: str) -> None:
    api.status_for["/crm/v3/owners/"] = status
    with pytest.raises(Exception) as raised:
        read(api, "owners")
    assert type(raised.value).__name__ == error and "VENDOR-WORDS" not in str(raised.value)


# ---- the whole path: adapter, dlt incremental, merge on the key, real Iceberg -------------------


def accept_all(reflected: schema_observer.ReflectedTable) -> schema_observer.Decision:
    return schema_observer.Decision(action="load", columns=None, changes=[])


def obj(**extra: Any) -> dict:
    return {"name": "deals", "target": "deals", "loadMode": "incremental_dedup", "incrementalKey": "hs_lastmodifieddate", "primaryKey": ["id"], **extra}


@pytest.mark.loader
def test_a_second_run_reads_only_the_changed_deal_and_the_table_has_no_duplicate(lab: Lab, api: FakeHubspot) -> None:
    now = int(time.time() * 1000)
    for i in range(1, 4):
        api.put(api.deals, str(i), now - 86_400_000 + i * 1000, dealname=f"d{i}", amount=i)

    def run() -> Any:
        built = saas_adapter.build_source(
            DIAL, SECRETS, [obj()], gate=accept_all, resolve_checked=resolver, http_get=api.get, http_post=api.post, sleep=lambda _s: None
        )
        assert built is not None
        return load_via_sink(built.source, "deals", lab.config, f._load_plan("saas", obj()))

    assert run().rows == 3
    time.sleep(1.1)
    api.put(api.deals, "2", now, dealname="d2-changed", amount=20)
    api.put(api.deals, "4", now + 1000, dealname="d4-new", amount=4)
    api.log.clear()
    second = run()
    assert ("POST", "/crm/v3/objects/deals/search") in api.log, "the second run searched"
    assert not any(m == "GET" and p == "/crm/v3/objects/deals" for m, p in api.log), "and did not list every deal"
    rows = {r["id"]: r["dealname"] for r in lab.rows("deals")}
    assert rows == {"1": "d1", "2": "d2-changed", "3": "d3", "4": "d4-new"}, "changed and new rows only; merged on id"
    # The search starts 60 s before the cursor (HubSpot's "few moments" lag), so it
    # also returned deals 1 and 3; the incremental merge loaded only the two that differ.
    assert api.searched == ["1", "3", "2", "4"] and second.rows == 2
    assert len(lab.rows("deals")) == 4
