# DATA-5: outside tools read published tables through the API, and every read is counted

Backlog `DATA-5`. Spec: `docs/core/specs/data-5.md`, option B, signed by the product owner on
2026-10-31. Design: ADR 0016 (read it first). Feature page: `docs/core/features/outside-readers.md`.
Base: `main` at `776f133b`. Every `file:line` below was read there.

Planner: Claude Opus. Developers: other agents (`AGENTS.md`). A developer does not change a
decision below. If the code disagrees with the plan, stop and report it.

## 0. Slicing

| Slice | Branch | What | Machine |
| --- | --- | --- | --- |
| **S1** | `feat/data-5-outside-readers-api` | Rust, migration, compose, `oidc-mock`, acceptance test, OPERATIONS | bluefin |
| **S2** | `feat/data-5-outside-readers-console` | Console only, against the contract in D9 | desktop |

The two run in parallel. S1 merges first. S2 is then rebased on `main` and merged.

## 1. Decisions (made; do not change)

- **D1. The scope.** `iceberg:read` is the permission an outside tool's service identity
  carries.
  - Service-identity scopes are parsed like role permissions
    (`lakehouse-auth/src/service_token.rs:215`), so no new mechanism is needed.
  - Only a service principal may use the door. In the handler, if the principal id is not
    `PrincipalId::Service` (`lakehouse-auth/src/principal.rs:28`), answer 403 with "Use a
    service identity's key: the door counts reads per tool." A session admin with `*:*` is
    refused too.
- **D2. The routes.** These go under `/api/iceberg/v1`, each in `POLICY_TABLE` as
  `RequiresPermission("iceberg:read")`:

  | Method | Path | Counted |
  | --- | --- | --- |
  | GET | `/api/iceberg/v1/config` | no |
  | GET | `/api/iceberg/v1/{prefix}/namespaces` | no |
  | GET, HEAD | `/api/iceberg/v1/{prefix}/namespaces/{namespace}` | no |
  | GET | `/api/iceberg/v1/{prefix}/namespaces/{namespace}/tables` | no |
  | HEAD | `/api/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}` | no |
  | GET | `/api/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}` | **yes** |
  | GET | `/api/iceberg/v1/{prefix}/namespaces/{namespace}/tables/{table}/credentials` | no |

  - A catch-all `/api/iceberg/{*rest}` for any method, also in `POLICY_TABLE`, answers 403 with
    the Iceberg error body `{"error":{"message":"Outside readers may only read published Gold
    tables.","type":"ForbiddenException","code":403}}`. So a write never reaches Lakekeeper.
  - `{namespace}` must be exactly `gold`, otherwise the same 403. The namespace list is filtered
    to `gold`.
  - `{prefix}` must equal the prefix Lakekeeper's own `config` returned for the catalog
    warehouse (`LAKEKEEPER_WAREHOUSE`), otherwise 404.
  - `route_auth.rs` asserts every new route both ways: no permission gives 403, and
    `iceberg:read` on a service principal does not.
- **D3. Forwarding.** Each allowed call is forwarded to `LAKEKEEPER_CATALOG_URI` (`config.rs:527`)
  with the same path tail, query and `X-Iceberg-Access-Delegation` header. It carries the
  bearer token from a new env var, `LAKEKEEPER_OUTSIDE_READ_TOKEN_FILE`, read with
  `read_token_file` (`lakekeeper_token.rs:42`).
  - Lakekeeper's status and body come back, with one exception: a non-2xx body is replaced by
    the Iceberg error shape with fixed text. Upstream text never reaches the response
    (principle 4).
  - The tool's own key is never forwarded.
  - Use a plain `reqwest` client with no redirects and a 30-second timeout. Lakekeeper is an
    internal, configured host, not a user-supplied one, so the connector allowlist
    (`resolve_checked`) does not apply. Say so in a comment at the site.
