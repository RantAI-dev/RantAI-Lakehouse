# ADR 0009 — Small-file compaction: the Trino-as-cron escape hatch

- **Status:** Accepted
- **Phase:** P4
- **Date:** 2026-09-01

## Context

`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §3 (P4/G3) pre-authorized a
specific escape hatch, decided by measurement, not preference: "if query
planning degrades beyond 2x, the pre-authorized escape hatch is a
Trino-as-cron container running `optimize` on Bronze only." R2 in the risk
register already flagged this as increasingly likely after
`docs/plans/CLICKHOUSE-MAINTENANCE-FINDINGS.md` found `OPTIMIZE ...
MANIFEST` to be a syntax error on ClickHouse 26.3.

P4 measured the full chain directly against a real catalog-registered
Bronze table (`docs/plans/G3-RESULT.md`). Result: of the three commands
`CLICKHOUSE-MAINTENANCE-FINDINGS.md` had narrowed the brief's original four
to, only `expire_snapshots` actually works. `remove_orphan_files` does not
exist for Iceberg tables (`Unknown EXECUTE command`, not a generic
NOT_IMPLEMENTED). `OPTIMIZE` (the only verb of the three that could
compact data files) parses and is correctly gated, but fails at runtime on
every attempt with an S3 403 — reproduced with both vended and static
admin credentials, ruling out a permissions cause and pointing to a
genuine ClickHouse defect in its Iceberg `OPTIMIZE` write path on 26.3.
`expire_snapshots` itself does not compact data files (it reclaims aged
snapshot/manifest metadata, a different concern).

Net: **zero working in-engine small-file compaction exists on this
ClickHouse version.** A synthetic load of 20 small files/partition across
14 day-partitions measured ~15-20x worse query-planning-time (proxied by a
single-partition `COUNT` with the Iceberg metadata cache disabled) than a
1-file/partition control with identical row totals — far past the plan's
2x threshold.

### Correction after re-measuring on 26.8 — the decision holds, the reason above does not

Everything above was measured on 26.3. Re-run on `26.8.2.7`
([`docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md`](../plans/CLICKHOUSE-26.8-REMEASUREMENT.md)),
two of the three premises changed:

- **The `OPTIMIZE` S3 403 is fixed.** It returns OK on 26.8.
- **`remove_orphan_files` now exists** and works.
- (And `expire_snapshots`, the one verb that *did* work on 26.3, is now
  explicitly refused for catalog-backed tables.)

So "OPTIMIZE fails with a 403" is no longer true and must not be cited as the
reason for this ADR.

**The decision is unchanged, because the real reason was never the 403.**
Measured on 26.8: seven Parquet data files in a single day partition,
`OPTIMIZE` returns OK, and **seven files remain**. ClickHouse's `OPTIMIZE`
merges *position-delete* files into data files; it does not bin-pack small
data files, and never claimed to — the original task brief said plainly
"NOT available in ClickHouse: bin-pack rewrite of small data files."

The 403 masked that on 26.3: a command that errors and a command that
succeeds without compacting are indistinguishable from the outside if you
only check the exit status. The correct statement of this ADR's premise is:

> ClickHouse has no bin-pack rewrite of small Iceberg data files at any
> version tested, so the small-file degradation G3 measured has no in-engine
> remedy.

That is a capability gap rather than a defect, which makes it *less* likely
to disappear in a future release — if anything it strengthens the case for
the escape hatch rather than weakening it.

## Decision

**Add Trino as a cron-driven compaction runner, scoped to Bronze only.**
Concretely (`docker-compose.yml`, `trino` profile — never started by a
plain `docker compose up`, matching every other opt-in profile in this
stack):

- **`trino`**: a single-node, coordinator-only Trino (`trinodb/trino:483`,
  pinned), with exactly one catalog, `iceberg`, pointed at the SAME
  Lakekeeper REST catalog URI / RustFS-or-SeaweedFS S3 endpoint every
  other Bronze consumer in this stack uses. No other catalog is
  configured — Trino has no route to Silver/Gold (ClickHouse MergeTree,
  not Iceberg) and this ADR does not give it one.
- **`trino-maintenance-cron`**: the loop itself
  (`ops/trino/optimize_bronze.sh`). Each pass: `SHOW TABLES FROM
  iceberg.bronze` (fresh discovery every run — no hardcoded table list, so
  a new Bronze table from P3/P5 ingestion is picked up automatically),
  then `ALTER TABLE iceberg.bronze."<table>" EXECUTE optimize` per table.
  Default interval 6h (`TRINO_CRON_INTERVAL_SECONDS`), a conservative
  cadence for a store under active CDC/dlt writes; one table failing to
  optimize does not stop the others in the same pass.
- **Verified working**, not assumed: `ALTER TABLE ... EXECUTE optimize`
  against the exact 280-small-file synthetic table rewrote it to 14 files
  (one per partition) and restored the planning-time proxy to the
  1-file/partition control's baseline (~0.053s vs. ~1.05s before). See
  `docs/plans/G3-RESULT.md` for the full before/after numbers.

## Why Trino and not another engine

The plan named Trino specifically as the pre-authorized choice — this ADR
does not re-litigate that, only records that the trigger condition (>2x
degradation, no in-engine fix) was met and the pre-authorized answer works.
Two properties made it a clean fit, confirmed during verification:

- Trino's Iceberg connector talks to the identical REST catalog protocol
  (Lakekeeper) every other component in this stack already uses — no new
  catalog, no new credential story beyond what `iceberg.rest-catalog.*`
  and `s3.*` connector properties already express declaratively.
- `ALTER TABLE ... EXECUTE optimize` is Trino's own native compaction verb
  for Iceberg tables (unlike ClickHouse's broken write path), requiring no
  workaround or manual manifest surgery.

## What this does NOT change

- **ClickHouse remains the only query engine the console/API talk to.**
  Trino never serves a user-facing query in this design; it exists solely
  as a background compactor. This does not violate "ClickHouse only for
  compute" any more than ADR 0010's Rust-side Gold export does — it adds a
  write-side maintenance tool, not a second query engine in the read path.
- **Silver/Gold are untouched.** They are ClickHouse MergeTree tables, not
  Iceberg; Trino's `iceberg` catalog has no way to reach them and none is
  configured.
- **`dagster/dispar_orchestrate/maintenance.py` is unchanged by this
  decision** — it still runs `expire_snapshots` (snapshot/manifest
  hygiene) in-engine. This ADR adds a second, independent maintenance path
  for the concern `expire_snapshots` cannot address (data-file
  compaction), not a replacement.

## Consequences

- Two new compose services (`trino`, `trino-maintenance-cron`), both
  behind the `trino` profile, opt-in exactly like `dagster`/`seaweedfs`
  before them. A plain `docker compose up` is unaffected.
- Operationally, a deployment that never brings up the `trino` profile has
  no small-file compaction at all — small files accumulate unbounded on
  Bronze, exactly as measured. This is a real, documented operational
  requirement from this ADR forward: **the `trino` profile must be running
  for any deployment taking CDC-rate or dlt-batch writes into Bronze at
  meaningful volume**, not an optional nicety.
- Cost/complexity: one more container pair to operate, monitor, and keep
  patched. Accepted because the alternative (query planning degrading
  15-20x as Bronze accumulates writes) is worse and there is no in-engine
  fix on this ClickHouse version to fall back to.
- If a future ClickHouse release fixes `OPTIMIZE` against catalog-registered
  Iceberg tables, `dagster/dispar_orchestrate/maintenance.py` is the
  natural place to add it back in-engine and this escape hatch can be
  retired — nothing in this design makes that harder later.

## Verification

- `docker compose -p <project> --profile trino up -d trino` then `docker
  exec <trino> trino --server http://localhost:8080 --execute "SHOW TABLES
  FROM iceberg.bronze"` lists every registered Bronze table.
- `docs/plans/G3-RESULT.md`: the full before/after file-count and
  planning-time numbers, plus the exact `optimize` command output
  (`rewritten_data_files_count: 280`, `added_data_files_count: 14`).
- `docker compose -p <project> --profile trino run --rm
  trino-maintenance-cron` (with `TRINO_CRON_INTERVAL_SECONDS` set low for
  a one-shot manual check) compacts every Bronze table in one pass and
  logs a per-table result line for each.
