# The report editor as a document

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-62` |
| Kind | Feature |
| Spec | `docs/core/specs/bi-62.md` |
| Status | Planned |
| Priority | P2 |
| Owner | The module's owner, in the base. |
| Acceptor | The product owner. |
| Plan | `docs/superpowers/plans/2026-11-15-bi-62-report-editor.md` |
| Started | 2026-11-15 |
| Shipped | |
| Evidence | |

## What the user can do when this is done

1. Open **New → Report** and start typing: the title is the first line, the text sits on the page.
2. Type `/` for headings, lists, a quote, a divider, a chart or "Ask AI to write…"; or use Markdown
   shortcuts as they type (`# `, `## `, `- `, `1. `, `> `, `---`).
3. Select words to make them bold or italic or a link (also ⌘B, ⌘I, ⌘K).
4. Move a block with its ⋮⋮ handle or Alt+↑/↓; add one below with its +.
5. Add a chart from the **Charts** tab of the side panel (live thumbnails of every dashboard's
   charts: drag one in, or click to add it at the cursor) or from the `/` menu's picker; make it
   text-width or wide; swap or remove it from its hover toolbar.
6. Ask the assistant to write a paragraph, or to **Describe** a chart; read the draft as it
   streams, then Accept (Enter) or Discard (Esc). The same exchange appears in the AI Copilot tab.
7. Never press Save: the page says Saving…, Saved, or Couldn't save · Retry.
8. Preview the reader's view in place, and Download PDF; both match the editor.

## Decisions

See the plan, section 1. The owner signed the layout (mockup D, 2026-11-15).

## Limits to tell a customer

- One person edits a report at a time; when two people save the same report, the later save
  wins (as before).
- A report holds at most 30 charts and 300 blocks (unchanged).

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `BI-62-AC1` | New → Report, type a title, `## Arrivals`, a paragraph with one bold word, one italic word and a link, a bulleted and a numbered list, a quote and a divider | Each turns into its block as typed; refresh shows the same page; no Save was pressed | Not run |
| `BI-62-AC2` | Add three charts: one dragged from the Charts tab, one clicked in, one from `/` → Chart; make one wide | All three draw live; the wide one spans past the text column; Alt+↑/↓ and ⋮⋮ move them | Not run |
| `BI-62-AC3` | Open an existing report made before this change (with a Link block) | It opens and reads the same; the link sits on its own line as before | Not run |
| `BI-62-AC4` | Preview, then Download PDF | Marks, lists, quote, divider and chart widths match the editor | Not run |
| `BI-62-AC5` | Turn the network off, type, turn it on | "Couldn't save · Retry" shows; Retry saves; nothing typed is lost | Not run |
| `BI-62-AC6` | "Ask AI to write…" a paragraph about the first chart; then Describe the second chart | Drafts stream, Accept inserts them, Discard leaves nothing; the AI Copilot tab shows both exchanges | Not run |
| `BI-62-AC7` | As a reader with only `dashboard:read`, open the report | It reads with the reader's own data; no editing controls | Not run |
| `BI-62-AC8` | On a 390 px phone: open, type, insert a chart through `/` | Works, no sideways scroll, the Charts panel opens as a sheet | Not run |