- **D4. The storage address.** A new optional env var, `OUTSIDE_S3_ENDPOINT`.
  - In a table-load response, replace `s3.endpoint` with it, both in `config` and in each
    `storage-credentials[].config`. Do the same in a credentials response.
  - Unit-test the rewrite on a fixture body.
  - If `OUTSIDE_S3_ENDPOINT` or the token file is unset, every door route answers 503
    (`ApiError::Unavailable`), naming the missing setting: "Outside reading is not set up:
    OUTSIDE_S3_ENDPOINT is not set." The check runs before anything is forwarded.
- **D5. The catalog principal.**
  - Add `outside-readers` to `PRINCIPALS` in `ops/oidc-mock/server.py:80-109`.
  - Add it to `lakekeeper-authz-init`'s self-register loop (`docker-compose.yml:~1351`). Grant
    it warehouse `describe` (the `grant()` helper, `:1365-1385`). Do the same in the SeaweedFS
    copy (`:~1870`).
  - Mount its token into `lakehouse-api` at `/tokens/outside-readers.jwt`, the way
    `lakehouse-api-reader.jwt` is mounted (`:687-689`).
  - Set `LAKEKEEPER_OUTSIDE_READ_TOKEN_FILE=/tokens/outside-readers.jwt`.
  - Add `OUTSIDE_S3_ENDPOINT: ${OUTSIDE_S3_ENDPOINT:-}` (empty means off, which is honest).
  - **The `gold` namespace grant.** When the Gold export ensures the `gold` namespace (find the
    call; `lakehouse-iceberg/src/catalog.rs:205`/`:291` create namespaces), the API grants
    `outside-readers` the `select` relation on that namespace. Use a new
    `LakekeeperAdminClient` method next to `grant_machine_principals`
    (`lakehouse-auth/src/openfga.rs:525`), calling the namespace assignments endpoint. It is
    idempotent.
  - This is the first namespace-scoped grant in the stack. If Lakekeeper 0.13 refuses it, or
    has no such endpoint, **stop and report**. Do not fall back to a warehouse-wide `select`.
- **D6. Counting.** Add a new migration, the next free number on `main` (0113+), with a
  why-header:

  ```sql
  CREATE TABLE console.outside_table_read_day (
    day date NOT NULL,
    identity_id uuid NOT NULL,      -- no FK: a deleted tool's reads stay
    identity_name text NOT NULL,    -- the name at the time of the read
    namespace text NOT NULL,
    table_name text NOT NULL,
    reads bigint NOT NULL,
    last_read_at timestamptz NOT NULL,
    PRIMARY KEY (day, identity_id, namespace, table_name)
  );
  ```

  - The store gets two functions: `record_outside_read` (upsert: `reads + 1`,
    `last_read_at = now()`) and `outside_readers_of(namespace, table, since)`. Values are bound.
  - After Lakekeeper answers a table load with 200, the API records the read **before** it
    returns the body. If the record fails, answer 503 "database error" and do not return the
    table (ADR 0016 point 5).
  - Prune rows with `day < current_date - 90` at most once a day per process: keep the last
    prune date in memory.
- **D7. The consumers route.** `GET /api/gold/export/{mart}/consumers` (`routes/gold.rs:694`)
  stays `RequiresAuth`.
  - When the door is set up, it answers
    `{ mart, supported: true, windowDays: 7, consumers: [{ identityId, name, reads, lastReadAt }] }`,
    reads summed over the last 7 days and ordered by reads descending. Use the mart's published
    table name in `gold` (read how the export names it).
  - When the door is not set up, it answers `supported: false` with the same reason text as D4.
    Keep `consumers: null`.
  - Replace the old Trino text and its test.
- **D8. Tenancy.** The door serves the catalog tenant's warehouse (`LAKEKEEPER_WAREHOUSE`)
  only, as the Gold export does. Tenant warehouses are out of scope.
- **D9. The contract for S2** (`src/services/contracts/gold.ts:71-76`):

  ```ts
  export type GoldConsumer = { identityId: string; name: string; reads: number; lastReadAt: string }
  export type GoldConsumers = {
    mart: string
    supported: boolean
    windowDays?: number
    consumers: GoldConsumer[] | null
    reason?: string
  }
  ```
