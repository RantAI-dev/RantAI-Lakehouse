"""Tests for dispar_orchestrate/adapters/rest.py -- WS3 item 22, WS3 plan
review Z5 (redirects refused, oauth2 tokenUrl SSRF-checked before any
request) and Z6 (build_source's arity matches ingest_factory.py's one
call shape). No network: `http_get`/`resolve_checked`/`requests.get`/
`requests.post` are injected or monkeypatched in every test.
"""

from __future__ import annotations

import pytest

from dispar_orchestrate.adapters.rest import _auth_headers_and_params, build_source
from dispar_orchestrate.ssrf_guard import ResolvedAddress, SsrfBlocked


def test_build_source_reads_the_bare_array_body_when_no_records_path_is_set():
    """`RestPagination::None` + no `recordsPath` -- the shape
    `ops/g6/rest_stub.py` actually exercises end-to-end, and the only
    combination valid on both the Rust contract and this adapter."""
    spec = {
        "baseUrl": "https://example.invalid", "auth": {"type": "bearer"},
        "pagination": {"type": "none"},
        "endpoints": [{"path": "/items"}],
    }
    result = build_source(
        spec, secrets={"token": "t"}, http_get=lambda *a, **k: [{"id": 1}, {"id": 2}],
        resolve_checked=lambda host, port: ResolvedAddress(ip="93.184.216.34", port=port, family=2),
    )
    assert list(result.source) == [{"id": 1}, {"id": 2}]


def test_build_source_paginates_by_page_number():
    spec = {
        "baseUrl": "https://example.invalid", "auth": {"type": "bearer"},
        "pagination": {"type": "page", "param": "page"},
        "endpoints": [{"path": "/items", "recordsPath": "items"}],
    }
    pages = iter([{"items": [{"id": 1}]}, {"items": []}])
    result = build_source(
        spec, secrets={"token": "t"}, http_get=lambda *a, **k: next(pages),
        resolve_checked=lambda host, port: ResolvedAddress(ip="93.184.216.34", port=port, family=2),
    )
    assert list(result.source) == [{"id": 1}]


def test_build_source_paginates_by_cursor_until_none_returned():
    """`RestPagination::Cursor` carries only `cursorField` -- no separate
    outgoing-parameter name -- so the adapter must re-send the cursor
    under that SAME key it read it from (asserted below via the second
    `http_get` call's params)."""
    spec = {
        "baseUrl": "https://example.invalid", "auth": {"type": "basic"},
        "pagination": {"type": "cursor", "cursorField": "nextCursor"},
        "endpoints": [{"path": "/items", "recordsPath": "items"}],
    }
    pages = iter([
        {"items": [{"id": 1}], "nextCursor": "c2"},
        {"items": [{"id": 2}], "nextCursor": None},
    ])
    calls = []

    def fake_http_get(*a, **k):
        calls.append(k.get("params"))
        return next(pages)

    result = build_source(
        spec, secrets={"username": "u", "password": "p"}, http_get=fake_http_get,
        resolve_checked=lambda host, port: ResolvedAddress(ip="93.184.216.34", port=port, family=2),
    )
    assert list(result.source) == [{"id": 1}, {"id": 2}]
    assert calls == [{}, {"nextCursor": "c2"}]


def test_build_source_refuses_an_offset_pagination_type_the_contract_does_not_admit():
    """`RestPagination` has no `offset` variant -- `Dial::parse` rejects
    `pagination.type = "offset"` before an ingest-spec is ever stored, so
    this adapter must never implement a branch nothing on the contract
    side can select."""
    spec = {
        "baseUrl": "https://example.invalid", "auth": {"type": "bearer"},
        "pagination": {"type": "offset", "param": "offset"},
        "endpoints": [{"path": "/items"}],
    }
    result = build_source(
        spec, secrets={"token": "t"}, http_get=lambda *a, **k: pytest.fail("must not be called"),
        resolve_checked=lambda host, port: ResolvedAddress(ip="93.184.216.34", port=port, family=2),
    )
    with pytest.raises(ValueError, match="offset"):
        list(result.source)


def test_build_source_refuses_an_internal_base_url():
    with pytest.raises(SsrfBlocked):
        build_source(
            {"baseUrl": "http://169.254.169.254", "auth": {"type": "bearer"}, "pagination": {"type": "none"}, "endpoints": []},
            secrets={"token": "t"},
            resolve_checked=lambda host, port: (_ for _ in ()).throw(SsrfBlocked("refused")),
        )


def test_auth_headers_api_key():
    headers, _ = _auth_headers_and_params({"type": "api_key", "header": "X-Api-Key"}, {"apiKey": "k1"})
    assert headers == {"X-Api-Key": "k1"}


