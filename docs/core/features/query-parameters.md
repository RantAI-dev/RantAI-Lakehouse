# Query parameters

| | |
| --- | --- |
| Module | Query Studio |
| Backlog | `QS-5` (part 1: parameters in SQL, saved queries and the assistant) |
| Kind | Feature |
| Spec | `docs/core/specs/qs-5.md` (the spec's *(proposed)* limits are decision 6) |
| Status | Building |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Started | 2026-11-07; also in `BACKLOG.md` Dates |
| Shipped | Not yet |
| Evidence | |
| Plan | Plan files are not kept in the repository. |

## Problem

SQL in Query Studio cannot take a parameter. To run the same query for another
region or another month, a person edits the text, and so does the assistant
when it runs a saved query. A saved query keeps only its SQL, so it cannot say
which values in it are meant to change.

## What the user can do when this is done

1. Write `{{region}}` in a SQL statement, give the parameter a value, and run
   it on ClickHouse or on Trino. The server checks the value against the
   parameter's type before it runs anything.
2. Save a query with its parameters. Each parameter keeps its type, its
   default and its options, and they live in the query's version history with
   the SQL: a revert brings back the parameters of that version.
3. Be refused, in one sentence, when a run misses a value that has no default.
4. Ask the assistant to run SQL or a saved query "for region EU". The
   assistant's `run_sql` and `run_saved_query` take a `parameters` object, and
   the assistant asks when a value is missing.

This page is part 1 of `QS-5`. The server and the assistant ship first. The
Query Studio fields where a person types the values follow in a later pull
request of the same feature.

## How this differs from Parameters (`BI-11`)

`docs/core/features/parameters.md` describes dashboard parameters. They are
defined once for the deployment, read in formulas and charts as `{name}` with
single braces, and never reach SQL text. The parameters on this page belong to
one query, are written as `{{name}}` with double braces inside the SQL text,
and are saved with that query. The two kinds do not share names or storage.
Linking a query's parameter to a global one is part 2.

## Not included

- A dashboard filter that drives a query's parameters, and a dropdown filled
  from another query. Both are part 2 and wait for `BI-18`'s date filter.
- Snippets, a chart from a result, a saved query used as a CTE, and alerts on
  a query. They are later parts of `QS-5`.
- Parameters in the SQL of a dashboard's SQL source. The server refuses them
  (decision 15).
- The Query Studio fields for values and definitions. They arrive in a later
  pull request of this feature.
- The SQL help check inside the editor: that is the variables part of
  `AI-12`.

## Asking the assistant

Yes, and it is part of this feature. `run_sql` and `run_saved_query` take a
`parameters` object that maps a parameter name to its value. A value is a
string, a number, `{ "start", "end" }` for a date range, or a list for a
multi-select. The assistant may also write `{{name}}` in the editor. When the
server refuses a run, the refusal sentence comes back to the assistant as the
tool's error, and it asks the person for the value instead of guessing. The
AI team reviews this part.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | A parameter is written `{{name}}` in the SQL. It does not clash with ClickHouse's `{name:Type}` or with the single-brace `{name}` of `BI-11` formulas | none | Owner, 2026-11-07 |
| 2 | A value enters the SQL as a checked literal, printed again from the parsed value, on ClickHouse and on Trino, before the role's row filters and masks are applied. It is not spliced as text and it is not driver-level binding. The spec's "always bound" wording becomes "checked by type and re-printed as a literal, never spliced as text" | none | Owner, 2026-11-07 |
| 3 | A saved query stores its parameters (type, default, options) with its SQL and in its version history. An unsaved tab keeps them in the browser. A link to a global `BI-11` parameter comes in part 2 | none | Owner, 2026-11-07 |
| 4 | Six types: text, number, date, date range, dropdown with fixed options, and multi-select. The dropdown filled from a query is part 2 | none | Owner, 2026-11-07 |
| 5 | A parameter with no value and no default refuses the run with "Fill in {{name}} to run this query.", with the parameter's name in place of `name`. No value is guessed and nothing is replaced by NULL | none | Owner, 2026-11-07 |
| 6 | Limits: 20 parameters per query, 200 characters in a text value, 200 options in a dropdown, 200 values in a multi-select | 20, 200, 200, 200 | Owner, 2026-11-07 |
| 7 | The assistant's `run_sql` and `run_saved_query` take a `parameters` object (name to value). `tests/fixtures/tool_schemas.json` is regenerated, cases for the assistant's standard request set are added, and the AI team reviews this part | none | Owner, 2026-11-07 |
| 8 | `{{name}}` marks a single value. A date range is written `{{name.start}}` and `{{name.end}}`. Spaces inside the braces are allowed, so `{{ region }}` works. A name follows the rule for `BI-11` parameter names: 1 to 64 letters, digits or `_`, not starting with a digit. A placeholder inside a comment (`--`, `#` or `/* */`) is ignored | none | Planner's choice, 2026-11-07 |
| 9 | A placeholder inside a quoted string or quoted identifier refuses the run with "{{name}} is inside quotes: write it without them, for example = {{name}}." A `{{` outside quotes and comments that is not a well-formed placeholder refuses it with "This query has a {{ that is not a parameter." A bare `{{name}}` for a date range is refused the second way | none | Planner's choice, 2026-11-07 |
| 10 | A definition is a name, a type, a default and, for a dropdown or multi-select, a list of options with a value and a label. Values are checked by type as in `BI-11`: text of at most 200 characters with no control character such as a newline, a date written `YYYY-MM-DD` between the years 1900 and 2299, a finite number, and for a dropdown or multi-select only a value from the definition's own list, which the server prints from its stored copy, not from the request's text | none | Planner's choice, 2026-11-07 |
| 11 | On saving a query, a definition whose name is not in the SQL, or a placeholder with no definition, is refused with a fixed sentence. On a run, a placeholder with no definition is read as text, so an unsaved statement can run with values only. A run whose request carries definitions uses those and applies no such fallback. Such a placeholder is refused with "{{name}} is used in the SQL but has no definition." | none | Planner's choice, 2026-11-07 |
| 12 | Printing: a number as `BI-11` prints it, a negative number in parentheses so that `-{{n}}` cannot become a `--` comment. Text as a string literal; on Trino a text that holds a backslash is written as pieces joined by `chr(92)`, for example `('a' || chr(92) || 'b')`, because the SQL guards read a backslash as an escape and Trino does not. A date as a date literal (`toDate32('YYYY-MM-DD')` on ClickHouse, `DATE 'YYYY-MM-DD'` on Trino). A date range as two dates. A multi-select as a comma-separated list of literals for `IN ({{name}})`; an empty selection is refused with "Pick at least one value for {{name}}." A dropdown prints its value as text, unless every option is a number | none | Planner's choice, 2026-11-07 |
| 13 | The run and the estimate routes take the values and, optionally, the definitions. The server fills the placeholders as soon as it reads the statement, so the read-only check, the Trino check, the table list, the role's row filters and masks, and the settings check all see the final statement. A statement with no placeholder and no values behaves exactly as before | none | Planner's choice, 2026-11-07 |
| 14 | History records the statement that ran, with the values printed in it, not the text with `{{name}}`. Download and "open in studio" then work without the values. Changing a saved query's parameters alone writes a version, like a change to its SQL, and edit level is enough to change them. Revert and duplicate carry the parameters | none | Planner's choice, 2026-11-07 |
| 15 | A dashboard's SQL source whose SQL holds a placeholder is refused with "Parameters in a dashboard's SQL source come later: give the value in the SQL for now." The SQL help check is the variables part of `AI-12`: it fills in the tab's values when the request carries them, and otherwise refuses with the missing-value sentence | none | Planner's choice, 2026-11-07 |

## Limits to tell a customer

- A value is a literal printed by the server after it checked the value. The
  server does not use the database driver's own binding.
- History keeps the statement that ran, with the values in it, so a value
  typed into a parameter stays in the caller's own History for 14 days.
  Download and "open in studio" need no values, because the statement already
  holds them.
- The server and the assistant take parameters first. The Query Studio fields
  for values and definitions arrive in a later pull request of this feature,
  so until then a person supplies values through the API or the assistant.
- A dashboard cannot drive a query's parameters yet, and a dropdown cannot be
  filled from another query. Both are part 2.
- A string literal that contains a well-formed `{{name}}` written by hand, for
  example `'{{region}}'`, is now refused. Before, it ran as plain text.
- A dashboard's SQL source with a placeholder is refused.
- A text value may not contain a control character such as a newline, as in
  `BI-11`.
- A query takes at most 20 parameters, a text value 200 characters, a
  dropdown 200 options and a multi-select 200 values.

## Acceptance checklist

Run by the product owner. A step not performed is never a pass. Rows marked
(operator) call the API or run on a running build; the row marked (console)
needs the Query Studio fields and waits for the later pull request. `AC2`
is the spec's `QS-5-AC2`. The spec's `AC1` (a dashboard filter drives a
parameter) and `AC3` (a chart from a result) belong to later parts, and the
ids on this page otherwise number part 1 only.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `QS-5-AC1` | (operator) On ClickHouse and then on Trino, run `POST /api/query/run` with a statement that uses a text, a number, a date, a date range (`.start` and `.end`) and a multi-select (`IN ({{ids}})`) parameter, each with a value | Both engines return rows filtered by the values, and the statement the engine received holds the values as literals | Not run |
| `QS-5-AC2` | (operator) Run a statement with a text parameter whose value is `x' OR '1'='1` or `'; DROP TABLE t; --`, on both engines | The value is one text literal; the filter matches nothing extra and nothing else runs. The statement in History shows the quote doubled or escaped inside one literal (on Trino, a backslash shows as `chr(92)` between pieces). | Not run |
| `QS-5-AC3` | (operator) Run a statement with `{{region}}` and no value, where the parameter has no default; then add the default and run again | The first run is refused with "Fill in {{region}} to run this query." and the engine receives nothing; the second runs with the default | Not run |
| `QS-5-AC4` | (operator) Save a query with a text parameter and a dropdown through the API, change only the parameter's default, read the versions, then revert to the first version | The change wrote a version; the revert brings back the first version's SQL and parameters; a duplicate carries the parameters | Not run |
| `QS-5-AC5` | (console) Open that saved query in Query Studio | The tab shows the query's parameters with their defaults and options | Not run |
| `QS-5-AC6` | In the assistant, say "run my saved query <title> for region EU"; then say it again for a query with a required parameter and give no value | The assistant runs the first with `{"region":"EU"}`; for the second it asks for the value instead of guessing | Not run |
| `QS-5-AC7` | (operator) Send 21 parameters, a 201-character text value, a dropdown with 201 options and a multi-select with 201 values, one request each | Each is refused with a message that names the limit | Not run |
| `QS-5-AC8` | Sign in as a user without `query:read` and run a parameterised statement | Refused with a permission message, and nothing runs on the engine | Not run |
| `QS-5-AC9` | (operator) Run a parameterised statement, open History, and use Download on that run | History shows the statement with the values printed in it, and Download returns the same rows without asking for values | Not run |
| `QS-5-AC10` | (operator) Put a parameterised statement in a dashboard's SQL source | Refused with "Parameters in a dashboard's SQL source come later: give the value in the SQL for now." | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status updated (part 1), with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
