# Folders for uploaded files

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `DATA-35` |
| Kind | Feature |
| Spec | `docs/core/specs/data-35.md` (the *(proposed)* rows are decisions 2 to 6 below) |
| Status | Released: merged in #259 on 2026-10-28; accepted by the product owner on 2026-10-29 (six of seven checklist rows run in the browser, all Pass; `DATA-35-AC5` not run on the QA instance, covered by CI) |
| Priority | P2 *(planner's proposal)* |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Plan | `docs/superpowers/plans/2026-10-26-data-35-upload-folders.md` |
| Started | 2026-10-26 |
| Shipped | 2026-10-28 (#259) |
| Evidence | #259; the acceptance below |

## Problem

Sources → Uploaded files lists every upload of the tenant in one flat list
(`src/features/connectors/uploads-panel.tsx`). With many uploads (monthly
exports, one file per site) there is no way to group them, and a file is
found only by scrolling or by its name.

## What the user can do when this is done

1. See folders on the Uploaded files tab, with the files of the open folder,
   and a "No folder" group for files in none.
2. Choose a folder (or create one) while uploading, on the Table step.
3. Move one or several files to another folder later.
4. Create folders inside folders, rename them, and delete an empty one.
5. Search the tab (new: it has no search today) and find a file in any folder, with its folder shown.

## Not included

- Folders for connector tables (decision 7).
- Folders in the Data Explorer or the Catalog.
- Permissions per folder.
- Deleting a folder together with its files (decision 8).

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed
to the AI team in `AI-16`.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | What a folder is | Only a grouping on the Uploaded files tab. A file's table keeps its name (`bronze.<table>`); moving a file changes nothing a pipeline, chart, saved query, lineage or search reads | 2026-10-26 |
| 2 | When a folder is chosen | Optional on the upload's Table step, default "No folder", with "New folder" there; files can be moved at any time; a re-upload into an existing table joins that table's folder | 2026-10-26 |
| 3 | Folders inside folders | Allowed, at most 3 levels | 2026-10-26 |
| 4 | Who manages folders | The same permission as uploading (`connector:manage`). The whole Sources page already needs it, so whoever can open the tab manages its folders and nobody else sees them (corrected 2026-10-26 against `policy.rs`; the decision is unchanged) | 2026-10-26 |
| 5 | Whose folders | The tenant's: one set for everyone in it, never visible to another tenant | 2026-10-26 |
| 6 | Folder names | Unique among the folders that share a parent; renaming allowed | 2026-10-26 |
| 7 | What gets folders | Uploaded files only, for now; connector tables do not | 2026-10-26 |
| 8 | Deleting a folder | A folder may be empty and an empty one can be deleted; while a folder holds any file its delete control is disabled, with the reason | 2026-10-26 |

## Limits to tell a customer

- Folders group uploaded files on the Sources page only; they do not change
  where a table lives or what it is called.
- A folder holding files or other folders cannot be deleted; move its files out and delete its folders first.
- A folder's list and a search show the newest 100 files that match, the same window the tab always had; the window is applied after the folder or the text narrows the list, so a folder's files are not hidden by newer files elsewhere.
- A table remembers the folder of its latest load, and the Table step sends the folder it shows: choosing "No folder" puts that file in none, and the table forgets its folder. A load that names no folder at all (an API call without a `folderId`) joins the table's remembered folder instead. The Table step offers the folder of the newest listed upload of the table, so it can differ from the remembered one after files were moved.
- A tenant that still has files in folders cannot be deleted from the database (the folder rows are kept by the files that point at them); move the files out first.

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
A step not performed is never Pass.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-35-AC1` | Create a folder, a folder inside it; upload a file choosing the inner folder on the Table step | The file appears in the inner folder | Pass 2026-10-29 (product owner) |
| `DATA-35-AC2` | Move that file and another one to a third folder, then to "No folder" | Both follow; their tables, a chart reading one of them and its lineage are unchanged | Pass 2026-10-29 (product owner) |
| `DATA-35-AC3` | Re-upload into the first file's table | The new file joins the table's folder | Pass 2026-10-29 (product owner) |
| `DATA-35-AC4` | Look at a folder that holds a file; move the file out; delete the folder | The delete control is disabled with the reason while the file is in it; afterwards the folder is deleted | Pass 2026-10-29 (product owner) |
| `DATA-35-AC5` | As a user without `connector:manage`; then as a user of another tenant | The first is refused, as on the Uploaded files tab today; the second sees none of the folders | Not run: the QA instance had no second user or tenant (creating them was refused to the planner); covered by `tests/route_auth.rs` and the cross-tenant tests in CI |
| `DATA-35-AC6` | Search the tab for a file that is in a folder | Found, with its folder shown | Pass 2026-10-29 (product owner) |
| `DATA-35-AC7` | Create a fourth level of folders | Refused with the limit named (decision 3) | Pass 2026-10-29 (product owner) |

**Acceptor** (not the Owner): product owner (held in the base) **Date:** 2026-10-29 **Build:** PR #259 head `54588c4e` (its CI images, on a QA instance)

Exceptions, each with an owner and a date:

- `DATA-35-AC5` not run in the browser (see its row); no follow-up needed beyond CI, which runs on every pull request.

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
