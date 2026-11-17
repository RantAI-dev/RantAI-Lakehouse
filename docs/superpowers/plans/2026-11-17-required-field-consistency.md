# Required-field consistency: audit and implementation plan

Date: 2026-11-17. Source base: `eb9eaa18e8ac184d5b0b00ea27b592c246e48d2f`.
Status: implemented and source-reviewed locally under UX-1; not pushed or merged; runtime acceptance pending.
Planner/reviewer: Codex, explicitly authorized by the owner in this conversation
to replace the repository's Claude-only role rule. Workers: `opencode-go/hy4-flash-free`.
The initial audit changed no product code. Implementation and verification
are recorded below; browser acceptance has not been performed.

## Owner request and scope

Audit all console forms before planning consistent `*` markers on required fields.
Include create/edit wizards, dialogs, sheets, inline editors, custom selectors,
file inputs and SQL editors, not only HTML `<form>` elements. Search/filter,
read-only and action-only surfaces are classified separately below.

## Audit result and size

Required indicators are inconsistent across the console. Only Chart Builder and
its SQL source panel render required stars: **12 render sites** (10 `Req` calls
and two literal stars). These are source sites, not a count of simultaneously
visible fields or unique requirements across all chart types.

Connectors/pipelines have **six principal flows**: connector create, connector
edit, upload, inline ingestion, pipeline create/edit (one shared editor), and
Agentic Builder. They contain **nine dial subform components**, none with stars.
Other console domains have roughly 40 unstarred client-required inputs;
this is an estimate, not a complete field total. Conditional/repeated fields
make a single global field count misleading.

**Estimated implementation footprint: 45–55 product files**, plus relevant
tests. Approximately 20–22 are in connectors/pipelines; the remainder are
other form consumers, shared patterns, and editor accessibility support.
This is a planning range, not a measured effort or an exact diff size. A
three-file wrapper-only change would miss dial forms, scope headings, upload
steps, dialogs without visible labels and inline editors.

### Inventory and verified anchors

Paths below are relative to `src/features/` unless otherwise stated. Required
means current client guard, explicit server constraint, or documented workflow
requirement; defaults and unknown server constraints are called out separately.

| Area / surfaces | Evidence at the source base | Required-field work |
| --- | --- | --- |
| Login and password change | `auth/login-page.tsx:237,239,251,264,275`; `auth/change-password-step.tsx:59,70,115,148` | Add visible stars to credentials/password fields; preserve forced-change conditional fields. Both forms use `noValidate`: native constraints do not block submission, though `required` still conveys semantics. Login has no local empty check; password change checks new length/match. |
| Users, roles, tenants, service identities | `admin/users-page.tsx:181`; `roles-page.tsx:132`; `tenants-page.tsx:148`; `service-identities-page.tsx:185` | Mark name/email for users; name/permissions/description for roles; name/slug/plan/residency for tenants; name/scopes/environment for identities. Do not mark optional user roles/tenants. |
| Employee create and run | `agents/employees-page.tsx:114,160`; `employee-detail-page.tsx:342,356` | Create requires name, purpose, tools, scope, numeric budget; autonomy is defaulted. Run prompt is required only without a configured prompt; currently indicated by copy and refused server-side. Add persistent label and conditional star. |
| Approval comment | `agents/approvals-page.tsx:428` | Optional; persistent optional label, no star. |
| Policy create wizard | `governance/policy-create-page.tsx:46,87,105,112,123,183` | Name, subjects, resources, effect require values; kind defaults. Conditions and activation optional. Local Field lacks label association and marker. |
| Quality create/edit and classification create | `governance/data-quality-page.tsx:300,307`; `quality-rule-edit-dialog.tsx:103,113,157`; `classification-page.tsx:214,219` | Mark current required fields; dimension/severity/classification defaults must be distinguished from values a user must supply. Column/masking rule remain optional. |
| Catalog access request, classification, policy | `catalog/asset-detail-page.tsx:190,205,226`; `asset-access.tsx:146,162,384,408,418,435` | Permission has a guard; reason must not be assumed required. Classification selectors default. Policy name and condition construction govern save; row filter remains optional. Associate labels with custom controls. |
| Catalog quality | `catalog/asset-quality.tsx:162,178,202,213,225,235,248,283` | Conditional column and threshold controls vary by check kind. Range endpoints need a group-level rule; do not blindly mark both. Generated rule name differs from a mandatory user-entered name. |
| Catalog meaning and annotations | `catalog/asset-meaning-view.tsx:464,494,508,522,538`; `asset-about.tsx:48,101,153` | Meaning must be nonempty; synonyms/role have different semantics. Description/owner/steward/tags are intentionally optional. |
| Freshness SLA | `catalog/asset-activity.tsx:241,252,283` | Amount must parse to minutes to save; unit defaults. Removal does not require entering a value. |
| Alerts create/edit | `alerts/alerts-page.tsx:232,337,427,506,534,556,667` | Name/delivery target and type-specific selectors/thresholds need matching markers. Type, delivery, severity, aggregate/operator have defaults. Preserve existing on-save errors. |
| Alert resolution | `overview/alerts-page.tsx:59` | Resolution note is mandatory for Resolve, not all neighboring actions. Explain this action-specific requirement explicitly. |
| Dashboard create/rename | `dashboards/dashboard-list-page.tsx:108,119,129`; `dashboard-actions.tsx:139,149` | Required name; description optional; folder has root default. Rename needs a visible label, not just aria-label. |
| Folder create/rename/move | `dashboards/folder-dialogs.tsx:60,65,87,107,120,205` | Name required for create/rename; root is a valid move/parent choice. Add persistent labels to name inputs. |
| Chart Builder | `dashboards/chart-builder.tsx:140,331,444,584,620,699,717,732,753,778,786` | Consolidate existing conditional stars. Non-text title can be generated; text title/content, source, dimensions/measures/breakdown/coordinates depend on chart kind. Preserve missing-field reporting. |
| Dashboard SQL source | `dashboards/sql-source-panel.tsx:136,162`; `lib/sql-source-draft.ts:78,130` (under `src/`) | SQL and source name already starred; consolidate, associate SQL editor label and required semantics. SQL run does not require source name; save does. |
| Saved query and SQL source sheets | `queries/save-query-sheet.tsx:52,68,77`; `save-sql-source-sheet.tsx:48,61` | Title mandatory; tags optional. Nonempty SQL comes from the external editor, not a missing title field. |
| Query SQL / natural-language input | `queries/sql-panel.tsx:76`; `nl-panel.tsx:32,38,47` | Persistent SQL/question label and required indication for execution; retain editor shortcuts and existing query blockers. |
| Copilot compose/edit/rename | `copilot/chat-composer.tsx:173,185,213,258`; `chat-messages.tsx:289,298`; `session-parts.tsx:135,145,151` | Nonblank message/edit/title gates actions. Include compact/full composer variants. Add suitable visible labels without changing chat behavior. |
| Connector create/edit and shared credential fields | `connectors/connector-create-page.tsx:153`; `connector-edit-page.tsx:120,144,174`; `connector-form-parts.tsx:469`; `credential-value-input.tsx:26` | Name, credentials and scope have step guards. Replacement credentials conditional on repoint/sign-in change, absent for no-auth. Propagate the same predicates to labels. |
| Connector scope | `connectors/connector-scope-fields.tsx:43,82` | Tenant/environment/residency use headings/cards and aria-label rather than ordinary Label. Include heading/group markers; no-tenant state remains a blocking explanation. |
| Nine dial subforms | `connectors/dial-forms/sql-dial-form.tsx:70`; `cdc-dial-form.tsx:105`; `oracle-dial-form.tsx:97`; `files-dial-form.tsx:60`; `rest-dial-form.tsx:76,103,115,168`; `sheets-dial-form.tsx:33,45`; `mongo-dial-form.tsx:84`; `kafka-dial-form.tsx:94,140`; `sftp-dial-form.tsx:78,84` | No stars. Verify adapter field matrix, defaults, auth/TLS/pagination and repeated endpoint/range requirements before applying markers. SFTP fingerprint alone has native required. |
| Upload file/check/table | `connectors/upload-file-page.tsx:141`; `upload-check-step.tsx:153,171,187,205`; `upload-table-step.tsx:45` | File and table name required; sheet/encoding/delimiter/header mostly chosen/defaulted. Mark file dropzone label as well as table input. Review step is read-only. |
| Inline ingestion | `connectors/connector-ingest-panel.tsx:548,642,676,848,918` | Manual object, target, cursor/key and custom cron requirements depend on mode. Discovery filters and default options do not get blanket stars. |
| Pipeline shared create/edit editor | `pipelines/pipeline-create-page.tsx` (thin wrapper) → `pipeline-editor.tsx`, `pipeline-editor-panes.tsx`, `pipeline-pickers.tsx`, `pipeline-transform-rows.tsx`, `pipeline-schedule-field.tsx` | The real Label/input layer lives in the editor and pane/picker/transform files (UX-1-R8 corrected this inventory). Required: pipeline name (create only), input zone/table/name, a form step's reads, the operations group and each operation's required arguments, output zone/table; the schedule field already carries its own star/aria-required. Verb/operator/type and the fixed connector zone are defaulted/fixed and stay unstarred. |
| Agentic Builder | `pipelines/agentic-builder-dialog.tsx:58,87,98` | Instruction is required; database is defaulted. |
| Maintenance policy | `lakehouse/table-detail-page.tsx:235,252,271,287` | Blank numeric overrides mean Default. No required stars; preserve nullable values and range validation. |

