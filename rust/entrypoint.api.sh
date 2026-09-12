#!/bin/sh
# Entrypoint for the rust/Dockerfile image.
#
# `main.rs` now applies migrations at boot via `lakehouse_store::migrate`
# (idempotent — `sqlx` records applied versions in `_sqlx_migrations`), but
# a failure there is logged rather than fatal, matching the "Postgres-down
# is quiet" posture the rest of the app takes (see `lakehouse-store`'s
# `connect_lazy` doc comment). Running `sqlx migrate run` here too, before
# exec'ing the API binary, is belt-and-suspenders: it applies the schema
# via a second, independent path in case the in-process migration was
# skipped or failed, so the bootstrap admin seed (which runs
# unconditionally on startup, see `main.rs`'s `bootstrap_admin`) doesn't
# fail with a generic "database error" because `users` doesn't exist yet.
# `sqlx-cli` is installed into this image at the same pinned version as
# the workspace's `sqlx` dependency (see `rust/Dockerfile`) specifically
# so its migration behavior matches what's compiled into the API.
#
# If DATABASE_URL is unset/unparseable, skip straight to starting the API
# — lakehouse-api is designed to boot and serve its Postgres-independent
# routes even without a database (see lakehouse-store's `connect_lazy`
# doc comment), and this entrypoint should not be stricter than the
# process it wraps.
set -eu

if [ -n "${DATABASE_URL:-}" ]; then
    echo "entrypoint: applying migrations via sqlx-cli..." >&2
    if sqlx migrate run --source /app/migrations --database-url "$DATABASE_URL"; then
        echo "entrypoint: migrations applied" >&2
    else
        echo "entrypoint: migration step failed; starting API anyway (it will surface its own Postgres errors per-request)" >&2
    fi
else
    echo "entrypoint: DATABASE_URL not set; skipping migrations" >&2
fi

exec /usr/local/bin/lakehouse-api
