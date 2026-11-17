"""Tests for the OAuth half of dispar_orchestrate/adapters/rest.py -- SRC-12
part C (decisions 6 to 8 and 12): the refresh-token grant, one new access token
on a 401, the expiry rule for both OAuth kinds, the report of a rotated refresh
token, and the fixed "sign in again" failure. No network: `requests.post` and
`http_get` are monkeypatched or injected in every test.
"""

from __future__ import annotations

import logging

import pytest
import requests

from dispar_orchestrate import ingest_errors, ingest_factory, schema_observer
from dispar_orchestrate.adapters import rest
from dispar_orchestrate.adapters.rest import AuthSession, OAuthSignInRefused, build_source
from dispar_orchestrate.secret_resolver import SecretRefRejected
from dispar_orchestrate.ssrf_guard import ResolvedAddress, SsrfBlocked

TOKEN_URL = "https://login.example.com/token"
PROVIDER_WORDS = "PROVIDER-WORDS-expired-for-someone@example.com"

CODE_AUTH = {
    "type": "oauth2_authorization_code",
    "clientId": "app-1",
    "authorizeUrl": "https://login.example.com/authorize",
    "tokenUrl": TOKEN_URL,
}
CODE_SECRETS = {"clientSecret": "client-secret-1", "refreshToken": "refresh-1"}
CC_AUTH = {"type": "oauth2_client_credentials", "tokenUrl": TOKEN_URL}
CC_SECRETS = {"clientId": "id-1", "clientSecret": "secret-1"}


def _public(host: str, port: int) -> ResolvedAddress:
    return ResolvedAddress(ip="203.0.113.10", port=port, family=2)


class _Resp:
    def __init__(self, status: int = 200, body: dict | None = None, headers: dict | None = None):
        self.status_code = status
        self._body = body if body is not None else {}
        self.headers = headers or {}

    def raise_for_status(self) -> None:
        if self.status_code >= 400:
            raise requests.HTTPError(f"HTTP {self.status_code}", response=self)

    def json(self) -> dict:
        return self._body


class _Provider:
    """A token endpoint: answers from a list and records every request."""

    def __init__(self, *answers: _Resp) -> None:
        self.answers = list(answers)
        self.calls: list[dict] = []

    def __call__(self, url, data, timeout, allow_redirects):
        self.calls.append({"url": url, "data": dict(data), "timeout": timeout, "allow_redirects": allow_redirects})
        return self.answers.pop(0)


def _token(access: str, **more) -> _Resp:
    return _Resp(200, {"access_token": access, "token_type": "bearer", **more})


@pytest.fixture
def provider(monkeypatch):
    def install(*answers: _Resp) -> _Provider:
        fake = _Provider(*answers)
        monkeypatch.setattr("dispar_orchestrate.adapters.rest.requests.post", fake)
        return fake

    monkeypatch.delenv("CONNECTOR_OAUTH_ALLOW_HTTP", raising=False)
    return install


def _session(auth=CODE_AUTH, secrets=CODE_SECRETS, **kwargs) -> AuthSession:
    return AuthSession(auth, dict(secrets), resolve_checked=_public, **kwargs)


class _Clock:
    def __init__(self) -> None:
        self.now = 1000.0

    def __call__(self) -> float:
        return self.now


# --- the refresh-token grant ---------------------------------------------------------


def test_the_authorization_code_kind_asks_for_a_token_with_the_refresh_token_grant(provider):
    fake = provider(_token("access-1"))
    headers = _session().headers()
    assert headers == {"Authorization": "Bearer access-1"}
    (call,) = fake.calls
    assert call["url"] == TOKEN_URL
    assert call["data"] == {
        "grant_type": "refresh_token",
        "refresh_token": "refresh-1",
        "client_id": "app-1",
        "client_secret": "client-secret-1",
    }
    assert call["allow_redirects"] is False


def test_one_token_is_reused_until_it_expires_or_the_source_refuses_it(provider):
    fake = provider(_token("access-1", expires_in=3600))
    session = _session()
    assert session.headers() == session.headers()
    assert len(fake.calls) == 1


def test_a_connector_nobody_signed_in_to_has_no_refresh_token_and_is_a_sign_in_failure(provider):
    fake = provider()
    with pytest.raises(OAuthSignInRefused) as raised:
        _session(secrets={"clientSecret": "s"}).headers()
    assert str(raised.value) == "sign in again"
    assert fake.calls == []


