# ADR 0005 — Dagster code-location ownership and compose packaging

- **Status:** Accepted
- **Phase:** P3
- **Date:** 2026-09-01

## Context

`lakehouse-dagster` (Rust) has talked to a `Dagster` GraphQL endpoint since
Phase 2 (`launchRun`, `terminateRun`, schedules, run status), and
`lakehouse-api::config::Config` has carried `DAGSTER_URL` / `DAGSTER_REPO`
/ `DAGSTER_LOCATION` since before this phase — but `docs/OPERATIONS.md`
recorded `Dagster` as deliberately absent from the compose stack ("Heavy
... and out of scope for a local dev loop"), so those routes have always
degraded to 503. P3 needs a real `Dagster` instance with a real code
location for G3a (dlt `sql_database` → Bronze Iceberg through Lakekeeper),
so this ADR decides three things the plan names as due here: where the
Python lives in this repo, how it is packaged into an image, and how it is
versioned relative to the Rust workspace.

`Dagster`'s own architecture forces some of this: a "code location" is a
Python process (or gRPC server) the webserver/daemon load definitions
from over gRPC — it is never in-process with the webserver, so it is
always at least one more container, one more image, one more place a
dependency version can drift from the Rust side.

## Decision — where the Python lives

**A new top-level directory, `dagster/`, sibling to `rust/`, not nested
inside it.** Concretely:

```
dagster/
  pyproject.toml            # dlt, dagster, dagster-webserver, pyiceberg extras
  dispar_orchestrate/
    __init__.py
    definitions.py           # `defs = Definitions(...)` — the code location's entrypoint
    assets.py                 # @op/@job materializing the Bronze ingest
    dlt_pipeline.py            # dlt.pipeline(...) + sql_database source + iceberg destination
  Dockerfile
```

- **Not inside `rust/`.** `rust/` is a Cargo workspace; every existing
  crate's `Cargo.toml`/`Cargo.lock` and CI job (`cargo fmt`, `cargo
  clippy`, `cargo test`, `cargo deny`) assume the directory tree under
  `rust/` is Rust source. Mixing a `pyproject.toml` and a Python package
  into that tree would make `rust/`'s own tooling (path-based crate
  discovery via `Cargo.toml`'s `[workspace] members`, `cargo fmt --check`
  invoked with `rust` as `working-directory` in `.github/workflows/ci.yml`)
  need to actively ignore Python files rather than simply not seeing them.
  A sibling top-level directory means neither toolchain's default
  file-discovery ever has to special-case the other.
- **`dispar_orchestrate`, not `dagster`, as the importable package name.**
  `DAGSTER_LOCATION`'s existing default is
  `"dispar_orchestrate.definitions"` — carried in `config.rs` since before
  this phase, ported from the pre-existing TypeScript `dagster.ts` client.
  This ADR does not invent that name; it makes the actual Python module
  match a default the Rust side already assumed existed. Naming the
  top-level *directory* `dagster/` (not `dispar_orchestrate/`) avoids the
  Python package and its container directory colliding in naming while the
  importable module underneath keeps the name every existing default
  string already names.
- **One code location, one package, for P3.** `docs/plans/
  LAKEHOUSE-FOUNDATION-PLAN.md` §3 names exactly one P3 job (dlt
  `sql_database` → Bronze) and one P4 job (per-Bronze-table maintenance
  chain) as due; both live under the same `dispar_orchestrate` package as
  separate modules (`assets.py` today, a `maintenance.py` when P4 lands),
  not as two separate code locations. Two code locations would mean two
  gRPC servers, two images, and two places `DAGSTER_LOCATION` could point
  — unjustified until a real reason to split (e.g. genuinely different
  Python dependency sets that conflict) appears.

## Decision — how it is packaged into an image

**A dedicated `dagster/Dockerfile`, built by compose the same way
`rust/Dockerfile` already is** (`build: { context: ./dagster, dockerfile:
Dockerfile }`), not a stock `dagster/dagster` image with a bind-mounted
`dispar_orchestrate/` directory.

- **Why a built image, not a bind mount.** A bind-mounted code location
  works for local edit-reload but does not answer "how does this ship" —
  CI's `g3a-test-runner` and any future production deployment need an
  image that carries its own dependencies (`dlt[pyiceberg]`, the pinned
  `pyiceberg`/`dlt` versions) baked in, not resolved against whatever
  happens to be on the host running compose. This mirrors exactly how
  `rust/Dockerfile` is already built rather than the workspace being
  bind-mounted into a stock `rust:*` image for the API service itself
  (only the *test runner* — `g1-test-runner` — uses the bind-mount +
  stock-image shape, because it exists purely to run `cargo test` inside
  the compose network, not to ship a Bronze/Dagster runtime artifact).
- **Three containers, one image.** `docker-compose.yml` gains
  `dagster-webserver`, `dagster-daemon`, and `dagster-code-location`, all
  built from `dagster/Dockerfile` (`dagster-webserver`/`dagster-daemon` run
  `dagster-webserver`/`dagster-daemon` against a `workspace.yaml` pointing
  at `dagster-code-location`'s gRPC port; `dagster-code-location` runs
  `dagster api grpc`). This is `Dagster`'s standard multi-container
  topology, not a custom shape invented for this repo.
- **Behind a `dagster` compose profile**, never started by a plain `docker
  compose up` — matching exactly how P2 gated `seaweedfs`/
  `seaweedfs-bucket-init`/`lakekeeper-warehouse-init-seaweedfs` behind the
  `seaweedfs` profile, and P1/P2 gated `g1-test-runner` behind `test`. The
  reasons `docs/OPERATIONS.md` gave for excluding `Dagster` from the
  default dev loop (heavy: three more containers, a webserver + daemon +
  gRPC code-location process) have not changed; this ADR does not
  overturn that call, it gives operators/CI an opt-in path to it.
- **`Dagster`'s own storage** (run/event-log/schedule storage) points at
  the existing `postgres` service, via a dedicated database
  (`dagster`, created by a one-shot `dagster-db-init` job mirroring
  `lakekeeper-db-init` exactly — separate database, not a schema mixed
  into the `lakehouse` application database), not `Dagster`'s default
  SQLite (which does not survive container restarts and cannot be shared
  between the webserver and daemon containers, both of which need to see
  the same run history).

## Decision — versioning relative to the Rust workspace

**Independent version pins, no shared lockfile, correctness enforced by
the G3a test, not by a version-matching mechanism.**

- `dagster/pyproject.toml` pins its own `dlt`, `dagster`, `pyiceberg`
  (transitively, via `dlt[pyiceberg]`) versions — a Python lockfile
  (`uv.lock` or `requirements.txt` with hashes), independent of
  `rust/Cargo.lock`. There is no dependency relationship between the two:
  the Python code location talks to Lakekeeper's REST catalog exactly the
  same way `lakehouse-iceberg` does (the Iceberg REST protocol, a
  network-level contract, not a shared library), and to Postgres via
  `sql_database`'s own driver, not through any Rust crate. Nothing in
  `dagster/` imports or links against anything in `rust/`.
- **The `iceberg-catalog-rest` protocol version is the actual coupling
  point**, not a language-level one. Both `lakehouse-iceberg`
  (`iceberg`/`iceberg-catalog-rest` `0.10.1`, per `docs/plans/
  G1-RESULT.md`) and `dlt`'s `pyiceberg` dependency talk to the *same*
  Lakekeeper server (`quay.io/lakekeeper/catalog:v0.13.3`, pinned in
  `docker-compose.yml`) over its REST catalog API. Lakekeeper's own pinned
  version is therefore the actual compatibility anchor for both language
  ecosystems — bumping Lakekeeper is the event that could break either
  side, not bumping `dlt` or `iceberg-rust` independently of each other.
- **No CI job cross-checks Python and Rust versions against each other**,
  by design: `docs/plans/G1-RESULT.md` already established the pattern of
  measuring interop directly (the G1 acceptance test) rather than
  asserting version compatibility as a standalone check. G3a is this
  ADR's equivalent: it proves the two ecosystems interoperate through
  Lakekeeper today, on the pinned versions this ADR records, and is the
  thing that must be re-run (not a version diff reviewed by eye) whenever
  either pin changes.

## Decision — ownership

**Whoever owns Bronze ingestion (the P3/P4/P5 phases: dlt, Dagster
maintenance jobs, eventually Debezium Server) owns `dagster/`, as a single
unit** — not split by "Python owner" vs. "Rust owner" along a language
boundary. The plan's own phase boundaries (P3 dlt, P4 maintenance, P5 CDC)
are the actual seams; `dagster/dispar_orchestrate/` grows one module per
phase under one ownership umbrella, mirroring how `lakehouse-iceberg`
(Rust) is one crate owned by whoever owns the catalog/storage boundary
regardless of which phase added which function to it.

## Consequences

- `docker-compose.yml` gains: `dagster-db-init` (one-shot, mirrors
  `lakekeeper-db-init`), `dagster-webserver`, `dagster-daemon`,
  `dagster-code-location` — all behind the `dagster` profile — plus a
  `g3a-test-runner` service, also behind the `dagster` profile (not `test`
  — it depends on `dagster-webserver`, itself `dagster`-gated, and Compose
  validates a service's entire `depends_on` closure against active
  profiles even for an unrelated `run <other-service>` invocation; see
  `g3a-source-init`'s comment in `docker-compose.yml`), following the
  `g1-test-runner` pattern otherwise (runs inside the compose network,
  brings up its own `depends_on` chain).
- `lakehouse-api`'s `DAGSTER_URL`/`DAGSTER_REPO`/`DAGSTER_LOCATION`
  defaults (`http://localhost:13030/graphql` / `__repository__` /
  `dispar_orchestrate.definitions`) now correspond to a real, buildable
  code location for the first time — the compose defaults for
  `lakehouse-api` itself stay pointed at the deliberately-unreachable
  `http://dagster.invalid:13030/graphql` placeholder (Dagster is still
  profile-gated, so a plain `docker compose up` still gets 503s on
  pipeline routes, per `docs/OPERATIONS.md`'s existing "what's
  deliberately NOT in the stack" posture) — an operator who brings up the
  `dagster` profile overrides `DAGSTER_URL` to
  `http://dagster-webserver:3000/graphql` (in-network) or
  `http://localhost:3000/graphql` (host) themselves.
- A future P4 (maintenance job) and P5 (Debezium/CDC config generation)
  add modules under the same `dispar_orchestrate` package and the same
  image, not new top-level directories or new ADRs about where Python
  lives — that question is answered once, here.

## Verification

No Rust code changes as a direct consequence of this ADR (it is a
packaging/location decision); its verification is that `dagster/`
actually builds (`docker compose --profile dagster build
dagster-code-location`) and that `lakehouse-dagster`'s existing GraphQL
client — unchanged since Phase 2 — can list jobs/launch runs against the
real webserver once `DAGSTER_URL` is pointed at it, which the G3a test
exercises end-to-end.
