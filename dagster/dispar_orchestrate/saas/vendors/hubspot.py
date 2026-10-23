"""HubSpot (`SRC-3` V-HubSpot.2; decisions 5, 6, 7, 9). The vendor module of the
`saas` adapter for `vendor = "hubspot"`; the code that talks to HubSpot's CRM is
the vendored dlt verified source (`saas/vendored/hubspot/`, AD4), read through
the `SaasSession` the adapter hands in.

# Objects (decision 5)

The CRM objects companies, contacts, deals, tickets, products and quotes, the
owners, and the deal and ticket pipelines. HubSpot has a fixed set of these, so
the list is static; what varies per account is each object's properties, which
the properties API describes (`describe`, decision 9: the schema is shown
before the load).

# Incremental or in full (decision 6)

The six CRM objects load incrementally: the first run lists everything, every
later run asks the CRM search API for records modified since the last cursor
(vendored local change 2). Owners and pipelines have no change filter and are
re-read in full, which the connector page says.

# Deletes (decision 7)

None are reported. HubSpot's search does not return archived records
(https://developers.hubspot.com/docs/api/crm/search), and the archived listing
(`archived=true`) is a separate full read, not part of the same read. So a
record archived at HubSpot stays in Bronze until a full reload, and no object
here says `deletes`.

# Search lag

HubSpot says a newly updated record can take "a few moments" to appear in
search. The object declares `SEARCH_LAG_SECONDS` as its `lag_seconds` (final
review SHOULD-FIX 21), so dlt's incremental hands `read` a `since` that much
earlier and keeps the margin's rows: before that the module stepped back by
hand and dlt dropped them as below the cursor. The records read twice are
merged on `id`.
"""

from __future__ import annotations

from collections.abc import Iterator
from dataclasses import dataclass
from typing import Any

from dispar_orchestrate.adapters.saas import Field, SaasSetupRefused
from dispar_orchestrate.saas.objects import ObjectDecl
from dispar_orchestrate.saas.vendored import hubspot as hs_source
from dispar_orchestrate.saas.vendored.hubspot import helpers as hs_helpers
from dispar_orchestrate.saas.vendored.hubspot import settings as hs_settings
from dispar_orchestrate.saas.vendored.hubspot.utils import chunk_properties

SEARCH_LAG_SECONDS = 60
_OBJECTS_PATH = "/crm/v3/objects/{plural}"


@dataclass(frozen=True)
class _Kind:
    """How one object is read: the vendored source's object type (singular), or
    `None` for an object that is not a CRM object with properties."""

    path: str
    crm_type: str | None = None
    full_reason: str | None = None


_KINDS: dict[str, _Kind] = {
    **{
        plural: _Kind(_OBJECTS_PATH.format(plural=plural), crm_type=singular)
        for plural, singular in hs_settings.OBJECT_TYPE_SINGULAR.items()
    },
    "owners": _Kind(hs_settings.CRM_OWNERS_ENDPOINT, full_reason="HubSpot cannot filter owners by change date"),
    "pipelines_deals": _Kind(
        hs_settings.CRM_PIPELINES_ENDPOINT.format(objectType="deals"),
        full_reason="HubSpot cannot filter pipelines by change date",
    ),
    "pipelines_tickets": _Kind(
        hs_settings.CRM_PIPELINES_ENDPOINT.format(objectType="tickets"),
        full_reason="HubSpot cannot filter pipelines by change date",
    ),
}


def _kind(name: str) -> _Kind:
    found = _KINDS.get(name)
    if found is None:
        raise SaasSetupRefused("HubSpot has no object by that name")
    return found


def declaration(session: Any, parts: Any, dial: dict, name: str) -> ObjectDecl:
    """The object's declaration (cursor, key, why it is re-read in full). A CRM object also declares
    the search lag: HubSpot says a change can take "a few moments" to appear in search, so dlt hands
    `read` a `since` this much earlier (PR #216 final review SHOULD-FIX 21; before that the module
    stepped back by hand and dlt then dropped those rows as below the cursor)."""
    kind = _kind(name)
    if kind.crm_type is not None:
        return ObjectDecl(name=name, primary_key=("id",), cursor=hs_settings.LAST_MODIFIED_PROPERTY[kind.crm_type], lag_seconds=SEARCH_LAG_SECONDS)
    return ObjectDecl(name=name, primary_key=("id",), full_reason=kind.full_reason)


