# Required-field consistency: audit and implementation plan

Date: 2026-11-17. Source base: `eb9eaa18e8ac184d5b0b00ea27b592c246e48d2f`.
Status: audit complete at static-source level; implementation authorized 2026-11-17 under UX-1.
Planner/reviewer: Codex, explicitly authorized by the owner in this conversation
to replace the repository's Claude-only role rule. Workers: `opencode-go/hy4-flash-free`.
No product code was changed, no build or browser verification was performed.

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
| Pipeline shared create/edit editor | `pipelines/pipeline-create-page.tsx:65,159,240,279,383,513,578`; `pipeline-schedule-field.tsx:74` | Name/source/target, incremental field, transform arguments and custom schedule are conditional. Shared Field needs explicit required state and control association. |
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

## Review / Handoff

Planner reconciled both read-only worker audits against shared components and
selected guards/server constraints. No code changes, tests, browser checks or
implementation handoff yet. Existing untracked user files were left untouched.
