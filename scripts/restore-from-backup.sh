#!/usr/bin/env bash
# Restore Lakehouse data from a backup.
#
# Two targets, one script:
#
# 1. The `backups` compose profile's S3 bucket (the default):
#    1. Postgres databases (application, Lakekeeper, OpenFGA, Dagster)
#    2. Warehouse S3 bucket (Iceberg data and metadata files)
#    3. ClickHouse non-catalog databases (ingest state, offsets, bronze_meta)
#    4. Bring up the remaining stack services
#
# 2. A LOCAL-disk backup directory (REL-2 S1-T4, decision D8) — what an
#    air-gapped host has, and what `ops/backups/backup-local.sh` writes:
#    1. The manifest's checksums are checked FIRST: a backup that was cut
#       short is refused, not restored as if it were whole.
#    2. Postgres (`pg_dumpall` of the whole instance)
#    3. ClickHouse (`BACKUP DATABASE ... TO Disk('backups', ...)` output)
#    4. The connector-secrets volume — every stored connector credential
#    5. The deployment's `.env` (the key the credentials are sealed with)
#    6. The release manifest and the install's state — the release the
#       data was taken from (REL-2 S4 decision D15)
#    7. Bring up the remaining stack services
#
# Usage:
#   scripts/restore-from-backup.sh [--force] [--skip-stack-up] <backup-timestamp>
#   scripts/restore-from-backup.sh [--force] [--skip-stack-up] --local <backup-dir>
#
# Example:
#   scripts/restore-from-backup.sh 20261016T090000Z
#   scripts/restore-from-backup.sh --force 20261016T090000Z
#   scripts/restore-from-backup.sh --local backups/20261031T090000Z-0.1.0
#
# Environment variables:
#   LAKEHOUSE_DIR          the deployment's own directory: the one
#                          holding `docker-compose.yml` and `.env`
#                          (REL-2 S4 decision D15). An installed host
#                          sets it to the install's own directory, and
#                          the script then runs `docker compose -p
#                          <project> -f
#                          "$LAKEHOUSE_DIR/docker-compose.yml"
#                          --env-file "$LAKEHOUSE_DIR/.env"`, plus
#                          `override.yml` when there is one. When it is
#                          NOT set, the script works on the repository
#                          it lives in and runs the command it always
#                          ran: `docker compose -p <project>` there,
#                          which also loads a developer's own
#                          `docker-compose.override.yml` (REL-2 S4
#                          review round 1 SHOULD-FIX 3).
#   COMPOSE_PROJECT          compose project name (pass -p or set here)
#   BACKUP_S3_ENDPOINT       backup S3 endpoint (e.g. http://rustfs:9000)
#   BACKUP_S3_BUCKET         backup bucket name (e.g. lakehouse-backups)
#   BACKUP_S3_ACCESS_KEY     backup S3 access key
#   BACKUP_S3_SECRET_KEY     backup S3 secret key
#   BACKUP_WAREHOUSE_ENDPOINT warehouse S3 endpoint (default: BACKUP_S3_ENDPOINT)
#   BACKUP_WAREHOUSE_BUCKET  warehouse bucket name (default: lakehouse-warehouse)
#   POSTGRES_USER            (default: lakehouse)
#   POSTGRES_PASSWORD        (default: from env)
#   APP_PG_DB                (default: lakehouse)
#   LAKEKEEPER_PG_DB         (default: lakekeeper)
#   OPENFGA_PG_DB            (default: openfga)
#   DAGSTER_PG_DB            (default: dagster)
#   CH_URL                   (default: http://clickhouse:8123)
#   CH_USER                  (default: default)
#   CH_PASSWORD              (default: empty)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# The deployment's own directory (REL-2 S4 decision D15). It is
# `LAKEHOUSE_DIR` when the variable is set — an installed host points
# it at the install's directory — and this repository's root when it is
# not: a developer who runs the script here gets the same behaviour as
# before (REL-2 S4 review round 1 SHOULD-FIX 3).
if [[ -n "${LAKEHOUSE_DIR:-}" ]]; then
  DEPLOYMENT_DIR="${LAKEHOUSE_DIR}"
else
  DEPLOYMENT_DIR="${REPO_ROOT}"
fi

