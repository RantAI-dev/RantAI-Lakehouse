# A BI and assistant only installation

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-44` |
| Kind | Feature |
| Spec | `docs/core/specs/bi-44.md` |
| Status | Building (slices 1–3 handed off; slice 4 console and installer built) |
| Priority | P2 |
| Owner | The module's owner, in the base. |
| Acceptor | The product owner. |
| Plan | `docs/superpowers/plans/2026-11-24-bi-44-bi-assistant-only-install.md` |
| Started | 2026-11-24 |
| Shipped | |
| Evidence | |

## Problem

A customer who already has a database should not have to copy it into the
lakehouse before they can use dashboards, Query Studio or the assistant.
Running the whole stack for that is heavy: the warehouse services and a
second query engine use memory on the same host even when nobody reads
from them.

## What the user can do when this is done

1. Install only the API, the console, the application database, one query
   engine and the PDF renderer, using one of the two standalone Compose
   files in `ops/bi-only/`.
2. Point that engine at their own tables (source list in the spec). Nothing
   is copied into the lakehouse; the full stack is untouched.
3. See the installation mode and the active engine in
   `GET /api/settings/deployment`, and see the other engine and the
   warehouse-only services reported as "unknown" / "skipped" rather than
   as broken.
4. Run a query in Query Studio without picking an engine: it uses the
   installation's engine. Asking explicitly for the other engine is
   refused with a sentence naming the configured engine, and nothing is
   dialed.
5. Later slices register external catalogs, keep masks and row filters on
   their columns, run dashboard tiles and SQL sources over them, and let
   the assistant use them.
6. Install with one command:
   `lakehousectl install --mode bi-only --engine clickhouse|trino`. It
   writes `LAKEHOUSE_INSTALL_MODE`, `LAKEHOUSE_QUERY_ENGINE` and exactly
   one engine URL into the install's mode-0600 `.env`; `--mode full`
   (the default) keeps the old byte-for-byte output. See
   `ops/bi-only/README.md`.
7. See the mode and the engine on Admin > Settings > Deployment, as the
   deployment route reports them. In a `bi-only` install the console also
   lists the external source types the engine can read in place and says
   that nothing is copied into the lakehouse. With that mode it hides the
   warehouse-only sidebar pages (Pipelines and the Gold exports entry)
   and offers only the installed engine in Query Studio's engine picker.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| D1 | One query engine per BI-only installation, chosen at install time; the two never run side by side. Owner decision 2026-11-11 | No default engine beyond ClickHouse's existing one | Owner, 2026-11-11 |
| D2 | Installation mode is `full` (existing stack) or `bi-only`; an unknown value stops the API at start | `full` | Planner; owner to confirm |
| D3 | An explicit request for the inactive engine is a 400 that names the configured engine, before any dial | Fail closed | Planner; owner to confirm |
| D4 | Services that are not deployed report `checked: false` / `"skipped"`, never "unhealthy"; full mode keeps its three readiness checks | Honest absence | Planner; owner to confirm |
| D5 | `installMode` and `queryEngine` are additive top-level fields of the deployment route | Present in both modes | Planner; owner to confirm |
| D6 | Standalone Compose files live in `ops/bi-only/`; the root `docker-compose.yml` keeps its current behavior | Separate files | Planner; owner to confirm |
| D7 | Memory budget is a structural target (one engine process) until slice 4 measures peak RSS; no number is claimed as measured here | *(proposed)* | Planner; owner to confirm |
| D8 | Source coverage: external engines for ClickHouse, federated catalogs for Trino, per the spec's matrix | Both matrices | Owner's decision recorded in BACKLOG; owner to confirm the list |

## Limits

- Slice 1 does not register external catalogs or schemas. It prepares the
  mode, routing, reporting and deployment shape.
- The federated coordinator is not published on the host; its catalog
  properties are operator-supplied and read-only mounted. No credential is
  committed to this repository.
- A warehouse-only route in a BI-only installation answers from its own
  guard (503 / unsupported), not by dialing a service that was not
  installed.
- Peak memory numbers are unmeasured until the slice 4 gate. No
  `docs/plans/BI-44-RESULT.md` measurement exists yet; the structural target
  is one engine process, not a measured RSS figure.
- `GET /api/settings/deployment` needs `settings:write`. The console reads
  it only for a holder of that permission, so the deployment card, the
  hidden sidebar pages and the one-engine picker are available to an
  administrator. Other roles get the unchanged menu and both engine
  choices; the API still refuses an inactive engine at run time, and
  warehouse-only pages answer honestly from their own guard. Fetching the
  route for everyone would mean a 403 on every page load (`WEB-11`,
  `ADM-13`).
- `lakehousectl install --mode bi-only ...` writes the mode variables into
  the install's `.env`; it does not change the release's root Compose file.
  To deploy only the seven named services, use the standalone files in
  `ops/bi-only/` as described there. A full-mode install keeps its exact
  former `.env` bytes.
- Slice 3 moves unsaved dashboard SQL-source preview and the assistant's
  `run_sql` / dataset tools to the active engine. It does **not** add a
  metadata store: saved SQL sources, charts and boards still live in
  `ClickHouse` tables, which a `bi-only+trino` install does not deploy. Those
  persistence routes answer a fixed 503 naming the missing store instead of
  dialing it; only a preview over unsaved SQL works there. Inline chart
  previews are refused honestly for the same reason — the chart builder
  still emits `ClickHouse` `SETTINGS` text. No persistence is fabricated.
- The assistant tool schemas and
  `tests/fixtures/tool_schemas.json` are unchanged by slice 3: the engine
  context is appended to the system prompt only in `bi-only` mode, so every
  `AI-32` full-mode prompt pin stays byte-identical.

## Acceptance checklist

- [ ] `BI-44-AC1` A fresh install keeps `full`/`clickhouse` behaviour; bad mode or engine values and a both-engines config stop the API at start.
- [ ] `BI-44-AC2` An omitted engine runs on the configured one; an explicit inactive engine is refused with a 400 and no dial.
- [ ] `BI-44-AC3` Masks and row filters still apply to external tables in the active engine's dialect; unparseable SQL is refused.
- [ ] `BI-44-AC4` Services view and readiness report only the chosen engine as checked; absent services are unknown/skipped.
- [ ] `BI-44-AC5` `GET /api/settings/deployment` shows `installMode` and `queryEngine`, and the console displays them.
- [ ] `BI-44-AC6` Both standalone files start exactly seven services with pinned images matching the root file; their static suite passes.
- [ ] `BI-44-AC7` *(proposed)* one engine process and no warehouse services, with peak RSS measured against `full` in slice 4.

## Asking the assistant

The assistant is a first-class user of this installation, not an after-
thought. Slice 3 plans the tool schemas (regenerate
`tests/fixtures/tool_schemas.json`), the context the dashboard page sends,
and the standard request set (`AI-15`) so the assistant can ask questions
over an in-place source through the active engine. Until that slice ships,
the assistant does not claim access to an external source and the
deployment answer reports the mode honestly.
