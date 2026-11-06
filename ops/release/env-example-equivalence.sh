#!/bin/sh
# D1 proof for REL-19: two `.env.example` files give the same stack.
#
#   sh ops/release/env-example-equivalence.sh <old-env-example> <new-env-example>
#
# Reads `docker compose --profile '*' config` twice: once with the old
# template and once with the new one. Both runs get the SAME fixed dummy
# values for the variables compose refuses to start without (`${X:?}`),
# so a difference in the output can only come from a setting the two
# templates resolve differently. Exit 0 when the outputs are identical;
# print the unified diff and exit 1 otherwise.
#
# The plan's rule D1 is the reason this exists: removing a line from
# `.env.example` must not change the value a fresh install gets. The one
# intended difference (`CONSOLE_INTERNAL_URL`, D4) is therefore a diff
# this script reports; T3 accepts exactly that one and nothing else.
#
# The dummy values are generated here, identical for both sides, and are
# never written anywhere a real deployment reads: they live in a
# temporary directory that is removed on exit.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <old-env-example> <new-env-example>" >&2
  exit 2
fi

old=$1
new=$2
for file in "$old" "$new"; do
  if [ ! -f "$file" ]; then
    echo "env-example-equivalence: $file not found." >&2
    exit 2
  fi
done

work=$(mktemp -d "${TMPDIR:-/tmp}/rel19-equivalence.XXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM

# The five settings compose reads with `${X:?}`. The values are fixed so
# the two configs are compared on what the templates decide, not on a
# random secret; a line the template already sets is replaced, so both
# sides start from the same value.
DUMMY_POSTGRES_PASSWORD=rel19-equivalence-postgres
DUMMY_RUSTFS_ACCESS_KEY=rel19-equivalence-access
DUMMY_RUSTFS_SECRET_KEY=rel19-equivalence-secret
DUMMY_LAKEKEEPER_ENCRYPTION_KEY=rel19-equivalence-lakekeeper
DUMMY_CONNECTOR_SECRETS_KEY=cmVsMTktZXF1aXZhbGVuY2UtY29ubmVjdG9yLWtleQ==
export DUMMY_POSTGRES_PASSWORD DUMMY_RUSTFS_ACCESS_KEY \
  DUMMY_RUSTFS_SECRET_KEY DUMMY_LAKEKEEPER_ENCRYPTION_KEY \
  DUMMY_CONNECTOR_SECRETS_KEY

prepare() {
  source_file=$1
  target_file=$2
  cp "$source_file" "$target_file"
  for name in POSTGRES_PASSWORD RUSTFS_ACCESS_KEY RUSTFS_SECRET_KEY \
    LAKEKEEPER_ENCRYPTION_KEY CONNECTOR_SECRETS_KEY; do
    eval "value=\${DUMMY_$name}"
    grep -v "^$name=" "$target_file" > "$target_file.next" || true
    printf '%s=%s\n' "$name" "$value" >> "$target_file.next"
    mv "$target_file.next" "$target_file"
  done
}

prepare "$old" "$work/old.env"
prepare "$new" "$work/new.env"

# One project name for both runs: `config` prints the project name, and a
# different name per side would be the only difference.
render() {
  env_file=$1
  output=$2
  (
    cd "$root"
    docker compose \
      --project-name rel19-env-equivalence \
      --env-file "$env_file" \
      --profile '*' \
      config > "$output"
  )
}

render "$work/old.env" "$work/old.yml"
render "$work/new.env" "$work/new.yml"

if diff -u "$work/old.yml" "$work/new.yml" > "$work/config.diff"; then
  echo "env-example-equivalence: identical ($old vs $new)"
  exit 0
fi

echo "env-example-equivalence: the two templates do not resolve the same:" >&2
cat "$work/config.diff" >&2
exit 1
