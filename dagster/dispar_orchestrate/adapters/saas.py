"""dagster/dispar_orchestrate/adapters/saas.py -- the `saas` adapter: one
adapter for the ten business-app vendors (`SRC-3` AD1), the vendor chosen by
`dial.vendor`.

This module is the part that is the same for every vendor: it builds the
vendor's address from the table and `dial.account` (AD2), signs in (a browser
sign-in's refresh token, or the vendor's integration credential), opens a
`SaasSession` (AD3: the link rule, the address check before any credential,
rate limits), asks the vendor's module what an object looks like before the
load (AD7), and turns the module's records into a dlt resource that the sink
loads (AD6: incremental by the object's declared cursor, merge on its key).

# What a vendor module provides

A module registers itself with `saas.vendors.register(vendor_id, module)` and
has these members (a `typing.Protocol` is not used: the modules are plain
modules, and the tests' fake vendor is a small class):

    adopt_suffixes: tuple[str, ...]          optional; hosts a sign-in may return
    list_objects(session, parts, dial)       -> list[ObjectDecl]; the vendor's objects
    declaration(session, parts, dial, name)  -> ObjectDecl for one object
    read(session, parts, dial, decl, since)  -> iterator of dict records; `since` is
                                                the cursor's last value, or None
    describe(session, parts, dial, name)     optional -> list[Field] or None; a vendor that
                                                describes its fields (decision 9); None for
                                                an object it cannot describe
    integration_headers(vendor, secrets)     optional -> dict; an integration
                                                credential that is not a single token

The module never builds a host: it asks the session for a path. It never sees
a secret it was not given in `secrets`, and every record it yields is data,
never an address to dial.

# The invariant the session keeps for the module

`SaasSession.get_json` / `post_json` are the only HTTP calls a module makes. A vendor module
that imports `requests` itself would go round the link rule; the SRC-9-style
lint of slice 7 is the place to forbid that (plan section 8).
"""

from __future__ import annotations

import logging
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from typing import Any

import dlt

from dispar_orchestrate import schema_observer, ssrf_guard
from dispar_orchestrate.adapters import rest as rest_adapter
from dispar_orchestrate.adapters.sink import INCREMENTAL_MODES, UnsupportedLoadMode
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.objects import ObjectDecl
from dispar_orchestrate.saas.session import SaasSession

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class Field:
    """One field of a vendor object as the vendor describes it (AD7)."""

    name: str
    type_name: str
    nullable: bool = True


@dataclass(frozen=True)
class AdapterBuildResult:
    """What `ingest_factory` loads. `resolved` is always `None`: the session
    resolves and pins every host itself, request by request, so the caller has
    nothing to wrap (the same "already guarded" shape `sheets` and `mongodb`
    use)."""

    source: Any
    resolved: ssrf_guard.ResolvedAddress | None = None
    # True when the vendor's fields were shown to the API before the load, so
    # the factory does not report them again afterwards.
    observed_before_load: bool = False


class SaasSetupRefused(Exception):
    """The dial or the credentials cannot be used for the vendor (a sign-in
    kind the vendor lacks, an integration credential this build cannot present).
    Classified `setting_rejected`; the message is fixed text."""


def _parts(vendor: vendors.Vendor, dial: dict) -> vendors.AccountParts:
    try:
        return vendor.validate_account(str(dial.get("account", "")))
    except vendors.InvalidAccount:
        raise SaasSetupRefused("the connector's account is not valid for its vendor") from None