# The alpine image the helper containers run — the same pinned image the
# stack itself uses (REL-2 S1-T3 pinned every image by digest).
HELPER_IMAGE="alpine:3.20@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc"

# Load .env if present and not already exported
if [[ -f "${DEPLOYMENT_DIR}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  . "${DEPLOYMENT_DIR}/.env"
  set +a
fi

FORCE=false
SKIP_STACK_UP=false
TIMESTAMP=""
LOCAL_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force)
      FORCE=true
      shift
      ;;
    --skip-stack-up)
      SKIP_STACK_UP=true
      shift
      ;;
    --local)
      LOCAL_DIR="${2:-}"
      if [[ -z "${LOCAL_DIR}" ]]; then
        echo "Error: --local needs the backup directory." >&2
        exit 1
      fi
      shift 2
      ;;
    --help|-h)
      echo "Usage: $0 [--force] [--skip-stack-up] <backup-timestamp>"
      echo "       $0 [--force] [--skip-stack-up] --local <backup-dir>"
      exit 0
      ;;
    -*)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
    *)
      if [[ -z "${TIMESTAMP}" ]]; then
        TIMESTAMP="$1"
      else
        echo "Unexpected argument: $1" >&2
        exit 1
      fi
      shift
      ;;
  esac
done

if [[ -z "${LOCAL_DIR}" && -z "${TIMESTAMP}" ]]; then
  echo "Usage: $0 [--force] [--skip-stack-up] <backup-timestamp>" >&2
  echo "       $0 [--force] [--skip-stack-up] --local <backup-dir>" >&2
  exit 1
fi

COMPOSE=(docker compose)
if [[ -n "${COMPOSE_PROJECT:-}" ]]; then
  COMPOSE=(docker compose -p "${COMPOSE_PROJECT}")
fi
# The Compose file of the deployment is named explicitly ONLY when the
# deployment is somewhere else: in this repository, compose finds the
# Compose file — and the developer's own `docker-compose.override.yml`
# — by itself, which is what it did before (REL-2 S4 review round 1
# SHOULD-FIX 3).
if [[ -n "${LAKEHOUSE_DIR:-}" ]]; then
  COMPOSE+=(-f "${DEPLOYMENT_DIR}/docker-compose.yml")
  # The `.env` is named only when it is there — a repository without one
  # is a normal thing, and compose would refuse to start on a file that
  # is not there.
  if [[ -f "${DEPLOYMENT_DIR}/.env" ]]; then
    COMPOSE+=(--env-file "${DEPLOYMENT_DIR}/.env")
  fi
  # The operator's own override, when there is one (REL-2 decision D5).
  if [[ -f "${DEPLOYMENT_DIR}/override.yml" ]]; then
    COMPOSE+=(-f "${DEPLOYMENT_DIR}/override.yml")
  fi
fi

resolve_ref() {
  local ref="${1:-}"
  if [[ "${ref}" =~ ^env:(.*)$ ]]; then
    local var="${BASH_REMATCH[1]}"
    echo "${!var:-}"
  else
    echo "${ref}"
  fi
}

BACKUP_S3_ENDPOINT="${BACKUP_S3_ENDPOINT:-${CH_RUSTFS_S3_ENDPOINT:-http://rustfs:9000}}"
BACKUP_S3_BUCKET="${BACKUP_S3_BUCKET:-lakehouse-backups}"
BACKUP_S3_ACCESS_KEY="${BACKUP_S3_ACCESS_KEY:-$(resolve_ref "${BACKUP_S3_ACCESS_KEY_SECRET_REF:-}")}"
BACKUP_S3_ACCESS_KEY="${BACKUP_S3_ACCESS_KEY:-${RUSTFS_ACCESS_KEY:-}}"
BACKUP_S3_SECRET_KEY="${BACKUP_S3_SECRET_KEY:-$(resolve_ref "${BACKUP_S3_SECRET_KEY_SECRET_REF:-}")}"
BACKUP_S3_SECRET_KEY="${BACKUP_S3_SECRET_KEY:-${RUSTFS_SECRET_KEY:-}}"

