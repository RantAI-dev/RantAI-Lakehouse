# `DATA-12` parts 2 and 3 — governed tags, and marks where tables are picked

| | |
| --- | --- |
| Item | `DATA-12` (parts 2 and 3; part 1 is built and QA-checked) |
| Branch / PR | `feat/data-12-certification-governed-tags`, PR #106 (Phase 2 master plan `2026-10-14-data-phase-2.md`) |
| Base verified at | `bfed22fc` (2026-10-14): branch with `origin/main` at `07c76324` merged |
| Decisions | Signed by the product owner 2026-10-09, feature page `docs/core/features/certification-and-governed-tags.md` rows 2–4; spec `docs/core/specs/data-12.md` marks **no** numbers *(proposed)* for these parts |
| Developer | The non-Opus coding agent (per `AGENTS.md`); planner wrote this plan and reviews |

## Decisions already made (signed 2026-10-09)

- **Part 2.** Governed tags are added *beside* free tags. An admin defines tag
  keys and the values each may take; a table gets `key = value` pairs that are
  checked; free one-word tags stay and keep working (decision 2).
- **Part 2 out.** A tag on a namespace applying to its tables is **not** built
  (`DATA-22`, decision 3).
- **Part 3.** Marks appear on Gold marts in the chart builder's data-source
  picker, and on the sources Query Studio lists for a ClickHouse query. SQL
  sources and the autocomplete are not marked (decision 4). On a deprecated
  choice the user sees the warning (feature page items 8–9).
- **Permissions, by analogy with part 1 (decision 1).** Defining keys is a
  governance act → `governance:write`. Giving a table a value is an edit of the
  table's annotation → `catalog:write`, riding today's annotation route.
- **Spec targets.** `DATA-12-AC2` (a value outside the allowed list is refused),
  tags visible "in search results, the Data Explorer list, the asset page, the
  chart builder's table picker and Query Studio" (spec, Where tags show).

## Planner decisions (not owner-signed; the reviewer may veto at review)

- **Storage** — two tables in one migration `0073_governed_tags.sql` (first
  numbered `0071`; renumbered twice, see "Second renumber" below):
  - `governed_tag_key(key TEXT PRIMARY KEY, allowed_values TEXT[] NOT NULL,
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL, updated_at
    TIMESTAMPTZ NOT NULL)`.
  - `asset_governed_tag(asset_id TEXT NOT NULL, tag_key TEXT NOT NULL REFERENCES
    governed_tag_key(key) ON DELETE RESTRICT, value TEXT NOT NULL, set_by TEXT
    NOT NULL, set_at TIMESTAMPTZ NOT NULL, PRIMARY KEY (asset_id, tag_key))`.
  - Bounds mirror the free-tag conventions (`annotation.rs:12-16`): key and
    value match `^[a-z0-9][a-z0-9_-]*$`, 1–64 chars; at most 50 values per key;
    at most 20 governed tags per table; `asset_id` ≤ 200; `created_by`/`set_by`
    ≤ 128. Shape bounds go in CHECK constraints (defense in depth, `0062`
    pattern); "value is in the key's allowed list" cannot be a CHECK — the
    handler validates and the migration header says so.
  - `ON DELETE RESTRICT` + a handler check that refuses deleting a key still in
    use, with the count in the message: no silent tag loss.
- **Routes** — `/api/governance/tag-keys`:
  `GET` → `catalog:read` (editors need the list to render pickers; definitions
  are not sensitive); `POST` and `PUT /{key}` and `DELETE /{key}` →
  `governance:write`. `PUT` may change `allowed_values` only — a key is never
  renamed; removing a value still assigned to any table is refused with the
  count. Every row goes into `POLICY_TABLE` and is asserted both ways in
  `tests/route_auth.rs`.
- **Setting values on a table** — the body of `PUT /api/catalog/{id}/annotation`
  (`catalog:write`, `policy.rs:194`) gains an optional `governedTags` object
  (`{"data_domain": "finance"}`). Sending it replaces the table's full governed
  set (same semantics as `tags`); omitting it changes nothing. Shape bounds and
  the ≤ 20-pairs cap are validated in the handler; whether a value is in the
  key's allowed list is checked in the store, inside the write transaction under
  a row lock on the key (`FOR SHARE` on write, `FOR UPDATE` on edit/delete), so
  the list checked is the one written against — the developer's task-1 design,
  accepted by the planner 2026-10-14. Refusals name the offending key and the
  allowed values. A pair whose value is unchanged keeps its `set_by`/`set_at`.
  Store side: one statement replaces the table's rows in the same transaction
  as the annotation upsert (`set_asset_tags_in(&mut PgConnection, …)`).

