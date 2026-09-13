"""P4 Dagster maintenance job: the in-engine ClickHouse Iceberg maintenance
chain, run per Bronze table, on a schedule.

# ClickHouse 26.8 rework — this module was rebuilt after the 26.3 -> 26.8
# bump changed which verbs work

Every fact this module used to document was measured on
`clickhouse/clickhouse-server:26.3`. The PR stack review pointed out that
the locked decision was 26.8 LTS all along, and a full re-measurement
(`docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md`) against the SAME stack
(Lakekeeper 0.13.3, RustFS, format-version 2, OpenFGA enforced) found the
working verb set had changed underneath this job:

| Verb | 26.3 | 26.8 |
| --- | --- | --- |
| `expire_snapshots` | the only verb that worked | **`Code: 48. ... not supported for Iceberg tables backed by a transactional catalog.`** — a deliberate restriction (a REST catalog owns snapshot expiry, not the query engine), not a bug |
| `remove_orphan_files` | did not exist (`Code: 36. Unknown EXECUTE command`) | **works** — `ALTER TABLE t EXECUTE remove_orphan_files(dry_run=1/0)`, gated by `allow_database_iceberg=1, allow_insert_into_iceberg=1, allow_iceberg_remove_orphan_files=1` (a setting that did not exist on 26.3 either) |
| `OPTIMIZE` | `Code: 499` HTTP 403 at runtime | **returns OK** — but measured directly against a 7-small-file partition: seven files go in, seven files come out. It does **not** bin-pack. Not a compaction remedy at any version tested. |

**This job now runs `remove_orphan_files` (dry-run, then applied) as its
one active in-engine verb**, replacing `expire_snapshots`. `expire_snapshots`
is attempted once per table anyway — specifically so the skip is a REAL,
freshly-observed error every run (`probe_expire_snapshots_skip`), not a
hardcoded string that could silently go stale if some future ClickHouse
version changes the behavior again — and its failure is logged loudly and
recorded in `skipped_verbs`, never silently dropped. `OPTIMIZE` is
deliberately NOT invoked (calling it would succeed and do nothing useful —
see the table above): that is a logged design skip, not a caught runtime
error, and the log message says so explicitly rather than pretending it is
the same kind of skip as `expire_snapshots`'s.

# Lakekeeper-side snapshot expiry — investigated, NOT wired in, and why

`expire_snapshots`'s disappearance on 26.8 is "a REST catalog should own
snapshot expiry" made concrete, so this fix investigated whether Lakekeeper
(pinned `quay.io/lakekeeper/catalog:v0.13.3` in `docker-compose.yml`)
actually exposes a maintenance/expiry call — not a guess: Lakekeeper's own
docs (`table-maintenance`, present since v0.10.0, well before the pinned
v0.13.3) document a real Management API surface for this:

```
GET  /management/v1/warehouse/{warehouse_id}/task-queue/expire_snapshots/config
POST /management/v1/warehouse/{warehouse_id}/task-queue/expire_snapshots/config
```

taking `enable-expire-snapshots` / `max-snapshot-age-ms` /
`min-snapshots-to-keep` / `min-snapshots-to-expire` / `max-ref-age-ms`.

**This module does NOT call it, for two concrete reasons, not laziness:**

1. **Wrong shape.** It is a warehouse-wide "turn on Lakekeeper's own
   background expiry task queue" TOGGLE — Lakekeeper then expires
   snapshots itself, asynchronously, on its own schedule after future
   commits. There is no documented per-table, per-run, dry-run-then-apply
   call that would slot into this job's existing `dry_run` → `apply` →
   `record_maintenance_run` shape the way `run_remove_orphan_files` does;
   forcing it into that shape would mean POSTing the same idempotent
   warehouse-wide config on every scheduled run, which is a materially
   different (and riskier) operation than "run this verb against this
   table and report what it did."
2. **Wrong principal.** Calling it needs `warehouse_id` (resolved via
   `GET /management/v1/warehouse`, an ADMIN-scoped list call — see
   `lakekeeper-authz-init` in `docker-compose.yml`, which is the ONLY
   place `admin.jwt` is mounted) plus, almost certainly, an `admin`-level
   grant on the warehouse to change its task-queue config. This job's only
   identity is `clickhouse-reader` (`select`+`modify` — granted for
   ClickHouse's OWN `expire_snapshots` attempt above, now failing), and
   `dagster-code-location` (where this op runs) mounts no admin token.
   Minting one for a recurring, unattended scheduled job — rather than the
   one-shot bootstrap job that currently holds it — is a real
   authorization-surface change this fix does not make unreviewed, and
   this repo has no measured proof (no live-stack run against v0.13.3)
   that `clickhouse-reader`'s existing grant would even be accepted by
   that endpoint if it somehow were used.

So: an actionable, cited path exists for a future fix (enable
`expire_snapshots` task-queue config once per warehouse, from a principal
with the right grant — mirroring how `lakekeeper-authz-init` already does
one-shot admin-scoped setup), but it is not built here. What IS built here,
per the brief's explicit fallback for exactly this situation ("add an
explicit, monitored gap"), is `measure_snapshot_growth` below: every run,
per Bronze table, it reads the table's OWN metadata document straight from
Lakekeeper's Iceberg REST catalog (`GET /v1/{prefix}/namespaces/{ns}/
tables/{table}` — the SAME endpoint `ops/g3a/g3a_test.py`'s
`step_verify_format_version_2` already uses and has proven works against
this stack) and records `len(metadata["snapshots"])` and
`len(metadata["metadata-log"])` through `record_maintenance_run`, so the
unbounded growth this section describes is VISIBLE and trending in
`GET /api/governance/maintenance` history — not silent — even though
nothing in this stack reclaims it yet. Orphan *files* are still reclaimed
by `remove_orphan_files` above; orphan *snapshots* are not reclaimed by
anything in this stack, and this module now says so with numbers attached,
every run.

# G3's consequence for this job is unchanged

`remove_orphan_files` reclaims files no manifest references — not
accumulated small data files that ARE referenced and simply never get
bin-packed. `OPTIMIZE` is the only verb that could compact, and it still
does not on 26.8 (measured directly, see the table above). So, exactly as
on 26.3, this job provides ZERO small-file mitigation;
`docs/plans/G3-RESULT.md` and `ops/g3/g3_loadgen.py` are what measure that
gap, and the Trino escape hatch (`docker-compose.yml`'s
`trino-maintenance-cron` service, ADR 0009) is what actually keeps Bronze
query-planning healthy. This job's job stays narrower: orphan-file and
(once expire_snapshots moves to the catalog) snapshot hygiene only.

# Bronze-only scoping

`discover_bronze_tables` filters `SHOW TABLES FROM {CATALOG_DB}` to names
starting with `bronze.` — the catalog's flat namespace also holds
`gold.*` (ADR 0010's Gold export target, a completely different job's
data) in the SAME flat `SHOW TABLES` listing, and prior to this fix
nothing filtered it out: applied maintenance ran against every namespace
the catalog knew about, `gold.*` included. See
`dagster/dispar_orchestrate/test_maintenance.py` for the regression test
proving a `gold.*` table is excluded.

# Per-table policy + Trino verbs

The verbs above are the job's fixed, unconditional behavior. Operators
who want to actually reclaim expired snapshots or bin-pack small files
need `table_maintenance_policy` (`rust/migrations/0030`), read once per
run from `GET /api/lakehouse/maintenance-policies` and routed through
Trino's Iceberg connector — which does not carry ClickHouse's
"transactional catalog" restriction on `expire_snapshots`, and whose
`optimize` genuinely bin-packs (ClickHouse's does not — see the table
above).

* **`snapshotsToKeep`.** Trino 483's `expire_snapshots` procedure takes
  only a `retention_threshold` age, not a keep-count, so keeping the N
  newest snapshots means computing an age: read every snapshot's
  `committed_at` (newest first) from Trino's `"<table>$snapshots"`
  metadata table, and if there are more than N, the threshold is the AGE
  of the Nth-newest snapshot, in whole seconds rounded up. Exactly N
  survive. A table with N or fewer snapshots has nothing to expire and
  the verb is skipped, not run with a threshold of zero — a `'0d'`
  threshold (the naive reading of "expire snapshots older than
  everything") would expire every snapshot but the current one,
  regardless of how many the operator asked to keep, which is real data
  loss for anyone who configured a keep-count greater than one. Trino's
  `iceberg.expire-snapshots.min-retention` (a 7-day default, unset in
  this stack's compose) can also refuse a threshold shorter than that
  floor; the refusal is recorded honestly and never retried with a
  larger value — lowering the floor is a deployment/product decision,
  not something this job decides for an operator.
* **`orphanAgeHours`.** Nothing in this repo proves
  `remove_orphan_files` accepts an age argument on this ClickHouse
  version, and it is called above with only `dry_run=1/0`. Setting this
  field is recorded, honestly, as "stored; not applied by this build" —
  never silently ignored, and never applied on a guess.
* **`schedule`.** A per-table cadence for the Trino verbs only (the
  ClickHouse-side steps above stay unconditional, every run): `weekly`
  requires the table's last recorded verb run to be at least 7 days old,
  `daily` at least 20 hours (tolerating this job's own start jitter — see
  `bronze_maintenance_schedule` below), `null` runs every time.
* **`compactSmallFiles`.** Runs Trino's `ALTER TABLE ... EXECUTE
  optimize` — the same call `ops/trino/optimize_bronze.sh` already
  proves works against this stack's Iceberg tables.

`_trino_execute` below is a deliberate, small duplication of
`rust/crates/lakehouse-trino`'s own `/v1/statement` polling loop: this
job runs inside `dagster-code-location`, a separate Python process
outside the Rust binary that crate is built into, so there is no way to
share that implementation from here. Every statement is sent as
`X-Trino-User: trino-maintenance` — Trino's file-based access control
(`docker-compose.yml`'s `trino` service) grants that user `all` on the
`iceberg` catalog and every other user only `read-only`.

Per-verb outcomes (`applied`/`refused`/`skipped`/`failed`) are recorded
in a NEW table, `bronze_meta.maintenance_verb_run`
(`bronze_catalog.record_maintenance_verb_run`), rather than as new
columns on the existing `bronze_meta.maintenance_run` table: that
table's schema (`bronze_catalog._MAINTENANCE_RUN_SCHEMA`) is guarded by
an R10 schema-drift check with no additive path (`_assert_or_create_schema`
either creates a table fresh or requires an EXISTING one's columns to
match exactly), so widening it would raise `SchemaDriftError` on every
deployment that already has this table — the demo included — on the
very next scheduled run, stopping maintenance entirely. A brand-new
table has no such deployment to break.
"""