def _auth(
    vendor: vendors.Vendor,
    parts: vendors.AccountParts,
    module: Any,
    dial: dict,
    secrets: dict[str, str],
    *,
    connector_id: str | None,
    resolve_checked: Callable[..., ssrf_guard.ResolvedAddress],
    report_refresh_token: Callable[[str], None] | None,
) -> tuple[Callable[[], dict], Callable[[], None] | None, Callable[[], str | None]]:
    """The headers provider of the connector's sign-in, the function that
    forgets the access token (for a 401), `None` when there is no token, and the
    function that answers the `instance_url` the last token answer carried (only a
    browser sign-in has one; `None` otherwise)."""
    auth = dial.get("auth") or {}
    kind = auth.get("type")
    if kind == vendors.OAUTH_AUTH_TYPE:
        if report_refresh_token is None and connector_id:
            report_refresh_token = rest_adapter.report_refresh_token_to_api(connector_id)
        session = rest_adapter.AuthSession(
            {
                "type": kind,
                "tokenUrl": vendors.rebase_for_gate(vendor.id, vendor.token_url(parts)),
                "clientId": auth["clientId"],
                "clientAuth": vendor.oauth["clientAuth"],
            },
            secrets,
            resolve_checked=resolve_checked,
            report_refresh_token=report_refresh_token,
        )
        return session.headers, session.refresh, lambda: session.instance_url
    if kind == "integration":
        if vendor.integration is None:
            raise SaasSetupRefused("this vendor does not offer an integration credential")
        build = getattr(module, "integration_headers", None)
        if build is not None:
            headers = build(vendor, secrets)
            return (lambda: headers), None, lambda: None
        if tuple(vendor.integration["secretFields"]) == ("token",):
            headers = {"Authorization": f"Bearer {secrets['token']}"}
            return (lambda: headers), None, lambda: None
        raise SaasSetupRefused("this build cannot present this vendor's integration credential yet")
    raise SaasSetupRefused("the connector's sign-in kind is not one this vendor offers")


def open_session(
    dial: dict,
    secrets: dict[str, str],
    *,
    connector_id: str | None = None,
    resolve_checked: Callable[..., ssrf_guard.ResolvedAddress] = ssrf_guard.resolve_checked,
    http_get: Callable[..., Any] | None = None,
    http_post: Callable[..., Any] | None = None,
    report_refresh_token: Callable[[str], None] | None = None,
    sleep: Callable[[float], None] | None = None,
) -> tuple[Any, vendors.Vendor, vendors.AccountParts, SaasSession]:
    """The vendor module, the vendor, the account parts and a session for a dial.

    # Errors

    `vendors.UnsupportedVendor` when the vendor has no registered module;
    `SaasSetupRefused` for an account or sign-in the vendor does not accept;
    `rest.OAuthSignInRefused` / `ssrf_guard.SsrfBlocked` when the sign-in's
    first token cannot be had.
    """
    vendor = vendors.vendor(str(dial.get("vendor", "")))
    module = vendors.module_for(vendor.id)
    parts = _parts(vendor, dial)
    headers, refresh, instance_url = _auth(
        vendor,
        parts,
        module,
        dial,
        secrets,
        connector_id=connector_id,
        resolve_checked=resolve_checked,
        report_refresh_token=report_refresh_token,
    )
    extra: dict[str, Any] = {}
    if sleep is not None:
        extra["sleep"] = sleep
    session = SaasSession(
        vendors.rebase_for_gate(vendor.id, vendor.api_url(parts)),
        headers,
        adopt_suffixes=tuple(getattr(module, "adopt_suffixes", ())),
        http_get=http_get,
        http_post=http_post,
        refresh_auth=refresh,
        resolve_checked=resolve_checked,
        **extra,
    )
    if getattr(module, "adopt_suffixes", ()) and dial.get("auth", {}).get("type") == vendors.OAUTH_AUTH_TYPE:
        # AD3, decision 8: a vendor whose sign-in returns the host to use (Salesforce)
        # gets it adopted here, through `adopt_origin` and the vendor's suffix rule
        # only, BEFORE the first read. The sign-in's first token is asked now (so a
        # refused sign-in surfaces at open, for these vendors only); the address of the
        # token request is the table's, and was checked like every host.
        headers()
        returned = instance_url()
        if returned:
            session.adopt_origin(returned, as_origin=vendors.rebase_for_gate(vendor.id, returned))
    return module, vendor, parts, session


def _object_name(obj: dict) -> str:
    return str(obj["name"])


