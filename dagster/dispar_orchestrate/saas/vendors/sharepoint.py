"""SharePoint (`SRC-3` slice 5, V-SharePoint.1 to .3; decisions 3, 5, 6, 7, 13). The vendor module
of the `saas` adapter for `vendor = "sharepoint"`, on Microsoft Graph
(https://learn.microsoft.com/en-us/graph/api/resources/sharepoint).

# `rest_api` or `SaasSession` (AD5, plan section 8 item 3)

As for ServiceNow and Confluence: the endpoints are called through `SaasSession`, which carries
the address check, the credential-after-check order, no redirects, the bounded `Retry-After` and
the same-origin rule on `@odata.nextLink` and `@odata.deltaLink`; a dlt `rest_api` source over a
plain `requests.Session` would not. No library is added.

# Objects (decision 5): the lists of the sites the person picks

An object is one list, named `<site id>/<list id>`: the site id is Graph's `hostname,collection
GUID,web GUID` and the list id a GUID, both checked to be exactly that before they go into a request
path. Files are not loaded (the feature page's "Not included"): a document library is a list in
Graph, and it is left out of the listing, as are hidden lists.

*Which sites.* Sites are found with `GET /sites?search=*`
(https://learn.microsoft.com/en-us/graph/api/site-search; the plain `GET /sites` list is
application-only, https://learn.microsoft.com/en-us/graph/api/site-list, so a browser sign-in could
not use it). The person narrows them by naming site ids in the dial's `objects`; with none named the
first `MAX_SITES` found are listed, and a larger tenant must name its sites. `search` was NOT tried
against a tenant (no Microsoft tenant is available to this project, decision 11).

# Reading: the delta query (decisions 6 and 7; plan AD5, section 8 item 6)

`GET /sites/{site}/lists/{list}/items/delta?$expand=fields` (permission `Sites.Read.All`, delegated or
application: https://learn.microsoft.com/en-us/graph/api/listitem-delta). The feed shows "the latest
state for each item, not each change"; it ends in `@odata.deltaLink`, which the NEXT run opens to
get only what changed. A deleted item comes as `{"id": ..., "deleted": {"state": "deleted"}}`.

* **The delta link lives in the resource's dlt state** (`state["delta_link"]`), set only after the
  last page of a full enumeration was handed on, so a read that fails halfway keeps the old link;
  dlt commits the state with the load of the rows and the sink restores it after a failed load
  (`sink._abort_pending`). A test runs this against a real Iceberg table.
* **A deleted item is marked, not removed** (like Salesforce and Zendesk): its row keeps only its
  `id`, `is_deleted = true` and the time this run saw the delete as `modified_at` (Graph sends none).
  Every other row has `is_deleted = false`.
* **410 Gone** ("the service can't provide a list of changes for a given token", e.g. an old token;
  the answer has a `Location` header that starts a fresh enumeration) is a full resync: the read
  starts again WITHOUT a token, reads every item and stores the new link. We do not follow the
  `Location` (a link from a header; the plain delta address needs no token). What a resync cannot do
  is learn which items were deleted while the link was stale: Graph's own advice is to compare with
  local state, which the sink cannot do, so those rows stay until a full reload (the page says so).
* A run in a load mode that is not incremental (`replace`, `append`) has no state and reads the
  list with `items?$expand=fields` in full; no deletes are seen.
* The cursor is `modified_at` (Graph's `lastModifiedDateTime`); the sink's cursor filter and merge on
  `id` handle the items a feed repeats.

# Sign-in (decision 3), checked on 2026-10-21

Both ways go to `login.microsoftonline.com/{tenant}/oauth2/v2.0/...` with the customer's tenant (a
GUID or a domain name, validated as part of the account, AD2):
* **Browser sign-in**: authorisation code with PKCE, `Sites.Read.All` (delegated) and
  `offline_access`; the refresh token is replaced on every refresh, which the existing rotated-token
  report handles; the v2 platform has no RFC 7009 revocation endpoint, so the table has none
  (https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow).
* **Client credentials** (the integration credential; `saas/client_credentials.py`): scope
  `https://graph.microsoft.com/.default`, the app's `Sites.Read.All` application permission granted by an
  admin; no refresh token is issued
  (https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-client-creds-grant-flow).
"""

from __future__ import annotations

import json
import re
from collections.abc import Callable, Iterator
from datetime import datetime, timezone
from typing import Any

import requests

