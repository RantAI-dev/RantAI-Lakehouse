"""`saas.session.SaasSession` and `saas.objects` (`SRC-3` T1.8): the link rule,
the address check before any credential, redirects, and rate limits. No network:
the HTTP call, the resolver and the sleep are injected."""

from __future__ import annotations

import pytest
import requests

from dispar_orchestrate import ssrf_guard
from dispar_orchestrate.saas import session as saas_session
from dispar_orchestrate.saas.objects import ObjectDecl
from dispar_orchestrate.saas.session import SaasPermissionMissing, SaasRateLimited, SaasSession, SaasSignInRefused

ORIGIN = "https://acme.example.invalid"


class Reply:
    def __init__(self, status: int = 200, body: object = None, headers: dict | None = None) -> None:
        self.status_code = status
        self._body = body if body is not None else {}
        self.headers = headers or {}

    def json(self) -> object:
        return self._body


class Recorder:
    """An injected `http_get` that answers from a script and keeps what it saw."""

    def __init__(self, *replies: Reply) -> None:
        self.replies = list(replies)
        self.urls: list[str] = []
        self.headers: list[dict] = []

    def __call__(self, url: str, *, headers: dict, params: dict | None, timeout: float) -> Reply:
        self.urls.append(url)
        self.headers.append(headers)
        return self.replies.pop(0) if self.replies else Reply()


def resolver(host: str, port: int, **_kw: object) -> ssrf_guard.ResolvedAddress:
    return ssrf_guard.ResolvedAddress(ip="93.184.216.34", port=port, family=2)


def make(http: Recorder, **kw: object) -> tuple[SaasSession, list[str]]:
    asked: list[str] = []

    def headers() -> dict:
        asked.append("asked")
        return {"Authorization": "Bearer t"}

    kw.setdefault("resolve_checked", resolver)
    kw.setdefault("sleep", lambda _s: None)
    return SaasSession(ORIGIN, headers, http_get=http, **kw), asked  # type: ignore[arg-type]


def test_a_path_and_a_same_origin_link_are_followed() -> None:
    http = Recorder(Reply(body={"next": f"{ORIGIN}/page2"}), Reply(body={"ok": True}))
    session, _ = make(http)
    assert session.get_json("/api/items") == {"next": f"{ORIGIN}/page2"}
    assert session.get_json(f"{ORIGIN}/page2") == {"ok": True}
    assert http.urls == [f"{ORIGIN}/api/items", f"{ORIGIN}/page2"]


@pytest.mark.parametrize(
    ("header", "expected"),
    [
        (f'<{ORIGIN}/p2>;rel="next",<{ORIGIN}/p9>;rel="last"', f"{ORIGIN}/p2"),
        (f'<{ORIGIN}/p1>; rel="prev", <{ORIGIN}/p3>; rel=next', f"{ORIGIN}/p3"),
        (f'<{ORIGIN}/p9>;rel="last"', None),
        ("garbage", None),
    ],
)
def test_the_next_target_of_a_link_header_is_handed_back_and_not_followed(header: str, expected: str | None) -> None:
    """ServiceNow pages by `Link: <...>;rel="next"`. The session only reports the target; following it
    goes through `get_json`, whose same-origin rule is tested below."""
    http = Recorder(Reply(body={"result": []}, headers={"Link": header}))
    session, _ = make(http)
    assert session.get_json_with_next("/api/x") == ({"result": []}, expected)
    assert http.urls == [f"{ORIGIN}/api/x"]


@pytest.mark.parametrize(
    "link",
    [
        "https://evil.example.invalid/steal",
        "https://acme.example.invalid.evil.example.invalid/x",
        "https://acme.example.invalid:8443/x",
        "http://acme.example.invalid/x",
        "https://user@acme.example.invalid/x",
        "//evil.example.invalid/x",
        "ftp://acme.example.invalid/x",
    ],
)
def test_a_link_to_another_host_stops_the_run_and_nothing_is_sent(link: str) -> None:
    http = Recorder()
    session, asked = make(http)
    with pytest.raises(ssrf_guard.SsrfBlocked) as raised:
        session.get_json(link)
    assert http.urls == [] and asked == [], "no request, no credential"
    assert "evil" not in str(raised.value), "the link is the vendor's text and is not echoed"


def test_a_blocked_address_stops_it_before_any_credential_is_sent() -> None:
    def blocked(host: str, port: int, **_kw: object) -> ssrf_guard.ResolvedAddress:
        raise ssrf_guard.SsrfBlocked("blocked")

    http = Recorder()
    session, asked = make(http, resolve_checked=blocked)
    with pytest.raises(ssrf_guard.SsrfBlocked):
        session.get_json("/api")
    assert asked == [] and http.urls == []


