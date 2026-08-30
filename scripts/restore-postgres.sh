#!/usr/bin/env bash
# Restore a Postgres dump produced by scripts/backup-postgres.sh into the
# `docker-compose.yml` `postgres` service using `pg_restore`.
#
# Usage:
#   scripts/restore-postgres.sh <dump-file> [target-db]
#
# By default restores into POSTGRES_DB (danger: this is a --clean restore,
# it drops and recreates objects in the target database). Pass a
# [target-db] to restore into a different, scratch database instead —
# strongly recommended for verifying a backup without touching the live
# database.
#
# Env overrides (defaults match docker-compose.yml / .env.example):
#   COMPOSE_PROJECT   docker compose project name (pass -p to override)
#   POSTGRES_SERVICE  compose service name (default: postgres)
#   POSTGRES_USER     (default: lakehouse)
#   POSTGRES_DB       (default: lakehouse) — used only if [target-db] omitted
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <dump-file> [target-db]" >&2
  exit 1
fi

DUMP_FILE="$1"
POSTGRES_SERVICE="${POSTGRES_SERVICE:-postgres}"
POSTGRES_USER="${POSTGRES_USER:-lakehouse}"
TARGET_DB="${2:-${POSTGRES_DB:-lakehouse}}"
COMPOSE=(docker compose)
if [[ -n "${COMPOSE_PROJECT:-}" ]]; then
  COMPOSE=(docker compose -p "${COMPOSE_PROJECT}")
fi

if [[ ! -f "${DUMP_FILE}" ]]; then
  echo "dump file not found: ${DUMP_FILE}" >&2
  exit 1
fi

echo "==> Ensuring target database '${TARGET_DB}' exists on service '${POSTGRES_SERVICE}'"
"${COMPOSE[@]}" exec -T "${POSTGRES_SERVICE}" \
  psql -U "${POSTGRES_USER}" -d postgres -tc \
  "SELECT 1 FROM pg_database WHERE datname = '${TARGET_DB}'" | grep -q 1 || \
  "${COMPOSE[@]}" exec -T "${POSTGRES_SERVICE}" \
    createdb -U "${POSTGRES_USER}" "${TARGET_DB}"

echo "==> Restoring ${DUMP_FILE} into '${TARGET_DB}' (--clean --if-exists)"
"${COMPOSE[@]}" exec -T "${POSTGRES_SERVICE}" \
  pg_restore -U "${POSTGRES_USER}" -d "${TARGET_DB}" --clean --if-exists --no-owner \
  < "${DUMP_FILE}"

echo "==> Restore complete into '${TARGET_DB}'"
