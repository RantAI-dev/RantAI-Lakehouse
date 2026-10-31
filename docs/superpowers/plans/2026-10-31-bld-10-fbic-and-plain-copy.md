# BLD-10: the pipeline form is honest about FBIC and about a pipeline with no transform

Backlog: `BLD-10` (Task, Build, P3), from the QA report of 2026-10-23. The product owner said
"build it" on 2026-10-30. Base: `main` at `6711ad65`. Every `file:line` below was read there.

Planner: Claude Opus. Developer: a different agent (`AGENTS.md`). The developer does not change
a decision below. If the code disagrees with the plan, stop and report it.

## 0. What is wrong

- **FBIC does nothing, but the form offers it as working.**
  - The Transform step has a switch, "FBIC enrichment", with the subtitle "Enable
    feature-based incremental compute." (`src/features/pipelines/pipeline-create-page.tsx:761-769`).
  - Nothing runs it. The job logs `FBIC_UNSUPPORTED_REASON`
    (`dagster/dispar_orchestrate/authored_factory.py:259-263`, used at `:770-774`) and writes its
    output anyway.
- **Switching FBIC on lets a pipeline through with no transform, and nothing says so.**
  - `canProceed` for the Transform step is `transformDrafts.length > 0 || fbicEnabled` (`:293`).
  - With FBIC on and no transform, the job runs `SELECT * FROM source`
    (`authored_factory.py:506-551`): a plain copy. The form never says so.
- **The detail page uses a third name.** It shows "Change capture (FBIC)" On/Off
  (`pipeline-definition.tsx:268-269`). The form calls it "FBIC enrichment".

## 1. Decisions (made; do not change)

- **D1. FBIC reads "Not available".** The FBIC row on the Transform step stays visible, but its
  switch is disabled and off for a new pipeline.
  - Title: "FBIC enrichment".
  - Subtitle: "Not available yet: nothing runs it in this version."
  - The create request always sends `fbicEnabled: false`.
  - An edit sends the stored value unchanged (`def?.fbicEnabled ?? false`). There is no control
    to change it.
  - The Review row reads `FBIC: Not available`.
- **D2. A plain copy is allowed, but chosen on purpose.** A pipeline that copies its source as is
  is legitimate (for example, Bronze to Silver with no change). It must be a choice, not a side
  effect.
  - The Transform step gets a second switch row, "Copy the source as is", with the subtitle
    "No transform: each run copies the source table unchanged."
  - The switch is disabled while at least one transform is in the list. Adding a transform
    switches it off.
  - `canProceed` for step 1 becomes `transformDrafts.length > 0 || plainCopy`. FBIC no longer
    counts.
  - When an existing pipeline with no transforms is edited, the switch starts on.
  - Nothing new is stored. A plain copy is a pipeline whose `transforms` list is empty.
  - The Review row for Transforms reads `None: a plain copy of the source` in place of `—`.
- **D3. The detail page says the same.**
  - `pipeline-definition.tsx:268-269`: the label becomes "FBIC enrichment". The value is "Not
    available" when the stored value is off, and "Not available (stored as on; nothing runs
    it)" when it is on.
  - `pipeline-flow-layout.ts`: with zero transforms, the target node gets the sublabel
    `plain copy of the source`.
- **D4. No API, store, Dagster or migration change.**
  - The API keeps accepting an empty `transforms` list, which is right for a plain copy.
  - The assistant's create path already sends `fbic_enabled: false`
    (`rust/crates/lakehouse-api/src/routes/pipelines.rs:3391`).
  - The job's skipped-verb log stays.
  - This PR touches TypeScript only.
- **D5. Not a BI feature.** The assistant-parity rule does not apply.

## 2. Anchors (at `6711ad65`)

| What | Where |
| --- | --- |
| FBIC state and request | `pipeline-create-page.tsx:141` (state), `:249` (update), `:328` (create) |
| Step validity | `pipeline-create-page.tsx:293` |
| Empty-list hint | `pipeline-create-page.tsx:625` ("No transforms added yet.") |
| FBIC switch row | `pipeline-create-page.tsx:761-769` |
| Review rows | `pipeline-create-page.tsx:827-833` |
| Detail row | `src/features/pipelines/pipeline-definition.tsx:268-269` |
| Flow layout | `src/features/pipelines/pipeline-flow-layout.ts:166-184` |
| Tests | `pipeline-create-page.test.tsx` (fixture `:152`, flow `:229-235`, `:313`), `pipeline-flow-layout.test.ts:52`, `pipeline-detail-page.test.tsx:74-75` |

## 3. Tasks (one commit each, in order)

**T1. The form** (`pipeline-create-page.tsx`, its test).
- D1 and D2. Cite `BLD-10` at the two switch rows and at `canProceed`.
- Tests (full-sentence names):
  - the FBIC switch is disabled and reads "Not available yet";
  - with no transform, Next is disabled until "Copy the source as is" is switched on;
  - adding a transform switches the copy off and disables it;
  - the Review shows `None: a plain copy of the source`;
  - a create request carries `fbicEnabled: false`;
  - editing a pipeline stored with `fbicEnabled: true` sends `true` back.
- *Accept:* `bun run typecheck`, `bun run lint`, and
  `bun test src/features/pipelines/pipeline-create-page.test.tsx`.

**T2. The detail page** (`pipeline-definition.tsx`, `pipeline-flow-layout.ts`, their tests).
- D3. Add a flow-layout test for zero transforms: the target sits in column 1 and has the
  sublabel. Add a definition test for both FBIC values.
- *Accept:* `bun test src/features/pipelines`.

**T3. Docs.** In `docs/core/reference/user-guide.md` §5 "Create a pipeline", step 3, add one
sentence: no transform is allowed only with **Copy the source as is** switched on. Do not touch
`BACKLOG.md`.

## 4. Verification before handoff

```bash
bun run typecheck && bun run lint && bun test src/features/pipelines
git status
```

Run `bun run test` for the whole suite once, in the foreground. If it cannot fit in memory, run
it one folder at a time and total the counts, and say so. No cargo, Python or compose: none is
touched (`AGENTS.md`, "Only build what changed").

## 5. Out of scope

- Building FBIC.
- Removing `fbic_enabled` from the API or the store.
- Any change to the job.
- Refusing an empty transform list in the API.

## 6. Handoff (developer appends below)

## 7. Review (planner appends below)
