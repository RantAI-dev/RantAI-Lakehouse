# `DATA-66` T9 result — Iceberg sync latency

**Status: not measured.**

No number in `docs/core/specs/data-66.md` is confirmed by this file. The
proposed targets (pipeline write to `success` under 60 s p95, outside change
to `success` under 90 s p95, 100,000-row Gold table) remain **unverified**
until someone runs the measurement below and replaces this file.

## What was asked

`docs/superpowers/plans/2026-11-27-data-66-iceberg-sync.md` T9: on the compose
stack, with a 100,000-row Gold table set to "On every update", measure the time
from the authored op's write (`AssetMaterialization` timestamp) to the
`success` row in `console.gold_export_run`, 10 runs; and the same for an
`INSERT` made with `clickhouse-client` outside a pipeline; then record p50 and
p95 and the raw numbers here.

## Why it was not measured

The developer VM could not bring the stack up:

- **No image of this branch exists to start.** The only images present are
  third-party (`postgres:16`, `clickhouse/clickhouse-server:26.8`,
  `rustfs/rustfs:1.0.3`, ...). `lakehouse-api`, `console`,
  `dagster-code-location` are `build:` services in `docker-compose.yml`
  (`rantai-lakehouse-api:local`, `rantai-lakehouse-console:local`,
  `rantai-lakehouse-dagster:local`), and building the API image is a release
  cargo build of the whole workspace inside Docker.
- **The disk could not take that build plus the running stack.** The root
  filesystem had 24 GB free and was 96% used (`df -h /`), with no Docker build
  cache (`docker system df`: `Build Cache 0B`). AGENTS.md forbids the shortcuts
  that would make room (a `cargo clean`, a per-worktree target, or deleting the
  shared cache), and a failed image build after filling the disk would break
  the other agents sharing this machine.
- **The measurement needs the acceptance setup, not only `up`.** 10 authored
  pipeline runs on a 100,000-row Gold table and an outside `INSERT` through
  `clickhouse-client` are the acceptance steps `DATA-66-AC1`/`AC2`, all still
  "Not run" on the feature page. Running them here would be a second,
  unreviewed acceptance pass on a stack the product owner has not accepted.

`docker compose --profile '*' config --quiet` **did** pass (with the throwaway
`.env` from `sh ops/init-env.sh`), so the compose file is valid; what is missing
is the built stack, the disk headroom to build it, and the operator acceptance
setup.

## How to measure it (for whoever does)

1. Build the stack with `GIT_SHA=$(git rev-parse HEAD) docker compose --profile
   dagster up -d --build` on a machine with room for the images and the runs.
2. Seed a Gold table with 100,000 rows, author a pipeline that writes it, and
   switch its publication on the card to "On every update"
   (`PUT /api/gold/export/{mart}/publication` with
   `{"enabled":true,"syncMode":"on_update"}` is the same change through the
   API).
3. For 10 runs, take the op's `AssetMaterialization` event time (Dagster's run
   event log) and the `started_at`/`finished_at` of the next `success` row in
   `console.gold_export_run` for that `(zone, mart)`. Record each pair, then
   p50 and p95.
4. For the outside change, record `now()` immediately before the
   `clickhouse-client` `INSERT` and the `success` row that follows it, 10
   times. The watcher's tick is 15 s, so the floor is one tick plus one export.
5. Replace this file with the numbers, the commit, and the machine, and cite it
   from `docs/core/specs/data-66.md` and the feature page's decision 10.
