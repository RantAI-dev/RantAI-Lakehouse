# Consistent required fields

Backlog: UX-1. Spec: [ux-1.md](../specs/ux-1.md).
Plan: [implementation plan](../../superpowers/plans/2026-11-17-required-field-consistency.md).

Delivery: merged in [PR #568](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/568)
on 2026-11-18 (`4e53e41c76eddaeb28161d7c7ecc194f2053a353`). The tracked
Task is Done; owner runtime acceptance below remains pending.

## Decisions

The owner approved implementation on 2026-11-17 ("go ahead ask hy4 to do it
for you, you plan and review"). Codex plans/reviews and Hy4 implements,
overriding the repository's Claude-only planner rule for this work.
Use visible `*` markers and accessible required state, following existing
requirements. Keep existing validation behavior and optional/default values.
No new assistant capability is needed for a presentation-only correction.

## Acceptance checklist

Owner runtime acceptance is pending; no step below has been executed.

- Required inputs show `*` and their form or step explains `* Required`.
- Screen readers receive the label and required state for native/custom inputs.
- Auth, admin, governance, employees and conditional run prompts follow the rule.
- Connector create/edit credentials match auth and repoint requirements; all
  dial forms, scope, upload and ingestion modes communicate required fields.
- Pipelines communicate source/target, transform and conditional cron requirements.
- Dashboard/chart/folder/query forms preserve generated names and optional values.
- Catalog quality ranges, meanings, access and SLA fields show accurate requirements.
- Alerts, resolution notes and Copilot inputs show action-specific requirements.
- Optional annotations, maintenance defaults and approval comments remain optional.
- Search/filter/read-only/action-only surfaces acquire no misleading stars.
- Existing submission errors, disabled actions and keyboard shortcuts still work.
- Labels and markers remain legible in light/dark themes and narrow dialogs.