from __future__ import annotations

import math
import os
import time
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Any

import requests
from dagster import DefaultScheduleStatus, Definitions, Failure, ScheduleDefinition, job, op

from dispar_orchestrate.bronze_catalog import (
    ClickHouseTarget,
    latest_maintenance_run_at,
    record_maintenance_run,
    record_maintenance_verb_run,
)

CATALOG_DB = "icecat_maintenance"


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


@dataclass(frozen=True)
class MaintenanceConfig:
    ch: ClickHouseTarget
    lakekeeper_catalog_uri: str
    lakekeeper_warehouse: str
    rustfs_endpoint: str
    # R1 (ADR 0011): `expire_snapshots` is a genuine catalog metadata
    # write ClickHouse performs on this stack's behalf, authenticated as
    # `clickhouse-reader` (granted `modify` for exactly this — see
    # `docker-compose.yml`'s `lakekeeper-authz-init`). Empty on a pre-R1
    # or authz-disabled stack.
    ch_oauth_client_id: str
    ch_oauth_server_uri: str
    # Per-table policy + Trino verbs (module doc, "Per-table policy +
    # Trino verbs"). Defaulted so every existing positional/keyword
    # construction of this dataclass elsewhere (this module's own test
    # module included) keeps working unchanged.
    api_url: str = "http://lakehouse-api:8080"
    maintenance_token: str = ""
    trino_url: str = "http://trino:8080"

    @classmethod
    def from_env(cls) -> "MaintenanceConfig":
        return cls(
            ch=ClickHouseTarget.from_env(),
            lakekeeper_catalog_uri=_env(
                "LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog"
            ),
            lakekeeper_warehouse=_env("LAKEKEEPER_WAREHOUSE", "default"),
            rustfs_endpoint=_env("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000"),
            ch_oauth_client_id=_env("CH_OAUTH_CLIENT_ID", ""),
            ch_oauth_server_uri=_env("CH_OAUTH_SERVER_URI", ""),
            api_url=_env("LAKEHOUSE_API_URL", "http://lakehouse-api:8080"),
            maintenance_token=_env("LAKEHOUSE_MAINTENANCE_TOKEN", ""),
            trino_url=_env("TRINO_URL", "http://trino:8080"),
        )

    def ch_auth_settings(self) -> str:
        if not self.ch_oauth_client_id:
            return ""
        return (
            f", catalog_credential = '{self.ch_oauth_client_id}:unused', "
            f"oauth_server_uri = '{self.ch_oauth_server_uri}'"
        )