def list_objects(session: Any, parts: Any, dial: dict) -> list[ObjectDecl]:
    return [declaration(session, parts, dial, name) for name in _KINDS]


def _property_types(session: Any, kind: _Kind) -> dict[str, str | None]:
    """The properties the connector reads and their HubSpot types: the vendored
    source's default set plus the account's custom properties."""
    assert kind.crm_type is not None
    return hs_source.fetch_props_with_types(kind.crm_type, session, hs_settings.ENTITY_PROPERTIES[kind.crm_type], True)


def describe(session: Any, parts: Any, dial: dict, name: str) -> list[Field] | None:
    """The object's fields as the properties API lists them, before the load
    (decision 9). `None` for owners and pipelines, which have no properties API:
    their schema change is seen after the load, as for other API sources."""
    kind = _kind(name)
    if kind.crm_type is None:
        return None
    fields = [Field("id", "text", nullable=False)]
    for prop, hs_type in sorted(_property_types(session, kind).items()):
        if prop != "id":
            fields.append(Field(prop, hs_settings.HS_TO_DLT_TYPE.get(hs_type or "", "text")))
    return fields


def _coerce(record: dict[str, Any], types: dict[str, str | None]) -> dict[str, Any]:
    """HubSpot sends every property as text, an empty text for no value. Numbers
    become numbers and booleans booleans (`== "true"`), so Bronze columns are
    typed as the properties API says they are."""
    out: dict[str, Any] = {}
    for key, value in record.items():
        hs_type = types.get(key)
        if value == "":
            value = None
        elif hs_type == "number" and isinstance(value, str):
            try:
                value = float(value)
            except ValueError:
                value = None
        elif hs_type == "bool" and isinstance(value, str):
            value = value == "true"
        out[key] = value
    return out


def _full(path: str, session: Any, props: list[str], later: list[list[str]]) -> Iterator[list[dict[str, Any]]]:
    params = {"properties": ",".join(props), "limit": hs_settings.SEARCH_PAGE_LIMIT}
    for page in hs_helpers.fetch_data(path, session, params=params):
        yield _hydrate(path, session, page, later)


def _hydrate(path: str, session: Any, page: list[dict[str, Any]], later: list[list[str]]) -> list[dict[str, Any]]:
    for chunk in later:
        extra = hs_helpers._batch_read(path, session, [str(o["id"]) for o in page], chunk)
        for obj in page:
            obj.update(extra.get(str(obj["id"]), {}))
    return page


def read(session: Any, parts: Any, dial: dict, decl: ObjectDecl, *, since: Any) -> Iterator[dict[str, Any]]:
    """The records of an object. `since` is the cursor's last value (an ISO time
    as HubSpot wrote it) or `None` for the first run, which reads everything."""
    kind = _kind(decl.name)
    if kind.crm_type is None:
        for page in hs_helpers.fetch_data(kind.path, session):
            yield from page
        return
    assert decl.cursor is not None
    types = _property_types(session, kind)
    props = sorted(types)
    first, *later = list(chunk_properties(props, hs_settings.SEARCH_MAX_PROPS_LENGTH)) or [[]]
    if since is None:
        pages = _full(kind.path, session, first, later)
    else:
        # PR #216 final review SHOULD-FIX 21: `since` already carries the declared lag
        # (`ObjectDecl.lag_seconds`, which dlt's incremental subtracts), so stepping back here again
        # would read 120 s and, worse, dlt's cursor filter would drop every row in the margin:
        # the margin only exists if the resource's cursor knows about it.
        floor = hs_helpers._to_ms(since)
        pages = _search_or_full(kind, session, decl.cursor, floor, props, first, later)
    for page in pages:
        for record in page:
            yield _coerce(record, types)


def _search_or_full(
    kind: _Kind, session: Any, cursor: str, floor: int, props: list[str], first: list[str], later: list[list[str]]
) -> Iterator[list[dict[str, Any]]]:
    try:
        yield from hs_helpers.search_data_since(kind.path, session, cursor, floor, props)
    except hs_helpers.SearchOutOfBoundsException:
        # 10,000 records share one instant: the search cannot move past them.
        # Reading the object in full is slow but never wrong; the merge on `id`
        # removes the records the search already yielded.
        yield from _full(kind.path, session, first, later)
