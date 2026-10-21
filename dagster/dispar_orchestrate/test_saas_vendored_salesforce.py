"""The vendored Salesforce source (`saas/vendored/salesforce/`, `SRC-3` V-Salesforce.1):
its HTTP goes only through the `SaasSession` it is given (AD3, AD4; no
`simple_salesforce`, whose own HTTP would go round the link and address rules), and
no password sign-in is left in it (decision 4). No network: `FakeSalesforce` answers
the REST describe and query calls as Salesforce documents them, a page at a time with
`nextRecordsUrl`."""

from __future__ import annotations

import ast
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlparse

import pytest

from dispar_orchestrate import ssrf_guard
from dispar_orchestrate.saas.session import SaasSession
from dispar_orchestrate.saas.vendored.salesforce import settings as sf_settings
from dispar_orchestrate.saas.vendored.salesforce.helpers import records
from dispar_orchestrate.test_saas_vendored_zendesk import Reply, resolver

VENDORED = Path(records.__file__).parent.parent
ORIGIN = "https://acme.my.salesforce.com"
V = sf_settings.API_VERSION


def field(name: str, kind: str = "string", compound: str | None = None) -> dict[str, Any]:
    return {"name": name, "type": kind, "compoundFieldName": compound}


ACCOUNT_FIELDS = [
    field("Id", "id"), field("Name", "string", "Name"), field("IsDeleted", "boolean"),
    field("BillingStreet", "string", "BillingAddress"), field("BillingAddress", "address"),
    field("Body", "base64"), field("SystemModstamp", "datetime"), field("Industry__c", "string"),
]  # fmt: skip


def account(i: int, stamp: str = "2024-05-01T10:20:30.000+0000", **extra: Any) -> dict[str, Any]:
    return {"attributes": {"type": "Account", "url": f"/services/data/v{V}/sobjects/Account/{i}"}, "Id": f"001{i:03d}", "Name": f"a{i}",
            "IsDeleted": False, "BillingStreet": "1 Road", "SystemModstamp": stamp, "Industry__c": "x", **extra}  # fmt: skip


class FakeSalesforce:
    """`describe` and `query` of one sObject, `page_size` records a page."""

    def __init__(self, rows: list[dict[str, Any]], fields: list[dict[str, Any]] = ACCOUNT_FIELDS, page_size: int = 2) -> None:
        self.rows, self.fields, self.page_size = rows, fields, page_size
        self.log: list[str] = []
        self.queries: list[str] = []
        self.headers: list[dict] = []

    def get(self, url: str, *, headers: dict, params: dict | None, timeout: float) -> Reply:
        self.headers.append(headers)
        parsed = urlparse(url)
        self.log.append(parsed.path)
        if parsed.path.endswith("/describe"):
            return Reply(body={"name": parsed.path.split("/")[-2], "fields": self.fields})
        if parsed.path == f"/services/data/v{V}/query":
            self.queries.append((params or {})["q"])
            return self._page(0)
        if parsed.path == f"/services/data/v{V}/query/01gxx-batches":
            return self._page(int(parse_qs(parsed.query)["start"][0]))
        return Reply(404)

    def _page(self, start: int) -> Reply:
        page = self.rows[start : start + self.page_size]
        end = start + len(page)
        body: dict[str, Any] = {"totalSize": len(self.rows), "done": end >= len(self.rows), "records": page}
        if not body["done"]:
            body["nextRecordsUrl"] = f"/services/data/v{V}/query/01gxx-batches?start={end}"
        return Reply(body=body)


def session_over(fake: FakeSalesforce) -> SaasSession:
    return SaasSession(ORIGIN, lambda: {"Authorization": "Bearer t"}, http_get=fake.get, resolve_checked=resolver, sleep=lambda _s: None)


def test_no_vendored_salesforce_module_imports_an_http_library_or_a_salesforce_client() -> None:
    forbidden = {"requests", "urllib3", "httpx", "http", "urllib", "simple_salesforce"}
    for path in VENDORED.rglob("*.py"):
        for node in ast.walk(ast.parse(path.read_text())):
            names = [a.name for a in node.names] if isinstance(node, ast.Import) else [node.module or ""] if isinstance(node, ast.ImportFrom) else []
            for name in names:
                assert name.split(".")[0] not in forbidden, f"{path.name} imports {name}"
            if isinstance(node, ast.ImportFrom) and node.module == "dlt.sources.helpers":
                assert all(a.name not in ("requests", "client") for a in node.names), f"{path.name} imports dlt's requests"
            if isinstance(node, ast.ImportFrom) and node.module == "dlt.sources.helpers.requests":
                raise AssertionError(f"{path.name} imports dlt's requests")