### Migration numbering after the `bfed22fc` merge (planner decision, 2026-10-14)

The merge left two migrations numbered `0062`: this branch's
`0062_asset_certification.sql` (part 1, written when main's last was `0061`)
and main's `0062_connector_type_unsupported_reason.sql` (main renumbered its
own `0061–0064` to `0062–0065` in `36f2a2ac`, before #115). The branch is the
second merger, so its own migration moves, following the rule in the master
plan and the `36f2a2ac` precedent:

- `0062_asset_certification.sql` → **`0070_asset_certification.sql`** (content
  unchanged — an applied migration is never edited; only its number moves).
- The governed-tags migration is **`0071_governed_tags.sql`**.

`ls rust/migrations | cut -c1-4 | sort | uniq -d` prints nothing after the
rename. Code comments that say "`0062`" (`connector_type.rs:11`, its tests)
refer to main's migration and stay correct.

**Record fix, needed only where asset_certification was already applied as
version 62** — known instance: the product owner's QA instance (`:18082`, not
running at the time of writing). Run this once on its Postgres **before**
starting the API on the new head; the content did not change, so the recorded
checksum stays valid, and main's `0062` then applies fresh on next start:

```sql
UPDATE _sqlx_migrations SET version = 70
 WHERE version = 62 AND description LIKE '%asset_certification%';
```

The running dev stack (`lakehouse-postgres-1`) stops at version 55 and needs
nothing. The part-1 plan keeps its historical `0062` mentions; `CHANGELOG.md`'s
part-1 entry is updated to `0070` by the renumber commit.

**Second renumber (2026-10-16, merge of `main` at `cacf7991`).** Main took
`0070_reporting_settings.sql` and `0071_pipeline_definition_version_tenant.sql`
while this branch's two were unmerged, so by the same rule this branch's moved
again, content unchanged:

- `0070_asset_certification.sql` → `0072_asset_certification.sql`
- `0071_governed_tags.sql` → `0073_governed_tags.sql`

The `CHANGELOG.md` part-1 entry and the code comments that name the migration
(`lakehouse-store/src/lib.rs`, `governed_tag.rs`, `routes/governance.rs`) say
`0072`/`0073` now. The text above keeps the numbers as they stood when each
decision was taken. `0073_governed_tags.sql` is byte-identical to the old
`0071` file, so its header still says "like `0032` and `0070`": that `0070` is
the certification migration (now `0072`), not main's. The header is left
alone because editing an applied migration changes its recorded checksum.
A database that applied this branch's migrations under the old numbers needs
the same kind of one-time, guarded record fix as above (version 70 → 72 for
asset_certification, 71 → 73 for governed_tags, each `WHERE version = <old> AND
description LIKE '%<name>%'`); none was run here.

- **Catalog overlay** — rows and detail carry `governedTags` (object, keys
  sorted) next to `tags`, via the annotation overlay in
  `routes/catalog.rs:1481-1490`; absent when empty, like part 1's fields.
- **Search** — the matcher (`routes/catalog_search.rs:155-160`) indexes
  governed pairs as `key: value`: matches on the key, the value, or the pair;
  `matchedOn.field` stays `"tag"`, `matchedOn.value` is `"key: value"`. The
  Data Explorer's existing `tags` filter (`filter_assets_by_query_matches`) and
  the Tags column match governed pairs too.
- **Part 3 surfaces** — `GET /api/dashboard/fields` (`dashboard.rs:678`) adds
  the part-1 certification fields to each mart, read from the annotation
  overlay (`list_all`); the chart builder shows the mark in the picker row and
  a warning banner when the chosen mart is deprecated (note + link to the
  replacement). The ClickHouse estimate transparency payload
  (`routes/query.rs:745`, `sources_out`) enriches each source with the same
  fields; `query-transparency-panel.tsx:174` renders mark + warning.
- **Assistant parity (part 3)** — the `describe_mart` tool's answer includes
  the certification fields, so the assistant can say a mart is deprecated;
  `tests/fixtures/tool_schemas.json` is regenerated; the AI team reviews this
  part (master plan).

## Anchors (verified at `bfed22fc`)

