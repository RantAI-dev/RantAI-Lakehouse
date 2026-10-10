# Semantic layer

| | |
| --- | --- |
| Module | Data |
| Backlog | `AI-16` |
| Status | In build |
| Plan | Plan files are not kept in the repository. |

## Problem

A `serving` or `silver` table built by a pipeline, or from any source that gives no column descriptions, has no description in the chat's data map, and the chat cannot tell what its columns mean. Tables from sources that do include descriptions carry them, but they may be outdated or not cover what the data means in this deployment.

## What the user can do when this is done

1. Ask the copilot a question about a table using a column's synonym and get an answer.
2. Wait for a drafting pass and see a `silver` or `serving` table's automatic descriptions within that pass.
3. See the table's automatic descriptions through `GET /api/semantic/{table}`.
4. Write a description of a table or column and confirm it through `PUT /api/semantic/{asset}`, replacing any draft.
5. The chat uses a confirmed text or a draft when it writes SQL; a person's text wins over a draft, and a draft never overwrites a person's text.

## Not included

- Tables in the `raw` layer, such as an uploaded file, are not described because the chat does not read them.
- A Catalog page to manage descriptions.
- Choosing which tables get drafts; a table is drafted once and left alone after.
- A draft for a column added to a table after the table's first draft.
- A request to re-draft a table.
- A route to delete an entry.
- How tables relate, or choosing which tables a question should search.

## Asking the assistant

Yes. The user asks the copilot about a table by name or by using a synonym the copilot learned.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Storage is a new table, `semantic_entry`, not `asset_annotation` | none | Owner, 2026-10-07 |
| 2 | A draft is written by the deployment's model in the background, with no request from anyone, and the chat reads it at once | on | Owner, 2026-10-07 |
| 3 | A person's text always wins | none | Owner, 2026-10-07 |
| 4 | The draft's language is the deployment's `AI_DEFAULT_REPLY_LANGUAGE` | English | Owner, 2026-10-07 |
| 5 | The switch is `AI_SEMANTIC_LAYER` | on | Owner, 2026-10-07 |

## Limits to tell a customer

- One deployment has one description per table and column, not one per tenant.
- The background pass drafts 10 tables per pass, `serving` first, then the rest.
- A table is drafted once at boot or when the background pass runs; a draft for a column added later is not created.
- Descriptions are at most 400 characters for a table, 200 for a column.
- Synonyms are at most 6, each 1 to 40 characters.
- A column can be marked with one of six roles, `measure`, `dimension`, `time`, `key`, `flag` or `non_additive`, or left unset. The chat reads a role from a draft as well as from a confirmed entry.
- A `flag` column holds 0/1 or yes/no values, and a `non_additive` column holds a number that must not be added up across rows, such as a distinct count, an average, a rate, a percentage or a price. In the chat's data map, a `flag` column is marked `[0/1 flag]` and a `non_additive` column `[never SUM across rows]`, and the table's summary of measures leaves out any column with a `flag`, `non_additive`, `key`, `dimension` or `time` role. A `flag` or `non_additive` column that would otherwise be a measure (a number whose name does not look like a key, such as a year, a month or an id) is also not part of the row's grain. Any other `flag` or `non_additive` column, such as a text or boolean one, still is. When the table has a `non_additive` column, the summary says which columns are not additive, so the chat does not total them.
- When the switch is on, which is the default, the chat's system prompt also says that a count in a table grouped by several columns can overlap between rows, so the copilot counts distinct things in a detail table or gives a figure per row, and does not add the count up.
- When the switch is off, the chat reads neither the semantic entries nor the table descriptions saved in the Catalog, and the drafting pass does nothing. The three `/api/semantic` routes still answer.

## Acceptance checklist

Run on a running deployment. Mark each Pass, Fail, or Not run with the reason. A step not performed is never Pass. In rows 1 to 14, `<asset>` is a qualified table name, such as `silver.orders` or `serving.dashboard_users`, picked in row 1.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Run a pipeline that creates a new `silver` or `serving` table, or restart the API after a table has been created. Then `GET /api/semantic` and pick that table (it is not listed until a pass has run). | The table is in the list. | |
| 2 | Restart the API, wait fifteen minutes, or finish a pipeline run to trigger the next drafting pass. Then `GET /api/semantic/<asset>` for that table. | Response lists entries for the table and each column, each with `status: "draft"`, `model: "<name>"`, and `writtenBy: null`. | |
| 3 | Ask the Copilot about that table using a word that is a synonym from its draft and not a column name. | The assistant answers from the table. | |
| 4 | In Query Studio's "Natural language" box, ask a question about the same table. | The answer shows formatted text (no literal `**` or `<span>` tags), with no row count when no query ran. | |
| 5 | `PUT /api/semantic/<asset>` with body `{"description": "my text"}`. | Response is 200 with `{"ok": true}`. | |
| 6 | `GET /api/semantic/<asset>` again. | Response shows the confirmed entry with the same description, `status: "confirmed"`, `writtenBy: <user>`, `model: null`. | |
| 7 | Set `AI_SEMANTIC_LAYER=false` and restart the API (operator). | The API starts. | |
| 8 | `GET /api/semantic` and `GET /api/semantic/<asset>`. | Both routes answer with existing entries; no new drafts are created. | |
| 9 | Set `AI_SEMANTIC_LAYER=maybe` and restart the API (operator). | The API stops with a `ConfigError` naming the accepted values. | |
| 10 | Call `PUT /api/semantic/serving.no_such_table` with a body. | Response is 404 with the table not found in `serving` or `silver`. | |
| 11 | Call `PUT /api/semantic/<asset>` with body `{"description": "..."}` (401-character description). | Response is 400 naming `description`. | |
| 12 | Call `PUT /api/semantic/<asset>` with body `{"column": "unknown", "description": "text"}`. | Response is 400 naming the unknown column. | |
| 13 | Call `PUT /api/semantic/<asset>` with body `{"column": "<a real column>", "description": "x", "synonyms": ["a", "b", "c", "d", "e", "f", "g"]}` (seven synonyms) or `{"column": "<a real column>", "description": "x", "role": "bogus"}` (unknown role). | Response is 400 naming `synonyms` or `role`. | |
| 14 | A user with `catalog:read` only calls `PUT /api/semantic/<asset>`. | Response is 403. | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
