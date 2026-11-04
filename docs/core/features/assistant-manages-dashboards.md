# The assistant manages dashboards

| | |
| --- | --- |
| Module | AI Copilot |
| Backlog | `AI-1`, `AI-2`, `AI-20`, `AI-5`, `AI-8`, `AI-15`, `AI-22` |
| Spec | `docs/core/specs/ai-1.md`, `docs/core/specs/ai-2.md`; `AI-20` has no spec (it is a follow-up from PR #104 review SHOULD-FIX 11) |
| Status | Decisions signed 2026-10-15 (two decisions); the rest are planner defaults, owner to confirm at QA |
| Plan | Plan files are not kept in the repository. |

## Problem

The assistant can build and change charts on a dashboard, but cannot rename
or delete the dashboard itself, cannot arrange or resize its tiles, cannot
move a chart from one board to another, and cannot change the dashboard's
filters from the chat. The page context the chat sees already lists the
active filters, but it does not list the on-screen time grain (the override
set on the dashboard's grain control), so a plain question about the time
grain answers from the chart's own grain or not at all.

## What the user can do when this is done

1. Ask the assistant to rename a board: "rename this dashboard to 'Q3
   review'".
2. Ask the assistant to delete a board; the assistant asks for it through
   an Approval, names the board, and says its charts go too.
3. Ask the assistant to arrange the tiles by plain instruction: "make the
   revenue chart half width and put the funnel next to it", "put the
   conversion rate on top". Only the tiles the person names move.
4. Ask the assistant to move a chart to another board: "move the
   conversion funnel to the Sales board".
5. Ask the assistant to change the view's filters: "show only 2025",
   "only the East region". The page address changes; nothing is saved for
   anyone else.
6. Ask the assistant to save a filter as the default for everyone: "save
   this as the default for everyone".
7. Ask the assistant about the dashboard's time grain; the answer names
   the grain set on the dashboard's control, not the chart's own grain.
8. Ask for a chart in words that do not name a dashboard ("Show the share of
   foreign visitors by country as a pie.") in Build mode; the assistant is
   offered the chart tools for that turn and drafts the chart.
9. Ask for a request with more than one step ("Build me a dashboard about
   events, then add a chart of the events per month."); after the first
   draft is confirmed, the chat goes on with the next step by itself.

## Not included

- **Undo** of any of the above. Boards have no version history yet; the
  feature page records it as an unmet target and the backlog rows waits
  for `BI-18` (version history part).
- Renaming, deleting or saving default filters on the built-in "Main"
  board. The console refuses each, and the assistant gets the same
  refusal as a plain message.
- "Linked filters" named in `docs/core/specs/ai-2.md`: the feature does
  not exist in the `BI-18` code at the base the planner read, and no
  tool here reaches for it. The row is left unmet.
- Layout changes from `BI-18` part C (tabs, phone layout, sections,
  stories, templates, favourites) and the rest of `BI-18`.
- Tabs and templates (`AI-9`).

## Asking the assistant

Six tools cover the things a person can already do to a dashboard from the
console: rename it, delete it, arrange its tiles, move a chart, change the
filters this viewer sees, and save a board's default filters. Four of the six
call the same route handler the console calls, so validation, permissions,
audit and the console's own refusals all carry through unchanged.

- `rename_board` (`dashboard:write`, WriteLow): renames a board; the
  built-in Main board is refused.
- `delete_board` (`dashboard:write`, WriteHigh): waits for approval; the
  approval reason names the board and says its charts are deleted too;
  the built-in Main board is refused.
- `arrange_board` (`dashboard:write`, WriteLow): takes the tiles that
  change and their boxes; the others keep their boxes; the grid is 12
  columns wide; a box outside the grid or with `w` or `h` below 1 is
  refused.
- `move_chart` (`dashboard:write`, WriteLow): moves a chart from one
  board to another; the chart's tile leaves the old board's layout;
  its other fields are unchanged.
- `set_board_view_filters` (`dashboard:read`, Read): validates the
  filters and writes nothing; returns the full list that should be
  active (an empty list clears); offered in Ask mode too, because it
  changes nothing for anyone.
- `save_board_filters` (`dashboard:write`, WriteLow): saves the board's
  default; refused on the built-in Main board; only used when the
  person asks to save or to make it the default for everyone.

The chart tools say what they did. `delete_chart` refuses a chart that does
not exist or that the person cannot see ("this tile does not exist"),
instead of reporting a delete that never happened. `update_chart` keeps the
formatting and the board the call leaves out, so changing a chart's title
does not move it to another board or cost it its colours; a change of the
chart's kind resets the formatting, because the old formatting does not fit
the new kind. `update_chart` keeps the data source the call leaves out
too, so a rename does not have to repeat the mart. `list_charts` names
each chart's board and its tab, so the assistant can read what an update
keeps.

`list_charts` also names the formatting a chart holds (the `style` it
was saved with, and the reference lines when it has any) and the names
its series can take (`seriesNames`): the names the chart draws, which are
the names a series colour and a value format are stored under. The
assistant names a series instead of guessing one. `format_chart` refuses
a series the chart does not draw, and names the series the chart does
draw, so the assistant corrects the call instead of trying again. A chart
whose series are not known when it is saved (one that breaks its data
down into one series per value, or one that reads a parameter) is not
checked, and for such a chart `seriesNames` names the measure and not a
series. A pie, a rose, a funnel or a treemap colours its marks by the
name of the item and not by a series.

A dashboard filter that names a saved segment is refused: a board filter
names a column, and a saved segment belongs to a chart's or a metric's own
filters.

The dashboard page applies a `set_board_view_filters` result when the
chat reply's trace holds the step for the board on screen: it writes the
filters into the page address with the same helpers a chip change uses.
On any other board the step card shows a link to that board with the
filters in the address.

The page context the assistant reads now also lists the time grain on
screen and the board's saved grain (`summarizeGrain`, new, beside the
filter summary), so the assistant can name the dashboard's grain rather
than the chart's own.

### What the chat is offered

In Build mode the chat is offered the chart and dashboard tools on every
turn, whatever the question says: the tools that list charts and SQL
sources, that suggest a dashboard or a join, that create and change a
chart, a board or a tab, that move charts into a tab, that read and set
the view's filters, and that write and check a calculated field. A
question that asks for a pie, a pivot, a progress bar or a bullet graph
no longer has to name a dashboard to reach them. Ask mode is unchanged: it
offers no write tool.

The console's own tool list (the Tools menu) is an upper bound, not a
replacement: a tool the person allowed is offered only when the turn needs
it, and a tool the person did not allow is never offered. The offered
list is ordered the same way every time - the tools every turn has, then
the chart tools of a Build turn, then the rest - so two Build turns from
the same page share the model's prompt.

### A request that goes on by itself

A write that the rest of the request needs cannot be finished in one
turn: the draft waits for the Confirm button, and the chart that needs
the board does not exist yet. The assistant therefore names the draft it
made and, when steps are left, the steps that are left; once the draft is
confirmed and succeeded, the console sends a message that begins
"Confirmed and saved: ..." in the same session. The message names each
object the Confirm saved (its tool, id and title, the parts the result
carries; a part the result does not carry is left out) and says not to
draft those objects again, then "Continue:" with the steps that are left;
it names what was saved, it does not re-send the tool results. A
cancelled or failed draft sends nothing, so a refused step stops the
request there. At most three such continuations follow one typed
message.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Six tools, with names that contain `board` or `chart` so the console's capability test covers them unchanged: `rename_board`, `delete_board`, `arrange_board`, `move_chart`, `set_board_view_filters`, `save_board_filters` | Planner default | Owner to confirm at QA |
| 2 | The tools go through the console's route handlers (`tools/calc_fields.rs` pattern); where a handler is private it is widened to `pub(in crate::routes)`; errors pass through the existing classification (`api_result_to_value`) | Planner default | Owner to confirm at QA |
| 3 | `arrange_board` takes only the tiles that change (`{chartId: {x, y, w, h}}`); the server reads the current layout, refuses a chart id that is not on that board, refuses a box outside the 12-column grid or with `w` or `h` below 1, merges and saves through the console's `boards_update` with `layout`. The prompt says the grid has 12 columns and that `list_boards` gives the current layout | Planner default | Owner to confirm at QA |
| 4 | `move_chart` reads the stored chart, changes only its board, saves through the path `specs_update` uses, and removes the chart's tile from the old board's layout | Planner default | Owner to confirm at QA |
| 5 | `set_board_view_filters` validates filters and writes nothing; returns `{applyViewFilters: true, board, filters}` (the full list that should be active; an empty list clears). Read risk, so it is offered in Ask mode too | Planner default | Owner to confirm at QA |
| 6 | The dashboard page applies a view-filter result. When a chat reply's trace holds a `set_board_view_filters` step with `ok: true` and its `board` is the board on screen, the page writes the filters into the address with the existing helpers (`encodeFilters` / `filtersToParam`). On any other page or another board the step card shows a link to that board with the filters in the address, built with the same helpers. The hook sits beside the existing way the dock tells the page about a tool result (`use-copilot.ts:301`, `:405`); no new channel | Planner default | Owner to confirm at QA |
| 7 | `save_board_filters` saves the default and is used only when the person asks to save or to make it the default for everyone. On the built-in Main board it fails as the console does. **The plan named a `grain?` argument alongside `board` and `filters`; the schema and implementation carry `board` + `filters` only — saving the time grain is not part of the tool** | Planner default | Owner to confirm at QA |
| 8 | The page context carries what the tools need: for `AI-2`, the active filters also as compact JSON (`FilterDef[]`), within their own 4,000-character budget, leaving room inside the page context's 6,000-character cap; when over, the JSON is left out and the chip summary stays. For `AI-20`, the on-screen time grain and the board's saved grain (`summarizeGrain`, new, beside `summarizeFilters`), and the page effect's dependency list gains them | Planner default | Owner to confirm at QA |
| 9 | `AI-2` is two tools, not one: `set_board_view_filters` changes only the person's view (the active filters in the address, nothing saved) by default, and `save_board_filters` saves the board's default only on an explicit request, through Confirm. A single tool that saved by default would change the dashboard for everyone without a second prompt | — | Owner, 2026-10-15 |
| 10 | "Undo in one step" is not built. Boards have no version history yet, so the assistant cannot undo a rename, a deletion, an arrange, a move or a saved-filters change. It is written as an unmet target, and the backlog gains a row that waits for `BI-18` (version history part) | — | Owner, 2026-10-15 |
| 11 | "Linked filters" named in `docs/core/specs/ai-2.md` are not in the `BI-18` code at `7a092caf`. The feature page says so and leaves the row unmet; no code for it | Planner default | Owner to confirm at QA |

## Limits to tell a customer

- **Layout: last write wins.** A board's layout is replaced whole by the
  next write. There is no undo and no version history.
- The built-in **Main** board cannot be renamed, deleted, or given
  default filters.
- **View filters live in the address.** They belong to the person
  looking, not to the board. The board's saved default stays; **Reset**
  returns to it; **Save as default** (an editor) writes it for everyone.
- **Undo is not built.** It waits for `BI-18` (version history part).
- "Linked filters" named in `docs/core/specs/ai-2.md` are not in the
  `BI-18` code at the base the planner read. The feature page says so
  and leaves the row unmet; no code for it.
- Filters are validated by the same `validate_filters` the console uses;
  one board accepts at most `MAX_FILTERS` (20, from `BI-18`).
- A delete is the only WriteHigh tool. It asks for approval, names the
  board and says its charts go too.
- The active filters in the page context are written as compact JSON
    within a 4,000-character budget of their own, which leaves room inside
    the page context's 6,000-character cap. When they do not, the JSON
  is left out and the chip summary stays.
- **The chat goes on by itself at most three times.** A request that needs
  more than four dependent writes stops after the fourth, and the person
  asks for the rest.
- A tool the person took out of the Tools menu is not offered, even when
  the request needs it.

## Acceptance checklist

Run on a running console. A step not performed is never Pass. The
quoted requests come from the specs.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `AI-1-AC1` | Ask "rename this dashboard to 'Q3 Tourism' and move the Bali chart to the top" (the spec's quoted request) | The board is renamed and the named chart's tile is at the top of its grid | Not run |
| `AI-1-AC2` | Ask to delete a board with charts, then reject the approval | Nothing on the board or its charts changes; the board is still there with all its charts | Not run |
| `AI-1-AC3` | As a role without `dashboard:write`, ask to rename a board, delete a board, arrange tiles, move a chart, or save filters | A plain refusal naming the missing permission; nothing changes | Not run |
| `AI-2-AC1` | Ask "show only 2025 and only Bali" (the spec's quoted request) | The view's filters become a 2025 number filter and a Bali filter; the address carries them | Not run |
| `AI-2-AC2` | After the step above, open the same board as another viewer | The board's saved default is unchanged; the address carries only what that viewer set, if anything | Not run |
| `AI-20-AC1` | Open a dashboard with the filter-row grain control set to a value that is not the chart's own grain; ask the assistant about the dashboard's time grain | The answer names the override, not the chart's own grain | Not run |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` items `AI-1`, `AI-2`, `AI-20` moved to **Released**, with the PRs in the PR column; the undo row added by this plan moves with them
- [ ] `CHANGELOG.md` entry a customer can read
- [ ] `dashboard-filters.md` "Asking the assistant" stays accurate (the assistant is the same one this page calls)