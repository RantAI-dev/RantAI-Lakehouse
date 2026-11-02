# ADM-5: switch modules and connector types off for an installation

- **Status:** WAITS FOR the owner to sign the eight decisions on `docs/core/features/switch-modules-off.md`. Until then the defaults hold.
- **Base:** `main` at `c5663c02`. Every anchor below was verified there.
- **Branch:** `feat/adm-5-switch-modules-off`, off `main`. Planner and reviewer: Claude Opus. Developer: another agent.
- **Spec:** `docs/core/specs/adm-5.md`.

## 0. What is wrong, with evidence

- `NEXT_PUBLIC_SHOW_PREVIEW` (`src/components/app-shell/nav-config.ts:244`) is inlined at image build (`Dockerfile.frontend`). It hides only entries with `preview: true` (:55), and only in the sidebar, through `visibleNavGroups()` (:256).
- The command palette reads the raw `NAV_GROUPS` (`src/components/command-palette.tsx:88`).
- No route guard looks at modules. The API serves every route, and the assistant offers every tool its permissions allow (`routes/ai/registry.rs:2643`).
- There is no runtime feature switch anywhere: no `/api/config`, and no `FEATURE_*` variable.

## 1. Decisions

D1–D8 are the feature page's decisions 1–8. The planner's engineering decisions follow.

**D9. One owner of the module map.** A new Rust module, `lakehouse-api/src/modules.rs`, holds:
- `enum Module` (the D2 keys) and its `FromStr`, with the error that lists the valid names;
- `fn module_for_path(path) -> Option<Module>`, the API path prefixes per module (table below);
- `fn module_for_tool(name) -> Option<Module>`, one entry per tool in `registry::TOOLS`.

A unit test fails if a tool in `TOOLS` has no entry, so a new tool must be assigned. The console gets the list over the API and never keeps its own copy of the rules (rule 4). Its nav mapping is a `module?: ModuleKey` field on each `NavItem`.

| Module | API prefixes (from `POLICY_TABLE`) | Console routes |
| --- | --- | --- |
| `assistant` | `/api/ai` | `/` chat (D5), `/copilot`, `/copilot/history` |
| `intelligence` | `/api/agents` except `/api/agents/approvals*` | `/agents/employees`, `/agents/runs` |
| `dashboards` | `/api/dashboard`, `/api/maps` | `/dashboards`, `/dashboards/metrics` |
| `reports` | `/api/reports` | the reports entries |
| `embed` | `/api/embed`, `/api/public` | `/embed/*`, `/public/*` |
| `query_studio` | none (D10) | `/query-studio` |
| `pipelines` | `/api/pipelines` | `/pipelines` |
| `uploads` | `/api/uploads` | `/connectors/upload` |
| `governance` | `/api/governance/{policies,tag-keys,classification,data-quality}*`, `/api/agents/approvals*` | the same pages under `/governance`, and `/agents/approvals` |
| `lineage` | the lineage routes | `/lineage` |
| `alerts` | `/api/alerts` | `/alerts` |
| `gold_exports` | `/api/gold` | `/gold-exports` |

**D10. A shared prefix is not gated in the API.**
- `/api/query` serves dashboards, the assistant and Query Studio alike. So `query_studio` is switched off in the console only: no nav entry, and the URL shows the switched-off page.
- **T1 checks every other prefix in the table the same way.** If a console client of a module that stays on calls the prefix, the prefix moves to "console only", and the developer reports it under `### Developer report` rather than gating it.
- `dashboards` off also switches off `reports` and `embed` (D6). `Config` expands this once, at start.

**D11. Where the API refuses.** A layer `module_gate` is added in `routes/mod.rs` beside `auth_gate` (:1549), INSIDE it, so an unauthenticated call still gets 401 first.
- It reads `MatchedPath`, calls `module_for_path`, and answers D3's 404 when that module is off.
- `POLICY_TABLE` is untouched, so `tests/route_auth.rs` stays green with the default config.
- A new test uses `spin_up_with_env` with every module off and asserts that each gated prefix's routes in `POLICY_TABLE` answer 404 `module_disabled` to a platform admin. That is the spec's "0 routes answer 2xx".

**D12. Connector types** (`LAKEHOUSE_DISABLED_CONNECTOR_TYPES`, names exactly as in `connector_type.name`).
- `list_types` (`routes/connectors.rs:303`) drops them.
- `create` (:767) and `update` (:463) refuse them with 400 "This connector type is switched off on this installation." The assistant's `create_connector` calls the same handler.
- `list_ingestible` (:215) drops connectors of those types, so neither the Dagster sensor nor the metrics read them. Run now is refused the same way.
- The connector list marks such a connector with `switchedOff: true`.
- A name that is not in `connector_type` stops the API at start (D8). That check needs the database, so it runs in `main` after the pool is up, before the listener binds.

**D13. Assistant.**
- `tool_schemas_for` (`registry.rs:2643`) takes the switched-off set and drops the tools of a switched-off module. So do `prompt::select_tools` (`prompt.rs:909`) and the agents' list (`agents.rs:976`).
- `run_tool` (`ai/tools/mod.rs:132`) refuses such a tool by name, with the D3 message. That covers a model that names a tool it was not offered.
- `tool_schemas()` and `tests/fixtures/tool_schemas.json` stay the full list, so the fixture does not change. The copy of the existing operator switch at `ai/mod.rs:313` is the pattern to follow.
- The console Tools menu (`src/features/copilot/capabilities.ts:20`) hides groups whose module is off.