@pytest.mark.parametrize("status", [400, 401, 403])
def test_a_refused_refresh_is_sign_in_again_and_the_providers_words_are_not_kept(provider, status):
    provider(_Resp(status, {"error": "invalid_grant", "error_description": PROVIDER_WORDS}))
    with pytest.raises(OAuthSignInRefused) as raised:
        _session().headers()
    assert str(raised.value) == "sign in again"
    assert PROVIDER_WORDS not in repr(raised.value) and "invalid_grant" not in repr(raised.value)


def test_a_refused_sign_in_is_classified_and_not_retried():
    classified = ingest_errors.classify(OAuthSignInRefused())
    assert classified.error_class == ingest_errors.SIGN_IN_REFUSED
    assert classified.retried is False
    assert ingest_errors.CLASS_PHRASES[classified.error_class]
    # Wrapped by a pipeline step, as dlt does.
    try:
        try:
            raise OAuthSignInRefused()
        except OAuthSignInRefused as inner:
            raise RuntimeError("step failed") from inner
    except RuntimeError as outer:
        assert ingest_errors.classify(outer).error_class == ingest_errors.SIGN_IN_REFUSED


def test_a_provider_that_is_down_is_a_transient_failure_not_a_sign_in_failure(provider):
    provider(_Resp(503, {"error": PROVIDER_WORDS}))
    with pytest.raises(requests.HTTPError) as raised:
        _session().headers()
    classified = ingest_errors.classify(raised.value)
    assert classified.error_class == ingest_errors.SOURCE_BUSY and classified.retried is True


def test_an_answer_without_an_access_token_is_rejected(provider):
    provider(_Resp(200, {"token_type": "bearer"}))
    with pytest.raises(ValueError):
        _session().headers()


# --- the token address ------------------------------------------------------------------


@pytest.mark.parametrize("auth,secrets", [(CODE_AUTH, CODE_SECRETS), (CC_AUTH, CC_SECRETS)])
def test_the_token_address_must_be_https(provider, auth, secrets):
    fake = provider()
    with pytest.raises(SsrfBlocked):
        _session({**auth, "tokenUrl": "http://login.example.com/token"}, secrets).headers()
    assert fake.calls == []


def test_the_gate_only_switch_admits_an_http_token_address_and_nothing_else_changes(provider, monkeypatch):
    fake = provider(_token("access-1"))
    monkeypatch.setenv("CONNECTOR_OAUTH_ALLOW_HTTP", "true")
    _session({**CODE_AUTH, "tokenUrl": "http://login.example.com/token"}).headers()
    assert fake.calls[0]["url"].startswith("http://")
    # Only the exact word turns it on.
    monkeypatch.setenv("CONNECTOR_OAUTH_ALLOW_HTTP", "1")
    with pytest.raises(SsrfBlocked):
        _session({**CODE_AUTH, "tokenUrl": "http://login.example.com/token"}).headers()


def test_a_token_address_that_resolves_internal_is_refused_before_any_request(provider):
    fake = provider()

    def blocked(host, port):
        raise SsrfBlocked("refused")

    with pytest.raises(SsrfBlocked):
        AuthSession(CODE_AUTH, dict(CODE_SECRETS), resolve_checked=blocked).headers()
    assert fake.calls == []


def test_a_redirect_from_the_token_address_is_refused_without_its_location(provider):
    provider(_Resp(302, headers={"Location": "http://169.254.169.254/latest/meta-data/"}))
    with pytest.raises(SsrfBlocked) as raised:
        _session().headers()
    assert "169.254" not in str(raised.value)


# --- once more on a 401 ------------------------------------------------------------------


def _http_error(status: int) -> requests.HTTPError:
    return requests.HTTPError(f"HTTP {status}", response=_Resp(status))


def _spec(auth=CODE_AUTH) -> dict:
    return {
        "baseUrl": "https://api.example.com",
        "auth": auth,
        "pagination": {"type": "none"},
        "endpoints": [{"path": "/items"}],
    }


