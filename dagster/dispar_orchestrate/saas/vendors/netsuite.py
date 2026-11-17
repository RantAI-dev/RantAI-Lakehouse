"""NetSuite (`SRC-3` slice 6, V-NetSuite.1 to .3; decisions 2, 3, 5, 6, 7, 9, 10, 13). The vendor
module of the `saas` adapter for `vendor = "netsuite"`: SuiteQL over REST, not SuiteAnalytics
JDBC (decision 2).

# `rest_api` or `SaasSession` (AD5, plan section 8 item 3)

As in slices 3 to 5, the endpoint is called through `SaasSession` (address check and pinning
before each request, the credential asked for after it, no redirects, bounded `Retry-After`),
here with `post_json`; the rows go to the same dlt resource every vendor uses. No library is
added: the assertion is signed with `cryptography`, already pinned for Google's key.

# Reading (SuiteQL over REST)

`POST /services/rest/query/v1/suiteql?limit=<n>&offset=<n>` with the header `Prefer: transient`
(required) and the body `{"q": "<query>"}`; the answer has `items`, `hasMore`, `count`,
`offset` and `totalResults`
(https://docs.oracle.com/en/cloud/saas/netsuite/ns-online-help/section_157909186990.html).
The account in the address is the table's, built from `account` (`1234567_SB1` is host
`1234567-sb1`, Oracle's "URLs for Account-Specific Domains",
https://docs.oracle.com/en/cloud/saas/netsuite/ns-online-help/section_1498251763.html).
Each item carries a `links` array, which is dropped (data, never dialled); a column that is null
is left out of an item by NetSuite, so a record lacks the key rather than holding `null`.

# Objects (decision 5): any record type SuiteQL can read

An object is a SuiteQL table name (`customer`, `transaction`, ...). Suggested are `SUGGESTED`;
any other table is named in the dial's `objects`. A name is checked to be a plain lower-case
identifier before it is put into a query (`check_table`); an unknown table, or one the role may
not read, is a rejected setting (the vendor's words are not repeated). NetSuite offers no list
of the tables SuiteQL reads that this connector could call, so no list is read.

# Incremental or in full (decision 6)

A table with a `lastmodifieddate` column loads incrementally and merges on `id`. The column's
value in an answer depends on the user's date format, so the query selects its own text form
(`TO_CHAR(lastmodifieddate, 'YYYY-MM-DD HH24:MI:SS') AS modified_at`) and `modified_at` is the
cursor, loaded as a timestamp; the filter compares `lastmodifieddate` with `TO_TIMESTAMP(...)`.
Which time zone NetSuite means by either is not stated on the pages read, so the cursor lags a
day (`LAG_SECONDS`) and the merge removes the repeats (as ServiceNow). A table without that
column is read in full ordered by `id`, with the reason on the page. **None of the SuiteQL
functions used here was run against a NetSuite account** (decision 10: no free developer
account); they are from Oracle's SuiteQL documentation and the developer's knowledge.

# Paging and the 100,000-row limit

SuiteQL stops at 100,000 rows per query. A table read in full pages by key, never by offset
(`WHERE id > <last id>`, offset 0), so it has no limit. An incremental read pages by `offset`
and `hasMore` and, before the offset reaches `RESTART_AT`, starts the query again from the last
change time seen (`>=`; the repeated rows are merged); if that time did not move, more than
`RESTART_AT` rows share one change time and the run stops with a plain message.

# Deletes (decision 7)

NetSuite reports none in a SuiteQL read (a deleted record is gone from the table, and the
deleted-record log is another table this connector does not read), so no object says `deletes`.

# Schema changes (decision 9)

Decision 9 lists NetSuite among the vendors that describe their fields before the load. SuiteQL's
columns are not the REST record API's properties, and no endpoint that lists a SuiteQL table's
columns was found in the documentation read, so there is NO `describe`: a changed column is seen
after the load, and the plan's handoff says so.

# Sign-in (decision 3)

* **Browser sign-in**: the vendor table's row (authorise at `<account>.app.netsuite.com`, token at
  `<account>.suitetalk.api.netsuite.com/services/rest/auth/oauth2/v1/token`, the client in the
  Basic header, scope `rest_webservices`;
  https://docs.oracle.com/en/cloud/saas/netsuite/ns-online-help/section_160855590788.html).
* **Client credentials with a certificate** (`integration_auth`): an assertion signed with the
  certificate's private key, `PS256` for an RSA key and `ES256` for a P-256 key (NetSuite does
  not take `RS256`), header `kid` = the certificate id from NetSuite's "OAuth 2.0 Client
  Credentials (M2M) Setup" (the dial's `certificateId`), claims `iss` the integration's client
  id, `scope` `rest_webservices`, `aud` the account's token address, `iat`, `exp`; sent to the
  token address as `grant_type=client_credentials`, `client_assertion_type=urn:ietf:params:oauth:
  client-assertion-type:jwt-bearer` and `client_assertion`
  (https://docs.oracle.com/en/cloud/saas/netsuite/ns-online-help/section_0312092247.html).
"""