def _shaper(columns: list[str] | None, excluded: frozenset[str]) -> Callable[[dict], dict]:
    """The function that leaves out the columns the decision or the object's
    `excludedColumns` names. `columns` (from the API's decision, which has
    already applied the exclusion) wins when the vendor describes its fields;
    otherwise the exclusion applies to each record."""
    if columns is not None:
        keep = set(columns)
        return lambda record: {k: v for k, v in record.items() if k in keep}
    if excluded:
        return lambda record: {k: v for k, v in record.items() if k not in excluded}
    return lambda record: record


def build_source(
    dial: dict,
    secrets: dict[str, str],
    source_objects: list[dict],
    *,
    connector_id: str | None = None,
    gate: Callable[[schema_observer.ReflectedTable], schema_observer.Decision] | None = None,
    resolve_checked: Callable[..., ssrf_guard.ResolvedAddress] = ssrf_guard.resolve_checked,
    http_get: Callable[..., Any] | None = None,
    http_post: Callable[..., Any] | None = None,
    report_refresh_token: Callable[[str], None] | None = None,
    sleep: Callable[[float], None] | None = None,
) -> AdapterBuildResult | None:
    """The source of ONE object (`source_objects` holds exactly it, as for the
    other adapters). Returns `None` when the API said `wait` on the schema
    change found before the load (AD7).

    # Errors

    `vendors.UnsupportedVendor`, `SaasSetupRefused`, `UnsupportedLoadMode`
    (an incremental load of an object whose module declares no such cursor),
    the sign-in's failures, and whatever the vendor module and the session raise.
    """
    obj = source_objects[0]
    module, vendor, parts, session = open_session(
        dial,
        secrets,
        connector_id=connector_id,
        resolve_checked=resolve_checked,
        http_get=http_get,
        http_post=http_post,
        report_refresh_token=report_refresh_token,
        sleep=sleep,
    )
    name = _object_name(obj)
    decl: ObjectDecl = module.declaration(session, parts, dial, name)

    mode = obj.get("loadMode")
    incremental = mode in INCREMENTAL_MODES
    if incremental:
        wanted = str(obj.get("incrementalKey") or "").strip()
        if decl.cursor is None or wanted != decl.cursor:
            raise UnsupportedLoadMode(
                f"load mode {mode!r} is not available for {name!r}: the vendor cannot filter it by that field"
            )

    columns: list[str] | None = None
    observed_before = False
    describe = getattr(module, "describe", None)
    # AD7, decision 9: the fields are shown to the API BEFORE the load, so "ask
    # first" and "pause" can hold the table back (the same gate the SQL sources
    # use). `loaded_name` is the name itself: a vendor field is already the name
    # the Bronze table will have. A vendor that describes some objects only
    # (HubSpot: no properties API for owners) answers `None` for the others,
    # which then take the after-load path.
    fields: list[Field] | None = describe(session, parts, dial, name) if describe is not None and gate is not None else None
    if fields is not None and gate is not None:
        reflected = schema_observer.ReflectedTable(
            columns=tuple(schema_observer.ReflectedColumn(f.name, f.type_name, f.nullable, loaded_name=f.name) for f in fields),
            primary_key=tuple(decl.primary_key),
        )
        decision = gate(reflected)
        if decision.action == "wait":
            return None
        columns = decision.columns
        observed_before = True
    excluded = frozenset(n for n in (obj.get("excludedColumns") or []) if isinstance(n, str))
    shape = _shaper(columns, excluded)

    def _records(since: Any) -> Iterator[dict]:
        for record in module.read(session, parts, dial, decl, since=since):
            yield shape(record)

    resource_name = str(obj.get("target") or name)
    if incremental:

        @dlt.resource(name=resource_name)
        def rows(cursor: dlt.sources.incremental = dlt.sources.incremental(decl.cursor, lag=decl.lag_seconds or None)):  # noqa: B008 -- dlt's own pattern
            yield from _records(cursor.last_value)

        # `sink._apply_cursor` re-applies the cursor and reads this to keep the lag.
        rows.cursor_lag_seconds = decl.lag_seconds or None
        return AdapterBuildResult(source=rows, observed_before_load=observed_before)

    def plain() -> Iterator[dict]:
        yield from _records(None)

    return AdapterBuildResult(source=plain(), observed_before_load=observed_before)
