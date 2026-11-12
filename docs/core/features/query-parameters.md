# Query parameters

| | |
| --- | --- |
| Module | Query Studio |
| Backlog | `QS-5` (part 1: parameters in SQL, saved queries and the assistant) |
| Kind | Feature |
| Spec | `docs/core/specs/qs-5.md` (the spec's *(proposed)* limits are decision 6) |
| Status | In Acceptance |
| Priority | P1 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Started | 2026-11-07; also in `BACKLOG.md` Dates |
| Shipped | 2026-11-08 (#420, #422) |
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

This page is part 1 of `QS-5`: the server, the assistant and the Query Studio fields. The Query Studio part is described in [Query Studio fields](#query-studio-fields).

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
- The SQL help check inside the editor: that is the variables part of
  `AI-12`.

## Asking the assistant

Yes, and it is part of this feature. `run_sql` takes a `parameters` object
that maps a parameter name to its value: a string, a number, or
`{ "start", "end" }` for a date range. A multi-select (a list) needs a saved
query, so it goes through `run_saved_query`, which uses the query's stored
definitions plus the values the model gives. When `run_saved_query` misses a
required value, it returns "Fill in {{name}} to run this query." as the
tool's error, and the assistant asks the person for the value instead of
guessing. A refusal inside a tool is not written to History; the HTTP run
records it as blocked. `save_query` refuses SQL that holds a `{{name}}`
placeholder, because the tool cannot give definitions. The person saves such
a query in Query Studio. The AI team reviews this part.

## Query Studio fields

Under the SQL editor, the tab shows one field for each `{{name}}` in its SQL, in the order each name first appears. A name in a comment or in a quoted string gets no field. A `{{name.start}}` and a `{{name.end}}` pair make one date range field. A new name gets text, or a date range when the SQL writes it only as `.start` and `.end`.

Each field has the control for its type: a text or number input, a date picker, a start and an end date for a date range, a select for a dropdown, and a multi-select control for a multi-select. A field starts at its default when the tab holds no value for it.

To set a field's type, default or options, press the small menu button on the field. The menu holds the type, the default with a Clear default button and, for a dropdown or a multi-select, the options: a label and a value for each, with Add option and a remove button. A list takes at most 200 options. Changing the type keeps the default only when it fits the new type, and keeps the options only for a dropdown or a multi-select. A value that no longer fits the new type is cleared.

The tab keeps its definitions and its values across a reload, in this browser. A name that leaves the SQL drops its field, but the tab keeps what the person set for that name until the tab is closed, so a cut and paste does not lose it. A definition that nobody has edited and that has no value is dropped as its name leaves, so typing `{{re` on the way to `{{region}}` leaves no field behind.

Run, Run all and Estimate send the values and the definitions of the names the SQL uses. When the SQL has no placeholder, they send the same request as before. The estimate waits while a value is missing: it sends no request, and the panel shows no estimate until every field has a value. While it waits, the panel says "Fill in {{name}} for a pre-run estimate." with the first missing name in parameter order.

When the server refuses a run or an estimate, its sentence shows where a run's error shows, and the field the sentence names is marked. The mark goes when that field's value or definition changes.

Saving sends the definitions with their defaults, never the current values. Opening a saved query fills its definitions, and each field starts at its default. A version whose parameters differ from the next older version shows "Parameters changed" with the names of the parameters that version holds, or "Parameters removed" when it holds none. The oldest version shows neither line. History's "open in studio" opens the statement that ran, with its values already printed in it, so it opens with no fields.

After `{{`, the editor offers the names that the tab defines.

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
| 11 | On saving a query, a definition whose name is not in the SQL, or a placeholder with no definition, is refused with a fixed sentence. On a run whose request carries no definitions, a placeholder is read from its value alone, so an unsaved statement can run with values only: a string is text, a JSON number is a number, and `{start,end}` used as `.start` and `.end` is a date range. A list is refused with "parameter 'name' is a list: a multi-select needs a saved query's definition.", with the parameter's name in place of `name`. A boolean is refused with "parameter 'name' must be text.", with the parameter's name in place of `name`, and null counts as not given. When the request carries definitions, a placeholder none of them names is refused with "{{name}} is used in the SQL but has no definition." | none | Planner's choice, 2026-11-07 |
| 12 | Printing: a number as `BI-11` prints it, a negative number in parentheses so that `-{{n}}` cannot become a `--` comment. Text as a string literal; on Trino a text that holds a backslash is written as pieces joined by `chr(92)`, for example `('a' || chr(92) || 'b')`, so that no raw backslash reaches the statement, and the SQL guards read each engine the way that engine reads it. A date as a date literal (`toDate32('YYYY-MM-DD')` on ClickHouse, `DATE 'YYYY-MM-DD'` on Trino). A date range as two dates. A multi-select as a comma-separated list of literals for `IN ({{name}})`; an empty selection is refused with "Pick at least one value for {{name}}." A dropdown prints its value as text, unless every option is a number | none | Planner's choice, 2026-11-07 |
| 13 | The run and the estimate routes take the values and, optionally, the definitions. The server fills the placeholders as soon as it reads the statement, so the read-only check, the Trino check, the table list, the role's row filters and masks, and the settings check all see the final statement. A statement with no placeholder and no values behaves exactly as before | none | Planner's choice, 2026-11-07 |
| 14 | History records the statement that ran, with the values printed in it, not the text with `{{name}}`. Download and "open in studio" then work without the values. Changing a saved query's parameters alone writes a version, like a change to its SQL, and edit level is enough to change them. Revert and duplicate carry the parameters | none | Planner's choice, 2026-11-07 |
| 15 | A dashboard's SQL source whose SQL holds a placeholder is refused with "Parameters in a dashboard's SQL source come later: give the value in the SQL for now." The SQL help check is the variables part of `AI-12`: it fills in the tab's values when the request carries them, and otherwise refuses with the missing-value sentence | none | Planner's choice, 2026-11-07 |
| 16 | On ClickHouse a heredoc (`$$ … $$` and `$tag$ … $tag$`) is read like a quoted string, so a placeholder inside it is refused with the same inside-quotes sentence a placeholder inside a string gets, and an unclosed heredoc runs to the end of the statement. A heredoc opens where its `$` starts a token: a run of `[A-Za-z0-9_$]` before the `$` is a name when it starts with a letter, `_` or `$`, and a number otherwise, so `SELECT 1$$ … $$` opens a heredoc and `a$$b` stays one identifier. Trino has no heredoc, so nothing changes there | none | Planner's choice, 2026-11-08 |
| 17 | After `{{` the list holds the tab's defined parameter names and nothing else (no SQL keyword), and a `{{` inside a comment or inside a quoted run offers no name at all | none | Planner's choice, 2026-11-08 |
| 18 | On ClickHouse the Unicode quotes U+2018 (`‘`), U+2019 (`’`), U+201C (`“`) and U+201D (`”`) outside an ASCII string, comment, heredoc or quoted identifier make the statement uncertain. The read-only guard refuses the whole statement, and the parameter scanner refuses any placeholder in it. Inside those spans they are plain text. Measured on 2026-11-12 on `clickhouse/clickhouse-server:26.8`: the engine reads `‘ … ’` as a string literal and `“ … ”` as a quoted identifier. Trino is unchanged, because its parser already fails these characters | none | Planner's choice, 2026-11-12 |

## Limits to tell a customer

- A value is a literal printed by the server after it checked the value. The
  server does not use the database driver's own binding.
- History keeps the statement that ran, with the values in it, so a value
  typed into a parameter stays in the caller's own History for 14 days.
  Download and "open in studio" need no values, because the statement already
  holds them.
- Completion after `{{` starts on the first letter typed or with Ctrl+Space.
  The list holds the tab's defined names and nothing else, and inside a
  comment or a quoted run it holds none. It still offers nothing after a dot,
  as in `{{name.`.
- An option list made in the console keeps its option values as text, so a numeric list is printed as text literals.
- A dashboard cannot drive a query's parameters yet, and a dropdown cannot be
  filled from another query. Both are part 2.
- A string literal that contains a well-formed `{{name}}` written by hand, for
  example `'{{region}}'`, is now refused. Before, it ran as plain text.
- A dashboard's SQL source with a placeholder is refused on save and on
  preview. A chart saved before whose SQL no longer passes shows the generic
  "no longer passes the SQL check" message.
- Through the HTTP run, a text value that holds a word the read-only check
  blocks, such as `drop`, is fine because the value is one literal. Through
  `run_sql`, the assistant's own read-only filter also reads the literal's
  text, so a value that contains a word such as `drop` or `delete` is refused
  there.
- A text value may not contain a control character such as a newline, as in
  `BI-11`.
- A negative number is written in parentheses, which a clause such as
  `INTERVAL {{n}} DAY` on Trino does not accept.
- A saved query is checked on save as ClickHouse SQL, so a Trino statement
  with a backslash before a quote can be read differently on save than on a
  Trino run. The result is a refusal, never a bypass.
- A ClickHouse heredoc (`$$ ... $$`) reads like a quoted string; a placeholder
  in it is refused with the inside-quotes sentence. Do not put one in it.
- On ClickHouse a Unicode quote (U+2018, U+2019, U+201C, U+201D) outside an
  ASCII string, comment, heredoc or quoted identifier makes the whole statement
  refused by the read-only guard, even when it holds no write. Do not use
  curly quotes as SQL quotes on ClickHouse.
- A query takes at most 20 parameters, a text value 200 characters, a
  dropdown 200 options and a multi-select 200 values.

## Acceptance checklist

Run by the product owner. A step not performed is never a pass. Rows marked
(operator) call the API or run on a running build; rows marked (console) are run in Query Studio on a running console. `AC2`
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
| `QS-5-AC11` | (console) On ClickHouse, write one statement that uses a text, a number, a date, a date range, a dropdown and a multi-select parameter; set each type in its field's menu, give each field a value, and run | Rows come back filtered by the six values, and History shows the statement with the values printed in it | Not run |
| `QS-5-AC12` | (console) The same as `QS-5-AC11`, on Trino | The same result as on ClickHouse | Not run |
| `QS-5-AC13` | (console) Type `x' OR '1'='1` in a text field, run it on ClickHouse, then run it on Trino | The field keeps the text as typed, and each run gives what `QS-5-AC2` expects, with nothing else run | Not run |
| `QS-5-AC14` | (console) Write `{{region}}` with no default and no value, then run; type `EU` in the field and run again | The estimate sends no request while the field is empty, and the panel says "Fill in {{region}} for a pre-run estimate."; the first run is refused with "Fill in {{region}} to run this query.", and the field is marked; the second run goes, and the mark goes with the value | Not run |
| `QS-5-AC15` | (console) Save a query with a text field whose default is `EU` and a dropdown with two options and no default; change the text field to `US` in the tab; save; reopen the saved query | The reopened fields show `EU` and no choice in the dropdown; `US` is not kept | Not run |
| `QS-5-AC16` | (console) Open a saved query, change only the default of one field, save, and open its version history | The new version shows "Parameters changed" and lists the names of every parameter it holds, including that field; the oldest version shows no such line | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status updated (part 1), with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
