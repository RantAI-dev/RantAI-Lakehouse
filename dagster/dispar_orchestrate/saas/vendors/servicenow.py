"""ServiceNow (`SRC-3` slice 4, V-ServiceNow.1 to .3; decisions 3, 5, 6, 7, 9, 13). The
vendor module of the `saas` adapter for `vendor = "servicenow"`.

# `rest_api` or `SaasSession` (AD5, plan section 8 item 3)

AD5 names a dlt `rest_api` configuration for the vendors without a verified source. dlt
1.30.0's `RESTClient` does take a caller-supplied session, but a `requests.Session`
cannot carry `SaasSession`'s rules: the address check and pinning BEFORE each request,
the credential asked for only after that check, no redirects, the bounded `Retry-After`
wait and the same-origin rule on a `Link` target are all in `SaasSession._send`. So, as in
slice 3, the endpoints are called through `SaasSession` directly and the records are
handed to the same dlt resource every other vendor uses (`adapters/saas.py`); no
`rest_api` source is built and no library is added.

# Objects (decision 5)

ServiceNow stores everything in tables, so an object is a table name. `incident`,
`change_request`, `problem`, `sys_user` and `cmdb_ci` are suggested; any other table the
connector's account may read is added by name (a name is checked to be a plain table
name before it is used; an unknown table is a rejected setting).

# Schema changes before the load (decision 9)

`describe` reads `sys_dictionary` (and `sys_db_object` for the parent tables), so "ask first"
and "pause" can hold a table back. Those tables need a role the connector is not promised; when the account may
not read them `describe` answers `None` and the change is seen after the load, as for the vendors that
do not describe. Not tried against a real instance.

# Reading (Table API, https://www.servicenow.com/docs/r/xanadu/api-reference/rest-apis/c_TableAPI.html)

`GET /api/now/table/{table}` with `sysparm_query`, `sysparm_limit` and
`sysparm_exclude_reference_link=true` (a reference field is then its `sys_id` text and
not an object, so every value is a string, as the Table API sends it with
`sysparm_display_value` off). The pages are followed by the `Link` header's `rel="next"`
target, which `SaasSession` accepts only on the same origin (decision 8). Offset paging is
not used: the Table API applies `sysparm_limit` BEFORE the access check, so a page may hold
fewer records than the limit and a short page cannot be taken for the last one.

# Incremental or in full (decision 6)

`sys_updated_on` (UTC, `YYYY-MM-DD HH:MM:SS`, loaded as a timestamp) is the cursor of every table that has it:
`sys_updated_on>=<since>^ORDERBYsys_updated_on^ORDERBYsys_id`, merged on `sys_id`. A table
that has no such field is re-read in full, and the connector page says so. Whether an encoded
query's date literal is read in UTC or in the calling user's time zone is not stated on the
page above, so the cursor lags by a day (`LAG_SECONDS`): the filter then over-reads by more
than any time zone offset and never misses a record; the merge on `sys_id` removes the repeats.
Not tried against a real instance.

# Deletes (decision 7)

The Table API does not report a deleted record in the table's own read. ServiceNow records
deletions in the table `sys_audit_delete`, which is an ordinary table this connector can read
by name where the account may; reading it was NOT tried, so no object claims `deletes` and a
record deleted at ServiceNow stays in Bronze until a full reload (the connector page says so).

# Rate limits (decision 13)

A 429 with `Retry-After` is waited out by `SaasSession` within its bound.

# Sign-in (decision 3)

* **Browser sign-in**: the vendor table's row (`oauth_auth.do`, `oauth_token.do`; the
  client in the form body).
* **Client credentials** (the integration credential, built here): `POST /oauth_token.do`
  with `grant_type=client_credentials`, `client_id` and `client_secret` in the form body
  (https://www.servicenow.com/docs/r/platform-security/authentication/client-credentials-grant-workflow.html).
  The instance must allow the grant (`glide.oauth.inbound.client.credential.grant_type.enabled`)
  and the OAuth application must name an application user whose roles bound what is read;
  a refused exchange surfaces as a refused sign-in. The token is kept until shortly before
  it expires and asked for again after a 401.
"""

from __future__ import annotations

import re
import time
from collections.abc import Callable, Iterator
from datetime import datetime, timezone
from typing import Any

import requests

from dispar_orchestrate.adapters import rest as rest_adapter
from dispar_orchestrate.adapters.saas import Field, SaasSetupRefused
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.objects import ObjectDecl
from dispar_orchestrate.saas.session import SaasPermissionMissing

TABLE_API = "/api/now/table"
CURSOR_FIELD = "sys_updated_on"
KEY_FIELD = "sys_id"
PAGE_SIZE = 1000
LAG_SECONDS = 86_400
MAX_PARENTS = 12
SUGGESTED = ("incident", "change_request", "problem", "sys_user", "cmdb_ci")
_TABLE_NAME = re.compile(r"^[a-z][a-z0-9_]{0,79}$")


