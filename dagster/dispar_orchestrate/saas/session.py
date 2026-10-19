"""`SaasSession`: every HTTP request of a business-app read (`SRC-3` AD3,
decisions 8 and 13).

# The link rule (replaces rule Z5 for `saas` only)

`adapters/rest.py` never dials a URL taken from a response (its rule Z5). The
vendors of `SRC-3` page by returning the next URL (Salesforce, Graph,
Dataverse, ServiceNow, Zendesk, Confluence), so this session follows one, under
a stricter rule than "any":

1. A target is a path on the session's origin, or an absolute URL whose
   scheme, host AND port equal the origin's. Anything else raises
   `ssrf_guard.SsrfBlocked` (classified `address_refused`, not retried), whose
   message does not carry the link.
2. The one exception is Salesforce's `instance_url` (returned after sign-in):
   `adopt_origin` accepts an `https` host that ends in one of the suffixes the
   vendor module names (`.salesforce.com`, `.force.com`) and makes it the
   session's origin from then on.
3. EVERY host, the origin's and an adopted one, goes through
   `ssrf_guard.resolve_checked` before the request, and the request runs inside
   `ssrf_guard.pinned_resolution`, so it dials the address that was checked.
4. The authentication headers are asked for AFTER the address check, so a
   blocked address makes no credential leave this process.
5. No redirect is followed. A 3xx is a refusal (`SsrfBlocked`).

`rest.py` keeps rule Z5 unchanged; this module is not used by it.

# Rate limits (decision 13)

A 429 is waited out: the `Retry-After` header (whole seconds) when it is within
`max_retry_after`, else a doubling back-off from one second, for at most
`max_attempts` requests. A `Retry-After` beyond the bound, or the last attempt
still answered with 429, raises `SaasRateLimited`; the run fails and the data
read so far is never reported as complete. The wait is `time.sleep` by default
and injectable.

# What never leaves

401 raises `SaasSignInRefused`, 403 `SaasPermissionMissing`, 429 past the bound
`SaasRateLimited`; any other error status raises `requests.HTTPError` (its
status is what `ingest_errors` reads). None of the messages holds the vendor's
words, the URL or a header.
"""

from __future__ import annotations

import logging
import os
import time
from typing import Any, Callable
from urllib.parse import urlparse

import requests

from dispar_orchestrate import ssrf_guard

logger = logging.getLogger(__name__)

_REQUEST_TIMEOUT_SECONDS = 30
DEFAULT_MAX_ATTEMPTS = 4
DEFAULT_MAX_RETRY_AFTER_SECONDS = 60.0


class SaasSignInRefused(Exception):
    """The vendor refused the credential (HTTP 401). Not retried: the same
    answer would come back. The connector page offers Sign in again."""

    def __init__(self) -> None:
        super().__init__("the vendor refused the sign-in")


class SaasPermissionMissing(Exception):
    """The vendor accepted the credential but the account may not read what was
    asked (HTTP 403). Not retried."""

    def __init__(self) -> None:
        super().__init__("the vendor account lacks a permission the connector needs")


class SaasRateLimited(Exception):
    """The vendor limited the rate and still did after the bounded waits
    (decision 13). Retried by the run's own retry policy as `source_busy`."""

    def __init__(self) -> None:
        super().__init__("the vendor limited the rate of requests")


def _allow_http() -> bool:
    """The gate-only switch (`CONNECTOR_OAUTH_ALLOW_HTTP=true`, default off),
    the same one `adapters/rest.py` and the API read: the acceptance gate's own
    mock vendor speaks `http`. A real installation never sets it."""
    return os.environ.get("CONNECTOR_OAUTH_ALLOW_HTTP", "").strip() == "true"


def _origin_of(url: str) -> tuple[str, str, int]:
    """`(scheme, host, port)` of `url`, lower-cased, with the scheme's default
    port filled in.

    # Errors

    `ssrf_guard.SsrfBlocked` for a URL with no host, with userinfo, or with a
    scheme other than `https` (`http` only under the gate-only switch).
    """
    parsed = urlparse(url)
    scheme = parsed.scheme.lower()
    if scheme != "https" and not (scheme == "http" and _allow_http()):
        raise ssrf_guard.SsrfBlocked("a business-app request must use https")
    if not parsed.hostname or parsed.username is not None or parsed.password is not None:
        raise ssrf_guard.SsrfBlocked("a business-app address must be a plain host")
    try:
        port = parsed.port
    except ValueError:
        raise ssrf_guard.SsrfBlocked("a business-app address has an invalid port") from None
    return scheme, parsed.hostname.lower(), port or (443 if scheme == "https" else 80)


