"""Dynamics 365 (`SRC-3` slice 5, V-Dynamics.1 to .3; decisions 2, 3, 5, 6, 7, 9, 13). The vendor module of
the `saas` adapter for `vendor = "dynamics365"`, on the Dataverse Web API
(https://learn.microsoft.com/en-us/power-apps/developer/data-platform/webapi/overview), not Synapse Link
(decision 2).

# `rest_api` or `SaasSession` (AD5, plan section 8 item 3)

As for ServiceNow, Confluence and SharePoint: the endpoints are called through `SaasSession`, which carries
the address check, the credential-after-check order, no redirects, the bounded `Retry-After` and the
same-origin rule on `@odata.nextLink` and `@odata.deltaLink`. No library is added.

# Objects (decision 5): any Dataverse table

An object is a table's logical name (`account`, `contact`, `new_widget`). The list is
`EntityDefinitions` (not paged: "there's no paging. The first response returns all matching resources",
https://learn.microsoft.com/en-us/power-apps/developer/data-platform/webapi/query-metadata-web-api) without
private and intersect tables. A name goes into a request path and a `$select`, so it is checked to be a plain
logical name first; a table Dataverse does not know is a rejected setting.

# Schema changes before the load (decision 9)

`describe` reads the table's `Attributes` from `EntityDefinitions(LogicalName='...')?$expand=Attributes(...)`
and names the columns a read selects: a lookup column is read as `_<name>_value` (its Web API form), and
virtual, formula-less display companions (`AttributeOf` set), images, files and other columns the Web API does
not return as plain values are left out.

# Incremental or in full (decision 6) and deletes (decision 7)

* **Change tracking** (`Prefer: odata.track-changes`; the table's `ChangeTrackingEnabled`,
  https://learn.microsoft.com/en-us/power-apps/developer/data-platform/use-change-tracking-synchronize-data-external-systems,
  read 2026-10-21) is the one way Dataverse reports a delete. For a table that has it, the first read asks for
  tracking and every page's `@odata.nextLink` is followed to the `@odata.deltaLink`, which is kept in the
  resource's dlt state with the `$select` it belongs to; the next run opens it and gets the changed rows and
  the deleted ones (`@odata.context` ending `$deletedEntity`, `id`, `reason`). A deleted row is marked, not
  removed (`is_deleted`, its key, the time this run saw it as `modifiedon`). Tracking takes no `$filter`,
  `$orderby`, `$expand` or `$top`. Changes are kept for 7 days by default (`ExpireChangeTrackingInDays`); an
  older link makes Dataverse "throw an exception" whose HTTP status the page does not give, so a client error
  on the SAVED link is taken as a full resync (a read from a fresh tracking request, which stores a new link).
  What a resync cannot learn is which rows were deleted while the link was stale; they stay until a full reload.
  A change-tracking read is not filtered by `modifiedon`: the link says what changed, and a change can be
  stamped before the last run's newest row (final review BLOCKER 17).
* **`modifiedon`** is the cursor of every other table that has it: `$filter=modifiedon ge <since>` ordered by
  `modifiedon, <key>` ("Specify an order for your query ... include a column that has a unique identifier"),
  merged on the table's `PrimaryIdAttribute`. The cursor lags by `LAG_SECONDS`, a margin of ours and not a
  figure Microsoft documents (a row can commit after a later one was stamped). No delete is seen for such a
  table, and the connector page says so. A table with no `modifiedon` is re-read in full, with the reason.
* Paging is `Prefer: odata.maxpagesize` (at most 5,000) with the same preference on every page and the
  `@odata.nextLink` used as given
  (https://learn.microsoft.com/en-us/power-apps/developer/data-platform/webapi/query/page-results).

# Sign-in (decision 3), checked on 2026-10-21

The tenant is a validated part of the account (`<org>.crm4.dynamics.com/<tenant>`, AD2); both ways go to
`login.microsoftonline.com/{tenant}/oauth2/v2.0/...`.
* **Browser sign-in**: authorisation code with PKCE, `https://<org>/user_impersonation` and `offline_access`;
  the refresh token is replaced on each refresh; the v2 platform has no RFC 7009 revocation endpoint
  (https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow).
* **Client credentials** (`saas/client_credentials.py`): scope `https://<org>/.default`; the app must also
  be added in the environment as an application user with a security role
  (https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-client-creds-grant-flow); a missing
  application user surfaces as a refused call (403).
"""