def test_every_host_goes_through_the_address_check_including_an_adopted_one() -> None:
    checked: list[str] = []

    def spy(host: str, port: int, **_kw: object) -> ssrf_guard.ResolvedAddress:
        checked.append(host)
        return resolver(host, port)

    http = Recorder()
    session, _ = make(http, resolve_checked=spy, adopt_suffixes=(".salesforce.com", ".force.com"))
    session.get_json("/a")
    session.adopt_origin("https://acme.my.salesforce.com")
    session.get_json("/b")
    assert checked == ["acme.example.invalid", "acme.my.salesforce.com"]
    assert http.urls[1] == "https://acme.my.salesforce.com/b"


@pytest.mark.parametrize(
    "returned",
    [
        "https://evil.example.invalid",
        "https://salesforce.com.evil.example.invalid",
        "https://salesforce.com",
        "https://acme.my.salesforce.com:8443",
        "http://acme.my.salesforce.com",
        "https://u@acme.my.salesforce.com",
    ],
)
def test_a_salesforce_host_outside_the_suffixes_stops_the_run(returned: str) -> None:
    session, _ = make(Recorder(), adopt_suffixes=(".salesforce.com", ".force.com"))
    with pytest.raises(ssrf_guard.SsrfBlocked):
        session.adopt_origin(returned)
    assert session.origin == ORIGIN, "the origin did not move"


def test_the_gate_origin_replaces_an_adopted_host_only_after_the_host_passed_the_rule(monkeypatch: pytest.MonkeyPatch) -> None:
    """SRC-3 V-Salesforce.3: under the gate the session dials the mock, but the host the
    vendor returned must still pass the suffix rule first."""
    monkeypatch.setenv("CONNECTOR_OAUTH_ALLOW_HTTP", "true")
    session, _ = make(Recorder(), adopt_suffixes=(".salesforce.com", ".force.com"))
    session.adopt_origin("https://acme.my.salesforce.com", as_origin="http://mock.invalid:8103")
    assert session.origin == "http://mock.invalid:8103"
    session, _ = make(Recorder(), adopt_suffixes=(".salesforce.com", ".force.com"))
    with pytest.raises(ssrf_guard.SsrfBlocked):
        session.adopt_origin("https://evil.example.invalid", as_origin="http://mock.invalid:8103")
    assert session.origin == ORIGIN


def test_a_session_with_no_adopt_suffixes_adopts_nothing() -> None:
    session, _ = make(Recorder())
    with pytest.raises(ssrf_guard.SsrfBlocked):
        session.adopt_origin("https://acme.my.salesforce.com")


def test_a_redirect_is_a_refusal_not_a_thing_to_follow() -> None:
    http = Recorder(Reply(302, headers={"Location": "http://169.254.169.254/"}))
    session, _ = make(http)
    with pytest.raises(ssrf_guard.SsrfBlocked) as raised:
        session.get_json("/api")
    assert "169.254" not in str(raised.value)
    assert len(http.urls) == 1


