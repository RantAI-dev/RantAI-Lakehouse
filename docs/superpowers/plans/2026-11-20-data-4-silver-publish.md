# DATA-4: Publish Silver tables in open format (Iceberg)

Backlog: `DATA-4` (Feature, Build, P3). Spec: `docs/core/specs/data-4.md`.
Feature page: `docs/core/features/silver-publish.md` (decisions 1–8).
Branch `data-4-silver-publish`, base `origin/main` at `6b2242ff6`. Every
`file:line` anchor below was read at `6b2242ff6`.

Developer: a non-Opus agent (GLM 5.3). The console part reuses an existing
card on one more page; it is not a revamp, so it stays with this developer
(`AGENTS.md`, console design exception). If anything in this plan is wrong
against the code, stop and write a "Mismatch found by the developer" entry
under Handoff. Do not improvise a different design.

## What this builds, in one paragraph

Gold tables (`serving.*` in ClickHouse) can be published as Iceberg tables in
the `gold` namespace: a per-table switch, automatic publish after a pipeline
run, skip when unchanged, the last 7 copies kept, masking at copy time, a
status card and a run history (`DATA-1`, `DATA-2`, `DATA-10`, `DATA-49`,
`DATA-56`). This plan makes the **same code path** publish Silver tables
(`silver.*` in ClickHouse) to an Iceberg `silver` namespace. Nothing is
copied: every Gold function that hard-codes "gold" takes a layer instead, and
Silver gets thin route handlers, one Dagster job and one prop on the existing
card.

## Decisions (made; do not change)

- **D1. Defaults accepted (product owner, 2026-11-20: "build it; the spec's
  proposed targets are the defaults").** Row limit 5,000,000, read from the
  same `GOLD_EXPORT_MAX_ROWS` (one setting for both layers). Batch size from
  `GOLD_EXPORT_BATCH_SIZE`. Copies kept 7, from the same `GOLD_KEEP_COPIES`.
  Skip when unchanged: the same `DATA-10` content fingerprint
  (`gold_export_history::mart_content_fingerprint`,
  `rust/crates/lakehouse-api/src/gold_export_history.rs:611`). Masking: the
  same `export_batch_sql` rewrite (`gold_export.rs:687`), so a policy on
  `silver.<t>` masks the copy when it is made. Status: the same states as Gold
  ("Last publish failed", "Never published", "Not measured", "Out of date",
  "Up to date", `src/lib/gold-freshness.ts:15-21`). Recorded on the feature
  page, decisions 2 and 4.
- **D2. One publish path, parameterised by layer (`AGENTS.md` rule 4).** A new
  `PublishedLayer { Gold, Silver }` enum in `lakehouse-iceberg` names the
  namespace. Every Gold function that hard-codes the namespace, the source
  database, the history key or the publication key gets a `layer` argument.
  Existing Gold functions keep their names and signatures where other code
  calls them (they delegate with `PublishedLayer::Gold`), so `DATA-6` and the
  assistant's tool keep compiling. No `silver_export.rs` copy of
  `gold_export.rs`, no second history table, no second store module.
- **D3. Target.** Namespace `silver` in the same Lakekeeper warehouse
  (`LAKEKEEPER_WAREHOUSE`), created on first publish exactly as `gold` is
  (`catalog.rs:284-307`). Same table naming (`gold::sanitize_mart_name`,
  `gold.rs:75`), same schema (`_exported_at` + the table's columns,
  `gold::gold_schema`, `gold.rs:87`), same `day(_exported_at)` partition,
  format version 2, append then trim. The `gold-export` catalog principal
  already holds warehouse-wide `create modify select`
  (`docker-compose.yml:1739`), so **no compose change** is needed.
- **D4. Source.** ClickHouse database `silver` (a constant, not a setting: the
  catalog reads Silver as `silver.<t>` whatever is configured,
  `routes/catalog.rs:752-758`). Reuse `routes::schema_versions::SILVER_DATABASE`
  (`routes/schema_versions.rs:84`, make it `pub(crate)`). Gold keeps
  `Config::gold_source_schema` (`config.rs:947`).
- **D5. Routes.** Silver mirrors every Gold route except `consumers`:
  `POST`/`GET /api/silver/export/{mart}`, `GET /api/silver/exports?mart=`,
  `GET /api/silver/publications`, `GET`/`PUT
  /api/silver/export/{mart}/publication`. The wire names are Gold's (`{mart}`
  path segment, `mart` query parameter, `mart` JSON field) so that ONE
  TypeScript client and ONE Dagster helper serve both layers; the module doc
  says "for Silver, `mart` is the Silver table's bare name". The publication
  body gains `"layer": "gold" | "silver"` for both layers.
- **D6. Permission and credentials (feature page decision 5).** Same as Gold:
  the `gold:export` permission (`policy.rs:543`, `check_export_token`
  `routes/gold.rs:247`), the same `GOLD_EXPORT_RUN_TOKEN` and
  `gold-export-scheduler` service identity, the same catalog token file
  `LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE`. No new permission, no new setting. The
  permission's description becomes "Publish Gold and Silver tables in open
  format and export Gold marts as open files." (`lakehouse-auth/src/permission_catalog.rs:138-141`).
- **D7. Storage keys.** Postgres `gold_publication` (migration `0054`) gains a
  `layer` column, `PRIMARY KEY (layer, mart)`; existing rows are Gold. The
  ClickHouse history tables `console.gold_export_run` and
  `console.gold_trim_run` gain `layer LowCardinality(String) DEFAULT 'gold'`;
  every read filters on it. A Silver table and a Gold mart of the same name
  never share a switch, a run, a fingerprint or a trim row.
- **D8. Single-flight.** The per-table lock (`state.rs:171`,
  `gold_lock.rs`) keeps the bare mart name as the Gold key (unchanged), and
  uses `"silver.<t>"` for Silver. A validated `Ident` holds no `.`, so the two
  can never collide.