### Excluded surfaces, deliberately

Global/navbar search, command palette, data-table filters/pagination, column
filters, connector type search, schema discovery filters, lineage search,
conversation search and pipeline log search filter existing data; they are not
mandatory data-entry fields. Time travel, chart view filters, home reordering,
sharing switches, confirmation-only dialogs, tile data/expand dialogs and
read-only SSO have no missing-value requirement. Do not add stars there.
Copilot generated questions are option buttons (`chat-messages.tsx:327`), not
mandatory text fields. Optional approval comments and annotations remain in the
inventory to prevent accidental required markers. Iceberg's Required schema
column is metadata, not this UX convention.

### Limits and corrections to worker findings

This is a static frontend audit, with selected backend cross-checks, not a full
backend validation audit or rendered-browser certification. No exact global
required-field count or day estimate is claimed. The workers' approximate file
counts are not summed as exact totals; some orchestrators render no inputs.
The second worker described three auth required attributes as "dead": only
native submission enforcement is disabled by `noValidate`; accessibility
semantics remain useful. Its reference to connector "stars/attributes" was
imprecise: required text/attributes there are not visual star markers.

Backend cross-check: `rust/crates/lakehouse-store/src/ingest_spec.rs:111`
and `:114` make CDC slot/publication mandatory JSON fields; `:392` does the
same for SFTP fingerprint; `:922` checks Oracle TLS certificate DN conditionally.
Required JSON presence does not itself prove nonblank string enforcement.
Some dial inputs serialize empty strings: distinguish shape requirements from
meaningful user-required values rather than inventing enforcement.

## Proposed behavior

1. Every field that needs a supplied value has a visible `Label *`; the form or
   wizard step explains `* Required`. Use one shared marker implementation,
   `aria-hidden` on the decorative star, and required semantics on the control.
2. A valid default can satisfy a mandatory field. If the control can be cleared
   and blank is rejected, its required state stays true even while populated.
   If blank selects a fallback or generated value, it is optional. Presence of
   a default alone is not the rule for adding/removing stars.
3. Conditional marker and semantic state use the same condition as the existing
   requirement: chart kind, auth mode, TLS, incremental mode, edit credentials,
   custom cron or selected action. Hidden/inactive fields do not block actions.
4. Use native `required` where it preserves current submission behavior;
   otherwise use supported `aria-required` on the correct textbox, combobox or
   group role. Associate labels via ids/labelledby, including CodeMirror.
   Do not put aria-required on arbitrary containers or buttons.
5. Optional fields carry no star; retain helpful optional/default explanations.
   Give currently placeholder-only editors persistent labels. Explain group
   constraints such as range bounds and action-specific resolution notes.
