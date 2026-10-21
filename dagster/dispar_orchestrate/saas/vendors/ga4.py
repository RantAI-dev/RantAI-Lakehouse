"""Google Analytics 4 (`SRC-3` V-GA4.2; decisions 2, 5, 6, 7, 9). The vendor module of
the `saas` adapter for `vendor = "ga4"`; the code that talks to the GA4 Data API is the
vendored dlt verified source (`saas/vendored/google_analytics/`, AD4), which calls the
API's REST endpoints (`runReport`, `getMetadata`) through the `SaasSession` the adapter
hands in (plan section 8 item 1: the Google client is not used).

# Objects are reports the person defines (decisions 2 and 5)

GA4 has no list of "tables": a report is a choice of dimensions and metrics. The
connector's `objects` (the dial) and a source object's `name` hold one report each, in
this form, which is data this module reads and never an address:

    traffic_by_country = country, date | sessions, activeUsers

The text before `=` is a label (letters, digits, underscore), the names before `|` are
dimensions and after it metrics, written as the Data API names them (`country`,
`sessionDefaultChannelGroup`, `customEvent:plan`). Names are checked against the
property's own `getMetadata` before a load (`describe`, decision 9), so a typo or a
custom dimension the property does not have is a plain message, and the schema of the
report is known before a row is read. Raw events (the BigQuery export Databricks reads)
are not offered (decision 2).

# Incremental by the date dimension (decision 6)

The report's time dimension (`date`, or the first of `dateHour`, `yearMonth`, ... the
definition has; `date` is added when it has none) is the cursor and the dimensions
together are the merge key. A later run starts `LOOKBACK_DAYS` days before the last
value (vendored local change 2: Google may change a day for 24-48 hours; the object's
`lag_seconds`, which dlt's incremental subtracts from the last value and for which it
switches its boundary de-duplication off) and the merge replaces the rows read again. A report always has a cursor, so none is "re-read in full".

# Deletes (decision 7)

GA4 reports have no deleted records; no object says `deletes`.

# Sign-in (decision 3), checked against Google's documentation on 2026-10-21

* **Browser sign-in**: authorise at `https://accounts.google.com/o/oauth2/v2/auth`, token at
  `https://oauth2.googleapis.com/token` with the client in the form body, revoke by
  `POST https://oauth2.googleapis.com/revoke` with a `token`, scope
  `https://www.googleapis.com/auth/analytics.readonly`
  (https://developers.google.com/identity/protocols/oauth2/web-server and the
  `runReport` / `getMetadata` reference pages, "Authorization scopes"). Google returns a
  refresh token ONLY when the authorise request carries `access_type=offline` ("the
  refresh token is only returned if your application set the access_type parameter to
  offline"), and only the first time unless `prompt=consent` is sent: the table row's
  authorise address carries both (slice 3 found the row without them, which would have
  signed a person in and never produced a token to load with).
* **Service-account key** (`integration_auth`): the key file's `client_email` and
  `private_key` sign a JWT (RS256, claims `iss`, `scope`, `aud`, `iat`, `exp` at most one
  hour after `iat`) that is exchanged at the token address with
  `grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer` for a bearer token that lives
  an hour (https://developers.google.com/identity/protocols/oauth2/service-account).
  The `aud` and the address dialled are the vendor table's, never the key file's
  `token_uri` (a key file is user data; a host is never taken from it, AD2). The property
  must have the service account's email added as a viewer. The API's connection test
  cannot present this credential yet and says so (`supported: false`); a load reports
  whether it worked.
"""

from __future__ import annotations

import base64
import json
import re
import time
from collections.abc import Callable, Iterator
from types import SimpleNamespace
from typing import Any

from cryptography.exceptions import UnsupportedAlgorithm
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding, rsa

from dispar_orchestrate.adapters import rest as rest_adapter
from dispar_orchestrate.adapters.saas import Field, SaasSetupRefused
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.objects import ObjectDecl
from dispar_orchestrate.saas.vendored import google_analytics as ga_source
from dispar_orchestrate.saas.vendored.google_analytics import helpers as ga_helpers
from dispar_orchestrate.saas.vendored.google_analytics.helpers.data_processing import get_metadata
from dispar_orchestrate.saas.vendored.google_analytics.settings import LOOKBACK_DAYS, START_DATE