from dispar_orchestrate.adapters.saas import SaasSetupRefused
from dispar_orchestrate.saas.client_credentials import integration_auth  # noqa: F401 -- the module protocol looks it up by name
from dispar_orchestrate.saas.objects import ObjectDecl

API = "/v1.0"
CURSOR_FIELD = "modified_at"
PAGE_SIZE = 200
MAX_SITES = 100
MAX_LISTS = 500
uses_state = True

_GUID = r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"
# `\Z`, not `$`: `$` also matches before a trailing newline, which must not reach a request path.
_SITE_ID = re.compile(rf"^[A-Za-z0-9][A-Za-z0-9.\-]{{0,252}},{_GUID},{_GUID}\Z")
_LIST_ID = re.compile(rf"^{_GUID}\Z")

# Looked up at the call: a test moves the clock.
now: Callable[[], datetime] = lambda: datetime.now(timezone.utc)  # noqa: E731


def _refuse() -> SaasSetupRefused:
    return SaasSetupRefused("SharePoint has no list by that name")


def check_object(name: str) -> tuple[str, str]:
    """`(site id, list id)` of an object name `<site id>/<list id>`.

    # Errors

    `SaasSetupRefused` for anything else (both ids go into a request path).
    """
    site, slash, list_id = name.rpartition("/")
    if not slash or not _SITE_ID.match(site) or not _LIST_ID.match(list_id):
        raise _refuse()
    return site, list_id


def _decl(name: str) -> ObjectDecl:
    return ObjectDecl(name=name, primary_key=("id",), cursor=CURSOR_FIELD, deletes=True)


def declaration(session: Any, parts: Any, dial: dict, name: str) -> ObjectDecl:
    """Every list is read by its delta feed: incremental on `modified_at`, merged on `id`, deletes marked."""
    check_object(name)
    return _decl(name)


def _pages(session: Any, target: str, params: dict | None) -> Iterator[dict[str, Any]]:
    """The answers of a Graph collection, along `@odata.nextLink` (same origin, by the session)."""
    link: str | None = target
    while link is not None:
        body = session.get_json(link, params)
        params = None  # the link carries its own query
        if not isinstance(body, dict) or not isinstance(body.get("value"), list):
            raise SaasSetupRefused("Microsoft Graph answered in a form this connector does not read")
        yield body
        link = body.get("@odata.nextLink") if isinstance(body.get("@odata.nextLink"), str) else None


def _picked_sites(dial: dict) -> list[str]:
    return [n for n in (dial.get("objects") or []) if isinstance(n, str) and _SITE_ID.match(n)]


def list_sites(session: Any, dial: dict) -> list[str]:
    """The site ids to list lists of: the ones the dial names, else the first `MAX_SITES` that
    `GET /sites?search=*` finds (personal sites left out)."""
    picked = _picked_sites(dial)
    if picked:
        return picked
    found: list[str] = []
    for body in _pages(session, f"{API}/sites", {"search": "*", "$select": "id,isPersonalSite", "$top": PAGE_SIZE}):
        for site in body["value"]:
            if isinstance(site, dict) and isinstance(site.get("id"), str) and not site.get("isPersonalSite") and _SITE_ID.match(site["id"]):
                found.append(site["id"])
        if len(found) >= MAX_SITES:
            break
    return found[:MAX_SITES]


def list_objects(session: Any, parts: Any, dial: dict) -> list[ObjectDecl]:
    """The lists of the sites (document libraries and hidden lists left out: files are not loaded)."""
    out: list[ObjectDecl] = []
    for site in list_sites(session, dial):
        for body in _pages(session, f"{API}/sites/{site}/lists", {"$select": "id,list", "$top": PAGE_SIZE}):
            for entry in body["value"]:
                if not isinstance(entry, dict) or not isinstance(entry.get("id"), str) or not _LIST_ID.match(entry["id"]):
                    continue
                facet = entry.get("list") if isinstance(entry.get("list"), dict) else {}
                if facet.get("hidden") or facet.get("template") == "documentLibrary":
                    continue
                out.append(_decl(f"{site}/{entry['id']}"))
                if len(out) >= MAX_LISTS:
                    return out
    return out


def _as_time(value: Any) -> datetime:
    """A Graph timestamp (`2021-10-14T23:27:27Z`) as a UTC `datetime`.

    # Errors

    `SaasSetupRefused` for something that is not a date and time.
    """
    try:
        moment = datetime.fromisoformat(str(value).strip().replace("Z", "+00:00"))
    except ValueError:
        raise SaasSetupRefused("Microsoft Graph sent a time this connector does not read") from None
    return moment.astimezone(timezone.utc) if moment.tzinfo else moment.replace(tzinfo=timezone.utc)


