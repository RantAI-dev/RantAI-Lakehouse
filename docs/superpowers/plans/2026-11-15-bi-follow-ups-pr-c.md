# BI follow-ups, PR C: alerts and hygiene

Plan: `docs/superpowers/plans/2026-11-13-bi-follow-ups.md` (PR #527); this file holds only PR C's handoff.

## Handoff, PR C (developer, 2026-11-15)

Branch `fix/bi-alerts-hygiene`, from `origin/main` at `d72bf4ccd`.

### Commits

1. `BI-43` (b0a0d41b3): `GET /api/alerts/mine?chart=` (`RequiresAuth` in `POLICY_TABLE`) lists the caller's own alert-kind rules on one chart. Removing is the existing `DELETE /api/alerts`, which already let a person remove their own chart alert (`may_manage`). Console: "Your alerts on this chart" in the chart alert dialog, each row with a Remove button and a `ConfirmActionDialog`; `AlertRuleService.listMine`. Docs: `pages-per-role.md` Limits line.
2. `BI-29` (79d421320): the filter and click types move into `contracts/dashboards` and are exported from `@/services`; `DashboardService.filterValues` and `useFilterValues` on `useService`; `no-restricted-imports` for `@/services/clients/*` under `src/features` (`src/features/auth` exempt).
3. `BI-33` (eec190d35): second chart in the AI-15 case, three new XSS tooltip cases, corrected comment in `chart-option.ts`.

### Anchors checked at `d72bf4ccd`

- `BI-43`: `GET /api/alerts` needs `alert:read`/`alert:write` (`policy.rs`); `DELETE /api/alerts` is `RequiresAuth` and `may_manage` already allows the owner. So only the list was missing. No assistant tool change: `list_alert_rules` goes through `listed_rules`; `tool_schemas.json` untouched.
- `BI-29`: the recount is 24 files under `src/features/dashboards` (21 outside tests), all `import type` from `clients/bi-store`, plus `copilot/chart-draft-card.tsx`; `features/auth` has 3 value imports of `clients/auth` that `clients/auth.ts` documents as intended. `use-filter-values.ts` used `apiFetch` from `@/services/http` (not a `clients/*` import) in a hand-rolled effect. Counts differ from the backlog's 25, the intent holds.
- `BI-33`: the three leftovers were as described. In echarts 6.1 `encodeHTMLContent` is read only in `TooltipView._showComponentItemTooltip`.

### Commands run (all in the worktree)

- `bun install --frozen-lockfile` (scoped, 678 packages)
- `bun test src/features/dashboards/chart-alert-dialog.test.tsx src/services/clients/alerts.test.ts`: 15 pass, 0 fail (before adding the client test); `alerts.test.ts` alone after: 5 pass
- `bun test src/features/dashboards src/features/copilot`: 825 pass, 0 fail, 96 files
- `bun test src/services/clients/dashboards.test.ts`: 38 pass; `bun test src/features/dashboards/chart-option.test.ts`: 20 pass
- `bun run typecheck`: exit 0 (after BI-29 final edit)
- `npx eslint src/features`: 0 errors, 4 warnings (all four were already there: `sso-page.tsx`, `alerts-page.tsx`, `login-page.tsx`, `use-copilot.ts`). The new rule was shown to fire on a temporary file, which was deleted.
- `python3 -m unittest test_ai_eval` in `ops/ai_eval`: 204 tests OK
- `cargo fmt --check` in `rust/`: clean

### Not verified

- **Rust compile, clippy and tests were not run locally, by rule (shared VM).** Written by reading `alerts.rs`, `chart_alerts.rs` and the existing tests. New code: `alerts::mine`, the route in `routes/mod.rs`, the `POLICY_TABLE` row, three tests in `tests/chart_alerts.rs` (the file is already a module of `tests/it.rs`) and one in `tests/route_auth.rs`. CI must compile them. Points worth a look: the `?`/`.into()` conversions to `ApiResult` in `mine`; the `UPDATE role ... WHERE name = 'Chart Owner'` in a test; `route_auth` test expecting 200 from `/api/alerts/mine` on a fake that lists no rules.
- Full `bun run lint`, `bun run test` and `bun run build` (rule: not on this VM). Only `src/features` was linted; `src/lib` and `src/components` still import `clients/*` (outside the rule's scope, D7 names features).
- Console dialog look in a browser (the design rules: one line per row, no tooltip repeating a label, confirm before remove, success toast); only the unit tests ran.
- `python -m pytest` for dagster, docker, and the AI-15 live run of the changed case (needs a stack and a model).
