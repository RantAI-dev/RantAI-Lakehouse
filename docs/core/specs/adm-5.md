# `ADM-5` Switch modules and connector types off for an installation

| | |
| --- | --- |
| Backlog | `ADM-5` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| Size | M (planner's estimate) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

The product owner asked for this on 2026-11-02, while planning QA for production: "if there is too much, we prioritise the connectors and hide the rest by env for now". A production install must not show a module, or a connector type, that has not yet passed QA.

Today the only switch is `NEXT_PUBLIC_SHOW_PREVIEW`. It is fixed when the console image is built, it hides only sidebar entries marked `preview`, and the API and the assistant keep serving everything. So the feature stays reachable by URL, by the command palette and by the assistant.

## What users get

An operator lists the modules and connector types to switch off in `.env` and restarts. Those features are gone from the sidebar, the command palette, the connector picker and the assistant's tools, and the API refuses them. Nothing is deleted, so switching a feature back on restores it as it was.

## Target specs

"Today" is `main` at `c5663c02`, read from the code.

| Capability | Today | Target |
| --- | --- | --- |
| What can be switched off | Sidebar entries marked `preview`, decided at image build | Each module in the plan's D2 table, and each connector type by name, decided at API start |
| Where it is hidden | Sidebar only | Sidebar, command palette, a direct URL (a "switched off" page), connector picker, assistant tool list, API |
| API on a switched-off route | Serves it | `404` with a fixed body, on **every** route of that module: 0 routes answer 2xx, asserted over `POLICY_TABLE` *(proposed)* |
| Assistant | Offers every tool | 0 tools of a switched-off module offered or runnable *(proposed)* |
| Connector of a switched-off type | — | Not offered and cannot be created. One that already exists stops loading, and its data stays |
| A wrong value in the env var | — | The API refuses to start and names the unknown key (fail closed) |
| Time to take effect | Image rebuild | One API restart, with no console rebuild *(proposed)* |

## Benchmark

- Metabase turns features off per instance in Admin settings, for example embedding, X-rays and uploads, and the API refuses a turned-off feature.
- Snowflake and Databricks switch features at the account or workspace level through parameters.

None of them needs a rebuild. Ours is set in `.env`, because an installed Lakehouse is configured there (REL-2).

## Acceptance checklist

On the feature page: `docs/core/features/switch-modules-off.md`.