def _flatten(item: dict[str, Any], seen_at: datetime) -> dict[str, Any]:
    """One list item as a flat row. A deleted item keeps its id, `is_deleted` and the time it was seen."""
    if "id" not in item:
        raise SaasSetupRefused("Microsoft Graph sent a list item with no id")
    if item.get("deleted") or "@removed" in item:
        return {"id": str(item["id"]), "is_deleted": True, CURSOR_FIELD: seen_at}
    if not item.get("lastModifiedDateTime"):
        raise SaasSetupRefused("Microsoft Graph sent a list item with no change time")
    content = item.get("contentType") if isinstance(item.get("contentType"), dict) else {}
    row: dict[str, Any] = {
        "id": str(item["id"]),
        "created_at": _as_time(item["createdDateTime"]) if item.get("createdDateTime") else None,
        CURSOR_FIELD: _as_time(item["lastModifiedDateTime"]),
        "web_url": item.get("webUrl"),
        "etag": item.get("eTag"),
        "content_type": content.get("name"),
        "is_deleted": False,
    }
    fields = item.get("fields") if isinstance(item.get("fields"), dict) else {}
    for key, value in fields.items():
        if key.startswith("@"):
            continue
        # A lookup, a person or a managed-metadata field is an object or a list: one JSON text.
        row[f"field_{key}"] = json.dumps(value, sort_keys=True) if isinstance(value, (dict, list)) else value
    return row


def _full(session: Any, base: str) -> Iterator[dict[str, Any]]:
    seen_at = now()
    for body in _pages(session, base, {"$expand": "fields", "$top": PAGE_SIZE}):
        for item in body["value"]:
            if isinstance(item, dict):
                yield _flatten(item, seen_at)


def _enumerate(session: Any, first: str, params: dict | None) -> Iterator[dict[str, Any] | str]:
    """A delta enumeration: the rows, then (last) the `@odata.deltaLink` as a string.

    # Errors

    `requests.HTTPError` (including the 410 the caller turns into a resync), `SaasSetupRefused`.
    """
    seen_at = now()
    link: str | None = first
    while link is not None:
        body = session.get_json(link, params)
        params = None
        if not isinstance(body, dict) or not isinstance(body.get("value"), list):
            raise SaasSetupRefused("Microsoft Graph answered in a form this connector does not read")
        for item in body["value"]:
            if isinstance(item, dict):
                yield _flatten(item, seen_at)
        if isinstance(body.get("@odata.nextLink"), str):
            link = body["@odata.nextLink"]
            continue
        delta = body.get("@odata.deltaLink")
        if not isinstance(delta, str):
            raise SaasSetupRefused("Microsoft Graph ended a change feed without a link to continue from")
        yield delta
        link = None


def read(session: Any, parts: Any, dial: dict, decl: ObjectDecl, *, since: Any, state: Any = None) -> Iterator[dict[str, Any]]:
    """The rows of a list. With `state` (an incremental load) the delta feed is used and its link kept;
    without it (any other load mode) the list is read in full. `since` is not used: the delta link, not a
    time, says where the last run stopped."""
    site, list_id = check_object(decl.name)
    base = f"{API}/sites/{site}/lists/{list_id}/items"
    if state is None:
        try:
            yield from _full(session, base)
        except requests.HTTPError as exc:
            if getattr(exc.response, "status_code", None) in (400, 404):
                raise _refuse() from None
            raise
        return
    saved = state.get("delta_link")
    fresh = (f"{base}/delta", {"$expand": "fields", "$top": PAGE_SIZE})
    attempts = [(saved, None), fresh] if isinstance(saved, str) and saved else [fresh]
    for number, (first, params) in enumerate(attempts):
        try:
            for produced in _enumerate(session, first, params):
                if isinstance(produced, str):
                    state["delta_link"] = produced  # only now: the whole feed was handed on
                else:
                    yield produced
            return
        except requests.HTTPError as exc:
            status = getattr(exc.response, "status_code", None)
            if status == 410 and number + 1 < len(attempts):
                # A full resync (listItem: delta, "410 Gone"). Rows already handed on are real changes;
                # the merge on `id` absorbs the repeats.
                state.pop("delta_link", None)
                continue
            if status in (400, 404):
                raise _refuse() from None
            raise