ROWS_PER_PAGE = 10_000
_LABEL = re.compile(r"^[A-Za-z][A-Za-z0-9_]{0,63}$")
# A Data API name: letters, digits, underscore, with the one `customEvent:name` form.
_API_NAME = re.compile(r"^[A-Za-z][A-Za-z0-9_]*(:[A-Za-z0-9_]+)?$")
_MAX_NAMES = 20


def _refuse() -> SaasSetupRefused:
    return SaasSetupRefused("a Google Analytics report is written as: label = dimension, dimension | metric, metric")


def parse_report(text: str) -> tuple[str, list[str], list[str]]:
    """`(label, dimensions, metrics)` of a report definition, the time dimension
    included (added as `date` when the definition has none).

    # Errors

    `SaasSetupRefused` for anything that is not the form above.
    """
    label, eq, rest = text.partition("=")
    dims_text, bar, metrics_text = rest.partition("|")
    if not eq or not bar or not _LABEL.match(label.strip()):
        raise _refuse()
    dims = [d.strip() for d in dims_text.split(",") if d.strip()]
    metrics = [m.strip() for m in metrics_text.split(",") if m.strip()]
    names = [*dims, *metrics]
    if not metrics or len(names) > _MAX_NAMES or len(set(dims)) != len(dims) or len(set(metrics)) != len(metrics):
        raise _refuse()
    if not all(_API_NAME.match(n) for n in names):
        raise _refuse()
    if not any(d in ga_source.TIME_DIMENSIONS for d in dims):
        dims.append("date")
    return label.strip(), dims, metrics


def time_dimension(dims: list[str]) -> str:
    return next(d for d in dims if d in ga_source.TIME_DIMENSIONS)


def declaration(session: Any, parts: Any, dial: dict, name: str) -> ObjectDecl:
    """The report's declaration: the time dimension is the cursor, the dimensions the key."""
    _label, dims, _metrics = parse_report(name)
    return ObjectDecl(name=name, primary_key=tuple(dims), cursor=time_dimension(dims), lag_seconds=LOOKBACK_DAYS * 86_400)


def list_objects(session: Any, parts: Any, dial: dict) -> list[ObjectDecl]:
    """The reports the connector defines (`dial.objects`): GA4 cannot list them."""
    return [declaration(session, parts, dial, name) for name in dial.get("objects") or []]


def describe(session: Any, parts: Any, dial: dict, name: str) -> list[Field] | None:
    """The report's columns as the property's metadata types them, before the load
    (decision 9). A dimension or metric the property does not have is a rejected setting."""
    _label, dims, metrics = parse_report(name)
    meta = get_metadata(session, int(parts.account))
    known_dims = {d["apiName"] for d in meta.get("dimensions") or []}
    known_metrics = {m["apiName"]: m.get("type", "METRIC_TYPE_UNSPECIFIED") for m in meta.get("metrics") or []}
    if not set(dims) <= known_dims or not set(metrics) <= set(known_metrics):
        raise SaasSetupRefused("this property has no dimension or metric by one of those names")
    fields = [Field(d, "timestamp" if d in ga_source.TIME_DIMENSIONS else "text", nullable=False) for d in dims]
    for metric in metrics:
        kind = known_metrics[metric]
        # The columns are named and typed as the verified source names them: `<metric>_<TYPE>`.
        fields.append(Field(f"{metric}_{kind.split('_')[1]}", {"TYPE_INTEGER": "bigint", "METRIC_TYPE_UNSPECIFIED": "text"}.get(kind, "double")))
    return fields


def read(session: Any, parts: Any, dial: dict, decl: ObjectDecl, *, since: Any) -> Iterator[dict[str, Any]]:
    """The rows of a report from `LOOKBACK_DAYS` before `since`, or from the start of
    Google Analytics' data for a first run (`since` is `None`)."""
    label, dims, metrics = parse_report(decl.name)
    cursor = time_dimension(dims)
    yield from ga_helpers.basic_report(
        session,
        ROWS_PER_PAGE,
        dims,
        metrics,
        int(parts.account),
        label,
        START_DATE,
        SimpleNamespace(last_value=since, cursor_path=cursor),
        # `since` already is the cursor minus the lag (`ObjectDecl.lag_seconds`, dlt's own);
        # subtracting the lookback here too would read it twice.
        lookback_days=0,
    )


