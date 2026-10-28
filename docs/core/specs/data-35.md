# `DATA-35` Folders for uploaded files

| | |
| --- | --- |
| Backlog | `DATA-35` in [BACKLOG.md](../BACKLOG.md) |
| Module | Data |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Blocked by | Nothing open |

## Why

Sources → Uploaded files is one flat list of every upload of the tenant. A
team that uploads many files (monthly exports, one file per site) cannot
group them, so the list grows into a long scroll and a file is found only by
its name. Asked for by the product owner on 2026-10-26.

## What users get

Uploaded files grouped in folders on the Uploaded files tab: a folder can be
chosen while uploading, files can be moved between folders later, and empty
folders can be created and removed.

## Decisions (product owner, 2026-10-26)

- A folder only groups uploads on the Uploaded files tab. It is not a place
  in the lakehouse: a file's table keeps its name (`bronze.<table>`), and
  moving a file between folders changes nothing a pipeline, a chart, a saved
  query, lineage or search reads.
- Only uploaded files have folders, for now. Connector tables do not.
- A folder may be empty. An empty folder can be deleted; deleting a folder
  that still holds files is not possible (the delete control is disabled
  while it holds any).

## Target specs

"Today" is `main` at `3040000d`, read from the code, not tested. A target is
either a competitor's documented number (named under Benchmark) or marked
*(proposed)*: the planner's number, which the product owner confirms or
changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Grouping | One flat list (`src/features/connectors/uploads-panel.tsx`) | Folders on the Uploaded files tab, with the files of the open folder listed and a "No folder" group for files in none |
| Choose a folder when uploading | Not possible | Optional on the upload's Table step; the default is "No folder" *(proposed)*; a new folder can be created there |
| Re-upload into an existing table | n/a | The file joins the folder the table's earlier uploads are in *(proposed)* |
| Move | n/a | Move one or several files to another folder or to "No folder" |
| Folders inside folders | n/a | Allowed, at most 3 levels *(proposed)* |
| Rename | n/a | A folder can be renamed; names are unique among the folders that share a parent |
| Delete | n/a | Only an empty folder; the delete control is disabled, with the reason, while it holds files |
| Who | Every upload route and the whole Sources page need `connector:manage` (`policy.rs:805-826`, `:663`) | The folder routes need the same permission *(proposed)*: whoever can open the tab manages its folders; anyone else is refused, as for the tab today |
| Scope | Uploads belong to a tenant | Folders belong to a tenant, the same set for everyone in it *(proposed)*; never visible to another tenant (`SEC-16`) |
| Search | None: the tab is a plain table of the newest 100 uploads (`uploads-panel.tsx`, `LIST_LIMIT` in `routes/uploads.rs`) | A search box on the tab that covers every folder, each result showing its folder |

## Benchmark

Not checked against competitors' documentation for this spec. Databricks
volumes and Snowflake stages hold uploaded files under path prefixes the user
chooses, which is a location; this feature is deliberately a grouping only
(first decision above).

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `DATA-35-AC1` Create a folder, a folder inside it, and upload a file into the inner one from the Table step: the file appears there
- `DATA-35-AC2` Move that file and another one to a third folder, then to "No folder": both follow; their tables, a chart reading one of them and its lineage are unchanged
- `DATA-35-AC3` Re-upload into the first file's table: the new file joins the table's folder
- `DATA-35-AC4` A folder holding a file: its delete control is disabled and says why; after the file is moved out, the folder can be deleted
- `DATA-35-AC5` A user without `connector:manage` is refused on the folder routes, as on the tab today; a user of another tenant sees none of the folders
- `DATA-35-AC6` Search the tab for a file in a folder: it is found, with its folder shown

## Not included

- Folders for connector tables
- Folders in the Data Explorer or the Catalog
- Permissions per folder
- Deleting a folder together with its files

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