from __future__ import annotations

import base64
import json
import re
import time
from collections.abc import Callable, Iterator
from datetime import datetime, timezone
from typing import Any

import requests
from cryptography.exceptions import UnsupportedAlgorithm
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec, padding, rsa, utils

from dispar_orchestrate.adapters import rest as rest_adapter
from dispar_orchestrate.adapters.saas import SaasSetupRefused
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.objects import ObjectDecl

# A SuiteQL query is built by NetSuite from the table's rows; a big table's page can take a while.
request_timeout_seconds = 120

QUERY_PATH = "/services/rest/query/v1/suiteql"
PAGE_SIZE = 1000
RESTART_AT = 90_000  # rows by offset, under the 100,000 NetSuite allows per query
LAG_SECONDS = 86_400
CURSOR_FIELD = "modified_at"
CHANGED_COLUMN = "lastmodifieddate"
KEY_FIELD = "id"
SUGGESTED = ("customer", "vendor", "employee", "item", "transaction", "account", "subsidiary", "contact")
_TABLE_NAME = re.compile(r"^[a-z][a-z0-9_]{0,63}$")
_STAMP = "%Y-%m-%d %H:%M:%S"
_EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)
_PREFER = {"Prefer": "transient"}


def _refuse() -> SaasSetupRefused:
    return SaasSetupRefused("NetSuite has no record type by that name that this account can read")


def check_table(name: str) -> str:
    """`name` when it is a plain SuiteQL table name.

    # Errors

    `SaasSetupRefused` for anything else (the name goes into a query).
    """
    if not _TABLE_NAME.match(name):
        raise _refuse()
    return name


def _full_reason(name: str) -> str:
    return f"{name} has no {CHANGED_COLUMN} column to filter by"


def _incremental(name: str) -> ObjectDecl:
    return ObjectDecl(name=name, primary_key=(KEY_FIELD,), cursor=CURSOR_FIELD, lag_seconds=LAG_SECONDS)


def _full(name: str) -> ObjectDecl:
    return ObjectDecl(name=name, primary_key=(KEY_FIELD,), full_reason=_full_reason(name))


def _query(session: Any, sql: str, *, offset: int = 0, limit: int | None = None) -> dict:
    """One SuiteQL request. A 400 is NetSuite saying the query cannot run (a table or column it does
    not have, or one the role may not read); it is a rejected setting and the words are not repeated.

    # Errors

    `SaasSetupRefused` for 400; the session's failures otherwise.
    """
    try:
        body = session.post_json(f"{QUERY_PATH}?limit={limit or PAGE_SIZE}&offset={offset}", {"q": sql}, headers=_PREFER)
    except requests.HTTPError as exc:
        if getattr(exc.response, "status_code", None) == 400:
            raise _refuse() from None
        raise
    except ValueError:
        raise SaasSetupRefused("NetSuite answered in a form this connector does not read") from None
    if not isinstance(body, dict) or not isinstance(body.get("items"), list):
        raise SaasSetupRefused("NetSuite answered in a form this connector does not read")
    return body


def declaration(session: Any, parts: Any, dial: dict, name: str) -> ObjectDecl:
    """The table's declaration: incremental by `lastmodifieddate` merged on `id` when the table has
    both columns, read in full when it has `id` only, a rejected setting when it has neither.

    Asked with a one-row query for the two columns (a missing column is the vendor's 400, so no
    sample row has to hold a value in it).
    """
    check_table(name)
    try:
        _query(session, f"SELECT {KEY_FIELD}, {CHANGED_COLUMN} FROM {name}", limit=1)
        return _incremental(name)
    except SaasSetupRefused:
        pass
    _query(session, f"SELECT {KEY_FIELD} FROM {name}", limit=1)  # refused again: no such table, or no id
    return _full(name)


def list_objects(session: Any, parts: Any, dial: dict) -> list[ObjectDecl]:
    """The suggested tables and the ones named in the dial. How each loads is settled when it is read, by
    `declaration`; the listing shows the incremental form (a table without the column is told when read)."""
    names = [*SUGGESTED, *(n for n in (dial.get("objects") or []) if isinstance(n, str) and n not in SUGGESTED)]
    return [_incremental(check_table(name)) for name in names]