def _refuse() -> SaasSetupRefused:
    return SaasSetupRefused("ServiceNow has no table by that name")


def check_table(name: str) -> str:
    """`name` when it is a plain ServiceNow table name.

    # Errors

    `SaasSetupRefused` for anything else (the name goes into a request path and a query).
    """
    if not _TABLE_NAME.match(name):
        raise _refuse()
    return name


def _full_reason(name: str) -> str:
    return f"{name} has no {CURSOR_FIELD} field to filter by"


def _incremental(name: str) -> ObjectDecl:
    return ObjectDecl(name=name, primary_key=(KEY_FIELD,), cursor=CURSOR_FIELD, lag_seconds=LAG_SECONDS)


def declaration(session: Any, parts: Any, dial: dict, name: str) -> ObjectDecl:
    """The table's declaration, from one sample record: incremental by `sys_updated_on` merged
    on `sys_id`, or re-read in full (with the reason) when the table has no change time.

    The sample asks for the two fields only. A table the account cannot read raises the
    session's permission failure; a table ServiceNow does not know is a rejected setting; a
    table with no records, or none this account may see, is taken to be a normal table.
    """
    check_table(name)
    try:
        body = session.get_json(
            f"{TABLE_API}/{name}", {"sysparm_limit": 1, "sysparm_fields": f"{KEY_FIELD},{CURSOR_FIELD}", "sysparm_exclude_reference_link": "true"}
        )
    except requests.HTTPError as exc:
        if getattr(exc.response, "status_code", None) in (400, 404):
            raise _refuse() from None
        raise
    found = body.get("result") if isinstance(body, dict) else None
    if not isinstance(found, list) or not found:
        return _incremental(name)
    sample = found[0] if isinstance(found[0], dict) else {}
    if KEY_FIELD not in sample:
        raise SaasSetupRefused("this ServiceNow table has no sys_id, so its records cannot be told apart")
    if CURSOR_FIELD not in sample:
        return ObjectDecl(name=name, primary_key=(KEY_FIELD,), full_reason=_full_reason(name))
    return _incremental(name)


def list_objects(session: Any, parts: Any, dial: dict) -> list[ObjectDecl]:
    """The suggested tables and the ones the connector was given by name (decision 5:
    any table by name; ServiceNow's table list is too large and too access-dependent to
    offer whole). How each loads is settled when it is read, by `declaration`."""
    names = [*SUGGESTED, *(n for n in (dial.get("objects") or []) if isinstance(n, str) and n not in SUGGESTED)]
    return [_incremental(check_table(name)) for name in names]


def _table_fields(session: Any, table: str) -> list[str]:
    rows = session.get_json(
        f"{TABLE_API}/sys_dictionary",
        {"sysparm_query": f"name={table}^elementISNOTEMPTY", "sysparm_fields": "element", "sysparm_limit": 10_000, "sysparm_exclude_reference_link": "true"},
    )
    return [str(r["element"]) for r in rows.get("result", []) if isinstance(r, dict) and r.get("element")]


def _parent_table(session: Any, table: str) -> str | None:
    rows = session.get_json(
        f"{TABLE_API}/sys_db_object",
        {"sysparm_query": f"name={table}", "sysparm_fields": "super_class.name", "sysparm_limit": 1, "sysparm_exclude_reference_link": "true"},
    )
    found = rows.get("result") or []
    parent = found[0].get("super_class.name") if found and isinstance(found[0], dict) else None
    return check_table(parent) if isinstance(parent, str) and parent else None


def describe(session: Any, parts: Any, dial: dict, name: str) -> list[Field] | None:
    """The table's fields before the load (decision 9), from `sys_dictionary`, which holds
    the fields a table defines itself, so the table's parents are walked (`sys_db_object`,
    `super_class`; `incident` extends `task`). `None` (the after-load path) when the account
    may not read the dictionary, which needs a role this connector is not promised
    (decision 9 says "where the vendor describes its fields"); every field comes as text
    because the Table API sends every value as text, except the change time.

    # Errors

    `SaasSetupRefused` when a parent chain is longer than `MAX_PARENTS` (a loop in the answer).
    """
    check_table(name)
    names: list[str] = []
    table: str | None = name
    try:
        for _ in range(MAX_PARENTS):
            if table is None:
                break
            names += _table_fields(session, table)
            table = _parent_table(session, table)
        else:
            raise SaasSetupRefused("ServiceNow answered with a table hierarchy this connector does not read")
    except SaasPermissionMissing:
        return None
    if not names:
        return None
    return [Field(n, "timestamp" if n == CURSOR_FIELD else "text", nullable=n != KEY_FIELD) for n in sorted(set(names))]


