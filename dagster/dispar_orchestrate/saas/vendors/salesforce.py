"""Salesforce (`SRC-3` V-Salesforce.2; decisions 4, 5, 6, 7, 9). The vendor module of
the `saas` adapter for `vendor = "salesforce"`; the code that talks to Salesforce is
the vendored dlt verified source (`saas/vendored/salesforce/`, AD4), read through the
`SaasSession` the adapter hands in.

# Objects (decision 5)

Every queryable sObject the org lists, standard and custom (`Foo__c`): the list is
the REST "describe global" call (`GET /services/data/vXX.X/sobjects`), so an org's
custom objects are offered without a list in code. An object that is not listed is
refused by name, and the name is checked to be a plain API name before it is used.

# Incremental or in full (decision 6)

`SystemModstamp` (Salesforce's system time of the last change, indexed, also moved by
system updates) is the cursor of every object that has the field: the object is read
with `WHERE SystemModstamp >= <last> ORDER BY SystemModstamp`, merged on `Id`. An
object without it is re-read in full, and the page says so. In the LIST the cursor is
taken from describe global's `replicateable` flag ("can be replicated through
getUpdated and getDeleted"), the objects that carry the system fields; the
declaration used for a load and a preview asks the object's own describe, which is
exact.

# Deletes (decision 7)

An object with `IsDeleted` is read through `queryAll`, which returns the records in the
recycle bin with `IsDeleted = true`, in the same read (vendored local change 3). The row
is marked, not removed. A record purged from the recycle bin is not reported. Objects
without `IsDeleted` (for example `User`) report no delete.

# Schema changes before the load (decision 9)

`describe` answers the object's fields from the object's own describe call, the list
the load selects (`queried_fields`), so "ask first" and "pause" can hold a table back.

# Sign-in (decision 3, decision 4)

Browser sign-in only: Salesforce's username + password + security-token sign-in (SOAP
`login()`) is retired in Summer '27 and is not offered. The returned `instance_url` is
adopted by the session through `SaasSession.adopt_origin` and the `.salesforce.com` /
`.force.com` rule (AD3); see `adopt_suffixes` and `adapters/saas.py`. The vendor table
gives Salesforce no integration credential.
"""

from __future__ import annotations

import re
from collections.abc import Iterator
from typing import Any

import requests

from dispar_orchestrate.adapters.saas import Field, SaasSetupRefused
from dispar_orchestrate.saas.objects import ObjectDecl
from dispar_orchestrate.saas.vendored.salesforce.helpers import records
from dispar_orchestrate.saas.vendored.salesforce.settings import API_VERSION

# AD3: the hosts the sign-in may return as `instance_url`.
adopt_suffixes: tuple[str, ...] = (".salesforce.com", ".force.com")

CURSOR_FIELD = "SystemModstamp"
_API_NAME = re.compile(r"^[A-Za-z][A-Za-z0-9_]*$")

# Salesforce field type -> the column type dlt gives the loaded value.
_TYPES = {
    "boolean": "bool",
    "int": "bigint",
    "long": "bigint",
    "double": "double",
    "currency": "double",
    "percent": "double",
    "datetime": "timestamp",
    "date": "date",
}


def _refuse() -> SaasSetupRefused:
    return SaasSetupRefused("Salesforce has no object by that name")


def _describe(session: Any, name: str) -> dict[str, Any]:
    """The object's describe result; an unknown or unreadable name is a rejected setting."""
    if not _API_NAME.match(name):
        raise _refuse()
    try:
        return records.describe_sobject(session, name)
    except requests.HTTPError as exc:
        if getattr(exc.response, "status_code", None) in (400, 404):
            raise _refuse() from None
        raise


def _decl(desc: dict[str, Any], name: str) -> ObjectDecl:
    names = {f["name"] for f in desc["fields"]}
    if CURSOR_FIELD in names:
        return ObjectDecl(name=name, primary_key=("Id",), cursor=CURSOR_FIELD, deletes="IsDeleted" in names)
    return ObjectDecl(name=name, primary_key=("Id",), full_reason=f"Salesforce keeps no {CURSOR_FIELD} on {name}")


def declaration(session: Any, parts: Any, dial: dict, name: str) -> ObjectDecl:
    """The object's declaration from its own describe: cursor, key, deletes, and why it
    is re-read in full."""
    return _decl(_describe(session, name), name)


def list_objects(session: Any, parts: Any, dial: dict) -> list[ObjectDecl]:
    listed = session.get_json(f"/services/data/v{API_VERSION}/sobjects")["sobjects"]
    out = []
    for entry in listed:
        name = entry.get("name", "")
        if not entry.get("queryable") or entry.get("deprecatedAndHidden") or not _API_NAME.match(name):
            continue
        if entry.get("replicateable"):
            out.append(ObjectDecl(name=name, primary_key=("Id",), cursor=CURSOR_FIELD))
        else:
            out.append(ObjectDecl(name=name, primary_key=("Id",), full_reason=f"Salesforce keeps no {CURSOR_FIELD} on {name}"))
    return out


def describe(session: Any, parts: Any, dial: dict, name: str) -> list[Field] | None:
    """The fields a load of the object selects, before the load (decision 9)."""
    fields = []
    for f in records.queried_fields(_describe(session, name)):
        fields.append(Field(f["name"], _TYPES.get(f["type"], "text"), nullable=f["name"] != "Id"))
    return fields


def read(session: Any, parts: Any, dial: dict, decl: ObjectDecl, *, since: Any) -> Iterator[dict[str, Any]]:
    """The records of an object. `since` is the cursor's last value, or `None` for the
    first run, which reads everything (the recycle bin too, for an object that reports
    deletes, so the table is complete)."""
    yield from records.get_records(
        session,
        decl.name,
        None if since is None else str(since),
        decl.cursor,
        include_deleted=decl.deletes,
    )
