#!/usr/bin/env bash
# Back up the Postgres database running in the `docker-compose.yml`
# `postgres` service using `pg_dump`, in the custom (`-Fc`) format so it can
# be restored selectively with `pg_restore` (see restore-postgres.sh).
#
# Usage:
#   scripts/backup-postgres.sh [output-dir]
#
# Env overrides (defaults match docker-compose.yml / .env.example):
#   COMPOSE_PROJECT   docker compose project name (default: whatever the
#                      stack was brought up with; pass -p to override)
#   POSTGRES_SERVICE  compose service name (default: postgres)
#   POSTGRES_USER     (default: lakehouse)
#   POSTGRES_DB       (default: lakehouse)
#   BACKUP_DIR        where dumps land (default: ./backups)
#   RETENTION_DAYS    delete dumps older than this many days (default: 14)
set -euo pipefail

POSTGRES_SERVICE="${POSTGRES_SERVICE:-postgres}"
POSTGRES_USER="${POSTGRES_USER:-lakehouse}"
POSTGRES_DB="${POSTGRES_DB:-lakehouse}"
BACKUP_DIR="${1:-${BACKUP_DIR:-./backups}}"
RETENTION_DAYS="${RETENTION_DAYS:-14}"
COMPOSE=(docker compose)
if [[ -n "${COMPOSE_PROJECT:-}" ]]; then
  COMPOSE=(docker compose -p "${COMPOSE_PROJECT}")
fi

mkdir -p "${BACKUP_DIR}"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
outfile="${BACKUP_DIR}/${POSTGRES_DB}-${timestamp}.dump"

echo "==> Backing up '${POSTGRES_DB}' from service '${POSTGRES_SERVICE}' to ${outfile}"

"${COMPOSE[@]}" exec -T "${POSTGRES_SERVICE}" \
  pg_dump -U "${POSTGRES_USER}" -d "${POSTGRES_DB}" -Fc \
  > "${outfile}"

size="$(du -h "${outfile}" | cut -f1)"
echo "==> Wrote ${outfile} (${size})"

# Retention: prune dumps for this database older than RETENTION_DAYS.
if [[ "${RETENTION_DAYS}" -gt 0 ]]; then
  find "${BACKUP_DIR}" -maxdepth 1 -name "${POSTGRES_DB}-*.dump" -mtime "+${RETENTION_DAYS}" -print -delete \
    | sed 's/^/==> Pruned (older than '"${RETENTION_DAYS}"'d): /'
fi

echo "==> Done: ${outfile}"
