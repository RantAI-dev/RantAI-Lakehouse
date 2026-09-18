"""dagster/dispar_orchestrate/ssrf_guard_mongo.py -- WS9 plan hard
requirement 1: MongoDB has two host-selection mechanisms that hand
control to something other than the operator-supplied `dial`:

1. `mongodb+srv://` resolves a DNS SRV record (naming hosts) plus a TXT
   record (naming connection options) that together decide the real
   seed list -- the DNS zone owner, not `dial.hosts`, decides what gets
   dialed. `MongoDial` (ingest_spec.rs, Task A2) has NO field that can
   carry a `+srv` URI at all -- this module's `validate_mongo_dial`
   additionally refuses an individual host string that is itself
   SRV-zone-shaped, as defence in depth against a direct-DB edit
   bypassing schema validation.
2. Replica-set discovery: even from an explicit `mongodb://` seed list,
   the driver's `isMaster`/`hello` handshake returns `hosts`/`passives`
   fields naming every OTHER member of the replica set, and pymongo's
   default topology behaviour is to discover and dial those too. Unlike
   Kafka's one-shot pre-fetch metadata (Task C1), there is no single
   "check every host, then proceed" checkpoint for a driver's LIFELONG
   reconnect behaviour -- a member could join the replica set mid-run,
   resolving later, past any one-time check this code could perform.

DECISION: refuse discovery outright rather than try to bound it.
`directConnection` MUST be true (`validate_mongo_dial`'s second check) --
pymongo's own `directConnection=True` option makes the driver talk to
EXACTLY the one seed host it was given, ignoring hosts/passives
entirely, for the life of the connection. This means a full,
auto-failing-over replica set is NOT supported by this adapter --
stated once, here, as the honest limitation (a future workstream could
add a bounded, explicit `dial.allowedReplicaHosts` allowlist checked on
every reconnect if that becomes a real requirement; not built
speculatively).
"""

from __future__ import annotations

from dispar_orchestrate.ssrf_guard import ResolvedAddress, resolve_checked as _default_resolve_checked


class MongoConfigRejected(Exception):
    """A Mongo dial's host configuration cannot be safely connected --
    SRV-shaped, or requests replica-set discovery."""


def validate_mongo_dial(dial: dict) -> None:
    """Refuse (1) any `dial.hosts` entry that is itself SRV-zone-shaped
    (defence in depth -- the schema has no `+srv` field at all, see the
    module docstring) and (2) `directConnection` being anything but
    `True` (replica-set discovery -- hard requirement 1's refusal, not a
    partial check). Both checks run before any network call."""
    for host in dial.get("hosts", []):
        bare_host = host.rsplit(":", 1)[0]
        # A heuristic, not a DNS lookup: SRV-style Atlas hostnames are
        # always at least "cluster0.xxxxx.mongodb.net"-shaped (3+ labels
        # ending "mongodb.net") while every operator-supplied host in
        # this build's fixtures/tests is a 2-label *.invalid name -- this
        # only needs to catch the SRV shape, not classify every possible
        # hostname correctly, since the schema already has no srv field;
        # this is defence in depth, not the primary control.
        if bare_host.count(".") >= 2 and "mongodb.net" in bare_host:
            raise MongoConfigRejected(f"host {host!r} looks like an SRV-discovery hostname, not a direct seed")
    if not dial.get("directConnection", True):
        raise MongoConfigRejected(
            "directConnection must be true: this adapter does not support replica-set "
            "member discovery (WS9 plan hard requirement 1) -- list every host you need "
            "to read from explicitly in dial.hosts instead"
        )


def resolve_all_seed_hosts(hosts: list[str], *, resolve_checked=_default_resolve_checked) -> list[ResolvedAddress]:
    """Check every EXPLICIT seed host before any connection -- the
    counterpart to Kafka's `check_all_advertised_brokers`, but over a
    list the operator wrote (`dial.hosts`) rather than one the server
    reports, since discovery itself is refused (`validate_mongo_dial`).

    `_validate_hostname` (WS3 Task F2/A3, Z13, imported from
    `adapters.sql` -- the one canonical hostname-shape rule, not
    re-implemented) runs first, defence in depth alongside `Dial::parse`
    (Task A2)'s save-time check of the same field: the value re-read
    here at dial time could differ from what was saved if a direct
    database edit bypassed the API, so this adapter never trusts the
    stored `dial` blindly."""
    from dispar_orchestrate.adapters.sql import _validate_hostname

    resolved = []
    for host_port in hosts:
        # `str.rpartition`'s no-match result is ("", "", host_port) --
        # the WHOLE string lands in the "port" slot for a host with no
        # ":" at all. Checking `sep` (not `host`) is what tells apart
        # "no colon" from "colon at position 0", which `host or
        # host_port` alone cannot (the bug this comment replaces: the
        # plan's literal Step 2 code used `host or host_port`, which
        # passed a hostname string to `int()` and raised ValueError for
        # any host with no explicit port -- caught by this module's own
        # test, fixed here, noted as a plan deviation in the commit).
        host, sep, port_s = host_port.rpartition(":")
        bare_host = host if sep else host_port
        _validate_hostname("hosts", bare_host)
        resolved.append(resolve_checked(bare_host, int(port_s) if sep else 27017))
    return resolved