| Anchor | What |
| --- | --- |
| `rust/crates/lakehouse-store/src/annotation.rs:92` | `upsert_annotation` — the annotation write to extend |
| `rust/crates/lakehouse-store/src/annotation.rs:167,200` | `set_certification` / `clear_certification` — the part-1 statement pattern to follow |
| `rust/crates/lakehouse-api/src/routes/catalog.rs:1481-1490` | annotation overlay onto catalog rows (`tags`, then part-1 fields) |
| `rust/crates/lakehouse-api/src/routes/catalog.rs:2785` | part-1 route section header (`PUT /api/catalog/{id}/certification`) — model for the tag-keys section |
| `rust/crates/lakehouse-api/src/policy.rs:193-198` | annotation and certification policy rows — neighbours for the new rows |
| `rust/crates/lakehouse-api/src/routes/catalog_search.rs:155-160` | tag indexing in the search matcher |
| `rust/crates/lakehouse-api/src/routes/mod.rs:729-730` | annotation route registration |
| `rust/crates/lakehouse-api/src/routes/dashboard.rs:678` | `fields` handler (marts list) |
| `rust/crates/lakehouse-api/src/routes/query.rs:745` | `"sources": sources_out` in the estimate transparency payload |
| `src/features/catalog/asset-about.tsx:36,66` | `parseTags` and the free-tags editor on the About card |
| `src/features/catalog/data-explorer-columns.tsx:61,116,178-200` | certification filter options, certification cell, tags column and filter |
| `src/features/dashboards/chart-builder.tsx:221,358,378` | marts state, picker options, `/api/dashboard/fields` fetch |
| `src/features/queries/query-transparency-panel.tsx:174` | the sources list render |
| `src/services/contracts/queries.ts:73` | `sources: string[]` contract |
| `src/app/(governance)/governance/policies` | route under which the Tag keys page is added as a sibling (`tag-keys`) |
| `rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json` | tool schemas fixture to regenerate |

## Tasks (one commit each, in order)

Each task names its scoped check (master plan: only what changed). A task whose
acceptance check fails stops the developer; mismatches between plan and code
are reported, not improvised (`AGENTS.md`, developer role).

1. **Renumber, then migration and store.** First commit of this task:
   `git mv rust/migrations/0062_asset_certification.sql
   rust/migrations/0070_asset_certification.sql`, update the `CHANGELOG.md`
   part-1 entry's migration number to `0070`, rename the developer's uncommitted
   migration to `0071_governed_tags.sql`, and run the duplicate check
   (`ls rust/migrations | cut -c1-4 | sort | uniq -d` prints nothing). Commit
   message follows the `36f2a2ac` style, citing this plan section and the
   precedent. Then the governed-tags migration itself (why-header citing spec
   and this plan); store statements in a new
   `rust/crates/lakehouse-store/src/governed_tag.rs`: `create_key`, `list_keys`,
   `update_key_values`, `delete_key` (refused when in use, count returned),
   `set_asset_tags` (replace-one-transaction), `list_for_asset`,
   `list_for_assets` (batch for overlays). No `unwrap`/`expect`; `# Errors` on
   every `Result` fn. Check: `cargo fmt --check` then
   `cargo clippy -p lakehouse-store --all-targets -- -D warnings` then
   `cargo test -p lakehouse-store`. Acceptance: store tests cover the value
   bounds, the in-use delete refusal, and replace semantics.
2. **Tag-keys routes.** Handlers in the governance routes module;
   `POLICY_TABLE` rows; `tests/route_auth.rs` asserts all four both ways;
   validation (pattern, lengths, ≤ 50 values) with 400s naming the field;
   audit events `governance.tag_key_create`, `..._update`, `..._delete`.
   Check: `cargo clippy -p lakehouse-api --all-targets -- -D warnings`,
   `cargo test -p lakehouse-api tag_key`. Acceptance: 403 without
   `governance:write` on the three writes; `GET` works with `catalog:read`.
3. **Governed tags on the annotation route and overlay.** `governedTags` in the
   annotation PUT body + validation; overlay onto rows and detail; store tests
   and handler tests; the deprecation notice copy stays untouched. Check:
   `cargo clippy -p lakehouse-api --all-targets -- -D warnings`,
   `cargo test -p lakehouse-api governed`. Acceptance: value not in the
   allowed list → 400 naming key and allowed values; unknown key → 400;
   omitting `governedTags` changes nothing (regression test beside the part-1
   "saving details never changes a mark" one).
4. **Search and the Data Explorer filter.** Matcher indexes governed pairs;
   `matchedOn` value `"key: value"`; the server-side `tags` filter matches
   governed pairs; tests beside `a_tag_finds_its_table`
   (`catalog_search.rs:411`). Check: `cargo test -p lakehouse-api search` +
   clippy. Acceptance: a search on the value alone, the key alone, and the pair
   each find the table; `matchedOn` shows `data_domain: finance`.
5. **Frontend contracts and Tag keys page.** `assets.ts` gains `governedTags`
   (`type`, camelCase); a `tagKeys` client in `services` (imports `@/services`
   only from features); governance feature page `tag-keys-page.tsx` (list,
   create, edit values, delete with the in-use refusal shown) mounted at
   `src/app/(governance)/governance/tag-keys`; sidebar entry beside Policies.
   Check: `bun run typecheck && bun run lint && bun run test`. Acceptance:
   create/edit/delete flows refuse what the API refuses, with the API's
   sentence.
