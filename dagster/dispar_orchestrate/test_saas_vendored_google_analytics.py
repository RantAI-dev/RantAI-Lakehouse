"""The vendored Google Analytics source (`saas/vendored/google_analytics/`, `SRC-3`
V-GA4.1): it reads the GA4 Data API's REST endpoints (`runReport`, `getMetadata`)
only through the `SaasSession` it is given (AD3, AD4), with no `google-analytics-data`
client, no gRPC and no protobuf. No network: `FakeGa4` answers as the Data API
documents (https://developers.google.com/analytics/devguides/reporting/data/v1/rest/v1beta/properties/runReport):
int64 `offset` and `limit` are text in the JSON, a page carries `rowCount` (the total),
metric values are text with a `type` in the header."""

from __future__ import annotations

import ast
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

import pytest

from dispar_orchestrate import ssrf_guard
from dispar_orchestrate.saas.session import SaasSession
from dispar_orchestrate.saas.vendored import google_analytics as ga
from dispar_orchestrate.saas.vendored.google_analytics.helpers import data_processing
from dispar_orchestrate.test_saas_vendored_zendesk import Reply, resolver

VENDORED = Path(data_processing.__file__).parent.parent
ORIGIN = "https://analyticsdata.googleapis.com"


class FakeGa4:
    """`runReport` over a list of rows `(date, country, sessions, revenue)`, and `metadata`."""

    def __init__(self, rows: list[tuple[str, str, int, float]] | None = None) -> None:
        self.rows = rows or []
        self.posts: list[tuple[str, dict]] = []
        self.gets: list[str] = []
        self.headers: list[dict] = []

    def get(self, url: str, *, headers: dict, params: dict | None, timeout: float) -> Reply:
        self.headers.append(headers)
        path = urlparse(url).path
        self.gets.append(path)
        if path.endswith("/metadata"):
            return Reply(body={
                "name": "properties/123/metadata",
                "dimensions": [{"apiName": "date", "uiName": "Date"}, {"apiName": "country", "uiName": "Country"}],
                "metrics": [{"apiName": "sessions", "type": "TYPE_INTEGER"}, {"apiName": "purchaseRevenue", "type": "TYPE_CURRENCY"}],
            })  # fmt: skip
        return Reply(404)

    def post(self, url: str, *, headers: dict, json: Any, timeout: float) -> Reply:
        self.headers.append(headers)
        path = urlparse(url).path
        self.posts.append((path, json))
        assert path.endswith(":runReport"), path
        lo, hi = json["dateRanges"][0]["startDate"].replace("-", ""), json["dateRanges"][0]["endDate"].replace("-", "")
        hits = [r for r in self.rows if lo <= r[0] <= hi]
        start, size = int(json["offset"]), int(json["limit"])
        page = hits[start : start + size]
        return Reply(body={
            "dimensionHeaders": [{"name": n} for n in (d["name"] for d in json["dimensions"])],
            "metricHeaders": [{"name": m["name"], "type": "TYPE_INTEGER" if m["name"] == "sessions" else "TYPE_CURRENCY"} for m in json["metrics"]],
            "rows": [
                {"dimensionValues": [{"value": {"date": r[0], "country": r[1]}[d["name"]]} for d in json["dimensions"]],
                 "metricValues": [{"value": str(r[2] if m["name"] == "sessions" else r[3])} for m in json["metrics"]]}
                for r in page
            ],
            "rowCount": len(hits),
            "kind": "analyticsData#runReport",
        })  # fmt: skip


def session_over(fake: FakeGa4) -> SaasSession:
    return SaasSession(ORIGIN, lambda: {"Authorization": "Bearer t"}, http_get=fake.get, http_post=fake.post, resolve_checked=resolver, sleep=lambda _s: None)


