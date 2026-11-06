// Fold keys of the SQL editor (QS-1 QA of 2026-11-05). The events carry the
// physical `keyCode` as well as `key`: CodeMirror's key handler falls back to
// the base key name through `keyCode`, so an event without it never reaches the
// fold binding and the test would prove nothing.
import { cleanup, fireEvent, render } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { foldedRanges } from "@codemirror/language"
import type { EditorView } from "@codemirror/view"
import { SqlEditor } from "./sql-editor"

afterEach(cleanup)

// @codemirror/lang-sql folds a whole multi-line statement (everything after its
// first line), not a parenthesised sub-query.
const SQL = ["SELECT *", "FROM (", "  SELECT id", "  FROM orders", ") t"].join("\n")

type Bracket = "[" | "]"

/**
 * Ctrl+Shift+[ or ] as a browser sends it. Most send the shifted character
 * (`{`, `}`); `shiftedKey: false` is a browser or layout that reports the
 * bracket itself, so the event looks like Ctrl+[ (indent less) to CodeMirror.
 */
function press(view: EditorView, bracket: Bracket, shiftedKey = true) {
  const key = shiftedKey ? (bracket === "[" ? "{" : "}") : bracket
  fireEvent.keyDown(view.contentDOM, {
    key,
    keyCode: bracket === "[" ? 219 : 221,
    ctrlKey: true,
    shiftKey: true,
  })
}

function mount(cursorLine: number, doc = SQL) {
  const views: EditorView[] = []
  render(<SqlEditor value={doc} onChange={() => {}} onView={(view) => views.push(view)} />)
  const view = views[0]
  if (!view) throw new Error("the editor did not render")
  view.dispatch({ selection: { anchor: view.state.doc.line(cursorLine).from } })
  return view
}

function foldedCount(view: EditorView) {
  let count = 0
  foldedRanges(view.state).between(0, view.state.doc.length, () => {
    count += 1
  })
  return count
}

describe("SqlEditor fold keys", () => {
  it("folds the statement on its first line with Ctrl+Shift+[ and unfolds it with Ctrl+Shift+], leaving the text alone", () => {
    const view = mount(1)

    press(view, "[")
    expect(foldedCount(view)).toBe(1)

    press(view, "]")
    expect(foldedCount(view)).toBe(0)
    expect(view.state.doc.toString()).toBe(SQL)
  })

  it("folds the statement from a line inside it, and unfolds it again", () => {
    const view = mount(3)

    press(view, "[")
    expect(foldedCount(view)).toBe(1)

    press(view, "]")
    expect(foldedCount(view)).toBe(0)
    expect(view.state.doc.toString()).toBe(SQL)
  })

  it("does not indent when the browser reports the bracket instead of the shifted character", () => {
    const view = mount(3)

    press(view, "[", false)
    expect(view.state.doc.toString()).toBe(SQL)
    expect(foldedCount(view)).toBe(1)

    press(view, "]", false)
    expect(view.state.doc.toString()).toBe(SQL)
    expect(foldedCount(view)).toBe(0)
  })

  it("changes nothing when there is nothing to fold, not even the indentation", () => {
    const view = mount(1, "  SELECT 1")

    for (const shiftedKey of [true, false]) {
      for (const bracket of ["[", "]"] as const) {
        press(view, bracket, shiftedKey)
        expect(view.state.doc.toString()).toBe("  SELECT 1")
      }
    }
    expect(foldedCount(view)).toBe(0)
  })
})
