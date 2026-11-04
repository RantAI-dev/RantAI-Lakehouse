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

### Handoff 1 (developer, 2026-11-04)

Base `fix/data-39-assistant-reads-registry-final` at `c3339533`; three commits, one
per task, each pushed to `origin/fix/data-39-assistant-reads-registry-final`.

| Task | Commit | What |
| --- | --- | --- |
| T1 | `af778ab7` | Both readers (`routes/ai/data_map.rs` `load_catalog`, `routes/ai/tools/data.rs` `describe_dataset`) read each registry `FINAL` and splice `crate::routes::catalog::NOT_IN_PRIMARY` into the secondary half. D2 holds by construction: the descriptions are a `HashMap<(slug, col), String>`. |
| T2 | `405774cd` | Six tests against a real `ClickHouse` via `routes::bronze_registry_fixture` (DATA-42 D7), plus `seed_column_with` in that fixture (`seed_column` now delegates to it, so its five existing call sites are unchanged). |
| T3 | `c068352c` | `CHANGELOG.md` → `## [Unreleased]` → `### Fixed`: the assistant no longer sees a Bronze column twice (`DATA-39`, the part `#315` left open). |
| Handoff | this commit | this entry |

**D3 respected:** `tests/fixtures/tool_schemas.json` is unchanged (`git status` /
`git diff --stat` after the full run name no fixture); no tool schema or prompt was
touched.

**Tests added** (each reader: written twice / in both registries / secondary only):

- `routes::ai::data_map::tests::registry_clash::{a_column_written_twice_to_the_primary_answers_once_from_the_newer_row,
  a_slug_in_both_registries_answers_with_the_primary_columns,
  a_slug_only_in_the_secondary_registry_still_answers}`
- `routes::ai::tools::data::registry_clash::{a_column_written_twice_to_the_primary_is_listed_once,
  a_slug_in_both_registries_answers_with_the_primary_columns,
  a_slug_only_in_the_secondary_registry_still_answers}`

Honest note on the DATA MAP's "written twice" test: a `HashMap` cannot hold two
entries for one key, so for that reader "appears once" is structural (D2 said so)
and what `FINAL` decides is *which* copy answers — the test asserts the newer
row's description and one entry. Measured: with the OLD statement restored for
that reader, the written-twice test PASSES (the map keeps one entry either way)
and the clash test FAILS — the secondary's description wins the map and its own
extra column is read:

```
test ...::a_column_written_twice_to_the_primary_answers_once_from_the_newer_row ... ok
test ...::a_slug_in_both_registries_answers_with_the_primary_columns ... FAILED
assertion `left == right` failed: the primary registry's description:
{("clash_map", "id"): "the secondary id", ("clash_map", "note"): "the secondary note"}
  left: Some("the secondary id")
 right: Some("the primary id")
test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 3006 filtered out
```

Restored, `cargo-locked test -p lakehouse-api --lib data_map::tests::registry_clash`:
`3 passed; 0 failed; 0 ignored; 0 measured; 3006 filtered out; finished in 4.54s`.
`describe_dataset` answers a LIST, so its written-twice test fails on the old
statement directly (two `amount` columns) — that is the mutation check below.

**Commands run, in the foreground, through `/home/shiro/bin/cargo-locked` with the
shared `CARGO_TARGET_DIR`.** Per commit: `cargo fmt --check` and
`cargo-locked clippy -p lakehouse-api --all-targets -- -D warnings` (both green;
T1 finished in 4m55s, T2 in 1m45s), plus the scoped tests below. No `cargo clean`,
no per-worktree `target/`.

| Command | Result |
| --- | --- |
| `cargo test -p lakehouse-api --lib data_map` | `77 passed; 0 failed` (2932 filtered out), 4.77s |
| `cargo test -p lakehouse-api --lib tools::data` | `22 passed; 0 failed` (2987 filtered out), 4.45s |
| `cargo test -p lakehouse-api --lib registry_clash` (all readers' fixtures) | `22 passed; 0 failed` (2987 filtered out), 16.96s |

**Mutation check (§4), both quoted.** Removed `FINAL` from reader 2's statement
(both halves), then
`cargo-locked test -p lakehouse-api --lib registry_clash::a_column_written_twice_to_the_primary_is_listed_once`:

```
test routes::ai::tools::data::registry_clash::a_column_written_twice_to_the_primary_is_listed_once ... FAILED
assertion `left == right` failed: the column a second run registered is one column:
{"slug":"twice_columns",...,"columns":[
  {"name":"amount","type":"UInt64","description":"the first description"},
  {"name":"amount","type":"UInt64","description":"the second description"}]}
  left: 2
 right: 1
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 3008 filtered out
```

Restored, then `cargo-locked test -p lakehouse-api --lib registry_clash`:
`22 passed; 0 failed; 0 ignored; 0 measured; 2987 filtered out; finished in 16.96s`.

**Plan §6 block, on `c068352c`:**

- `cd rust && cargo fmt --check` — clean.
- `cargo-locked clippy --workspace --all-targets --all-features -- -D warnings` — green, `Finished dev profile ... in 5m 15s`.
- `cargo-locked test --workspace` — exit 0. 184 `test result:` lines, `9579 passed;
  0 failed; 52 ignored` (counted from the saved log of the second run; the first
  run was green too and its counts were not saved).
- `git status` — nothing to commit, nothing generated staged.

**Testcontainers.** The shared fixture drops its container per test; measured
around a `registry_clash` run, `docker ps` went 14 → 14. The full `--workspace`
run does leak three containers it does not own the drop of: the three integration
binaries that hold a container in a `OnceCell` static — `tests/ai_sessions.rs`,
`tests/gold_publication.rs`, `tests/query_result_cap.rs` (the last two document
why). Pre-existing, not this change; I stopped and removed the six my two runs
left and left the seven from earlier sessions alone.

**Not verified / not run, with the reason.**

- The non-Rust lines of `AGENTS.md`'s verification block (`bun run typecheck /
  lint / test`, the Python lints and Dagster pytest, `docker compose config`):
  nothing outside `rust/` and `CHANGELOG.md` changed, so per "Only build what
  changed" no TypeScript, Python or compose check was run.
- Nothing else was skipped.

## 8. Review (planner appends below)
