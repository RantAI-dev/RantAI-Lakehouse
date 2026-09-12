#!/bin/sh
# Drop a Postgres logical-replication slot and publication left behind by a
# removed CDC connector — the operational half of R5's mitigation
# (`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md`'s risk register: "a stuck or
# lagging replication slot pins WAL and fills the customer's production
# database disk").
#
# `DELETE /api/connectors/{id}` (see
# `lakehouse_store::connectors::delete_connector`'s doc comment) does NOT
# call this — it only removes the registry row, because the Rust API has no
# mechanism to safely resolve an arbitrary customer's `secretRef` and dial
# an arbitrary `host` from inside the product process (see ADR 0007's "what
# P5 does NOT do"). This script is the concrete, tested mechanism a real
# deprovisioning flow would call; `ops/g4/g4_test.py` runs it directly
# against the demo CDC connector to prove slot cleanup actually works, per
# G4's acceptance criterion.
#
# Usage: deprovision_connector.sh <connector_slug>
# Requires: psql on PATH, PGHOST/PGPORT/PGUSER/PGPASSWORD/PGDATABASE (or a
# full libpq connection string via PGSERVICE/PGURL-style env) already set
# in the environment — this script does not itself resolve a secretRef.

set -eu

SLUG="${1:?usage: deprovision_connector.sh <connector_slug>}"
SLOT="${SLUG}_slot"
PUB="${SLUG}_pub"

echo "[deprovision] dropping publication '${PUB}' and replication slot '${SLOT}' for connector '${SLUG}'"

# The publication can be dropped regardless of slot state.
psql -v ON_ERROR_STOP=1 -c "DROP PUBLICATION IF EXISTS \"${PUB}\";"

# A slot must be inactive before it can be dropped — if a Debezium Server
# process for this connector is still consuming it, terminate that
# backend's replication connection first (this is the "the process must
# stop before the slot can go" ordering every Postgres logical-replication
# deprovisioning flow needs, not something specific to this build).
psql -v ON_ERROR_STOP=1 -tc "
  SELECT pg_terminate_backend(active_pid)
  FROM pg_replication_slots
  WHERE slot_name = '${SLOT}' AND active_pid IS NOT NULL;
" >/dev/null || true

# Give Postgres a moment to mark the slot inactive after terminating the
# backend above, then drop it. Idempotent: dropping an already-absent slot
# is reported, not treated as fatal.
for _ in 1 2 3 4 5; do
  ACTIVE=$(psql -v ON_ERROR_STOP=1 -tc "SELECT active FROM pg_replication_slots WHERE slot_name = '${SLOT}';" | tr -d '[:space:]')
  if [ "$ACTIVE" != "t" ]; then
    break
  fi
  sleep 1
done

EXISTS=$(psql -v ON_ERROR_STOP=1 -tc "SELECT 1 FROM pg_replication_slots WHERE slot_name = '${SLOT}';" | tr -d '[:space:]')
if [ "$EXISTS" = "1" ]; then
  psql -v ON_ERROR_STOP=1 -c "SELECT pg_drop_replication_slot('${SLOT}');"
  echo "[deprovision] dropped slot '${SLOT}'"
else
  echo "[deprovision] slot '${SLOT}' did not exist — nothing to drop"
fi