from __future__ import annotations

import re
from collections.abc import Callable, Iterator
from datetime import datetime, timezone
from typing import Any

import requests

from dispar_orchestrate.adapters.saas import Field, SaasSetupRefused
from dispar_orchestrate.saas.client_credentials import integration_auth  # noqa: F401 -- the module protocol looks it up by name
from dispar_orchestrate.saas.objects import ObjectDecl

API = "/api/data/v9.2"
CURSOR_FIELD = "modifiedon"
LAG_SECONDS = 300
PAGE_SIZE = 500
uses_state = True

_LOGICAL = re.compile(r"^[a-z][a-z0-9_]{0,63}\Z")
_SET_NAME = re.compile(r"^[A-Za-z][A-Za-z0-9_]{0,127}\Z")
_HEADERS = {"Accept": "application/json", "OData-MaxVersion": "4.0", "OData-Version": "4.0"}
# AttributeType values the Web API returns as no plain column (virtual companions, files, images, party lists...).
_SKIPPED_TYPES = frozenset({"Virtual", "ManagedProperty", "EntityName", "CalendarRules", "PartyList", "Image", "File"})
_LOOKUP_TYPES = frozenset({"Lookup", "Owner", "Customer"})
_TYPES = {
    "Integer": "bigint", "BigInt": "bigint", "Picklist": "bigint", "State": "bigint", "Status": "bigint",
    "Decimal": "double", "Double": "double", "Money": "double",
    "Boolean": "bool", "DateTime": "timestamp",
}  # fmt: skip

# Looked up at the call: a test moves the clock.
now: Callable[[], datetime] = lambda: datetime.now(timezone.utc)  # noqa: E731


def _refuse() -> SaasSetupRefused:
    return SaasSetupRefused("Dynamics 365 has no table by that name")


def check_table(name: str) -> str:
    """`name` when it is a plain Dataverse logical name.

    # Errors

    `SaasSetupRefused` for anything else (the name goes into a request path and a `$select`).
    """
    if not _LOGICAL.match(name):
        raise _refuse()
    return name


def _prefer(*extra: str) -> dict[str, str]:
    return {**_HEADERS, "Prefer": ",".join([*extra, f"odata.maxpagesize={PAGE_SIZE}"])}


class _Table:
    """What a read needs to know about one table, from its definition."""

    def __init__(self, definition: dict[str, Any]) -> None:
        entity_set, key = definition.get("EntitySetName"), definition.get("PrimaryIdAttribute")
        if not isinstance(entity_set, str) or not _SET_NAME.match(entity_set) or not isinstance(key, str) or not _LOGICAL.match(key):
            raise SaasSetupRefused("Dynamics 365 sent a table definition this connector does not read")
        self.entity_set, self.key = entity_set, key
        self.tracked = definition.get("ChangeTrackingEnabled") is True
        # (data column, type) pairs, by the attribute they come from
        self.columns: list[tuple[str, str]] = []
        for attribute in definition.get("Attributes") or []:
            if not isinstance(attribute, dict) or attribute.get("AttributeOf") or attribute.get("IsValidForRead") is False:
                continue
            logical, kind = attribute.get("LogicalName"), attribute.get("AttributeType")
            if not isinstance(logical, str) or not _LOGICAL.match(logical) or kind in _SKIPPED_TYPES:
                continue
            if kind in _LOOKUP_TYPES:
                self.columns.append((f"_{logical}_value", "text"))
            else:
                self.columns.append((logical, _TYPES.get(str(kind), "text")))
        self.names = [name for name, _type in self.columns]
        if self.key not in self.names:
            raise SaasSetupRefused("Dynamics 365 sent a table with no readable key")
        self.has_cursor = CURSOR_FIELD in self.names

    @property
    def select(self) -> str:
        return ",".join(self.names)


def _definition(session: Any, name: str) -> _Table:
    check_table(name)
    try:
        body = session.get_json(
            f"{API}/EntityDefinitions(LogicalName='{name}')",
            {
                "$select": "LogicalName,EntitySetName,PrimaryIdAttribute,ChangeTrackingEnabled",
                "$expand": "Attributes($select=LogicalName,AttributeType,AttributeOf,IsValidForRead)",
            },
            headers=_HEADERS,
        )
    except requests.HTTPError as exc:
        if getattr(exc.response, "status_code", None) in (400, 404):
            raise _refuse() from None
        raise
    if not isinstance(body, dict):
        raise SaasSetupRefused("Dynamics 365 answered in a form this connector does not read")
    return _Table(body)


