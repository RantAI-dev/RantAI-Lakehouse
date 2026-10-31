#!/usr/bin/env bash
# REL-2 S1-T4 (decision D8): a COMPLETE backup of this stack, on the LOCAL
# disk.
#
# The `backups` compose profile's job backs up to an S3-compatible bucket
# (`ops/backups/docker-compose.backups.override.yml`,
# `dagster/dispar_orchestrate/backup_job.py`). That is the right target for
# a deployment that HAS a bucket — and it is the wrong target for an
# air-gapped host, which has none. This script is the local-disk target:
# one command, no bucket, no network.
#
# # What one backup is
#
# A directory `backups/<UTC timestamp>-<version>/` holding:
#
#   postgres.dump            `pg_dumpall` of the whole Postgres instance
#                            (the application database, Lakekeeper's,
#                            OpenFGA's and Dagster's, and the roles).
#   clickhouse/<stamp>/<db>/ `ClickHouse`'s own `BACKUP DATABASE ... TO
#                            Disk('backups', ...)` output, per database.
#   connector-secrets.tar.gz the `lakehouse_connector_secrets` volume —
#                            every stored connector credential, SEALED.
#                            Without this, a restored deployment has no
#                            working connector credential.
#   env                      a copy of the deployment's `.env` (the
#                            generated secrets, including the key the
#                            connector credentials are sealed with).
#   manifest.json            each part's size and SHA-256. The restore
#                            checks them BEFORE it restores anything.
#
# `release.json` and `state.json` (the release manifest and the installed
# version) are part of the same backup set in the plan (D8), but they do
# not exist yet: REL-2 S2 publishes the release manifest and S4 writes the
# state file. This script says so in its output rather than pretending.
#
# # Usage
#
#   ops/backups/backup-local.sh [backup-root]
#
#   backup-root   where the backup directory lands (default: ./backups)
#
# # Environment
#
#   COMPOSE_PROJECT   the compose project (default: this checkout's
#                     directory name — the same default compose uses)
#   API_HOST_PORT     the API's host port, read for the version
#                     (default: 8080)
#   POSTGRES_USER     (default: lakehouse)
#   CH_USER/CH_PASSWORD  the ClickHouse credentials, when set
#
# # What it needs
#
# Docker, `docker compose`, `curl`, `jq` and `sha256sum` on the host. The
# PostgreSQL and ClickHouse clients are the ones in the running containers
# (`pg_dumpall`, `clickhouse-client`), so a host needs no database client
# of its own.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# The alpine image the helper containers run — the same pinned image the
# stack itself uses (REL-2 S1-T3 pinned every image by digest).
HELPER_IMAGE="alpine:3.20@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc"

BACKUP_ROOT="${1:-${BACKUP_ROOT:-${REPO_ROOT}/backups}}"
PROJECT="${COMPOSE_PROJECT:-$(basename "${REPO_ROOT}")}"
POSTGRES_USER="${POSTGRES_USER:-lakehouse}"
API_HOST_PORT="${API_HOST_PORT:-8080}"
CH_USER="${CH_USER:-default}"
CH_PASSWORD="${CH_PASSWORD:-}"

cd "${REPO_ROOT}"
COMPOSE=(docker compose -p "${PROJECT}")

CONNECTOR_SECRETS_VOLUME="${PROJECT}_lakehouse_connector_secrets"
CLICKHOUSE_BACKUPS_VOLUME="${PROJECT}_lakehouse_clickhouse_backups"

# ── The version this backup belongs to ──────────────────────────────────────
# The API's own `/health` reports the build's version (REL-2 D7). A stack
# whose API is down cannot be asked, and a backup without a version is
# still a backup — the directory is then named `…-unknown` rather than
# carrying a version that was never read.
version="unknown"
if health_json="$(curl -fsS --max-time 5 "http://127.0.0.1:${API_HOST_PORT}/health" 2>/dev/null)"; then
  version="$(printf '%s' "${health_json}" | jq -r '.version // "unknown"')"
fi

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
backup_dir="${BACKUP_ROOT}/${stamp}-${version}"
# Mode 0700: this directory holds the deployment's generated secrets (the
# `.env` copy) and every stored connector credential.
mkdir -p -m 0700 "${backup_dir}"

# The helper containers run as root, so what they write is owned by root
# and unreadable by the operator. They hand it back: the host user's own
# id, so the files belong to whoever runs the backup.
HOST_UID="$(id -u)"
HOST_GID="$(id -g)"

echo "==> Backing up compose project '${PROJECT}' to ${backup_dir}"

# ── Postgres ───────────────────────────────────────────────────────────────
# `pg_dumpall`, not four `pg_dump`s: D8 asks for the whole instance in one
# part, and one part is one checksum. The local socket inside the container
# is trusted, so no password travels.
echo "--> Postgres: pg_dumpall"
"${COMPOSE[@]}" exec -T postgres pg_dumpall -U "${POSTGRES_USER}" > "${backup_dir}/postgres.dump"
if [[ ! -s "${backup_dir}/postgres.dump" ]]; then
  echo "Error: the Postgres dump is empty — refusing to write a backup that holds nothing." >&2
  exit 1
fi

