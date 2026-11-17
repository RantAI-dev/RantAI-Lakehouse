"""Zendesk Support (`SRC-3` V-Zendesk.2; decisions 5, 6, 7). The vendor module of
the `saas` adapter for `vendor = "zendesk"`; the code that talks to Zendesk is the
vendored dlt verified source (`saas/vendored/zendesk/`, AD4), read through the
`SaasSession` the adapter hands in. Chat and Talk are not offered (decision 5).

# Objects (decision 5)

Tickets, ticket events, ticket metric events, users, organizations, and the verified
source's other Support objects (groups, brands, SLA policies, macros, views, ...).

# Incremental or in full (decision 6)

Tickets, users, organizations, ticket events and ticket metric events have an
incremental export and load incrementally. Tickets and users use the cursor-based
export and ticket events and organizations the time-based one, whose repeated
items are dropped (vendored local change 3, upstream issue #659). Every other
object has no change filter and is re-read in full, which the connector page says.

# Deletes (decision 7)

A deleted ticket stays in the export with `status = "deleted"`
(https://developer.zendesk.com/documentation/api-basics/working-with-data/using-the-incremental-export-api/,
"Excluding deleted tickets"), so the same read that carries a change carries the
delete, and the row's status says it. Zendesk scrubs a deleted ticket's content after
30 days and drops the record from exports 90 days after a permanent deletion, so the
row is marked, not removed. Only tickets say `deletes`: no other export documents one.

# Rate limits

Incremental exports are limited to 10 requests a minute (30 with the High Volume
add-on) and Zendesk answers 429 with `Retry-After` (developer.zendesk.com, rate
limits); `SaasSession` waits that out within its bound (decision 13).

# Sign-in (decision 3), checked against Zendesk's documentation on 2026-10-19

* **Email + API token** (built): HTTP Basic with `<email>/token` as the user name
  and the token as the password
  (https://developer.zendesk.com/api-reference/introduction/security-and-auth/;
  Zendesk marks API tokens "deprecated" there in favour of OAuth, which is why the
  browser sign-in matters).
* **Browser sign-in** (built in the slice 2 fix round, BLOCKER 3): the row's addresses and
  scope are right (`/oauth/authorizations/new` with PKCE, scope `read`, refresh tokens that
  rotate: https://support.zendesk.com/hc/en-us/articles/4408845965210 and
  https://developer.zendesk.com/api-reference/ticketing/oauth/grant_type_tokens/), and
  `POST /oauth/tokens` "must be formatted as JSON", so the row's client authentication is
  `post_json` (`client_secret_post_json`: client id and secret in a JSON body).
* Revocation is `DELETE /api/v2/oauth/tokens/{id}`, not RFC 7009, so the table's
  revocation address stays empty.

# Schema changes (decision 9)

Zendesk does not describe its fields before a read (the decision lists the vendors that
do), so there is no `describe`: a changed field is seen after the load.
"""

from __future__ import annotations

import base64
import json
from collections.abc import Iterator
from datetime import datetime, timezone
from typing import Any

from dispar_orchestrate.adapters.saas import SaasSetupRefused
from dispar_orchestrate.saas.objects import ObjectDecl
from dispar_orchestrate.saas.vendored.zendesk import settings as zd_settings
from dispar_orchestrate.saas.vendored.zendesk.helpers import api_helpers
from dispar_orchestrate.saas.vendored.zendesk.helpers.talk_api import (
    INCREMENTAL_EXPORTS,
    PaginationType,
    ZendeskAPIClient,
)

# object name -> (the field the export is filtered by, deletes). The cursor is the
# field of the exported record whose last value the next run starts from.
_INCREMENTAL: dict[str, tuple[str, bool]] = {
    "tickets": ("updated_at", True),
    "users": ("updated_at", False),
    "organizations": ("updated_at", False),
    "ticket_events": ("timestamp", False),
    "ticket_metric_events": ("time", False),
}
_METRIC_EVENTS = "/api/v2/incremental/ticket_metric_events"

