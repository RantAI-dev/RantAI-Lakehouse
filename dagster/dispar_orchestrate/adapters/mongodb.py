"""dagster/dispar_orchestrate/adapters/mongodb.py -- wraps `pymongo`
directly, since dlt ships no mongodb source in the installed package
(WS9 plan Correction 2). `directConnection=True` is enforced
(ssrf_guard_mongo.validate_mongo_dial) before any network call, then
EVERY explicit seed host is resolve_checked before MongoClient is
constructed -- see ssrf_guard_mongo.py's module doc comment for why
replica-set discovery is refused outright rather than checked live.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterator

from pymongo import MongoClient

from dispar_orchestrate import ssrf_guard
from dispar_orchestrate.ssrf_guard_mongo import resolve_all_seed_hosts, validate_mongo_dial


@dataclass(frozen=True)
class AdapterBuildResult:
    sources: dict[str, Iterator[dict]]
    resolved: list[ssrf_guard.ResolvedAddress]


def _collection_rows(collection) -> Iterator[dict]:
    yield from collection.find({})


def build_source(
    spec: dict,
    secrets: dict[str, str],
    source_objects: list[dict],
    *,
    resolve_checked=ssrf_guard.resolve_checked,
) -> AdapterBuildResult:
    """Build one dlt-shaped source per `source_objects` entry over a
    Mongo collection. `validate_mongo_dial` (SRV-shape/directConnection
    refusal) and `resolve_all_seed_hosts` (every explicit seed host
    checked) both run BEFORE `MongoClient` is constructed -- a rejected
    or blocked dial never reaches the driver at all (see this module's
    two "before connecting" tests).
    """
    validate_mongo_dial(spec)
    resolved = resolve_all_seed_hosts(spec["hosts"], resolve_checked=resolve_checked)
    # WS3 Task F2 (Z13) teaches the general lesson this call applies:
    # `username`/`password`/`authSource` are handed to MongoClient as
    # STRUCTURED keyword parameters, never concatenated into a
    # "mongodb://user:pass@host/db"-shaped URI string -- pymongo builds
    # its own internal MongoCredential object from these fields directly,
    # so there is no delimiter-separated text for a `;`/`@`/`/` in a
    # username or password to break out of. This is the SAME choice
    # Task F2's `sql.py` makes for `mysql`/`postgresql` (a Python dict of
    # credentials, not an interpolated DSN) -- `_odbc_quote`
    # (`sql.py`'s `mssql` arm) exists ONLY because ODBC's raw connection
    # string has no structured-parameter escape hatch; Mongo's driver
    # does, so no quoting function is needed here.
    client: Any = MongoClient(
        host=spec["hosts"],
        username=spec["username"],
        password=secrets["password"],
        authSource=spec["database"],
        directConnection=True,
    )
    db = client[spec["database"]]
    sources = {obj["target"]: _collection_rows(db[obj["name"]]) for obj in source_objects}
    return AdapterBuildResult(sources=sources, resolved=resolved)