def _decl(name: str, table: _Table) -> ObjectDecl:
    if not table.has_cursor:
        return ObjectDecl(name=name, primary_key=(table.key,), full_reason=f"{name} has no {CURSOR_FIELD} column to filter by")
    if table.tracked:
        # PR #216 final review BLOCKER 17: a change-tracking read replays the saved delta link, so
        # it is not filtered by `modifiedon` (a change can be stamped before the last run's newest
        # row, or before a delete's mark). No lag applies: there is no cursor filter to move back.
        return ObjectDecl(name=name, primary_key=(table.key,), cursor=CURSOR_FIELD, deletes=True, delta=True)
    return ObjectDecl(name=name, primary_key=(table.key,), cursor=CURSOR_FIELD, lag_seconds=LAG_SECONDS, deletes=table.tracked)


def declaration(session: Any, parts: Any, dial: dict, name: str) -> ObjectDecl:
    """The table's declaration, from its definition: its key, `modifiedon` as the cursor when it has one,
    and deletes when change tracking is on."""
    return _decl(name, _definition(session, name))


def list_objects(session: Any, parts: Any, dial: dict) -> list[ObjectDecl]:
    """Every public, non-intersect table. The `modifiedon` column is asked for in the same call (`$expand` with a
    filter), so the list can say which tables are incremental; if Dataverse refuses that form, the tables are
    listed as incremental and `declaration` (called by every load) tells the truth."""
    base = {"$select": "LogicalName,EntitySetName,PrimaryIdAttribute,ChangeTrackingEnabled", "$filter": "IsPrivate eq false and IsIntersect eq false"}
    try:
        body = session.get_json(
            f"{API}/EntityDefinitions",
            {**base, "$expand": f"Attributes($select=LogicalName;$filter=LogicalName eq '{CURSOR_FIELD}')"},
            headers=_HEADERS,
        )
        known_cursor = True
    except requests.HTTPError as exc:
        if getattr(exc.response, "status_code", None) != 400:
            raise
        body, known_cursor = session.get_json(f"{API}/EntityDefinitions", base, headers=_HEADERS), False
    rows = body.get("value") if isinstance(body, dict) else None
    if not isinstance(rows, list):
        raise SaasSetupRefused("Dynamics 365 answered in a form this connector does not read")
    out: list[ObjectDecl] = []
    for entry in rows:
        if not isinstance(entry, dict):
            continue
        name, key, entity_set = entry.get("LogicalName"), entry.get("PrimaryIdAttribute"), entry.get("EntitySetName")
        if not (isinstance(name, str) and _LOGICAL.match(name) and isinstance(key, str) and _LOGICAL.match(key) and isinstance(entity_set, str)):
            continue
        if known_cursor and not entry.get("Attributes"):
            out.append(ObjectDecl(name=name, primary_key=(key,), full_reason=f"{name} has no {CURSOR_FIELD} column to filter by"))
        elif entry.get("ChangeTrackingEnabled") is True:
            # Final review BLOCKER 17: the delta link, not a cursor filter, says what changed.
            out.append(ObjectDecl(name=name, primary_key=(key,), cursor=CURSOR_FIELD, deletes=True, delta=True))
        else:
            out.append(ObjectDecl(name=name, primary_key=(key,), cursor=CURSOR_FIELD, lag_seconds=LAG_SECONDS, deletes=False))
    return out


def describe(session: Any, parts: Any, dial: dict, name: str) -> list[Field] | None:
    """The columns a read of the table selects, before the load (decision 9), and the `is_deleted` mark every row has."""
    table = _definition(session, name)
    return [Field(column, kind, nullable=column != table.key) for column, kind in table.columns] + [Field("is_deleted", "bool", nullable=False)]


def _as_time(value: Any) -> datetime:
    """A Dataverse timestamp (`2026-10-01T10:00:00Z`) or a cursor value as a UTC `datetime`.

    # Errors

    `SaasSetupRefused` for something that is not a date and time.
    """
    try:
        moment = value if isinstance(value, datetime) else datetime.fromisoformat(str(value).strip().replace("Z", "+00:00"))
    except ValueError:
        raise SaasSetupRefused("Dynamics 365 sent a time this connector does not read") from None
    return moment.astimezone(timezone.utc) if moment.tzinfo else moment.replace(tzinfo=timezone.utc)


