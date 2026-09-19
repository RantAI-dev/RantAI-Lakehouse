# syntax=docker/dockerfile:1
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
# `sqlx-cli`, pinned to the same version as the workspace's `sqlx`
# dependency, for the migration step in entrypoint.api.sh. `main.rs` now
# also applies migrations at boot via `lakehouse_store::migrate` (see
# `main.rs`), so this is belt-and-suspenders: whichever runs first wins,
# `sqlx migrate run` is idempotent, and neither path is fatal on failure.
#
# Installed BEFORE the sources are copied in: it depends on nothing in this
# repo, so its layer stays cached across code changes instead of being
# recompiled from scratch after every edit.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo install sqlx-cli --version 0.8.6 --no-default-features --features rustls,postgres --locked
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# `sqlx::migrate!("../../migrations")` (lakehouse-store/src/lib.rs) embeds
# the migration SQL into the binary at *compile* time, so the directory
# must exist in the build context before `cargo build` runs, or the build
# fails outright.
COPY migrations ./migrations
# Any source change invalidates this layer, so without the cache mounts
# every rebuild compiled all ~600 dependencies again in release mode. The
# mounts keep the crate registry and `target/` between local builds, so a
# rebuild recompiles only the crates that changed. `target/` is a cache,
# not part of the image, hence the binary is copied out in the same step.
# (CI's GHA layer cache does not persist cache mounts; there it behaves as
# before.)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked -p lakehouse-api \
    && cp target/release/lakehouse-api /usr/local/bin/lakehouse-api

# `debian:trixie-slim`, not `bookworm-slim`: `rust:1.96.1-slim` builds on
# Debian 13 (trixie), whose glibc is newer than bookworm's. A bookworm
# runtime image fails to even exec the trixie-built `sqlx-cli` binary
# (`GLIBC_2.39' not found`) — matching the base OS avoids a glibc skew
# between the two stages.
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /usr/local/bin/lakehouse-api /usr/local/bin/lakehouse-api
COPY --from=builder /usr/local/cargo/bin/sqlx /usr/local/bin/sqlx
COPY migrations ./migrations
COPY entrypoint.api.sh /usr/local/bin/entrypoint.api.sh
RUN chmod +x /usr/local/bin/entrypoint.api.sh
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/entrypoint.api.sh"]
