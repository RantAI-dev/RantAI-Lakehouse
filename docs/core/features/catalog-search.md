# Search that finds a table by its columns and tags

| | |
| --- | --- |
| Module | Data (Catalog) |
| Backlog | `DATA-11` |
| Spec | `docs/core/specs/data-11.md` (one *(proposed)* number, signed under Decisions 1) |
| Status | Decisions signed 2026-10-09. In build |
| Plan | `docs/superpowers/plans/2026-10-09-data-11-catalog-search.md` |

## Problem

Search looks for the typed text as one piece. `customer revenue` does not
find a table named `revenue_by_customer`, `revnue` finds nothing, and a
column name finds nothing, because column names are not searched at all.
Results come back in catalog order, not best match first. The ⌘K box and the
Data Explorer search different fields, and by reading the code the ⌘K box
hides a table that matched only by its description or tag. Read from `main`
at `c338862` on 2026-10-09; evidence is in the plan, section 2.

## What the user can do when this is done

1. Type a column name in the ⌘K box or the Data Explorer and get the tables
   that have that column.
2. Type words in any order and get the tables that match all of them.
3. Make one typing mistake in a word of four letters or more (`revnue`) and
   still get the table.
4. See the best match first, and under each result why it matched: "column
   revenue_amount", "tag finance", "description".
5. Get the same results from the ⌘K box and the Data Explorer for the same
   words.
6. Open every result of a ⌘K search in the Data Explorer with one click.
7. Filter the Data Explorer by owner and by tag, as well as by type and
   layer.

## Not included

- A certification filter. Certification does not exist yet; the filter comes
  with `DATA-12`. **Waits for `DATA-12`.**
- Tags with a key and a value. Tags are single words until `DATA-12`; search
  matches them as they are.
- Hiding single tables from single users. The product has no per-table
  visibility today (Decision 5).
- Plain-language search ("tables about customer churn"): the AI team,
  `AI-16`.
- The assistant's own table lookup. It keeps searching titles only; changing
  it is the AI team's (`AI-16`).

## Asking the assistant

Not part of the dashboards assistant. The assistant's `list_datasets` tool is
not changed by this work.

## Decisions

All eight defaults signed by the product owner on 2026-10-09 ("run as proposed").

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Speed target *(proposed)*. No competitor publishes a number. | Results within 500 ms for a catalog of 10,000 tables, measured and written to `docs/plans/DATA-11-RESULT.md`. If the measurement misses it, that is reported, not hidden. | 2026-10-09 |
| 2 | How fresh search is. Searching 10,000 tables and their columns from scratch on every keystroke cannot meet Decision 1, so search reads a copy of the catalog that is refreshed on a timer. | The copy is at most 30 seconds old. A table created by a load can take up to 30 seconds to appear in search. An edit made in the console (description, tags, owner) shows at once. The Data Explorer list with an empty search box stays live, as today. | 2026-10-09 |
| 3 | How forgiving a typo is. | One wrong, missing, extra or swapped letter, in a word of four letters or more. Shorter words must match exactly. | 2026-10-09 |
| 4 | What "ranked by use" means. | Relevance decides the order. Between two equally good matches, the table read by more queries in the last 7 days comes first. This is the count the asset page already shows; it covers Query Studio and assistant queries, not dashboard reads. | 2026-10-09 |
| 5 | The spec says "only assets the user may see appear". The product has no per-table visibility: whoever may open the catalog sees every table in it. | Search shows exactly what the Data Explorer list shows to the same user, never more. Hiding single tables from single users is not built here; it becomes a backlog item of its own. | 2026-10-09 |
| 6 | The certification filter and tag keys and values. | Both wait for `DATA-12`. | 2026-10-09 |
| 7 | How many tables the ⌘K box lists. | Eight, as today, with a row "See all results" that opens the Data Explorer with the same words. | 2026-10-09 |
| 8 | Phones. The search box in the top bar is hidden on narrow screens and ⌘K needs a keyboard. | Left as it is. On a phone, search from the Data Explorer page. | 2026-10-09 |

## Limits to tell a customer

- Search covers table names, ids, namespaces, owners, descriptions, tags,
  column names and column descriptions. It does not search the data inside
  tables.
- A table loaded for the first time can take up to 30 seconds to appear in
  search.
- A typo is forgiven only in words of four letters or more, one letter per
  word.
- Everyone who may open the catalog finds every table in it. Search does not
  hide single tables from single users.
- There is no certification filter until certification exists.

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
A step not performed is never Pass. Steps needing a terminal are marked
(operator).

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Press ⌘K (Ctrl+K) on three different pages; type a table's name | The table is listed first | |
| 2 | Type a column name that is in no table name | Every table with that column is listed, each saying "column <name>" | |
| 3 | Type a tag that is in no table name | The tagged tables are listed, each saying "tag <name>" | |
| 4 | Type two words of a table's description in the opposite order | The table is listed | |
| 5 | Type `revnue` where a table or column is named `revenue` | It is found, and the result shows it is an approximate match | |
| 6 | Type the same words in the Data Explorer search box | The same tables, in the same order | |
| 7 | In the ⌘K box, choose "See all results" | The Data Explorer opens with the words filled in | |
| 8 | In the Data Explorer, filter by owner, then by tag, then by layer | Each filter narrows the list; together they combine | |
| 9 | Edit a table's tags, then search the new tag straight away | The table is found | |
| 10 | Sign in as a user without `catalog:read`; press ⌘K and type a table name | No tables are listed; the Data Explorer says access is missing | |
| 11 | On a deployment with two tenants and `CATALOG_TENANT_ID` set, search as a member of the other tenant | No tables are listed, and the Data Explorer shows the reason | |
| 12 | Type words that match nothing | "No results", not an error and not every table | |
| 13 | (operator) Stop ClickHouse; search | A plain "catalog unavailable" message, with no server text in it | |
| 14 | (operator) Read `docs/plans/DATA-11-RESULT.md` | It gives the measured time for 10,000 tables, the command that produced it, and whether 500 ms was met | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