WAREHOUSE_ENDPOINT="${BACKUP_WAREHOUSE_ENDPOINT:-${CH_RUSTFS_S3_ENDPOINT:-http://rustfs:9000}}"
WAREHOUSE_BUCKET="${BACKUP_WAREHOUSE_BUCKET:-${LAKEHOUSE_WAREHOUSE_BUCKET:-lakehouse-warehouse}}"
WAREHOUSE_ACCESS_KEY="${BACKUP_WAREHOUSE_ACCESS_KEY:-$(resolve_ref "${BACKUP_WAREHOUSE_ACCESS_KEY_SECRET_REF:-}")}"
WAREHOUSE_ACCESS_KEY="${WAREHOUSE_ACCESS_KEY:-${RUSTFS_ACCESS_KEY:-}}"
WAREHOUSE_SECRET_KEY="${BACKUP_WAREHOUSE_SECRET_KEY:-$(resolve_ref "${BACKUP_WAREHOUSE_SECRET_KEY_SECRET_REF:-}")}"
WAREHOUSE_SECRET_KEY="${WAREHOUSE_SECRET_KEY:-${RUSTFS_SECRET_KEY:-}}"

POSTGRES_USER="${POSTGRES_USER:-lakehouse}"
APP_PG_DB="${APP_PG_DB:-lakehouse}"
LAKEKEEPER_PG_DB="${LAKEKEEPER_PG_DB:-lakekeeper}"
OPENFGA_PG_DB="${OPENFGA_PG_DB:-openfga}"
DAGSTER_PG_DB="${DAGSTER_PG_DB:-dagster}"

if [[ -z "${LOCAL_DIR}" && ( -z "${BACKUP_S3_ACCESS_KEY}" || -z "${BACKUP_S3_SECRET_KEY}" ) ]]; then
  echo "Error: BACKUP_S3_ACCESS_KEY and BACKUP_S3_SECRET_KEY must be set." >&2
  echo "       (A local-disk restore needs no bucket: use --local <backup-dir>.)" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d /tmp/restore-lakehouse.XXXXXX)"
