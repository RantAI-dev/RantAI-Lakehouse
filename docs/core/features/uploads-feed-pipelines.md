# Uploaded tables feed pipelines and dashboards

| | |
| --- | --- |
| Module | Data |
| Backlog | `DATA-19` |
| Kind | Feature |
| Spec | `docs/core/specs/data-19.md` (no *(proposed)* numbers) |
| Status | Planned |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | `docs/superpowers/plans/2026-10-14-data-19-uploads-feed-pipelines.md` (PR #165). Nothing open blocks it (`BLD-3` shipped). |
| Started | |
| Shipped | |
| Evidence | |

## Problem

An uploaded file lands in a raw table, but that table cannot be the source of
a pipeline, so it never reaches Silver, Gold or a dashboard.

## What the user can do when this is done

1. Choose an uploaded table as a pipeline's source, like any other table.
2. Build a Silver and then a Gold table from it, and chart the Gold table.
3. Re-upload the file (replace or append) and have the downstream pipeline run, as after a connector load.
4. See in lineage that the table came from an upload.

## Not included

- Anything not in the spec's target table.

## Asking the assistant

The assistant's `create_pipeline` can name an uploaded table as its source
once the builder can; no new tool. AI work for the Data module is the AI
team's (`AI-16`).

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Who builds it | The Build lane, with `BLD-3` | Product owner, 2026-10-14 |

## Limits to tell a customer

- A re-upload runs the downstream pipeline once per upload.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-19-AC1` | Upload a file; build a Silver table from it; build a Gold table from that; chart it | All from the console; the chart shows the file's data | Not run |
| `DATA-19-AC2` | Re-upload the file with one row changed | The downstream pipeline runs; the chart shows the change | Not run |
| `DATA-19-AC3` | Open lineage for the Gold table | The upload is shown as the origin | Not run |
| `DATA-19-AC4` | Delete the upload while a pipeline reads it | Refused while the pipeline depends on it, with a clear message | Not run |
| `DATA-19-AC5` | As a user without the permission, pick the uploaded table as a source | Refused with an honest message | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