def _flatten(table: _Table, record: dict[str, Any], seen_at: datetime) -> dict[str, Any]:
    context = record.get("@odata.context")
    if isinstance(context, str) and context.endswith("$deletedEntity"):
        if not isinstance(record.get("id"), str):
            raise SaasSetupRefused("Dynamics 365 sent a deleted row with no id")
        return {table.key: record["id"], "is_deleted": True, CURSOR_FIELD: seen_at}
    row = {k: v for k, v in record.items() if not k.startswith("@")}
    if table.has_cursor:
        if not row.get(CURSOR_FIELD):
            raise SaasSetupRefused("Dynamics 365 sent a row with no change time")
        row[CURSOR_FIELD] = _as_time(row[CURSOR_FIELD])
    row["is_deleted"] = False
    return row


def _pages(session: Any, first: str, params: dict | None, headers: dict) -> Iterator[dict[str, Any]]:
    """The answers of a Dataverse collection, along `@odata.nextLink` as given (same origin, by the session)."""
    link: str | None = first
    while link is not None:
        body = session.get_json(link, params, headers=headers)
        params = None  # the link carries its own query
        if not isinstance(body, dict) or not isinstance(body.get("value"), list):
            raise SaasSetupRefused("Dynamics 365 answered in a form this connector does not read")
        yield body
        link = body.get("@odata.nextLink") if isinstance(body.get("@odata.nextLink"), str) else None


def _stamp(since: Any) -> str:
    return _as_time(since).strftime("%Y-%m-%dT%H:%M:%SZ")


def _tracked(session: Any, table: _Table, state: dict) -> Iterator[dict[str, Any]]:
    """A table with change tracking: from the saved delta link when there is one for this `$select`, else a
    fresh tracking request. The new link is stored only after the last page was handed on."""
    seen_at = now()
    saved = state.get("delta")
    fresh = (f"{API}/{table.entity_set}", {"$select": table.select}, _prefer("odata.track-changes"))
    usable = isinstance(saved, dict) and saved.get("select") == table.select and isinstance(saved.get("link"), str)
    attempts = [(saved["link"], None, _prefer()), fresh] if usable else [fresh]
    for number, (first, params, headers) in enumerate(attempts):
        try:
            link: str | None = first
            while link is not None:
                body = session.get_json(link, params, headers=headers)
                params = None
                if not isinstance(body, dict) or not isinstance(body.get("value"), list):
                    raise SaasSetupRefused("Dynamics 365 answered in a form this connector does not read")
                for record in body["value"]:
                    if isinstance(record, dict):
                        yield _flatten(table, record, seen_at)
                if isinstance(body.get("@odata.nextLink"), str):
                    link = body["@odata.nextLink"]
                    continue
                delta = body.get("@odata.deltaLink")
                if not isinstance(delta, str):
                    raise SaasSetupRefused("Dynamics 365 ended a change feed without a link to continue from")
                state["delta"] = {"link": delta, "select": table.select}
                link = None
            return
        except requests.HTTPError as exc:
            status = getattr(exc.response, "status_code", None)
            if number + 1 < len(attempts) and isinstance(status, int) and 400 <= status < 500:
                # The saved link is no longer served (changes are kept for 7 days by default): a full resync.
                state.pop("delta", None)
                continue
            raise


def read(session: Any, parts: Any, dial: dict, decl: ObjectDecl, *, since: Any, state: Any = None) -> Iterator[dict[str, Any]]:
    """The rows of a table. With `state` and change tracking: the delta feed. With a cursor and a `since`: the
    rows modified since, in `modifiedon, key` order. Otherwise every row in key order."""
    table = _definition(session, decl.name)
    if state is not None and decl.deletes and table.tracked:
        yield from _tracked(session, table, state)
        return
    params: dict[str, Any] = {"$select": table.select}
    if decl.cursor and since is not None:
        params["$filter"] = f"{CURSOR_FIELD} ge {_stamp(since)}"
        params["$orderby"] = f"{CURSOR_FIELD} asc,{table.key} asc"
    else:
        params["$orderby"] = f"{table.key} asc"
    seen_at = now()
    for body in _pages(session, f"{API}/{table.entity_set}", params, _prefer()):
        for record in body["value"]:
            if isinstance(record, dict):
                yield _flatten(table, record, seen_at)