def test_no_vendored_google_analytics_module_imports_an_http_library_a_google_client_grpc_or_protobuf() -> None:
    forbidden = {"requests", "urllib3", "httpx", "http", "urllib", "google", "grpc", "proto", "apiclient", "googleapiclient", "requests_oauthlib", "google_auth_oauthlib"}
    for path in VENDORED.rglob("*.py"):
        for node in ast.walk(ast.parse(path.read_text())):
            names = [a.name for a in node.names] if isinstance(node, ast.Import) else [node.module or ""] if isinstance(node, ast.ImportFrom) else []
            for name in names:
                assert name.split(".")[0] not in forbidden, f"{path.name} imports {name}"
    assert not (VENDORED / "setup_script_gcp_oauth.py").exists(), "an interactive local sign-in script has no place in the product"


def test_a_report_is_pages_of_run_report_posts_through_the_session_with_text_offsets() -> None:
    rows = [(f"2024010{d}", c, d * 10, d * 1.5) for d in range(1, 4) for c in ("ID", "US")]
    fake = FakeGa4(rows)
    got = list(data_processing.get_report(session_over(fake), 123, ["date", "country"], ["sessions", "purchaseRevenue"], 4, "2024-01-01", "2024-01-31"))
    assert len(got) == 6
    assert [(p, b["offset"], b["limit"]) for p, b in fake.posts] == [("/v1beta/properties/123:runReport", "0", "4"), ("/v1beta/properties/123:runReport", "4", "4")]
    assert fake.posts[0][1]["dimensions"] == [{"name": "date"}, {"name": "country"}] and fake.posts[0][1]["dateRanges"] == [{"startDate": "2024-01-01", "endDate": "2024-01-31"}]
    assert all(h == {"Authorization": "Bearer t"} for h in fake.headers)


def test_values_are_typed_as_the_verified_source_types_them() -> None:
    fake = FakeGa4([("20240102", "ID", 7, 12.5)])
    (row,) = data_processing.get_report(session_over(fake), 123, ["date", "country"], ["sessions", "purchaseRevenue"], 100, "2024-01-01", "2024-01-31")
    assert row["date"].isoformat() == "2024-01-02T00:00:00+00:00", "the date dimension is a UTC datetime"
    assert row["country"] == "ID" and row["sessions_INTEGER"] == 7 and row["purchaseRevenue_CURRENCY"] == 12.5


def test_metadata_is_read_with_a_get_through_the_session() -> None:
    fake = FakeGa4()
    meta = data_processing.get_metadata(session_over(fake), 123)
    assert fake.gets == ["/v1beta/properties/123/metadata"] and meta["metrics"][0]["apiName"] == "sessions"


def test_a_property_id_that_is_not_a_number_never_reaches_the_address() -> None:
    fake = FakeGa4()
    with pytest.raises(ValueError):
        data_processing.get_metadata(session_over(fake), "123/../x")  # type: ignore[arg-type]
    assert fake.gets == []


def test_a_response_that_names_another_host_is_not_followed() -> None:
    """The Data API has no next-page link (paging is offset and limit), so nothing in an answer
    is ever dialled; a redirect, the one way to another host, is refused by the session."""
    fake = FakeGa4([("20240102", "ID", 1, 1.0)])
    original = fake.post
    fake.post = lambda url, **kw: Reply(302, {}, {"Location": "https://evil.example.invalid/"})  # type: ignore[method-assign]
    with pytest.raises(ssrf_guard.SsrfBlocked):
        list(data_processing.get_report(session_over(fake), 123, ["date"], ["sessions"], 10, "2024-01-01", "2024-01-31"))
    fake.post = original  # type: ignore[method-assign]


def test_the_source_builds_its_resources_over_a_session_and_the_metadata_tables_are_dicts() -> None:
    fake = FakeGa4([("20240102", "ID", 1, 1.0)])
    source = ga.google_analytics(session_over(fake), property_id=123, queries=[{"resource_name": "traffic", "dimensions": ["country"], "metrics": ["sessions"]}])
    names = sorted(source.resources)
    assert names == ["dimensions", "metrics", "traffic"]
    assert [r["apiName"] for r in source.resources["metrics"]] == ["sessions", "purchaseRevenue"]
    assert [r["apiName"] for r in source.resources["dimensions"]] == ["date", "country"]
