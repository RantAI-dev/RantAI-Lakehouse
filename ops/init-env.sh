#!/bin/sh
# SEC-18 -- write a local `.env` from `.env.example` with generated secrets.
#
# docker-compose.yml has no default credentials: it refuses to start while
# POSTGRES_PASSWORD, RUSTFS_ACCESS_KEY, RUSTFS_SECRET_KEY or
# LAKEKEEPER_ENCRYPTION_KEY is empty. This script is the one-line way to a
# working local stack:
#
#   sh ops/init-env.sh
#
# It never overwrites an existing `.env` (an install that already has one
# keeps its values), writes the file readable by its owner only, and prints
# the NAMES of the variables it filled, never a value. Values are random
# hex from `openssl rand` (or /dev/urandom when openssl is absent), which is
# safe inside the connection strings compose builds from them.
#
# AUTH_BOOTSTRAP_PASSWORD is generated too, so the first login works: read
# it from `.env` once, then change it in the console. The SeaweedFS keys are
# filled so `--profile seaweedfs` starts; they are unused otherwise.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
example="$root/.env.example"
target="$root/.env"

if [ -e "$target" ] || [ -L "$target" ]; then
  echo "init-env: $target already exists; refusing to overwrite it." >&2
  echo "init-env: move it away first if you want a fresh one." >&2
  exit 1
fi
if [ ! -f "$example" ]; then
  echo "init-env: $example not found." >&2
  exit 1
fi

gen() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex "$1"
  else
    od -An -N "$1" -tx1 /dev/urandom | tr -d ' \n'
    echo
  fi
}

umask 077
tmp=$(mktemp "$root/.env.init.XXXXXX")
trap 'rm -f "$tmp"' EXIT HUP INT TERM
cp "$example" "$tmp"

filled=""
for var in POSTGRES_PASSWORD RUSTFS_ACCESS_KEY RUSTFS_SECRET_KEY \
  LAKEKEEPER_ENCRYPTION_KEY SEAWEEDFS_ACCESS_KEY SEAWEEDFS_SECRET_KEY \
  AUTH_BOOTSTRAP_PASSWORD; do
  # Only an empty assignment is filled: a value already in the example is
  # never replaced. The value travels in the environment, not in argv, so
  # it does not show up in `ps`.
  if grep -q "^$var=\$" "$tmp"; then
    VALUE=$(gen 24) VAR=$var awk '
      BEGIN { name = ENVIRON["VAR"]; value = ENVIRON["VALUE"] }
      $0 == name "=" { print name "=" value; next }
      { print }
    ' "$tmp" > "$tmp.next"
    mv "$tmp.next" "$tmp"
    filled="$filled $var"
  fi
done

# Local dev only: the tenant warehouse pair may equal the RustFS pair (ADR 0002
# Addendum 2, "may equal, must never alias"), as .env.example used to say.
# Without it, tenant provisioning answers "not configured".
rk=$(sed -n 's/^RUSTFS_ACCESS_KEY=//p' "$tmp")
rs=$(sed -n 's/^RUSTFS_SECRET_KEY=//p' "$tmp")
TA=$rk TS=$rs awk '
  $0 == "TENANT_WAREHOUSE_S3_ACCESS_KEY=" { print $0 ENVIRON["TA"]; next }
  $0 == "TENANT_WAREHOUSE_S3_SECRET_KEY=" { print $0 ENVIRON["TS"]; next }
  { print }
' "$tmp" > "$tmp.next"
mv "$tmp.next" "$tmp"
filled="$filled TENANT_WAREHOUSE_S3_ACCESS_KEY TENANT_WAREHOUSE_S3_SECRET_KEY"

mv "$tmp" "$target"
trap - EXIT HUP INT TERM
echo "init-env: wrote $target (mode 600). Filled:"
for var in $filled; do
  echo "  $var"
done
echo "init-env: AUTH_BOOTSTRAP_EMAIL is still the example address; edit it in .env."
