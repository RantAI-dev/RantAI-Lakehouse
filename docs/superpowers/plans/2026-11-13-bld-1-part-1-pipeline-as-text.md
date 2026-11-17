# `BLD-1` part 1, last slice: the pipeline as text

Planner and reviewer: Claude Opus. Developer: a separate Opus developer agent
(new console UI, product owner's rule of 2026-11-04). The reviewer does not
write the code (`AGENTS.md`).

Base: `origin/main` at `2306e7302` (S1, S2 and S3 of part 1 are merged: #481,
#500, #499). Branch: `feat/bld-1-pipeline-as-text`. One PR slice. Console
only: no Rust, no migration, no API change.

Spec: `docs/core/specs/bld-1.md` ("The pipeline as text"). Feature page:
`docs/core/features/pipeline-joins-and-sql.md`, capability 8, decision 10,
acceptance `BLD-1-AC13`. The main plan of part 1 is
`docs/superpowers/plans/2026-11-12-bld-1-part-1-steps-and-sql.md`; its
decisions D1, D2, D6a and D17 still hold here.

**Not started before the product owner has looked at the editor of S3 in a
browser.** This slice adds a tab to that editor; a change the owner asks for
there comes first.

## 1. The problem

Decision 10 (product owner, 2026-11-12): a pipeline exists as one text file
that can be copied and used to make a pipeline, as a Snowflake, Databricks or
dbt pipeline does. Today the only text is the read view's "Stored
definition": the API's JSON, read-only, with a SQL step on one line of
escapes (`pipeline-definition.tsx:354`).

## 2. Decisions (planner)

- **X1. The format is YAML.** A SQL step is the reason: in YAML it is a
  block of plain lines, in JSON it is one line of `\n`. The console gains one
  dependency, the `yaml` package, used in one file (X3). No YAML parser is a
  dependency today (`package.json`). If CI's dependency check refuses it, the
  developer stops and reports; the format is not changed to get green.

- **X2. What the text holds.**

  ```yaml
  version: 1
  name: sales_by_country
  description: Revenue per country, paid orders only.
  inputs:
    - name: orders
      zone: bronze
      table: orders
    - name: customers
      zone: bronze
      table: customers
  steps:
    - name: paid
      kind: transform
      reads: orders
      transforms:
        - filter(status = 'paid')
    - name: by_country
      kind: sql
      sql: |
        SELECT c.country, sum(p.amount) AS revenue
        FROM paid AS p
        LEFT JOIN customers AS c ON p.customer_id = c.id
        GROUP BY c.country
  output:
    zone: silver
    table: sales_by_country
  schedule: "0 * * * *"
  ```

  The keys are in this order, always. `inputs` and `steps` are the API's own
  shape (D1), so the text and the request body cannot drift. A step of a
  kind the console does not know is written with the fields it was stored
  with and read back the same (S3 review R1-S2).
  Not in the text, and the tab says so: the connector, the two trigger
  switches, target lag, who the pipeline reads as, status. They belong to one
  installation; the text is what the pipeline computes.

- **X3. One pure module, `src/lib/pipeline-text.ts`.**
  `pipelineText(...)` writes the text from the editor's state
  (`EditorState`, `src/lib/pipeline-editor.ts:89`), the output settings
  (`OutputSettings`, `:348`), the name, the description and the schedule.
  `parsePipelineText(text)` returns either those values or a list of
  problems. Writing then reading gives the same values; reading then writing
  a text this module wrote gives the same text.
  A problem is fixed text written here with a path and, where the parser
  gives one, a line: `steps[1].sql: a SQL step needs "sql"`. Refused, each
  with its own sentence: text that is not YAML; `version` missing or not 1; a
  missing key; a value of the wrong type; a key the format does not have
  (an unknown key is refused, not ignored, so a typo is not lost silently).
  This module checks the shape only. D2's limits, the names a step may read
  and everything else stay with `editorProblems` (`:381`) and the API, which
  run after the text is applied.

- **X4. A third tab in the editor: Text.** After Build and Schedule and
  settings (`pipeline-editor.tsx:633`).
  - It shows the text of the editor's current state in a plain code editor
    (`@uiw/react-codemirror`, already a dependency, with no language
    extension; no YAML highlighting package is added).
  - **Copy** and **Download** (`<name>.pipeline.yml`).
  - Editing the text changes nothing until **Apply**. Apply reads the text
    (X3); with no problem it replaces the inputs, steps, output, schedule and
    description of the editor's state, and on Create the name too. A problem
    is listed under the editor with its path and line, and nothing is
    replaced. **Discard** puts the state's text back.
  - On Edit the name is fixed, as it is on the page: a text whose `name`
    differs from the pipeline's is refused with "a saved pipeline keeps its
    name".
  - Apply does not save. After it the page behaves as after any edit: the
    rail shows the new items, the page's rules flag what is wrong, D17a's
    sentence appears when the inputs or steps of someone else's pipeline
    changed.
  - While the text holds edits that are not applied, Save is not offered:
    the page's list of why it cannot save (`saveBlockers`, `:464`) gains
    "Apply or discard the text". What is saved is always what the other two
    tabs show.
  - With a connector, the rule of D12 stands: the page already flags a second
    input and a table that is not the connector's.

- **X5. Making a pipeline from a text** (`BLD-1-AC13`) is Create pipeline,
  the Text tab, paste, Apply, Save. No file upload in this slice.

- **X6. The read view shows the text.** The "Stored definition" panel of the
  pipeline page (`pipeline-definition.tsx:347`) shows the text form with a
  Copy button in place of the API's JSON.

- **X7. Nothing else changes.** No API route takes YAML; the assistant keeps
  writing `inputs` and `steps` through `create_pipeline`.

## 3. Anchors (`main` at `2306e7302`)

- `src/lib/pipeline-editor.ts`: `EditorState` `:89`, `editorStateFrom`
  `:110`, `OutputSettings` `:348`, `editorProblems` `:381`, `saveBlockers`
  `:464`, `definitionBody` `:491`, `definitionChanged` `:514`, `knownStep`
  `:71`.
- `src/features/pipelines/pipeline-editor.tsx`: `FormPage` `:588`, the tabs
  `:633` to `:796`.
- `src/features/pipelines/pipeline-definition.tsx`: "Stored definition"
  `:347`, its `CodeView` `:354`.
- `src/components/patterns/code-view.tsx:23` (`CodeView`),
  `src/components/sql-editor.tsx` (how CodeMirror is mounted).
- `package.json`: the CodeMirror packages; no YAML parser.

## 4. Tasks

One task per commit, in order; push after every commit; scoped checks per
commit. The reviewer opens a draft pull request after the first code commit.

- **T1. The dependency and the module (X1, X2, X3).** `bun add yaml`, the
  lockfile, `src/lib/pipeline-text.ts` and its tests.
  *Check:* unit tests: the text of a pipeline of one input and one form
  step; of X2's example, compared with the whole expected text; a SQL step
  with several lines, quotes, a `:` and a `#`, a trailing space and no final
  line break reads back character for character; a description of several
  lines; no steps; a step of an unknown kind kept as stored; write then read
  and read then write are stable. Problems, each with its path: not YAML
  (with the line); `version: 2`; no `inputs`; `steps` not a list; a step with
  no `name`; a `sql` step with no `sql`; an unknown top-level key; an unknown
  key inside an input.
- **T2. The Text tab (X4, X5).**
  *Check:* page tests: the tab shows the text of the state; an edit and
  Apply changes the rail and the Build pane; Discard restores; a text with a
  problem lists it and replaces nothing; Save is not offered while an edit is
  not applied and the page says why; on Edit a different `name` is refused;
  on Create the name is taken; Copy puts the text on the clipboard; D17a's
  sentence appears after applying changed steps to a pipeline that reads as
  someone else; a pipeline with a connector and a text of two inputs is
  flagged as the page flags it today.
- **T3. The read view (X6).**
  *Check:* the pipeline page's test: the panel shows the text form of a
  two-input pipeline with a SQL step, and Copy.
- **T4. Words.** The feature page's capability 8 names what the text holds
  and what it leaves out (X2); comments say why (principle 1).

Verification before handoff, on the head commit:

```bash
bun run typecheck && bun run lint
flock /home/shiro/rantai/.heavy.lock bun test src/lib/
flock /home/shiro/rantai/.heavy.lock bun test src/features/pipelines/
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status
```

The whole console suite runs in CI. How the tab looks in a browser is the
developer's to state: seen, in which theme, or *not verified*.

## 5. Out of scope

- A pipeline kept in a Git repository; a file upload; an API route that takes
  the text.
- The connector, triggers, target lag and who it reads as in the text.
- YAML highlighting in the editor.
- Parts 2 and 3 of `BLD-1`.

## 6. Handoff

*(the developer appends)*

## 7. Review

*(the reviewer appends findings, tagged `BLOCKER` or `SHOULD-FIX`)*
