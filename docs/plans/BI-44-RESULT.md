# BI-44: host memory of one engine versus the full stack

This file is the home for the slice-4 memory gate (`BI-44-AC7`, spec
`docs/core/specs/bi-44.md`). It exists so the reader looks in one place for
the number; it does **not** claim a number yet.

## Measurement status: NOT VERIFIED

No `docker compose up` was run from a clean project name for either
standalone file or for the full stack. No peak RSS was sampled, no query was
run against a real in-place source, and `AC1`–`AC7` were not exercised on a
running installation. The only proof in this branch is static: the Rust unit
and integration suites, the TypeScript suite, and the standalone Compose
static tests. Quoting a memory figure here would invent one.

The structural fact is true in the deployment files: each standalone file
starts seven services and one engine, with no warehouse services. That is
not a measurement of host memory.

## What would make it verified

A clean-host run, in the foreground, with the exact commands and counts
recorded here:

1. `docker compose -f docker-compose.yml -p <full-project> up -d` and, after
   readiness, sample the total RSS of the Compose project's containers over
   a settled interval (say the same window used for each variant).
2. `docker compose -f ops/bi-only/docker-compose.bi-only-clickhouse.yml
   --env-file .env -p <bi-project> up -d`, then the same sampling window,
   with one read query against a registered external source.
3. The same for `docker-compose.bi-only-trino.yml`.
4. Record the binary and image versions, the sampling command, the samples,
   the peak of each stack, and the readiness answers, then replace this
   section with the measured result and update the *(proposed)* row of the
   spec and the feature page by name.

Until that run happens, the target stays *proposed*: one engine process and
no warehouse services, with the memory number unmeasured.
