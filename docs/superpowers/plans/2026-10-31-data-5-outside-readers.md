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

## 6. Review (planner appends below)