def test_no_password_or_security_token_sign_in_is_left_in_the_vendored_source() -> None:
    """Decision 4: Salesforce retires the SOAP password + security-token sign-in."""
    text = "\n".join(p.read_text() for p in VENDORED.rglob("*.py")).lower()
    assert not (VENDORED / "helpers" / "client.py").exists()
    for word in ("security_token", "securitytokenauth", "organizationidauth", "consumerkeysecretauth", "privatekey", "user_name"):
        assert word not in text, word


def test_records_come_through_the_session_page_by_page_by_next_records_url() -> None:
    fake = FakeSalesforce([account(i) for i in range(1, 6)])
    got = list(records.get_records(session_over(fake), "Account"))
    assert [r["Id"] for r in got] == [f"001{i:03d}" for i in range(1, 6)]
    assert fake.log == [f"/services/data/v{V}/sobjects/Account/describe", f"/services/data/v{V}/query", *[f"/services/data/v{V}/query/01gxx-batches"] * 2]
    assert all(h == {"Authorization": "Bearer t"} for h in fake.headers)
    assert "attributes" not in got[0], "the record's attributes are not data"


def test_the_query_leaves_out_compound_and_file_body_fields_and_keeps_name() -> None:
    fake = FakeSalesforce([account(1)])
    list(records.get_records(session_over(fake), "Account"))
    query = fake.queries[0]
    assert query.startswith("SELECT Id, Name, IsDeleted, BillingStreet, SystemModstamp, Industry__c FROM Account")
    assert "BillingAddress" not in query.split("FROM")[0] and "Body" not in query.split("FROM")[0]


def test_dates_are_written_in_one_form_and_the_cursor_is_a_literal_salesforce_reads() -> None:
    fake = FakeSalesforce([account(1, "2024-05-01T10:20:30.000+0000")])
    got = list(records.get_records(session_over(fake), "Account", "2024-05-01T10:20:30.123456Z", "SystemModstamp"))
    assert got[0]["SystemModstamp"] == "2024-05-01T10:20:30.000000Z"
    assert "WHERE SystemModstamp > 2024-05-01T10:20:30.123Z ORDER BY SystemModstamp ASC" in fake.queries[0]


@pytest.mark.parametrize("bad", ["Account; DELETE", "Account Where", "../x", "a-b", ""])
def test_a_name_that_is_not_an_api_name_never_reaches_the_query(bad: str) -> None:
    fake = FakeSalesforce([])
    with pytest.raises(ValueError):
        list(records.get_records(session_over(fake), bad))
    assert fake.log == []
    with pytest.raises(ValueError):
        list(records.get_records(session_over(FakeSalesforce([account(1)])), "Account", "2024-01-01T00:00:00Z", "Id; DROP"))


def test_a_cursor_value_that_is_not_a_time_is_refused_not_spliced_in() -> None:
    fake = FakeSalesforce([account(1)])
    with pytest.raises(Exception):
        list(records.get_records(session_over(fake), "Account", "2024-01-01' OR Id != '", "SystemModstamp"))
    assert fake.queries == []


def test_an_object_with_too_many_fields_is_a_fixed_failure_not_an_address_too_long() -> None:
    many = [field("Id", "id"), *[field(f"Custom_Field_Number_{i:04d}__c") for i in range(800)]]
    fake = FakeSalesforce([], many)
    with pytest.raises(records.QueryTooLongError) as raised:
        list(records.get_records(session_over(fake), "Big__c"))
    assert "Custom_Field" not in str(raised.value) and fake.queries == []


def test_a_next_records_url_on_another_host_is_refused() -> None:
    fake = FakeSalesforce([account(i) for i in range(1, 4)])
    original = fake.get

    def to_another_host(url: str, **kw: Any) -> Reply:
        reply = original(url, **kw)
        if "nextRecordsUrl" in reply._body:
            reply._body["nextRecordsUrl"] = "https://evil.example.invalid/services/data/next"
        return reply

    fake.get = to_another_host  # type: ignore[method-assign]
    with pytest.raises(ssrf_guard.SsrfBlocked):
        list(records.get_records(session_over(fake), "Account"))
