#!/bin/sh
# Entrypoint for the rust/Dockerfile image.
#
# The API NEVER migrates. The schema is applied by the `migrate`
# subcommand of this same binary, which the one-shot `lakehouse-db-migrate`
# compose service runs with the database OWNER's credentials: a console
# that owns a table can switch off the audit trail's append-only triggers
# (`ALTER TABLE audit_event DISABLE TRIGGER`, `SET
# session_replication_role = replica`), so the role that applies the
# schema must not be the role the console runs as. The API's boot path
# checks the migration state instead of applying it, and fails closed on a
# pending one.
#
# `sqlx-cli` is still in this image (see `rust/Dockerfile`), but nothing
# in it runs the tool any more.
set -eu

# SRC-12 decision 5: `docker compose run --rm lakehouse-api rotate-secrets-key`
# passes a command through to the binary. A command is a one-off task, not a
# server start, and the binary decides what it accepts (it refuses anything
# it does not know). The same path carries the `migrate` subcommand the
# `lakehouse-db-migrate` service runs: it waits for the database and applies
# the migrations.
if [ "$#" -gt 0 ]; then
    exec /usr/local/bin/lakehouse-api "$@"
fi

exec /usr/local/bin/lakehouse-api