**D14. How the console knows.**
- `GET /api/auth/me` (`routes/auth.rs:340`, `MeResponse` :296) gains `disabledModules: string[]` and `disabledConnectorTypes: string[]`, which are empty when nothing is off. It is read on every page load already, so there is no new endpoint and no build-time variable.
- `visibleNavGroups()` and the command palette filter by `item.module`.
- A shared `ModuleGate` component wraps the switched-off routes' layouts and renders the switched-off page.
- The picker drops hidden types; the server has already dropped them.
- Administration → Settings (`src/app/(admin)/admin/settings/page.tsx`) shows the two lists, read-only.

**D15. Dagster.** It needs no change for connectors, because of D12. Module-level schedules that call a switched-off module's API get 404 `module_disabled`:
- pipelines (`authored_factory.py:822`, :1416);
- alerts (`alerts_run.py:140`);
- gold exports (`gold_export.py:1098`);
- agents (`agent_runs.py:356`).

Each one treats that answer as "nothing to do": it logs once with the reason and records the tick as skipped, not failed. That is the `skipped_verbs` pattern, because a switched-off module is degraded, not a broken config. Each gets a unit test with a mocked 404.

## 2. Anchors (verified at `c5663c02`)

- **Console:**
  - `src/components/app-shell/nav-config.ts:55`, `:113`, `:244`, `:256`
  - `src/components/command-palette.tsx:88`
  - `src/features/connectors/connector-type-picker.tsx`
  - `src/features/copilot/capabilities.ts:20`
  - `src/app/(admin)/admin/settings/page.tsx`
- **API core:**
  - `rust/crates/lakehouse-api/src/policy.rs:147` (`POLICY_TABLE`), `:1091` (`policy_for`), `:1156` (`auth_gate`)
  - `routes/mod.rs:1151` (`router`), `:1549` (the `auth_gate` layer)
  - `config.rs:227` (`Config`), `:1267` (`parse_switch`), `:1418` (`from_map`)
- **API features:**
  - `routes/auth.rs:296`, `:340`
  - `routes/connectors.rs:215`, `:303`, `:463`, `:767`
  - `routes/ai/registry.rs:49`, `:1860`, `:2643`
  - `routes/ai/prompt.rs:909`
  - `routes/ai/mod.rs:313`, `:549`
  - `routes/ai/tools/mod.rs:132`
  - `routes/agents.rs:976`
- **Tests:** `rust/crates/lakehouse-api/tests/route_auth.rs`; `tests/common/mod.rs` (`spin_up_with_env`)
- **Dagster:** `dagster/dispar_orchestrate/{authored_factory,alerts_run,gold_export,agent_runs}.py`
- **Config files:** `docker-compose.yml` (the `lakehouse-api` `environment:` block); `.env.example` ("Optional features" section); `README.md` Configuration table

## 3. Tasks (one commit each, in order; the Rust tasks sit together)

1. **T1. The module map and the config** (`modules.rs`, `config.rs`).
   - Parse both variables. Unknown module → `ConfigError` naming it and listing the valid ones. Apply D6's expansion.
   - Check D10's shared-prefix rule against the console clients (`src/services/clients/*`). Write the final table into `modules.rs`'s module doc.
   - *Accept:* unit tests for parsing (empty, two names, a mistyped name, whitespace, a duplicate) and for every tool in `TOOLS` having a module.
2. **T2. The API gate** (`module_gate`, D11).
   - *Accept:* the new `route_auth`-style test (every gated route → 404 with everything off) and a test that one module off leaves another module's routes alone. The existing `route_auth.rs` passes unchanged.
3. **T3. Connector types** (D12), including the start-up name check.
   - *Accept:* `sqlx::test`s for list, create, update, ingestible and Run now with one type off.
4. **T4. Assistant** (D13).
   - *Accept:* tests that a switched-off module's tools are not offered and that `run_tool` refuses one by name. `tests/fixtures/tool_schemas.json` is unchanged (`git diff --exit-code`).
5. **T5. `/api/auth/me`** (D14, API half).
   - *Accept:* a test that it returns both lists.
6. **T6. Dagster** (D15).
   - *Accept:* the unit tests, `(cd dagster && python -m pytest dispar_orchestrate -q)`.
7. **T7. Console** (D14).
   - `module?` on the nav items, the filter in the sidebar and the palette, `ModuleGate` with the switched-off page, the picker, the Tools menu, and the Settings rows. D5: Home without the assistant shows the Data Explorer search.
   - *Accept:* vitest for the filter, the gate and the picker; `bun run typecheck && bun run lint && bun run test`.
8. **T8. Compose and docs.**
   - `LAKEHOUSE_DISABLED_MODULES: ${LAKEHOUSE_DISABLED_MODULES:-}` and the connector line in `docker-compose.yml`, each with a why-comment.
   - `.env.example` entries in the "Optional features" section, and the README table.
   - The feature page's Status.
   - *Accept:* `docker compose --profile '*' config --quiet`, and a real `docker compose up` of the default profile with `LAKEHOUSE_DISABLED_MODULES=pipelines` showing AC3 and AC4 (rule 8). Quote the output in the handoff.

## 4. PR slicing

One PR, because the API, the console and Dagster must change together to be honest. A console that hides a page the API still serves is the problem this item exists to fix.

## 5. Out of scope

- Per-tenant or per-role switches.
- A console toggle.
- Deleting data.
- Switching off the core modules (decision 2).
- Changing `NEXT_PUBLIC_SHOW_PREVIEW`. It stays as it is; the two filters combine.

## 6. Handoff (developer appends below)

## 7. Review (planner appends below)
