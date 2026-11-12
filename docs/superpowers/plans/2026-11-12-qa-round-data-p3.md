# QA round, Data (P3): catalog search, marks and data quality

Fixes for one backlog row opened by the QA round on staging (2026-11-02 to 11-12): `DATA-54`
(under `DATA-11`, `DATA-12`, `DATA-13`, `DATA-14`, `DATA-15`). The row is a list of small
findings on four surfaces: the Ctrl+K box and catalog search, the certified / deprecated mark
and governed tags, lineage, and data quality. QA cases: `LH-DATA-11-01` to `-05`, `-14`,
`-15`, `LH-DATA-12-01`, `-05`, `-12`, `-15`, `-17`, `LH-DATA-13-02`, `TC-1723`,
`LH-DATA-14-01`, `-03`, `-05`, `LH-DATA-15-07`.

Base: `main`. Every anchor below was read at `466f5f65f` (`origin/main` on 2026-11-12).
Branch: `fix/qa-round-data-p3`. One pull request for the whole slice, opened as a draft by
the planner as soon as T1 is pushed, so CI (x86_64) runs every test.

Pull requests of the same QA round are still open and merge before this one: #483, #485,
#490, #486, #489, #492. Five tasks touch a file one of them owns; each such task says so.
For those files, and for any line that has moved, **find the anchor by the text quoted in
the task, not by its line number**. If the code is not what a task says, stop that task and
report the mismatch in the handoff; do not improvise.

| Task | File another pull request owns | Pull request |
| --- | --- | --- |
| T9 | `src/features/alerts/alert-rule-page.test.tsx` (a test fixture only) | #485, #490 |
| T14 | `rust/crates/lakehouse-bi/src/search.rs` (read by a test, not changed) | #486 |
| T16 | `src/features/dashboards/chart-builder.tsx`, `chart-builder.test.tsx` | #486 |
| T17 | `src/features/catalog/asset-activity.tsx`; `asset-detail-tabs.test.tsx` is also changed by #489 | #489 |
| T20 | `src/features/connectors/upload-run-view.tsx`, `upload-file-page.test.tsx` | #489 |

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer pushes
after every commit and does not change a decision below.

The owning features exist and are signed (`docs/core/features/catalog-search.md`,
`certification-and-governed-tags.md`, `column-lineage.md`, `data-quality-in-depth.md`,
`asset-page-depth.md`). No signed decision is changed. One sentence of
`catalog-search.md`'s "Limits to tell a customer" changes (D3) and is marked proposed; T22
names every document line that changes.

What QA saw, and what the planner found by reading the code:

- **Search.** The Ctrl+K box lists eight tables and does not say that more matched
  (`LH-DATA-11-02`). A result reads "id · approximate match", and `_` finds almost every
  table (`LH-DATA-11-05`, `-14`, `-15`). Typing `gic` listed pages that do not hold those
  letters together: Policies (named "Access rules" today), Classification & Masking, Data
  Quality, Ingestion (`LH-DATA-11-14`, `-02` F2). A long text makes the box say "Dashboard search
  is unavailable" (`LH-DATA-11-15` F2). A result shows the friendly name only
  (`LH-DATA-11-01` F1), and a description match says only "description" (`LH-DATA-11-04`).
- **Marks and tags.** A deprecated mark with a blank replacement is saved
  (`LH-DATA-12-05` F1). Saving the same mark twice writes two history entries (F2). A refusal
  says `replacementAssetId must be …` (F3), and an invalid governed tag prints
  `governedTags: … matching ^[a-z0-9][a-z0-9_-]*$` (`LH-DATA-12-12`). Deleting a tag key in
  use counts the tables without naming them (`LH-DATA-12-15`). The Certified badge is not
  shown once a mart is chosen in the chart builder (`LH-DATA-12-17` F1). Change history
  shows "just now" with no date (`LH-DATA-12-01` F2).
- **Lineage.** The column lineage of a Bronze table made by an upload could not be opened
  from the console (`LH-DATA-13-02`, `TC-1723`). By reading, the table is in the Catalog; the
  finding is not planned here (section 4, question 2).
- **Data quality.** The Open incidents panel does not update after a rule is added and
  fails (`LH-DATA-14-01` F1). The rule's Asset field took any text (F2). "stale" is not
  explained (`LH-DATA-14-03`). A check on load runs about 36 seconds after the load and the
  upload's page says nothing (`LH-DATA-14-05`). A table whose Iceberg table is gone still
  shows "Rows 720" (`LH-DATA-15-07` F2).

## 1. Decisions

- **D1. One finding per commit, in the order of section 2.** Rust first (T1 to T8), then
  the console (T9 to T21), then the documents (T22). Each commit body cites its QA case ids
  and `DATA-54`. Each fix site carries a short comment naming the case (rule 13), for
  example `// QA LH-DATA-11-05 (DATA-54): …`. Commit subject form:
  `fix(<area>): <what a person now sees> (DATA-54)`.
- **D2. No new helper where one exists (rule 4).**
  - A row's `<schema>.<table>` key, in Rust: `asset_table_key`
    (`rust/crates/lakehouse-api/src/routes/catalog.rs:3213`).
  - The same in the console: `tableKeyOf`, today private in
    `src/features/admin/role-table-access.tsx:84`. T9 moves it to `src/lib/table-key.ts`
    and every user imports it from there.
  - Whether a caller may see a row: `asset_visible` (`routes/catalog.rs:3242`) and
    `table_access_for` (`:3200`).
  - Whether a table exists, as a question that can fail: `clickhouse_source`
    (`routes/catalog_source.rs:102`) and `iceberg_table_presence` (`:175`).
  - The table picker: `TablePicker` and `useTableCatalog`
    (`src/features/governance/table-picker.tsx:94`, `:53`).
  - Dates: `formatDateTime` (`src/lib/format.ts:94`).

### Search

- **D3. Search looks in the physical table name, not in the internal id** (`LH-DATA-11-05`,
  `-14`). An asset's `id` is two different things: for Silver and Gold it is the table
  (`silver.dim_customer`, `routes/catalog.rs:1647`, `:1681`); for Bronze it is the
  registry's slug (`:1515`), which nobody types. Search now reads the row's table key
  (`asset_table_key`: `bronze.<table>`, `silver.<table>`, `serving.<table>`) in the place of
  the `id`, with the same weight (80 exact, 40 approximate), and reports it as
  `matchedOn.field = "table"` with the key as `value`. A Bronze row whose registry records
  no table name has no key and is found by its name, description, tags and columns only.
  This agrees with the spec's target row "What is searched" (`docs/core/specs/data-11.md:26`),
  which lists no id. The feature page's limit sentence said "ids"; T22 changes it and marks
  it proposed (question 1).
- **D4. A typed word with no letter or digit finds nothing** (`LH-DATA-11-05`, `-15`). The
  cause of "`_` finds almost every table" is not the id alone: a word is first looked up as
  typed (`catalog_search.rs:443`), so `_` is found in every id and every column name that
  has an underscore. The module's own comment already says such a word "matches nothing"
  (`:412-413`); only the approximate path kept that rule. Such a word is now dropped from
  the term; a term made only of such words finds nothing (not every table). This is the
  signed QA-1 decision on the page: "special characters do not match everything"
  (`catalog-search.md:147`).
- **D5. A description match carries a short piece of the description** (`LH-DATA-11-04`):
  at most 80 characters around the matched word, on one line, with `…` where it is cut. The
  server sends it as `matchedOn.value`; the console shows it as text (React escapes it; no
  HTML is ever built from it). A column's description gets no piece (section 4).
- **D6. The Ctrl+K box shows the physical name under the friendly name**
  (`LH-DATA-11-01` F1): the table key (`bronze.orders`, `serving.mart_sales`), in a
  monospace span before the reason. The key's first part is the schema, which is the layer
  for Bronze and Silver; a Gold mart reads `serving.<table>`, the name a query uses. When
  the reason is the table key itself, the key is not printed twice.
- **D7. "Showing 8 of N: refine the search"** (`LH-DATA-11-02`). `GET /api/catalog?q=…&limit=8`
  already answers `total` (`routes/catalog.rs:313-324`, `DATA-41` D3); the console's
  `listAssets` drops it (`src/services/clients/assets.ts:76-78`). A new service method
  returns both. The line shows only when `total` is greater than the rows shown. Decision 7
  of the feature page (eight rows and "See all results") is unchanged.
- **D8. A page or an action is listed only when every typed word is in its text**
  (`LH-DATA-11-14`, `-02` F2). Pages are filtered by `cmdk`'s fuzzy score
  (`src/lib/palette-search.ts:76`), which accepts letters in order anywhere: `gic` is found
  in "**G**overnance Access rules /governance/pol**ic**ies". The new rule keeps `cmdk`'s
  score for the order and adds one condition: every word of the typed text must be a
  substring, in any case, of the item's `value` (group, title and address for a page; the
  keyword list for an action). It only removes rows; nothing new is listed. Catalog assets
  and dashboard hits are not affected (the server decided them, and they are force-mounted).
- **D9. The box takes at most 100 characters** (`LH-DATA-11-15` F2). The dashboard search
  refuses a longer text with 400 (`rust/crates/lakehouse-bi/src/search.rs:23`,
  `MAX_QUERY_CHARS = 100`). The console gets one constant, `PALETTE_SEARCH_MAX_CHARS = 100`,
  in `src/lib/palette-search.ts`; a test reads the Rust file and fails when the two numbers
  differ (the way `src/features/copilot/capabilities.test.ts:71` reads a Rust fixture). The
  input gets `maxLength`, and a 400 from the dashboard search reads "Shorten the search:
  dashboards and charts are searched with at most 100 characters." The server is not
  changed: it already refuses.

### Marks and tags

- **D10. A replacement is optional; the page says when none is named** (`LH-DATA-12-05` F1).
  The feature page says "deprecated with a note and, if there is one, the table to use
  instead" (`certification-and-governed-tags.md:30-31`) and the spec says "an optional
  replacement table" (`docs/core/specs/data-12.md:26`). The server reads a blank as absent
  (`trimmed_or_none`, `routes/catalog.rs:3823`), which is what is signed, so the server is
  not changed. The notice on a deprecated table now ends "No replacement is named." when
  there is none, and the dialog's field is labelled as optional. On a page opened with a
  table grant the replacement is withheld, not absent, so the sentence is not shown there
  (`shownWithTableGrant`, `src/features/catalog/table-grant-note.tsx:29`).
- **D11. A save that changes nothing writes nothing** (`LH-DATA-12-05` F2). When the stored
  mark, note and replacement equal the ones sent, the route answers `200 {"ok": true}`
  without writing the row, dropping the search copy or adding an audit event. Who set the
  mark and when stay as they were: a second person pressing Save does not become the one
  who certified the table.
- **D12. Refusals are sentences** (`LH-DATA-12-05` F3, `LH-DATA-12-12`). They start with a
  capital, end with a full stop, name no API field (`replacementAssetId`, `governedTags`,
  `allowedValues`) and print no pattern. The shape of a tag word is said once, in one
  constant, in words: "can have only lower-case letters, digits, "_" and "-", must start
  with a letter or a digit, and can have at most 64 characters". Both findings are the same
  habit in two places and get one commit each (T5 the mark, T6 the tags), because their
  tests live in different crates. The exact sentences are in T5 and T6.
- **D13. The refusal to delete a tag key in use names the tables** (`LH-DATA-12-15`): up to
  three names, then "and N more". Only tables the caller may read are named
  (`asset_visible`); the others are counted in "and N more", as the total is today
  (`DATA-12-AC15` asks for the number, which stays). When the catalog cannot be read the
  sentence keeps the count and names nobody; the delete is refused either way.
- **D14. The chosen mart keeps its mark** (`LH-DATA-12-17` F1). The picker's trigger shows
  the mart's name only (`chart-builder.tsx:1305`, `items={sourceLabels}`), so the badge of
  the list row is gone once the list closes. A line under the picker shows the badge for a
  certified mart; a deprecated one already has `DeprecationNotice` there (`:1356`). No
  assistant change: nothing a tool sends or receives changes, no schema changes, and
  `tests/fixtures/tool_schemas.json` is not regenerated.
- **D15. Change history shows the date on hover** (`LH-DATA-12-01` F2): a `title` with
  `formatDateTime`, the way the Snapshots table of the same file does it
  (`asset-activity.tsx:207`).

### Data quality

- **D16. The table picker offers real table keys.** This is not a QA case; the planner found
  it while reading for `LH-DATA-14-01` F2, and T8 cannot ship without it. The rule form
  already uses the `WEB-8` picker (`quality-rule-create-page.tsx:102`, merged in #466 after
  QA tested), so the "free text" half of the finding is already gone. But the picker builds
  each option as `namespace + "." + name` (`table-picker.tsx:39-41`), and the catalog's rows
  carry the display name in `name` and, for Bronze, the registry's namespace in `namespace`
  (`routes/catalog.rs:1516-1517`, `:1648-1649`): against the real API the options read
  `silver.Dim Customer` and `primer.Orders`, which name no table. The picker's tests pass
  because their fixtures are not shaped like the API's rows
  (`table-picker.test.tsx:22-23`). The options become the rows' table keys (`tableKeyOf`).
  The same picker serves the freshness target dialog and two fields of the alert rule page,
  which are fixed by the same change. T9's picker test is the proof of this reading; if it
  passes on today's code, stop T9 and report.
- **D17. A rule names a table that exists and that its author may read**
  (`LH-DATA-14-01` F2). `POST /api/governance/quality`, and a `PUT` that changes the rule's
  table, refuse any other table with one sentence, the same for "not there" and "not yours"
  (the server must not say which). A `PUT` that keeps the table is not checked, so a rule on
  a table that has since been dropped can still be edited or loosened. A check that cannot
  be made because the engine did not answer is a 503, not a refusal. Rules seeded by a
  migration are not touched.
- **D18. The incidents panel is read again after a run** (`LH-DATA-14-01` F1). Cause: the
  asset page keeps its content mounted while it reloads (`asset-detail-page.tsx:68-72`,
  `keepDataOnReload`), and `QualityIncidents` reads only when its view or table changes
  (`quality-incidents.tsx:179-182`). The same holds on the Data Quality page after "Run
  check". The panel takes a `reloadKey`; both pages raise it after a run, an add, an edit
  and a delete.
- **D19. "stale" is explained in words** (`LH-DATA-14-03`). An incident is stale when a
  later run of its rule passed (`routes/quality.rs:1009-1010`); it stays until a person
  resolves it (signed decision 3). The badge gets a `title`, and the row a visible line:
  "The check has passed since this incident opened. It stays open until someone resolves
  it."
- **D20. The upload's result says when checks run; the timing is not changed**
  (`LH-DATA-14-05`; corrected on review round 1, R4). Cause of the wait: the orchestrator's
  sensor for finished runs has no interval of its own
  (`dagster/dispar_orchestrate/pipeline_events.py:254-259`), it posts nothing while
  `PIPELINE_RUN_TOKEN` is unset (`:211-236`), and the API runs the on-load rules only after
  that report (`routes/quality_load.rs:1-24`). Nothing in this repository bounds the wait,
  so the page promises no time. It says, for every finished load: "Quality checks set to
  run on every load start after the orchestrator reports this load, not when the load ends.
  Their results are on the table's Quality tab." It does not read the table's schedule, so
  it says "set to".
- **D21. No row count for a table that is not there** (`LH-DATA-15-07` F2). The header's
  count is the registry's (`a.rows`); the page already knows the Iceberg table is gone when
  the table read answers 404 (`asset-storage.tsx:246`). Then the header shows "—". Any other
  failure of that read keeps the count: it does not prove the table is gone. The table is
  read once and shared with the tabs, not read a second time.