def test_a_401_gets_one_new_access_token_and_the_request_is_sent_again(provider):
    fake = provider(_token("access-1"), _token("access-2"))
    seen: list[str] = []

    def http_get(url, *, headers, params):
        seen.append(headers["Authorization"])
        if len(seen) == 1:
            raise _http_error(401)
        return [{"id": 1}]

    result = build_source(_spec(), dict(CODE_SECRETS), http_get=http_get, resolve_checked=_public)
    assert list(result.source) == [{"id": 1}]
    assert seen == ["Bearer access-1", "Bearer access-2"]
    assert len(fake.calls) == 2


def test_a_second_401_is_raised_and_not_retried_again(provider):
    fake = provider(_token("access-1"), _token("access-2"))
    attempts: list[int] = []

    def http_get(url, *, headers, params):
        attempts.append(1)
        raise _http_error(401)

    result = build_source(_spec(), dict(CODE_SECRETS), http_get=http_get, resolve_checked=_public)
    with pytest.raises(requests.HTTPError) as raised:
        list(result.source)
    assert len(attempts) == 2 and len(fake.calls) == 2
    assert ingest_errors.classify(raised.value).error_class == ingest_errors.SIGN_IN_REFUSED


def test_a_401_with_a_static_credential_is_raised_at_once(provider):
    provider()
    attempts: list[int] = []

    def http_get(url, *, headers, params):
        attempts.append(1)
        raise _http_error(401)

    result = build_source(_spec({"type": "bearer"}), {"token": "t"}, http_get=http_get, resolve_checked=_public)
    with pytest.raises(requests.HTTPError):
        list(result.source)
    assert len(attempts) == 1


def test_another_error_status_is_not_a_reason_to_ask_for_a_new_token(provider):
    fake = provider(_token("access-1"))

    def http_get(url, *, headers, params):
        raise _http_error(500)

    result = build_source(_spec(), dict(CODE_SECRETS), http_get=http_get, resolve_checked=_public)
    with pytest.raises(requests.HTTPError):
        list(result.source)
    assert len(fake.calls) == 1


# --- expiry, for both kinds ----------------------------------------------------------------


@pytest.mark.parametrize("auth,secrets", [(CODE_AUTH, CODE_SECRETS), (CC_AUTH, CC_SECRETS)])
def test_an_expired_access_token_is_replaced_before_the_next_request(provider, auth, secrets):
    # 100 seconds, less a margin of 30 or a fifth of it (20): replaced after 80.
    fake = provider(_token("access-1", expires_in=100), _token("access-2", expires_in=3600))
    clock = _Clock()
    session = _session(auth, secrets, clock=clock)
    assert session.headers()["Authorization"] == "Bearer access-1"
    clock.now += 79
    assert session.headers()["Authorization"] == "Bearer access-1"
    assert len(fake.calls) == 1
    clock.now += 2
    assert session.headers()["Authorization"] == "Bearer access-2"
    assert len(fake.calls) == 2


def test_a_short_lived_token_is_replaced_after_four_fifths_of_its_life_not_at_once(provider):
    fake = provider(_token("access-1", expires_in=5), _token("access-2", expires_in=5))
    clock = _Clock()
    session = _session(clock=clock)
    session.headers()
    clock.now += 3.9
    session.headers()
    assert len(fake.calls) == 1
    clock.now += 0.2
    assert session.headers()["Authorization"] == "Bearer access-2"


def test_client_credentials_ask_for_a_token_with_the_client_credentials_grant_and_get_the_401_rule(provider):
    fake = provider(_token("access-1"), _token("access-2"))
    seen: list[str] = []

    def http_get(url, *, headers, params):
        seen.append(headers["Authorization"])
        if len(seen) == 1:
            raise _http_error(401)
        return []

    result = build_source(_spec(CC_AUTH), dict(CC_SECRETS), http_get=http_get, resolve_checked=_public)
    assert list(result.source) == []
    assert seen == ["Bearer access-1", "Bearer access-2"]
    assert fake.calls[0]["data"] == {
        "grant_type": "client_credentials",
        "client_id": "id-1",
        "client_secret": "secret-1",
    }


# --- a rotated refresh token ----------------------------------------------------------------


def test_a_rotated_refresh_token_is_reported_and_used_for_the_next_refresh(provider):
    fake = provider(
        _token("access-1", refresh_token="refresh-2", expires_in=60),
        _token("access-2", refresh_token="refresh-3"),
    )
    reported: list[str] = []
    clock = _Clock()
    session = _session(clock=clock, report_refresh_token=reported.append)
    session.headers()
    assert reported == ["refresh-2"]
    clock.now += 100
    session.headers()
    assert fake.calls[1]["data"]["refresh_token"] == "refresh-2", "the retired token is not sent again"
    assert reported == ["refresh-2", "refresh-3"]


