# TODO — Copilot answers in Bronze / Silver / Gold terms

Status: **not started**. Found on 2026-09-25 while reviewing the Copilot chat
UI; left for a separate piece of work. Everything below is in the Rust API
(`rust/crates/lakehouse-api/src/routes/ai/`); no frontend change is needed.

## The symptom

Asked "what data do we have?", Copilot lists datasets as **"primer" /
"sekunder"** and never mentions the lakehouse layers. A user expects the
answer in medallion terms: what is raw in **Bronze**, what is cleaned in
**Silver**, what is served from **Gold**.

## Why it happens (verified in code and on the local stack)

1. **`tier` is not a layer.** The catalog column `tier` records where a
   dataset comes from: `primer` = the tenant's own source systems, `sekunder`
   = third-party feeds and processed extracts (`tenant.rs:73-102`,
   `NAMESPACE_PRIMER` / `NAMESPACE_SEKUNDER`). It says nothing about
   Bronze/Silver/Gold.
2. **The tools hand the model only that column.**
   - `list_datasets` (`tools/data.rs:124`) returns `{slug, title, tier}`.
   - `describe_dataset` (`tools/data.rs:156`) returns `tier`, the columns,
     and a row count from `silver.<table_name>`.
   - Their schemas (`registry.rs:112`, `:120`) describe the filter as
     "tier primer|sekunder".

   No tool reports which layers a dataset exists in, so the model repeats
   `tier`.
3. **The system prompt never explains the layers.** `SYSTEM_BASE`
   (`routes/ai/mod.rs:96`) is written in Indonesian and hardcoded to one
   tenant's domain and region. It points the model at `serving.mart_*` and
   `list_datasets`, but never says what Bronze, Silver and Gold are.
   `SYSTEM_ASK_SUFFIX` / `SYSTEM_BUILD_SUFFIX` (`:98`, `:100`) are also
   Indonesian.
4. **`get_lineage` hardcodes one source name.** `tools/data.rs:244`
   labels every primary dataset's source as one specific tenant's open-data
   portal, regardless of tenant. Its chain also stops at Silver, with no
   Gold step.
5. **Seed data disagrees with the tools** (local `lakehouse-run` stack
   only). `bronze_meta.dataset_catalog.table_name` holds Gold mart names
   (`mart_event`, …). `describe_dataset` then counts `silver.mart_event`,
   which does not exist, and falls back to `rows: 0`. Silver on that stack
   is `silver.dtw_kunjungan`, `event_tahunan`, `wisman_bulanan`. The
   dataset → table mapping per layer is therefore not something the tools
   can assume from one column.

What exists per layer, as seen on the local stack's ClickHouse:

| Layer | Where | How to list it |
| --- | --- | --- |
| Bronze metadata | ClickHouse `lake` database: `bronze_meta.dataset_catalog`, `dataset_column`, `dataset_sync`, plus `bronze_meta_sec.*` | `CATALOG_UNION` (`tools/data.rs:18`) |
| Bronze data | Iceberg tables behind Lakekeeper | `routes/lakehouse.rs` `namespaces` (`:506`) / `tables` (`:559`) |
| Silver | ClickHouse `silver.*` | `system.tables WHERE database = 'silver'` |
| Gold | ClickHouse `serving.mart_*` | `system.tables WHERE database = 'serving'` (already used by `describe_mart`) |

## What to do

1. **Rewrite the system prompt** (`SYSTEM_BASE` and both suffixes):
   - English and tenant-neutral: no tenant name, region or domain in code
     (AGENTS.md rule 12).
   - Explain the three layers in two or three lines each: Bronze = raw
     landed data in Iceberg, Silver = typed/cleaned ClickHouse `silver.*`,
     Gold = serving marts `serving.mart_*`.
   - Say plainly that primary/secondary is a **source** category, never a
     layer.
   - Tell the model to answer in the user's language.
   - Any tenant-specific domain hint should come from tenant config or the
     page context, not a constant.
2. **Report layers from the dataset tools.** Relabel `tier` in tool output
   as `sourceKind` with value `"primary source"` or `"secondary source"`.
   Add a `layers` object built from what really exists:
   ```json
   "layers": {
     "bronze": { "table": "…", "present": true },
     "silver": { "table": "…", "rows": 720, "present": true },
     "gold":   [ { "mart": "serving.mart_…", "rows": 6 } ]
   }
   ```
   A missing layer is `"present": false` with no row count, never a
   fabricated `0` (AGENTS.md principle 2). Resolve names against real
   tables (`system.tables`, the Iceberg listing) instead of assuming
   `silver.<catalog table_name>`.
3. **Add a read-only `lakehouse_overview` tool** (`Risk::Read`) that lists
   each layer's tables with row counts and last update: Bronze via the
   Iceberg listing, Silver and Gold via `system.tables`. It answers "what
   data do we have?" in layer terms. Register it in `registry.rs`, gate it
   like the other read tools, and add it to `POLICY_TABLE` /
   `tests/route_auth.rs` only if it gets a route.
4. **Fix `get_lineage`.** Take the source label from the tenant's source
   configuration instead of the hardcoded name, and extend the chain to the
   Gold marts built from the dataset.
5. **Classify tool errors.** `get_quality` (`tools/data.rs:262`) and several
   others put raw ClickHouse error text into the tool result, which the
   model then repeats to the user; one live answer showed
   `DB::Exception: Database _silver_meta does not exist`. Map these to
   honest short messages ("quality checks have not run yet") per AGENTS.md
   principle 4.
6. **Tests.**
   - Unit tests for the layer resolution, including a dataset missing from
     one layer.
   - An end-to-end test in the style of `tests/ai_chat_stream.rs`, where the
     mocked model calls `list_datasets` / `lakehouse_overview` and the tool
     result carries `layers` and `sourceKind`.
   - Keep `tests/ai_citations.rs` green: the citation checker must still see
     the numbers the new tools return.

## Related, seen during the same review

- **Categorical counts flagged as unverified.** `citations::annotate_answer`
  marked "4 primer + 2 sekunder" as unverified, even though `list_datasets`
  returned those tiers. Its count derivation does not cover grouping a tool
  result by a column. Worth a look once the tools return `layers`.
- **Seed data.** Fix the local seed so catalog `table_name`, Silver tables and
  Gold marts line up. Otherwise every layer answer on the demo stack reads
  as "missing".

## Constraints for whoever picks this up

- Test only against the local `lakehouse-run` stack. Never query, exec into
  or restart any `dispar-lakehouse` / `lake-*` container (client data).
- One `cargo` process at a time on this machine. Use narrow test scopes
  (`-p lakehouse-api --test …`) with `RUST_TEST_THREADS=2`, and remove the
  testcontainers your runs leave behind.
- Rebuild the API image with
  `docker compose -p lakehouse-run --profile dagster build lakehouse-api`,
  then `up -d --no-deps lakehouse-api`, to try the prompt live. Copilot's
  streaming (`delta` / `reasoning` events) is already in place and needs no
  change.