# ── ClickHouse ─────────────────────────────────────────────────────────────
# `BACKUP DATABASE ... TO Disk('backups', ...)`: the disk is configured in
# `ops/backups/clickhouse/backups.xml` (mounted into the `clickhouse`
# service), so this needs no S3 and no registry.
echo "--> ClickHouse: BACKUP DATABASE ... TO Disk('backups', ...)"
ch_client=(clickhouse-client)
if [[ -n "${CH_PASSWORD}" ]]; then
  # The password goes through the environment, never the command line:
  # `clickhouse-client` reads `CLICKHOUSE_PASSWORD`.
  ch_client=(env "CLICKHOUSE_USER=${CH_USER}" "CLICKHOUSE_PASSWORD=${CH_PASSWORD}" clickhouse-client)
fi
ch_databases="$("${COMPOSE[@]}" exec -T clickhouse "${ch_client[@]}" -q \
  "SELECT name FROM system.databases WHERE name NOT IN ('system', 'INFORMATION_SCHEMA', 'information_schema') AND engine != 'DataLakeCatalog' ORDER BY name" \
  | tr -d '\r')"
if [[ -z "${ch_databases}" ]]; then
  echo "Error: ClickHouse named no database to back up." >&2
  exit 1
fi
for db in ${ch_databases}; do
  # The name comes from the server, and a backtick in it would end the
  # identifier early — doubled, the way `quote_clickhouse_identifier`
  # does it in Python.
  quoted="${db//\`/\`\`}"
  echo "    ${db}"
  "${COMPOSE[@]}" exec -T clickhouse "${ch_client[@]}" -q \
    "BACKUP DATABASE \`${quoted}\` TO Disk('backups', 'clickhouse/${stamp}/${quoted}')" >/dev/null
done

# The backup files live in the ClickHouse container's own volume; copy them
# out to the backup directory, so the backup survives the volume.
echo "--> ClickHouse: copying the backup files out of ${CLICKHOUSE_BACKUPS_VOLUME}"
docker run --rm \
  -e STAMP="${stamp}" \
  -e HOST_UID="${HOST_UID}" \
  -e HOST_GID="${HOST_GID}" \
  -v "${CLICKHOUSE_BACKUPS_VOLUME}:/src:ro" \
  -v "${backup_dir}:/out" \
  "${HELPER_IMAGE}" \
  sh -c 'mkdir -p /out/clickhouse &&
         cp -a "/src/clickhouse/${STAMP}" /out/clickhouse/ &&
         chown -R "${HOST_UID}:${HOST_GID}" /out/clickhouse'

# ── The connector secrets ──────────────────────────────────────────────────
# The volume holds every stored connector credential, SEALED with the key in
# `.env`. Losing it loses every connector's credential; losing the key
# makes them unreadable. Both are in this backup.
if docker volume inspect "${CONNECTOR_SECRETS_VOLUME}" >/dev/null 2>&1; then
  echo "--> Connector secrets: ${CONNECTOR_SECRETS_VOLUME}"
  docker run --rm \
    -e HOST_UID="${HOST_UID}" \
    -e HOST_GID="${HOST_GID}" \
    -v "${CONNECTOR_SECRETS_VOLUME}:/secrets:ro" \
    -v "${backup_dir}:/out" \
    "${HELPER_IMAGE}" \
    sh -c 'tar czf /out/connector-secrets.tar.gz -C /secrets . &&
           chown "${HOST_UID}:${HOST_GID}" /out/connector-secrets.tar.gz'
else
  echo "Warning: the volume ${CONNECTOR_SECRETS_VOLUME} does not exist — no connector" >&2
  echo "         credentials are in this backup. If the stack never stored one, this is" >&2
  echo "         expected; if it did, the backup is incomplete." >&2
fi

# ── The configuration ──────────────────────────────────────────────────────
if [[ -f "${REPO_ROOT}/.env" ]]; then
  echo "--> Configuration: .env"
  cp -p "${REPO_ROOT}/.env" "${backup_dir}/env"
else
  echo "Warning: there is no .env in ${REPO_ROOT} — the deployment's generated" >&2
  echo "         secrets are NOT in this backup." >&2
fi

# ── The manifest ───────────────────────────────────────────────────────────
# Each part's size and SHA-256. The restore checks them before it restores
# anything: a backup that was cut short must not be restored as if it were
# whole.
echo "--> Manifest: manifest.json"
parts_json="$(
  cd "${backup_dir}"
  find . -type f ! -name manifest.json -printf '%P\n' | sort | while read -r part; do
    size="$(stat -c%s "${part}")"
    sum="$(sha256sum "${part}" | cut -d' ' -f1)"
    jq -n --arg name "${part}" --argjson size "${size}" --arg sha256 "${sum}" \
      '{name: $name, sizeBytes: $size, sha256: $sha256}'
  done | jq -s '.'
)"
jq -n \
  --arg version "${version}" \
  --arg createdAt "${stamp}" \
  --arg composeProject "${PROJECT}" \
  --argjson parts "${parts_json}" \
  '{
     version: $version,
     createdAt: $createdAt,
     composeProject: $composeProject,
     parts: $parts
   }' > "${backup_dir}/manifest.json"

# ── The summary ────────────────────────────────────────────────────────────
total_bytes="$(jq '[.parts[].sizeBytes] | add // 0' "${backup_dir}/manifest.json")"
echo
echo "==> Backup complete: ${backup_dir}"
echo "    version:   ${version}"
echo "    created:   ${stamp}"
echo "    parts:     $(jq '.parts | length' "${backup_dir}/manifest.json")"
echo "    size:      ${total_bytes} bytes"
echo "    restore:   scripts/restore-from-backup.sh --local ${backup_dir}"
echo
echo "Note: this backup holds no release.json and no state.json — the release"
echo "      manifest and the installed-version state do not exist yet (REL-2 S2"
echo "      and S4). The same backup set will carry them."
