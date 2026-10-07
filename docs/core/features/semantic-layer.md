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
- A column can be marked as a `measure`, `dimension`, `time` or `key` role, or left unset.
- When the switch is off, the three `/api/semantic` routes still answer, but no drafts are created.

## Acceptance checklist

Run on a running deployment. Mark each Pass, Fail, or Not run with the reason. A step not performed is never Pass.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | `GET /api/semantic` and pick a `silver` or `serving` table that has no entry (missing from the list). | The table is not listed. | |
| 2 | Wait for the next drafting pass at API start, or after fifteen minutes, or after a pipeline run. Then `GET /api/semantic/serving.<table>` or `GET /api/semantic/silver.<table>` for that table. | Response lists entries for the table and each column, each with `status: "draft"`, `model: "<name>"`, and `writtenBy: null`. | |
| 3 | Ask the Copilot about that table using a word that is a synonym from its draft and not a column name. | The assistant answers from the table. | |
| 4 | In Query Studio's "Natural language" box, ask a question about the same table. | The answer shows formatted text (no literal `**` or `<span>` tags), with no row count when no query ran. | |
| 5 | `PUT /api/semantic/serving.<table>` with body `{"description": "my text"}`. | Response is 200 with `{"ok": true}`. | |
| 6 | `GET /api/semantic/serving.<table>` again. | Response shows the confirmed entry with the same description, `status: "confirmed"`, `writtenBy: <user>`, `model: null`. | |
| 7 | Set `AI_SEMANTIC_LAYER=false` and restart the API (operator). | The API starts. | |
| 8 | `GET /api/semantic` and `GET /api/semantic/serving.<table>`. | Both routes answer with existing entries; no new drafts are created. | |
| 9 | Set `AI_SEMANTIC_LAYER=maybe` and restart the API (operator). | The API stops with a `ConfigError` naming the accepted values. | |
| 10 | Call `PUT /api/semantic/serving.no_such_table` with a body. | Response is 404 with the table not found in `serving` or `silver`. | |
| 11 | Call `PUT /api/semantic/serving.<table>` with body `{"description": "..."}` (401-character description). | Response is 400 naming `description`. | |
| 12 | Call `PUT /api/semantic/serving.<table>` with body `{"column": "unknown", "description": "text"}`. | Response is 400 naming the unknown column. | |
| 13 | Call `PUT /api/semantic/serving.<table>` with body `{"column": "<a real column>", "description": "x", "synonyms": ["a", "b", "c", "d", "e", "f", "g"]}` (seven synonyms) or `{"column": "<a real column>", "description": "x", "role": "bogus"}` (unknown role). | Response is 400 naming `synonyms` or `role`. | |
| 14 | A user with `catalog:read` only calls `PUT /api/semantic/serving.<table>`. | Response is 403. | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