6. **About card display and editor.** Governed pairs shown beside free tags as
   `key: value` chips; in the editor, one select per key (allowed values +
   "none") beside the free-tags input; free-tag editing unchanged. Check: bun
   trio. Acceptance: a `catalog:write` user picks a value outside the list —
   impossible in the UI — and sees the API's refusal if the list changed under
   them (stale-allowed-values case shows the 400 sentence).
7. **Data Explorer column and filter.** Tags cell renders governed pairs as
   `key: value` chips beside free tags; the tags filter matches them. Check:
   bun trio. Acceptance: filter `finance` finds the table; the column shows
   both tag kinds.
8. **Part 3, API.** Certification fields on `fields` marts and on transparency
   `sources`; handler tests for both; batch annotation read, no N+1 per source.
   Check: `cargo clippy -p lakehouse-api --all-targets -- -D warnings`,
   `cargo test -p lakehouse-api`. Acceptance: a deprecated mart reaches the
   picker payload with note and replacement id; a Trino query's sources are
   unchanged (ClickHouse only — spec, Not included).
9. **Part 3, console.** Picker rows show the mark; choosing a deprecated mart
   shows the warning banner (note + link); transparency panel renders marks and
   the warning. Check: bun trio. Acceptance: feature page items 8–9 doable.
10. **Assistant parity.** `describe_mart` includes the fields; regenerate
    `tests/fixtures/tool_schemas.json`; note for the AI-team review in the
    handoff. Check: `cargo test -p lakehouse-api describe_mart` + fixture
    regeneration command quoted in the handoff. Acceptance: the fixture diff
    shows only the intended schema change.
11. **Handoff.** Full verification block (master plan; languages touched:
    Rust, TS) in the foreground; append the Handoff entry to this plan
    (commits, commands with counts, anything *not verified* and why); push so
    CI runs on #106.

## Out of scope

- Namespace inheritance (`DATA-22`); tag-based masking; ranking certified
  tables higher in search; marks on dashboard SQL sources, on Trino sources and
  in autocomplete (feature page, Not included).
- A tag on a Gold mart *as a governed-tag example* is not staged by this work;
  the acceptance steps use whatever tables the console already has.

## Acceptance rows this part adds to the feature page

`DATA-12-AC10` … `AC17` (drafted by the planner on the feature page in the same
commit as this plan; the owner runs them after QA).

## Handoff and review

### Planner's review, 2026-10-16, at `3b0b08a0`

The developer's tasks 1 to 10 are on the branch (`22e2f485` to `c5e76658`,
renumbered in `cf68c369`); task 11's handoff entry was never written: the
session that ran it ended. The planner ran task 11's verification itself and
reviewed the diff against this plan.

No `BLOCKER`, no `SHOULD-FIX`.

**Checked and correct.**
- Store and migration `0073_governed_tags.sql`: `governed_tag_key` (key,
  allowed values, creator) and the per-asset pairs, shapes checked in the
  database as well as in the handler.
- Tag keys behind `GET/POST/PUT/DELETE /api/governance/tag-keys`, writes on
  `governance:write`, with `POLICY_TABLE` rows and `route_auth` cases.
- `governedTags` on the annotation PUT is optional: omitting it changes
  nothing, so an older caller cannot wipe a table's governed tags. Values are
  checked against the key's allowed list.
- Search and the Tags filter find governed pairs; the Tag keys page, the
  About card editor and the Data Explorer column show them.
- Part 3: the mart list and a query's sources carry the certification
  fields; the chart builder shows the mark and the deprecation notice for a
  chosen mart (reusing part 1's `DeprecationNotice`); the Query Studio panel
  shows marks; `describe_mart` returns the fields. The tool's schema did not
  change, so `tool_schemas.json` needs no regeneration.

**Verification, by the planner, at `3b0b08a0`** (detached job, log kept
outside the repo): `cargo fmt --check` exit 0; `cargo clippy --workspace
--all-targets --all-features --locked -- -D warnings` clean in 3m13s
(`CARGO_INCREMENTAL=0`); `bun run typecheck` exit 0; `bun run lint` 0 errors,
8 warnings; `bun run test` 1551 pass, 1 skip, 0 fail (1552 tests, 166
files); both Python lint lines OK; no duplicate migration numbers.

**Not verified.** No Rust test has run on this machine; CI on #106 runs
them. The parts 2 and 3 screens have not been tried in a browser; the
product owner's check of `DATA-12-AC10` onwards waits for the end of the
phase.