def test_auth_headers_bearer():
    headers, _ = _auth_headers_and_params({"type": "bearer"}, {"token": "t1"})
    assert headers == {"Authorization": "Bearer t1"}


def test_auth_headers_basic():
    headers, _ = _auth_headers_and_params({"type": "basic"}, {"username": "u", "password": "p"})
    assert headers["Authorization"].startswith("Basic ")


def test_auth_headers_oauth2_client_credentials_exchanges_a_token(monkeypatch):
    def fake_post(url, data, timeout, allow_redirects):
        assert url == "https://auth.example.invalid/token"
        assert data["grant_type"] == "client_credentials"
        assert allow_redirects is False

        class _Resp:
            status_code = 200

            def raise_for_status(self):
                return None

            def json(self):
                return {"access_token": "exchanged-token"}

        return _Resp()

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", fake_post)
    headers, _ = _auth_headers_and_params(
        {"type": "oauth2_client_credentials", "tokenUrl": "https://auth.example.invalid/token"},
        {"clientId": "id", "clientSecret": "secret"},
        resolve_checked=lambda host, port: ResolvedAddress(ip="93.184.216.34", port=port, family=2),
    )
    assert headers["Authorization"] == "Bearer exchanged-token"


# WS3 plan review Z5 -- the redirect regression. `_real_http_get` (the
# DEFAULT `http_get`, exercised here by NOT injecting one) must refuse a
# 3xx response with exactly one request made -- never follow `Location`
# to a second, unchecked host.
def test_build_source_refuses_a_redirect_response_with_exactly_one_request(monkeypatch):
    calls = []

    class _RedirectResp:
        status_code = 302
        headers = {"Location": "http://169.254.169.254/latest/meta-data/"}

        def raise_for_status(self):
            return None

        def json(self):
            return {}

    def fake_requests_get(url, *, headers, params, timeout, allow_redirects):
        calls.append(url)
        assert allow_redirects is False
        return _RedirectResp()

    monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.get", fake_requests_get)
    spec = {
        "baseUrl": "https://example.invalid", "auth": {"type": "bearer"},
        "pagination": {"type": "none"}, "endpoints": [{"path": "/items", "recordsPath": "items"}],
    }
    result = build_source(
        spec, secrets={"token": "t"},
        resolve_checked=lambda host, port: ResolvedAddress(ip="93.184.216.34", port=port, family=2),
    )
    with pytest.raises(SsrfBlocked):
        list(result.source)
    assert len(calls) == 1


# WS3 plan review Z5 -- the OAuth tokenUrl regression: refused BEFORE any
# request, using the injected `resolve_checked` the same way build_source's
# own baseUrl check is tested, and requests.post must never be reached.
def test_oauth2_token_url_is_refused_before_any_request_when_it_resolves_internal(monkeypatch):
    monkeypatch.setattr(
        "dispar_orchestrate.adapters.rest.requests.post",
        lambda *a, **k: pytest.fail("must not be called -- tokenUrl was internal"),
    )

    def blocking_resolve_checked(host, port):
        raise SsrfBlocked("refused")

    with pytest.raises(SsrfBlocked):
        _auth_headers_and_params(
            {"type": "oauth2_client_credentials", "tokenUrl": "https://internal.invalid/token"},
            {"clientId": "id", "clientSecret": "secret"},
            resolve_checked=blocking_resolve_checked,
        )


def test_oauth2_token_url_must_be_https():
    with pytest.raises(SsrfBlocked):
        _auth_headers_and_params(
            {"type": "oauth2_client_credentials", "tokenUrl": "http://auth.example.invalid/token"},
            {"clientId": "id", "clientSecret": "secret"},
        )


# WS3 plan review Z6: build_source's arity matches ingest_factory.py's
# ONE call shape used for every non-sheets adapter
# (`adapter.build_source(dial, secrets, [obj])`) -- a positional
# source_objects, accepted and ignored here (REST's "objects" are
# dial.endpoints, declared in the dial itself).
def test_build_source_accepts_a_positional_source_objects_argument():
    spec = {
        "baseUrl": "https://example.invalid", "auth": {"type": "bearer"},
        "pagination": {"type": "none"}, "endpoints": [{"path": "/items", "recordsPath": "items"}],
    }
    result = build_source(
        spec, {"token": "t"}, [{"name": "ignored"}],
        http_get=lambda *a, **k: {"items": [{"id": 1}]},
        resolve_checked=lambda host, port: ResolvedAddress(ip="93.184.216.34", port=port, family=2),
    )
    assert list(result.source) == [{"id": 1}]
