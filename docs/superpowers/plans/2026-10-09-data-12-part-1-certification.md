# `DATA-12` part 1, certified and deprecated marks — Implementation Plan

**Status:** ready to build. Decisions 1 to 4 on the feature page were signed
by the product owner on 2026-10-09, all as proposed.
Written 2026-10-09 by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md`.

**Spec:** `docs/core/specs/data-12.md`. Backlog `DATA-12`.
**Feature page:** `docs/core/features/certification-and-governed-tags.md`.
Decision numbers below (D1–D4) are its rows.

**Why.** Nothing marks a table as the one to trust, or as one to stop using.

This plan is part 1 of three. Part 2 (governed tags) and part 3 (marks in
the chart builder and Query Studio) get their own plans after this part is
reviewed. **Part 2 waits for part 1** (both change `asset_annotation` and
the About card).

---

## 1. Slice and base

One pull request. Base `origin/main` at `bc048e0`, branch
`feat/data-12-certification-governed-tags`, worktree
`/home/hv/lakehouse-data12`. Commits stay local; the planner pushes and
opens the pull request.

## 2. What exists today

Anchors verified at `origin/main` `bc048e0`. `A` is
`rust/crates/lakehouse-api/src`, `R` is `A/routes`. Re-find by symbol if a
line has moved.

| # | Code | Verdict |
| --- | --- | --- |
| F1 | `rust/migrations/0032_asset_annotation.sql`: `asset_annotation` has `asset_id`, `owner`, `steward`, `tags`, `description`, `updated_at`. The latest migration is `0061`. | No column for a mark. The next migration is `0062`. |
| F2 | `lakehouse-store/src/annotation.rs`: `upsert_annotation` (`:65`) writes exactly `owner`, `steward`, `tags`, `description`. `AnnotationRow` (`:42`) and `list_all` (`:113`) read the same four. | An edit of the details cannot touch a column this plan adds, as long as that statement keeps naming its columns. |
| F3 | `R/catalog.rs`: `put_annotation` (`:2576`) needs `catalog:write` (`A/policy.rs:194`), runs `catalog_tenant_refusal`, audits `catalog.annotate` with field names, and calls `catalog_search_cache.invalidate()` (`:2611`). | One body, one permission: a mark added to it would be settable by every `catalog:write` holder (D1 forbids that). |
| F4 | `apply_annotation` (`R/catalog.rs:1463`) overlays `owner`, `description`, `steward`, `tags` on list rows; `assemble_catalog` feeds the live list, `/api/catalog/query` and the search copy. `mark_annotation_and_history` (`:1517`) adds `annotation`, `registry`, `changeHistory` to the detail body. | Where a mark must be added to reach every surface. |
| F5 | `R/catalog_query.rs`: `FILTERABLE_FIELDS` (`:36`), `SORTABLE_FIELDS` (`:67`), `GROUPABLE_FIELDS` (`:92`); a test (`:683`) pins sortable to filterable minus the list fields. | No `certification` field. |
| F6 | `change_entry` (`R/catalog_governance.rs:402`) turns audit actions into history lines; an unknown action prints as its raw name. | New actions need arms. |
| F7 | Console: the asset header's badges (`src/features/catalog/asset-detail-page.tsx:117`), the Data Explorer's name cell with the reason line (`data-explorer-columns.tsx:84-118`), the ⌘K rows (`src/components/command-palette.tsx:219-229`, `PaletteItem` at `:257`). The only notice on an asset page is `StandInNotice` (`asset-stand-in.tsx:11`); there is no shared alert component. | Where the mark and the warning go. |
| F8 | `get_annotation` (`R/catalog.rs:2529`) takes no `Principal` and runs no tenant check. | An open gap owned by `SEC-16`. This plan adds nothing to that route's answer. |
| F9 | `tests/route_auth.rs:386`: the seeded app has four tenants and no `CATALOG_TENANT_ID`, so a happy-path write through the router is answered by the tenant check. | Positive-path tests are handler-level, as `put_annotation`'s are (`R/catalog.rs:4065`). |

## 3. Decisions already made by the planner

- **Storage.** Migration `0062_asset_certification.sql`, why-header, adds to
  `asset_annotation`: `certification TEXT` (`CHECK` in `'certified'`,
  `'deprecated'`; null is no mark), `certification_note TEXT` (at most 1000
  characters), `replacement_asset_id TEXT` (at most 200), `certified_by
  TEXT` (the display name, at most 128), `certified_at TIMESTAMPTZ`. One
  table-level `CHECK`: a note or a replacement is allowed only with
  `'deprecated'`; `certified_by` and `certified_at` are set exactly when
  `certification` is. No foreign key: assets do not live in Postgres.
- **Its own write.** `lakehouse-store/src/annotation.rs` gains
  `set_certification(pool, asset_id, input)`: an upsert that writes only the
  five new columns (an asset with no annotation row gets one with empty
  details), and `clear_certification`. `upsert_annotation` keeps its column
  list, so editing details never changes a mark; a store test proves both
  directions.
- **Its own route (D1).** `PUT /api/catalog/{id}/certification`,
  `Policy::RequiresPermission("governance:write")`, body `{ "status":
  "certified" | "deprecated" | null, "note"?: string, "replacementAssetId"?:
  string }`. Order: `catalog_tenant_refusal` (403 with the fixed reason, as
  `put_annotation`), `validate_annotation_id`, body validation, then the
  write. Refused with `400` and a fixed sentence each: an unknown status; a
  note or replacement without `deprecated`; a note over 1000 characters; a
  replacement equal to the asset itself; a replacement that is not an asset
  in the catalog (checked against the search copy's asset ids, `DATA-11`'s
  `search_snapshot`); a replacement that is itself deprecated. `status:
  null` clears all five columns. Answer `{ "ok": true }`. After a
  successful write: `catalog_search_cache.invalidate()`.
- **Audit.** Actions `catalog.certify`, `catalog.deprecate`,
  `catalog.uncertify`, `resource_kind` `catalog`, `resource_id` the asset
  id, args `{ note, replacementAssetId }` for a deprecation. `change_entry`
  gains three arms: "Marked certified", "Marked deprecated" (plus ": use
  <replacement>" when there is one), "Removed the certified / deprecated
  mark" (one sentence saying which, from an `args.was`). The note's text is
  not put in the history line.
- **On every row.** `apply_annotation` adds, when a mark is set:
  `certification` (the status string), `certificationNote`,
  `replacementAssetId`, `certifiedBy`, `certifiedAt` (RFC 3339). Absent when
  there is no mark, as `tags` is. The detail body gets the same fields
  through the same overlay; nothing is added to `GET …/annotation` (F8).
- **Filter.** `certification` joins `FILTERABLE_FIELDS`, `SORTABLE_FIELDS`
  and `GROUPABLE_FIELDS` (two values and empty: low cardinality).
- **Search.** No ranking change and no new searchable field. A result
  carries the mark because it is on the row.
- **Console.** `CertificationBadge` in
  `src/components/patterns/status-badge.tsx` beside the other badges
  (certified: `success`; deprecated: `warning`), label and title from
  `@/lib/status` as the others. Shown: in the asset header's
  `titleAccessory`; in the Data Explorer's name cell (the Tags column is
  hidden on most screens, so not a column of its own); in a ⌘K row beside
  the name. On a deprecated asset's page, a notice between the header and
  the metadata list, `role="note"`, in the `StandInNotice` style: "This
  table is deprecated." plus the note, plus "Use <replacement name>
  instead" linking to `/data/assets/<id>` when there is one. A
  "Certification" action on the asset page, shown only with
  `hasPermission("governance:write")`, opening a dialog: three choices (no
  mark, certified, deprecated), and for deprecated a note and a replacement
  asset id field; the API's refusal sentence is shown as it comes. The Data
  Explorer gets a `certification` filter as a hidden column with a
  multi-select filter (follow how the hidden `id` column is declared).
- **No new permission, no new grant, no new dependency.**

## 4. Tasks

One task per commit, Rust first, in this order. Cite `DATA-12` and the
finding at each fix site and in the commit body (rule 13).

**R1. Migration and store (F1, F2).** As decided. `AnnotationRow` gains the
five fields; `list_all` and `get_annotation` select them; every struct
literal that builds an `AnnotationRow` or `AnnotationInput` is updated
(`R/catalog.rs`, `R/ai/data_map.rs`, `R/ai/mod.rs`, the store tests).
*Check:* `sqlx::test`s in `lakehouse-store/tests/annotation.rs`: set and
read back each status; clear; `upsert_annotation` after a mark leaves the
mark; `set_certification` after details leaves the details; a note with
`certified` is refused by the `CHECK`; an unknown status is refused.

**R2. The route (F3, F8, F9).** Handler in `R/catalog.rs`, route in
`R/mod.rs`, `POLICY_TABLE` row, the audit insert. *Check:* handler-level
tests beside `put_annotation`'s: each refusal of section 3 with its
sentence and nothing written; a member certifies and the row holds the
caller's display name; clearing removes all five; the tenant refusal is 403
and writes nothing; the search copy is dropped on success and kept on a
refusal. `tests/route_auth.rs`: a named test that a seeded Data Engineer
(`catalog:write`, no `governance:write`) gets 403 and nothing is stored;
the table-driven tests cover the new row both ways. `policy.rs`: a unit
test pinning the permission.

**R3. Rows, filter and history (F4, F5, F6).** `apply_annotation`, the
three field lists, `change_entry`. *Check:* unit tests: a marked row
carries the five fields and an unmarked row none; `certification` filters
with `eq` and `inArray`, sorts and groups; the sortable-pinning test still
holds; each new action renders its sentence and the note's text is not in
it.

**T1. Contract and client.** `src/services/contracts/assets.ts`: `Asset`
gains `certification?: "certified" | "deprecated" | string`,
`certificationNote?`, `replacementAssetId?`, `certifiedBy?`,
`certifiedAt?`; `AssetService` gains `setCertification?(id, input,
signal)`. `src/services/clients/assets.ts` implements it with `apiFetch`
and `errorFor`. No mock gains a method.

**T2. The badge and where it shows (F7).** `CertificationBadge`; the asset
header; the Data Explorer name cell and the hidden filter column; the ⌘K
row. *Check:* tests beside the existing ones (`data-explorer-columns.test.tsx`,
`command-palette.test.tsx`, `asset-detail-tabs.test.tsx` or the page's
own): each surface shows the badge for each status and nothing without a
mark.

**T3. The notice and the dialog (F7).** As decided. *Check:* tests: the
notice shows the note and the replacement link; no control without
`governance:write`; choosing deprecated sends the note and replacement;
choosing no mark sends `status: null`; a refusal's sentence is shown and
the dialog stays open; after a save the page reloads the asset.

**T4. Docs (planner, same branch).** `CHANGELOG.md`, `docs/core/BACKLOG.md`
(`DATA-12` state, new `DATA-22`), `docs/core/PRODUCT.md` (the `DATA-11` row
now that #98 is merged).

## 5. Build limits on this machine

Same as `docs/superpowers/plans/2026-10-08-src-6-broken-connectors.md`
section 5: shared target dir and two jobs
(`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target
CARGO_BUILD_JOBS=2`), `df -h /` before each cargo command, `touch
rust/crates/*/src/lib.rs rust/crates/lakehouse-store/src/*.rs` before a
quoted clippy run, no `cargo test` here (Rust tests are *not verified*; CI
runs them first), `cargo fmt --check` and `cargo clippy -p lakehouse-api -p
lakehouse-store --all-targets -- -D warnings`. The migration and the code
that uses it land in the same sitting (`sqlx::test` embeds migrations).
TypeScript: `bun install --frozen-lockfile`, then `bun run typecheck && bun
run lint && bun run test` in full.

## 6. Out of scope

- Governed tags (part 2). Marks in the chart builder and Query Studio
  (part 3). Inheritance (`DATA-22`).
- `GET /api/catalog/{id}/annotation` and its missing tenant check
  (`SEC-16`).
- Any change to search ranking; any new permission or role.
- The catalog page's namespace links (found while planning: by reading,
  they filter by a namespace's display name where assets carry its id; a
  backlog note, not this task).

## Handoff

*(developer)*

## Review

*(planner)*
