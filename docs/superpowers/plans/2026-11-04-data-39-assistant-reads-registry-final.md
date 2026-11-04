# DATA-39 (assistant part): the assistant reads the Bronze column registry once per column

Backlog `DATA-39`, the part left open for the AI team (the data part shipped in #315).
Planner: Claude Opus, 2026-11-04. Anchors verified at `main` `6e5be45e`. Base branch:
this one, `fix/data-39-assistant-reads-registry-final`.

## 0. What is wrong, with evidence

The Bronze column registry (`bronze_meta.dataset_column`, `bronze_meta_sec.dataset_column`)
is a `ReplacingMergeTree` that a re-run writes again, and `ClickHouse` merges the copies only
in the background. Two assistant readers read it without `FINAL` and from both registries
with a plain `UNION ALL`:

- `routes/ai/data_map.rs:601-606`: the descriptions of every column of every dataset;
- `routes/ai/tools/data.rs:477-485`: one dataset's columns, types and descriptions.

So the assistant can see a column twice: once per unmerged copy, and once per registry when
a slug is in both. Every other reader was fixed by `DATA-42`, which made the primary registry
win through one clause, `catalog::NOT_IN_PRIMARY` (`routes/catalog.rs:574`), and reads with
`FINAL`.

## 1. Decisions

- **D1. Same rule as `DATA-42`, same clause (rule 4).** Both statements read
  `bronze_meta.dataset_column FINAL`, and the secondary half reads
  `bronze_meta_sec.dataset_column FINAL WHERE slug <NOT_IN_PRIMARY>` (plus the existing
  `slug={slug}` filter in `tools/data.rs`). Splice `crate::routes::catalog::NOT_IN_PRIMARY`
  with `format!` — it is a constant, never caller input; do not retype the string.
- **D2. `data_map`'s per-slug grouping keeps one description per column.** After D1 a
  column appears once; if the code builds a map keyed by `(slug, col)`, nothing else
  changes. If it appends to a list, check that the list no longer repeats.
- **D3. No change to the assistant's tool schema or prompt.** The answer shape is unchanged;
  only duplicates go. `tests/fixtures/tool_schemas.json` must not change (if it does, stop
  and report).

## 2. Anchors (at `6e5be45e`)

| What | Where |
| --- | --- |
| Reader 1 | `rust/crates/lakehouse-api/src/routes/ai/data_map.rs:601-606` |
| Reader 2 | `rust/crates/lakehouse-api/src/routes/ai/tools/data.rs:477-485` |
| The shared clause | `rust/crates/lakehouse-api/src/routes/catalog.rs:555-575` (`NOT_IN_PRIMARY`) |
| The registry test fixture | `rust/crates/lakehouse-api/src/routes/bronze_registry_fixture.rs` (`DATA-42` D7) |
| How `DATA-42` tests a reader | `routes/catalog_profile.rs:521-…` (`mod registry_clash`) |

## 3. Tasks (one commit each, in order; push after each)

**T1. Both readers (D1, D2).**
- *Accept:* `cargo clippy -p lakehouse-api --all-targets -- -D warnings`.

**T2. Tests, against a real `ClickHouse`, with `bronze_registry_fixture` (as `DATA-42` did).**
For each reader:
- a column written twice to the primary registry (two inserts, no merge) appears once;
- a slug in both registries answers with the primary's columns only;
- a slug only in the secondary registry still answers with its columns.
- *Accept:* `cargo test -p lakehouse-api --lib data_map` and `--lib tools::data` (or the
  module paths they live under).

**T3. Changelog.** `## [Unreleased]` → `### Fixed`: the assistant no longer sees a Bronze
column twice when the registry holds two copies of it or a dataset is in both registries
(`DATA-39`, the part `#315` left open).

## 4. Mutation check (in the handoff)

Remove `FINAL` from reader 2 and run its "written twice" test: it must fail. Restore, rerun,
quote both.

## 5. Out of scope

- Any reader outside `routes/ai/`.
- The column count's first-row problem (`DATA-40`).

## 6. Verification before handoff

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
git status
```

## 7. Handoff (developer appends below)

## 8. Review (planner appends below)
