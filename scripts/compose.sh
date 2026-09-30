#!/usr/bin/env bash
# `docker compose`, with GIT_SHA set from this checkout.
#
# Why: `lakehouse-api` and the Dagster images are built with a GIT_SHA
# build arg. Every Dagster op stamps it as its `commit`, and the API serves
# an op's source code on the pipeline page only when that commit equals its
# own (`pipeline_source.rs::check_commit`): otherwise the file baked into
# the API image might not be the code that ran. Compose cannot run
# `git rev-parse` itself, so a plain `docker compose build` leaves GIT_SHA
# at its placeholder "unknown", and the source view says it is unavailable.
#
# A tree with uncommitted changes to tracked files gets a "-dirty" suffix,
# so a build from edited files never claims to be the commit it is not.
# An explicit GIT_SHA in the environment wins.
#
# Usage: scripts/compose.sh <any docker compose arguments>
#   scripts/compose.sh up --build
#   scripts/compose.sh --profile dagster up -d --build lakehouse-api dagster-code-location
set -euo pipefail

cd "$(dirname "$0")/.."

if [ -z "${GIT_SHA:-}" ]; then
  GIT_SHA="$(git rev-parse HEAD)"
  if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    GIT_SHA="${GIT_SHA}-dirty"
  fi
fi
export GIT_SHA

exec docker compose "$@"