- **D9. Tenant isolation (feature page decisions 6 and 7; `SEC-20`, `SEC-50`,
  `SEC-80`).**
  - (a) *Reach.* Silver is the shared catalog (`docs/core/features/
    queries-scoped-to-the-tenant.md:94`). Every Silver route that reads the
    table (export, publication `GET`/`PUT`, **and** read-back and history,
    which Gold does not gate) asks the same gate Gold asks,
    `gated_source_table` (`routes/gold.rs:369`), on
    `SELECT count() AS c FROM silver.\`<t>\``: a person as themselves, a job
    as the catalog's tenant (`gold_export_gate_actor`, `routes/gold.rs:329`).
    `export_mart` repeats the gate itself (`gold_export.rs:923-931`).
  - (b) *Data of another organisation.* When the deployment holds more than
    one tenant, a Silver table is **not publishable** if an upload claimed by
    a tenant other than the catalog's has its name (`upload_table_claim`,
    read through `routes::query::claim_on`, `routes/query.rs:1017`, on
    `bronze.<t>`), or if any `pipeline_definition` whose `target` is
    `silver.<t>` belongs to a tenant other than the catalog's (a `NULL`
    tenant counts as the catalog's, as `SEC-20` decision 5 rules for rows
    with no tenant). With several tenants and no `CATALOG_TENANT_ID`, every
    Silver table is refused. With one tenant nothing is checked (there is no
    other organisation). The answer is `422` with the fixed sentence
    `SILVER_OF_ANOTHER_TENANT` (Task 5). On `PUT ... publication` it refuses
    before anything is stored. On `POST ... export` it is a failed publish
    and is recorded (it can become true after the switch was turned on, and
    the card must then say "Last publish failed" with the reason). This is
    stricter than Gold; the same gap on Gold is follow-up F2.
  - (c) *The open-format catalog.* The copy lands in the catalog tenant's
    warehouse (ADR 0003). The `outside-readers` principal is **never** granted
    `silver` (`grant_outside_readers_select`, `gold_export.rs:1079`, runs for
    Gold only), and the door keeps serving `gold` only
    (`routes/iceberg_door.rs:315`, ADR 0016 point 2 unchanged). Inside the
    product the copy reads as `icecat_api.\`silver.<t>\`` or
    `iceberg.silver.<t>`, which canonicalises to `silver.<t>`
    (`sql_rewrite.rs:576`), the Silver table's own key: the same tenant rule
    (`Unclaimed` → catalog tenant only, `routes/query.rs:972`), the same role
    table access and the same masking policies apply. A test pins this
    (Task 5).
- **D10. Failure handling (principles 2–4).** Exactly Gold's, because it is
  Gold's code: every `GoldExportError` is classified before it is answered or
  stored (`routes/gold.rs:83-123`); a failure after the gate is a recorded
  failed run (`DATA-56`, `routes/gold.rs:419-441`); a `409` for a publish in
  flight records nothing; `?ifChanged=true` never skips after a failed run
  (`routes/gold.rs:155-174`); a table never published reads
  `published: false`, not an error (`routes/gold.rs:930-951`); an unreadable
  catalog token is a `503` with a fixed sentence. No new error text reaches a
  response unclassified.
- **D11. Dagster.** A separate `silver_export_job` (new module
  `dagster/dispar_orchestrate/silver_export.py`) built from the same helpers
  as `gold_export_job`, made layer-aware in `gold_export.py`: its own
  schedule (`30 4 * * *`, half an hour after Gold's `0 4 * * *`, so the two
  jobs do not start in the same minute), its own after-authored sensor, on the
  serial list (`OPS-37`), with the run limit read from the **same**
  `DAGSTER_RUN_LIMIT_GOLD_EXPORT_JOB_SECONDS` (default 7200; no compose or
  `.env.example` change). No `SILVER_EXPORT_MARTS` override: the console
  switch is the only source. The `POST` timeout is 60 s for Gold (unchanged)
  and 3600 s for Silver: Silver tables are detail rows and may be near the
  5,000,000-row cap; the export time of such a table is **not measured**
  (recorded under Mismatch, M3).
- **D12. Console.** The existing `OpenFormatCard`
  (`src/features/catalog/open-format-card.tsx:153`) gets a `layer` prop and is
  shown on Silver asset pages too (`asset-overview.tsx:276`). For Silver it
  never calls a consumers route (there is none) and shows "Readers (7 days):
  Not measured" with a fixed reason; the "All published marts" link stays
  Gold-only. The overview page `/gold-exports` stays Gold-only (feature page
  decision 8, Mismatch M1).
- **D13. Assistant.** Not a dashboard feature (spec, "Asking the assistant").
  No tool, no schema, no `tool_schemas.json` change. The only lines touched in
  `routes/ai/tools/gold.rs` (`:116`, `:158`) pass `PublishedLayer::Gold` to the
  two functions that gain a layer argument; the AI team reviews those two
  lines.
- **D14. Compatible with `DATA-3` (blocked by this item).** `DATA-3` makes
  Gold "open format first" and leaves "Silver on this path" out of scope
  (`specs/data-3.md`, Not included). This plan therefore (1) keeps Silver on
  ADR 0010's publish path, (2) keys every switch and run by `(layer, name)`
  so `DATA-3` can add a per-table path column without a key change, (3) puts
  no "gold" literal into shared code: the namespace always comes from
  `PublishedLayer`. ADR 0010 gets a short amendment (Task 11) saying Silver
  uses its path; `DATA-3`'s new ADR replaces it for Gold tables only.
- **D15. Compatible with `DATA-6` (a `_latest` table next to each published
  table), which merges first.** See "Files shared with DATA-6" below. Rule: if
  `DATA-6`'s `_latest` code is in `export_mart` or `GoldTable`, Silver gets
  `silver.<t>_latest` for free; any `GOLD_NAMESPACE` / `"gold"` literal that
  `DATA-6` adds to shared code is replaced by `layer.namespace()` in this
  branch. `DATA-6`'s naming guard (a table whose own name ends in `_latest`)
  applies to Silver unchanged.

## Files shared with DATA-6 (merge order: DATA-6 first)

Both plans are expected to touch these files. `DATA-6` merges first; this
branch is then rebased onto `main` hunk by hunk (rule 9) and rebuilt before
the handoff.

| File | DATA-6 (expected) | DATA-4 (this plan) |
| --- | --- | --- |
| `rust/crates/lakehouse-iceberg/src/catalog.rs` | a replace/overwrite of `<t>_latest` | layer-aware create/load, file prefix |
| `rust/crates/lakehouse-iceberg/src/gold.rs` | the `_latest` name rule | none, or a doc line |
| `rust/crates/lakehouse-api/src/gold_export.rs` | write `_latest` after the append | `layer` argument, Gold-only grant |
| `rust/crates/lakehouse-api/src/routes/gold.rs` | `_latest` name in the read-back / publication body | layer-generic handlers, Silver handlers |
| `rust/crates/lakehouse-api/tests/gold_publication.rs` | tests | Silver tests |
| `dagster/dispar_orchestrate/gold_export.py` | trim leaves `_latest` alone | layer-aware helpers |
| `src/services/contracts/gold.ts`, `src/services/clients/gold.ts` | `_latest` name field | `OpenFormatService`, `silverService` |
| `src/features/catalog/open-format-card.tsx` (+ test) | the name with a copy button | `layer` prop |
| `docs/core/features/gold-publish-per-mart.md`, `docs/OPERATIONS.md` | `_latest` text | Silver text |

If `DATA-6` has not merged when you start, start from `6b2242ff6`, keep going,
and rebase before Task 12. If `DATA-6`'s merged code conflicts with a decision
here, stop and report (do not pick a side).

## Anchors (verified at `6b2242ff6`)

Rust, `lakehouse-iceberg`:
- `src/lib.rs:63-69` module list and re-exports.
- `src/gold.rs:43` `GOLD_NAMESPACE`; `:64` `gold_namespace()`; `:75`
  `sanitize_mart_name`; `:87` `gold_schema`; `:118` `export_day_partition_spec`;
  `:142` `partition_key_for`.