def test_the_same_refresh_token_back_is_not_reported(provider):
    provider(_token("access-1", refresh_token="refresh-1"), _token("access-2"))
    reported: list[str] = []
    session = _session(report_refresh_token=reported.append)
    session.headers()
    session.refresh()
    session.headers()
    assert reported == []


def test_a_report_that_fails_is_logged_without_the_token_and_the_run_goes_on(provider, caplog):
    provider(_token("access-1", refresh_token="refresh-SECRET-2"))

    def failing(token: str) -> None:
        raise schema_observer.ObservationUnreachable(f"the API could not be reached with {token}")

    with caplog.at_level(logging.WARNING):
        headers = _session(report_refresh_token=failing).headers()
    assert headers["Authorization"] == "Bearer access-1"
    text = " ".join(r.getMessage() for r in caplog.records)
    assert "ObservationUnreachable" in text and "refresh-SECRET-2" not in text


def test_no_token_reaches_a_log_line(provider, caplog):
    provider(_token("access-SECRET-1", refresh_token="refresh-SECRET-2"))
    with caplog.at_level(logging.DEBUG):
        _session(report_refresh_token=lambda token: None).headers()
    text = " ".join(r.getMessage() for r in caplog.records)
    for secret in ("access-SECRET-1", "refresh-SECRET-2", "refresh-1", "client-secret-1"):
        assert secret not in text


def test_a_load_reports_to_the_api_by_default_when_it_knows_the_connector(provider, monkeypatch):
    provider(_token("access-1", refresh_token="refresh-2"))
    posted: list[tuple] = []
    monkeypatch.setattr(
        schema_observer, "_post_json", lambda cfg, path, payload, what: posted.append((path, payload, what))
    )
    monkeypatch.setattr(schema_observer.ObserverConfig, "from_env", classmethod(lambda cls: cls("http://api", "token")))
    build_source(
        _spec(), dict(CODE_SECRETS), connector_id="conn-9", http_get=lambda *a, **k: [], resolve_checked=_public
    )
    assert posted == [
        ("/api/connectors/conn-9/oauth/refresh-token", {"refreshToken": "refresh-2"}, "rotated refresh token")
    ]


def test_a_static_credential_never_reports_anything(provider, monkeypatch):
    provider()
    monkeypatch.setattr(
        schema_observer, "_post_json", lambda *a, **k: pytest.fail("a bearer token has nothing to report")
    )
    build_source(
        _spec({"type": "bearer"}), {"token": "t"}, connector_id="conn-9", http_get=lambda *a, **k: [], resolve_checked=_public
    )


# --- the connector's own refresh token is a secret slot -----------------------------------------


def test_a_missing_refresh_token_file_is_sign_in_again_not_a_rejected_setting(monkeypatch):
    def resolve(ref):
        if ref == "file:/run/secrets/client":
            return "client-secret-1"
        raise SecretRefRejected(f"secretRef {ref!r} does not resolve to a real file")

    monkeypatch.setattr(ingest_factory.secret_resolver, "resolve_secret_ref", resolve)
    connector = {"secretRef": "file:/run/secrets/client", "secretRefSecondary": "file:/run/secrets/token"}
    with pytest.raises(OAuthSignInRefused):
        ingest_factory._resolve_object_secrets(connector, "rest", {"auth": CODE_AUTH})
    # The same failure for another auth kind stays a rejected setting.
    with pytest.raises(SecretRefRejected):
        ingest_factory._resolve_object_secrets(connector, "rest", {"auth": {"type": "basic"}})


def test_both_secrets_of_an_authorization_code_connector_are_resolved(monkeypatch):
    monkeypatch.setattr(ingest_factory.secret_resolver, "resolve_secret_ref", lambda ref: f"value-of-{ref}")
    connector = {"secretRef": "a", "secretRefSecondary": "b"}
    assert ingest_factory._resolve_object_secrets(connector, "rest", {"auth": CODE_AUTH}) == {
        "clientSecret": "value-of-a",
        "refreshToken": "value-of-b",
    }
