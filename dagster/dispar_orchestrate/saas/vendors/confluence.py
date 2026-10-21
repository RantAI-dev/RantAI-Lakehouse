"""Confluence Cloud (`SRC-3` slice 4, V-Confluence.1 to .3; decisions 3, 5, 6, 7, 13). The
vendor module of the `saas` adapter for `vendor = "confluence"`, on the REST API v2
(https://developer.atlassian.com/cloud/confluence/rest/v2/intro/).

# `rest_api` or `SaasSession` (AD5, plan section 8 item 3)

As for ServiceNow (`servicenow.py`): the endpoints are called through `SaasSession`, which
carries the address check, the credential-after-check order, no redirects, the bounded
`Retry-After` and the same-origin rule on a link; a dlt `rest_api` source over a plain
`requests.Session` would not. No library is added.

# Objects (decision 5)

`spaces`, `pages`, `blogposts` and `comments` (footer and inline comments, told apart by
`comment_type`). Pages and blog posts are loaded with their storage-format body in `body`;
attachments are not loaded (the feature page's "Not included"). Nested values are flattened
(`version.createdAt` is `modified_at`, `version.number` is `version_number`).

# Paging

Every list takes `limit` (at most 250 per the OpenAPI document) and `cursor`; the next page is
the `_links.next` of the answer. That link is relative to the site, and behind Atlassian's OAuth
gateway the site's `/wiki/...` path is NOT under the gateway's `/ex/confluence/<cloud id>` prefix,
so a relative link is prefixed with `parts.base_path` when it does not already start with it. An
absolute link goes to `SaasSession` as is: the same-origin rule stops any other host (decision 8).
Which of the two forms Atlassian sends was not observed (no real site); both are handled and tested.

# Incremental or in full (decision 6)

The list endpoints have no date filter, but pages, blog posts and comments sort by
`-modified-date` (the `PageSortOrder`, `BlogPostSortOrder` and `CommentSortOrder` of the v2 OpenAPI
document, https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json). So an incremental
read walks newest first and stops at the first record modified before the cursor: sound because the
order is by the very field the cursor is (`version.createdAt`, the time of the last version), and a
record edited while the read runs is newer than everything this run saw, so the NEXT run starts
before it. The cursor lags by `LAG_SECONDS`, a margin of ours and not a figure Atlassian documents,
against a listing that is not instantly consistent. Spaces cannot be sorted or filtered by time and
are re-read in full.

# Deletes (decision 7)

A page or blog post moved to the trash is no longer in the default listing and nothing reports it,
so no object claims `deletes`: it stays in Bronze until a full reload (the connector page says so).

# Rate limits (decision 13)

429 with `Retry-After` (https://developer.atlassian.com/cloud/confluence/rate-limiting/) is waited
out by `SaasSession` within its bound; an hourly quota that is used up answers 429 until the next
UTC hour, which is far beyond the bound, so the run fails as `rate_limited`.

# Sign-in (decision 3), checked against Atlassian's documentation on 2026-10-21

* **Browser sign-in** (OAuth 2.0 3LO, https://developer.atlassian.com/cloud/confluence/oauth-2-3lo-apps/):
  `https://auth.atlassian.com/authorize` with `audience=api.atlassian.com` and `prompt=consent`;
  `offline_access` for a refresh token; the token endpoint takes a JSON body (`post_json`); the refresh
  token ROTATES (the old one is disabled), which the existing rotated-token report handles; a failed
  exchange is HTTP 403 `invalid_grant`. The API is at `https://api.atlassian.com/ex/confluence/<cloud id>/`
  (the table's `oauth.apiBase`, a fixed host); the cloud id comes from
  `GET https://api.atlassian.com/oauth/token/accessible-resources` and is matched to the connector's
  site by the site address, never taken from a typed value; `resolve_base_path` validates its shape.
  The page documents no revocation endpoint, so the table has none.
* **Email + API token** (built): HTTP Basic, the email as the user name, at `<site>.atlassian.net`.
"""

from __future__ import annotations

import base64
import re
from collections.abc import Iterator
from datetime import datetime, timezone
from typing import Any
from urllib.parse import urlparse

from dispar_orchestrate.adapters.saas import SaasSetupRefused
from dispar_orchestrate.saas.objects import ObjectDecl

API = "/wiki/api/v2"
CURSOR_FIELD = "modified_at"
LAG_SECONDS = 300
PAGE_LIMIT = 100
SPACE_LIMIT = 250
# `\Z`, not `$`: `$` also matches before a trailing newline, and this id goes into every request path.
_UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\Z")

# object -> (endpoints it reads, whether it carries a body and a version)
_ENDPOINTS: dict[str, tuple[tuple[str, ...], bool]] = {
    "spaces": (("/spaces",), False),
    "pages": (("/pages",), True),
    "blogposts": (("/blogposts",), True),
    "comments": (("/footer-comments", "/inline-comments"), True),
}
_COMMENT_TYPE = {"/footer-comments": "footer", "/inline-comments": "inline"}


def integration_headers(vendor: Any, secrets: dict[str, str]) -> dict[str, str]:
    """Email + API token: HTTP Basic with the email as the user name
    (https://developer.atlassian.com/cloud/confluence/basic-auth-for-rest-apis/)."""
    raw = f"{secrets['email']}:{secrets['apiToken']}".encode()
    return {"Authorization": f"Basic {base64.b64encode(raw).decode()}"}