- `src/catalog.rs:284` `ensure_gold_namespace`; `:321` `create_gold_table`;
  `:351` `load_gold_table`; `:374` `load_gold_table_if_exists`; `:397`
  `create_or_load_gold_table`; `:563` `GoldTable`; `:624` `GoldTable::append`,
  file-name prefix literal `"gold"` at `:633`.
- `src/rest.rs:875-916` the two `load_gold_table_if_exists` wiremock tests to
  mirror.

Rust, `lakehouse-store`:
- `src/gold_publication.rs:43` `GoldPublication`; `:64` `get`; `:81`
  `list_enabled`; `:99` `upsert`; row struct and `From` below.
- `src/pipelines.rs:2316-2350` `targets_for_conn` / `targets_for` (the shape
  to follow for the new reader; there is no reader by `target` today).
- `tests/gold_publication.rs` (in `tests/it.rs`).
- `rust/migrations/0054_gold_publication.sql` (the table); highest migration
  on `main` is `0151_kafka_connectors_stream.sql`.

Rust, `lakehouse-api`:
- `src/lib.rs:34-36` `pub mod gold_export; gold_export_history; gold_lock;`.
- `src/gold_export.rs:812` `GoldExportResult`; `:888` `export_mart`; `:901`
  namespace literal; `:923-931` the gate; `:971-979` create-or-load and grant;
  `:1079` `grant_outside_readers_select`; `:1156` `read_back_row_count`;
  `:1612` the test that calls `export_mart`.
- `src/gold_export_history.rs:60-93` trim-table ensure; `:101-151` run-table
  ensure; `:259` `NewGoldExportRun`; `:310` `insert_sql`; `:341`
  `record_export_run`; `:346` `list_sql`; `:357` `last_success_sql`; `:433`
  `list_export_runs`; `:458` `last_success_export`; `:484` `last_run_sql`;
  `:518` `last_export_run`; `:691` `last_trim_sql`; `:742` `last_trim`.
- `src/routes/gold.rs`: `:83` `From<GoldExportError>`; `:155`
  `if_changed_skip_reason`; `:191` `unchanged_since_last_export_reason`;
  `:247` `check_export_token`; `:311` `read_catalog_token`; `:329`
  `gold_export_gate_actor`; `:369` `gated_source_table` (source name built at
  `:374`); `:453` `export`; `:510-519` lock; `:556` `export_mart` call; `:619`
  `record_export_history`; `:669` `Attempt`; `:690` `Attempt::open`; `:801`
  `early_failure`; `:821` `early_failure_under_lock`; `:887` `read_back`;
  `:930` `read_back_body`; `:955` `ExportsQuery`; `:972` `exports`; `:1023`
  `consumers`; `:1092` `mart_exists`; `:1112` `mart_last_changed_at`; `:1172`
  `publication_body`; `:1253` `pool`; `:1282` `publications`; `:1326`
  `publication`; `:1371` `record_publication_audit`; `:1410`
  `set_publication`; tests from `:1468`.
- `src/routes/mod.rs:66` `mod gold;`; `:1436-1448` the Gold routes.
- `src/policy.rs:515-543` the Gold `POLICY_TABLE` rows.
- `src/audit_coverage.rs:246-247` the Gold `AUDIT_TABLE` rows.
- `src/routes/query.rs:948` `deployment_tenant_count` (private today);
  `:972` `table_is_reachable`; `:1017` `claim_on`.
- `src/routes/catalog.rs:157` `COULD_NOT_VERIFY_TENANT_COUNT`.
- `src/routes/schema_versions.rs:84` `SILVER_DATABASE` (private today).
- `src/sql_rewrite.rs:576` `canonicalize`.
- `src/routes/iceberg_door.rs:109` `GOLD_NAMESPACE`; `:315` `namespace_is_gold`.
- `src/routes/ai/tools/gold.rs:116` `export_mart` call; `:158`
  `read_back_row_count` call; `:162` `read_back_body` call.
- `src/state.rs:166-171` `gold_export_locks` doc and field.
- `tests/gold_publication.rs:40-104` the one shared ClickHouse per test binary
  and `spin_up_with_clickhouse`; `:180` `send`.
- `lakehouse-auth/src/permission_catalog.rs:138-141` `gold:export`.

Python, `dagster/dispar_orchestrate`:
- `gold_export.py:291` `export_one_mart` (timeout 60 at `:302`); `:308`
  `_enabled_publication_marts`; `:384` `list_gold_marts`; `:431`
  `export_gold_mart`; `:555` `ensure_gold_trim_run_table`; `:580`
  `record_gold_trim_run`; `:638` `run_trim_for_mart` (table load `:722`,
  orphan removal `:889-899`); `:960` `trim_gold_mart`; `:994`
  `summarize_gold_export`; `:1010` `gold_export_job`; `:1080`
  `evaluate_authored_success`; `:1093-1107` the sensor; `:1110` the schedule.
- `definitions.py:28-32` import; `:116`, `:135`, `:147` registration.
- `ingest_factory.py:333-341` `SERIAL_JOB_NAMES`.
- `test_job_serial_tags.py:75-81` `_SERIAL_JOBS`; `test_job_run_limits.py:89-102`
  `_JOB_SPECS`.

TypeScript:
- `src/services/contracts/gold.ts:135` `GoldPublication`; `:151` `GoldService`.
- `src/services/clients/gold.ts:63` `goldService`.
- `src/services/index.ts:21` import, `:142` export of `goldService`.
- `src/features/catalog/open-format-card.tsx:32` `trimmedMart`; `:153`
  `OpenFormatCard`; `:171-174` consumers; `:272` readers row; `:355-360`
  overview link.
- `src/features/catalog/asset-overview.tsx:276` the card is shown for Gold only.

## Tasks (one commit each, in this order)

Rust first (Tasks 1–7, one sitting: the migration lands with the store code
that uses it), then Python (8–9), then TypeScript (10), then docs (11), then
the full run (12). Per commit run only the scoped checks named in the task
("Keep build time down"). Push after every commit.

Every new or changed code site cites `DATA-4` and the decision (`D7`, `D9b`,
...) in its comment. Comments say why, never restate the next line.

### Task 1. `lakehouse-iceberg`: the layer, and layer-aware create/load

