# Build for lakehouse-api, referenced by docker-compose.yml.
#
# This file used to diverge from `rust/Dockerfile.api`, which fixed the same
# clean-clone build problem in a different (and better) way; the two have
# since been converged onto this single file, and `Dockerfile.api` is gone.
#
# Base image pin: `rust:1.85-slim` (this file's old pin) cannot build this
# workspace — `Cargo.lock` pulls in `time@0.3.55`, which requires rustc
# >= 1.88. Rather than guessing the minimum that happens to satisfy today's
# lockfile, this pins the exact toolchain the workspace already commits to
# in `rust-toolchain.toml` (`channel = "1.96.1"`), so the Docker build and
# local dev/CI builds are guaranteed to use the same compiler.
FROM rust:1.96.1-slim AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# `sqlx::migrate!("../../migrations")` (lakehouse-store/src/lib.rs) embeds
# the migration SQL into the binary at *compile* time, so the directory
# must exist in the build context before `cargo build` runs, or the build
# fails outright.
COPY migrations ./migrations
RUN cargo build --release --locked -p lakehouse-api
# `sqlx-cli`, pinned to the same version as the workspace's `sqlx`
# dependency, for the migration step in entrypoint.api.sh. `main.rs` now
# also applies migrations at boot via `lakehouse_store::migrate` (see
# `main.rs`), so this is belt-and-suspenders: whichever runs first wins,
# `sqlx migrate run` is idempotent, and neither path is fatal on failure.
RUN cargo install sqlx-cli --version 0.8.6 --no-default-features --features rustls,postgres --locked

# `debian:trixie-slim`, not `bookworm-slim`: `rust:1.96.1-slim` builds on
# Debian 13 (trixie), whose glibc is newer than bookworm's. A bookworm
# runtime image fails to even exec the trixie-built `sqlx-cli` binary
# (`GLIBC_2.39' not found`) — matching the base OS avoids a glibc skew
# between the two stages.
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/lakehouse-api /usr/local/bin/lakehouse-api
COPY --from=builder /usr/local/cargo/bin/sqlx /usr/local/bin/sqlx
COPY migrations ./migrations
# WS4 (grand plan §6): the API image carries a read-only copy of the
# Dagster code location's own source, so GET /api/pipelines/{id}/source
# can serve real op source text without a runtime call back into Dagster
# (which has no "give me this file's text" GraphQL field) or a shared
# volume between two independently-deployed images. Copied from the
# `pipeline_src` ADDITIONAL build context (docker-compose.yml's
# `additional_contexts: { pipeline_src: ./dagster/dispar_orchestrate }`),
# never from the primary `./rust` context above — the primary context is
# deliberately never widened to the repository root, since the untracked,
# developer-owned `.dockerignore` excludes `rust`/`docs`/`*.md` and would
# break every existing COPY the moment the context became `.`, and a
# repo-root context with NO `.dockerignore` present at all (a clean
# checkout) would instead pull in `.git`, `node_modules`, and everything
# else unbounded.
COPY --from=pipeline_src . /opt/pipeline-src/dispar_orchestrate
# Same convention `dagster/Dockerfile` already uses (WS4 item B1): the
# commit this image was built from, read by `crate::pipeline_source::
# check_commit` via `AppState`/`Config::git_sha` to refuse serving source
# text that cannot be verified against the running Dagster code location's
# own declared commit. Default "unknown" so a bare `docker build .` (no
# --build-arg) still succeeds; `check_commit` treats "unknown" on either
# side as unverifiable, never a fabricated match.
ARG GIT_SHA=unknown
ENV GIT_SHA=${GIT_SHA}
# The built-in "Main" dashboard's shipped tile catalog (WS6,
# lakehouse-bi::specs). Deployments opt in by setting
# BUILTIN_DASHBOARD_SPEC=/app/specs/builtin-default.json; unset means no
# built-in tiles at all.
COPY --from=builder /build/crates/lakehouse-bi/specs ./specs
COPY entrypoint.api.sh /usr/local/bin/entrypoint.api.sh
RUN chmod +x /usr/local/bin/entrypoint.api.sh
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/entrypoint.api.sh"]