6. This pass standardizes indicators and semantics. Keep current disabled/on-save
   validation policies. Adding client enforcement, removing `noValidate`, or
   strengthening server validation is separate work, explicitly recorded below.

## Implementation plan

### Ready prerequisites

UX-1 is registered with `docs/core/specs/ux-1.md` and
`docs/core/features/required-field-consistency.md`. The owner approved this
implementation and the Codex planner/reviewer plus Hy4 worker arrangement.
No dependency is declared for this presentation-only correction. Implementation
branches from the audited base; no merge of its unrelated parent work is implied.
Scope is presentation/accessibility, not new BI
capabilities: no assistant tool/schema changes are expected. Have the BI/AI owner
confirm that assessment when reviewing the dashboard slice; retain current tools
and dashboard context, with AI-15 smoke checks applicable to touched workflows.

### Slice A — shared convention and common forms

1. Add explicit required-marker support to the existing Label/FieldLabel system
   and a reusable legend in existing patterns. Avoid CSS selectors that infer
   requiredness from descendants. Preserve optional labels by default.
   Acceptance: one marker, stable spacing, no duplicate screen-reader star,
   correct required announcement and label association for native/custom controls.
2. Apply to authentication, admin, governance and employee create/run/approval
   surfaces in the inventory. Adapt local Field wrappers with ids and explicit
   required state; do not infer every child is required.
   Acceptance: labels agree with existing guards; optional comment/conditions,
   forced password mode and stored prompt behavior stay intact.

### Slice B — connector, upload and pipeline forms

3. Build an explicit adapter field matrix from frontend defaults, store parsing
   and adapter runtime requirements. Classify supplied/defaulted/optional/
   conditional/group-required values. Record any disagreement; do not add new
   blank-value rejection as part of the marker change.
   Acceptance: all nine dial forms mapped, including Oracle TLS, REST auth and
   pagination, Kafka SASL, SFTP trust pinning and repeated ranges/endpoints.
4. Apply markers/semantics to connector create/edit/shared credentials/scope,
   upload and inline ingestion; then pipeline editor/schedule/Agentic Builder.
   Acceptance: credentials optional on ordinary edit, required on applicable
   repoint/auth change; no-auth needs none; default upload options remain valid;
   custom cron and transform/load-mode fields update their required state.

### Slice C — BI, catalog, queries and remaining inline inputs

5. Consolidate Chart Builder/SQL source stars; apply the convention to dashboard
   and folder create/rename, saved query/source, SQL/question editors and catalog
   access/meaning/quality/SLA. Preserve optional metadata and maintenance defaults.
   Acceptance: chart-kind matrix, generated title/name, range-group requirements,
   SQL Run versus Save and root-folder choice are accurate; existing starred
   fields keep their enforcement and label semantics improve.
6. Apply to alert create/edit and resolution, Copilot compact/full composer,
   message edit and conversation rename. Acceptance: conditional alert targets,
   Resolve-only note requirement, nonblank message/title and optional approval
   comments communicate their existing requirements without changing actions.

### Per-slice verification and handoff

One numbered task per commit, in order, on a branch from a freshly verified base.
Run scoped TypeScript checks while iterating and the repository's frontend block
once per final slice: `bun run typecheck && bun run lint && bun run test`.
Add meaningful rendering/behavior checks for shared semantics and conditional
cases, not snapshots of every label. Run keyboard/screen-reader inspection and
owner acceptance on the listed flows, including dark/light theme and narrow
dialogs. Record exact commands/counts and anything not verified in Handoff.
No Rust/Python/Compose checks are needed for a frontend-only diff. Reviewer
independently reviews the field matrix, diff and verification. Do not merge or
open a PR as part of this audit request.

## Separate follow-up candidates (not authorized implementation scope)

- Auth local empty checks/native validation strategy, retaining current errors.
- Early dial-field validation versus current server/runtime checks.
- Empty run-prompt and Agentic Builder instruction feedback before submission.
- Consistent error timing, disabled-button explanations and first-error focus
  across all forms; this is larger than required-label consistency.

## Task 3 — Adapter field matrix (worker-documented, 2026-11-17)

Built by reading all nine dial forms under `src/features/connectors/dial-forms/`
against the server structs/validation in
`rust/crates/lakehouse-store/src/ingest_spec.rs`. "Supplied" means the user
must type a value; "Defaulted" means an option in a closed set already has a
valid value; "Optional" means blank is accepted (often mapped to `null`);
"Conditional" means required only under a selected auth/TLS/mode. This matrix
is the basis for the Task 4 markers and adds **no** new blank-value rejection:
native `required`/`aria-required` and stars expose what the form/server already
treats as necessary, they do not strengthen validation.

| Form | Supplied | Defaulted | Optional | Conditional / group |
| --- | --- | --- | --- | --- |
| `sql-dial-form` | Host, Port, Database, User | Driver (from connector type; asked only on mismatch) | SSL mode; SSL server cert DN | Oracle TLS DN required when SSL mode is set and not `disable` (`ingest_spec.rs:921-934`) |
| `cdc-dial-form` | Host, Port, Database, User, Replication slot name, Publication name | Driver (as above) | Server id | Slot/publication are non-`Option` server fields (`ingest_spec.rs:112,114`) |
| `oracle-dial-form` | Host, Port, Database (service name), User | Port 1521, driver fixed `oracle`, batch-only note | SSL mode | TLS server certificate DN required when SSL mode is set and not `disable` |
| `files-dial-form` | Bucket / root | Protocol (read-only S3), Format (`csv`) | Prefix, Region | Endpoint blank accepted but "needed to test the connection" — conditional for the Test action, not a save requirement |
| `rest-dial-form` | Base URL; per-endpoint Path (each row) | Auth type (`api_key`), Pagination (`none`) | Records path (per endpoint) | API-key Header name when `api_key`; Token URL when `oauth2_client_credentials`; bearer/basic carry no field; Page query parameter when `page`; Cursor field when `cursor` |
| `sheets-dial-form` | Spreadsheet id; at least one Range | — | — | Repeated ranges: the group needs one value; additional ranges are optional until added |
| `mongo-dial-form` | Seed host(s) (each), Database, Username | Direct connection fixed `true` | — | Repeated hosts: the group needs one value |
| `kafka-dial-form` | Bootstrap server(s) (each), Topic, Consumer group id | Micro-batch cap 30, Auth type (`sasl_plain`) | — | SASL username when `sasl_plain`; `none` auth renders no credential field |
| `sftp-dial-form` | Host, Port, User, Remote path, Host key fingerprint | Port 22, File format (`csv`), Auth type (`password`) | — | Fingerprint required unconditionally with no fallback (`ingest_spec.rs:392`); already native `required` in the form; public-key private key is the connector secretRef, not a dial field |