def _ch_query(cfg: MaintenanceConfig, sql: str) -> str:
    resp = requests.post(
        cfg.ch.url, auth=(cfg.ch.user, cfg.ch.password), data=sql.encode("utf-8"), timeout=60
    )
    resp.raise_for_status()
    return resp.text


def _ensure_catalog_database(cfg: MaintenanceConfig) -> None:
    """A dedicated `DataLakeCatalog` database for maintenance, separate
    from any per-test catalog database (`icecat`/`icecat_g3a` in
    `ops/g3a/g3a_test.py`) so a maintenance run never depends on test
    fixtures having created one first."""
    _ch_query(
        cfg,
        f"CREATE DATABASE IF NOT EXISTS {CATALOG_DB} "
        f"ENGINE = DataLakeCatalog('{cfg.lakekeeper_catalog_uri}') "
        f"SETTINGS catalog_type = 'rest', warehouse = '{cfg.lakekeeper_warehouse}', "
        f"storage_endpoint = '{cfg.rustfs_endpoint}'{cfg.ch_auth_settings()} "
        "SETTINGS allow_database_iceberg = 1",
    )


def discover_bronze_tables(cfg: MaintenanceConfig) -> list[str]:
    """Every table under the catalog's flat `bronze` namespace (ADR 0004) --
    discovered from the catalog itself (`SHOW TABLES`), not from
    `bronze_meta.dataset_catalog`, so a table that failed catalog
    registration but still exists in Lakekeeper is still maintained (the
    two failure modes are independent and this job should not silently
    skip a table just because the OTHER P3 step didn't run for it).

    `SHOW TABLES FROM {CATALOG_DB}` returns every namespace the catalog
    knows about in one flat list -- `bronze.*` AND `gold.*` (ADR 0010's
    Gold export target) side by side, since `DataLakeCatalog` has no
    per-namespace listing call, only a whole-database one. PR #30 review
    blocker: this used to return that whole list unfiltered, so applied
    maintenance ran against `gold.*` too. The `bronze.` prefix filter
    below is the fix; `test_maintenance.py::DiscoverBronzeTablesTest`
    is the regression test proving a `gold.*` table is excluded."""
    text = _ch_query(
        cfg, f"SHOW TABLES FROM {CATALOG_DB} SETTINGS allow_database_iceberg=1 FORMAT TabSeparated"
    )
    all_tables = [line.strip() for line in text.splitlines() if line.strip()]
    return [name for name in all_tables if name.startswith("bronze.")]


_ICEBERG_MAINT_SETTINGS = "allow_database_iceberg=1, allow_insert_into_iceberg=1"

_REMOVE_ORPHAN_FILES_SETTINGS = f"{_ICEBERG_MAINT_SETTINGS}, allow_iceberg_remove_orphan_files=1"

_EXPIRE_SNAPSHOTS_SETTINGS = f"{_ICEBERG_MAINT_SETTINGS}, allow_experimental_expire_snapshots=1"

# `remove_orphan_files`'s result-set column names, verified empirically
# against a live ClickHouse 26.8 + Lakekeeper stack (see the module doc).
# A superset of `expire_snapshots`'s old columns (adds
# `deleted_metadata_files_count`/`skipped_missing_metadata_count`/
# `failed_deletions_count`) — `bronze_catalog.record_maintenance_run` only
# persists the three columns it always has (data/manifest files, manifest
# lists), so the extra ones are surfaced in the op's own return value/logs
# without needing a registry schema change.
_ROF_COUNT_COLUMNS = (
    "deleted_data_files_count",
    "deleted_position_delete_files_count",
    "deleted_equality_delete_files_count",
    "deleted_manifest_files_count",
    "deleted_manifest_lists_count",
    "deleted_metadata_files_count",
    "deleted_statistics_files_count",
    "skipped_missing_metadata_count",
    "failed_deletions_count",
)


