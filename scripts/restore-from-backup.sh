#!/usr/bin/env bash
# Restore Lakehouse data from a backup created by the `backup-job` service:
# 1. Postgres databases (application, Lakekeeper, OpenFGA, Dagster)
# 2. Warehouse S3 bucket (Iceberg data and metadata files)
# 3. ClickHouse non-catalog databases (ingest state, offsets, bronze_meta)
# 4. Bring up the remaining stack services
#
# Usage:
#   scripts/restore-from-backup.sh [--force] [--skip-stack-up] <backup-timestamp>
#
# Example:
#   scripts/restore-from-backup.sh 20261016T090000Z
#   scripts/restore-from-backup.sh --force 20261016T090000Z
#
# Environment variables:
#   COMPOSE_PROJECT          compose project name (pass -p or set here)
#   BACKUP_S3_ENDPOINT       backup S3 endpoint (e.g. http://rustfs:9000)
#   BACKUP_S3_BUCKET         backup bucket name (e.g. lakehouse-backups)
#   BACKUP_S3_ACCESS_KEY     backup S3 access key
#   BACKUP_S3_SECRET_KEY     backup S3 secret key
#   BACKUP_WAREHOUSE_ENDPOINT warehouse S3 endpoint (default: BACKUP_S3_ENDPOINT)
#   BACKUP_WAREHOUSE_BUCKET  warehouse S3 bucket (default: lakehouse-warehouse)
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

# Load .env if present and not already exported
if [[ -f "${REPO_ROOT}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  . "${REPO_ROOT}/.env"
  set +a
fi

FORCE=false
SKIP_STACK_UP=false
TIMESTAMP=""

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
    --help|-h)
      echo "Usage: $0 [--force] [--skip-stack-up] <backup-timestamp>"
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

if [[ -z "${TIMESTAMP}" ]]; then
  echo "Usage: $0 [--force] [--skip-stack-up] <backup-timestamp>" >&2
  exit 1
fi

COMPOSE=(docker compose)
if [[ -n "${COMPOSE_PROJECT:-}" ]]; then
  COMPOSE=(docker compose -p "${COMPOSE_PROJECT}")
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

if [[ -z "${BACKUP_S3_ACCESS_KEY}" || -z "${BACKUP_S3_SECRET_KEY}" ]]; then
  echo "Error: BACKUP_S3_ACCESS_KEY and BACKUP_S3_SECRET_KEY must be set." >&2
  exit 1
fi

TMP_DIR="$(mktemp -d /tmp/restore-lakehouse.XXXXXX)"
cleanup() {
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

TOTAL_START=$(date +%s)
echo "================================================================="
echo "Starting Lakehouse restore from backup timestamp: ${TIMESTAMP}"
echo "================================================================="

# Detect compose project network
NETWORK_NAME="${COMPOSE_PROJECT:-$(basename "${REPO_ROOT}")}_default"

# -------------------------------------------------------------------
# 0. Check empty volumes (E4: refuse to run on non-empty unless --force)
# -------------------------------------------------------------------
if [[ "${FORCE}" != "true" ]]; then
  echo "==> Verifying volumes / databases are empty..."
  PROJECT_ID="${COMPOSE_PROJECT:-$(basename "${REPO_ROOT}")}"
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
  docker run --rm --network "${NETWORK_NAME}" \
    -v "${TMP_DIR}:/out" \
    -e AWS_ACCESS_KEY_ID="${BACKUP_S3_ACCESS_KEY}" \
    -e AWS_SECRET_ACCESS_KEY="${BACKUP_S3_SECRET_KEY}" \
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
  if docker run --rm --network "${NETWORK_NAME}" \
    -e AWS_ACCESS_KEY_ID="${WAREHOUSE_ACCESS_KEY}" \
    -e AWS_SECRET_ACCESS_KEY="${WAREHOUSE_SECRET_KEY}" \
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
  docker run --rm --network "${NETWORK_NAME}" \
    -e AWS_ACCESS_KEY_ID="${BACKUP_S3_ACCESS_KEY}" \
    -e AWS_SECRET_ACCESS_KEY="${BACKUP_S3_SECRET_KEY}" \
    -e AWS_DEFAULT_REGION=us-east-1 \
    -e AWS_EC2_METADATA_DISABLED=true \
    amazon/aws-cli:2.36.34 s3 sync \
    "s3://${BACKUP_S3_BUCKET}/warehouse" \
    "s3://${WAREHOUSE_BUCKET}" \
    --endpoint-url "${BACKUP_S3_ENDPOINT}"
else
  mkdir -p "${TMP_DIR}/warehouse"
  docker run --rm --network "${NETWORK_NAME}" \
    -v "${TMP_DIR}/warehouse:/wh" \
    -e AWS_ACCESS_KEY_ID="${BACKUP_S3_ACCESS_KEY}" \
    -e AWS_SECRET_ACCESS_KEY="${BACKUP_S3_SECRET_KEY}" \
    -e AWS_DEFAULT_REGION=us-east-1 \
    -e AWS_EC2_METADATA_DISABLED=true \
    amazon/aws-cli:2.36.34 s3 sync \
    "s3://${BACKUP_S3_BUCKET}/warehouse" \
    "/wh" \
    --endpoint-url "${BACKUP_S3_ENDPOINT}"

  docker run --rm --network "${NETWORK_NAME}" \
    -v "${TMP_DIR}/warehouse:/wh" \
    -e AWS_ACCESS_KEY_ID="${WAREHOUSE_ACCESS_KEY}" \
    -e AWS_SECRET_ACCESS_KEY="${WAREHOUSE_SECRET_KEY}" \
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
CH_DBS=$(docker run --rm --network "${NETWORK_NAME}" \
  -e AWS_ACCESS_KEY_ID="${BACKUP_S3_ACCESS_KEY}" \
  -e AWS_SECRET_ACCESS_KEY="${BACKUP_S3_SECRET_KEY}" \
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
