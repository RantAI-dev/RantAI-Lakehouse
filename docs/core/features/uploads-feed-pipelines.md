# Uploaded tables feed pipelines and dashboards

| | |
| --- | --- |
| Module | Data |
| Backlog | `DATA-19` |
| Kind | Feature |
| Spec | `docs/core/specs/data-19.md` (no *(proposed)* numbers) |
| Status | Released (accepted 2026-10-23; PR #165) |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | `docs/superpowers/plans/2026-10-14-data-19-uploads-feed-pipelines.md` (PR #165). Nothing open blocks it (`BLD-3` shipped). |
| Started | 2026-10-14 |
| Shipped | 2026-10-15 |
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

## A connector's tables feed pipelines

This page is the closest one for `BLD-7`, `BLD-8` and `BLD-9` (a connector's
tables feeding a pipeline, the same way an upload does).

A connector on a pipeline says **where the pipeline's data came from** — it is
never a second way to read the source (`DEC-15`). So:

- *Create pipeline* on a connector's page opens the form with the source zone
  fixed to `bronze` and *Source table* listing that connector's ingest targets
  (the tables its ingest job actually wrote). Creating or editing a pipeline
  with a connector and any other table is refused, with one sentence that names
  the tables to pick; a connector that has not loaded anything yet says to run
  its ingest first.
- The pipeline runs after that table's load: `ingest_job` is in `dependsOn`, on
  by default (the form shows a switch and the user may turn it off). The sensor
  counts only the ingest runs that loaded this pipeline's own Bronze table, so
  another connector's load does not start it. (`BLD-4`.)
- Editing a pipeline can change or clear its connector; *No connector* is a
  real choice and clears the stored one. (`BLD-9`.)
- A failed run shows the reason our own code raised (`config rejected: …`,
  `refused: …`). Any other engine or driver text is not shown as the reason:
  the page says "The run failed; see the run log" and the log is one click
  away. (`BLD-8`.)

Limits worth saying out loud:

- There is no direct read from a source into Silver. A pipeline with a
  connector reads one of that connector's Bronze tables, and nothing else.
- A pipeline stored before this rule (a connector on a non-Bronze source) fails
  its run with a fixed message that says to edit it; it is never run against
  the source directly.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Who builds it | The Build lane, with `BLD-3` | Product owner, 2026-10-14 |
| 2 | Re-upload trigger | Only a re-upload of the pipeline's own source table, behind a switch shown for Bronze sources, ON by default | Planner, plan 2026-10-14 D1 and D3 |

## Limits to tell a customer

- A re-upload runs the downstream pipeline once per upload.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-19-AC1` | Upload a file; build a Silver table from it; build a Gold table from that; chart it | All from the console; the chart shows the file's data | Pass 2026-10-23 (product owner) |
| `DATA-19-AC2` | Re-upload the file with one row changed | The downstream pipeline runs; the chart shows the change | Pass 2026-10-23 (product owner) |
| `DATA-19-AC3` | Open lineage for the Gold table | The upload is shown as the origin | Pass 2026-10-23 (product owner) |
| `DATA-19-AC4` | Delete the upload (the file), then run the pipeline | The table and the pipeline keep working; lineage shows the origin as "deleted upload" | Pass 2026-10-23 (product owner) |
| `DATA-19-AC5` | Upload a file into a different table | The pipeline does not run again | Pass 2026-10-23 (product owner) |
| `DATA-19-AC6` | Switch "Run again after this table is re-uploaded" off; re-upload | The pipeline does not run | Pass 2026-10-23 (product owner) |
| `DATA-19-AC7` | As a user without the permission, pick the uploaded table as a source | Refused with an honest message | Pass 2026-10-23 (product owner) |

**Acceptor** (not the Owner): product owner (held in the base) **Date:** 2026-10-23 **Build:** main 0e5498d2

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [x] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [x] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
