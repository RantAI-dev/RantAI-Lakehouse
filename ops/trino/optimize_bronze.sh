#!/bin/sh
# ADR 0009's escape hatch, running loop. `docs/plans/G3-RESULT.md` measured
# that ClickHouse has NO working small-file compaction path against a
# catalog-registered Iceberg table on 26.3 (`OPTIMIZE` parses but fails at
# runtime with an S3 403 on every attempt — a genuine ClickHouse bug, not a
# credentials problem, reproduced with both vended and static admin
# credentials). Trino's `ALTER TABLE ... EXECUTE optimize` against the
# SAME Lakekeeper/RustFS-backed table DOES work (measured: 280 tiny files
# -> 14 files, one per partition, restoring query-planning time to the
# uncompacted baseline). This script is the cron loop that runs it.
#
# Scope: Bronze ONLY, every table under the `bronze` namespace, discovered
# fresh each run (never a hardcoded table list, so a new Bronze table from
# P3/P5 ingestion is picked up automatically) — per the pre-authorized
# decision in `docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §3 ("Trino-as-cron
# for `optimize` on Bronze only"). Silver/Gold are ClickHouse MergeTree,
# not Iceberg — Trino has no reason to ever touch them, and this script
# does not attempt to.
set -eu

INTERVAL_SECONDS="${TRINO_CRON_INTERVAL_SECONDS:-21600}"  # 6h default

log() { echo "[trino-maintenance-cron] $(date -u +%Y-%m-%dT%H:%M:%SZ) $*"; }

run_once() {
  # Trino's file-based access control (docker-compose.yml, `trino` service)
  # grants `all` on the `iceberg` catalog to exactly this user; every other
  # user gets `read-only` there. Without `--user trino-maintenance` the CLI
  # sends no `X-Trino-User` at all, falls back to the CLI's default identity,
  # and both calls below would be denied.
  # Capture the CLI's own exit status (not the trailing `tail`'s) so a real
  # Trino failure (coordinator not yet warm, network blip) is distinguished
  # from a genuinely empty `bronze` namespace — piping straight into `tail`
  # loses that distinction because a pipeline's status is its last command's.
  if ! raw=$(trino --server http://trino:8080 --user trino-maintenance \
      --execute "SHOW TABLES FROM iceberg.bronze" \
      --output-format TSV_HEADER 2>&1); then
    log "WARNING: could not list Bronze tables (trino unavailable?): $raw"
    return 0
  fi
  tables=$(printf '%s\n' "$raw" | tail -n +2)
  if [ -z "$tables" ]; then
    log "no Bronze tables found via iceberg.bronze — nothing to optimize"
    return 0
  fi
  echo "$tables" | while IFS= read -r t; do
    [ -z "$t" ] && continue
    log "optimize iceberg.bronze.\"$t\""
    if ! trino --server http://trino:8080 --user trino-maintenance \
        --execute "ALTER TABLE iceberg.bronze.\"$t\" EXECUTE optimize" 2>&1; then
      log "WARNING: optimize failed for $t (continuing with remaining tables)"
    fi
  done
}

log "starting, interval=${INTERVAL_SECONDS}s"
while true; do
  run_once || log "WARNING: run_once failed (continuing)"
  sleep "$INTERVAL_SECONDS"
done