def _stamp(since: Any) -> str:
    """`since` (the cursor's last value, as text or a datetime) as ServiceNow writes it.

    # Errors

    `SaasSetupRefused` for a value that is not a date and time.
    """
    try:
        moment = since if isinstance(since, datetime) else datetime.fromisoformat(str(since).strip().replace("Z", "+00:00"))
    except ValueError:
        raise SaasSetupRefused("the saved cursor of this ServiceNow table is not a date and time") from None
    if moment.tzinfo is not None:
        moment = moment.astimezone(timezone.utc)
    return moment.strftime("%Y-%m-%d %H:%M:%S")


def _as_time(text: Any) -> datetime:
    """`sys_updated_on` as a UTC `datetime`: dlt's incremental compares the cursor with a
    time (it applies the lag to one), and the column is a timestamp in Bronze.

    # Errors

    `SaasSetupRefused` when ServiceNow sent something that is not a date and time.
    """
    try:
        return datetime.strptime(str(text), "%Y-%m-%d %H:%M:%S").replace(tzinfo=timezone.utc)
    except ValueError:
        raise SaasSetupRefused("ServiceNow sent a change time this connector does not read") from None


def _query(decl: ObjectDecl, since: Any) -> str:
    order = f"ORDERBY{CURSOR_FIELD}^ORDERBY{KEY_FIELD}" if decl.cursor else f"ORDERBY{KEY_FIELD}"
    if decl.cursor and since is not None:
        return f"{CURSOR_FIELD}>={_stamp(since)}^{order}"
    return order


def read(session: Any, parts: Any, dial: dict, decl: ObjectDecl, *, since: Any) -> Iterator[dict[str, Any]]:
    """The records of a table, page by page along the `Link` header. `since` is the cursor's
    last value (already moved back by the lag), or `None` for the first run, which reads
    everything."""
    params: dict[str, Any] | None = {
        "sysparm_query": _query(decl, since),
        "sysparm_limit": PAGE_SIZE,
        "sysparm_exclude_reference_link": "true",
    }
    target: str | None = f"{TABLE_API}/{check_table(decl.name)}"
    while target is not None:
        try:
            body, target = session.get_json_with_next(target, params)
        except requests.HTTPError as exc:
            if getattr(exc.response, "status_code", None) in (400, 404):
                raise _refuse() from None
            raise
        params = None  # the next link carries its own query
        result = body.get("result") if isinstance(body, dict) else None
        if not isinstance(result, list):
            raise SaasSetupRefused("ServiceNow answered in a form this connector does not read")
        for record in result:
            if isinstance(record, dict):
                if decl.cursor and decl.cursor in record:
                    record[decl.cursor] = _as_time(record[decl.cursor])
                yield record


# ---- client credentials (decision 3, second way) ------------------------------------------------

_EXPIRY_MARGIN_SECONDS = 60


def integration_auth(
    vendor: Any,
    secrets: dict[str, str],
    *,
    resolve_checked: Callable[..., Any],
    parts: Any,
    clock: Callable[[], float] | None = None,
) -> tuple[Callable[[], dict], Callable[[], None]]:
    """`(headers, refresh)` for the client-credentials grant: a bearer token fetched when
    first needed, again once it has expired, and again on demand after a 401.

    The token is asked for at the vendor table's address for this instance (`rebase_for_gate`
    only moves it to the gate's mock under the gate-only switch), after the address check and
    pinning that `rest._token_request` applies.

    # Errors

    `OAuthSignInRefused` (at the first request) when ServiceNow refuses the exchange.
    """
    clock = clock or (lambda: time.time())  # looked up at the call: a test moves the time
    token_url = vendors.rebase_for_gate(vendor.id, vendor.token_url(parts))
    form = {"grant_type": "client_credentials", "client_id": secrets["clientId"], "client_secret": secrets["clientSecret"]}
    state: dict[str, Any] = {"headers": None, "expires_at": 0.0}

    def headers() -> dict:
        if state["headers"] is None or clock() >= state["expires_at"]:
            body = rest_adapter._token_request(token_url, form, resolve_checked=resolve_checked)
            lifetime = body.get("expires_in")
            valid = isinstance(lifetime, (int, float)) and not isinstance(lifetime, bool) and lifetime > 0
            # No stated lifetime: replaced only after a 401 (as `rest.AuthSession` does).
            state["expires_at"] = clock() + max(lifetime - _EXPIRY_MARGIN_SECONDS, 0) if valid else float("inf")
            state["headers"] = {"Authorization": f"Bearer {body['access_token']}"}
        return state["headers"]

    def refresh() -> None:
        state["headers"] = None

    return headers, refresh
