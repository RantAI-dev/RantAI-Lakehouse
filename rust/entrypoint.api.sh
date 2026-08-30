#!/bin/sh
# Entrypoint for the rust/Dockerfile.api image.
#
# `lakehouse-api`'s own `main.rs` never calls `lakehouse_store::migrate`
# (that function only gets invoked from the test harness — see
# `rust/crates/lakehouse-api/tests/common/mod.rs`) — the shipped binary
# assumes migrations were already applied by something outside the
# process. In a genuinely clean `docker compose up`, nothing else does
# that, so the API boots against a schema-less database and the bootstrap
# admin seed (which runs unconditionally on startup, see `main.rs`'s
# `bootstrap_admin`) fails with a generic "database error" because
# `users` doesn't exist yet.
#
# This is a real gap in the application, not something specific to this
# Dockerfile — but `main.rs` is out of scope to edit here (see
# docs/OPERATIONS.md and the branch's in-progress work by another
# author), so the fix lives at the container boundary instead: run `sqlx
# migrate run` here, once, before exec'ing the API binary. `sqlx-cli` is
# installed into this image at the same pinned version as the workspace's
# `sqlx` dependency (see rust/Dockerfile.api) specifically so its
# migration behavior matches what's compiled into the API.
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