def _real_get(url: str, *, headers: dict, params: dict | None, timeout: float) -> Any:
    return requests.get(url, headers=headers, params=params, timeout=timeout, allow_redirects=False)


class SaasSession:
    """One vendor's HTTP reads under the link and rate-limit rules above.

    `auth_headers` returns the headers of the next request (an `Authorization`
    for a token the caller refreshes itself); it is called once per request,
    after the address was checked.
    """

    def __init__(
        self,
        origin_url: str,
        auth_headers: Callable[[], dict],
        *,
        adopt_suffixes: tuple[str, ...] = (),
        http_get: Callable[..., Any] | None = None,
        resolve_checked: Callable[..., ssrf_guard.ResolvedAddress] = ssrf_guard.resolve_checked,
        sleep: Callable[[float], None] = time.sleep,
        max_attempts: int = DEFAULT_MAX_ATTEMPTS,
        max_retry_after: float = DEFAULT_MAX_RETRY_AFTER_SECONDS,
    ) -> None:
        self._origin = _origin_of(origin_url)
        self._auth_headers = auth_headers
        self._adopt_suffixes = adopt_suffixes
        self._http_get = http_get or _real_get
        self._resolve_checked = resolve_checked
        self._sleep = sleep
        self._max_attempts = max(1, max_attempts)
        self._max_retry_after = max_retry_after

    @property
    def origin(self) -> str:
        scheme, host, port = self._origin
        default = 443 if scheme == "https" else 80
        return f"{scheme}://{host}" if port == default else f"{scheme}://{host}:{port}"

    def adopt_origin(self, url: str) -> None:
        """Make the host a vendor returned after sign-in the session's origin
        (Salesforce `instance_url`), when it is an `https` host ending in one
        of the suffixes the vendor module named.

        # Errors

        `ssrf_guard.SsrfBlocked` for any other host.
        """
        scheme, host, port = _origin_of(url)
        if scheme != "https" or port != 443 or not any(host.endswith(s) and host != s.lstrip(".") for s in self._adopt_suffixes):
            raise ssrf_guard.SsrfBlocked("the vendor returned a host this connector does not accept")
        self._origin = (scheme, host, port)

    def _target(self, target: str) -> str:
        if target.startswith("//"):
            raise ssrf_guard.SsrfBlocked("a link in a vendor answer pointed to another host; not followed")
        if target.startswith("/"):
            return self.origin + target
        if _origin_of(target) != self._origin:
            raise ssrf_guard.SsrfBlocked("a link in a vendor answer pointed to another host; not followed")
        return target

    def _wait_for(self, response: Any, attempt: int) -> float:
        raw = getattr(response, "headers", {}).get("Retry-After")
        try:
            seconds = float(raw) if raw is not None else float(2**attempt)
        except (TypeError, ValueError):
            seconds = float(2**attempt)
        if seconds < 0 or seconds > self._max_retry_after:
            raise SaasRateLimited()
        return seconds

    def get_json(self, target: str, params: dict | None = None) -> Any:
        """GET `target` (a path on the origin, or a same-origin link from an
        earlier answer) and return its JSON.

        # Errors

        `ssrf_guard.SsrfBlocked` (another host, a blocked address, a redirect),
        `SaasSignInRefused`, `SaasPermissionMissing`, `SaasRateLimited`,
        `requests.HTTPError` for another error status, and whatever `requests`
        raises for a connection or timeout failure.
        """
        url = self._target(target)
        _scheme, host, port = _origin_of(url)
        for attempt in range(self._max_attempts):
            resolved = self._resolve_checked(host, port)
            headers = self._auth_headers()
            with ssrf_guard.pinned_resolution(host, resolved):
                response = self._http_get(url, headers=headers, params=params, timeout=_REQUEST_TIMEOUT_SECONDS)
            status = response.status_code
            if 300 <= status < 400:
                raise ssrf_guard.SsrfBlocked("the vendor answered with a redirect; not followed")
            if status == 401:
                raise SaasSignInRefused()
            if status == 403:
                raise SaasPermissionMissing()
            if status == 429:
                if attempt + 1 >= self._max_attempts:
                    raise SaasRateLimited()
                wait = self._wait_for(response, attempt)
                logger.info("a vendor limited the rate; waiting %.0f s (attempt %d of %d)", wait, attempt + 1, self._max_attempts)
                self._sleep(wait)
                continue
            if status >= 400:
                raise requests.HTTPError(f"HTTP {status}", response=response)
            return response.json()
        raise SaasRateLimited()  # unreachable: the last attempt raises above