1. New file `rust/crates/lakehouse-iceberg/src/published.rs`, with a module
   doc (why: one publish path for Gold and Silver, `DATA-4` D2/D3; Silver's
   namespace is `silver`; tenant isolation stays the warehouse boundary,
   ADR 0003). Contents:
   - `pub const SILVER_NAMESPACE: &str = "silver";`
   - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum PublishedLayer { Gold, Silver }`
   - `impl PublishedLayer`:
     - `pub const fn as_str(self) -> &'static str` → `"gold"` / `"silver"`
       (the key stored in Postgres and ClickHouse history, D7).
     - `pub const fn namespace(self) -> &'static str` →
       `crate::gold::GOLD_NAMESPACE` / `SILVER_NAMESPACE`.
     - `pub fn namespace_ident(self) -> iceberg::Result<NamespaceIdent>`
       (`NamespaceIdent::from_strs([self.namespace()])`, with `# Errors`).
   - Unit tests: `the_published_layers_are_gold_and_silver_and_neither_is_bronze`
     (namespaces `gold`/`silver`, both `!= bronze::BRONZE_NAMESPACE`, `as_str`
     equals `namespace` for both), `a_silver_namespace_is_flat_and_single_level`.
2. `src/lib.rs`: `pub mod published;` (keep alphabetical order) and add
   `PublishedLayer` to the `pub use` line.
3. `src/catalog.rs`: add layer-aware versions and make the Gold ones delegate:
   - `ensure_published_namespace(&self, layer)`; `ensure_gold_namespace()`
     becomes `self.ensure_published_namespace(PublishedLayer::Gold)`.
   - `create_published_table(&self, layer, name, domain_fields)`,
     `load_published_table(&self, layer, name)`,
     `load_published_table_if_exists(&self, layer, name)`,
     `create_or_load_published_table(&self, layer, name, domain_fields)`:
     the bodies of the Gold functions with `gold::gold_namespace()` replaced
     by `layer.namespace_ident()`. The four Gold functions become one-line
     delegations with `PublishedLayer::Gold` (keep their doc comments, add one
     line "the Gold case of [`Self::..._published_...`]").
   - All of them still return `GoldTable`. Add one doc paragraph on
     `GoldTable` (`:556-565`): "Also the type of a published Silver table
     (`DATA-4` D2): the schema, partition and append are identical; the name
     stays to keep `DATA-6`'s and the assistant's call sites unchanged."
   - `GoldTable::append` (`:632-636`): the data-file prefix is the table's
     namespace instead of the literal `"gold"`
     (`self.table.identifier().namespace().to_url_string()`), so a Silver
     file is not named `gold-...`. Gold files keep the prefix `gold`.
4. `src/rest.rs` tests (next to `:875-916`): add
   `a_silver_table_the_catalog_does_not_have_loads_as_none` (mock
   `GET /v1/namespaces/silver/tables/never` → 404, call
   `load_published_table_if_exists(PublishedLayer::Silver, "never")`, expect
   `Ok(None)`) and `a_catalog_that_refuses_the_caller_is_still_an_error_for_a_silver_table`
   (401 → `IcebergError::Catalog`).

Acceptance: the four new tests fail on `6b2242ff6` (they do not compile:
`PublishedLayer` does not exist) and pass now. Every existing test of the
crate passes unchanged.
```
cd rust && cargo fmt --check && cargo clippy -p lakehouse-iceberg --all-targets -- -D warnings && cargo test -p lakehouse-iceberg
```

### Task 2. Migration and store: the switch is keyed by layer

1. New migration `rust/migrations/NNNN_publication_layer.sql`, `NNNN` = the
   next free number above the highest on `main` at build time (`0152` at
   `6b2242ff6`; check `ls rust/migrations` and the open PRs first). Why-header
   (`DATA-4` D7: one switch table for Gold and Silver; a Silver table and a
   Gold mart of the same name are two rows; existing rows are Gold; never
   edited once applied). Body:
   ```sql
   ALTER TABLE gold_publication
       ADD COLUMN layer TEXT NOT NULL DEFAULT 'gold'
           CHECK (layer IN ('gold', 'silver'));
   ALTER TABLE gold_publication DROP CONSTRAINT gold_publication_pkey;
   ALTER TABLE gold_publication ADD PRIMARY KEY (layer, mart);
   ```
   (`gold_publication_pkey` is Postgres's default name for the `0054`
   primary key; confirm with `\d gold_publication` on a migrated database
   before committing, and say so in the commit body.)
2. `rust/crates/lakehouse-store/src/gold_publication.rs`:
   - `GoldPublication` gains `pub layer: String` (serialised as `layer`).
   - `get(pool, layer: &str, mart)`, `list_enabled(pool, layer: &str)`,
     `upsert(pool, layer: &str, mart, enabled, updated_by)`: every statement
     binds `layer` (`WHERE layer = $1 AND mart = $2`, `INSERT (layer, mart,
     ...)`, `ON CONFLICT (layer, mart)`), `RETURNING layer, ...`. Values are
     bound, never formatted. Module doc: one line on `DATA-4` D7.