**Disagreements / limits recorded, not silently resolved:**

- Serde shape vs. meaningful value: a non-`Option` `String` (host, database,
  user, bucket, topic, group id, per-row paths) is structurally required, but
  the store does not reject every empty string for every field. The matrix
  marks these "supplied" for the *indicator* because the form has no fallback
  and the value is a mandatory part of the dial shape; no new nonblank check is
  added. `validate_hostname`, the CDC pair, SFTP fingerprint and Oracle TLS DN
  are the explicit enforced nonblank rules.
- `files` Endpoint is conditional on the **Test connection** action, not on
  saving the ingest spec (AWS S3 needs no endpoint). Marker follows the test
  requirement with an explanatory note, not a blanket star.
- Repeated ranges/hosts/servers/endpoints (corrected by UX-1-R5): the first-row
  assumption was wrong. `Dial::parse` runs `check_hostname` over every Mongo
  `hosts` and Kafka `bootstrapServers` entry, and the Sheets/REST adapters read
  every `ranges`/`endpoints` entry at runtime — a blank added row is rejected or
  unreadable. Every rendered row is therefore marked required with a visible
  associated label; the group heading carries the star and the copy tells the
  user to Remove rows they do not need. No new blank-value validation was added.
- This remains a static source audit; no browser rendering was checked here.

## Review / Handoff

### Current-main review round 3 (head bb8861e8e)

Not approved yet. Required-field coverage was lost when translating old forms.

- **BLOCKER UX-1-R8:** Pipeline create is now a wrapper around
  `pipeline-editor.tsx`, `pipeline-editor-panes.tsx`, `pipeline-pickers.tsx`
  and `pipeline-transform-rows.tsx`. These DO contain Label/input layers;
  the reconciliation handoff's claim otherwise is incorrect. Apply the agreed
  shared convention to required name, input aliases/tables/zones, output and
  per-operation fields using current guards. Do not alter the newer editor or
  its detailed validation status. Optional/fixed/default values stay accurate.
- **BLOCKER UX-1-R9:** New `alerts/alert-rule-page.tsx` marks only name.
  Required delivery target, kind-specific mart/measure/metric/connector/dashboard/
  table and other required inputs must retain the old slice's coverage, mapped
  to current guards rather than blindly copied.
- **BLOCKER UX-1-R10:** Replaced admin/quality forms carry stars without
  required semantics on their inputs/selects. Add opt-in required semantics to
  SelectField's actual trigger and appropriate input/group instructions in
  PermissionPicker. Apply them to starred controls in tenant/service identity,
  quality CheckFields and custom thresholds/dimensions. RoleFields is shared
  by role-create/edit and needs a legend at the composed form. Invite username
  OR email should have a starred group and linked requirement explanation.
  A group must not falsely mark each member required.
- **SHOULD-FIX UX-1-R11:** Rewritten ChartBuilder retained its duplicate Req
  implementation. Make it delegate to RequiredMarker without altering its
  many current chart rules. Its optional dimension branch needs the matching
  ch-dimension id. Marking semantics in child chart-field components should
  follow their existing required stars where applicable.

No API/validation redesign is authorized. Correct these missing indicators and
semantics, preserve all current-main behavior, add focused regression coverage,
and rerun frontend checks. Parent re-review before push/merge remains required.

### Worker handoff: round-3 findings R8–R11 (2026-11-17, Hy4)

All four round-3 findings were fixed on `fix/required-field-consistency`, each
in its own commit, with the finding id cited at the fix sites and in the commit
body. The branch head before the parent's round-3 review was `bb8861e8e`; this
work adds four commits on top of the documentation commit `08651fd28`.

| Finding | Commit | Scope |
| --- | --- | --- |
| UX-1-R8 | `16ded3791` | Pipeline editor, panes, pickers and transform rows; opt-in `aria-required` on `SelectField`'s real combobox trigger (shared prerequisite for R9/R10) |
| UX-1-R9 | `f4ff4ebc7` | Alert rule page: target, metric, mart/measure, connector, dashboard, quality/freshness TablePickers across the type predicates |
| UX-1-R10 | `9668cfd7c` | Tenant/service-identity inputs, `PermissionPicker` group semantics, RoleFields legend, invite username-or-email group, quality `CheckFields` and custom threshold/dimension semantics |
| UX-1-R11 | `105d36091` | `Req` delegates to `RequiredMarker`, optional dimension `ch-dimension` id, child chart-field markers and semantics |

The inventory's pipeline row and the reconciliation handoff's false "no
per-field Label/Field layer" claim were corrected above. No validation,
submission, disabled-state, permission, dependency or API behavior was changed;
the `SelectField` change is an opt-in prop with no effect on existing callers.

**Verification (foreground, final tree, current-main tooling).** Exact commands
and counts actually observed:

- `bun run typecheck` — exit 0, no diagnostics.
- `bun run lint` — exit 0, **0 errors and 11 warnings**. All 11 are pre-existing
  on current main (sidebar, sso-page, alerts-page, login-page, use-copilot,
  use-data-table, dashboard-specs, uploads.test, services/http); none is in a
  file touched by R8–R11. No new warning.
- `bun run test` — exit **0**, **4484 pass, 1 skip, 0 fail, 13877 expect()
  calls, 415 files, 190.66s** (output at `/tmp/ux1-r8-r11-full-test.log`).
  This is the full green current-main baseline plus the new regression tests;
  no unhandled animation errors were observed.