# ---- the service-account key (decision 3, second way) ---------------------------------------------

JWT_GRANT = "urn:ietf:params:oauth:grant-type:jwt-bearer"
# Google allows an assertion to live one hour; a little less leaves room for a clock that runs fast.
_ASSERTION_SECONDS = 3000
_EXPIRY_MARGIN_SECONDS = 60


def _b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def _service_account(secret: str) -> tuple[str, rsa.RSAPrivateKey]:
    """`(client_email, private key)` of a service-account key file.

    # Errors

    `SaasSetupRefused`, with a fixed text that quotes nothing of the file, for text that
    is not such a file or whose key is not an RSA private key.
    """
    refused = SaasSetupRefused("the Google service-account key is not a valid key file")
    try:
        document = json.loads(secret)
        email, pem = document["client_email"], document["private_key"]
        if document.get("type") != "service_account" or not isinstance(email, str) or not isinstance(pem, str):
            raise refused
        key = serialization.load_pem_private_key(pem.encode(), password=None)
    except (ValueError, KeyError, TypeError, UnsupportedAlgorithm):
        raise refused from None
    if not isinstance(key, rsa.RSAPrivateKey):
        raise refused
    return email, key


def build_assertion(email: str, key: rsa.RSAPrivateKey, scope: str, audience: str, now: float) -> str:
    """The signed JWT Google exchanges for an access token: header `RS256`/`JWT`, claims
    `iss`, `scope`, `aud`, `iat`, `exp`, signed with RSASSA-PKCS1-v1_5 and SHA-256."""
    header = _b64url(json.dumps({"alg": "RS256", "typ": "JWT"}, separators=(",", ":")).encode())
    claims = _b64url(
        json.dumps(
            {"iss": email, "scope": scope, "aud": audience, "iat": int(now), "exp": int(now) + _ASSERTION_SECONDS},
            separators=(",", ":"),
        ).encode()
    )
    signature = key.sign(f"{header}.{claims}".encode(), padding.PKCS1v15(), hashes.SHA256())
    return f"{header}.{claims}.{_b64url(signature)}"


def integration_auth(
    vendor: Any,
    secrets: dict[str, str],
    *,
    resolve_checked: Callable[..., Any],
    clock: Callable[[], float] | None = None,
) -> tuple[Callable[[], dict], Callable[[], None]]:
    """`(headers, refresh)` for a service-account key: a bearer token fetched when first
    needed, again when it has expired, and again on demand after a 401.

    The token is asked for at the vendor table's address (`rebase_for_gate` only moves it
    to the gate's mock under the gate-only switch) after the address check and pinning
    that `rest._token_request` applies.

    # Errors

    `SaasSetupRefused` for a key file that is not one; `OAuthSignInRefused` (at the first
    request) when Google refuses the assertion (a deleted or disabled key).
    """
    clock = clock or (lambda: time.time())  # looked up at the call: a test moves the time
    email, key = _service_account(secrets["serviceAccountJson"])
    audience = vendor.oauth["tokenUrl"]
    token_url = vendors.rebase_for_gate(vendor.id, audience)
    scope = vendor.oauth["scopes"][0]
    state: dict[str, Any] = {"headers": None, "expires_at": 0.0}

    def headers() -> dict:
        if state["headers"] is None or clock() >= state["expires_at"]:
            body = rest_adapter._token_request(
                token_url,
                {"grant_type": JWT_GRANT, "assertion": build_assertion(email, key, scope, audience, clock())},
                resolve_checked=resolve_checked,
            )
            lifetime = body.get("expires_in")
            valid = isinstance(lifetime, (int, float)) and not isinstance(lifetime, bool) and lifetime > 0
            # No stated lifetime: replaced only after a 401 (as `rest.AuthSession` does).
            state["expires_at"] = clock() + max(lifetime - _EXPIRY_MARGIN_SECONDS, 0) if valid else float("inf")
            state["headers"] = {"Authorization": f"Bearer {body['access_token']}"}
        return state["headers"]

    def refresh() -> None:
        state["headers"] = None

    return headers, refresh
