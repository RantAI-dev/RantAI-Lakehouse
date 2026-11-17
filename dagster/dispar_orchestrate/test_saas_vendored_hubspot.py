"""The vendored HubSpot source (`saas/vendored/hubspot/`, `SRC-3` V-HubSpot.1):
its HTTP goes only through the session it is given (AD3, AD4). No network: the
session is a fake that answers from a table."""

from __future__ import annotations

import ast
from pathlib import Path
from typing import Any

from dispar_orchestrate.saas.vendored.hubspot import helpers

VENDORED = Path(helpers.__file__).parent


class FakeSession:
    def __init__(self, answers: dict[str, Any]) -> None:
        self.answers = answers
        self.asked: list[tuple[str, dict | None]] = []

    def get_json(self, target: str, params: dict | None = None) -> Any:
        self.asked.append((target, params))
        return self.answers[target]


def test_no_vendored_hubspot_module_imports_an_http_library() -> None:
    forbidden = {"requests", "urllib3", "httpx", "http", "urllib.request"}
    for path in VENDORED.glob("*.py"):
        for node in ast.walk(ast.parse(path.read_text())):
            names = [a.name for a in node.names] if isinstance(node, ast.Import) else [node.module or ""] if isinstance(node, ast.ImportFrom) else []
            for name in names:
                assert name.split(".")[0] not in forbidden and "helpers.requests" not in name, f"{path.name} imports {name}"
            if isinstance(node, ast.ImportFrom) and node.module == "dlt.sources.helpers":
                assert all(a.name != "requests" for a in node.names), f"{path.name} imports dlt's requests"


def test_fetch_data_pages_through_the_session_and_follows_the_next_link_with_it() -> None:
    first = {"results": [{"id": "1", "properties": {"name": "a"}}], "paging": {"next": {"link": "https://api.example.invalid/crm/v3/objects/deals?after=1"}}}
    second = {"results": [{"id": "2", "properties": {"name": "b"}}]}
    session = FakeSession({"/crm/v3/objects/deals": first, "https://api.example.invalid/crm/v3/objects/deals?after=1": second})
    pages = list(helpers.fetch_data("/crm/v3/objects/deals", session, params={"limit": 100}))
    assert [[o["id"] for o in page] for page in pages] == [["1"], ["2"]]
    assert session.asked == [("/crm/v3/objects/deals", {"limit": 100}), ("https://api.example.invalid/crm/v3/objects/deals?after=1", None)]


# ---- local change 2: incremental reads through the CRM search API ------------------

import pytest  # noqa: E402

from dispar_orchestrate.saas.vendored.hubspot import settings as hs_settings  # noqa: E402