3. `rust/crates/lakehouse-store/src/pipelines.rs`: new
   `pub async fn tenants_writing_target(pool: &PgPool, target: &str) ->
   Result<Vec<Option<Uuid>>, StoreError>` — `SELECT tenant_id FROM
   pipeline_definition WHERE target = $1`, every status included (an archived
   pipeline's data may still be in the table; `DATA-4` D9b). `# Errors`.
4. Store tests, `rust/crates/lakehouse-store/tests/gold_publication.rs`
   (`#[sqlx::test]`, existing file, already in `tests/it.rs`): update the
   existing calls to pass `"gold"`, and add
   - `a_silver_and_a_gold_publication_of_the_same_name_are_two_rows`
   - `list_enabled_lists_only_the_layer_it_is_asked_for`
   - `a_row_written_without_a_layer_is_gold` (raw `INSERT INTO
     gold_publication (mart, enabled, updated_by)` → `get(pool, "gold", ..)`
     returns it, `get(pool, "silver", ..)` returns `None`)
   - `a_layer_other_than_gold_or_silver_is_refused_by_the_table` (raw insert
     with `layer = 'bronze'` fails).
   And in `tests/pipelines.rs`: `the_tenants_of_every_pipeline_writing_a_target_are_listed`
   (two pipelines, one `NULL` tenant, one other target → exactly the two
   tenants of the matching target). Insert rows as
   `lakehouse-api/tests/lakehouse_tenant_scope.rs:219-231` does.

Acceptance: the new store tests fail on `6b2242ff6` and pass now.
```
cd rust && cargo fmt --check && cargo clippy -p lakehouse-store --all-targets -- -D warnings && cargo test -p lakehouse-store --test it gold_publication:: && cargo test -p lakehouse-store --test it pipelines::
```
`lakehouse-api` will not compile after this commit until Task 4; that is
expected inside the Rust sitting. Do not push a "fix" that skips the layer.

### Task 3. History tables keyed by layer

`rust/crates/lakehouse-api/src/gold_export_history.rs`:
1. `ensure_gold_export_run_table_uncached` (`:108`) and
   `ensure_gold_trim_run_table_uncached` (`:67`): after the existing `ALTER`,
   add `ALTER TABLE console.gold_export_run ADD COLUMN IF NOT EXISTS layer
   LowCardinality(String) DEFAULT 'gold'` (and the same for
   `console.gold_trim_run`). Also add `layer` to both `CREATE TABLE`
   statements so a fresh install has it from the start. Comment: `DATA-4` D7,
   rows written before are Gold.
2. `NewGoldExportRun` gains `pub layer: &'a str`; `insert_sql` writes it.
3. `list_sql`, `last_success_sql`, `last_run_sql`, `last_trim_sql` take
   `layer: &str` and add `AND layer = {SqlLiteral::from(layer)}`; the public
   functions `list_export_runs`, `last_success_export`, `last_export_run`,
   `last_trim`, `last_success_started_at`, `last_success_fingerprint` take
   `layer: &str` as their second argument.
4. Unit tests (in the existing `mod tests`): `every_history_read_filters_on_the_layer`
   (each of the four builders contains `layer = 'silver'` for `"silver"`),
   `the_insert_writes_the_layer`, `both_history_tables_gain_a_layer_column_that_defaults_to_gold`
   (factor the two `ALTER` statements into `const` strings and assert on them).

Acceptance: the new unit tests fail on `6b2242ff6` and pass now (run after
Task 4, when the crate compiles again).

### Task 4. Export and routes: one generic path, Silver handlers

`rust/crates/lakehouse-api/src/gold_export.rs`:
1. `export_mart` (`:888`) gains `layer: lakehouse_iceberg::PublishedLayer`
   right after `actor`; update the `#[allow(clippy::too_many_arguments,
   reason = ...)]` reason with one clause for `DATA-4`. Inside: `namespace =
   layer.namespace().to_owned()` (`:901`);
   `create_or_load_published_table(layer, ...)` (`:972`); the grant (`:979`)
   runs only `if grants_outside_readers(layer)`.
2. New `pub(crate) const fn grants_outside_readers(layer: PublishedLayer) ->
   bool { matches!(layer, PublishedLayer::Gold) }`, doc: ADR 0016 point 2,
   the door serves `gold` only; granting `silver` would open Silver to every
   outside tool (`DATA-4` D9c). Unit test
   `only_a_gold_publish_grants_the_outside_readers`.
3. `GoldExportResult.namespace` doc: "`gold` or `silver`".
4. `read_back_row_count(iceberg_config, layer, mart_name)` uses
   `load_published_table_if_exists(layer, ..)`.
5. New `pub(crate) fn source_schema(state: &AppState, layer) -> &str` (Gold →
   `state.config.gold_source_schema`, Silver →
   `crate::routes::schema_versions::SILVER_DATABASE`, made `pub(crate)`) and
   `pub(crate) fn lock_key(layer, name: &str) -> String` (Gold → `name`,
   Silver → `format!("silver.{name}")`, D8). Unit tests
   `a_gold_lock_key_is_the_bare_name_and_a_silver_one_is_qualified`,
   `silver_reads_from_the_silver_database`.
6. Update the existing test call at `:1612` (`PublishedLayer::Gold`).

`rust/crates/lakehouse-api/src/routes/gold.rs` (module doc: one section
"Silver (`DATA-4`)" saying the same handlers serve `/api/silver/...`, the
wire names, the extra gates and the Gold-only consumers):
7. Give every private helper a `layer` argument and use the Task 4.5 helpers
   instead of `state.config.gold_source_schema` / the bare mart:
   `unchanged_since_last_export_reason`, `gated_source_table` (source name
   `format!("{}.\`{}\`", source_schema(state, layer), mart)`), `Attempt`
   (new field `layer`; `Attempt::open` uses `source_schema`),
   `read_content_fingerprint`, `early_failure` / `early_failure_under_lock`
   (lock with `lock_key`), `record_export_history` (writes `layer`),
   `mart_last_changed_at` caller, `publication_body` (adds
   `"layer": layer.as_str()`), `read_back_body(layer, mart, readback)`
   (namespace from the layer), `record_publication_audit` (Gold:
   `"gold.publication_set"` / `"gold_mart"` unchanged; Silver:
   `"silver.publication_set"` / `"silver_table"` — both literals must appear
   in this file, the audit guard test greps for them).
8. Turn each Gold handler body into a `*_for(layer, ...)` function and keep
   the public Gold handler as a one-line call with `PublishedLayer::Gold`:
   `export_for`, `read_back_for`, `exports_for`, `publications_for`,
   `publication_for`, `set_publication_for`. Add the Silver handlers as
   one-line calls with `PublishedLayer::Silver`: `silver_export`,
   `silver_read_back`, `silver_exports`, `silver_publications`,
   `silver_publication`, `set_silver_publication`. `consumers` stays Gold-only.
9. Silver-only steps inside the `*_for` functions (each with a `DATA-4 D9`
   comment):
   - `read_back_for` and `exports_for`: for Silver, call `gated_source_table`
     with `gold_export_gate_actor(principal, &headers)` before reading
     anything (Gold is not changed: follow-up F1).
   - `export_for`: for Silver, right after `Attempt::open`, call
     `crate::silver_publish::ownership_refusal(&state, mart)` (Task 5); an
     `Err` goes through `early_failure(&attempt, "", err)` so it is recorded
     (D9b).
   - `set_publication_for`: for Silver, after the 404 existence check and
     before `upsert`, call `ownership_refusal`; an `Err` is returned as it is
     (nothing stored, nothing recorded). Also gate the PUT for Silver through
     `gated_source_table` before the existence check, so a person outside the
     reach gets the gate's 404 and the table's existence is never answered.
   - `pool()`'s sentence becomes layer-neutral: "open-format publications
     unavailable: ..." (grep the tests for the old text and update them).
10. `src/routes/mod.rs` after `:1448`: the six Silver routes (D5), with a
    comment `DATA-4: the same handlers as Gold, POLICY_TABLE mirrors them`.
11. `src/policy.rs` after `:543`: six rows, `RequiresAuth` for the five reads
    and the `POST`, `RequiresPermission("gold:export")` for the `PUT`, with a
    comment (D6).
12. `src/audit_coverage.rs` after `:247`:
    `("POST", "/api/silver/export/{mart}", Coverage::Central { action: "silver.export", resource_kind: "silver_table", id_param: IdParam::Path("mart") })`
    and `("PUT", "/api/silver/export/{mart}/publication", Coverage::Handler { action: "silver.publication_set" })`.
13. `src/routes/ai/tools/gold.rs:116`, `:158`, `:162`: pass
    `PublishedLayer::Gold` (D13). No other line there changes.
14. `src/state.rs:166-170`: the lock doc names both route families.
15. `lakehouse-auth/src/permission_catalog.rs:140`: the description of D6.

Acceptance: compiles; every existing `lakehouse-api` test still passes with
Gold behaviour unchanged (route_auth's `POLICY_TABLE` loops now also walk the
six new rows both ways; `tests/audit_coverage.rs` passes with the two new
rows and fails if either is removed).
```
cd rust && cargo fmt --check && cargo clippy -p lakehouse-api --all-targets -- -D warnings
cargo test -p lakehouse-api --lib gold_export && cargo test -p lakehouse-api --lib routes::gold
cargo test -p lakehouse-api --test it route_auth:: && cargo test -p lakehouse-api --test it audit_coverage:: && cargo test -p lakehouse-api --test it permission_catalog::
```

### Task 5. The ownership check (D9b) and the governance guard (D9c)

New file `rust/crates/lakehouse-api/src/silver_publish.rs`
(`pub(crate) mod silver_publish;` in `src/lib.rs`), module doc: why a Silver
table built from another organisation's data is not published, `SEC-20`
decision 5 for `NULL` tenants, `SEC-80` for the reach, and that the same gap
on Gold is follow-up F2.
1. `pub(crate) const SILVER_OF_ANOTHER_TENANT: &str = "This Silver table is
   built from data another organisation owns, so it is not published in open
   format.";` It names nothing about which organisation or which table
   (principle 4).
2. A pure function, unit-tested on every branch:
   ```rust
   pub(crate) fn holds_another_tenants_data(
       tenant_count: i64,
       catalog_tenant: Option<Uuid>,
       upload_claim: TableClaim,          // as the catalog's tenant sees it
       writer_tenants: &[Option<Uuid>],
   ) -> bool
   ```
   `tenant_count <= 1` → `false`; `catalog_tenant == None` → `true`;
   `upload_claim == TableClaim::Theirs` → `true`; any `Some(t)` in
   `writer_tenants` with `Some(t) != catalog_tenant` → `true`; else `false`.
3. `pub(crate) async fn ownership_refusal(state: &AppState, table: &str) ->
   Result<(), ApiError>`: no Postgres pool → `Err(ApiError::Unavailable(
   COULD_NOT_VERIFY_TENANT_COUNT))`; tenant count through
   `routes::query::deployment_tenant_count` (make it `pub(crate)`; `None` →
   the same 503); claim through `routes::query::claim_on(state,
   catalog_tenant, &format!("bronze.{table}"))` only when the catalog tenant
   is known; writers through `lakehouse_store::pipelines::tenants_writing_target(
   pool, &format!("silver.{table}"))`, a store error classified with
   `upstream_error::store(&upstream_error::STORE, ..)` as `claim_on` does;
   `true` → `Err(ApiError::Unprocessable(SILVER_OF_ANOTHER_TENANT.to_owned()))`.
   `# Errors`.
4. Unit tests: one per branch of the pure function (sentence names, e.g.
   `one_tenant_publishes_whatever_wrote_the_table`,
   `several_tenants_and_no_catalog_tenant_publish_nothing`,
   `an_upload_of_another_tenant_with_the_same_name_is_refused`,
   `a_pipeline_of_another_tenant_writing_the_table_is_refused`,
   `a_pipeline_with_no_tenant_counts_as_the_catalogs`), and the guard test
   `the_published_silver_copy_is_governed_under_the_silver_tables_own_name`:
   `sql_rewrite::canonicalize(&["icecat_api".into(), "silver.orders".into()])
   == "silver.orders"`, the same for `["iceberg", "silver", "orders"]`, and
   `table_is_reachable(TenantReach::Tenant { tenant_id, owns_catalog: false },
   TableClaim::Unclaimed) == false`. Its doc says what breaks if it fails (an
   outside-the-catalog tenant could read the copy inside the product).

Acceptance: the tests fail on `6b2242ff6` (module absent) and pass now.
```
cd rust && cargo fmt --check && cargo clippy -p lakehouse-api --all-targets -- -D warnings && cargo test -p lakehouse-api --lib silver_publish
```

### Task 6. HTTP tests for Silver

All in the existing `rust/crates/lakehouse-api/tests/gold_publication.rs`,
**not** a new file: that file owns the one ClickHouse per test binary
(`:40-80`), and a second container would race
`gold_export_history`'s process-wide "table ensured" cache. Use
`spin_up_with_clickhouse` (`:90`), a `gold:export` principal, and table names
used by no other test. Seed `silver.<t>` with `CREATE DATABASE IF NOT EXISTS
silver` then a `MergeTree` table. Each test name is a sentence; each fails on
`6b2242ff6` (the Silver routes answer 404 there):
1. `a_silver_table_switched_on_is_listed_by_the_silver_publications_and_not_by_gold`
2. `a_gold_mart_and_a_silver_table_of_the_same_name_keep_separate_switches`
3. `switching_on_a_silver_table_that_does_not_exist_is_404`
4. `a_silver_table_never_published_reads_never_published` (detail body:
   `layer: "silver"`, `enabled: false`, `lastExportedAt: null`,
   `lastFailure: null`; read-back `published: false`, `namespace: "silver"`)
5. `an_unchanged_silver_table_is_skipped_under_if_changed` (seed a success
   run with `layer: "silver"` and the current fingerprint, as `:496-560` does
   for Gold)
6. `a_failed_silver_run_never_shows_on_a_gold_mart_of_the_same_name`
7. `a_silver_publish_with_a_dead_catalog_records_a_classified_failure`
   (mirror `catalog_down_mart`, `:780-830`: the stored and returned error is
   the classified sentence, never catalog text)
8. `a_silver_table_another_tenants_upload_claims_is_refused_the_switch_and_nothing_is_stored`
   (insert `upload_table_claim (bronze_table, tenant_id, upload_id)` with a
   seeded tenant that is not `CATALOG_TENANT`, see
   `rust/migrations/0002_seed_identity.sql`; `PUT` → 422 with
   `SILVER_OF_ANOTHER_TENANT`; `GET` detail still `enabled: false`)
9. `a_silver_table_another_tenants_pipeline_writes_is_refused_the_switch`
10. `a_scheduled_silver_publish_of_another_tenants_table_is_a_recorded_failure`
    (switch on first, then add the claim, then `POST ?ifChanged=true` with the
    run token → 422; `GET /api/silver/exports?mart=` has one `failed` row with
    that sentence; the detail body's `lastFailure.reason` is that sentence)
11. `a_person_whose_tenant_does_not_own_the_catalog_gets_the_gates_404_on_every_silver_route`
    (detail, PUT, read-back, history, export)
12. `an_analyst_sees_the_silver_switch_state_but_cannot_change_it`
    (`canEdit: false`; `PUT` 403)

Acceptance:
```
cd rust && cargo test -p lakehouse-api --test it gold_publication:: && cargo test -p lakehouse-api --test it gold_export_auth::
```
If Docker is not available on the machine, say *not verified* in the handoff
and leave the tests in; CI runs them.

### Task 7. Rust sitting closes

No new code. Run the whole of each touched crate once, so the next three
tasks start from a green Rust tree:
```
cd rust && cargo fmt --check && cargo clippy -p lakehouse-iceberg -p lakehouse-store -p lakehouse-auth -p lakehouse-api --all-targets -- -D warnings
cargo test -p lakehouse-iceberg && cargo test -p lakehouse-store && cargo test -p lakehouse-auth
cargo test -p lakehouse-api --lib && cargo test -p lakehouse-api --test it gold_ && cargo test -p lakehouse-api --test it route_auth:: && cargo test -p lakehouse-api --test it audit_coverage:: && cargo test -p lakehouse-api --test it sec11_guard::
```
(This is a checkpoint, not a commit. The full workspace run is Task 12.)

### Task 8. Dagster: layer-aware helpers in `gold_export.py`

`dagster/dispar_orchestrate/gold_export.py` (module doc: one paragraph "Silver
(`DATA-4`)" pointing at `silver_export.py`):
1. `export_one_mart(cfg, mart, layer="gold")`: `POST
   {api_url}/api/{layer}/export/{mart}`; timeout from a module dict
   `PUBLISH_REQUEST_TIMEOUT_SECONDS = {"gold": 60, "silver": 3600}` with a
   comment: Gold unchanged; Silver is detail rows up to the 5,000,000-row cap
   and its export time is not measured (D11, Mismatch M3).
2. `_enabled_publication_marts(cfg, layer="gold")`: `GET
   /api/{layer}/publications`; the `Failure` messages name the route they
   called.
3. Move the body of `export_gold_mart` into `_export_and_record(context, cfg,
   layer, mart)`: `record_maintenance_run(table_name=f"{layer}.{mart}")` and
   `AssetMaterialization(asset_key=f"{layer}.{mart}")`. `export_gold_mart`
   calls it with `"gold"`.
4. Move the fan-out body of `list_gold_marts` into
   `_publication_outputs(context, names_with_sources)` (dedup and the
   mapping-key collision `Failure`), used by both layers.
5. `ensure_gold_trim_run_table` adds the same `layer` column as Task 3
   (`ALTER TABLE console.gold_trim_run ADD COLUMN IF NOT EXISTS layer
   LowCardinality(String) DEFAULT 'gold'`, and the column in the `CREATE`).
   `record_gold_trim_run(..., layer="gold")` writes it.
6. `run_trim_for_mart(cfg, mart, context=None, layer="gold")`: every
   `f"gold.{mart}"` becomes `f"{layer}.{mart}"` (table load `:722`, orphan
   removal `:895`, every log line), and every `record_gold_trim_run` call
   passes `layer`.
7. Gold behaviour is unchanged: every existing test in `test_gold_export.py`
   passes without edits except where it asserts the exact SQL of the trim
   table (update those to include `layer`, and say why in the commit body).

New tests in `test_gold_export.py` (names are sentences, no network):
`test_a_silver_export_posts_to_the_silver_route_and_waits_longer`,
`test_the_silver_publications_are_read_from_the_silver_route`,
`test_a_silver_trim_loads_the_table_from_the_silver_namespace_and_records_the_layer`,
`test_a_gold_trim_still_records_the_gold_layer`.

Acceptance: the four new tests fail on `6b2242ff6` and pass now; the module
suite passes.
```
python3 ops/lint/check_intra_package_imports.py && (cd dagster && python -m pytest dispar_orchestrate/test_gold_export.py -q)
```

### Task 9. Dagster: `silver_export_job`

1. New `dagster/dispar_orchestrate/silver_export.py`, narrative module doc
   (D11; why a separate job: a Silver failure does not fail the Gold run, the
   two are serial each on their own; no env override; same token and run
   limit), `from __future__ import annotations`. Ops, each with
   `retry_policy=DEFAULT_RETRY_POLICY` and `source_metadata(...)` with
   non-empty `writes`, named `list_silver_tables`, `export_silver_table`,
   `trim_silver_table`, `summarize_silver_export`, each a thin call into the
   Task 8 helpers with `layer="silver"`. Job:
   `@job(tags=job_tags("silver_export_job", gold_export.RUN_LIMIT_ENV,
   gold_export.DEFAULT_RUN_LIMIT_SECONDS))` — the Gold setting, on purpose
   (D11). Schedule `silver_export_schedule`, cron `"30 4 * * *"`,
   `default_status=DefaultScheduleStatus.RUNNING` with the reason (the same
   one `gold_export.py:1028-1041` gives). Sensor
   `silver_export_after_authored_sensor`: `run_status_sensor(SUCCESS,
   monitored_jobs=None, request_job=silver_export_job,
   default_status=RUNNING)` whose body is
   `yield from gold_export.evaluate_authored_success(context)`.
2. `ingest_factory.py:333-341`: add `"silver_export_job"` to
   `SERIAL_JOB_NAMES` (it writes the same tables a second run would).
3. `definitions.py`: import and register the job, the schedule and the sensor
   next to Gold's, with a comment.
4. Tests: `test_job_serial_tags.py` `_SERIAL_JOBS` gains
   `(silver_export, "silver_export_job")`; `test_job_run_limits.py`
   `_JOB_SPECS` gains `_JobSpec(silver_export, "silver_export_job",
   "DAGSTER_RUN_LIMIT_GOLD_EXPORT_JOB_SECONDS", 7_200)`. New
   `test_silver_export.py`:
   `test_the_silver_job_publishes_each_switched_on_table_then_trims_it`,
   `test_a_skipped_silver_export_is_not_trimmed`,
   `test_the_silver_sensor_fires_only_after_an_authored_pipeline`,
   `test_the_silver_schedule_runs_half_an_hour_after_gold`.

Acceptance: the new tests fail on `6b2242ff6` (no module) and pass now; the
whole code location suite passes (the definitions load test includes the new
job).
```
python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py && (cd dagster && python -m pytest dispar_orchestrate -q)
```

### Task 10. Console: the card on Silver tables

1. `src/services/contracts/gold.ts`:
   `export type OpenFormatLayer = "gold" | "silver"`; `GoldPublication` gains
   `layer: OpenFormatLayer | string` (the API always sends it now, D5; not
   optional); split the interface:
   `export interface OpenFormatService { getLastExport; triggerExport;
   listExportRuns; getPublication; setPublication }` and
   `export interface GoldService extends OpenFormatService { listMarts;
   getConsumers }`. Doc: Silver has no consumers route (D9c).
2. `src/services/clients/gold.ts`: `function openFormatClient(layer:
   OpenFormatLayer): OpenFormatService` holding the five methods with
   `/api/${layer}/...` URLs (all through `apiFetch` via the existing
   helpers); `goldService: GoldService = { ...openFormatClient("gold"),
   listMarts, getConsumers }`; `export const silverService: OpenFormatService
   = openFormatClient("silver")`.
3. `src/services/index.ts`: export `silverService` the way `goldService` is
   exported (`:21`, `:142`).
4. `src/features/catalog/open-format-card.tsx`:
   - props `{ assetId: string; layer: OpenFormatLayer }`;
     `trimmedMart(assetId, layer)` strips `serving.` for Gold and `silver.`
     for Silver; `const service = layer === "gold" ? goldService :
     silverService` and every call uses `service` (`LastFiveRuns` gets the
     service as a prop).
   - Readers: the hook stays unconditional; for Silver it resolves to `null`
     without a request, and the row reads "Not measured" with the title
     "Outside tools read published Silver tables through the catalog, not
     through the console's door, so reads are not counted (ADR 0016)."
   - Description for Silver: "A copy in open Iceberg format, in the catalog's
     silver namespace. Publishing adds one full copy each time; old copies
     stay." Gold's text unchanged.
   - The "All published marts" link renders for Gold only.
5. `src/features/catalog/asset-overview.tsx:276`: show the card for
   `a.layer === "gold" || a.layer === "silver"`, passing `layer`.
6. Tests:
   - `src/services/clients/gold.test.ts`: `the silver client calls the silver
     routes` (publication `GET`/`PUT`, export, history, read-back URLs).
   - `src/features/catalog/open-format-card.test.tsx`: a Silver card
     `never asks for consumers and says readers are not measured`,
     `publishes through the silver route`, `has no link to the Gold overview`,
     `shows the switch disabled with the permission sentence for an Analyst`;
     add `layer: "gold"` to the existing fixtures.

Acceptance: the new tests fail on `6b2242ff6` and pass now.
```
bun run typecheck && bun run lint && bun test src/services/clients/gold.test.ts src/features/catalog/open-format-card.test.tsx src/lib/gold-freshness.test.ts src/features/gold/gold-exports-page.test.tsx
```

### Task 11. Docs

1. `docs/adr/0010-gold-export-to-iceberg-from-rust.md`: append a section
   `## Amendment — Silver (DATA-4, 2026-11-20)`: Silver tables use this
   decision's path unchanged, into the `silver` namespace; the door
   (ADR 0016) still serves `gold` only; `DATA-3`'s ADR replaces this
   decision for Gold tables that switch, not for Silver.
2. `docs/OPERATIONS.md`, after "Outside tools read published tables
   (DATA-5, ADR 0016)" (`:2825`): `## Silver tables in open format (DATA-4)`
   — where they land, that the same settings apply (`GOLD_EXPORT_MAX_ROWS`,
   `GOLD_EXPORT_BATCH_SIZE`, `GOLD_KEEP_COPIES`, `GOLD_EXPORT_RUN_TOKEN`,
   `DAGSTER_RUN_LIMIT_GOLD_EXPORT_JOB_SECONDS`), that the door does not serve
   them, how an operator reads one for `DATA-4-AC2` (a `pyiceberg`
   `RestCatalog` against the catalog with an operator token, filtered on the
   newest `_exported_at`, or Trino `iceberg.silver.<t>` in Query Studio as
   the catalog tenant), and the ownership refusal sentence.
3. `docs/CONFIGURATION.md:177-179`, `:229`, `:255`: say "Gold and Silver"
   where the setting now serves both.
4. `docs/core/features/silver-publish.md` "Limits to tell a customer": add
   the lines "Outside tools do not read Silver through the console's door;
   their reads are not counted.", "A Silver table built from another
   organisation's data is not published.", "The overview of published tables
   lists Gold only.", "The Usage page does not count the Silver copy's
   storage yet."
5. `docs/core/features/gold-publish-per-mart.md` acceptance row 3: "Open a
   raw (Bronze) table | No publish switch" (a Silver table has one since
   `DATA-4`), and under "Not included" drop "Publishing Silver" with a
   pointer to `silver-publish.md`.

Acceptance: `git diff --stat` shows only these five files; no product claim
in them that the code does not do.

### Task 12. Full verification and handoff

Rebase onto `origin/main` (after `DATA-6` merged) hunk by hunk, rebuild, then
run the full block below once, in the foreground, and write the Handoff
entry.

## Mismatch (targets or rows this plan does not meet; for the owner)

- **M1. Gold acceptance row 17 ("Open the overview of all tables") is not
  carried over to Silver.** `DATA-4-AC3` says the Gold rows that do not name
  Gold pass for Silver. Row 17 does not name Gold, but the overview page
  (`src/features/gold/gold-exports-page.tsx`, "Gold Exports") lists
  `serving.*` marts only and asks three routes per row; adding Silver is new
  UI (a second section or a layer column), which the owner's 2026-11-04 rule
  sends to an Opus developer. Feature page decision 8 records it. Owner:
  accept the exception, or order the overview as a follow-up (F4).
- **M2. Gold row 3 ("Open a Silver or raw table → No publish switch") becomes
  false by design.** Task 11.5 corrects that row.
- **M3. Whether a 5,000,000-row Silver table publishes inside the run limit
  is not measured.** The row cap is the signed default; the export time of
  that size has never been measured for Gold or Silver. The Silver `POST`
  waits 3600 s and the job may run 7200 s. The acceptance run should record
  one large-table timing in a `docs/plans/*-RESULT.md`; if it does not fit,
  that is a finding, not a lowered cap.

## Follow-ups (for the reviewer to add to `BACKLOG.md`)

- F1. Gold's read-back and history routes are not tenant-gated (Silver's are,
  D9a).
- F2. A Gold mart built by another tenant's pipeline is published like any
  other (the D9b check exists for Silver only).
- F3. Outside tools reading Silver through the door (ADR 0016 amendment,
  `outside-readers` grant on `silver`, consumers route).
- F4. Silver on the overview page (UI, Opus developer).
- F5. The Usage page (`storage_snapshot.py`, `ICEBERG_LAYERS = ("bronze",
  "gold")`) does not measure the Iceberg `silver` namespace, whose layer name
  is already the ClickHouse `silver` database's.

## PR slicing

One PR, `data-4-silver-publish` → `main`, Tasks 1–11 as eleven commits (Task
7 and 12 are checks, not commits). Title: `DATA-4 feat(data): Silver tables
publish in open format, as Gold does`. It touches a migration, so CI's full
Rust run is required; it touches no dependency, so a red dependency check on
`main` does not block it (but must be named in the review).

## Out of scope

- Storing Silver in open format first (`DATA-3` covers Gold only).
- The door, the consumers count and the overview page for Silver (F3, F4).
- A Silver-specific row cap, copy count, run limit, token or permission: one
  setting serves both layers (D1, D6, D11).
- Changing anything about Gold's behaviour beyond the `layer` column, the
  file-name prefix (unchanged for Gold), the neutral `pool()` sentence and
  the permission description.
- Any compose, `.env.example` or dependency change.
- The assistant (D13).

## Verification (on the final commit, in the foreground)

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py
(cd dagster && python -m pytest dispar_orchestrate -q)
docker compose --profile '*' config --quiet   # no compose edit in this plan; run it anyway (SEC-18 env: sh ops/init-env.sh or throwaway values)
git status                                    # nothing generated staged
```
Quote each command with its counts in the handoff (rule 7). A command not run
is *not verified*, with the reason.

## Handoff

(empty — the developer appends one entry per PR slice: commits, the exact
commands run with their counts, anything skipped or not verified and why)

## Review

(empty — the reviewer writes findings here, tagged `BLOCKER` or `SHOULD-FIX`)