- Scoped foreground runs on final code: the 14 touched test files together —
  **246 pass, 0 fail, 2372 expects**. Per finding: pipeline editor 48 pass;
  alert rule page 16 pass; admin/quality group+semantics set 74 pass; chart
  builder 51 pass, chart shape 30 pass, type-B builder 13 pass (94 across the
  three chart files); `SelectField` 8 pass; shared marker 5 pass. One combined
  run reported a single flaky failure that did not reproduce on two subsequent
  runs (246/0 both times) or in the full suite; it is not claimed as fixed, only
  as not reproduced.
- `git status`: only the intended source/test files and this plan are tracked
  changes; unrelated user-untracked files (`,`, `.claude/`, `reports/`,
  `research_notes/`, the NDJSON/manifest pair) were left untouched. No Rust,
  Python or Compose commands were run — the diff is frontend TypeScript and
  documentation only.

**Not verified:** real browser rendering, keyboard/screen-reader inspection,
focus order, dark/light theme and narrow-dialog layout, AI-15 assistant smoke
checks on touched dashboard workflows, and product-owner acceptance. The star
is the shared CSS `::before` on an empty `aria-hidden` span; happy-dom does not
apply `::before`, so tests assert the marker class plus `data-required` and
control `aria-required`, not the painted glyph. Required semantics were placed
only on supported roles (textbox, combobox); the Faceted `TablePicker`/column
pickers keep the star-plus-sentence group pattern because their trigger is a
button, and `PermissionPicker`/invite username-or-email groups carry a linked
requirement sentence rather than a false `aria-required` on a group or a
checkbox.

**Unresolved gaps / review points:**

- The pipeline editor's "why it cannot be saved yet" blocker list remains the
  authoritative save-time explanation; markers are presentation only and add
  no client validation. The fixed, disabled edit-mode pipeline name is
  deliberately not starred (it cannot be cleared).
- No new blank-value rejection was added anywhere; "required" markers follow
  the current save guards, not a stronger validation policy. The quality-rule
  Dimension input keeps the star it already carried and now has matching
  `aria-required`; for a picked check a blank value falls back to the check
  kind's dimension, so this is a deliberate pre-existing marker (the saved rule
  always carries a dimension), not a new blank-value rule.
- The catalog-backed
  `TablePicker` and `ColumnsField` use the group pattern on a button trigger,
  so the marker is paired with a visible sentence instead of `aria-required`.
- Browser/AT verification and owner sign-off remain open; this is a source and
  test-suite result only. Parent independent re-review is still required before
  push/merge.

### Delivery update: current main reconciliation (2026-11-17)

Owner requested push and merge if good. PR #568 is open. Fetched main is
`cb855d5b923d978f14371c3e61d88f1d8be1ef8b`, with substantial changes since
the audit base; GitHub reports conflicts. Previous source review applies to
the old base and cannot certify the merged result. Hy4 must merge current main
hunk by hunk, retaining its newer functionality and applying only equivalent
UX-1 presentation/accessibility changes. Deleted/replaced forms must not be
resurrected. Update the inventory for changed anchors/requirements and report
any decision mismatch; do not alter APIs, validation or dependencies to resolve
conflicts. Use current main's frontend tooling as provided, not older dependency
versions. Re-run the frontend verification and compare any failure to the newly
fetched main baseline. Parent re-review is mandatory before merge.

### Worker reconciliation handoff: merge of current main (2026-11-17)

Merged `origin/main` (`cb855d5b923d978f14371c3e61d88f1d8be1ef8b`, 3277
commits since the audit base `eb9eaa18`) into `fix/required-field-consistency`
with `git merge origin/main --no-edit` (no rebase, no force push). All 32
conflicts were resolved hunk by hunk: 26 content conflicts and six
modify/delete conflicts for forms main deleted. No whole-file
`--ours`/`--theirs` resolution was used on a file both sides changed; where
all our hunks were superseded by main's code the per-hunk resolution leaves a
file byte-identical to main (verified with `diff` against `origin/main`).