- **D10. The console** (S2).
  - **Gold Exports page** (`src/features/gold/gold-exports-page.tsx:56-106`, header "Consumers
    (7d)"):
    - supported with readers: the number of tools, with a tooltip line per tool (`name: N reads,
      last <relative time>`);
    - supported with none: "No reads in 7 days";
    - not supported: "Not measured" with the reason, as today.
  - **Open-format card** (`src/features/catalog/open-format-card.tsx:166-176`): a row "Readers
    (7 days)" with the same three states. It shows each tool's name and reads, up to 5 tools,
    then "and N more".
  - **Service identities page** (`src/features/admin/service-identities-page.tsx`): under the
    scopes field, a hint: "`iceberg:read` lets an outside tool read published Gold tables at
    <this console's address>/api/iceberg. Use Rotate to get its key."
- **D11. Docs** (S1). Add a section to `docs/OPERATIONS.md`, "Outside tools read published
  tables (DATA-5, ADR 0016)". It covers:
  - how to publish storage and set `OUTSIDE_S3_ENDPOINT`;
  - creating the identity;
  - one example each for pyiceberg (`RestCatalog(uri=…/api/iceberg, token=…, warehouse=…)`)
    and DuckDB;
  - the limits from the feature page.

  In `docs/core/features/gold-publish-per-mart.md`, point to it.

## 2. Tasks

**S1 (bluefin)**, one commit each, in this order:

- **S1-T1. Compose and `oidc-mock`** (D5, without the namespace grant).
  - *Accept:* `docker compose --profile '*' config --quiet`, then a real `up` of the core
    stack. `lakekeeper-authz-init` exits 0, and `/tokens/outside-readers.jwt` exists in
    `lakehouse-api`. Quote it.
- **S1-T2. Migration and store** (D6).
  - Tests in `lakehouse-store`:
    - the upsert adds to the day row;
    - the 7-day sum and its order;
    - a deleted identity's name stays.
  - *Accept:* `cargo test -p lakehouse-store outside`.
- **S1-T3. The door** (D1–D4, D6 recording, D8).
  - Unit tests:
    - the path guard (`gold` only; writes and the catch-all 403);
    - the `s3.endpoint` rewrite;
    - "not set up" gives 503 with the name of the missing setting;
    - a session principal is refused.
  - An integration test with `wiremock` standing in for Lakekeeper: a table load is forwarded
    with the `outside-readers` token, not the tool's key, and a read is recorded. If the
    record fails, the answer is 503 and the body is not returned.
  - Add the `route_auth.rs` entries.
  - *Accept:* `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
    `cargo test -p lakehouse-api iceberg_door`; `cargo test -p lakehouse-api --test route_auth`.
- **S1-T4. The namespace grant** (D5's last two bullets).
  - *Accept:* the grant call is unit-tested against `wiremock`. The real call is proven in
    S1-T6.
- **S1-T5. Consumers route** (D7).
  - *Accept:* `cargo test -p lakehouse-api gold`.
- **S1-T6. Acceptance test.** Extend `ops/gold_export/gold_export_test.py` (it runs in CI's
  "Gold export to Iceberg" job; read how it starts). After the export:
  1. create a service identity with `iceberg:read`, then rotate it to get the key;
  2. with pyiceberg, `RestCatalog(uri=<api>/api/iceberg, token=<key>, warehouse=…)`, load the
     exported table and count its rows: the count equals the seeded rows;
  3. a `create_table` in `gold` is refused 403, and a `load_table` in another namespace is
     refused 403;
  4. the consumers route lists the identity with `reads == 1`.

  In the test's compose service, set `OUTSIDE_S3_ENDPOINT` to the in-network address, because
  the runner is inside the network; the rewrite itself is covered by the unit test. If
  pyiceberg is not in the runner image, add it pinned, with a comment. Remove the old
  `step_consumers_route_is_honest_about_being_unsupported`, or change it to cover the
  not-set-up case.
  - *Accept:* run that compose test locally once. If it cannot run on this machine, write *not
    verified* with the reason, and CI's job is the proof.
- **S1-T7. Docs** (D11).

**S2 (desktop)**, one commit each:

- **S2-T1. Contract and client** (D9). Update the existing test fixtures.
- **S2-T2. Gold Exports page and open-format card** (D10). Tests for the three states each.
- **S2-T3. Service identities hint** (D10).
- *Accept, per commit:* `bun run typecheck`, `bun run lint`, and the touched test files.
  Before handoff: `bun run test`.

## 3. Verification before handoff

- **S1:**
  - `cd rust && cargo fmt --check`, then clippy and tests scoped to `lakehouse-api` and
    `lakehouse-store`. CI runs the whole workspace (owner's rule, 2026-10-30).
  - `docker compose --profile '*' config --quiet`, plus the `up` from S1-T1.
  - `python3 -m py_compile` on the changed Python files.
  - `git status`.
- **S2:** as in its tasks.

## 4. Out of scope

- Tenant warehouses.
- The OAuth2 client-credentials exchange.
- Counting reads that skip the door.
- Replacing `oidc-mock`.
- Silver tables.
- Rows or bytes per read.

## 5. Handoff (developers append below, one entry per slice)

### Handoff S1 (developer, 2026-10-31)

Branch `feat/data-5-outside-readers-api`, base `main` at `776f133b`. Seven
commits, one per task, in plan order:

| Task | Commit | What |
| --- | --- | --- |
| S1-T1 | `6ffad161` | compose + `oidc-mock`: the `outside-readers` catalog principal (D5) |
| S1-T2 | `b787f767` | migration `0117_outside_table_read_day.sql` + the store's counter (D6) |
| S1-T3 | `b5e1fb9c` | the door: the seven routes and the catch-all (D1–D4, D6, D8) |
| S1-T4 | `c25323d9` | the namespace-scoped `select` grant for `outside-readers` (D5) |
| S1-T5 | `7399baf9` | the consumers route (D7) |
| S1-T6 | `6c658ded` | the acceptance test: the outside reader (D7) |
| S1-T7 | `1190ae0e` | `docs/OPERATIONS.md` + the feature page pointer (D11) |

**Checks on the final commit (`1190ae0e`), every one run in the foreground:**

| Command | Result |
| --- | --- |
| `cargo fmt --check` (workspace) | exit 0 |
| `cargo clippy -p lakehouse-api -p lakehouse-store -p lakehouse-auth --all-targets -- -D warnings` | exit 0 |
| `cargo test -p lakehouse-store` | 836 passed, 0 failed |
| `cargo test -p lakehouse-auth` | 143 passed, 0 failed |
| `cargo test -p lakehouse-api --test iceberg_door` | 13 passed, 0 failed |
| `cargo test -p lakehouse-api --test route_auth` | 66 passed, 0 failed |
| `cargo test -p lakehouse-api --test sec11_guard` | 4 passed, 0 failed |
| `cargo test -p lakehouse-api gold` | 134 passed, 0 failed (90 test binaries) |
| `docker compose --profile '*' config --quiet` | exit 0 |
| `python3 -m py_compile ops/gold_export/gold_export_test.py` | exit 0 |
| `python3 ops/lint/check_intra_package_imports.py` | "OK: all intra-package dispar_orchestrate imports resolve" |
| `python3 ops/lint/check_bare_iceberg_count.py` | "R11 guard: no bare count() against a Bronze Iceberg table found." |
| `git status` | clean (nothing generated staged) |

**The acceptance test, run locally (S1-T6's accept).** A fresh project
(`docker compose -p data5 --profile '*' down -v`, then `up -d` with the
dagster profile), the API image rebuilt from this branch, and:

```
docker compose -p data5 --profile dagster run --rm gold-export-test-runner
```

exits 0 and prints `[gold-export] PASSED`. The DATA-5 lines of that run:

```
[gold-export] outside tool gold-export-outside-tool-4e1d48f5 (900160e1-…) has its key (iceberg:read)
[gold-export] the outside tool read 7 rows of gold.gold_export_smoke through the door
[gold-export] the door refused a write: ForbiddenException: Outside readers may only read published Gold tables.
[gold-export] the door refused another namespace: ForbiddenException: Outside readers may only read published Gold tables.
[gold-export] the consumers route lists the outside tool: {'identityId': '900160e1-…', 'name': 'gold-export-outside-tool-4e1d48f5', 'reads': 1, 'lastReadAt': '2026-10-31T12:28:10.853Z'}
```

The seven seeded rows are the seeded rows, the write and the other
namespace are refused 403 with the Iceberg error shape, and the consumers
route lists the tool with exactly one read.

**Per task, what was checked when.**

- **S1-T1** (`6ffad161`): *not re-verified in this session* beyond the
  final block above — the commit message records what it ran:
  `docker compose --profile '*' config --quiet` (exit 0),
  `docker compose -p data5 up -d` (exit 0),
  `data5-lakekeeper-authz-init-1` exit code 0, `/tokens/outside-readers.jwt`
  present in `lakehouse-api`, and Lakekeeper's own assignments endpoint
  listing `{"type":"describe","user":"oidc~outside-readers"}`. The stack
  used by S1-T6's run was built from this compose file, and the door's
  forwarded table load answered 200 under the `outside-readers` token —
  so the principal was registered by the same init container and holds
  what the door needs.
- **S1-T2** (`b787f767`): the commit message records `cargo fmt --check`,
  `cargo clippy -p lakehouse-store --all-targets -- -D warnings`,
  `cargo test -p lakehouse-store --test outside_reads` (4 passed) and
  `cargo test -p lakehouse-store` (all green). *Re-run in this session*:
  `cargo test -p lakehouse-store` — 836 passed, 0 failed.
- **S1-T3** (`b5e1fb9c`): the commit message records `cargo fmt --check`,
  `cargo clippy -p lakehouse-api --all-targets -- -D warnings`,
  `cargo test -p lakehouse-api iceberg_door` (9 unit + 9 integration),
  `cargo test -p lakehouse-api --test route_auth` (66 passed) and the
  whole crate (91 test binaries, all ok). *Re-run in this session*: the
  same targets on the final commit — `iceberg_door` 13 passed,
  `route_auth` 66 passed, `sec11_guard` 4 passed, `gold` filter 134 passed.
- **S1-T4** (`c25323d9`): the commit message records `cargo fmt --check`,
  `cargo clippy -p lakehouse-auth -p lakehouse-api --all-targets -- -D warnings`,
  `cargo test -p lakehouse-auth --test openfga` (15 passed),
  `cargo test -p lakehouse-auth` and `cargo test -p lakehouse-api` (91 test
  binaries, all ok). *Re-run in this session*: `cargo test -p lakehouse-auth`
  — 143 passed, 0 failed. The real grant was measured against the
  compose stack's Lakekeeper 0.13.3 by that session (204 granted, the
  assignments listing then showing `select` on the `gold` namespace); this
  session's stack shows the same assignment:
  `{"assignments":[{"type":"ownership","user":"oidc~gold-export"},{"type":"select","user":"oidc~outside-readers"}]}`.
- **S1-T5** (`7399baf9`): `cargo fmt --check` (exit 0),
  `cargo clippy -p lakehouse-api --all-targets -- -D warnings` (exit 0),
  `cargo test -p lakehouse-api gold` (134 passed),
  `cargo test -p lakehouse-api --test iceberg_door` (13 passed),
  `cargo test -p lakehouse-api --test route_auth` (66 passed),
  `cargo test -p lakehouse-api --test sec11_guard` (4 passed).
- **S1-T6** (`6c658ded`): `python3 -m py_compile` (exit 0),
  `docker compose --profile '*' config --quiet` (exit 0), and the compose
  test itself, run from a clean project: exit 0, "[gold-export] PASSED".
- **S1-T7** (`1190ae0e`): docs only; no check in this repo runs on
  Markdown. The content was checked against the running stack — the API's
  own answers, plus the two client examples (see below).

**Three judgment calls a reviewer should look at.**

1. **S1-T5 changed the door's error type.** `iceberg_door::setup()` now
   answers with a `NotSetUp` value instead of an `ApiError`, so the
   sentence the door's 503 carries and the sentence the console repeats
   are the same string, built once. The alternative — the console
   stringifying the door's error — is a route line that puts an error's
   own text into a response, which `tests/sec11_guard.rs` fails on (it
   failed, with `src/routes/gold.rs:713: "reason": err.to_string(),`
   before the change). `lakekeeper_token::token_unavailable_message` is
   now the shared text for an unreadable token file, so that sentence is
   written once too. The door's answers are unchanged: same status, same
   bodies.
2. **S1-T6 pins `pyiceberg[pyarrow]==0.9.0`**, not the latest. The
   `s3fs` extra was dropped: 0.9.0's default file IO is pyarrow's own
   `S3FileSystem`, and `s3fs` pulls `aiobotocore`, whose `botocore` pin
   conflicts with the unpinned `boto3` this runner already installs
   (`ResolutionImpossible`). 0.9.0's `RestCatalog` was read before the
   pin: it takes the warehouse's prefix from the config response, sends
   the token as a bearer, and raises `ForbiddenError` on a 403 — all
   three are what the test asserts.
3. **S1-T6 changed the CI job's own `.env`** (`OUTSIDE_S3_ENDPOINT=http://rustfs:9000`).
   The runner is INSIDE the compose network, so the published address an
   outside reader can reach from there is the in-network one. The
   endpoint REWRITE itself is covered by the Rust unit test
   (`routes::iceberg_door`'s `rewrite_endpoint`). The job's `.env`
   already carried `CATALOG_TENANT_ID`, `GOLD_EXPORT_RUN_TOKEN` and
   `GOLD_ORPHAN_MIN_AGE_SECONDS=0`, which the local run needed too.

**Not verified, and why.**

- **The whole `cargo test -p lakehouse-api` crate suite (91 test binaries)
  was not run in this session for S1-T5/T6/T7**: the machine's rule for
  this slice is not to spend 20+ minutes on it (CI runs it), and the
  scoped targets above are the ones the plan names. T3's and T4's commit
  messages record that suite green on those commits.
- **The full workspace run** (`cargo clippy --workspace --all-targets
  --all-features -- -D warnings` and `cargo test --workspace`) was not
  run: the plan's §3 scopes S1 to `lakehouse-api` and `lakehouse-store`
  (plus `lakehouse-auth`, which S1-T4 touched). CI runs the workspace.
- **`bun run typecheck` / `bun run lint` / `bun run test`** were not run:
  S1 touches no TypeScript (the console is S2's slice).
- **`(cd dagster && python -m pytest dispar_orchestrate -q)`** was not
  run: S1 touches no Python package (only `ops/gold_export/`, the
  acceptance script, whose `py_compile` passed).
- **`cargo test -p lakehouse-iceberg --test g1_lakekeeper -- --ignored`**
  was not run: it needs a live Lakekeeper stack and is not in this
  plan's accept list.
- **The DuckDB example in `docs/OPERATIONS.md`** was run by hand against
  this stack (`ATTACH … (TYPE iceberg, URI '…/api/iceberg',
  AUTHORIZATION_TYPE 'OAUTH2', TOKEN '<the key>')`, then
  `SELECT count(*) FROM cat.gold.gold_export_smoke`), not by the
  acceptance test. The pyiceberg example IS the acceptance test.

**One local-environment note for whoever runs the acceptance test again.**
This machine's `.env` is host-facing, so the local run needed these
exported for the WHOLE cycle (a different value for a variable compose
interpolates into a running container makes compose recreate that
container mid-test): `LAKEKEEPER_BASE_URI=http://lakekeeper:8181`,
`DAGSTER_URL=http://dagster-webserver:3000/graphql`,
`GOLD_EXPORT_RUN_TOKEN`, `CATALOG_TENANT_ID`,
`CONNECTOR_S3_ACCESS_KEY`/`CONNECTOR_S3_SECRET_KEY` (the RustFS keys) and
`GOLD_ORPHAN_MIN_AGE_SECONDS=0`. CI's job `.env` already sets every one
of them, so CI needs none of this. The API image was rebuilt from this
branch (`docker compose -p data5 build lakehouse-api`, 7m10s) before the
run.

**The stack is torn down**: `docker compose -p data5 --profile '*' down -v`
removed the project's containers and volumes, and the testcontainers the
cargo tests started were removed
(`docker ps -q --filter label=org.testcontainers.managed-by=testcontainers | xargs -r docker rm -f`).
No container is left running.

## 6. Review (planner appends below)

### Review S1 round 1 (planner, 2026-10-31, at `38d04fe4`)

**What holds.**
- A session principal is refused (D1).
- The tool's key is never forwarded: the bearer is the `outside-readers` token (D3).
- Lakekeeper's error bodies are replaced with fixed text (principle 4).
- The read is recorded before the body is returned, and a failed record answers 503 (D6).
- `s3.endpoint` is rewritten in both places (D4).
- The prefix must equal Lakekeeper's own (D2).
- The namespace grant is namespace-scoped, with no warehouse-wide fallback (D5).
- **Two good deviations, accepted:**
  - `strip_uri_override` removes Lakekeeper's `uri` override, so a client cannot leave the door.
  - The migration drops the plan's `console.` prefix, because `console` is a ClickHouse
    database, not a Postgres schema. The header says why.
- No default credentials in compose. `OUTSIDE_S3_ENDPOINT` defaults to empty, which means off.

**BLOCKER 1: the table name can walk out of the door.**
- `table` and `credentials` take `{table}` from the path. axum decodes `%2F` to `/`, and the
  value is then pasted into the forwarded URL with `format!`.
  - Example: a request for `…/namespaces/gold/tables/..%2F..%2F..%2Fsilver%2Ftables%2Fx`
    becomes `…/v1/{prefix}/namespaces/gold/tables/../../../silver/tables/x`.
  - reqwest's URL parsing resolves the `..`, so the call goes to another Lakekeeper path.
- The `gold`-only check in the API is bypassed. The namespace grant still refuses `select`
  outside `gold`, but that is the second layer, not the first.
- Any GET that answers 200 would also be counted as a "read" under a made-up table name.
- **Fix:**
  - Accept `{table}` only if it matches `^[A-Za-z0-9_-]{1,255}$`. No dot, no slash, no
    percent; check what names the Gold export writes and make sure they pass. Anything else is
    refused (the Iceberg 403, `refused()`) before any forwarding.
  - Also build the forwarded path with each segment percent-encoded (the `url` crate's
    `path_segments_mut().push(...)`), so a later change cannot reopen this.
  - Test in `tests/iceberg_door.rs`: a `%2F..` table name is refused, `wiremock` receives no
    call, and no read is recorded. Cite `DATA-5 S1 review BLOCKER 1` at the site.

**SHOULD-FIX 1: the query is encoded twice.**
- `query_for` splits the raw query without decoding it, then `reqwest::RequestBuilder::query`
  encodes it again. So `pageToken=a%3D` reaches Lakekeeper as `a%253D`, and paging through a
  long table list breaks.
- **Fix:** parse with `url::form_urlencoded::parse` (decoded pairs), then let `query()` encode
  once. Add a test that a `%`-encoded `pageToken` reaches `wiremock` unchanged.

**Open: BLOCKER 1, SHOULD-FIX 1.**
