# Switch modules and connector types off for an installation

| | |
| --- | --- |
| Module | Administration & Security |
| Backlog | `ADM-5` |
| Kind | Feature |
| Spec | `docs/core/specs/adm-5.md` |
| Status | Planned |
| Priority | P1 (wanted before the first production install) |
| Owner | In the base |
| Acceptor | In the base (QA) |
| Plan | `docs/superpowers/plans/2026-11-02-adm-5-switch-modules-off.md` |
| Started | 2026-11-02 |
| Shipped | — |
| Evidence | — |

## Problem

A production install cannot hide a feature that has not passed QA. The only switch today, `NEXT_PUBLIC_SHOW_PREVIEW`, is fixed when the image is built and hides only some sidebar entries. The page still opens by its URL, the command palette still lists it, the assistant still uses it, and the API still serves it.

## What the user can do when this is done

1. An operator sets `LAKEHOUSE_DISABLED_MODULES=pipelines,reports` in `.env` and restarts the stack. Those modules are gone from the sidebar and the command palette.
2. A user who opens a switched-off page by its URL sees "This feature is switched off on this installation", not an error and not the page.
3. The assistant does not offer or run the tools of a switched-off module. If asked, it says the feature is switched off on this installation.
4. An operator sets `LAKEHOUSE_DISABLED_CONNECTOR_TYPES=Kafka,MongoDB`. Those types are gone from the connector picker, and creating one is refused.
5. A connector that already exists with a switched-off type stays listed, marked "switched off". It does not load and cannot be run, and its tables stay readable.
6. A Platform Admin sees, on Administration → Settings, which modules and connector types are switched off (read-only).
7. Taking a name out of the list and restarting brings the feature back, with all its data.
8. A mistyped name stops the API at start with a message that names it. Nothing is silently left on.

## Not included

- Switching per tenant, per role or per user. Roles already do that (RBAC). This switch covers the whole installation.
- A switch in the console. It is set in `.env` only, like the rest of an install's configuration (decision 4).
- Hiding Home, Data Explorer, Sources, Administration, Health or the audit log. These are the core of every install and cannot be switched off (decision 2).
- Deleting a switched-off module's data.

## Asking the assistant

The assistant's tools follow the switch (point 3). Its tool schema snapshot `tests/fixtures/tool_schemas.json` stays the full list; only the list offered to the model is filtered. The AI team reviews that part. The standard request set (`AI-15`) runs with nothing switched off, and once with `dashboards` switched off.

## Decisions

Until signed, the default is used.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Names of the env vars | `LAKEHOUSE_DISABLED_MODULES` and `LAKEHOUSE_DISABLED_CONNECTOR_TYPES`, comma-separated, unset = everything on | |
| 2 | Which modules can be switched off | `assistant`, `dashboards` (with maps), `reports`, `embed`, `query_studio`, `pipelines`, `uploads`, `governance`, `lineage`, `alerts`, `gold_exports`, `intelligence`. Not switchable: Home, Data Explorer and catalog, Sources, Administration, Monitoring health and audit | |
| 3 | What the API answers on a switched-off route | `404` with the fixed body `{"error":"module_disabled","message":"This feature is switched off on this installation."}`. A 404 does not confirm what the route would do, and it leaves the permission tests as they are | |
| 4 | Where the switch lives | `.env` only. A restart applies it, and there is no console toggle | |
| 5 | When the assistant module is off | Home shows the Data Explorer search instead of the chat, and the chat history page is switched off too | |
| 6 | When dashboards are off | Reports and embeds are off too, because they show dashboards | |
| 7 | A connector of a switched-off type that already exists | Listed and marked "switched off", it does not load, and its Bronze tables stay readable | |
| 8 | A wrong name in either list | The API does not start, and the message names the wrong value and lists the valid ones | |

## Limits to tell a customer

- The switch covers the whole installation and is set by the operator in `.env`. It does not replace roles.
- A switched-off module's data is kept. Data that was already loaded or published can still be read where its core module shows it, for example a Gold table in Data Explorer.

## Acceptance checklist

Run on a real deployment, with an operator who can edit `.env` and restart.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `ADM-5-AC1` | With nothing set, open every sidebar entry | Everything is there, as before | Not run |
| `ADM-5-AC2` | Set `LAKEHOUSE_DISABLED_MODULES=pipelines,reports`, restart, and sign in | Pipelines and Reports are gone from the sidebar and from the command palette (⌘K) | Not run |
| `ADM-5-AC3` | Open `/pipelines` by its URL | "This feature is switched off on this installation" | Not run |
| `ADM-5-AC4` | (operator) Call `GET /api/pipelines` with an admin session | `404` with the `module_disabled` body | Not run |
| `ADM-5-AC5` | Ask the assistant to create a pipeline | It says pipelines are switched off on this installation, and nothing is created | Not run |
| `ADM-5-AC6` | Set `LAKEHOUSE_DISABLED_CONNECTOR_TYPES=Kafka`, restart, and open Add connector | Kafka is not in the picker | Not run |
| `ADM-5-AC7` | (operator) `POST /api/connectors` with type `Kafka` | Refused with a message that the type is switched off | Not run |
| `ADM-5-AC8` | A Kafka connector made before the switch | Listed as "switched off", does not run on its schedule, and Run now is refused. Its tables still open in Data Explorer | Not run |
| `ADM-5-AC9` | Set `LAKEHOUSE_DISABLED_MODULES=pipelnes` (mistyped) and restart | The API does not start, and its log names `pipelnes` and lists the valid names | Not run |
| `ADM-5-AC10` | Administration → Settings, as Platform Admin | The switched-off modules and types are listed | Not run |
| `ADM-5-AC11` | Empty both vars and restart | Everything is back, with the same data | Not run |
| `ADM-5-AC12` | Set `LAKEHOUSE_DISABLED_MODULES=assistant` and open Home | Home shows the Data Explorer search, the chat is not there, and `/copilot` says switched off | Not run |