def _parse_tsv_kv(text: str) -> dict[str, int]:
    """`ALTER TABLE ... EXECUTE <verb>()` returns a `key\\tvalue`
    two-column result set (verified empirically — see module doc), not the
    usual query row shape. `FORMAT TabSeparated` on an ALTER EXECUTE
    statement returns exactly this, for both `remove_orphan_files` and the
    old `expire_snapshots`."""
    counts: dict[str, int] = {}
    for line in text.splitlines():
        if not line.strip():
            continue
        key, _, value = line.partition("\t")
        try:
            counts[key] = int(value)
        except ValueError:
            counts[key] = 0
    return counts


def run_remove_orphan_files(
    cfg: MaintenanceConfig, table_name: str, *, dry_run: bool
) -> dict[str, Any]:
    """Run (or dry-run) `remove_orphan_files` against one catalog-registered
    Bronze table — the working verb on ClickHouse 26.8 (did not exist on
    26.3). `table_name` is the catalog's own two-part name, e.g.
    `bronze.g3a_orders` (already backtick-safe: no user input reaches this,
    only names `discover_bronze_tables` returned, which is itself filtered
    to the `bronze.` prefix)."""
    dry_run_arg = "dry_run=1" if dry_run else "dry_run=0"
    sql = (
        f"ALTER TABLE {CATALOG_DB}.`{table_name}` "
        f"EXECUTE remove_orphan_files({dry_run_arg}) "
        f"SETTINGS {_REMOVE_ORPHAN_FILES_SETTINGS} FORMAT TabSeparated"
    )
    text = _ch_query(cfg, sql)
    counts = _parse_tsv_kv(text)
    return {
        "table_name": table_name,
        "dry_run": dry_run,
        **{k: counts.get(k, 0) for k in _ROF_COUNT_COLUMNS},
    }


def probe_expire_snapshots_skip(cfg: MaintenanceConfig, table_name: str) -> str:
    """Attempt `expire_snapshots(dry_run=1)` against `table_name` and
    return the REAL error text ClickHouse gives THIS run, rather than a
    hardcoded string that could silently drift out of sync with what the
    server actually says (the exact trap the review flagged: "a loud,
    logged skip carrying the real error — never a silent no-op"). On
    ClickHouse 26.8 this is `Code: 48 ... not supported for Iceberg tables
    backed by a transactional catalog` — a deliberate restriction (a REST
    catalog owns snapshot expiry, not the query engine), not a bug; see
    `docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md`. If some future
    ClickHouse version makes this succeed again, that is reported loudly
    too (the caller logs whatever this returns either way — this function
    never swallows the outcome, it just describes it)."""
    sql = (
        f"ALTER TABLE {CATALOG_DB}.`{table_name}` "
        f"EXECUTE expire_snapshots(dry_run=1) "
        f"SETTINGS {_EXPIRE_SNAPSHOTS_SETTINGS} FORMAT TabSeparated"
    )
    try:
        text = _ch_query(cfg, sql)
    except requests.HTTPError as exc:
        body = exc.response.text.strip() if exc.response is not None else str(exc)
        return f"unsupported this run — real ClickHouse error: {body}"
    return (
        f"unexpectedly SUCCEEDED this run (result: {text.strip()!r}) — this "
        "ClickHouse version may no longer restrict expire_snapshots against "
        "a transactional catalog; re-check docs/plans/"
        "CLICKHOUSE-26.8-REMEASUREMENT.md before trusting this as a skip"
    )


def _mint_lakekeeper_read_token(cfg: MaintenanceConfig) -> str | None:
    """Mint a short-lived bearer token from `ops/oidc-mock`'s
    client-credentials endpoint as the `clickhouse-reader` principal — the
    SAME pattern `ops/g3a/g3a_test.py`'s `_mint_lakekeeper_read_token` uses
    to hit Lakekeeper's Iceberg REST catalog directly, reusing this job's
    existing `select` grant (see `lakekeeper-authz-init` in
    `docker-compose.yml`) rather than requesting a new one. Returns `None`
    on a pre-R1/authz-disabled stack (`ch_oauth_client_id` unset there
    too), where `measure_snapshot_growth` degrades to an honest
    "not measured" rather than a failure."""
    if not cfg.ch_oauth_client_id:
        return None
    resp = requests.post(
        cfg.ch_oauth_server_uri, data={"client_id": cfg.ch_oauth_client_id}, timeout=10
    )
    resp.raise_for_status()
    token = resp.json().get("access_token")
    if not token:
        raise RuntimeError(f"token endpoint returned no access_token: {resp.json()}")
    return token


def _resolve_catalog_prefix(cfg: MaintenanceConfig, token: str) -> str:
    """`GET /v1/config` resolves the warehouse NAME (`cfg.lakekeeper_warehouse`)
    to the REST catalog's own path `prefix` — the same two-step resolution
    `ops/g3a/g3a_test.py`'s `step_verify_format_version_2` already performs
    and has proven works against this stack."""
    resp = requests.get(
        f"{cfg.lakekeeper_catalog_uri}/v1/config",
        params={"warehouse": cfg.lakekeeper_warehouse},
        headers={"Authorization": f"Bearer {token}"},
        timeout=10,
    )
    resp.raise_for_status()
    body = resp.json()
    prefix = {**body.get("defaults", {}), **body.get("overrides", {})}.get("prefix")
    if not prefix:
        raise RuntimeError(f"catalog /v1/config returned no warehouse prefix: {body}")
    return prefix