def iso(ms: int) -> str:
    from datetime import datetime, timezone

    return datetime.fromtimestamp(ms / 1000, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.") + f"{ms % 1000:03d}Z"


class FakeCrm:
    """HubSpot's CRM search and batch-read as documented: GTE filter, one sort,
    `after` paging, an error past `cap` results, a 3,000-character body limit."""

    def __init__(self, records: dict[str, dict[str, Any]], cap: int = 10_000) -> None:
        self.records = records  # id -> properties (hs_lastmodifieddate as epoch ms int)
        self.cap = cap
        self.searches: list[dict] = []
        self.batches: list[dict] = []

    def _props(self, rec_id: str, names: list[str]) -> dict[str, Any]:
        row = self.records[rec_id]
        out = {n: row[n] for n in names if n in row}
        if "hs_lastmodifieddate" in names:
            out["hs_lastmodifieddate"] = iso(row["hs_lastmodifieddate"])
        return out

    def post_json(self, target: str, body: Any) -> Any:
        if target.endswith("/batch/read"):
            self.batches.append(body)
            assert len(body["inputs"]) <= 100
            return {"results": [{"id": i["id"], "properties": self._props(i["id"], body["properties"])} for i in body["inputs"]]}
        assert target.endswith("/search")
        assert len(str(body)) < 3000, "the documented body limit"
        self.searches.append(dict(body))
        flt = body["filterGroups"][0]["filters"][0]
        assert flt["operator"] == "GTE" and body["sorts"][0]["direction"] == "ASCENDING"
        floor = int(flt["value"])
        hits = sorted((i for i, r in self.records.items() if r[flt["propertyName"]] >= floor), key=lambda i: (self.records[i][flt["propertyName"]], i))
        start = int(body.get("after", 0))
        assert start < self.cap, "HubSpot answers 400 past 10,000 results"
        limit = body["limit"]
        page = hits[start : start + limit]
        out: dict[str, Any] = {"total": len(hits), "results": [{"id": i, "properties": self._props(i, body["properties"])} for i in page]}
        if start + limit < len(hits):
            out["paging"] = {"next": {"after": str(start + limit)}}
        return out


def crm(n: int, **kw: Any) -> FakeCrm:
    return FakeCrm({str(i): {"name": f"n{i}", "hs_lastmodifieddate": 1_000_000 + i * 1000} for i in range(1, n + 1)}, **kw)


def read_all(fake: FakeCrm, since_ms: int, props: list[str]) -> list[dict]:
    pages = helpers.search_data_since("/crm/v3/objects/deals", fake, "hs_lastmodifieddate", since_ms, props)
    return [o for page in pages for o in page]


def test_a_search_since_reads_only_records_modified_at_or_after_the_cursor() -> None:
    fake = crm(5)
    got = read_all(fake, 1_000_000 + 3000, ["name", "hs_lastmodifieddate"])
    assert [o["id"] for o in got] == ["3", "4", "5"]
    assert got[0] == {"name": "n3", "hs_lastmodifieddate": iso(1_003_000), "id": "3"}, "the same shape fetch_data gives"
    body = fake.searches[0]
    assert body["filterGroups"][0]["filters"][0]["value"] == "1003000" and body["limit"] == 100


def test_the_search_pages_with_after_until_the_last_page() -> None:
    fake = crm(250)
    got = read_all(fake, 0, ["name", "hs_lastmodifieddate"])
    assert len(got) == 250 and [s.get("after") for s in fake.searches] == [None, "100", "200"]


def test_at_the_ten_thousand_limit_the_search_restarts_from_the_newest_time_seen(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(helpers, "SEARCH_MAX_RESULTS", 200)
    fake = crm(450, cap=200)
    got = read_all(fake, 0, ["name", "hs_lastmodifieddate"])
    assert {o["id"] for o in got} == {str(i) for i in range(1, 451)}, "nothing is lost across the restarts"
    floors = [s["filterGroups"][0]["filters"][0]["value"] for s in fake.searches]
    assert floors[0] == "0" and floors[2] == "1200000", "the second query starts at the newest time of the first (record 200)"
    ids = [o["id"] for o in got]
    assert len(ids) > len(set(ids)), "the record at the restart instant comes twice; the merge key removes it"


def test_records_sharing_one_instant_past_the_limit_raise_instead_of_looping(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(helpers, "SEARCH_MAX_RESULTS", 200)
    fake = FakeCrm({str(i): {"name": "x", "hs_lastmodifieddate": 5_000} for i in range(300)}, cap=200)
    with pytest.raises(helpers.SearchOutOfBoundsException):
        read_all(fake, 5_000, ["name", "hs_lastmodifieddate"])


def test_properties_that_do_not_fit_the_body_limit_are_batch_read_and_merged_by_id() -> None:
    fake = crm(3)
    many = [f"custom_{i:03d}_" + "x" * 30 for i in range(120)]  # about 5,000 characters of names
    for row in fake.records.values():
        row.update({p: f"v-{p[:10]}" for p in many})
    got = read_all(fake, 0, ["name", "hs_lastmodifieddate", *many])
    assert len(fake.batches) >= 2 and all(len(b["inputs"]) == 3 for b in fake.batches)
    assert all(set(many) | {"name"} <= set(o) for o in got), "every property is on every record"
    assert "hs_lastmodifieddate" in fake.searches[0]["properties"], "the cursor is in the first request"


def test_the_cursor_property_of_contacts_is_the_one_hubspot_names_for_contacts() -> None:
    assert hs_settings.LAST_MODIFIED_PROPERTY["contact"] == "lastmodifieddate"
    assert all(v == "hs_lastmodifieddate" for k, v in hs_settings.LAST_MODIFIED_PROPERTY.items() if k != "contact")
    for kind, prop in hs_settings.LAST_MODIFIED_PROPERTY.items():
        assert prop in hs_settings.ENTITY_PROPERTIES[kind], f"{kind}: the default properties carry the cursor"