# name -> (endpoint, key of the items, cursor-paginated); the full-read objects of the
# verified source (users and organizations are incremental above, so left out).
_FULL: dict[str, tuple[str, str, bool]] = {
    name: (endpoint, data_key or name, cursor_paginated)
    for name, endpoint, data_key, cursor_paginated in [*zd_settings.SUPPORT_ENDPOINTS, *zd_settings.SUPPORT_EXTRA_ENDPOINTS]
    if name not in _INCREMENTAL
}
_FULL["ticket_fields"] = ("/api/v2/ticket_fields.json", "ticket_fields", False)
# Tags are a list of `{name, count}`: the name is the key.
_PRIMARY_KEY = {"tags": ("name",)}


def integration_headers(vendor: Any, secrets: dict[str, str]) -> dict[str, str]:
    """Email + API token: HTTP Basic with `<email>/token` as the user name
    (https://developer.zendesk.com/api-reference/introduction/security-and-auth/)."""
    raw = f"{secrets['email']}/token:{secrets['apiToken']}".encode()
    return {"Authorization": f"Basic {base64.b64encode(raw).decode()}"}


def declaration(session: Any, parts: Any, dial: dict, name: str) -> ObjectDecl:
    """The object's declaration (cursor, key, deletes, why it is re-read in full)."""
    if name in _INCREMENTAL:
        cursor, deletes = _INCREMENTAL[name]
        return ObjectDecl(name=name, primary_key=("id",), cursor=cursor, deletes=deletes)
    if name in _FULL:
        return ObjectDecl(
            name=name,
            primary_key=_PRIMARY_KEY.get(name, ("id",)),
            full_reason=f"Zendesk has no incremental export for {name.replace('_', ' ')}",
        )
    raise SaasSetupRefused("Zendesk has no object by that name")


def list_objects(session: Any, parts: Any, dial: dict) -> list[ObjectDecl]:
    return [declaration(session, parts, dial, name) for name in [*_INCREMENTAL, *_FULL]]


def _start_time(since: Any) -> int:
    """UNIX seconds to start an export from: `since` (an ISO text, a datetime or
    seconds, as the cursor field holds it), or the verified source's default start
    (2000-01-01) on a first run."""
    if since is None:
        return int(zd_settings.DEFAULT_START_DATE.int_timestamp)
    if isinstance(since, (int, float)):
        return int(since)
    if isinstance(since, datetime):
        return int((since if since.tzinfo else since.replace(tzinfo=timezone.utc)).timestamp())
    return int(datetime.fromisoformat(str(since).replace("Z", "+00:00")).timestamp())


def _ticket(record: dict[str, Any]) -> dict[str, Any]:
    """A ticket as the verified source cleans it, without pivoting the custom
    fields (a pivot names columns after field titles that an admin can rename, and
    needs the source's persisted state). `tags` and `custom_fields` are kept as one JSON
    text each, the column type the verified source gives them."""
    cleaned = api_helpers.process_ticket(record, {}, pivot_custom_fields=False)
    for key in ("tags", "custom_fields"):
        if key in cleaned:
            cleaned[key] = json.dumps(cleaned[key], sort_keys=True, default=str)
    return cleaned


def read(session: Any, parts: Any, dial: dict, decl: ObjectDecl, *, since: Any) -> Iterator[dict[str, Any]]:
    """The records of an object. `since` is the cursor's last value, or `None` for
    the first run, which reads everything."""
    client = ZendeskAPIClient(session)
    name = decl.name
    if name == "ticket_metric_events":
        pages = client.get_pages(_METRIC_EVENTS, name, PaginationType.CURSOR, params={"start_time": _start_time(since)})
    elif name in _INCREMENTAL:
        assert name in INCREMENTAL_EXPORTS
        pages = client.get_incremental(name, _start_time(since))
    elif name in _FULL:
        endpoint, key, cursor_paginated = _FULL[name]
        pages = client.get_pages(endpoint, key, PaginationType.CURSOR if cursor_paginated else PaginationType.OFFSET)
    else:
        raise SaasSetupRefused("Zendesk has no object by that name")
    for page in pages:
        for record in page:
            yield _ticket(record) if name == "tickets" else record
