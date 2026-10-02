# Publish Gold tables in open format, per table

| | |
| --- | --- |
| Module | Build (shown on the table's page in Data); touches Monitoring and Administration |
| Backlog | `DATA-1` |
| Status | In build. Decisions 1–4 use defaults, not yet signed |
| Plan | `docs/superpowers/plans/2026-10-02-gold-publish-per-mart.md` |

## Problem

Curated (Gold) tables are stored in the analytics engine's own format, which
only that engine can read. A copy in open Iceberg format is what lets other
tools read them. Today that copy needs someone to press "Export now" on a
separate page: the nightly job that should do it cannot sign in and has
never succeeded, and it only covers tables named in a configuration file.

Nothing inside the console uses the open copy. It exists for outside tools.

## What the user can do when this is done

1. On a Gold table's page, switch on "Publish in open format (Iceberg)". It
   is off by default.
2. See the table published automatically after its pipeline succeeds,
   without pressing anything.
3. See on the table's page: last published time, the snapshot ID, and one of
   "Up to date", "Out of date", "Not measured", "Never published".
4. Press "Publish now" to repair a failed or stale publish.
5. See a failed publish with its reason. It is never shown as published.
6. Switch publishing off. Updates stop; the published table stays.
7. See all tables and their publish state from one overview page.
8. No longer see "Exports" in the Build menu.

A user without permission sees the switch and the state, but cannot change
them.

A table that has not changed is not copied again.

## Not included

- Publishing Silver or raw data (raw is already open).
- Removing old copies from a published table.
- Counting who reads published tables.
- Storing Gold in open format first. That is a later project (`DATA-3`).
- File downloads. This is not a CSV or Excel export.

## Asking the assistant

Not in this version. The assistant already has a Gold export tool; whether
it should also switch publishing on or off is left for a follow-up, since
that changes who can read the data.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Who may switch publishing on or off | Holders of the existing `gold:export` permission. Among the built-in roles only Platform Admin has it; Data Engineer does not | |
| 2 | What switching off does | Stops updates; keeps the published table | |
| 3 | After upgrading | Publishing is off for every table until switched on. An install that listed tables in configuration must switch them on or keep the list set | |
| 4 | Acceptable delay from pipeline finishing to "Up to date" | Not set. 10 minutes is a proposed starting point, not a measurement | |

## Limits to tell a customer

- Each publish of a changed table adds a full copy, stamped with an export
  time. Old copies stay. Outside tools must select the latest export time or
  they will see every past copy. Storage grows by one copy per publish.
- Tables over 5,000,000 rows are refused, not cut short. An operator can
  raise the limit.
- The open copy is current only after the publish that follows a pipeline
  run.
- If the system cannot tell when a table last changed, the page says "Not
  measured" and the table is published on every run.
- Masking is applied when the copy is made. Later policy changes do not
  alter copies already published.
- Automatic publishing needs the operator to set the export run token.
  Without it, only "Publish now" works.

## Acceptance checklist

Screen labels are as specified in the plan. If the built screen differs,
record it as a finding.

**Before starting:** the orchestrator is running; the operator has set the
export run token (operator); there is a Gold table fed by a pipeline you can
run; you have a Platform Admin login and an Analyst login; an operator can
read Iceberg tables from a second tool (operator).

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | As Platform Admin, open the Build menu | No "Exports" item | |
| 2 | Open the Gold table in Data | The publish switch is shown, off | |
| 3 | Open a Silver or raw table | No publish switch | |
| 4 | Switch publishing on; reload | Switch stays on; status "Never published" | |
| 5 | Press "Publish now" | Completes; a time and snapshot ID appear | |
| 6 | (operator) Read the table from a second tool; count rows with the latest export time | Equals the row count shown in the console | |
| 7 | Run the pipeline with input that changes the table. Do nothing else. Reload each minute | Time and snapshot change; status "Up to date". Record the delay: ____ min | |
| 8 | Run the pipeline again with no change. Wait the same delay plus five minutes | Time and snapshot do not change; no new entry in publish history | |
| 9 | Leave it running overnight | No failed publish recorded by morning | |
| 10 | (operator) Force a failure, e.g. lower the row limit below the table's size; press "Publish now" | A failure with a reason; status does not say up to date; previous time unchanged | |
| 11 | (operator) Restore the setting; press "Publish now" | Succeeds | |
| 12 | As Analyst, open the same table | Switch visible but not changeable, with an explanation; status readable | |
| 13 | As Analyst, look for "Publish now" | Not available, or refused with a clear message | |
| 14 | As Platform Admin, switch publishing off; reload | Stays off | |
| 15 | (operator) Read the table from the second tool | Still there, same rows as before step 14 | |
| 16 | Run the pipeline with a change; wait the delay plus five minutes | Nothing new published | |
| 17 | Open the overview of all tables | This table shows as not enabled | |
| 18 | Open a Gold table never published, switch off | "Never published"; no invented time or count | |

**Accepted** when 1–9 and 12–17 pass, and 10, 11, 18 pass or have an agreed
exception.

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md`: `DATA-1` moved to Done; `DATA-2`, `DATA-6` kept as follow-ups
- [ ] `CHANGELOG.md` entry a customer can read