cleanup() {
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

TOTAL_START=$(date +%s)
if [[ -n "${LOCAL_DIR}" ]]; then
  echo "================================================================="
  echo "Starting Lakehouse restore from local backup: ${LOCAL_DIR}"
  echo "================================================================="
else
  echo "================================================================="
  echo "Starting Lakehouse restore from backup timestamp: ${TIMESTAMP}"
  echo "================================================================="
fi

# Detect compose project network
NETWORK_NAME="${COMPOSE_PROJECT:-$(basename "${DEPLOYMENT_DIR}")}_default"
PROJECT_ID="${COMPOSE_PROJECT:-$(basename "${DEPLOYMENT_DIR}")}"
CONNECTOR_SECRETS_VOLUME="${PROJECT_ID}_lakehouse_connector_secrets"
CLICKHOUSE_BACKUPS_VOLUME="${PROJECT_ID}_lakehouse_clickhouse_backups"

# -------------------------------------------------------------------
# 0. Check empty volumes (E4: refuse to run on non-empty unless --force)
# -------------------------------------------------------------------
if [[ "${FORCE}" != "true" ]]; then
  echo "==> Verifying volumes / databases are empty..."
  VOLUMES=$(docker volume ls -q --filter "name=^${PROJECT_ID}_" 2>/dev/null || true)
  for vol in ${VOLUMES}; do
    # Check if data volume has existing files
    if [[ "${vol}" =~ (postgres_data|rustfs_data|clickhouse_data) ]]; then
      count=$(docker run --rm -v "${vol}:/v:ro" busybox sh -c 'ls -A /v 2>/dev/null | wc -l' 2>/dev/null || echo "0")
      if [[ "${count}" -gt 2 ]]; then
        echo "Error: volume '${vol}' is not empty (${count} files/dirs found)." >&2
        echo "A restore requires empty volumes to prevent partial overwrite corruption." >&2
        echo "Pass --force to override this check." >&2
        exit 1
      fi
    fi
  done
fi

# -------------------------------------------------------------------
# 0b. A LOCAL backup: check the manifest's checksums FIRST
# -------------------------------------------------------------------
# A backup that was cut short must not be restored as if it were whole.
# Every part's size and SHA-256 are checked before anything is written.
if [[ -n "${LOCAL_DIR}" ]]; then
  MANIFEST="${LOCAL_DIR}/manifest.json"
  if [[ ! -f "${MANIFEST}" ]]; then
    echo "Error: ${MANIFEST} is missing — this is not a local backup directory." >&2
    exit 1
  fi
  echo "==> Verifying the manifest's checksums..."
  CHECK_FAILURES=0
  while IFS=$'\t' read -r part part_size part_sum; do
    [[ -z "${part}" ]] && continue
    if [[ ! -f "${LOCAL_DIR}/${part}" ]]; then
      echo "Error: the part '${part}' is missing from the backup." >&2
      CHECK_FAILURES=$((CHECK_FAILURES + 1))
      continue
    fi
    actual_size="$(stat -c%s "${LOCAL_DIR}/${part}")"
    actual_sum="$(sha256sum "${LOCAL_DIR}/${part}" | cut -d' ' -f1)"
    if [[ "${actual_size}" != "${part_size}" ]]; then
      echo "Error: the part '${part}' is ${actual_size} bytes, the manifest says ${part_size}." >&2
      CHECK_FAILURES=$((CHECK_FAILURES + 1))
      continue
    fi
    if [[ "${actual_sum}" != "${part_sum}" ]]; then
      echo "Error: the part '${part}' does not match its SHA-256 (the backup is damaged)." >&2
      CHECK_FAILURES=$((CHECK_FAILURES + 1))
      continue
    fi
    echo "    ok: ${part} (${part_size} bytes)"
  done < <(jq -r '.parts[] | [.name, (.sizeBytes | tostring), .sha256] | @tsv' "${MANIFEST}")
  if [[ "${CHECK_FAILURES}" -gt 0 ]]; then
    echo "Error: ${CHECK_FAILURES} part(s) of this backup do not match the manifest." >&2
    echo "       Nothing was restored. A backup that does not match its manifest is" >&2
    echo "       not a backup — take a new one, or restore the damaged part by hand." >&2
    exit 1
  fi
  LOCAL_STAMP="$(jq -r '.createdAt' "${MANIFEST}")"
  LOCAL_VERSION="$(jq -r '.version' "${MANIFEST}")"
  echo "==> The backup is whole: version ${LOCAL_VERSION}, created ${LOCAL_STAMP}"
fi

# -------------------------------------------------------------------
# 0c. A LOCAL backup: restore it (and only it)
# -------------------------------------------------------------------
# The S3 path below is untouched: the bucket is still a target, this is
# another one. What differs is where the parts come from, and that this
# set carries two the bucket job never wrote — the connector secrets and
# the deployment's `.env`.
if [[ -n "${LOCAL_DIR}" ]]; then
  LOCAL_PG_START=$(date +%s)
  echo "==> [Step 1/5] Starting Postgres..."
  "${COMPOSE[@]}" up -d postgres
  echo "==> Waiting for Postgres to be healthy..."
  local_ready=false
  for _ in $(seq 1 60); do
    if "${COMPOSE[@]}" exec -T postgres pg_isready -U "${POSTGRES_USER}" >/dev/null 2>&1; then
      local_ready=true
      break
    fi
    sleep 1
  done
  if [[ "${local_ready}" != "true" ]]; then
    echo "Error: Postgres did not become ready within 60 s — nothing was restored." >&2
    exit 1
  fi

  echo "--> Restoring the whole instance from pg_dumpall..."
  # `pg_dumpall` restores into the `postgres` database: the dump carries
  # its own CREATE DATABASE and \connect statements. A role the dump
  # creates that already exists in the fresh instance is reported by psql
  # as an error and the restore carries on — expected, not a failure.
  "${COMPOSE[@]}" exec -T postgres psql -U "${POSTGRES_USER}" -d postgres \
    < "${LOCAL_DIR}/postgres.dump"
  LOCAL_PG_DURATION=$(( $(date +%s) - LOCAL_PG_START ))

  LOCAL_CH_START=$(date +%s)
  echo "==> [Step 2/5] Starting ClickHouse..."
  "${COMPOSE[@]}" up -d clickhouse
  echo "==> Waiting for ClickHouse to be healthy..."
  local_ready=false
  for _ in $(seq 1 60); do
    if "${COMPOSE[@]}" exec -T clickhouse clickhouse-client -q "SELECT 1" >/dev/null 2>&1; then
      local_ready=true
      break
    fi
    sleep 1
  done
  if [[ "${local_ready}" != "true" ]]; then
    echo "Error: ClickHouse did not become ready within 60 s — nothing was restored." >&2
    exit 1
  fi

  echo "--> Copying the ClickHouse backup back into ${CLICKHOUSE_BACKUPS_VOLUME}..."
  # The files belong to the ClickHouse server's own user (uid 101 in the
  # pinned image), which is who has to read them back.
  docker run --rm \
    -e STAMP="${LOCAL_STAMP}" \
    -v "${CLICKHOUSE_BACKUPS_VOLUME}:/dst" \
    -v "${LOCAL_DIR}:/src:ro" \
    "${HELPER_IMAGE}" \
    sh -c 'mkdir -p "/dst/clickhouse/${STAMP}" &&
           cp -a "/src/clickhouse/${STAMP}/." "/dst/clickhouse/${STAMP}/" &&
           chown -R 101:101 "/dst/clickhouse/${STAMP}"'

  for db_dir in "${LOCAL_DIR}/clickhouse/${LOCAL_STAMP}"/*; do
    [[ -d "${db_dir}" ]] || continue
    db="$(basename "${db_dir}")"
    quoted="${db//\`/\`\`}"
    echo "--> Restoring ClickHouse database '${db}'..."
    # A server that has just started answers a query before it opens the
    # listeners its own RESTORE machinery connects back through, so the
    # statement is retried for a while before it is called a failure.
    local_restored=false
    for attempt in 1 2 3 4 5 6; do
      if "${COMPOSE[@]}" exec -T clickhouse clickhouse-client -q \
        "RESTORE DATABASE \`${quoted}\` FROM Disk('backups', 'clickhouse/${LOCAL_STAMP}/${quoted}')"; then
        local_restored=true
        break
      fi
      echo "    attempt ${attempt} failed; waiting 5 s and trying again"
      sleep 5
    done
    if [[ "${local_restored}" != "true" ]]; then
      echo "Error: the ClickHouse database '${db}' could not be restored." >&2
      exit 1
    fi
  done
  LOCAL_CH_DURATION=$(( $(date +%s) - LOCAL_CH_START ))

  LOCAL_SECRETS_START=$(date +%s)
  if [[ -f "${LOCAL_DIR}/connector-secrets.tar.gz" ]]; then
    echo "==> [Step 3/5] Restoring the connector secrets into ${CONNECTOR_SECRETS_VOLUME}..."
    docker run --rm \
      -v "${CONNECTOR_SECRETS_VOLUME}:/secrets" \
      -v "${LOCAL_DIR}:/src:ro" \
      "${HELPER_IMAGE}" \
      tar xzf /src/connector-secrets.tar.gz -C /secrets
  else
    echo "==> [Step 3/5] This backup holds no connector secrets — skipping."
  fi
  LOCAL_SECRETS_DURATION=$(( $(date +%s) - LOCAL_SECRETS_START ))

  LOCAL_ENV_START=$(date +%s)
  if [[ -f "${LOCAL_DIR}/env" ]]; then
    echo "==> [Step 4/5] Restoring the deployment's .env..."
    # The connector credentials are sealed with the key in this file: a
    # restore whose `.env` is not the one they were sealed with leaves
    # every connector unreadable. The current `.env` is kept beside the
    # new one, so a restore never destroys the only copy.
    if [[ -f "${DEPLOYMENT_DIR}/.env" ]] && ! cmp -s "${LOCAL_DIR}/env" "${DEPLOYMENT_DIR}/.env"; then
      cp -p "${DEPLOYMENT_DIR}/.env" "${DEPLOYMENT_DIR}/.env.before-restore-${LOCAL_STAMP}"
      echo "    the previous .env was kept as .env.before-restore-${LOCAL_STAMP}"
    fi
    cp -p "${LOCAL_DIR}/env" "${DEPLOYMENT_DIR}/.env"
  else
    echo "==> [Step 4/5] This backup holds no .env — skipping."
  fi
  LOCAL_ENV_DURATION=$(( $(date +%s) - LOCAL_ENV_START ))

  # The release manifest and the install's state: they are part of the
  # backup set (REL-2 S4 decision D15), so the restore puts back the
  # release the data came from. The current ones are kept beside the
  # new ones, so a restore never destroys the only copy.
  for release_file in release.json state.json; do
    if [[ -f "${LOCAL_DIR}/${release_file}" ]]; then
      echo "--> Restoring ${release_file}..."
      if [[ -f "${DEPLOYMENT_DIR}/${release_file}" ]] \
         && ! cmp -s "${LOCAL_DIR}/${release_file}" "${DEPLOYMENT_DIR}/${release_file}"; then
        cp -p "${DEPLOYMENT_DIR}/${release_file}" \
          "${DEPLOYMENT_DIR}/${release_file}.before-restore-${LOCAL_STAMP}"
        echo "    the previous ${release_file} was kept as ${release_file}.before-restore-${LOCAL_STAMP}"
      fi
      cp -p "${LOCAL_DIR}/${release_file}" "${DEPLOYMENT_DIR}/${release_file}"
    else
      echo "--> This backup holds no ${release_file} — skipping."
    fi
  done

  LOCAL_STACK_DURATION=0
  if [[ "${SKIP_STACK_UP}" != "true" ]]; then
    echo "==> [Step 5/5] Starting the remaining services in the stack..."
    LOCAL_STACK_START=$(date +%s)
    "${COMPOSE[@]}" up -d
    LOCAL_STACK_DURATION=$(( $(date +%s) - LOCAL_STACK_START ))
  else
    echo "==> [Step 5/5] Skipping full stack up (--skip-stack-up passed)"
  fi

  TOTAL_DURATION=$(( $(date +%s) - TOTAL_START ))
  echo "================================================================="
  echo "Restore complete from the local backup ${LOCAL_DIR}"
  echo "Timing summary:"
  echo "  Postgres restore:        ${LOCAL_PG_DURATION}s"
  echo "  ClickHouse restore:      ${LOCAL_CH_DURATION}s"
  echo "  Connector secrets:       ${LOCAL_SECRETS_DURATION}s"
  echo "  Configuration:           ${LOCAL_ENV_DURATION}s"
  echo "  Stack bring-up:          ${LOCAL_STACK_DURATION}s"
  echo "  Total restore time:      ${TOTAL_DURATION}s"
  echo "================================================================="
  exit 0
fi

# -------------------------------------------------------------------
# 1. Postgres restore (alone first)
# -------------------------------------------------------------------
echo "==> [Step 1/4] Starting Postgres..."
PG_START=$(date +%s)
"${COMPOSE[@]}" up -d postgres

echo "==> Waiting for Postgres to be healthy..."
for _ in $(seq 1 60); do
  if "${COMPOSE[@]}" exec -T postgres pg_isready -U "${POSTGRES_USER}" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

# The four databases backed up by backup_job.py
DATABASES=("${APP_PG_DB}" "${LAKEKEEPER_PG_DB}" "${OPENFGA_PG_DB}" "${DAGSTER_PG_DB}")
TOTAL_PG_BYTES=0

for db in "${DATABASES[@]}"; do
  dump_file="${TMP_DIR}/${db}.dump"
  echo "--> Downloading dump for database '${db}' from s3://${BACKUP_S3_BUCKET}/${db}/${TIMESTAMP}.dump"
  AWS_ACCESS_KEY_ID="${BACKUP_S3_ACCESS_KEY}" \
  AWS_SECRET_ACCESS_KEY="${BACKUP_S3_SECRET_KEY}" \
  docker run --rm --network "${NETWORK_NAME}" \
    -v "${TMP_DIR}:/out" \
    -e AWS_ACCESS_KEY_ID \
    -e AWS_SECRET_ACCESS_KEY \
    -e AWS_DEFAULT_REGION=us-east-1 \
    -e AWS_EC2_METADATA_DISABLED=true \
    amazon/aws-cli:2.36.34 s3 cp \
    "s3://${BACKUP_S3_BUCKET}/${db}/${TIMESTAMP}.dump" \
    "/out/${db}.dump" \
    --endpoint-url "${BACKUP_S3_ENDPOINT}" >/dev/null

  if [[ ! -s "${dump_file}" ]]; then
    echo "Error: dump file for '${db}' is missing or empty" >&2
    exit 1
  fi

  db_size=$(stat -c%s "${dump_file}")
  TOTAL_PG_BYTES=$((TOTAL_PG_BYTES + db_size))

  echo "--> Restoring database '${db}' (${db_size} bytes)..."
  # Ensure target database exists
  db_exists=$("${COMPOSE[@]}" exec -T postgres \
    psql -U "${POSTGRES_USER}" -d postgres -tc \
    "SELECT 1 FROM pg_database WHERE datname = '${db}'" | tr -d ' ' || true)
  if [[ "${db_exists}" != "1" ]]; then
    "${COMPOSE[@]}" exec -T postgres createdb -U "${POSTGRES_USER}" "${db}"
  fi

  # Restore with --clean --if-exists --no-owner
  "${COMPOSE[@]}" exec -T postgres \
    pg_restore -U "${POSTGRES_USER}" -d "${db}" --clean --if-exists --no-owner \
    < "${dump_file}" || true

  rm -f "${dump_file}"
done

PG_DURATION=$(( $(date +%s) - PG_START ))
echo "==> [Step 1/4] Postgres restore complete: 4 databases (${TOTAL_PG_BYTES} bytes) in ${PG_DURATION}s"

# -------------------------------------------------------------------
# 2. Warehouse S3 bucket restore
# -------------------------------------------------------------------
echo "==> [Step 2/4] Starting RustFS..."
WH_START=$(date +%s)
"${COMPOSE[@]}" up -d rustfs rustfs-bucket-init

echo "==> Waiting for RustFS to be healthy..."
for _ in $(seq 1 60); do
  if AWS_ACCESS_KEY_ID="${WAREHOUSE_ACCESS_KEY}" \
     AWS_SECRET_ACCESS_KEY="${WAREHOUSE_SECRET_KEY}" \
     docker run --rm --network "${NETWORK_NAME}" \
       -e AWS_ACCESS_KEY_ID \
       -e AWS_SECRET_ACCESS_KEY \
    -e AWS_DEFAULT_REGION=us-east-1 \
    -e AWS_EC2_METADATA_DISABLED=true \
    amazon/aws-cli:2.36.34 s3 ls "s3://${WAREHOUSE_BUCKET}" \
    --endpoint-url "${WAREHOUSE_ENDPOINT}" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

echo "--> Restoring warehouse bucket from s3://${BACKUP_S3_BUCKET}/warehouse/ to s3://${WAREHOUSE_BUCKET}/"
if [[ "${BACKUP_S3_ENDPOINT}" == "${WAREHOUSE_ENDPOINT}" && "${BACKUP_S3_ACCESS_KEY}" == "${WAREHOUSE_ACCESS_KEY}" ]]; then
  AWS_ACCESS_KEY_ID="${BACKUP_S3_ACCESS_KEY}" \
  AWS_SECRET_ACCESS_KEY="${BACKUP_S3_SECRET_KEY}" \
  docker run --rm --network "${NETWORK_NAME}" \
    -e AWS_ACCESS_KEY_ID \
    -e AWS_SECRET_ACCESS_KEY \
    -e AWS_DEFAULT_REGION=us-east-1 \
    -e AWS_EC2_METADATA_DISABLED=true \
    amazon/aws-cli:2.36.34 s3 sync \
    "s3://${BACKUP_S3_BUCKET}/warehouse" \
    "s3://${WAREHOUSE_BUCKET}" \
    --endpoint-url "${BACKUP_S3_ENDPOINT}"
else
  mkdir -p "${TMP_DIR}/warehouse"
  AWS_ACCESS_KEY_ID="${BACKUP_S3_ACCESS_KEY}" \
  AWS_SECRET_ACCESS_KEY="${BACKUP_S3_SECRET_KEY}" \
  docker run --rm --network "${NETWORK_NAME}" \
    -v "${TMP_DIR}/warehouse:/wh" \
    -e AWS_ACCESS_KEY_ID \
    -e AWS_SECRET_ACCESS_KEY \
    -e AWS_DEFAULT_REGION=us-east-1 \
    -e AWS_EC2_METADATA_DISABLED=true \
    amazon/aws-cli:2.36.34 s3 sync \
    "s3://${BACKUP_S3_BUCKET}/warehouse" \
    "/wh" \
    --endpoint-url "${BACKUP_S3_ENDPOINT}"

  AWS_ACCESS_KEY_ID="${WAREHOUSE_ACCESS_KEY}" \
  AWS_SECRET_ACCESS_KEY="${WAREHOUSE_SECRET_KEY}" \
  docker run --rm --network "${NETWORK_NAME}" \
    -v "${TMP_DIR}/warehouse:/wh" \
    -e AWS_ACCESS_KEY_ID \
    -e AWS_SECRET_ACCESS_KEY \
    -e AWS_DEFAULT_REGION=us-east-1 \
    -e AWS_EC2_METADATA_DISABLED=true \
    amazon/aws-cli:2.36.34 s3 sync \
    "/wh" \
    "s3://${WAREHOUSE_BUCKET}" \
    --endpoint-url "${WAREHOUSE_ENDPOINT}"
  rm -rf "${TMP_DIR}/warehouse"
fi

WH_DURATION=$(( $(date +%s) - WH_START ))
echo "==> [Step 2/4] Warehouse restore complete in ${WH_DURATION}s"

# -------------------------------------------------------------------
# 3. ClickHouse restore
# -------------------------------------------------------------------
echo "==> [Step 3/4] Starting ClickHouse..."
CH_START=$(date +%s)
"${COMPOSE[@]}" up -d clickhouse

echo "==> Waiting for ClickHouse to be healthy..."
for _ in $(seq 1 60); do
  if "${COMPOSE[@]}" exec -T clickhouse clickhouse-client -q "SELECT 1" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

echo "--> Discovering ClickHouse databases backed up at timestamp ${TIMESTAMP}..."
CH_DBS=$(AWS_ACCESS_KEY_ID="${BACKUP_S3_ACCESS_KEY}" \
  AWS_SECRET_ACCESS_KEY="${BACKUP_S3_SECRET_KEY}" \
  docker run --rm --network "${NETWORK_NAME}" \
  -e AWS_ACCESS_KEY_ID \
  -e AWS_SECRET_ACCESS_KEY \
  -e AWS_DEFAULT_REGION=us-east-1 \
  -e AWS_EC2_METADATA_DISABLED=true \
  amazon/aws-cli:2.36.34 s3 ls \
  "s3://${BACKUP_S3_BUCKET}/clickhouse/${TIMESTAMP}/" \
  --endpoint-url "${BACKUP_S3_ENDPOINT}" | awk '/PRE / {print $2}' | tr -d '/')

if [[ -n "${CH_DBS}" ]]; then
  for db in ${CH_DBS}; do
    echo "--> Restoring ClickHouse database '${db}'..."
    "${COMPOSE[@]}" exec -T clickhouse clickhouse-client -q "DROP DATABASE IF EXISTS \`${db}\`;"
    s3_path="${BACKUP_S3_ENDPOINT%/}/${BACKUP_S3_BUCKET}/clickhouse/${TIMESTAMP}/${db}/"
    "${COMPOSE[@]}" exec -T clickhouse clickhouse-client -q \
      "RESTORE DATABASE \`${db}\` FROM S3('${s3_path}', '${BACKUP_S3_ACCESS_KEY}', '${BACKUP_S3_SECRET_KEY}');"
  done
else
  echo "--> Note: No ClickHouse databases found under s3://${BACKUP_S3_BUCKET}/clickhouse/${TIMESTAMP}/"
fi

CH_DURATION=$(( $(date +%s) - CH_START ))
echo "==> [Step 3/4] ClickHouse restore complete in ${CH_DURATION}s"

# -------------------------------------------------------------------
# 4. Bring up the rest of the stack
# -------------------------------------------------------------------
STACK_DURATION=0
if [[ "${SKIP_STACK_UP}" != "true" ]]; then
  echo "==> [Step 4/4] Starting the remaining services in the stack..."
  STACK_START=$(date +%s)
  "${COMPOSE[@]}" up -d
  STACK_DURATION=$(( $(date +%s) - STACK_START ))
  echo "==> [Step 4/4] Remaining services brought up in ${STACK_DURATION}s"
else
  echo "==> [Step 4/4] Skipping full stack up (--skip-stack-up passed)"
fi

TOTAL_DURATION=$(( $(date +%s) - TOTAL_START ))
echo "================================================================="
echo "Restore complete for backup ${TIMESTAMP}"
echo "Timing summary:"
echo "  Postgres restore:   ${PG_DURATION}s"
echo "  Warehouse restore:  ${WH_DURATION}s"
echo "  ClickHouse restore: ${CH_DURATION}s"
echo "  Stack bring-up:     ${STACK_DURATION}s"
echo "  Total restore time: ${TOTAL_DURATION}s"
echo "================================================================="