def measure_snapshot_growth(cfg: MaintenanceConfig, table_name: str) -> dict[str, Any]:
    """Measure the unbounded-growth gap the module doc describes:
    `snapshot_count` (from the table's own `metadata.snapshots`) and
    `metadata_log_count` (from `metadata.metadata-log`, the retained-old-
    metadata-file trail) — both read straight from the Iceberg REST
    catalog's table-metadata document (`GET /v1/{prefix}/namespaces/{ns}/
    tables/{table}`), never guessed or derived from ClickHouse. Neither
    `expire_snapshots` (unsupported on 26.8 for catalog-backed tables) nor
    `remove_orphan_files` (files, not snapshots) reduces these numbers —
    this function exists to make sure that fact is VISIBLE and trending
    rather than silent, per the module doc's "Lakekeeper-side snapshot
    expiry" section. `table_name` is the catalog's own two-part name (e.g.
    `bronze.g3a_orders`, same shape `discover_bronze_tables` returns).

    Returns `{"measured": False, ...}` (zeros) rather than raising when
    `CH_OAUTH_CLIENT_ID` is unset (pre-R1/authz-disabled stack, no way to
    mint a catalog token) — a stack with no per-principal auth wired up
    yet has no `expire_snapshots` write path to measure a gap in either,
    so a skip here is consistent with the rest of this module's degrade-
    gracefully posture on that same condition."""
    token = _mint_lakekeeper_read_token(cfg)
    if token is None:
        return {"measured": False, "snapshot_count": 0, "metadata_log_count": 0}

    prefix = _resolve_catalog_prefix(cfg, token)
    namespace, _, table = table_name.partition(".")
    resp = requests.get(
        f"{cfg.lakekeeper_catalog_uri}/v1/{prefix}/namespaces/{namespace}/tables/{table}",
        headers={"Authorization": f"Bearer {token}"},
        timeout=10,
    )
    resp.raise_for_status()
    metadata = resp.json().get("metadata") or {}
    snapshots = metadata.get("snapshots") or []
    metadata_log = metadata.get("metadata-log") or []
    return {
        "measured": True,
        "snapshot_count": len(snapshots),
        "metadata_log_count": len(metadata_log),
    }


def _table_namespace_and_name(table_name: str) -> tuple[str, str]:
    """Splits the catalog's own two-part name (e.g. `bronze.g3a_orders`,
    the shape `discover_bronze_tables` returns) into
    `(namespace, table_name)` — the exact key shape
    `table_maintenance_policy` (and its store's `(namespace, table_name)`
    primary key) uses, and the two identifiers a Trino statement
    interpolates as `iceberg.<namespace>."<table_name>"`."""
    namespace, _, name = table_name.partition(".")
    return namespace, name


def _fetch_policy_index(
    cfg: MaintenanceConfig,
) -> tuple[dict[tuple[str, str], dict[str, Any]], str | None]:
    """Fetches every configured maintenance policy from
    `GET /api/lakehouse/maintenance-policies` ONCE per run (not once per
    table), authenticating as the `lakehouse-maintenance-policy-reader`
    service identity with `LAKEHOUSE_MAINTENANCE_TOKEN` as a bearer
    token — the same `requests.get(..., headers={"Authorization": f"Bearer
    {token}"}, timeout=10)` shape `measure_snapshot_growth`'s Lakekeeper
    call already uses. Returns an index keyed by
    `(namespace, tableName)`.

    A 401/403 means the token is missing or wrong — a configuration or
    authentication problem, so this raises `dagster.Failure` rather than
    degrading (AGENTS.md: config/auth problems are a `Failure`, not a
    silent skip). Anything else that keeps this job from getting a real
    policy list — unreachable, a 5xx, or a malformed/unexpected body — is
    NOT the same as "no policies configured": it returns an empty index
    together with a non-`None` error string, so the caller records
    `"policy fetch failed: <reason>"` in `skipped_verbs` instead of
    silently treating every table as unconfigured.
    """
    if not cfg.maintenance_token:
        return {}, "LAKEHOUSE_MAINTENANCE_TOKEN is not set"

    url = f"{cfg.api_url}/api/lakehouse/maintenance-policies"
    try:
        resp = requests.get(
            url,
            headers={"Authorization": f"Bearer {cfg.maintenance_token}"},
            timeout=10,
        )
    except requests.RequestException as exc:
        return {}, f"{type(exc).__name__}"

    if resp.status_code in (401, 403):
        raise Failure(
            f"maintenance policy fetch failed with HTTP {resp.status_code} "
            f"from {url} — check LAKEHOUSE_MAINTENANCE_TOKEN"
        )

    try:
        resp.raise_for_status()
        policies = resp.json()["policies"]
        index = {(p["namespace"], p["tableName"]): p for p in policies}
    except (requests.HTTPError, ValueError, KeyError, TypeError) as exc:
        return {}, f"{type(exc).__name__}"

    return index, None


class TrinoQueryError(RuntimeError):
    """Trino reported a query-level error (a `/v1/statement` response
    page's `error.message`) — the real message, so a caller can record it
    (e.g. the 7-day `expire-snapshots.min-retention` floor refusing a
    shorter threshold), never an invented one."""


class TrinoUnavailableError(RuntimeError):
    """The `/v1/statement` request failed at the transport level, or the
    60-second result-paging cap (below) was exceeded — a degraded state
    (Trino runs only under this stack's `trino` compose profile) rather
    than a query-level failure, and never raised as a job failure."""


_TRINO_PAGING_CAP_SECONDS = 60.0