def resolve_base_path(vendor: Any, parts: Any, session: Any) -> str:
    """The gateway path prefix of a browser sign-in: `/ex/confluence/<cloud id>`, with the cloud
    id of the signed-in account's site whose address is this connector's site.

    # Errors

    `SaasSetupRefused` when the account has no Confluence site by that name or the answer holds
    an id that is not a UUID (the id goes into every later request path).
    """
    sites = session.get_json("/oauth/token/accessible-resources")
    wanted = f"{parts.account}.atlassian.net"
    for site in sites if isinstance(sites, list) else []:
        if not isinstance(site, dict) or not isinstance(site.get("url"), str):
            continue
        scopes = [s for s in site.get("scopes") or [] if isinstance(s, str)]
        if (urlparse(site["url"]).hostname or "").lower() != wanted or not any("confluence" in s for s in scopes):
            continue
        cloud_id = site.get("id")
        if not isinstance(cloud_id, str) or not _UUID.match(cloud_id):
            raise SaasSetupRefused("Atlassian answered with a site id this connector does not accept")
        return urlparse(vendor.oauth["apiBase"]).path.replace("{cloudId}", cloud_id)
    raise SaasSetupRefused("the signed-in Atlassian account has no Confluence site by that name")


def _refuse() -> SaasSetupRefused:
    return SaasSetupRefused("Confluence has no object by that name")


def declaration(session: Any, parts: Any, dial: dict, name: str) -> ObjectDecl:
    """Spaces are re-read in full; pages, blog posts and comments are read newest first and stop at the cursor."""
    if name not in _ENDPOINTS:
        raise _refuse()
    if name == "spaces":
        return ObjectDecl(name=name, primary_key=("id",), full_reason="Confluence cannot sort or filter spaces by change time")
    return ObjectDecl(name=name, primary_key=("id",), cursor=CURSOR_FIELD, lag_seconds=LAG_SECONDS)


def list_objects(session: Any, parts: Any, dial: dict) -> list[ObjectDecl]:
    return [declaration(session, parts, dial, name) for name in _ENDPOINTS]


def _as_time(value: Any) -> datetime:
    """A Confluence timestamp (`2026-10-01T10:00:00.000Z`) or a cursor value as a UTC `datetime`.

    # Errors

    `SaasSetupRefused` for something that is not a date and time.
    """
    try:
        moment = value if isinstance(value, datetime) else datetime.fromisoformat(str(value).strip().replace("Z", "+00:00"))
    except ValueError:
        raise SaasSetupRefused("Confluence sent a change time this connector does not read") from None
    return moment.astimezone(timezone.utc) if moment.tzinfo else moment.replace(tzinfo=timezone.utc)


def _flatten(record: dict[str, Any], *, content: bool, comment_type: str | None) -> dict[str, Any]:
    out = {k: v for k, v in record.items() if not isinstance(v, (dict, list))}
    if content:
        version = record.get("version") if isinstance(record.get("version"), dict) else {}
        out["version_number"] = version.get("number")
        out["version_author_id"] = version.get("authorId")
        if not version.get("createdAt"):
            raise SaasSetupRefused("Confluence sent a record with no change time")
        out[CURSOR_FIELD] = _as_time(version["createdAt"])
        storage = (record.get("body") or {}).get("storage") if isinstance(record.get("body"), dict) else None
        out["body"] = storage.get("value") if isinstance(storage, dict) else None
    if comment_type:
        out["comment_type"] = comment_type
    return out


def _next(parts: Any, link: Any) -> str | None:
    """The target of an answer's `_links.next`: a relative link gets the gateway prefix when it
    lacks it; an absolute one is left to the session's same-origin rule."""
    if not isinstance(link, str) or not link:
        return None
    if link.startswith("/") and not link.startswith("//") and parts.base_path and not link.startswith(parts.base_path + "/"):
        return parts.base_path + link
    return link


def _list(session: Any, parts: Any, path: str, params: dict[str, Any]) -> Iterator[dict[str, Any]]:
    target: str | None = f"{parts.base_path}{API}{path}"
    query: dict[str, Any] | None = params
    while target is not None:
        body = session.get_json(target, query)
        query = None  # the next link carries its own query
        results = body.get("results") if isinstance(body, dict) else None
        if not isinstance(results, list):
            raise SaasSetupRefused("Confluence answered in a form this connector does not read")
        yield from (r for r in results if isinstance(r, dict))
        target = _next(parts, (body.get("_links") or {}).get("next") if isinstance(body.get("_links"), dict) else None)


def read(session: Any, parts: Any, dial: dict, decl: ObjectDecl, *, since: Any) -> Iterator[dict[str, Any]]:
    """The records of an object. `since` is the cursor's last value (already moved back by the
    lag), or `None` for the first run. Content is read newest first and the read stops at the first
    record modified before `since`."""
    if decl.name not in _ENDPOINTS:
        raise _refuse()
    paths, content = _ENDPOINTS[decl.name]
    floor = _as_time(since) if since is not None and decl.cursor else None
    for path in paths:
        params: dict[str, Any] = {"limit": PAGE_LIMIT if content else SPACE_LIMIT}
        if content:
            params.update({"sort": "-modified-date", "body-format": "storage"})
        for record in _list(session, parts, path, params):
            flat = _flatten(record, content=content, comment_type=_COMMENT_TYPE.get(path) if decl.name == "comments" else None)
            if floor is not None and flat[CURSOR_FIELD] < floor:
                break  # everything after this is older: the list is sorted by this field
            yield flat