def _stamp(since: Any) -> str:
    """`since` as the text of a SuiteQL timestamp literal.

    # Errors

    `SaasSetupRefused` for a value that is not a date and time.
    """
    try:
        moment = since if isinstance(since, datetime) else datetime.fromisoformat(str(since).strip().replace("Z", "+00:00"))
    except ValueError:
        raise SaasSetupRefused("the saved cursor of this NetSuite table is not a date and time") from None
    if moment.tzinfo is not None:
        moment = moment.astimezone(timezone.utc)
    return moment.strftime(_STAMP)


def _as_time(text: Any) -> datetime:
    try:
        return datetime.strptime(str(text), _STAMP).replace(tzinfo=timezone.utc)
    except ValueError:
        raise SaasSetupRefused("NetSuite sent a change time this connector does not read") from None


def _select(name: str, decl: ObjectDecl, where: str, order: str) -> str:
    columns = f"{name}.*" + (f", TO_CHAR({name}.{CHANGED_COLUMN}, 'YYYY-MM-DD HH24:MI:SS') AS {CURSOR_FIELD}" if decl.cursor else "")
    return f"SELECT {columns} FROM {name}{where} ORDER BY {order}"


def _records(body: dict, decl: ObjectDecl) -> Iterator[dict[str, Any]]:
    for item in body["items"]:
        if not isinstance(item, dict):
            continue
        item.pop("links", None)  # NetSuite's own addresses: data, never dialled
        if decl.cursor:
            # NetSuite leaves a null column out of an item; dlt's incremental needs the cursor on
            # every record, so a record with no change time gets the epoch in `modified_at` only (it
            # is loaded once, by the first run, and the real column stays absent).
            item[decl.cursor] = _as_time(item[decl.cursor]) if decl.cursor in item else _EPOCH
        yield item


def _read_by_key(session: Any, decl: ObjectDecl) -> Iterator[dict[str, Any]]:
    """A table read in full: ordered by `id`, each page after the last id (offset 0 always), so the
    100,000-row limit of one query never applies."""
    name = decl.name
    last: int | None = None
    while True:
        where = f" WHERE {name}.{KEY_FIELD} > {last}" if last is not None else ""
        body = _query(session, _select(name, decl, where, f"{name}.{KEY_FIELD}"))
        rows = list(_records(body, decl))
        yield from rows
        if not body.get("hasMore") or not rows:
            return
        try:
            newest = int(str(rows[-1][KEY_FIELD]))
        except (KeyError, ValueError):
            raise SaasSetupRefused("NetSuite sent a record id this connector does not read") from None
        if last is not None and newest <= last:
            raise SaasSetupRefused("NetSuite answered in a form this connector does not read")
        last = newest


def _read_by_time(session: Any, decl: ObjectDecl, since: Any) -> Iterator[dict[str, Any]]:
    """An incremental table: ordered by change time and id, by `offset` and `hasMore`, started again
    from the last change time before the offset nears SuiteQL's row limit."""
    name = decl.name
    start = _stamp(since) if since is not None else None
    while True:
        where = (
            f" WHERE {name}.{CHANGED_COLUMN} >= TO_TIMESTAMP('{start}', 'YYYY-MM-DD HH24:MI:SS')" if start is not None else ""
        )
        sql = _select(name, decl, where, f"{name}.{CHANGED_COLUMN}, {name}.{KEY_FIELD}")
        offset, newest = 0, None
        while True:
            body = _query(session, sql, offset=offset)
            rows = list(_records(body, decl))
            yield from rows
            if rows:
                newest = rows[-1].get(CURSOR_FIELD, newest)
            offset += len(rows)
            if not body.get("hasMore") or not rows:
                return
            if offset >= RESTART_AT:
                break
        stamp = newest.strftime(_STAMP) if isinstance(newest, datetime) else None
        if stamp is None or stamp == start:
            raise SaasSetupRefused("more rows of this NetSuite table share one change time than one query can return")
        start = stamp


def read(session: Any, parts: Any, dial: dict, decl: ObjectDecl, *, since: Any) -> Iterator[dict[str, Any]]:
    """The records of a table. `since` is the cursor's last value (already moved back by the lag), or
    `None` for the first run, which reads everything."""
    check_table(decl.name)
    if decl.cursor:
        return _read_by_time(session, decl, since)
    return _read_by_key(session, decl)


# ---- the certificate (decision 3, second way) -------------------------------------------------------

CLIENT_ASSERTION_TYPE = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer"
# NetSuite's page shows a two-minute assertion; a little longer leaves room for a clock that runs fast.
_ASSERTION_SECONDS = 600
_EXPIRY_MARGIN_SECONDS = 60
_SIGNING_KEY = rsa.RSAPrivateKey | ec.EllipticCurvePrivateKey