def _trino_execute(cfg: MaintenanceConfig, sql: str) -> list[list[Any]]:
    """Runs `sql` against Trino's `/v1/statement` REST protocol, sent as
    `X-Trino-User: trino-maintenance`, following the response's `nextUri`
    with plain synchronous `requests` calls under a 60-second wall-clock
    cap (`time.monotonic()`-based, matching this module's existing
    synchronous style deliberately — no `asyncio` introduced). This is a
    small, deliberate duplication of `rust/crates/lakehouse-trino`'s own
    polling loop (see the module doc): this job runs in a separate Python
    process outside the Rust binary that crate is built into.

    The `columns`/`data`/`nextUri`/`error` field names and the
    `error.message` shape below match Trino's documented `/v1/statement`
    protocol and the exact fixture shape `lakehouse-trino`'s own
    `wiremock`-backed tests already assert against
    (`rust/crates/lakehouse-trino/src/lib.rs`); this job's own G2 gate
    exercises the real syntax live.

    # Errors
    Raises [`TrinoQueryError`] with Trino's own message on a query-level
    error, [`TrinoUnavailableError`] on a transport failure or if the
    60-second cap is exceeded before paging finishes.
    """
    deadline = time.monotonic() + _TRINO_PAGING_CAP_SECONDS

    def _post(uri: str, *, body: bytes | None) -> dict[str, Any]:
        try:
            if body is None:
                resp = requests.get(uri, timeout=10)
            else:
                resp = requests.post(
                    uri, data=body, headers={"X-Trino-User": "trino-maintenance"}, timeout=10
                )
            resp.raise_for_status()
            return resp.json()
        except requests.RequestException as exc:
            raise TrinoUnavailableError(f"trino unreachable: {exc}") from exc

    page = _post(f"{cfg.trino_url}/v1/statement", body=sql.encode("utf-8"))
    rows: list[list[Any]] = []
    while True:
        error = page.get("error")
        if error:
            raise TrinoQueryError(error.get("message", "trino query failed"))
        rows.extend(page.get("data") or [])
        next_uri = page.get("nextUri")
        if not next_uri:
            return rows
        if time.monotonic() >= deadline:
            raise TrinoUnavailableError(
                "trino query exceeded the 60s result-paging cap"
            )
        page = _post(next_uri, body=None)