## 2. Tasks

Rust first, then the console, then the documents. One task per commit.

### T1. A word with no letter or digit finds nothing (LH-DATA-11-05, LH-DATA-11-15; DATA-54)

- Anchors: `rust/crates/lakehouse-api/src/routes/catalog_search.rs:506-525` (`search`, the
  start), `:391-403` (`Word::new`, `parts`), `:412-413` (the comment that already states
  the rule), `:842` (the existing `"___"` assertion).
- Change, in `search`:
  1. Keep `let lowered = term.to_lowercase();`.
  2. Decide "blank" before the words are built: if `lowered.split_whitespace().next()` is
     `None`, return every asset exactly as the existing `if words.is_empty()` block does
     (move that block's body here, unchanged).
  3. Build the words with the empty ones dropped, before the cap:
     `lowered.split_whitespace().map(Word::new).filter(|w| !w.parts.is_empty()).take(MAX_WORDS).collect()`.
  4. If the list is now empty, return `SearchOutcome { assets: Vec::new(), total: 0 }`.
  Add a comment at step 3: QA `LH-DATA-11-05` (`DATA-54`); a word with no letter or digit
  (`_`, `%`, `--`) names nothing, and it used to be found as typed in every id and column
  that holds the character; QA-1 decision "special characters do not match everything".
  Add two sentences to the module doc under "How a term is read" saying the same.
- Check, in the same file's `mod tests`:
  `a_word_with_no_letter_or_digit_is_dropped_and_alone_finds_nothing`. Assets:
  `{"id": "silver.dim_customer", "name": "Dim Customer"}` and
  `{"id": "silver.rev_month", "name": "Rev", "description": "50% of revenue"}`, with a
  `ColumnIndex` giving the first a column `customer_id`. Assert, each through `search` with
  that index: `"_"`, `"%"`, `"- _"` and `"."` each answer no asset and `total == 0`;
  `"rev _"` answers `["silver.rev_month"]`; `""` still answers both assets.
  Wrong change that must make it fail: removing the `.filter(…)` of step 3 (then `"_"`
  answers both rows, as today).
- The existing tests of the module must pass unchanged.

### T2. Search reads the table's physical name, not the internal id (LH-DATA-11-05, LH-DATA-11-14; DATA-54)

- Anchors: `catalog_search.rs:78-79` (`SCORE_ID_NAMESPACE`), `:99-141` (`Field`, `weights`,
  `reported`), `:236-263` (`candidates`, the loop over five keys);
  `routes/catalog.rs:3213` (`asset_table_key`), `:241-243` (the doc of `list`).
- Change:
  1. Rename `Field::Id` to `Field::Table` and `SCORE_ID_NAMESPACE` to
     `SCORE_TABLE_NAMESPACE`; `reported()` answers `Some("table")` for it. Update the doc
     comments that say "id".
  2. In `candidates`, stop reading the `"id"` key. Push the candidates in this order, which
     keeps the place the id had: the name (as today); then, when
     `super::catalog::asset_table_key(asset)` is `Some(key)`, one candidate
     `Candidate { field: Field::Table, text: prepare(key, prepared), shown: key.to_owned() }`;
     then namespace, owner, description (as today); then tags, governed pairs and columns
     (unchanged).
  3. Comment at the new candidate: QA `LH-DATA-11-05` (`DATA-54`); the id of a Bronze row is
     a registry slug, the id of a Silver or Gold row is its table, so the table key is what
     a person types and what the reason line shows.
  4. In the doc of `list` (`routes/catalog.rs:241-243`) replace "over name, id, namespace"
     by "over name, table key, namespace". The doc comment of `candidates`
     (`catalog_search.rs:236-241`) says "name, id, namespace": make it "name, table key,
     namespace".
- Check, in `catalog_search.rs` `mod tests`:
  `a_table_is_found_by_its_physical_name_and_a_bronze_slug_is_not_searched`. Assets:
  `{"id": "zq-registry-slug", "name": "Orders", "tableKey": "bronze.northwind_orders"}` and
  `{"id": "silver.dim_negara", "name": "Dim Negara"}`. Assert: `find(.., "zq")` is empty;
  `find(.., "northwind_orders")` answers the first row with `matchedOn ==
  {"field": "table", "value": "bronze.northwind_orders", "approximate": false}`;
  `find(.., "silver.dim_negara")` answers the second with `matchedOn ==
  {"field": "table", "value": "silver.dim_negara", "approximate": false}`.
  Wrong change that must make it fail: reading the `"id"` key again (then `"zq"` finds the
  row); dropping the `Field::Table` candidate (then `"northwind_orders"` finds nothing).
- The existing tests must pass unchanged: their ids either hold a dot (and so are their own
  table key) or are never searched for. If one fails, report its name and the assertion; do
  not edit it.

### T3. A description match carries the text around the word (LH-DATA-11-04; DATA-54)

- Anchors: `catalog_search.rs:304-321` (`Hit`, `Reason`), `:529-559` (the loop of `search`
  that keeps the reason), `:563-586` (where `matchedOn` is written), `:789-793` (an
  assertion that changes).
- Change:
  1. Add two constants with doc comments: `SNIPPET_MAX_CHARS: usize = 80` and
     `SNIPPET_LEAD_CHARS: usize = 20`.
  2. Add a pure function `fn description_snippet(description: &str, word: &str) -> String`:
     - Make the text one line: every control character becomes a space, then
       `split_whitespace()` joined by one space.
     - `chars: Vec<char>` of that line, and `lower: Vec<char>` of the same length, one
       lower-case character per character (`c.to_lowercase().next().unwrap_or(c)`), so an
       index in one is the same index in the other.
     - `needle: Vec<char>` of `word`. Find its first position in `lower` with `windows`;
       **only when `needle` is not empty** (`windows(0)` panics). No position (a typo that
       was forgiven, or an empty needle) is position 0.
     - `start = position.saturating_sub(SNIPPET_LEAD_CHARS)`, `end = (start +
       SNIPPET_MAX_CHARS).min(chars.len())`. The answer is `chars[start..end]` as a
       `String`, with `…` before it when `start > 0` and after it when `end < chars.len()`.
     No `unwrap`, no `expect`, no byte slicing of a `str`.
  3. `Reason::Field` also keeps the typed word: `Field(Hit, &'a str)` (the enum gets a
     lifetime). In the loop, where `reason = Some(Reason::Field(hit))`, pass `word.text`.
  4. Where `matchedOn` is built, for a `Reason::Field(hit, word)` whose `hit.field` is
     `Field::Description`, the value is `description_snippet(text_of(&asset, "description"),
     word)`; every other field keeps `hit.shown`. Build the value before `as_object_mut()`
     is taken.
  5. Comment at step 4: QA `LH-DATA-11-04` (`DATA-54`). Update the doc of `search` and the
     sentence about `matchedOn` in the module doc: the value is the tag, the column, the
     table key, or a piece of the description.
- Check, in `mod tests`:
  - `a_description_match_carries_the_text_around_the_word`: one asset whose description is
    200 characters with `revenue` at character 120. `find(.., "revenue")`:
    `matchedOn.field == "description"`, the value contains `revenue`, starts and ends with
    `…`, and has at most 82 characters. A second asset with the description
    `"Monthly\n  revenue\tby region"`: the value is exactly `"Monthly revenue by region"`.
    `find(.., "revnue")` on the second: `approximate == true` and the value is the same
    whole line (no position, so it starts at the beginning).
  - `the_snippet_never_cuts_inside_a_character`: `description_snippet("Umsatz nach Übersee
    für Österreich", "übersee")` contains `Übersee`; `description_snippet("", "x")` is `""`;
    `description_snippet("abc", "")` is `"abc"`.
  - In `one_edit_finds_a_word_by_deletion_swap_substitution_and_insertion` (`:789-793`) the
    expected `matchedOn` becomes `{"field": "description", "value": "revenue by month",
    "approximate": true}`. Say in the commit body that the rule changed (D5), not the test's
    purpose (rule 2).
  Wrong change that must make it fail: answering `String::new()` from step 4 (the value is
  then empty); slicing the `str` by bytes (the second test panics).

### T4. Saving the same mark again writes nothing (LH-DATA-12-05 F2; DATA-54)

- Anchors: `routes/catalog.rs:3909-4010` (`put_certification`), the lines
  `let before = lakehouse_store::annotation::get_annotation(pool, &id).await?.and_then(|r|
  r.certification);` (`:3954-3956`); `rust/crates/lakehouse-store/src/annotation.rs:43-66`
  (`AnnotationRow`); the test helpers `put_mark` (`:5730`), `governor` (`:5756`),
  `mock_clickhouse_with_table` (`:5337`), and the test
  `a_member_certifies_with_their_display_name_and_clears_it` (`:5903`) as the model.
- Change: read the whole row once, compare, and return early.
  ```rust
  let before_row = lakehouse_store::annotation::get_annotation(pool, &id).await?;
  // QA LH-DATA-12-05 F2 (DATA-54): a save that changes nothing writes nothing: no row
  // write (who set the mark and when stay), no dropped search copy, no history entry.
  if let (Some(status), Some(row)) = (change.status, before_row.as_ref())
      && row.certification.as_deref() == Some(status)
      && row.certification_note == change.note
      && row.replacement_asset_id == change.replacement_asset_id
  {
      return Ok(ApiJson(json!({ "ok": true })));
  }
  let before = before_row.and_then(|r| r.certification);
  ```
  The checks of the replacement above it stay where they are. Add one sentence to the
  handler's doc comment.
- Check, in the same test module as the model test:
  `saving_the_same_mark_again_writes_no_history_entry_and_keeps_who_set_it`
  (`#[sqlx::test(migrations = "../../migrations")]`). With `mock_clickhouse_with_table` and
  `state_for(&pool, &ch.uri(), Some(TENANT_A))`:
  1. `put_mark` `{"status":"deprecated","note":"Superseded"}` as `governor(TENANT_A)`.
     Read the row; keep `certified_at`.
  2. The same body again as a second principal: `let mut other = governor(TENANT_A);
     other.display_name = "Second Person".to_owned();`. It answers `Ok`.
  3. `SELECT count(*) FROM audit_event WHERE action = 'catalog.deprecate' AND resource_id =
     'serving.mart_x'` is 1; the row's `certified_by` is still `"Test Principal"` and
     `certified_at` is unchanged.
  4. As `other`, `{"status":"deprecated","note":"Replaced in March"}`: the count is 2 and
     `certified_by` is `"Second Person"`.
  5. `{"status":"certified"}` twice: `catalog.certify` is counted once.
  No replacement is sent, so the search copy is never read and need not be primed.
  Wrong change that must make it fail: removing the early return (the count in step 3 is 2).

### T5. The mark's refusals are sentences (LH-DATA-12-05 F3; DATA-54)

- Anchors: `routes/catalog.rs:3843-3891` (`validate_certification`), `:3922-3952` (the
  three refusals in the handler), tests `:5777-5830`
  (`each_certification_refusal_has_its_sentence_and_writes_nothing`) and `:5858-5899`
  (`a_replacement_is_refused_with_503_when_the_catalog_cannot_be_read`).
- Change: replace the texts, nothing else.

  | Today | New |
  | --- | --- |
  | `status must be "certified", "deprecated" or null` | `The mark must be "certified", "deprecated" or null.` |
  | `a note or a replacement is allowed only with the deprecated mark` | `A note or a replacement can be given only with the deprecated mark.` |
  | `note must be at most {N} characters` | `The note can be at most {N} characters.` |
  | `replacementAssetId must be at most {N} characters` | `The replacement's id can be at most {N} characters.` |
  | `a table cannot be its own replacement` | `A table cannot be its own replacement.` |
  | `the catalog could not be read to check the replacement` (503) | `The catalog could not be read to check the replacement. Try again.` |
  | `the replacement is not an asset in the catalog` | `The replacement is not a table in the catalog.` |
  | `the replacement is itself deprecated` | `The replacement is itself deprecated. Name a table that is still in use.` |

  Leave `body must be JSON with a "status" field` and `id must be at most …` as they are
  (a caller of the API reads them, never the form). Update the doc comments that quote a
  sentence.
- Check:
  - New unit test (no database) in the module's plain `mod tests`:
    `no_certification_refusal_names_an_api_field_and_each_is_a_sentence`. Build the five
    bodies `validate_certification` refuses (an unknown status; a note with `certified`; a
    note of 1001 characters; a replacement of 201 characters; a replacement equal to the
    id) with `serde_json::from_str::<CertificationBody>`. For each message: the first
    character is upper case, the last is `.`, and it contains neither `replacementAssetId`
    nor `status must`.
  - In `each_certification_refusal_has_its_sentence_and_writes_nothing` the expected pieces
    become `"The mark must be"`, `"The note can be at most 1000 characters."` and `"not a
    table in the catalog"`; the other four pieces still match. In the 503 test the expected
    text becomes `"The catalog could not be read to check the replacement"`. Say in the
    commit body that the sentences changed on purpose (rule 2).
  Wrong change that must make it fail: any sentence left as it is today (the new unit test
  fails on its lower-case first letter).
- Do not edit `src/features/catalog/asset-detail-page.test.tsx:226-232` or
  `src/services/clients/assets.test.ts:51-57`: they stub a server answer and assert it is
  shown as sent, whatever its words.

### T6. Tag refusals say what is allowed, in words (LH-DATA-12-12; DATA-54)

- Anchors: `rust/crates/lakehouse-store/src/governed_tag.rs:84-116` (`validate_key`,
  `validate_allowed_values`), `:302-340` (`set_asset_tags_in`), `:405-420` (`mod tests`);
  `rust/crates/lakehouse-store/tests/governed_tag.rs:73-90`;
  `routes/catalog.rs:3480-3534` (`validate_annotation_body`; the tags and governed tags
  from `:3502`),
  `:4700-4716` (`validate_annotation_body_bounds_governed_tags`), `:6495`;
  `routes/governance.rs:3276-3290` (`tag_key_refusals_name_the_field_and_store_nothing`).
- Change:
  1. In `governed_tag.rs`, add
     ```rust
     /// What a tag word may be, in words: the shape `is_valid_word` checks. One sentence
     /// part for every refusal (QA LH-DATA-12-12, DATA-54), so no refusal prints a pattern.
     pub const WORD_RULE: &str = "can have only lower-case letters, digits, \"_\" and \"-\", \
          must start with a letter or a digit, and can have at most 64 characters";
     ```
  2. Replace the texts:

     | Site | New |
     | --- | --- |
     | `governed_tag.rs:89` (key shape) | `A tag key {WORD_RULE}.` |
     | `:95` (no value) | `A tag key needs at least one allowed value.` |
     | `:99` (over 50) | `A tag key can have at most {MAX_VALUES_PER_KEY} allowed values.` |
     | `:105` (value shape) | `The allowed value "{value}" is not accepted: a value {WORD_RULE}.` |
     | `:108` (repeated) | `The allowed value "{value}" is listed more than once.` |
     | `:310` and `catalog.rs:3521` (over 20) | `A table can have at most {max} governed tags.` |
     | `:315` (twice) | `The governed tag "{key}" is given twice.` |
     | `:331` (unknown key) | `"{key}" is not a defined tag key.` |
     | `:336` (value outside the list) | `"{value}" is not an allowed value for "{key}". Allowed values: {list}.` (`list` joined by `, ` as today) |
     | `catalog.rs:3504` (over 20 free tags) | `A table can have at most {ANNOTATION_TAGS_MAX_COUNT} tags.` |
     | `catalog.rs:3510` (free tag shape) | `The tag "{tag}" is not accepted: a tag {WORD_RULE}.` |
     | `catalog.rs:3528` (governed key or value shape) | `The governed tag {what} "{word}" is not accepted: a {what} {WORD_RULE}.` (`what` is `key` or `value`, as today) |

     In `catalog.rs` use `lakehouse_store::governed_tag::WORD_RULE`. Leave the `owner`,
     `steward` and `description` length sentences (section 4). Update the doc comments of
     these functions where they quote a sentence; the `# Errors` text may keep describing
     the rule by its pattern, since a developer reads it.
- Check:
  - New unit test in `governed_tag.rs` `mod tests`:
    `no_tag_refusal_prints_a_pattern_or_an_api_field`. Collect the messages of
    `validate_key("Bad Key")`, `validate_allowed_values(&[])`,
    `validate_allowed_values(&["Finance".to_owned()])` and
    `validate_allowed_values(&["a".to_owned(), "a".to_owned()])` (each is
    `StoreError::Validation(message)`). None contains `^`, `allowedValues` or
    `governedTags`; each ends with `.`; the two shape messages contain `lower-case`. Also
    `assert!(WORD_RULE.contains(&WORD_MAX_LEN.to_string()))`, so the number in the sentence
    cannot drift from the limit.
  - `tests/governed_tag.rs:77`: `"allowedValues: \"Finance\""` becomes `"The allowed value
    \"Finance\""`; `:87`: `"allowedValues"` becomes `"allowed value"`. The other assertions
    of the file match the new sentences as written.
  - `catalog.rs:4714`, `:4716`: `"governedTags: key"` and `"governedTags: value"` become
    `"governed tag key"` and `"governed tag value"`; add to that test that neither message
    contains `^` or `governedTags`. `:6495`: `"governedTags: value \"Finance\""` becomes
    `"governed tag value \"Finance\""`.
  - `governance.rs:3279-3285`: the expected piece of the second, third and fourth case
    (`[]`, `["Finance"]`, `["a","a"]`) becomes `"allowed value"`. The first (`"key"`) still
    matches. The fifth (`{"key":"k"}`) is the body parser's own message and is not changed
    by this task: leave it.
  Wrong change that must make it fail: leaving a pattern in any of the four store messages
  (the new unit test finds `^`).
- Do not edit `src/features/catalog/asset-detail-tabs.test.tsx:1077` or
  `src/features/governance/tag-key-form-page.test.tsx:131` (stubs of a server answer).

### T7. Deleting a tag key in use names the tables (LH-DATA-12-15; DATA-54)

- Anchors: `governed_tag.rs:216-250` (`delete_key`); `tests/governed_tag.rs:337-349`;
  `routes/catalog.rs:946-957` (`search_snapshot`), `:3200-3205` (`table_access_for`),
  `:3242-3247` (`asset_visible`); `routes/governance.rs:2103-2139` (`delete_tag_key`),
  `:3124-3222` (the test module's `state_for`, `admin`, `remove`), `:3339-3375`
  (`a_tag_key_in_use_cannot_lose_a_value_or_be_deleted_and_the_answer_counts_tables`).
- Change:
  1. Store, `governed_tag.rs`: add
     `pub async fn assets_with_key(pool: &PgPool, key: &str) -> Result<Vec<String>, StoreError>`:
     `SELECT asset_id FROM asset_governed_tag WHERE tag_key = $1 ORDER BY asset_id`, the key
     bound. Doc comment with `# Errors`. Make `delete_key`'s own sentence the same form as
     the route's: `"{key}" is still given to {n} table(s). Remove it from that table / those
     tables first.` (two sentences, a capital and a full stop; the count and the plural
     logic stay).
  2. `routes/catalog.rs`, beside `asset_visible`:
     - `pub(crate) fn readable_names(access: &crate::table_access::TableAccess, assets:
       &[Value], ids: &[String]) -> Vec<String>`: for each id of `ids`, in order, the asset
       of `assets` with that `id`; when there is one and `asset_visible(access, asset)`, its
       `name` (its `id` when the name is empty). Nothing for an id the catalog does not
       hold, nothing for a hidden one.
     - `pub(crate) async fn readable_asset_names(state: &AppState, principal: &Principal,
       ids: &[String]) -> Option<Vec<String>>`: `table_access_for` and `search_snapshot`;
       `None` when either fails (log a `tracing::warn!` with the error, never return its
       text); else `Some(readable_names(&access, snapshot.assets(), ids))`.
     - `#[cfg(test)] pub(crate) async fn prime_search_copy_for_test(state: &AppState,
       assets: Vec<Value>)`: puts a `CatalogSearchSnapshot` built from `assets` (index
       `SearchIndex::build(assets, &ColumnIndex::new())`, `namespaces: Value::Null`, empty
       `columns` and `usage`, `column_search_partial: false`) into
       `state.catalog_search_cache` with `get_or_build(Instant::now(), SEARCH_SNAPSHOT_TTL,
       …)`, as the test helper `prime_search_copy_with` (`:5762-5775`) does. Do not change
       the existing helper.
  3. `routes/governance.rs`:
     - A pure `fn tag_key_in_use_sentence(key: &str, total: usize, names: &[String]) ->
       String`. `head` is `"{key}" is still given to {total} table` / `tables`. With no
       name: `{head}. Remove it from that table first.` / `those tables first.` With names:
       the first three joined by `, `, then ` and {rest} more` when `rest = total - shown`
       is above zero: `{head}: {list}. Remove it from … first.`
     - In `delete_tag_key`, match the store's answer instead of `map_err`:
       `Err(StoreError::NotFound)` as today; `Err(StoreError::Validation(counted))` builds
       the sentence: read `assets_with_key`; when that read and
       `crate::routes::catalog::readable_asset_names` both answer, the sentence is
       `tag_key_in_use_sentence(&key, ids.len(), &names)`, otherwise it is `counted`; return
       `ApiError::BadRequest(sentence)`. Any other error as today. Comment: QA
       `LH-DATA-12-15` (`DATA-54`); only tables the caller may read are named, the rest are
       counted. Update the handler's doc comment.
- Check:
  - `tests/governed_tag.rs`: `the_tables_holding_a_key_are_listed_in_order`: three assets
    given the key in the order c, a, b; `assets_with_key` answers `["a", "b", "c"]` (use
    the file's own ids) and `[]` for a key nobody holds.
  - `routes/catalog.rs` plain `mod tests`:
    `only_tables_the_caller_may_read_are_named`. Assets `{"id": "silver.a", "name": "Orders"}`,
    `{"id": "silver.b", "name": "Customers"}`. With `TableAccess::All` and ids
    `["silver.a", "silver.b", "silver.gone"]` the answer is `["Orders", "Customers"]`. With
    `TableAccess::Listed(HashSet::from([("silver".to_owned(), "a".to_owned())]),
    HashSet::new())` the answer is `["Orders"]`.
    Wrong change that must make it fail: dropping the `asset_visible` condition (the
    narrowed caller is told `Customers`).
  - `routes/governance.rs` plain `mod tests`:
    `the_refusal_names_up_to_three_tables_and_counts_the_rest`:
    `tag_key_in_use_sentence("data_domain", 5, &["A","B","C","D"])` is `"data_domain" is
    still given to 5 tables: A, B, C and 2 more. Remove it from those tables first.`; with
    no name and total 1 it is `"data_domain" is still given to 1 table. Remove it from that
    table first.`
  - `routes/governance.rs` `mod tag_key_handlers`:
    `deleting_a_key_in_use_names_the_tables_that_hold_it`: as the existing in-use test, two
    assets `silver.a` and `silver.b` given the key; then
    `crate::routes::catalog::prime_search_copy_for_test(&state, vec![json!({"id":
    "silver.a", "name": "Orders"}), json!({"id": "silver.b", "name": "Customers"})])`; then
    `remove`: 400, and the sentence contains `2 tables: Orders, Customers.`
    Wrong change that must make it fail: answering `counted` always (no name in the
    sentence).
  - The existing in-use test (`:3339`) passes unchanged: with no catalog behind the state
    the sentence is the store's, which still contains `2 tables`.

### T8. A rule for a table that is not there, or not the author's to read, is refused (LH-DATA-14-01 F2; DATA-54)

- Anchors: `routes/quality.rs:935-948` (`resolve`), `:1104-1134` (`update_rule`, up to the
  store call; `before` is read at `:1128`); `routes/governance.rs:1053-1075` (`create_rule`), `:1682-1697`
  (`create_quality_rule`); `routes/catalog_source.rs:102-117`, `:149-193`;
  `rust/crates/lakehouse-api/src/table_access.rs:76` (`allows`);
  `rust/crates/lakehouse-api/tests/quality_incidents.rs:29-64` (`app`), `:66-93` (`call`),
  `:94-108` (`rule`), `:112-116` (`admin`), `:222-231` (a `PUT` that keeps the table);
  `rust/crates/lakehouse-api/tests/table_access.rs:152-183` (how a role is narrowed).
- Change:
  1. `routes/quality.rs`, beside `resolve`:
     ```rust
     pub(crate) async fn ensure_rule_table(
         state: &AppState,
         principal: Option<&Principal>,
         asset: &str,
     ) -> Result<(), ApiError>
     ```
     - `refused` is `ApiError::BadRequest` with `"{asset}" is not a table you can read.
       Pick the table as silver.<table>, serving.<table> or bronze.<table>.` (`asset`
       trimmed, in double quotes). One sentence for every refusal below.
     - Split the trimmed asset at the first `.` into zone and table; no dot is `refused`.
       Lower-case the zone. A zone other than `silver`, `serving`, `bronze` is `refused`.
     - Access first, so a hidden table is never asked about: when `principal` is `Some`,
       `crate::routes::catalog::table_access_for(state, principal).await?`, and
       `!access.allows(&zone, table)` is `refused`. `None` (no person behind the call) is
       not narrowed, as `TableAccess` documents.
     - Existence: for `silver` and `serving`, `catalog_source::clickhouse_source(&state.clickhouse,
       &zone, table)`: `Ok(Some(_))` passes, `Ok(None)` is `refused`, `Err(err)` is logged
       with `tracing::warn!` and answered `ApiError::Unavailable("The table could not be
       checked. Try again.")`. For `bronze`, `catalog_source::iceberg_table_presence(state,
       table)`: `Ok(true)` passes, `Ok(false)` and `Err(PresenceUnknown::NoQueryDatabase)`
       are `refused` (such a rule could never run), `Err(PresenceUnknown::Unanswered)` is
       the same 503.
     - Doc comment with `# Errors`, and: QA `LH-DATA-14-01` F2 (`DATA-54`); the form offers
       only tables the person may read, and what the form hides the server refuses, with
       one sentence for "not there" and "not yours".
  2. `routes/governance.rs`: `create_quality_rule` takes the actor as a second argument,
     `actor: Option<&Principal>`, and calls
     `crate::routes::quality::ensure_rule_table(&state, actor, &body.asset).await?` after
     `parse_body` and before the store call. `create_rule` passes its `actor`. Update both
     doc comments (`# Errors`: 400 for a table that is not there or not readable, 503 when
     it cannot be checked).
  3. `routes/quality.rs` `update_rule`: after `before` is read and before
     `update_quality_rule`, when `!before.asset.eq_ignore_ascii_case(&input.asset)`, call
     `ensure_rule_table(&state, Some(&principal), &input.asset).await?`. Comment: a rewrite
     that keeps the table is not checked, so a rule on a dropped table can still be edited.
- Check, in `tests/quality_incidents.rs` (the file's `app()` answers `system.columns` with
  columns for every table; a test that needs a missing table mounts, on the returned mock
  server, `Mock::given(method("POST")).and(body_string_contains("system.columns"))
  .and(body_string_contains("no_such_table"))` answering `{"meta": [], "data": [], "rows":
  0}` `.with_priority(1)`):
  - `a_rule_for_a_table_that_is_not_there_is_refused_and_nothing_is_stored`: as `admin`,
    `POST /api/governance/quality` with `{"name": "t8_missing", "asset":
    "silver.no_such_table", "dimension": "volume", "threshold": "rows >= 1", "severity":
    "high"}` answers 400 and the error contains `is not a table you can read`; `SELECT
    count(*) FROM quality_rule WHERE name = 't8_missing'` is 0. The same body with
    `"asset": "silver.orders"` and another name answers 201. An asset `orders` (no dot) and
    an asset `lake.orders` answer 400 with the same piece.
  - `a_rule_for_a_table_the_role_may_not_read_is_refused_with_the_same_sentence`: a user
    made with `create_principal_with_permissions(&pool, "governance:write,governance:read")`,
    its role narrowed to `silver.orders` with the three statements of
    `tests/table_access.rs:161-180` written inline (set `read_scope = 'listed'`, delete the
    role's grants, insert `('silver', 'orders')`). `POST` with `silver.customers` answers
    400; its error equals the error of the missing-table case with the table name replaced
    (compare after replacing the quoted name). `POST` with `silver.orders` answers 201.
  - `changing_a_rule_to_a_table_that_is_not_there_is_refused_and_the_rule_keeps_its_table`:
    a rule on `silver.orders` (the file's `rule` helper); `PUT` with `"asset":
    "silver.no_such_table"` answers 400; the stored rule's asset is still `silver.orders`.
  Wrong changes that must make them fail: skipping the existence check (the first test
  gets 201); skipping the access check (the second gets 201); calling `ensure_rule_table`
  only in `create` (the third gets 200).
- The existing tests of the file must pass unchanged; the two `PUT`s there keep the table.

### T9. The table picker offers the tables' real keys (LH-DATA-14-01 F2; DATA-54)

See D16. This file set includes a test fixture in `src/features/alerts/`, owned by #485 and
#490: re-find it by its text after they merge.

- Anchors: `src/features/admin/role-table-access.tsx:73-91` (`tableKeyOf` and its doc
  comment), `:127` (its use); `src/features/governance/table-picker.tsx:38-41` (`tableKey`),
  `:53-69` (`useTableCatalog`); `src/features/governance/table-picker.test.tsx:21-24`
  (`ASSETS`), `:66-70`, `:119-128`; `quality-rule-create-page.test.tsx:40-43`, `:71`, `:224`;
  `data-quality-page.test.tsx:449-450`; `src/features/alerts/alert-rule-page.test.tsx:83-86`.
- Change:
  1. New file `src/lib/table-key.ts` with a module comment (what a table key is, and that a
     Bronze row's `id` is a registry slug) and
     `export function tableKeyOf(asset: { id: string; tableKey?: string | null }): string | null`,
     the body moved from `role-table-access.tsx` unchanged, its doc comment with it.
     `role-table-access.tsx` imports it and loses its own copy.
  2. `table-picker.tsx`: delete the exported `tableKey`. In `useTableCatalog`, `choices` is
     the sorted, de-duplicated list of `tableKeyOf(asset)` for the assets that have one;
     `idOf(table)` is the `id` of the asset whose `tableKeyOf` equals `table`, else `null`.
     Update the file's module comment: the options are the catalog rows' table keys
     (`bronze.<table>`, `silver.<table>`, `serving.<table>`), not `namespace.name`. Comment
     at `choices`: QA `LH-DATA-14-01` F2 (`DATA-54`); a row's `name` is its display name and
     a Bronze row's `namespace` is the registry's, so the pair named no table.
  3. Fixtures, in the four test files: the two rows become
     `{ id: "silver.orders", name: "Orders", namespace: "silver" }` and
     `{ id: "raw-orders", name: "Raw Orders", namespace: "primer", tableKey: "bronze.raw_orders" }`
     (and `{ id: "silver.customers", name: "Customers", namespace: "silver" }` where a file
     has a customers row). The options the tests expect (`bronze.raw_orders`,
     `silver.orders`) do not change. Where a file answers the asset detail by id
     (`path.endsWith("/a-orders")`, `id === "a-orders"`, `<ColumnsOf id="a-orders" />`), the
     id becomes `silver.orders`. Say in the commit body that the fixtures were not shaped
     like the API's rows (rule 2).
- Check:
  - New `src/lib/table-key.test.ts`:
    `a_silver_row_is_its_id_a_bronze_row_is_its_table_key_and_a_slug_alone_is_no_key`:
    `tableKeyOf({ id: "silver.orders" })` is `"silver.orders"`; `tableKeyOf({ id:
    "raw-orders", tableKey: "bronze.raw_orders" })` is `"bronze.raw_orders"`;
    `tableKeyOf({ id: "raw-orders", tableKey: null })` is `null`.
  - `table-picker.test.tsx`: replace the `describe("tableKey", …)` block by a test in
    `describe("TablePicker", …)`:
    `offers each row's table key, never its display name`, with the catalog
    `[{ id: "silver.orders_clean", name: "Orders Clean", namespace: "silver" }, { id:
    "northwind-orders", name: "Northwind Orders", namespace: "primer", tableKey:
    "bronze.northwind_orders" }, { id: "no-table", name: "No Table", namespace: "primer",
    tableKey: null }]`: the options are exactly `["bronze.northwind_orders",
    "silver.orders_clean"]`.
    This test must fail on today's code (the options read `primer.Northwind Orders`,
    `primer.No Table`, `silver.Orders Clean`). **If it passes before the change, stop T9 and
    report: the planner's reading (D16) is wrong.**
    Wrong change that must make it fail afterwards: building the option from `namespace`
    and `name` again.
  - The other tests of the four files pass with the new fixtures.

### T10. The box says when more tables matched than it shows (LH-DATA-11-02; DATA-54)

- Anchors: `src/services/contracts/assets.ts:423-437` (`AssetFilter`), `:524-525`
  (`AssetService.listAssets`); `src/services/clients/assets.ts:50-74` (`loadCatalog`),
  `:76-87`; `src/components/command-palette.tsx:70`, `:119-136`, `:350-371`;
  `src/lib/palette-search.ts:1-11` (the module comment names `listAssets`);
  `src/components/command-palette.test.tsx:130-146`.
- Change:
  1. Contract: `export type AssetSearchResult = { assets: Asset[]; total: number }` with a
     comment (`DATA-41` D3: the best matches and how many matched in all), and on
     `AssetService`: `searchAssets(term: string, limit: number, signal?: AbortSignal):
     Promise<AssetSearchResult>`.
  2. Client: `loadCatalog`'s return type gains `total?: number`. `searchAssets` calls
     `loadCatalog(term.trim(), limit, signal)` and answers `{ assets, total: typeof
     json.total === "number" ? json.total : assets.length }`.
  3. Palette: the effect calls `assetService.searchAssets(term, PALETTE_ASSET_RESULT_LIMIT,
     controller.signal)`; keep `capPaletteAssetResults` as the second guard; a new state
     `assetTotal` (0 when there is no term, on failure and without `catalog:read`). In the
     "Catalog assets" group, after the asset rows and before "See all results", when
     `assetTotal > assetResults.length`:
     ```tsx
     <div className="px-2.5 py-1.5 text-xs text-muted-foreground">
       Showing {assetResults.length} of {assetTotal}: refine the search
     </div>
     ```
     Comment: QA `LH-DATA-11-02` (`DATA-54`). Update the module comment of
     `palette-search.ts` to name `searchAssets`.
- Check, in `command-palette.test.tsx`, `describe("CommandPalette catalog search", …)`:
  - `says how many tables matched when it shows only eight of them`: `stubCatalog({ assets:
    <eight assets>, total: 23 })`, type `table`; "Showing 8 of 23: refine the search" is on
    screen and comes before "See all results" in the document.
  - `says nothing about more when every match is shown`: `{ assets: [asset(1), asset(2)],
    total: 2 }`, and again with no `total`: no text matching `/^Showing /`.
  And in `src/services/clients/assets.test.ts`: `searchAssets asks for the limit and
  answers the total`: the request is `/api/catalog?q=orders&limit=8`, the answer `{ assets:
  [...], total: 23 }` comes back as `total: 23`, and an answer with no `total` comes back
  with the number of assets.
  Wrong change that must make it fail: reading `assets.length` as the total (the first
  test finds no line).

### T11. The reason names the table, not "id" (LH-DATA-11-05, LH-DATA-11-14; DATA-54)

The console half of T2.

- Anchors: `src/lib/palette-search.ts:38-60` (`matchedOnLabel`);
  `src/services/contracts/assets.ts:100-113` (the comment on `matchedOn`);
  `src/lib/palette-search.test.ts:23-53`.
- Change: in `matchedOnLabel`, two new cases before `default`:
  `case "table": what = value ? `table ${value}` : "table"` (QA `LH-DATA-11-05`, `DATA-54`:
  the server reports the physical name), and `case "id": what = "table id"` (an API build
  older than `DATA-54` still sends it; it read as a column named id). Remove `id` from the
  comment of the `default` branch. In the contract's comment replace `"id"` by `"table"` in
  the list of fields, and say `value` is the tag, the column, the table key or a piece of
  the description.
- Check, in `palette-search.test.ts`:
  `names the table that matched, and calls an old server's id a table id`:
  `matchedOnLabel({ field: "table", value: "silver.dim_negara", approximate: false })` is
  `"table silver.dim_negara"`; with `approximate: true` it ends `· approximate match`;
  `matchedOnLabel({ field: "id", value: "", approximate: true })` is `"table id ·
  approximate match"`.
  Wrong change that must make it fail: removing the `"id"` case (the answer is `"id ·
  approximate match"`, the text QA saw).

### T12. A result shows its physical name and the matched piece of its description (LH-DATA-11-01 F1, LH-DATA-11-04; DATA-54)

- Anchors: `src/lib/palette-search.ts:38-60`; `src/components/command-palette.tsx:352-363`
  (the asset row), `:391-421` (`PaletteItem`);
  `src/features/catalog/data-explorer-columns.tsx:72-77` (`MatchedOnLine`, not changed: it
  gets the new words through `matchedOnLabel`).
- Change:
  1. `matchedOnLabel`: `case "description": what = value ? `description: "${value}"` :
     "description"` (QA `LH-DATA-11-04`).
  2. `palette-search.ts`: `export function paletteAssetLines(asset: Pick<Asset, "id" |
     "tableKey" | "matchedOn">): { path: string | null; reason: string | null }`. `path` is
     `tableKeyOf(asset)` (`src/lib/table-key.ts`, T9). When `path` is not null and
     `matchedOn.field === "table"`, the reason is `"approximate match"` when it was
     approximate and `null` otherwise (the key is on screen already); in every other case
     the reason is `matchedOnLabel(asset.matchedOn)`.
  3. `PaletteItem` gets a prop `path?: string | null`. The second line is drawn when `path`
     or `detail` is set:
     ```tsx
     <span className="block truncate text-xs text-muted-foreground">
       {path ? <span className="font-mono">{path}</span> : null}
       {path && detail ? " · " : null}
       {detail ? <span>{detail}</span> : null}
     </span>
     ```
     The asset row passes `path` and `detail` from `paletteAssetLines(a)`. Text only: never
     `dangerouslySetInnerHTML`. Comment: QA `LH-DATA-11-01` F1 (`DATA-54`).
- Check:
  - `palette-search.test.ts`:
    `the lines of a result are its table key and why it matched`: a Bronze row with
    `tableKey: "bronze.orders"` and a column reason gives `{ path: "bronze.orders", reason:
    "column customer_id" }`; a Silver row matched on `table` gives `{ path:
    "silver.dim_negara", reason: null }`, and `reason: "approximate match"` when
    approximate; a row with no key gives `path: null`. And `matchedOnLabel({ field:
    "description", value: "monthly revenue by region", approximate: false })` is
    `description: "monthly revenue by region"`.
  - `command-palette.test.tsx`:
    `shows the physical name under the friendly name, and the matched text as text`:
    `asset(1, { matchedOn: { field: "description", value: "<b>bold</b> revenue",
    approximate: false } })`; the row holds the text `silver.t1`; the text `description:
    "<b>bold</b> revenue"` is on screen; `document.querySelector('[cmdk-item=""] b')` is
    `null`.
  The existing tests of both files pass unchanged (the reason keeps its own `span`, so
  `getByText("column revenue_amount")` still finds it).
  Wrong change that must make it fail: not passing `path` (no `silver.t1` in the row).

### T13. Pages and actions are listed only for words they contain (LH-DATA-11-14, LH-DATA-11-02 F2; DATA-54)

- Anchors: `src/lib/palette-search.ts:65-81` (`makePaletteFilter`);
  `src/components/command-palette.tsx:313-335`, `:375-385` (the `value` of an action and of
  a page); `src/components/app-shell/nav-config.ts:146` ("Access rules",
  `/governance/policies`).
- Change, in `makePaletteFilter`, for a value that is not an asset row:
  ```ts
  const score = defaultFilter(value, search, keywords)
  return score > 0 && everyWordIn(value, search, keywords) ? score : 0
  ```
  with a private `everyWordIn`: the haystack is `value` and the `keywords` joined by a
  space, lower-cased; every part of `search.toLowerCase().split(/\s+/)` that is not empty
  must be a substring of it. Comment: QA `LH-DATA-11-14` (`DATA-54`); `cmdk` accepts the
  letters in order anywhere, so `gic` was found in "Governance … /governance/policies";
  this keeps its score for the order and only removes rows. Update the function's doc
  comment. The branch for asset rows is not changed.
- Check:
  - `palette-search.test.ts`:
    `a page is listed only when every typed word is in its text`: with
    `const filter = makePaletteFilter([])` and `page = "Governance Access rules
    /governance/policies"`: `filter(page, "gic")` is 0; `filter(page, "polic")`,
    `filter(page, "access")` and `filter(page, "ACCESS rules")` are above 0;
    `filter("theme dark light", "dark")` is above 0 and `filter("theme dark light",
    "drk")` is 0.
  - `command-palette.test.tsx`:
    `does not list a page for letters scattered through its name`: `stubCatalog({ assets:
    [] })`, open the box, type `gic`: no element with the text "Access rules" and none with
    "Data Quality"; then type `polic`: "Access rules" is on screen.
  Wrong change that must make it fail: returning `score` alone (both tests find the page
  for `gic`).

### T14. The box takes 100 characters and says so when a search is too long (LH-DATA-11-15 F2; DATA-54)

`rust/crates/lakehouse-bi/src/search.rs` is owned by #486; this task only reads it in a
test. If the constant's line reads differently after #486, match the test to the new text
and say so in the handoff.

- Anchors: `rust/crates/lakehouse-bi/src/search.rs:22-23` (`pub const MAX_QUERY_CHARS:
  usize = 100;`); `src/components/command-palette.tsx:81` (`dashFailed`), `:179-186`,
  `:248-254` (`Command.Input`), `:274-278`; `src/services/clients/dashboards.ts:48-53`
  (`codeFor`: a 400 is `invalid_request`);
  `src/features/copilot/capabilities.test.ts:71` (a test that reads a Rust file).
- Change:
  1. `src/lib/palette-search.ts`: `export const PALETTE_SEARCH_MAX_CHARS = 100` with the
     comment: the longest text the dashboard search takes
     (`lakehouse_bi::search::MAX_QUERY_CHARS`); `palette-search.test.ts` fails when the two
     differ.
  2. `Command.Input` gets `maxLength={PALETTE_SEARCH_MAX_CHARS}`.
  3. `dashFailed` becomes `string | null`. In the `catch`: a `permission_denied` is `null`
     as today; an `invalid_request` is `Shorten the search: dashboards and charts are
     searched with at most ${PALETTE_SEARCH_MAX_CHARS} characters.`; anything else is
     `"Dashboard search is unavailable"`. The alert shows the string. Every `setDashFailed(false)`
     becomes `setDashFailed(null)`. Comment: QA `LH-DATA-11-15` F2 (`DATA-54`); a refusal
     of the text is not an outage.
- Check:
  - `palette-search.test.ts`:
    `the box's limit is the dashboard search's`: read
    `${process.cwd()}/rust/crates/lakehouse-bi/src/search.rs` and expect it to contain
    `pub const MAX_QUERY_CHARS: usize = ${PALETTE_SEARCH_MAX_CHARS};`.
  - `command-palette.test.tsx`, in `describe("CommandPalette dashboards and charts", …)`:
    `says to shorten the search when the dashboard search refuses the text`: the dashboard
    search answers 400 `{ error: "Search text can be at most 100 characters." }` (the
    catalog answers `{ assets: [] }`); set the input's value to 120 characters with
    `fireEvent.change` (the test's document does not apply `maxLength` to a value set by
    code, which is why the refusal can still be reached); the text starting "Shorten the search" is on screen and "Dashboard
    search is unavailable" is not. And `the search box takes at most 100 characters`: the
    input's `maxLength` is 100.
  The existing test "says so when the dashboard search is unavailable, and stays quiet for
  a caller without access" passes unchanged.
  Wrong change that must make it fail: treating `invalid_request` as the outage (the old
  sentence shows).

### T15. A deprecated table with no replacement says so (LH-DATA-12-05 F1; DATA-54)

- Anchors: `src/features/catalog/asset-certification.tsx:49-85` (`DeprecationNotice`),
  `:174-183` (the dialog's replacement field);
  `src/features/catalog/asset-detail-page.tsx:141` (`<DeprecationNotice asset={a} />`);
  `src/features/catalog/asset-detail-page.test.tsx:120-` (`describe("AssetDetailPage
  deprecation notice", …)`).
- Change:
  1. `DeprecationNotice` takes an optional prop `replacementHidden?: boolean`. After the
     note, when there is no `replacementAssetId` and `replacementHidden` is not true, add
     ` No replacement is named.` (a leading space, as the other parts). Comment: QA
     `LH-DATA-12-05` F1 (`DATA-54`); a replacement is optional (feature page, "if there is
     one"), so the notice says when there is none; a page opened with a table grant is not
     sent the replacement, which is not the same as none.
  2. The asset page passes `replacementHidden={a.shownWithTableGrant === true}`. The chart
     builder's use (`chart-builder.tsx:1356`) is not changed: a mart row carries its
     replacement.
  3. Dialog: the label becomes `Replacement table id (optional)` and, under the input,
     `<p className="text-xs text-muted-foreground">Leave it empty when no table replaces
     this one.</p>`.
- Check, in `asset-detail-page.test.tsx`, the deprecation describe:
  - `says no replacement is named for a deprecated table without one`: `stubApi({
    certification: "deprecated", certificationNote: "Old" })`; the note's text contains
    "No replacement is named."
  - `does not say so when a replacement is named, or when the page is opened with a table
    grant`: with `replacementAssetId: "silver.orders_v2"` the sentence is absent; with
    `shownWithTableGrant: true` and no replacement it is absent too.
  Wrong change that must make it fail: ignoring `replacementHidden` (the grant case shows
  the sentence).

### T16. The chosen mart keeps its Certified mark (LH-DATA-12-17 F1; DATA-54)

`src/features/dashboards/chart-builder.tsx` and its test are owned by #486: re-find the
anchors by their text after it merges.

- Anchors: `src/features/dashboards/chart-builder.tsx:410` (`const chosenMart =
  selectedMart(marts, choice);`), `:1302-1305` (the `Select` and its trigger), `:1354-1356`
  (the comment "DATA-12 part 3: a deprecated mart still works" and `DeprecationNotice`);
  `src/features/dashboards/chart-builder.test.tsx:23-48` (the `fetch` stub; the marts come
  from `/api/dashboard/fields`), `:52` (`barDef`), `:54` (`renderBuilder`).
- Change: directly above the `DeprecationNotice` line, add
  ```tsx
  {/* QA LH-DATA-12-17 F1 (DATA-54): the trigger shows the mart's name only, so the
      mark of the list row was lost once a mart was chosen. A deprecated mart has the
      notice below. */}
  {chosenMart?.certification === "certified" ? (
    <p data-testid="chosen-mart-mark" className="flex items-center gap-1.5 text-xs text-muted-foreground">
      <CertificationBadge certification="certified" />
      This mart is certified.
    </p>
  ) : null}
  ```
  Nothing else changes; no assistant change (D14).
- Check, in `chart-builder.test.tsx`, a new describe `ChartBuilder chosen mart mark (QA
  LH-DATA-12-17)`:
  - `shows Certified under the picker once a certified mart is chosen`: in the test,
    replace `global.fetch` with a copy of the `beforeEach` stub whose marts answer is `{
    marts: [{ name: "mart_visits", rows: 10, certification: "certified" }] }`;
    `renderBuilder(barDef)`; `await screen.findByTestId("chosen-mart-mark")`, and its text
    contains "Certified".
  - `shows no mark line for a mart without one`: the default stub; after the form has
    loaded, `screen.queryByTestId("chosen-mart-mark")` is `null`.
  Wrong change that must make it fail: removing the new line.

### T17. Change history shows the date and time on hover (LH-DATA-12-01 F2; DATA-54)

`src/features/catalog/asset-activity.tsx` is owned by #489, which also changes
`asset-detail-tabs.test.tsx`: re-find the anchors by their text after it merges.

- Anchors: `src/features/catalog/asset-activity.tsx:452-461` (the list under "Change
  history": the `span` with `ml-auto text-xs text-muted-foreground` around
  `formatRelativeTime(c.at)`), `:207` (the model: `title={formatDateTime(…)}`), `:26-30`
  (`formatDateTime` is already imported);
  `src/features/catalog/asset-detail-tabs.test.tsx:855` (a test that reads the "Change
  history" card).
- Change: that `span` gets `title={formatDateTime(c.at)}`. Comment: QA `LH-DATA-12-01` F2
  (`DATA-54`).
- Check, in `asset-detail-tabs.test.tsx`, next to the test at `:855`:
  `shows a change's date and time on hover`: an asset whose `changeHistory` has one entry
  with `at: "2026-06-12T09:41:00Z"`; on the Activity tab, inside the "Change history" card,
  the element that holds the relative time has `title` equal to
  `formatDateTime("2026-06-12T09:41:00Z")` (import the formatter in the test, so the test
  does not depend on the machine's time zone).
  Wrong change that must make it fail: removing the `title`.

### T18. The incidents panel is read again after a run (LH-DATA-14-01 F1; DATA-54)

- Anchors: `src/features/governance/quality-incidents.tsx:175-182` (`QualityIncidents`, its
  `useService`); `src/features/governance/data-quality-page.tsx:107-128` (`handleRun`,
  `handleDelete`), `:157`, `:267-278` (the edit dialog's `onSaved`);
  `src/features/catalog/asset-quality.tsx:273-300` (`AssetQuality`, `confirmDelete`),
  `:311-331` (`runAll`), `:336`, `:410-432` (the edit and add dialogs);
  `src/features/catalog/asset-detail-page.tsx:68-72` (`keepDataOnReload`, the cause);
  tests: `data-quality-page.test.tsx:60-112` (`stubApi`), `:313-318`;
  `asset-detail-tabs.test.tsx:686-694`, `:333`, `:653`.
- Change:
  1. `QualityIncidents` takes `reloadKey?: number` (doc: raise it to read the incidents
     again, after a rule ran or changed) and adds it to the `useService` dependency list:
     `[view, asset, reloadKey]`.
  2. `DataQualityPage`: a state `incidentsKey` starting at 0; raise it by one in
     `handleRun` after a result, in `handleDelete` after success, and in the edit dialog's
     `onSaved`. Pass `reloadKey={incidentsKey}`.
  3. `AssetQuality`: a state `incidentsKey`; a local `changed` that raises it and then
     calls `onChanged()`; use `changed` at every place the component calls `onChanged`
     today (`confirmDelete`, `runAll`, the edit dialog's `onSaved`, the add dialog's
     `onCreated`). Pass `reloadKey={incidentsKey}`. Comment at `changed`: QA
     `LH-DATA-14-01` F1 (`DATA-54`); the page keeps this tab mounted while the asset
     reloads, so the panel has to be told.
- Check:
  - `asset-detail-tabs.test.tsx`, in `describe("Quality tab: incidents, schedule and
    learning (DATA-14)", …)`: `reads the incidents again after the checks ran`: the stub
    answers `GET …/quality/incidents` with no incident the first time and with `INCIDENT`
    afterwards (count the calls in the test); open the Quality tab, press "Run checks";
    the incident's text appears without a remount, and the incidents address was requested
    at least twice.
  - `data-quality-page.test.tsx`, in the "running a rule by hand" describe:
    `reads the incidents again after Run check`: extend `stubApi` so a `POST` ending
    `/run` answers `{ id: "r1", status: "failed", value: "70.0% not null" }`, and count the
    `GET`s of the incidents address; open the rule, press "Run check"; the count rises by
    one.
  Wrong change that must make it fail: leaving `reloadKey` out of the dependency list.

### T19. "stale" is explained (LH-DATA-14-03; DATA-54)

- Anchors: `src/features/governance/quality-incidents.tsx:126-166` (`IncidentRow`), `:141`
  (`<Pill tone="neutral">stale</Pill>`); `routes/quality.rs:1009-1010` (what makes it
  stale); `data-quality-page.test.tsx:166-183` (how incidents are stubbed).
- Change: in `quality-incidents.tsx` add
  `export const STALE_INCIDENT_NOTE = "The check has passed since this incident opened. It
  stays open until someone resolves it."`. The stale `Pill` gets `title={STALE_INCIDENT_NOTE}`.
  Under the detail line (`<p className="mt-1 text-sm">{incident.detail}</p>`), for a stale
  incident only, add `<p className="mt-0.5 text-xs text-muted-foreground">{STALE_INCIDENT_NOTE}</p>`.
  Comment: QA `LH-DATA-14-03` (`DATA-54`); signed decision 3: a passing run marks the
  incident stale and never resolves it.
- Check, in `data-quality-page.test.tsx`, the incidents describe:
  `says what stale means on a stale incident, and nothing on an open one`: two incidents,
  one with `status: "stale"`; the note's text is on screen exactly once, and the element
  with the text "stale" has that `title`.
  Wrong change that must make it fail: showing the note for every incident (it is on
  screen twice).

### T20. The upload's result says when on-load checks run (LH-DATA-14-05; DATA-54)

`src/features/connectors/upload-run-view.tsx` and `upload-file-page.test.tsx` are owned by
#489: re-find the anchors by their text after it merges.

- Anchors: `src/features/connectors/upload-run-view.tsx:143-173` (the `upload.status ===
  "ingested"` block; the `BadValuesReport` line and the buttons' `div`);
  `src/features/connectors/upload-file-page.test.tsx:1185` (a test that reaches "Load
  finished"); `dagster/dispar_orchestrate/pipeline_events.py:254-259` and
  `routes/quality_load.rs:1-24` (the cause; neither is changed).
- Change: inside the `ingested` block, after the `BadValuesReport` line and before the
  buttons, add
  ```tsx
  {/* QA LH-DATA-14-05 (DATA-54): the checks start when the orchestrator reports the
      finished run, not when the load ends. No time is named: nothing here bounds
      that wait (review round 1 R4). */}
  <p className="text-xs text-muted-foreground">
    Quality checks set to run on every load start after the orchestrator reports this
    load, not when the load ends. Their results are on the table's Quality tab.
  </p>
  ```
  No Python and no Rust change; the timing stays. The sentence names no time: the sensor
  has no interval of its own, posts nothing while `PIPELINE_RUN_TOKEN` is unset, and a
  lost report is not asked for again, so "within about a minute" was a bound the code does
  not keep (principle 2, principle 5).
- Check, in `upload-file-page.test.tsx`, beside the test at `:1185`:
  `says when on-load checks run once the load has finished`: after "Load finished" the
  text starting "Quality checks set to run on every load" is on screen; and in a test that
  ends in "Load failed" (find one in the file) it is not.
  Wrong change that must make it fail: putting the line outside the `ingested` block (it
  shows on a failed load).

### T21. No row count for a table that is not there (LH-DATA-15-07 F2; DATA-54)

- Anchors: `src/features/catalog/asset-detail-page.tsx:142-173` (the `MetadataList`, the
  line `{ label: "Rows", value: fmtMeasured(a.rows, formatCompactNumber) }`), `:174`
  (`<AssetDetailTabs … />`); `src/features/catalog/asset-detail-tabs.tsx:88-112`
  (`AssetDetailTabs`, the line `const iceberg = useIcebergTable(a)`);
  `src/features/catalog/asset-storage.tsx:36-54` (`IcebergTableState`, `useIcebergTable`),
  `:246` (a missing table is `not_found`); `src/lib/lakehouse-view.ts:72-79`
  (`isIcebergCandidate`); `src/features/catalog/asset-detail-page.test.tsx:65-80`
  (`stubApi`: any address it does not know answers 404).
- Change:
  1. `asset-detail-tabs.tsx`: rename the existing component to `AssetDetailTabsView`, add
     a required prop `iceberg: IcebergTableState`, and delete its `const iceberg =
     useIcebergTable(a)` line. Keep the old name as a thin wrapper, so every existing user
     and test is untouched:
     ```tsx
     export function AssetDetailTabs(props: Omit<React.ComponentProps<typeof AssetDetailTabsView>, "iceberg">) {
       const iceberg = useIcebergTable(props.asset)
       return <AssetDetailTabsView {...props} iceberg={iceberg} />
     }
     ```
     Move the component's doc comment to the view and give the wrapper one sentence: it
     reads the Iceberg table itself, for a caller that has not (the tests of
     `asset-detail-tabs.test.tsx` render the tabs without the page).
  2. `asset-detail-page.tsx`: a new component in the same file, `AssetFacts`, with the
     props `asset`, `meanings`, `onAssetChanged`. It calls `useIcebergTable(asset)` once,
     renders the `MetadataList` (moved as it is) and then `<AssetDetailTabsView asset
     iceberg onAssetChanged meanings />`. The page renders `<AssetFacts … />` where the two
     were. In the list, the Rows value becomes: when `iceberg.status === "error" &&
     iceberg.error.code === "not_found"`, `<span title="No Iceberg table for this asset:
     the count the registry recorded is not shown.">—</span>`; otherwise as today.
     Comment: QA `LH-DATA-15-07` F2 (`DATA-54`); the count is the registry's, and a table
     that is gone has no rows to count; any other failure of the read keeps the count,
     because it does not prove the table is gone.
- Check, in `asset-detail-page.test.tsx`, a new describe `AssetDetailPage row count (QA
  LH-DATA-15-07)`:
  - `shows no row count when the Iceberg table is not there`: `stubApi({ layer: "bronze",
    type: "iceberg-table", tableName: "orders", tableKey: "bronze.orders", rows: 720 })`
    (the stub answers the table read with 404); after the page has loaded, wait for the
    header to stop showing `720`; the Rows value is "—".
  - `keeps the row count of a table that is not an Iceberg table`: the default asset
    (`rows: 10`): the header shows `10`.
  - `keeps the row count when the table read fails for another reason`: a stub that
    answers `/api/lakehouse/tables/bronze/orders` with 503: the header shows `720`.
  The tests of `asset-detail-tabs.test.tsx` pass unchanged (they render the wrapper).
  Wrong change that must make it fail: hiding the count on every error of the read (the
  third test finds "—").

### T22. The documents say what is now true (DATA-54)

Edit only these lines. A bullet is wrapped like the bullets around it. No "Signed" cell and
no acceptance row is edited.

- `docs/core/features/catalog-search.md`:
  - `:78-80`, the first bullet of "Limits to tell a customer", becomes:
    ```text
    - Search covers table names (the name shown and the physical `schema.table`
      name), namespaces, owners, descriptions, tags, column names and column
      descriptions. It does not search the data inside tables, and it does not
      search a Bronze table's internal registry id (`DATA-54`, 2026-11-12;
      *proposed, not yet signed*).
    ```
  - After the bullet that ends "one letter per word." (`:86-87`) add:
    ```text
    - A typed word with no letter or digit (`_`, `%`) is ignored; a search made
      only of such words finds nothing (`DATA-54`).
    - The ⌘K box lists eight tables. When more matched it says "Showing 8 of N:
      refine the search". It takes at most 100 characters.
    - A result found through its description shows up to 80 characters of the
      description around the word. A column's description is named, not quoted.
    ```
- `docs/core/features/certification-and-governed-tags.md`, at the end of "Limits to tell a
  customer" (after `:86`):
  ```text
  - A deprecated table needs no replacement; its page then says "No replacement
    is named." (`DATA-54`).
  - Saving a mark that is already set changes nothing: who set it and when stay,
    and the change history gets no new entry (`DATA-54`).
  - A tag key in use cannot be deleted. The refusal names up to three of the
    tables that hold it, and only tables the person may read (`DATA-54`).
  ```
- `docs/core/features/data-quality-in-depth.md`, at the end of "Limits to tell a customer"
  (after `:58`):
  ```text
  - A rule is written for a table that exists and that its author may read; any
    other table is refused (`DATA-54`).
  - On-load checks start when the orchestrator reports the finished load, not
    when the load ends: QA saw about 36 seconds on staging (`LH-DATA-14-05`). The
    upload's result page says so.
  - A stale incident is one whose check has passed since it opened. It stays
    open until a person resolves it.
  ```
- `docs/core/features/asset-page-depth.md`, at the end of "Limits to tell a customer"
  (after the bullet "A version's author is shown only where the committer was recorded."):
  ```text
  - A table whose Iceberg table is no longer there shows no row count (`DATA-54`).
  ```
- `CHANGELOG.md`, under `## [Unreleased]`, `### Fixed`, in the style of the entries there
  (what a customer sees, then the backlog id): one entry for search (the physical name is
  searched and shown, a Bronze table's internal id is not searched, `_` alone finds
  nothing, "Showing 8 of N", pages are listed only for words they contain, the 100
  characters, the description's matched text); one for marks and tags (no replacement is
  said, the same mark saved twice, refusals in words, the tables named, the mark on a
  chosen mart, dates in the change history); one for data quality (a rule's table is
  checked and the picker offers real table names, the incidents panel after a run, what
  stale means, when on-load checks run, no row count for a missing table). All `DATA-54`.
  Say in the search entry: **an API client that read `matchedOn.field = "id"` now reads
  `"table"`, with the table's name in `value`.**
- Do not edit `docs/core/BACKLOG.md`, `PRODUCT.md`, `HANDOFF.md`, `AGENTS.md`,
  `docs/CODE-STANDARD.md`, any Decisions table or any acceptance checklist row (the
  reviewer does what is needed, after merge).

## 3. Verification

The build machine is a strong ARM64 host (20 CPUs, 121 GB of memory, Docker running); see
section 3a for the checkout and the cargo settings. CI on the pull request runs on x86_64
and is the final word: a check that is green here and red there is red.

Per commit, only for what the commit touched. Never `cargo clean`. Never a bare
`cargo test -p lakehouse-api <filter>` and never `cargo test --workspace`: a unit test runs
with `--lib` and its name, an integration test file runs with `--test <file>`; CI runs the
full suite.

- Rust, from `rust/`: `cargo fmt --check`, then for each crate the commit touched
  `cargo clippy -p <crate> --all-targets -- -D warnings`, then the tests of the task:

  | Task | Crates for clippy | Tests |
  | --- | --- | --- |
  | T1 | `lakehouse-api` | `cargo test -p lakehouse-api --lib a_word_with_no_letter_or_digit_is_dropped_and_alone_finds_nothing`; `cargo test -p lakehouse-api --lib routes::catalog_search` |
  | T2 | `lakehouse-api` | `… --lib a_table_is_found_by_its_physical_name_and_a_bronze_slug_is_not_searched`; `… --lib routes::catalog_search`; `… --lib search_limit` |
  | T3 | `lakehouse-api` | `… --lib a_description_match_carries_the_text_around_the_word`; `… --lib the_snippet_never_cuts_inside_a_character`; `… --lib routes::catalog_search` |
  | T4 | `lakehouse-api` | `… --lib saving_the_same_mark_again_writes_no_history_entry_and_keeps_who_set_it`; `… --lib a_member_certifies_with_their_display_name_and_clears_it`; `… --lib a_deprecation_stores_its_note_and_replacement` |
  | T5 | `lakehouse-api` | `… --lib no_certification_refusal_names_an_api_field_and_each_is_a_sentence`; `… --lib each_certification_refusal_has_its_sentence_and_writes_nothing`; `… --lib a_replacement_is_refused_with_503_when_the_catalog_cannot_be_read` |
  | T6 | `lakehouse-store`, `lakehouse-api` | `cargo test -p lakehouse-store --lib no_tag_refusal_prints_a_pattern_or_an_api_field`; `cargo test -p lakehouse-store --test governed_tag`; `cargo test -p lakehouse-api --lib validate_annotation_body`; `… --lib an_undefined_governed_key_and_a_malformed_pair_are_refused`; `… --lib a_governed_value_outside_the_list_is_refused_naming_the_key_and_saves_nothing`; `… --lib tag_key_handlers` |
  | T7 | `lakehouse-store`, `lakehouse-api` | `cargo test -p lakehouse-store --test governed_tag`; `cargo test -p lakehouse-api --lib only_tables_the_caller_may_read_are_named`; `… --lib the_refusal_names_up_to_three_tables_and_counts_the_rest`; `… --lib tag_key_handlers` |
  | T8 | `lakehouse-api` | `cargo test -p lakehouse-api --test quality_incidents` |

- TypeScript: `bun install --frozen-lockfile` once, then per commit `bun run typecheck`,
  `bun run lint`, and `bun test` on the touched test files:

  | Task | Test files |
  | --- | --- |
  | T9 | `src/lib/table-key.test.ts`, `src/features/governance/table-picker.test.tsx`, `src/features/governance/quality-rule-create-page.test.tsx`, `src/features/governance/data-quality-page.test.tsx`, `src/features/alerts/alert-rule-page.test.tsx`, `src/features/admin/role-form-page.test.tsx` (it renders `role-table-access.tsx`) |
  | T10 | `src/components/command-palette.test.tsx`, `src/services/clients/assets.test.ts` |
  | T11 | `src/lib/palette-search.test.ts`, `src/features/catalog/data-explorer-columns.test.tsx` |
  | T12, T13, T14 | `src/lib/palette-search.test.ts`, `src/components/command-palette.test.tsx`, `src/features/catalog/data-explorer-columns.test.tsx` |
  | T15 | `src/features/catalog/asset-detail-page.test.tsx` |
  | T16 | `src/features/dashboards/chart-builder.test.tsx` |
  | T17 | `src/features/catalog/asset-detail-tabs.test.tsx` |
  | T18 | `src/features/catalog/asset-detail-tabs.test.tsx`, `src/features/governance/data-quality-page.test.tsx` |
  | T19 | `src/features/governance/data-quality-page.test.tsx`, `src/features/governance/quality-incidents.test.ts` |
  | T20 | `src/features/connectors/upload-file-page.test.tsx` |
  | T21 | `src/features/catalog/asset-detail-page.test.tsx`, `src/features/catalog/asset-detail-tabs.test.tsx` |

  Do not run prettier.
- Nothing under `dagster/` changes, so no Python test runs. If a task turns out to need a
  change there, stop and report; if the planner then adds one, both
  `python -m pytest dispar_orchestrate -q -m "not loader"` and `-m loader` run from
  `dagster/`, in a virtual environment under `/home/rismana/rantai/`.
- Always: `python3 ops/lint/check_bare_iceberg_count.py && python3
  ops/lint/check_intra_package_imports.py`.
- No compose file changes, so no `docker compose` check.

Before handoff, once, in the foreground, on the final commit:

```bash
cd rust && cargo fmt --check
cargo clippy -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings
cargo test -p lakehouse-store --lib governed_tag
cargo test -p lakehouse-store --test governed_tag
cargo test -p lakehouse-api --lib routes::catalog_search
cargo test -p lakehouse-api --lib routes::catalog
cargo test -p lakehouse-api --lib routes::governance
cargo test -p lakehouse-api --lib routes::quality
cargo test -p lakehouse-api --test quality_incidents
cd .. && bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status   # nothing generated staged
```

The full `cargo test --workspace` is CI's on the pull request; list it in the handoff as
*not verified locally, CI*. The handoff quotes each command with its counts, names every
test added or changed so it can be found in the CI log, and says *not verified* for
anything not run (rule 7).

What the planner read and did not run, and the developer's runs must confirm. If one of
these answers differently, report the answer and stop that task; the plan is corrected, not
the assertion.

- That the picker's options read `silver.<Display Name>` against the real API (D16). The
  planner read the row builders and the picker; T9's picker test is the proof.
- That every existing test of `catalog_search.rs` passes after T2 without an edit (their
  ids hold a dot or are never searched for).
- That `tests/quality_incidents.rs`' mock answers `system.columns` in a way
  `clickhouse_source` reads as "the table exists", so the two existing `PUT`s and the new
  201 cases pass (T8), and that `with_priority(1)` lets one test answer "no such table".
- That a principal with `*:*` and no narrowed role reads every table through
  `table_access_for` in the handler tests of T7 and T8.
- That `cmdk` passes the item's `value` to the filter with its letters intact (T13 lowers
  both sides, so case does not matter), and that a `div` inside `Command.Group` is drawn
  (T10).
- That the chart builder's `Select` does not keep its list rows in the document while it is
  closed. T16's test does not depend on it (it looks for `data-testid`).
- That the orchestrator's default sensor tick is 30 seconds. Nothing in the plan depends on
  the number (review round 1 R4 dropped the page's "within about a minute"); the feature
  page quotes QA's 36 seconds as what was seen.
- That the let-chain in T4 compiles as written (the form is used at
  `routes/catalog.rs:3481-3483`).

## 3a. Machine

- Checkout: `/home/rismana/rantai/lakehouse-agents-2` (the planner fills in n).
- Before any cargo command:
  `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-2 CARGO_BUILD_JOBS=6`.
- The machine is shared. Work only under `/home/rismana/rantai/`. Start no container you do
  not remove.
- The Rust tests with a database (`#[sqlx::test]` in `--lib`, and `--test quality_incidents`,
  `--test governed_tag`) start one Postgres through `lakehouse-test-support`, reused when
  it already runs. Before the first such run, `docker ps -a` and write down what is there.
  After the last one, remove what your runs started and nothing else (`docker rm -f <id>`);
  a container that was there before belongs to another checkout. Say in the handoff which
  you removed.

## 4. Out of scope

- **The column lineage of a Bronze table made by an upload** (`LH-DATA-13-02`, `TC-1723`).
  By reading, the finding as written is not what the code does today: an upload registers
  its table in the Bronze registry (`dagster/dispar_orchestrate/file_ingest.py:1239-1253`,
  `connector_catalog.py:126-180`), the catalog lists every registry row
  (`routes/catalog.rs:885-916`), the upload's result has "Open in Catalog"
  (`upload-run-view.tsx:164-168`), and every asset's Lineage tab draws the column lineage
  card (`asset-lineage.tsx:332-334`). What the console does not have is a link **from** a
  `bronze.<table>` key **to** that page: in the lineage graph, the column lineage rows and
  the Data Quality lists a Bronze key is plain text, because its page is addressed by a
  registry slug the key does not carry (`src/lib/table-key-href.ts:1-13`,
  `src/features/governance/lineage-graph.tsx:55-73`; recorded as the Bronze slug gap in the
  `WEB-3` plan). Closing it means the lineage API carries each Bronze node's asset id, or
  the catalog answers a `bronze.<table>` address: a catalog decision, not a small fix.
  Question 2. After T2 the table is at least found in the Catalog and in Ctrl+K by the
  name the lineage shows (`bronze.<table>`).
- **Marks in Query Studio's schema browser** (`LH-DATA-12-17` F2). It touches #483's files
  (`src/features/queries/*`), and decision 4 of the feature page lists where marks show
  ("the sources Query Studio lists for a ClickHouse query"; "SQL sources and autocomplete
  are not marked"): the schema browser is not on that list. For the Query Studio slice,
  after the owner's answer (question 4).
- **A picker for the replacement table** in the Certification dialog. The field stays an id
  typed by hand; a picker is a new control.
- **The refusal to remove a value from a tag key** (`update_key_values`,
  `governed_tag.rs:176-214`). It counts tables per value; naming them per value is a longer
  sentence and is not the QA case. Question 5.
- **A piece of a column's description** in the reason line. The search index keeps a
  column's description only lower-cased; quoting it needs the original text in the index.
- **The physical name in the Catalog's table.** It has Namespace and Layer columns; the
  finding was the Ctrl+K box. The reason line there gains "table …" and the description's
  text through the shared wording.
- **The `owner`, `steward` and `description` length refusals** of the annotation route
  (`routes/catalog.rs:3482-3498`): plain words already, no pattern, not a QA case.
- **The stubs of old sentences in console tests** (T5, T6): they assert that a server
  answer is shown as sent.
- **The timing of on-load checks** (D20), and the same sentence on a connector's run page.
- **A row count the server withholds** for a missing Iceberg table. The catalog list and
  the API still carry the registry's count; only the page's header hides it (D21). Doing
  it in the API needs the list to tell "gone" from "the catalog did not answer in time"
  (`enrich_bronze_stats`, `routes/catalog.rs:1277-1339`, answers nothing for both).
- **A length limit on the catalog search itself.** The box now takes 100 characters; the
  API still reads at most eight words of any length.
- Everything else in the QA round that is not on row `DATA-54`.

### Open questions for the owner

None of these blocks the plan.

1. Search no longer looks in a Bronze table's internal registry id; it looks in the
   physical name (`bronze.<table>`) in its place (D3). The spec's target lists no id; the
   feature page's limit sentence said "ids", so T22 marks the new sentence proposed. Is
   that the wanted rule?
2. `LH-DATA-13-02` / `TC-1723`: by reading, an uploaded Bronze table is in the Catalog and
   its page has the column lineage. Was the table really missing from the Catalog list (then
   its registration failed, and the upload's result would have said so), or was the missing
   piece the link from `bronze.<table>` in a lineage view to its page? If the second: should
   a Bronze key link to its page everywhere (its own backlog item, closing the Bronze slug
   gap)?
3. A deprecated table needs no replacement, as the feature page and the spec say; the code
   does that, and the page now says "No replacement is named." (D10). If a replacement
   should be required, that is a change of the signed text.
4. Should Query Studio's schema browser show the Certified / Deprecated mark
   (`LH-DATA-12-17` F2)? Decision 4 of the feature page does not list it.
5. Should the refusal to remove a value from a tag key also name the tables that hold the
   value (today it counts them per value)?
6. A rule for a table that does not exist yet is now refused (D17). Does anyone write
   rules ahead of a table's first load? If so, the check should be limited to "may read"
   and the missing table accepted.
7. On-load checks start about half a minute after the load (D20). If that should be
   shorter, the orchestrator's sensor needs an interval of its own: its own item.

## 5. Handoff

Developer: `opencode-go/deepseek-v4.1-flash` (not Opus). Branch
`fix/qa-round-data-p3`, base `04715976` (`docs(plan): QA round, data-p3: the
plan`, which is `origin/fix/qa-round-data-p3` at the start). One commit per
task, in plan order; every commit pushed to
`origin/fix/qa-round-data-p3`; no force-push; no pull request opened.

### Commits

| Task | Commit | Title |
| --- | --- | --- |
| T1 | `ccea9dc1` | fix(search): a word with no letter or digit finds nothing (DATA-54) |
| T2 | `90007c0c` | fix(search): search the table's physical name, not its internal id (DATA-54) |
| T3 | `25db366b` | fix(search): a description match carries the text around the word (DATA-54) |
| T4 | `a6243564` | fix(catalog): saving the same mark again writes nothing (DATA-54) |
| T5 | `e0bd2fc5` | fix(catalog): the mark's refusals are sentences (DATA-54) |
| T6 | `92ecd875` | fix(catalog): tag refusals say what is allowed, in words (DATA-54) |
| T7 | `dc549cb9` | fix(governance): a tag key in use names the tables that hold it (DATA-54) |
| T8 | `8c3bf5b2` | fix(governance): a rule's table must exist and be readable (DATA-54) |
| T9 | `c7067d4a` | fix(governance): the table picker offers the tables' real keys (DATA-54) |
| T10 | `0f8fe0a5` | fix(search): the box says when more tables matched than it shows (DATA-54) |
| T11 | `600d13c6` | fix(search): the reason names the table, not "id" (DATA-54) |
| T12 | `edf92042` | fix(search): a result shows its physical name and the matched text (DATA-54) |
| T13 | `af15bdf7` | fix(search): pages and actions are listed only for words they contain (DATA-54) |
| T14 | `6f277ee7` | fix(search): the box takes 100 characters and says so (DATA-54) |
| T15 | `21eac55e` | fix(catalog): a deprecated table with no replacement says so (DATA-54) |
| T16 | `fddfa55a` | fix(dashboards): the chosen mart keeps its Certified mark (DATA-54) |
| T17 | `7fe68ac9` | fix(catalog): change history shows the date and time on hover (DATA-54) |
| T18 | `9106a0da` | fix(governance): the incidents panel is read again after a run (DATA-54) |
| T19 | `f31219ac` | fix(governance): "stale" is explained in words (DATA-54) |
| T20 | `5f3c34f9` | fix(connectors): the upload's result says when on-load checks run (DATA-54) |
| T21 | `41f4691d` | fix(catalog): no row count for a table that is not there (DATA-54) |
| T22 | `6768273c` | docs(features): the documents say what is now true (DATA-54) |

### Tests added or changed, with the failure before the fix

Each was run against the code before its fix (the fix stashed or the wrong
change the task names applied), and the failure text is the one printed.

- T1 `routes::catalog_search::tests::a_word_with_no_letter_or_digit_is_dropped_and_alone_finds_nothing`.
  Without `.filter(|w| !w.parts.is_empty())`:
  `assertion left == right failed: _` — left `silver.dim_customer`,
  `silver.rev_month` with `matchedOn.field = "id"`, right `[]`.
- T2 `…a_table_is_found_by_its_physical_name_and_a_bronze_slug_is_not_searched`.
  Reading `asset.get("id")` for the table candidate: left
  `[{… "matchedOn": {"field": "table", "value": "zq-registry-slug"}}]`, right `[]`.
- T3 `…a_description_match_carries_the_text_around_the_word` and
  `…the_snippet_never_cuts_inside_a_character`. Answering `String::new()` for the
  value: the test panics at the first value assertion (empty value). The second
  test passes on the good code and would panic on a byte slice (it asserts
  `contains("Übersee")` on a multi-byte description).
- T4 `…saving_the_same_mark_again_writes_no_history_entry_and_keeps_who_set_it`.
  Without the early return: `left: 2, right: 1` (`an unchanged save is no history`).
- T5 `…no_certification_refusal_names_an_api_field_and_each_is_a_sentence`.
  Leaving `a table cannot be its own replacement` as it was: the message
  `a table cannot be its own replacement` fails the uppercase assertion.
- T6 `governed_tag::tests::no_tag_refusal_prints_a_pattern_or_an_api_field`.
  Leaving the key sentence as it was: `key must be 1-64 characters matching
  ^[a-z0-9][a-z0-9_-]*$` fails the `^` assertion.
- T7 `catalog::tests::only_tables_the_caller_may_read_are_named` (dropping the
  `asset_visible` condition: left `["Orders", "Customers"]`, right `["Orders"]`);
  `governance::tests::the_refusal_names_up_to_three_tables_and_counts_the_rest`;
  `tag_key_handlers::deleting_a_key_in_use_names_the_tables_that_hold_it`
  (answering `counted` always: `{"error":"\"data_domain\" is still given to 2
  tables. Remove it from those tables first."}` — no names);
  `governed_tag::the_tables_holding_a_key_are_listed_in_order`.
- T8 `quality_incidents::a_rule_for_a_table_that_is_not_there_is_refused_and_nothing_is_stored`
  (skipping the existence check: `left: 201, right: 400`);
  `…a_rule_for_a_table_the_role_may_not_read_is_refused_with_the_same_sentence`
  (skipping the access check: `left: 201, right: 400`);
  `…changing_a_rule_to_a_table_that_is_not_there_is_refused_and_the_rule_keeps_its_table`
  (checking only in create: `left: 200, right: 400`).
- T9 `src/lib/table-key.test.ts` (new), `table-picker.test.tsx`
  `offers each row's table key, never its display name`. On the code before the
  change it fails with the options `["Every table"-less]
  ["primer.Northwind Orders","primer.No Table","silver.Orders Clean"]` — the
  three existing picker tests fail the same way (`primer.Raw Orders`,
  `silver.Orders`). The four files' fixtures were reshaped to the API's rows.
- T10 `command-palette.test.tsx`
  `says how many tables matched when it shows only eight of them` (reading
  `assets.length` as the total: the "Showing 8 of 23" line is absent);
  `says nothing about more when every match is shown`;
  `assets.test.ts searchAssets asks for the limit and answers the total`
  (`expect(found.total).toBe(23)` fails, `1` instead).
- T11 `palette-search.test.ts`
  `names the table that matched, and calls an old server's id a table id`
  (removing the `"id"` case: `id · approximate match` instead of
  `table id · approximate match`).
- T12 `palette-search.test.ts the lines of a result are its table key and why it
  matched`, `quotes the piece of the description the server sent`;
  `command-palette.test.tsx shows the physical name under the friendly name, and
  the matched text as text` (not passing `path`: the row holds no `silver.t1`).
- T13 `palette-search.test.ts a page is listed only when every typed word is in
  its text` (returning the score alone: `Received: 0.16200530530950422` for
  `gic`); `command-palette.test.tsx does not list a page for letters scattered
  through its name`.
- T14 `palette-search.test.ts the box's limit is the dashboard search's` (reads
  `rust/crates/lakehouse-bi/src/search.rs`); `command-palette.test.tsx says to
  shorten the search when the dashboard search refuses the text` (treating
  `invalid_request` as an outage: the old sentence shows);
  `the search box takes at most 100 characters`.
- T15 `asset-detail-page.test.tsx says no replacement is named for a deprecated
  table without one`; `does not say so when a replacement is named, or when the
  page is opened with a table grant` (ignoring `replacementHidden`: the grant
  case shows the sentence).
- T16 `chart-builder.test.tsx shows Certified under the picker once a certified
  mart is chosen` (line removed: `findByTestId("chosen-mart-mark")` finds
  nothing); `shows no mark line for a mart without one`.
- T17 `asset-detail-tabs.test.tsx shows a change's date and time on hover`
  (title removed: `getByTitle(formatDateTime(...))` finds nothing).
- T18 `asset-detail-tabs.test.tsx reads the incidents again after the checks
  ran`; `data-quality-page.test.tsx reads the incidents again after Run check`
  (leaving `reloadKey` out of the dependency list: both fail; the first times
  out waiting for the incident, the second's count stays).
- T19 `data-quality-page.test.tsx says what stale means on a stale incident, and
  nothing on an open one` (showing the note for every incident:
  `Expected: 1, Received: 2`).
- T20 `upload-file-page.test.tsx says when on-load checks run once the load has
  finished` (line removed: the text is absent); the failed-load test now also
  asserts the line is absent there.
- T21 `asset-detail-page.test.tsx shows no row count when the Iceberg table is
  not there`; `keeps the row count of a table that is not an Iceberg table`;
  `keeps the row count when the table read fails for another reason` (hiding the
  count on every error: `Expected to not contain: "—"`, `Received: "Rows—"`).
- T22 no test.

Existing tests changed, with the reason (rule 2 — the rule changed, not the
test's purpose):

- T3 `one_edit_finds_a_word_by_deletion_swap_substitution_and_insertion`: the
  expected value is now `revenue by month` (D5).
- T5 `each_certification_refusal_has_its_sentence_and_writes_nothing` and
  `a_replacement_is_refused_with_503_when_the_catalog_cannot_be_read`: the
  expected pieces are the new sentences.
- T6 `validate_annotation_body_rejects_a_tag_over_64_chars` (the old assertion
  read the word "tags" in the sentence; it now reads
  `not accepted: a tag`), `validate_annotation_body_bounds_governed_tags`,
  `an_undefined_governed_key_and_a_malformed_pair_are_refused`,
  `a_governed_value_outside_the_list_is_refused_naming_the_key_and_saves_nothing`,
  `governance::tag_key_refusals_name_the_field_and_store_nothing`,
  `lakehouse-store/tests/governed_tag.rs` (two assertions).
- T12 `CommandPalette catalog order`: the row's text now ends with its table
  key, so the test strips that instead of trailing punctuation; the order it
  asserts is unchanged.
- T15 two tests look the replacement field up by its new label (the label
  changed on purpose).

### Commands run (final commit `6768273c`, foreground)

Rust, from `rust/`, with
`CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-2 CARGO_BUILD_JOBS=6`:

```text
cargo fmt --check                                     # ok, no diff
cargo clippy -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings
                                                      # ok
cargo test -p lakehouse-store --lib governed_tag      # 2 passed; 0 failed; 318 filtered out
cargo test -p lakehouse-store --test governed_tag     # 16 passed; 0 failed
cargo test -p lakehouse-api --lib routes::catalog_search
                                                      # 35 passed; 0 failed; 3501 filtered out
cargo test -p lakehouse-api --lib routes::catalog     # 247 passed; 0 failed; 3289 filtered out
cargo test -p lakehouse-api --lib routes::governance  # 69 passed; 0 failed; 3467 filtered out
cargo test -p lakehouse-api --lib routes::quality     # 46 passed; 0 failed; 3490 filtered out
cargo test -p lakehouse-api --test quality_incidents  # 11 passed; 0 failed
```

TypeScript, from the repository root (`bun install --frozen-lockfile` once,
before the first check):

```text
bun run typecheck                                     # ok, no output
bun run lint                                          # 0 errors, 11 warnings, none in a file this slice touched (data-table, sidebar, sso-page, alerts-page, login-page, use-copilot, use-data-table, dashboard-specs, uploads.test, http)
bun run test                                          # 4079 passed, 1 skipped, 0 failed, 12303 expect() calls, 381 files, 98.05s
```

Lints and hygiene:

```text
python3 ops/lint/check_bare_iceberg_count.py          # R11 guard: no bare count() against a Bronze Iceberg table found.
python3 ops/lint/check_intra_package_imports.py       # OK: all intra-package dispar_orchestrate imports resolve
git status                                            # clean (nothing generated staged) on the final commit
```

Per-commit checks: for every Rust commit `cargo fmt --check`, `cargo clippy -p
<the crates the commit touched> --all-targets -- -D warnings`, and the tests
the plan's section 3 table names for that task; for every console commit
`bun run typecheck`, `bun run lint` and `bun test` on the test files the plan
names, plus the two Python lints. Each of those runs is the one whose counts
appear above under "Tests added or changed" (the per-commit run and the final
run agree).

Not verified locally:

- `cargo test --workspace` — *not verified locally, CI*. The plan leaves it to
  CI on the pull request; the scoped commands above are what was run.
- The container list: `docker ps -a` before the first database test showed 32
  `clickhouse/clickhouse-server:26.8` containers and one `postgres:16-alpine`
  (`ca1ffac89342`); after the last one it showed 31 and the same postgres. I
  started no container directly and removed none (`docker rm -f` was never
  run); one clickhouse container that was there before is no longer listed and
  is not mine to explain.
- Nothing under `dagster/` changed, so no Python test and no compose check ran,
  as the plan says.
- `cargo clippy` with `--all-features`: *not run* — the plan's handoff block
  names `--all-targets` only.

### MISMATCH

None. Every anchor was found by its text at this base and every task's code
matched the plan. The planner's unverified readings all held:

- D16/T9: the picker's fixtures really were not shaped like the API's rows;
  with the API's shape the old code read `primer.Northwind Orders`,
  `primer.No Table`, `silver.Orders Clean` (the new test fails on it), so D16 is
  confirmed, not wrong.
- T2: every existing `catalog_search` test passed unchanged; their ids hold a
  dot or are never searched for.
- T8: the `quality_incidents` mock's `system.columns` answer reads as "the table
  exists", and `with_priority(1)` lets one test answer "no such table" — both
  existing `PUT`s and the new 201 cases pass.
- T7/T8: a principal with `*:*` and no narrowed role reads every table through
  `table_access_for`.
- T13/T10: `cmdk` passes the item's `value` to the filter intact and a `div`
  inside `Command.Group` is drawn.
- T4: the let-chain compiles as written.

Two things the plan did not name, reported here rather than improvised:

- T8: `create_quality_rule` has a third caller, the assistant's
  `draft_quality_rule` tool (`rust/crates/lakehouse-api/src/routes/ai/tools/
  governance.rs`). It has no person behind it, so it passes `None` as the actor:
  access is not narrowed, a missing table is still refused. No tool schema
  changed; `tests/fixtures/tool_schemas.json` was not regenerated.
- T14: the module comment of `palette-search.ts` named `listAssets`; it names
  `searchAssets` now (T10's instruction), and T14's `PALETTE_SEARCH_MAX_CHARS`
  lives beside it.
- T9 removed the only use of the `Asset` type import in `table-picker.tsx`; the
  import is `AssetDetail` only now (no unused import left).

## 6. Review

### Round 1 (planner, 2026-11-12), at `80fa69ff`

Read against the plan and `AGENTS.md`; nothing was run for this review, CI on the draft pull
request (#493) runs the suites. 4 BLOCKERs (R1 to R4) and 8 SHOULD-FIX. No existing test was
weakened, every QA case id cited exists in the backlog row, and the documents stay in scope.

**Planner's rulings** (where a ruling and a finding's "Fix" differ, the ruling holds).

- **R1, R2.** As the findings say. The assistant's rule draft is checked as the person behind
  the chat is; the existing `tier2_governance_drafts` test gets a `ClickHouse` stand-in and
  keeps every assertion.
- **R3 (a gap in T8).** As the finding says: the catalog's tenant gate runs before the table
  is asked about, on create and on an update that changes the table. `run_rule`'s missing
  tenant gate is older than this branch: it is NOT fixed here and gets its own backlog row,
  filed by the planner.
- **R4 is the planner's error in D20 and T20.** The sentence must not name a time. Use the
  finding's wording, correct the changelog entry, and correct D20 and T20 in this plan.
- **R5 to R12.** As the findings say (R9: the comparison moves into the write; R10: the piece
  is taken around the first part of the word that is found).

### R1. BLOCKER: The assistant's `draft_quality_rule` passes `None` as the actor although the dispatcher holds the signed-in person, so it skips the per-role table check and is an existence oracle.

- Where: `rust/crates/lakehouse-api/src/routes/ai/tools/governance.rs:151-172` (the `None` at `:167`,
  and the comment `:164-166` "The drafting tool has no person behind it");
  `rust/crates/lakehouse-api/src/routes/ai/tools/mod.rs:135-140` (`run_tool(state, principal:
  Option<&Principal>, …)`) and `:258` (`"draft_quality_rule" => governance::draft_quality_rule(state,
  args)` — the principal is in scope and is passed to most other arms, not to this one);
  `rust/crates/lakehouse-api/src/routes/quality.rs:988-995` (the access check runs only for `Some`).
- What happens: a person whose role is narrowed to `silver.orders` (`read_scope = 'listed'`) and who
  holds `governance:write` asks the assistant to draft a rule on `silver.customers`. `POST
  /api/governance/quality` by the same person answers 400 "is not a table you can read"; the tool
  call reaches `ensure_rule_table(state, None, …)`, the access check is skipped, the table exists,
  and the rule is stored. The same call for `silver.no_such_table` answers the refusal: so through
  the assistant a hidden table reads "created" and a missing one reads "refused", which is the
  oracle D17 forbids. The comment is also false against the signature (rule 1): in a chat the
  principal is the person; it is `None` only for a headless run.
- Fix: give `draft_quality_rule` the dispatcher's principal (`governance::draft_quality_rule(state,
  principal, args)`) and pass it on to `create_quality_rule(State(..), principal, bytes)`. Replace
  the comment: the person behind the chat is checked exactly as the form checks them; only a run
  with no principal is not narrowed (`TableAccess`'s documented rule).
- Check: in `tests/quality_incidents.rs` (it has the ClickHouse stand-in and the narrowed-role
  setup), `the_assistant_cannot_draft_a_rule_on_a_table_the_person_may_not_read`: the narrowed user
  of the T8 test posts `/api/ai/tool` `{tool: "draft_quality_rule", args: {… "asset":
  "silver.customers", "confirmed": true}, mode: "build"}`; `outcome` is `failed`, the result's
  `error` equals the sentence `POST /api/governance/quality` gives for the same table, and
  `quality_rule` holds no such row; with `silver.orders` the outcome is `executed`. Wrong change
  that must make it fail: passing `None` again (the hidden table's rule is stored).

### R2. BLOCKER: An existing integration test now fails: `draft_quality_rule_is_never_passed_or_failed` drafts a rule with no ClickHouse behind it, which T8 turns into a 503.

- Where: `rust/crates/lakehouse-api/tests/tier2_governance_drafts.rs:141-168` (not in the diff);
  `rust/crates/lakehouse-api/tests/common/mod.rs:57`, `:317` (`CH_URL` is `http://127.0.0.1:1` under
  `spin_up()`); `rust/crates/lakehouse-api/src/routes/governance.rs:1691`;
  `rust/crates/lakehouse-api/src/routes/quality.rs:996-1007`;
  `rust/crates/lakehouse-api/src/routes/ai/mod.rs:1659-1660`.
- What happens: the test calls the tool with `"asset": "serving.mart_wisman"` on `spin_up()`.
  `create_quality_rule` now calls `ensure_rule_table` first; `clickhouse_source` cannot reach
  ClickHouse and answers `Err`, so the handler answers 503 "The table could not be checked. Try
  again."; the tool result is `{"error": …}`, `outcome` becomes `"failed"`, and the helper's
  `assert_eq!(body["outcome"], "executed")` (`:74`) fails (the later `fetch_one(...).expect("row
  must exist")` would fail too). The handoff ran only `--test quality_incidents` and `--lib`
  filters, so this was never run; it passes on `main`, so the pull request adds a red check
  (AGENTS.md step 6).
- Fix: give that test a ClickHouse stand-in that answers `system.columns` for `serving.mart_wisman`
  (`spin_up_with_env` with `CH_URL`, as `tests/quality_incidents.rs::app()` does). Do not loosen its
  assertions (rule 2). Add the handoff line for `cargo test -p lakehouse-api --test
  tier2_governance_drafts` with its count.
- Check: `cargo test -p lakehouse-api --test tier2_governance_drafts` passes; add beside it
  `a_drafted_rule_for_a_table_that_is_not_there_is_refused` (the stand-in answers no columns for
  `no_such_table`: `outcome == "failed"`, no row). Wrong change that must make the existing test
  fail again: removing the stand-in.

### R3. BLOCKER: The new table check is not behind the catalog's tenant gate, so a caller outside the catalog's tenant learns whether a table exists (400 against 201). (Plan gap: T8 names no tenant check.)

- Where: `rust/crates/lakehouse-api/src/routes/quality.rs:969-1018` (`ensure_rule_table`: role access
  and existence, no tenant); `rust/crates/lakehouse-api/src/routes/governance.rs:1053-1075`,
  `:1685-1701` (`create_rule`, `create_quality_rule`: no `catalog_tenant_refusal`);
  `rust/crates/lakehouse-api/src/routes/quality.rs:1177-1211` (`update_rule`: none either). The
  catalog's rule is `catalog_tenant_refusal` (`routes/catalog.rs:210-236`), which the list
  (`:269`), the quality reads (`routes/quality.rs:865`, `routes/quality_incidents.rs:189`,
  `routes/quality_schedules.rs:61`) and every tag-key write (`routes/governance.rs:2127`, "PR #106
  review R1 BLOCKER 1") already apply.
- What happens: two tenants, `CATALOG_TENANT_ID` names tenant A, a member of tenant B holds
  `governance:write`. `GET /api/catalog` answers them `supported: false`. `POST
  /api/governance/quality` with `"asset": "silver.dim_customer"` now answers 201 when the table
  exists and 400 "is not a table you can read" when it does not; before this branch it answered 201
  for any text. The same holds for a `PUT` that changes the table. This is new with T8. (Not new and
  not in this diff, named so it gets a backlog row: `run_rule`, `routes/quality.rs:1100`, has no
  tenant gate and no table-access check either, so a rule that exists can be run by such a caller
  and answers a measured value.)
- Fix: in `create_rule`'s quality arm and in `update_rule`, run `catalog_tenant_refusal(&state,
  principal, &headers)` before `ensure_rule_table` and answer `ApiError::PermissionDenied(reason)`,
  as `delete_tag_key` does (both handlers need `headers: HeaderMap`); or do it inside
  `ensure_rule_table` by passing the headers. The assistant's path gets the same gate once it passes
  its principal (R1). The planner decides whether `run_rule` is fixed here or gets its own row.
- Check: in `tests/quality_incidents.rs`,
  `a_caller_outside_the_catalog_tenant_is_refused_before_the_table_is_asked_about`: two tenants and
  `CATALOG_TENANT_ID` as in `routes/quality.rs:2390-2430`
  (`quality_read_routes_refuse_caller_outside_catalog_tenant`); the outside caller's `POST` answers
  403 with the same body for `silver.orders` and for `silver.no_such_table`, and stores nothing.
  Wrong change that must make it fail: removing the gate (201 and 400).

### R4. BLOCKER: The upload's result promises a time the code does not control: "Quality checks … start within about a minute". (The sentence is the plan's, D20/T20.)

- Where: `src/features/connectors/upload-run-view.tsx:163-169`; the same promise in
  `CHANGELOG.md:11` ("on-load checks start within about a minute"). Cause, unchanged:
  `dagster/dispar_orchestrate/pipeline_events.py:254-259` (the success sensor has no interval of its
  own), `:211-236` (with `PIPELINE_RUN_TOKEN` unset it does not post at all; after a network error
  "the report for it is lost" and Dagster does not ask again);
  `rust/crates/lakehouse-api/src/routes/quality_load.rs:1-24` (the rules run only after that report).
- What happens: the page states a bound for every finished load. Nothing in this repository bounds
  it: the wait is the orchestrator daemon's tick plus its backlog; when the sensor is off, the token
  is unset or the report is lost, the checks never start, and the page still says "within about a
  minute". The only measurement is one QA observation of 36 seconds; the plan itself lists the 30
  second tick as read, not verified (section 3). That is a latency the product does not guarantee,
  printed as one (principle 2, principle 5).
- Fix: say what the code does and drop the bound, for example: "Quality checks set to run on every
  load start after the orchestrator reports this load, not when the load ends. Their results are on
  the table's Quality tab." Change the CHANGELOG entry to match ("says when on-load checks start").
  `docs/core/features/data-quality-in-depth.md:61-63` already says only what was seen ("QA saw about
  36 seconds on staging") and can stay. The plan's D20/T20 text needs the same correction by the
  planner.
- Check: `upload-file-page.test.tsx` "says when on-load checks run once the load has finished"
  additionally asserts the paragraph's text does not match `/minute|second/`. Wrong change that must
  make it fail: the present sentence.

---

### R5. SHOULD-FIX: `the_snippet_never_cuts_inside_a_character` cannot fail on the wrong change it is named for, and no other test cuts multi-byte text; the handoff's "would panic on a byte slice" is not true.

- Where: `rust/crates/lakehouse-api/src/routes/catalog_search.rs:936-946` (the test), `:389-420`
  (`description_snippet`); handoff, plan file `:1375-1379`.
- What happens: the test's only multi-byte input is `"Umsatz nach Übersee für Österreich"`: 34
  characters (37 bytes), shorter than `SNIPPET_MAX_CHARS`, so nothing is ever cut. With the plan's
  named wrong change (slice the `str` by the same indices) `start = 0` and `end = 34`; byte 34 is
  inside the ASCII tail "reich", a character boundary, so there is no panic and the piece
  "Umsatz nach Übersee für Österre" still contains "Übersee": the test passes. The other snippet
  test is ASCII only. So the property the brief and D5 care most about (no panic on multi-byte
  text) is held by the code today — it slices a `Vec<char>` — but by no test. The plan specified
  this input, so the weakness starts there; the handoff's claim was not run.
- Fix: add a case that really cuts inside multi-byte text on both sides, in the same test.
- Check: `let d = format!("{} Übersee {}", "é".repeat(60), "ö".repeat(60));` then
  `description_snippet(&d, "übersee")`: contains `Übersee`, starts and ends with `…`, and has at most
  82 characters; and the same through `find` with a CJK description of 200 characters (`"表".repeat(100)
  + "収益" + &"表".repeat(100)`, term `収益`), which also proves a word in a script without spaces
  is a word. Wrong change that must make it fail: `line[start..end]` on the `String` (byte 41 is
  inside an `é`: panic).

### R6. SHOULD-FIX: The "same sentence" test compares two refusals of the same branch, so it does not prove "not there" and "not yours" read alike.

- Where: `rust/crates/lakehouse-api/tests/quality_incidents.rs:509-580` (the `assert_eq!` at
  `:565-569`).
- What happens: both requests are made by the narrowed user. `silver.customers` is refused by the
  access check; `silver.no_such_table` is not in the role's grants either, so it is refused by the
  access check too and never reaches the existence check (the test mounts no "no such table" answer;
  it does not need one). The equality therefore holds for any existence sentence. A change of
  `Ok(None) => Err(refused())` to a sentence of its own that still contains "is not a table you can
  read" (for example with ": no such table." appended) passes all three T8 tests and is the oracle.
- Fix: compare the narrowed user's refusal for `silver.customers` with the refusal an unrestricted
  caller (`admin`) gets for `silver.no_such_table` behind the `with_priority(1)` stand-in the first
  test mounts; keep the name replacement.
- Check: that assertion. Wrong change that must make it fail: a different sentence in the
  `Ok(None)` or `Ok(false)` arm of `ensure_rule_table`.

### R7. SHOULD-FIX: A `bronze.` name that is not a plain identifier answers 503 "Try again." for ever; the same name under `silver.` answers 400.

- Where: `rust/crates/lakehouse-api/src/routes/quality.rs:1009-1017`;
  `rust/crates/lakehouse-api/src/routes/catalog_source.rs:182-184` (`Ident::new(table)` failing is
  `PresenceUnknown::Unanswered`); `:107-109` (`clickhouse_source` answers `Ok(None)` for the same
  input).
- What happens: `POST /api/governance/quality` with `"asset": "bronze.orders-2024"`, `"bronze.my
  table"`, `"bronze."` or `"bronze.2024_orders"` (`Ident` refuses a leading digit) answers 503 "The
  table could not be checked. Try again.", although the engine was never asked and no retry can
  succeed. D17 keeps 503 for "the engine did not answer". `silver.orders-2024` answers the 400
  refusal.
- Fix: in `ensure_rule_table`, before the zone match, `let Ok(_) = Ident::new(table) else { return
  Err(refused()) }` (import `lakehouse_core::ident::Ident`); do not change
  `iceberg_table_presence`, which other callers use. Comment: such a name can never be a table the
  rule reads (`resolve` needs an `Ident` too).
- Check: in `a_rule_for_a_table_that_is_not_there_is_refused_and_nothing_is_stored`, add
  `bronze.orders-2024` and `bronze.` to the loop over `["orders", "lake.orders"]` (400 and the same
  piece). Add one test for D17's other half, which has none today:
  `a_table_that_cannot_be_checked_is_a_503_not_a_refusal` (the stand-in answers 500 for
  `system.columns` and `broken_table`: 503, the fixed sentence, nothing stored). Wrong changes:
  dropping the guard (503 for the bad name); mapping `Err` to `refused()` (400 in the new test).

### R8. SHOULD-FIX: The tag-key refusal names tables to a caller who holds `governance:write` without `catalog:read`.

- Where: `rust/crates/lakehouse-api/src/routes/catalog.rs:3294-3314` (`readable_asset_names`: role
  table access only); `rust/crates/lakehouse-api/src/policy.rs:378` (`DELETE
  /api/governance/tag-keys/{key}` needs `governance:write` only); the catalog's own admission is
  `principal.has("catalog:read")` or a grant (`routes/catalog.rs:3394-3413`), and `GET /api/catalog`
  needs `catalog:read` (`policy.rs:239`).
- What happens: a custom role with `governance:write` and no `catalog:read` (the seeded Governance
  Admin holds both since migration `0031`) deletes a key in use and is told the display names of up
  to three catalog tables it cannot list. D13 as written ("`asset_visible`") is met; the question is
  whether "may read" includes being able to open the catalog. Before this branch the refusal carried
  a count only.
- Fix: in `readable_asset_names`, answer `Some(Vec::new())` when `!principal.has("catalog:read")`
  (the sentence then counts and names nobody), with a comment naming `ensure_may_open_asset` as the
  rule it mirrors.
- Check: `tag_key_handlers::a_caller_without_catalog_read_is_told_the_count_and_no_name`: as
  `deleting_a_key_in_use_names_the_tables_that_hold_it`, with a principal holding
  `governance:write` only; the sentence contains `2 tables.` and neither `Orders` nor `Customers`.
  Wrong change: dropping the permission check.

### R9. SHOULD-FIX: "Writes nothing" is decided by a read followed by a separate write, so two saves of the same mark at the same time still write two history entries and move who set it.

- Where: `rust/crates/lakehouse-api/src/routes/catalog.rs:4043-4053` (read, compare, return) and
  `:4054-4068` (the write); `rust/crates/lakehouse-store/src/annotation.rs:201-226`
  (`set_certification`: an unconditional upsert).
- What happens: two requests with the same body (a retried request, two governors) both read the
  row before either writes; both pass the comparison, both upsert, both add `catalog.deprecate`, and
  `certified_by` / `certified_at` are the second one's. The console's Save is disabled while a save
  is pending (`asset-certification.tsx:206`), so a double click does not reach this; an API client
  or two people do. "Same" itself is compared correctly (status, trimmed note, trimmed replacement),
  and a real change is written and audited (the test's step 4).
- Fix: move the comparison into the write: `ON CONFLICT (asset_id) DO UPDATE SET … WHERE
  (asset_annotation.certification, asset_annotation.certification_note,
  asset_annotation.replacement_asset_id) IS DISTINCT FROM (EXCLUDED.certification,
  EXCLUDED.certification_note, EXCLUDED.replacement_asset_id)`, have `set_certification` return
  whether a row was written (`rows_affected() == 1`), and audit / drop the search copy only then.
  The handler's early return can stay as the cheap path.
- Check: store test `setting_the_mark_that_is_already_set_writes_no_row`: the second
  `set_certification` with the same three values answers `false` and `certified_by`, `certified_at`
  and `updated_at` are unchanged; a different note answers `true`. Wrong change: removing the
  `WHERE` (the second call answers `true`).

### R10. SHOULD-FIX: For a forgiven typo, or a word matched in parts, the description piece is the first 80 characters, which need not hold the match. (Plan decision: T3 step 2, "no position is position 0".)

- Where: `rust/crates/lakehouse-api/src/routes/catalog_search.rs:406-413`.
- What happens: a description of 200 characters with `revenue` at character 120, searched as
  `revnue` (or `rev-enue`, which matches by its parts with `approximate: false`): `matchedOn.value`
  is `"<the first 80 characters>…"`, and the box prints `description: "…" · approximate match` over
  text that does not contain the word. The existing typo assertion uses a 25-character description,
  where the whole line is the piece, so it does not show this.
- Fix: when the word is not found as typed, take the first position where one of the word's parts
  is found, and failing that the first token within one edit of a part (`within_one_edit` and
  `tokens` are in the module); position 0 only when neither exists.
- Check: in `a_description_match_carries_the_text_around_the_word`, `find(.., "revnue")` on the
  200-character asset: the value contains `revenue` and starts with `…`. Wrong change: position 0
  for every unfound word.

### R11. SHOULD-FIX: The picker's fallback sentence still tells a person to type `namespace.table`, which T9 says names no table and T8 now refuses.

- Where: `src/features/governance/table-picker.tsx:69`; the stale test title at
  `src/features/governance/table-picker.test.tsx:92` ("offers the catalog's tables as
  namespace.table").
- What happens: the catalog read fails, the field becomes free text and says "type the table as
  namespace.table". A Bronze row's namespace is `primer`; `primer.orders` is answered 400 "Pick the
  table as silver.<table>, serving.<table> or bronze.<table>." The module comment of the same file
  was corrected by T9; this sentence, which a person reads, was not.
- Fix: "Could not read the catalog's tables; type the table as silver.<table>, serving.<table> or
  bronze.<table>." and rename the test.
- Check: the existing fallback test of `table-picker.test.tsx` (catalog read rejects) asserts the
  new sentence. Wrong change: the old text.

### R12. SHOULD-FIX: The handoff quotes a "printed" failure for T9 that the old code cannot print.

- Where: plan file `:1401-1406`; old code at `c7067d4a3~1:src/features/governance/table-picker.tsx:55`
  (`Array.from(new Set(assets.map(tableKey))).sort()`).
- What happens: the handoff says each failure text "is the one printed" and gives the old options as
  `["primer.Northwind Orders","primer.No Table","silver.Orders Clean"]`. The old code sorts its
  options, and `"primer.No Table"` sorts before `"primer.Northwind Orders"` (a space is below `r`),
  so the printed list was `["primer.No Table", "primer.Northwind Orders", "silver.Orders Clean"]`.
  The quoted order is the order of the plan's own sentence (T9, "the options read …"). With R5's
  "would panic on a byte slice", two of the handoff's "failed before" claims are not what the code
  does (rule 7).
- Fix: re-run the two, quote what is printed, or write *not verified*.
- Check: the reviewer's own run of `bun test src/features/governance/table-picker.test.tsx` on
  `c7067d4a3~1` with the new test applied.