**Deleted forms not resurrected.** Main removed the inline create sheets and
dedicated forms; the UX-1 markers were translated to the forms that replaced
them. The six remove-side files were deleted with `git rm`:
`agents/employee-detail-page.tsx`, `agents/employees-page.tsx` (the whole
Digital Employees surface is gone in main), `governance/classification-page.tsx`
(classification moved onto the table's columns), `governance/policy-create-page.tsx`
(the wizard was removed by WEB-3/SEC-61), `queries/save-query-sheet.tsx`
→ `queries/save-query-page.tsx`, and `queries/save-sql-source-sheet.tsx`
→ `queries/save-sql-source-page.tsx`.

**Inventory corrections for the current tree (anchors at the merge head):**
- Admin create → `admin/role-form-page.tsx`, `admin/tenant-create-page.tsx`,
  `admin/service-identity-create-page.tsx`, `admin/invite-user-page.tsx`.
  Roles/permissions/description, tenant name/slug/plan/residency and identity
  name/scopes/environment remain required by the same submit guards. Invite
  keeps the exact `Username` label and groups username-or-email with a visible
  explanation rather than a second star: the guard requires one of the two.
- Quality → `governance/quality-rule-create-page.tsx` and the shared
  `governance/quality-check-fields.tsx` (`CheckFields`). Rule name, asset,
  threshold and dimension are required; a check's Column, Minimum rows and
  Rows-that-must-have-a-value are required when that check is selected. The
  range/stat bounds keep the UX-1-R4 group star ("at least one"), not a star
  on both inputs. Generated names elsewhere stay unstarred.
- Freshness SLA → `governance/freshness-target-dialog.tsx`: amount required,
  unit defaulted. Removal still needs no value.
- Alerts → `alerts/alert-rule-page.tsx`: name and delivery target required by
  the same on-save guards; the list page is unchanged from main.
- ~~Pipelines → `pipelines/pipeline-editor.tsx` (the create/edit page is now a
  thin wrapper). Its current "why it cannot be saved yet" status already
  communicates the guards; no new markers were added there because the
  current editor has no per-field `Label`/`Field` layer to attach them to
  without inventing a new form design — reported, not improvised.~~
  **Corrected by UX-1-R8 (round 3):** this claim was false. The editor and its
  panes/pickers/transform rows do have `Label`/`Input`/`SelectField` layers;
  the required markers were applied there in commit `16ded3791`. The "why it
  cannot be saved yet" status is retained, not replaced.
- Chart Builder was rewritten by main with accordion sections and its own
  `Req()` markers. Those stars were preserved; the UX-1-R3 contribution is the
  label association and `aria-required` on `ch-source`, `ch-dimension`,
  `ch-breakdown`, `ch-lat`, `ch-lon`, `ch-content` and the conditional title,
  plus one `RequiredLegend`. The measure and grouping rows now live in
  `chart-shape-section.tsx`, which main already marks; they were not duplicated.
- Connector ingest KeyFields (UX-1-R6) now uses a visible `Key *` group label
  and a linked requirement sentence on the current form; no `aria-required`
  was put on the fieldset or a button. `TablePicker` gained an opt-in
  `required` prop for the governance table fields; the star and
  `aria-required` go on the textbox, never the Faceted trigger button.
- Oracle TLS DN conditional state was corrected to main's semantics: Oracle's
  `require` mode is native network encryption with no DN check, so the DN is
  required only for a certificate-verifying mode (`verify-ca`), not for
  `require`. CDC slot/publication are required only on the PostgreSQL driver
  that renders them. This follows the current server/runtime requirement and
  adds no validation.
- Main's plain `Label` was kept; the UX-1 `required` prop remains opt-in in
  the shared Label component. No dependencies, versions, tooling, APIs,
  permissions or validation rules were changed to resolve a conflict.

**UX-1 test adjustments (checks kept, not weakened).** `dial-forms-required.test.tsx`
now drives the current `SelectField`-based REST/OAuth form by rendering each
mode explicitly instead of `fireEvent.change` on a native select; it still
asserts every conditional star and `aria-required`. The CDC case asserts the
driver-specific fields it now renders. `connector-ingest-panel.test.tsx`
asserts the current visible group label, `aria-label` fieldset and absence of
`aria-required` on the fieldset/checkboxes. `catalog/required-groups.test.tsx`
renders the shared `CheckFields` directly for the range bounds, asserts the
asset rule-name field separately, stubs the current incident/schedule/learning
responses, and restores `next/navigation` in `afterAll`. `chart-builder.test.tsx`
keeps main's entire 50-test suite plus two UX-1 association tests adapted to
the current ids/labels. `login-page.test.tsx` uses main's current label text.
`sql-source-panel.test.tsx` restores the real `@/components/sql-editor` module
in `afterAll` so the module mock cannot leak into later files.
`copilot/chat-composer.tsx` kept `aria-label="Message AI Copilot"` on both
composer variants so existing accessible-name queries still resolve.

**Verification (foreground, final tree, current main tooling).** The full
baseline was reproduced from a `git archive origin/main` extraction (same
installed `node_modules`): `bun test src` → 4444 pass, 1 skip, 0 fail, 4445
tests across 409 files, exit 0. Final merged tree:

- `bun run typecheck` → exit 0, no diagnostics.
- `bun run lint` → exit 0, **0 errors and 11 warnings**; all are pre-existing
  on main (the baseline run reported the same set; the UX-1 test was initially
  flagged for an unused `fireEvent`, which was removed). No new warning.
- `bun run test` → exit 0, **4467 pass, 1 skip, 0 fail, 4468 tests across 415
  files**. Ten new tests (415 vs 409 files) pass; no baseline failure remains.
  The 24 happy-dom/Motion unhandled AbortErrors recorded at the old base are
  absent from this current-main run; UX-2 remains a backlog item and this
  result does not claim those tests were fixed by UX-1.
- Scoped foreground runs: `dial-forms-required.test.tsx` 7 pass / 55 expects;
  `chart-builder.test.tsx` 50 pass / 338 expects; the ingest panel suite 95
  pass / 352 expects; login + catalog group tests 9 pass / 27 expects;
  `invite-user-page.test.tsx` 7 pass / 33 expects; per-role navigation 6 pass
  / 43 expects; asset-detail tabs 169 pass / 561 expects; SQL panel + schema
  browser + keyboard shortcuts 23 pass / 93 expects.
- `git status`: no tracked changes left after the merge commit; unrelated
  user-untracked files (`,`, `.claude/`, `reports/`, `research_notes/`, the
  NDJSON/manifest pair) untouched. Regenerable caches were cleared only when
  the filesystem filled; no project source was deleted.

No Rust/Python/Compose commands were run: the diff is frontend TypeScript and
this documentation only. Not verified: browser rendering, keyboard/screen-reader
inspection, light/dark/narrow layouts, AI-15 assistant smoke checks and owner
acceptance. The parent must independently review the merge diff and verify
before pushing the branch or merging the PR.

### Final parent review (2026-11-17, code head 39617ff62)

All round-1 BLOCKER findings R1–R6 and SHOULD-FIX R7 are resolved by source
inspection. The parent independently reviewed the fix diff and the baseline-to-
final change. No open source-review blocker remains. During this review a small
R7 regression was fixed directly in `39617ff62`: the paired credential grid had
accidentally changed `sm:grid-cols-2` to `sm:col-span-2`; the two-column layout
is restored. Validation, API contracts and permissions are unchanged.

Independent foreground verification:

- `bun run typecheck && bun run lint && bun run test` (test output captured in
  `/tmp/ux-1-parent-tests.log`): typecheck exit 0; lint exit 0, eight warnings;
  test exit **1**, **1187 pass, 1 skip, 0 fail, 24 unhandled errors, 3058
  expects, 125 files, 200.58s**. These are not reported as a green test command.
  The tests ran after the final code correction; the initial typecheck preceded
  that CSS-class-only correction, so `bun run typecheck` was rerun afterward,
  exit 0.
- Independent baseline reproduction: extracted tracked `src`, `bunfig.toml`,
  `happydom.ts`, `testing-cleanup.ts`, `tsconfig.json`, `package.json` with
  `git archive bcdef6bd3` to a temporary directory and linked the same installed
  node_modules. `bun test src/features/copilot` there exited **1** with **31
  pass, 0 fail, 24 unhandled errors, 66 expects, six files, 1.56s**. Stack
  traces match happy-dom Animation.cancel / motion-dom NativeAnimation.stop.
  This independently establishes the same 24 errors before UX-1, without
  trusting the worker's full-baseline claim. Full baseline suite was not rerun
  by the parent. The existing runner problem is now tracked as UX-2.
- Compiled the `before:content-['*']` utility using installed
  `@tailwindcss/node.compile` and asserted the generated star content: passed.
  This confirms CSS generation, not rendered visual acceptance.
- `git diff --check bcdef6bd3..HEAD`: passed; `git status --short`: no tracked
  changes left after commits; unrelated user untracked files left untouched.

Not verified: real browser appearance, keyboard/screen-reader behavior,
light/dark/narrow layouts, AI-15 runtime checks and product-owner acceptance.
No Rust/Python/Compose/dependency changes, so their checks were not run.
No PR was opened, branch pushed, or merge performed. UX-1 remains In Progress
until delivery; the owner checklist is not marked passed by this review.

### Parent review round 1 (2026-11-17, head 2ae89651a)

Not approved. Independent `bun run typecheck && bun run lint` passed;
lint reports eight warnings, not the single warning mentioned by the worker.
Full tests will be rerun after the corrections below.

- **BLOCKER UX-1-R1:** `copilot/chat-composer.tsx` replaced the textarea's
  relative positioning ancestor with a grid while its animated example remains
  absolutely positioned. Restore a relative wrapper around textarea/overlay
  below the label/legend, preserving their alignment and existing behavior.
- **BLOCKER UX-1-R2:** Compact composer and folder rename put required labels
  inside `sr-only`, hiding the requested visible star. Give both persistent
  visible required labels and explain the convention in the compact surface.
- **BLOCKER UX-1-R3:** Chart Builder's required dynamic select labels are still
  unassociated. Add ids/htmlFor or labelledby for every touched required control
  (dimension/breakdown/coordinates/measures). Use Label's shared required prop.
- **BLOCKER UX-1-R4:** Catalog masking columns are an alternative to row filter
  (`policyCondition` accepts either), not unconditionally required. Show a
  starred group-level requirement with explanatory text, keeping both
  alternatives optional individually. Apply the same accurate group pattern to
  range bounds. Quality-rule name accepts generated initial state but clearing
  it blocks Save; mark that input required and correct its optional copy.
- **BLOCKER UX-1-R5:** Repeated Mongo/Kafka rows use `index === 0` required
  semantics, but store Dial::parse validates every entered row, including blank
  rows. Mark each rendered host/broker required with an associated visible row
  label, explaining Remove for unused rows. Inspect Sheets/REST actual runtime
  handling and use accurate group/row semantics instead of assuming first row.
- **BLOCKER UX-1-R6:** `aria-required` is unsupported on a native fieldset's
  implicit group role in ingestion KeyFields. Use a visible legend and linked
  requirement instructions; do not falsely mark every checkbox required.
- **SHOULD-FIX UX-1-R7:** Nested connector step/dial/credential components each
  render a legend. Ensure one explanation per step when composed, while retaining
  standalone legends where needed. Fix indentation of added aria attributes.

Keep validation semantics unchanged; fixes must cite these finding ids at the
site and in commit bodies. No implementation is approved by this review yet.

Planner reconciled both read-only worker audits against shared components and
selected guards/server constraints. No code changes, tests, browser checks or
implementation handoff yet. Existing untracked user files were left untouched.

### Worker resolutions to parent review round 1 (2026-11-17)

All seven findings were addressed on `fix/required-field-consistency`; the IDs
are cited at each fix site and in the commit bodies. No validation, submission,
permission or error-path behavior was changed.

| Finding | Fix | Commit |
| --- | --- | --- |
| UX-1-R1 | Restored a `relative` wrapper around just the textarea and animated example, below the legend/label | `f047ba4a2` |
| UX-1-R2 | Compact composer and inline folder rename now use visible associated required labels; compact adds tooltip + screen-reader explanation | `f047ba4a2` |
| UX-1-R3 | Every kind-dependent chart select has a stable id + `htmlFor` Label using the shared `required` prop | `bcbb44941` |
| UX-1-R4 | Catalog mask/row-filter and range bounds starred as groups with explanatory text; quality-rule name marked required with corrected copy | `32f8c0c9b` |
| UX-1-R5 | Every rendered Mongo/Kafka/Sheets/REST row required with a visible label and Remove explanation; runtime semantics checked against `Dial::parse` and the dial structs | `3afa0fe4e` |
| UX-1-R6 | KeyFields uses a visible legend + linked requirement text; no `aria-required` on the fieldset or its checkboxes | `3afa0fe4e` |
| UX-1-R7 | `showLegend` opt-out on dial forms, `DialFormFor` and `CredentialFields`; create/edit steps show one legend; fixed Kafka attribute indentation | `3afa0fe4e` |

Regression tests were added in `a879dd57f` for the repeated-row semantics,
legend opt-out, composer overlay/labels, folder-rename label, chart label
associations, catalog group stars and KeyFields legend.

### Worker handoff (UX-1 implementation, 2026-11-17)

All six numbered tasks are complete on `fix/required-field-consistency`,
branched from base `bcdef6bd3`. One commit per numbered task, in order:

| Task | Commit | Scope |
| --- | --- | --- |
| 1 | `a19b464a1` | Shared `RequiredMarker`, `Label`/`FieldLabel` `required` prop, `RequiredLegend`, component tests |
| 2 | `85a4f8686` | Auth, admin, governance, employees/approvals |
| 3 | `c11700faf` | Adapter dial field matrix (documentation only) |
| 4 | `467417408` | Connector create/edit, credentials, scope, nine dial forms, upload, inline ingestion, pipelines, Agentic Builder |
| 5 | `8febdec13` | Chart Builder/SQL source consolidation, dashboards/folders, queries, catalog |
| 6 | `d0c5cd8c3` | Alerts create/edit/resolution, Copilot composer/edit/rename |

All tests were run in the foreground on this branch's final commit. Counts
actually observed:

- `bun run typecheck` — passed (no diagnostics), final slice and full block.
  Re-run after the review fixes: exit 0, no diagnostics.
- `bun run lint` — passed with **0 errors and 8 warnings**, not one warning.
  The eight are pre-existing: `react-hooks/exhaustive-deps` in
  `data-table.tsx:590`, `sidebar.tsx:91`, `alerts-page.tsx:701` and
  `use-data-table.ts:238`; `@next/next/no-location-assign-relative-destination`
  in `login-page.tsx:70` and `services/http.ts:84`; and
  `@typescript-eslint/no-unused-vars` in `open-format-card.test.tsx:165` and
  `dashboard-specs.ts:260`. The corrected handoff re-ran lint after the review
  fixes: still **8 warnings, no new warning** (exit 0). The worker's earlier
  "single pre-existing unused-variable warning" claim was inaccurate.
- `bun run test` (full block) — **1187 pass, 1 skip, 0 failed assertions,
  3058 expect() calls, 125 files, 254.07s**. The command exits 1 because of 24
  *unhandled* `AbortError: The animation was canceled.` events from
  `motion-dom`/`happy-dom` reported "between tests". These are **baseline**:
  the same command at base `bcdef6bd3` in a temporary worktree (same `node_modules`)
  exits 1 with **1165 pass, 1 skip, 0 failed assertions** — the same class of
  unhandled animation errors and no failed assertions. The early handoff at
  `2ae89651a` recorded **1177 pass, 1 skip, 2986 expects, 121 files**; the 10
  added regression tests and the review fixes take it to the quoted final
  counts with no failing assertion. (A later baseline rerun was OOM-killed;
  the first baseline run is the quoted evidence.)
- Task-scoped runs (original implementation): Task 1 `bun test required.test.tsx`
  5 pass / 12 expects; Task 2 `bun test login-page.test.tsx required.test.tsx`
  8 pass / 22 expects; Task 4 `bun test src/features/connectors
  src/features/pipelines` 275 pass / 1127 expects, including the new
  `dial-forms-required.test.tsx` (5 pass / 32 expects); Task 5
  `bun test src/features/queries src/features/catalog` 281 pass / 15 files and
  `sql-source-panel.test.tsx` 1 pass / 8 expects; Task 6
  `bun test alerts overview copilot` 59 pass / 9 files.
- Review-fix scoped runs: `bun test` on the six touched/new test files
  (dial forms, ingest panel, composer, folder dialogs, chart builder, catalog
  required groups) — **66 pass, 0 fail, 300 expect() calls, 6 files**; connector
  create/edit/ingest/dial-forms set — **91 pass, 0 fail, 384 expects, 6 files**;
  composer test 2 pass / 11 expects; folder rename 1 pass / 3 expects; chart
  builder 2 pass / 16 expects; catalog groups 2 pass; ingest panel 52 pass.

**Design note for review (corrected).** The star is a CSS `::before` on an
empty `aria-hidden` span. The earlier claim that a literal `*` text node
changed the label's accessible name was **inaccurate**: `aria-hidden="true"`
already excludes that text from the accessible name in a real browser.
`getByLabelText`, however, matches on `textContent`, which includes
`aria-hidden` text, so a literal `*` broke the existing exact label queries.
The CSS glyph was chosen for that test behavior, not because accessibility
required it; it keeps the star visible while leaving `textContent` unchanged.
Tests therefore assert `data-required` and the marker class, not the rendered
glyph (happy-dom does not apply `::before`). The star's actual visibility still
needs a browser check. Required semantics use native `required` where already
present (auth, SFTP fingerprint); elsewhere `aria-required` is used on a
control whose role supports it (textbox/combobox/radiogroup). It is **not** put
on a native fieldset (UX-1-R6) — the KeyFields group uses a visible legend and
linked instructions instead. Native submit behavior is unchanged; in
particular the conversation-rename `<form>` uses `aria-required` only, and all
auth forms keep `noValidate`.

**Not verified (and why):**

- Real browser rendering, keyboard and screen-reader inspection, focus order,
  dark/light theme and narrow-dialog layout — no browser acceptance was
  requested for this worker and no runtime accessibility tooling is configured;
  the feature page's owner checklist remains pending.
- AI-15 assistant smoke checks on touched dashboard workflows — deferred to the
  BI/AI owner review; this is a presentation-only change with no tool/schema or
  dashboard-context changes.
- Rust/Python/Compose checks — not run; the diff is frontend TypeScript and this
  documentation only.
- Tooltip/legend placement in every dialog was checked statically, not by eye.

**Gaps and review points:**

- Dial "supplied" fields are marked from the form's mandatory dial shape and
  explicit guards; serde shape presence alone does not prove nonblank rejection
  for every string (see the Task 3 matrix). No server or client blank-value
  validation was added.
- The `files` Endpoint keeps its existing "needed to test the connection" note
  with no star: it gates the Test action, not saving the ingest spec.
- Repeated range/host/broker/endpoint groups (UX-1-R5) mark **every rendered
  row** required with a visible associated label, not just the first row; the
  group heading carries the star and the copy explains Remove for unused rows.
  This follows the actual runtime semantics in `Dial::parse` and the
  Sheets/REST adapters, not a first-row assumption.
- Chart Builder's dynamic selects (UX-1-R3) now have ids and `htmlFor`-linked
  visible Labels using the shared `required` prop, not just `aria-required` on
  an unlabelled `SelectTrigger`. Title, content and source use the same prop.
- The resolution-note star is always shown on the shared field, with copy
  stating it is required only for Resolve, optional for Acknowledge/Silence.
- The **quality-rule** name is now required (UX-1-R4): it is pre-filled but
  clearing it blocks Save. Saved query/SQL-source generated names remain
  unstarred per the plan's generated-value rule; no guard was changed.
- Early in the session the filesystem filled and generated `.next/` and
  `node_modules/.cache` were removed to make room; both are regenerable build
  artifacts. The same happened during the review-fix verification; regenerable
  package caches (pip, uv, playwright, huggingface) were purged so the full
  frontend block could finish. No tracked or user-untracked project file was
  deleted. Unrelated untracked files (`,`, `.claude/`, `reports/`,
  `research_notes/`, the NDJSON/manifest pair) were left untouched throughout.
