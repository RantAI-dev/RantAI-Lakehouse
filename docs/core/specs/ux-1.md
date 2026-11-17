# UX-1 Required-field indicators

Backlog: [UX-1](../BACKLOG.md). Feature page:
[Required fields](../features/required-field-consistency.md).

## Target

Every console data-entry surface in the implementation plan uses the same
visible `*` for fields requiring a supplied value and explains `* Required`.
Required state is exposed accessibly and follows existing conditional rules.
Optional, generated and fallback values remain valid. No new validation,
permissions, assistant tools or API requirements are introduced.

The baseline is a static audit at `eb9eaa18`: stars exist only in two
dashboard components. This is an internal consistency correction, not a
competitor performance claim. There are no proposed numeric targets.

## Acceptance

Use the checklist on the feature page. Source inventory and implementation
slices are in `docs/superpowers/plans/2026-11-17-required-field-consistency.md`.