def _parse_trino_timestamp(value: str) -> datetime:
    """Parses a Trino `timestamp(p) with time zone` column value from the
    `/v1/statement` protocol — the documented rendering is
    `YYYY-MM-DD HH:MM:SS[.fff] <zone name>` (e.g.
    `2026-01-01 00:00:00.000 UTC`) — into an aware UTC `datetime`. Falls
    back to `datetime.fromisoformat` for a plain ISO-8601 string with no
    trailing zone name, so a differently-configured Trino session still
    parses. Not measured against a live Trino response: the procedure
    syntax is resolved from `ops/trino/optimize_bronze.sh`'s existing proof
    and Trino's documented form, not a fresh live capture; this job's own
    G2 gate exercises the real response shape."""
    text = value.strip()
    if text.endswith(" UTC"):
        text = text[: -len(" UTC")]
        fmt = "%Y-%m-%d %H:%M:%S.%f" if "." in text else "%Y-%m-%d %H:%M:%S"
        return datetime.strptime(text, fmt).replace(tzinfo=timezone.utc)
    parsed = datetime.fromisoformat(text.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed


def _parse_run_at(value: str | None) -> datetime | None:
    """Parses `bronze_catalog._utc_now_iso`'s fixed
    `%Y-%m-%dT%H:%M:%SZ` format (the `bronze_meta.maintenance_run.run_at`
    column's own shape) back into an aware UTC `datetime`, or `None` for
    a missing/unparseable value — "never run before" is exactly the case
    the schedule cadence gate below treats as "always allowed"."""
    if not value:
        return None
    try:
        return datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
    except ValueError:
        return None


def _cadence_allows(
    schedule: str | None, now: datetime, last_run_at: datetime | None
) -> tuple[bool, str | None]:
    """Per-table schedule cadence for the Trino verbs only (module doc).
    `schedule=None` or no prior recorded run always allows a run. Returns
    `(True, None)` when the run is allowed, `(False, <reason>)`
    otherwise. Pure — takes `now` and `last_run_at` explicitly so this is
    testable on fixed times, with no wall-clock or network dependency."""
    if schedule is None or last_run_at is None:
        return True, None
    elapsed = now - last_run_at
    if schedule == "weekly" and elapsed < timedelta(days=7):
        return False, f"schedule=weekly: last Trino verb run {elapsed} ago, needs >= 7 days"
    if schedule == "daily" and elapsed < timedelta(hours=20):
        return False, f"schedule=daily: last Trino verb run {elapsed} ago, needs >= 20 hours"
    return True, None


def _compute_retention_threshold(
    committed_at: list[datetime], keep: int, now: datetime
) -> tuple[str | None, str | None]:
    """Computes the `expire_snapshots(retention_threshold => ...)`
    argument that keeps exactly the `keep` newest snapshots, from
    `committed_at` (every snapshot's commit time, NEWEST FIRST — the
    order `SELECT committed_at FROM iceberg.<ns>."<t>$snapshots" ORDER BY
    committed_at DESC` returns). Trino 483's `expire_snapshots` takes only
    an age, never a keep-count, so a table with `keep` or fewer snapshots
    has nothing to expire (returns `(None, <skip reason>)` — never a
    `'0d'` threshold, which would expire every snapshot but the current
    one regardless of `keep`, a real data-loss bug the naive reading of
    this procedure invites). Otherwise the threshold is the AGE of the
    `keep`-th newest snapshot, in whole seconds rounded up
    (`'<secs>s'`) — snapshots newer than that survive, and there are
    exactly `keep` of them. Pure — `now` is explicit so this is testable
    on fixed times."""
    if len(committed_at) <= keep:
        return None, (
            f"snapshotsToKeep={keep}: table has {len(committed_at)} snapshot(s), "
            "nothing to expire"
        )
    nth_newest = committed_at[keep - 1]
    age_seconds = math.ceil((now - nth_newest).total_seconds())
    return f"{age_seconds}s", None


_TRINO_UNREACHABLE_SKIP = "expire_snapshots/optimize skipped: trino unreachable"


def _expire_snapshots_result(
    cfg: MaintenanceConfig,
    namespace: str,
    name: str,
    policy: dict[str, Any] | None,
    expire_skip_reason: str,
    now: datetime,
    last_run_at: datetime | None,
) -> tuple[str | None, dict[str, str] | None]:
    """Decides and (if requested) executes this table's `expire_snapshots`
    verb via Trino. Returns `(skipped_verbs_entry, verb_run_record)`:
    `skipped_verbs_entry` is `None` exactly when the verb genuinely ran
    (it must not also be recorded as skipped); `verb_run_record` is
    `None` only when no policy requested this verb at all (nothing to
    record in `bronze_meta.maintenance_verb_run` either).

    With no policy, or a policy that does not set `snapshotsToKeep`,
    this keeps today's exact behavior: the real, freshly-observed
    ClickHouse probe failure (`expire_skip_reason`) is the recorded skip
    reason, and no Trino call is made."""
    keep = None if policy is None else policy.get("snapshotsToKeep")
    if keep is None:
        return f"expire_snapshots ({expire_skip_reason})", None

    cadence_ok, cadence_reason = _cadence_allows(policy.get("schedule"), now, last_run_at)
    if not cadence_ok:
        return (
            f"expire_snapshots ({cadence_reason})",
            {"verb": "expire_snapshots", "engine": "trino", "outcome": "skipped", "detail": cadence_reason or ""},
        )

    try:
        rows = _trino_execute(
            cfg,
            f'SELECT committed_at FROM iceberg.{namespace}."{name}$snapshots" '
            "ORDER BY committed_at DESC",
        )
    except TrinoUnavailableError:
        return (
            _TRINO_UNREACHABLE_SKIP,
            {"verb": "expire_snapshots", "engine": "trino", "outcome": "skipped", "detail": "trino unreachable"},
        )
    except TrinoQueryError as exc:
        detail = str(exc)
        return (
            f"expire_snapshots (failed reading snapshot times: {detail})",
            {"verb": "expire_snapshots", "engine": "trino", "outcome": "failed", "detail": detail},
        )

    committed_at = [_parse_trino_timestamp(row[0]) for row in rows]
    threshold, skip_reason = _compute_retention_threshold(committed_at, keep, now)
    if skip_reason is not None:
        return (
            f"expire_snapshots ({skip_reason})",
            {"verb": "expire_snapshots", "engine": "trino", "outcome": "skipped", "detail": skip_reason},
        )

    sql = (
        f'ALTER TABLE iceberg.{namespace}."{name}" '
        f"EXECUTE expire_snapshots(retention_threshold => '{threshold}')"
    )
    try:
        _trino_execute(cfg, sql)
    except TrinoUnavailableError:
        return (
            _TRINO_UNREACHABLE_SKIP,
            {"verb": "expire_snapshots", "engine": "trino", "outcome": "skipped", "detail": "trino unreachable"},
        )
    except TrinoQueryError as exc:
        # Most likely Trino's `iceberg.expire-snapshots.min-retention`
        # floor (7 days by default, unset in this stack) refusing a
        # shorter threshold — recorded honestly and never retried larger.
        detail = str(exc)
        return (
            f"expire_snapshots refused: {detail}",
            {"verb": "expire_snapshots", "engine": "trino", "outcome": "refused", "detail": detail},
        )

    return (
        None,
        {
            "verb": "expire_snapshots",
            "engine": "trino",
            "outcome": "applied",
            "detail": f"retention_threshold={threshold}",
        },
    )


def _optimize_result(
    cfg: MaintenanceConfig,
    namespace: str,
    name: str,
    policy: dict[str, Any] | None,
    now: datetime,
    last_run_at: datetime | None,
) -> tuple[str | None, dict[str, str] | None]:
    """Decides and (if requested) executes this table's `optimize` verb
    via Trino — the same shape [`_expire_snapshots_result`] documents.
    With no policy, or `compactSmallFiles=False`, this keeps today's
    exact fixed design-skip message: `OPTIMIZE` genuinely never runs on
    this ClickHouse (it does not bin-pack — see the module doc), and
    Trino owns compaction instead only when a policy asks for it."""
    if policy is None or not policy.get("compactSmallFiles"):
        return (
            "OPTIMIZE (returns OK on 26.8 but does not bin-pack — not a "
            "compaction remedy; Trino owns that, see ADR 0009)",
            None,
        )

    cadence_ok, cadence_reason = _cadence_allows(policy.get("schedule"), now, last_run_at)
    if not cadence_ok:
        return (
            f"optimize ({cadence_reason})",
            {"verb": "optimize", "engine": "trino", "outcome": "skipped", "detail": cadence_reason or ""},
        )

    sql = f'ALTER TABLE iceberg.{namespace}."{name}" EXECUTE optimize'
    try:
        _trino_execute(cfg, sql)
    except TrinoUnavailableError:
        return (
            _TRINO_UNREACHABLE_SKIP,
            {"verb": "optimize", "engine": "trino", "outcome": "skipped", "detail": "trino unreachable"},
        )
    except TrinoQueryError as exc:
        detail = str(exc)
        return (
            f"optimize failed: {detail}",
            {"verb": "optimize", "engine": "trino", "outcome": "failed", "detail": detail},
        )

    return None, {"verb": "optimize", "engine": "trino", "outcome": "applied", "detail": ""}


@op
def run_bronze_maintenance(context) -> list[dict[str, Any]]:
    """The P4 maintenance chain, per Bronze table: `remove_orphan_files`
    dry-run (metrics only, matching the task brief's "dry_run metrics
    surfaced in console" requirement) then the real run — the working verb
    on ClickHouse 26.8. Also measures (never reclaims —
    `measure_snapshot_growth`) per-table snapshot/metadata-log counts, so
    the unbounded growth `expire_snapshots`'s removal leaves behind is
    tracked every run instead of silent (see module doc's "Lakekeeper-side
    snapshot expiry" section).

    The table's configured policy (module doc, "Per-table policy + Trino
    verbs") then decides `expire_snapshots`/`optimize`: with no policy at
    all, this keeps today's exact behavior — the real, freshly-observed
    ClickHouse probe failure recorded for `expire_snapshots`, and
    `OPTIMIZE`'s fixed design-skip message, both unconditional. The
    policy list itself is fetched ONCE per run (not once per table); a
    401/403 is a configuration/auth problem and raises `dagster.Failure`,
    while an unreachable API, a 5xx, or a malformed body is recorded as a
    per-table skip and the run continues with unconditional orphan
    removal (never silently treated as "no policy")."""
    cfg = MaintenanceConfig.from_env()
    _ensure_catalog_database(cfg)
    tables = discover_bronze_tables(cfg)
    context.log.info(f"discovered {len(tables)} Bronze table(s): {tables}")

    policy_index, policy_fetch_error = _fetch_policy_index(cfg)
    if policy_fetch_error is not None:
        context.log.warning(
            f"maintenance policy fetch failed ({policy_fetch_error}) — every "
            "table is treated as unconfigured this run, unconditional "
            "orphan-file removal still runs"
        )

    results: list[dict[str, Any]] = []
    for table_name in tables:
        expire_skip_reason = probe_expire_snapshots_skip(cfg, table_name)
        context.log.warning(f"[{table_name}] skipping expire_snapshots: {expire_skip_reason}")

        dry = run_remove_orphan_files(cfg, table_name, dry_run=True)
        context.log.info(f"[dry-run] {table_name}: {dry}")
        real = run_remove_orphan_files(cfg, table_name, dry_run=False)
        context.log.info(f"[applied] {table_name}: {real}")

        growth = measure_snapshot_growth(cfg, table_name)
        if growth["measured"]:
            context.log.info(
                f"[{table_name}] snapshot growth: "
                f"{growth['snapshot_count']} snapshot(s), "
                f"{growth['metadata_log_count']} metadata-log entr(y/ies) — "
                "nothing in this stack reclaims these yet, see module doc"
            )
        else:
            context.log.warning(
                f"[{table_name}] snapshot growth NOT measured this run "
                "(no CH_OAUTH_CLIENT_ID — pre-R1/authz-disabled stack)"
            )

        namespace, name = _table_namespace_and_name(table_name)
        policy = None if policy_fetch_error is not None else policy_index.get((namespace, name))
        now = datetime.now(timezone.utc)

        skipped_verbs: list[str] = []
        verb_runs: list[dict[str, str]] = []

        if policy_fetch_error is not None:
            skipped_verbs.append(f"policy fetch failed: {policy_fetch_error}")

        last_run_at: datetime | None = None
        if policy is not None:
            if policy.get("orphanAgeHours") is not None:
                detail = (
                    "orphan_age_hours not applied: remove_orphan_files is "
                    "called without an age argument on this ClickHouse"
                )
                skipped_verbs.append(detail)
                verb_runs.append(
                    {
                        "verb": "orphan_age_hours",
                        "engine": "clickhouse",
                        "outcome": "skipped",
                        "detail": detail,
                    }
                )
            last_run_at = _parse_run_at(latest_maintenance_run_at(cfg.ch, table_name))

        expire_skip, expire_verb_run = _expire_snapshots_result(
            cfg, namespace, name, policy, expire_skip_reason, now, last_run_at
        )
        if expire_skip is not None and expire_skip not in skipped_verbs:
            skipped_verbs.append(expire_skip)
        if expire_verb_run is not None:
            verb_runs.append(expire_verb_run)

        optimize_skip, optimize_verb_run = _optimize_result(
            cfg, namespace, name, policy, now, last_run_at
        )
        if optimize_skip is not None and optimize_skip not in skipped_verbs:
            skipped_verbs.append(optimize_skip)
        if optimize_verb_run is not None:
            verb_runs.append(optimize_verb_run)

        context.log.info(f"[{table_name}] skipped_verbs: {skipped_verbs}")

        record_maintenance_run(
            table_name=table_name,
            dry_run_metrics=dry,
            applied_metrics=real,
            skipped_verbs=skipped_verbs,
            snapshot_growth=growth,
            target=cfg.ch,
        )
        if verb_runs:
            record_maintenance_verb_run(
                table_name=table_name,
                run_at=datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                verb_runs=verb_runs,
                target=cfg.ch,
            )
        results.append(
            {
                "table_name": table_name,
                "dry_run": dry,
                "applied": real,
                "skipped_verbs": skipped_verbs,
                "snapshot_growth": growth,
            }
        )

    context.add_output_metadata({"tables_maintained": len(results)})
    return results


@job
def bronze_maintenance_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `bronze_maintenance_job`.
    Launched the same way `bronze_ingest_job` (P3) is — no Rust-side
    special-casing needed."""
    run_bronze_maintenance()


# Daily at 03:00 — arbitrary but conservative cadence for a Bronze table
# under active CDC/dlt writes; `dagster-daemon` (already in the P3 compose
# topology, see ADR 0005) is what actually fires this.
# `default_status=RUNNING` is load-bearing, not decoration. A
# `ScheduleDefinition` without it is created STOPPED, so on a fresh
# deployment this never fires until somebody notices and toggles it in the
# Dagster UI — and nothing surfaces that it is off. Bronze maintenance
# silently never running is exactly the failure this job exists to prevent,
# and an off schedule looks identical to a healthy one from outside.
#
# Stated because RUNNING is the less reversible default: on first boot this
# fires at the next 03:00 without anyone opting in. That is intended — an
# operator who wants it off stops it in the UI, a visible action, whereas
# the previous default required a hidden action to get the behaviour the
# module already documented.
bronze_maintenance_schedule = ScheduleDefinition(
    job=bronze_maintenance_job,
    cron_schedule="0 3 * * *",
    default_status=DefaultScheduleStatus.RUNNING,
)

maintenance_defs = Definitions(
    jobs=[bronze_maintenance_job],
    schedules=[bronze_maintenance_schedule],
)