def test_http_is_refused_unless_the_gate_only_switch_is_on(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("CONNECTOR_OAUTH_ALLOW_HTTP", raising=False)
    with pytest.raises(ssrf_guard.SsrfBlocked):
        SaasSession("http://acme.example.invalid", dict)
    monkeypatch.setenv("CONNECTOR_OAUTH_ALLOW_HTTP", "true")
    assert SaasSession("http://acme.example.invalid", dict).origin == "http://acme.example.invalid"


def test_a_429_then_a_200_succeeds_after_waiting_the_retry_after() -> None:
    http = Recorder(Reply(429, headers={"Retry-After": "7"}), Reply(body={"ok": 1}))
    slept: list[float] = []
    session, _ = make(http, sleep=slept.append)
    assert session.get_json("/api") == {"ok": 1}
    assert slept == [7.0]


def test_a_429_without_retry_after_backs_off_by_doubling() -> None:
    http = Recorder(Reply(429), Reply(429), Reply(body={}))
    slept: list[float] = []
    session, _ = make(http, sleep=slept.append)
    session.get_json("/api")
    assert slept == [1.0, 2.0]


def test_a_retry_after_past_the_bound_fails_as_rate_limited_without_waiting() -> None:
    http = Recorder(Reply(429, headers={"Retry-After": "3600"}))
    slept: list[float] = []
    session, _ = make(http, sleep=slept.append, max_retry_after=60)
    with pytest.raises(SaasRateLimited):
        session.get_json("/api")
    assert slept == [] and len(http.urls) == 1


def test_a_429_that_never_stops_fails_as_rate_limited_after_the_attempts() -> None:
    http = Recorder(*[Reply(429, headers={"Retry-After": "1"}) for _ in range(10)])
    session, _ = make(http, max_attempts=3)
    with pytest.raises(SaasRateLimited) as raised:
        session.get_json("/api")
    assert len(http.urls) == 3
    assert str(raised.value) == "the vendor limited the rate of requests"


@pytest.mark.parametrize(
    ("status", "error"),
    [(401, SaasSignInRefused), (403, SaasPermissionMissing)],
)
def test_a_401_and_a_403_are_their_own_failures_with_fixed_text(status: int, error: type[Exception]) -> None:
    session, _ = make(Recorder(Reply(status, body={"message": "VENDOR-WORDS"})))
    with pytest.raises(error) as raised:
        session.get_json("/api")
    assert "VENDOR-WORDS" not in str(raised.value)


def test_another_error_status_is_an_http_error_that_carries_its_status() -> None:
    session, _ = make(Recorder(Reply(503)))
    with pytest.raises(requests.HTTPError) as raised:
        session.get_json("/api")
    assert raised.value.response.status_code == 503


def test_the_module_exports_the_three_failures() -> None:
    assert {"SaasSignInRefused", "SaasPermissionMissing", "SaasRateLimited"} <= set(dir(saas_session))


def test_an_object_without_a_cursor_must_say_why_it_is_re_read_in_full() -> None:
    with pytest.raises(ValueError, match="no reason"):
        ObjectDecl(name="owners", primary_key=("id",))
    full = ObjectDecl(name="owners", primary_key=("id",), full_reason="the API has no last-modified filter")
    assert full.load_description() == "re-read in full: the API has no last-modified filter"
    incremental = ObjectDecl(name="tickets", primary_key=("id",), cursor="updated_at")
    assert incremental.incremental and incremental.load_description() == "incremental"
    with pytest.raises(ValueError, match="also a reason"):
        ObjectDecl(name="x", primary_key=("id",), cursor="u", full_reason="r")
    with pytest.raises(ValueError, match="primary key"):
        ObjectDecl(name="x", primary_key=(), cursor="u")


def test_a_401_asks_for_a_new_token_once_and_a_second_401_is_the_credential() -> None:
    refreshed: list[str] = []
    http = Recorder(Reply(401), Reply(body={"ok": 1}))
    session, _ = make(http, refresh_auth=lambda: refreshed.append("new token"))
    assert session.get_json("/api") == {"ok": 1}
    assert refreshed == ["new token"]
    twice = Recorder(Reply(401), Reply(401))
    session, _ = make(twice, refresh_auth=lambda: None)
    with pytest.raises(SaasSignInRefused):
        session.get_json("/api")
    assert len(twice.urls) == 2


def test_a_token_request_for_a_basic_client_carries_the_client_in_the_header_only(monkeypatch: pytest.MonkeyPatch) -> None:
    import base64

    from dispar_orchestrate.adapters import rest

    seen: dict = {}

    class Answer:
        status_code = 200

        def raise_for_status(self) -> None:
            return None

        def json(self) -> dict:
            return {"access_token": "a"}

    def post(url: str, **kw: object) -> Answer:
        seen.update(kw)
        return Answer()

    monkeypatch.setattr(rest.requests, "post", post)
    body = rest._token_request(
        "https://login.example.invalid/token",
        {"grant_type": "refresh_token", "refresh_token": "r"},
        resolve_checked=resolver,
        basic=("app id", "s3cret:+"),
    )
    assert body["access_token"] == "a"
    assert "client_id" not in seen["data"] and "client_secret" not in seen["data"]
    sent = seen["headers"]["Authorization"].removeprefix("Basic ")
    assert base64.b64decode(sent).decode() == "app%20id:s3cret%3A%2B"


class PostRecorder:
    """An injected `http_post`: keeps the url, the headers and the JSON body."""

    def __init__(self, *replies: Reply) -> None:
        self.replies = list(replies)
        self.calls: list[tuple[str, dict, object]] = []

    def __call__(self, url: str, *, headers: dict, json: object, timeout: float) -> Reply:
        self.calls.append((url, headers, json))
        return self.replies.pop(0) if self.replies else Reply()


def test_a_post_runs_under_the_same_rules_as_a_get() -> None:
    """SRC-3 V-HubSpot.1: the CRM search is a POST; it must not be a way round the link rule."""
    post = PostRecorder(Reply(429, headers={"Retry-After": "2"}), Reply(body={"results": []}))
    slept: list[float] = []
    session, asked = make(Recorder(), http_post=post, sleep=slept.append)
    assert session.post_json("/crm/v3/objects/deals/search", {"limit": 1}) == {"results": []}
    assert slept == [2.0] and [c[0] for c in post.calls] == [f"{ORIGIN}/crm/v3/objects/deals/search"] * 2
    assert post.calls[0][2] == {"limit": 1} and asked == ["asked", "asked"]
    with pytest.raises(ssrf_guard.SsrfBlocked):
        session.post_json("https://evil.example.invalid/search", {})
    assert len(post.calls) == 2, "another host: nothing was sent"
    redirect = PostRecorder(Reply(307, headers={"Location": "http://169.254.169.254/"}))
    session, _ = make(Recorder(), http_post=redirect)
    with pytest.raises(ssrf_guard.SsrfBlocked):
        session.post_json("/search", {})
