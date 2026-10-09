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
#
# Three build stages share one tools base, so the two `cargo install`s below
# are compiled once and stay above every `COPY` of repo files (their layer
# changes only when a pin here changes):
#
#   tools    apt packages, `sqlx-cli`, `cargo-chef`
#   planner  copies the manifests + sources, writes `recipe.json`
#   builder  compiles the dependencies from `recipe.json` alone (`cook`),
#            then copies the sources and compiles the workspace
FROM rust:1.96.1-slim AS tools
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
# `sqlx-cli`, pinned to the same version as the workspace's `sqlx`
# dependency, for the migration step in entrypoint.api.sh. `main.rs` now
# also applies migrations at boot via `lakehouse_store::migrate` (see
# `main.rs`), so this is belt-and-suspenders: whichever runs first wins,
# `sqlx migrate run` is idempotent, and neither path is fatal on failure.
#
# `cargo-chef` (exact pin, `--locked`; CI-SPEED-PLAN §4.2) is what makes the
# ~500 dependency crates a layer of their own: `prepare` reduces the
# workspace to a `recipe.json` that changes only when `Cargo.lock` or a
# `Cargo.toml` changes, and `cook` compiles exactly that. No `latest` tag and
# no third-party base image: it is built from source here, like `sqlx-cli`.
#
# Both are installed BEFORE any repo file is copied in: they depend on
# nothing in this repo, so their layer stays cached across code changes
# instead of being recompiled from scratch after every edit.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo install sqlx-cli --version 0.8.6 --no-default-features --features rustls,postgres --locked \
    && cargo install cargo-chef --version 0.1.78 --locked

FROM tools AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# `recipe.json` holds the manifests and the lockfile, not the sources, so a
# source-only change re-runs this (cheap) step and produces a byte-identical
# recipe: the `cook` layer below stays a cache hit.
RUN cargo chef prepare --recipe-path recipe.json

FROM tools AS builder
# Where cargo writes `target/`. The default is the cache mount below, exactly
# as before this stage was split: a local `docker compose up --build` keeps
# its compiled dependencies in the mount between builds, so a one-line source
# change recompiles only the crates that changed, which is what makes a local
# rebuild fast.
#
# A cache mount is not part of the image and is NOT restored by the GitHub
# Actions layer cache, so with the default the `cook` step below would be
# recompiled from scratch on every CI run. CI passes
# `CARGO_TARGET_DIR=/build/target-layer` instead: that path is outside every
# mount, so `cook`'s output lands in the image layer, and `cache-from:
# type=gha` can restore it (.github/workflows/ci.yml, `images` job). The mount
# stays at the fixed path `/build/target` on both steps either way; with the
# CI value it is simply unused.
ARG CARGO_TARGET_DIR=/build/target
ENV CARGO_TARGET_DIR=${CARGO_TARGET_DIR}
COPY --from=planner /build/recipe.json recipe.json
# Dependencies only, `-p lakehouse-api` and the same `--release --locked` as
# the build below so cargo reuses every artifact (a different package set or
# profile would change feature unification and recompile them).
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo chef cook --release --locked -p lakehouse-api --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# `sqlx::migrate!("../../migrations")` (lakehouse-store/src/lib.rs) embeds
# the migration SQL into the binary at *compile* time, so the directory
# must exist in the build context before `cargo build` runs, or the build
# fails outright.
COPY migrations ./migrations
# Any source change invalidates this layer, but not `cook` above, so only the
# workspace's own crates are compiled here. The mounts keep the crate registry
# and (with the default `CARGO_TARGET_DIR`) `target/` between local builds.
# `target/` is a cache, not part of the image, hence the binary is copied out
# in the same step.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked -p lakehouse-api \
    && cp "$CARGO_TARGET_DIR/release/lakehouse-api" /usr/local/bin/lakehouse-api

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
#
# The destination's LAST COMPONENT is the code location's Python package
# name, and `pipeline_source::build_allowlist` keys every file it finds
# under exactly that name — a `source_ref` is emitted by the code location
# itself and is always relative to its own package root. So a deployment
# that bakes a different code location must change BOTH halves together:
# the `pipeline_src` context it copies from, and this package name. Hence
# one ARG driving the copy destination and `PIPELINE_SOURCE_DIR`, instead
# of a literal repeated in two places that could silently disagree.
ARG PIPELINE_PACKAGE=dispar_orchestrate
COPY --from=pipeline_src . /opt/pipeline-src/${PIPELINE_PACKAGE}
ENV PIPELINE_SOURCE_DIR=/opt/pipeline-src/${PIPELINE_PACKAGE}
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