def _b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def _load_key(pem: str) -> _SIGNING_KEY:
    """The certificate's private key.

    # Errors

    `SaasSetupRefused`, with a fixed text that quotes nothing of the key, for text that is not a
    PEM RSA or P-256 private key.
    """
    refused = SaasSetupRefused("the NetSuite private key is not a usable RSA or P-256 key")
    try:
        key = serialization.load_pem_private_key(pem.encode(), password=None)
    except (ValueError, TypeError, UnsupportedAlgorithm):
        raise refused from None
    if isinstance(key, rsa.RSAPrivateKey) or (isinstance(key, ec.EllipticCurvePrivateKey) and isinstance(key.curve, ec.SECP256R1)):
        return key
    raise refused


def build_assertion(client_id: str, certificate_id: str, key: _SIGNING_KEY, scope: str, audience: str, now: float) -> str:
    """The signed JWT NetSuite exchanges for a token: header `PS256` (RSASSA-PSS, SHA-256, salt of the
    hash's length) or `ES256` (raw `r || s`), `typ` `JWT` and `kid` the certificate id; claims `iss`,
    `scope`, `aud`, `iat`, `exp`."""
    algorithm = "PS256" if isinstance(key, rsa.RSAPrivateKey) else "ES256"
    header = _b64url(json.dumps({"alg": algorithm, "typ": "JWT", "kid": certificate_id}, separators=(",", ":")).encode())
    claims = _b64url(
        json.dumps(
            {"iss": client_id, "scope": scope, "aud": audience, "iat": int(now), "exp": int(now) + _ASSERTION_SECONDS},
            separators=(",", ":"),
        ).encode()
    )
    signing_input = f"{header}.{claims}".encode()
    if isinstance(key, rsa.RSAPrivateKey):
        signature = key.sign(signing_input, padding.PSS(mgf=padding.MGF1(hashes.SHA256()), salt_length=hashes.SHA256.digest_size), hashes.SHA256())
    else:
        r, s = utils.decode_dss_signature(key.sign(signing_input, ec.ECDSA(hashes.SHA256())))
        signature = r.to_bytes(32, "big") + s.to_bytes(32, "big")
    return f"{header}.{claims}.{_b64url(signature)}"


def integration_auth(
    vendor: Any,
    secrets: dict[str, str],
    *,
    resolve_checked: Callable[..., Any],
    parts: Any,
    dial: dict | None = None,
    clock: Callable[[], float] | None = None,
) -> tuple[Callable[[], dict], Callable[[], None]]:
    """`(headers, refresh)` for the certificate way: a bearer token fetched when first needed, again
    when it has expired, and again on demand after a 401.

    The token is asked for at the vendor table's address for this account (`rebase_for_gate` only moves
    it to the gate's mock under the gate-only switch); the assertion's `aud` is that real address.

    # Errors

    `SaasSetupRefused` for a missing certificate id or a key that is not usable; `OAuthSignInRefused`
    (at the first request) when NetSuite refuses the assertion.
    """
    clock = clock or (lambda: time.time())  # looked up at the call: a test moves the time
    certificate_id = str((dial or {}).get("certificateId") or "").strip()
    if not certificate_id:
        raise SaasSetupRefused("the NetSuite certificate id is not set on the connector")
    key = _load_key(secrets["privateKey"])
    client_id = secrets["clientId"]
    audience = vendor.token_url(parts)
    token_url = vendors.rebase_for_gate(vendor.id, audience)
    scope = vendor.oauth["scopes"][0]
    state: dict[str, Any] = {"headers": None, "expires_at": 0.0}

    def headers() -> dict:
        if state["headers"] is None or clock() >= state["expires_at"]:
            body = rest_adapter._token_request(
                token_url,
                {
                    "grant_type": "client_credentials",
                    "client_assertion_type": CLIENT_ASSERTION_TYPE,
                    "client_assertion": build_assertion(client_id, certificate_id, key, scope, audience, clock()),
                },
                resolve_checked=resolve_checked,
            )
            lifetime = body.get("expires_in")
            valid = isinstance(lifetime, (int, float)) and not isinstance(lifetime, bool) and lifetime > 0
            state["expires_at"] = clock() + max(lifetime - _EXPIRY_MARGIN_SECONDS, 0) if valid else float("inf")
            state["headers"] = {"Authorization": f"Bearer {body['access_token']}"}
        return state["headers"]

    def refresh() -> None:
        state["headers"] = None

    return headers, refresh
