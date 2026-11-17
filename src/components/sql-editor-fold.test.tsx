// Fold keys of the SQL editor (QS-1 QA of 2026-11-05). The events carry the
// physical `keyCode` as well as `key`: CodeMirror's key handler falls back to
// the base key name through `keyCode`, so an event without it never reaches the
// fold binding and the test would prove nothing.
import { cleanup, fireEvent, render } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { foldCode, foldedRanges } from "@codemirror/language"
import type { EditorView } from "@codemirror/view"
import { EDITOR_SHORTCUTS } from "@/lib/editor-shortcuts"
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

const BRACKET_KEY_CODES = { "[": 219, "]": 221 } as const
const SHIFTED_BRACKETS = { "[": "{", "]": "}" } as const

/**
 * The keydown a browser sends for a chord string of `EDITOR_SHORTCUTS`
 * ("Ctrl-Shift-[", "Cmd-Alt-]"). It throws on a shape it does not know, so a
 * new modifier or key makes the test fail instead of silently pressing nothing.
 */
function chordToKeyDown(chord: string): KeyboardEventInit & { keyCode: number } {
  const parts = chord.split("-")
  const bracket = parts.pop()
  if (bracket !== "[" && bracket !== "]") {
    throw new Error(`fold chord "${chord}" does not end in a bracket key`)
  }
  const init = { ctrlKey: false, shiftKey: false, altKey: false, metaKey: false }
  for (const modifier of parts) {
    if (modifier === "Ctrl") init.ctrlKey = true
    else if (modifier === "Shift") init.shiftKey = true
    else if (modifier === "Alt") init.altKey = true
    else if (modifier === "Cmd") init.metaKey = true
    else throw new Error(`fold chord "${chord}" has an unknown modifier "${modifier}"`)
  }
  return {
    ...init,
    key: init.shiftKey ? SHIFTED_BRACKETS[bracket] : bracket,
    keyCode: BRACKET_KEY_CODES[bracket],
  }
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

  // `foldKeys` in `sql-editor.tsx` matches the physical keys itself, so a
  // change to the chords `EDITOR_SHORTCUTS` shows to a person has to reach it.
  describe("matches the chords listed in EDITOR_SHORTCUTS", () => {
    for (const action of ["Fold", "Unfold"] as const) {
      const entry = EDITOR_SHORTCUTS.find((e) => e.action === action)
      if (!entry) throw new Error(`EDITOR_SHORTCUTS has no "${action}" entry`)
      if (!entry.macKey) throw new Error(`the "${action}" entry has no macKey`)

      for (const chord of [entry.key, entry.macKey]) {
        it(`${action.toLowerCase()}s on ${chord}, leaving the text and indentation alone`, () => {
          const view = mount(1)
          // Start folded for Unfold, so the key has something to undo.
          if (action === "Unfold") {
            foldCode(view)
            expect(foldedCount(view)).toBe(1)
          }

          fireEvent.keyDown(view.contentDOM, chordToKeyDown(chord))
          expect(foldedCount(view)).toBe(action === "Fold" ? 1 : 0)
          expect(view.state.doc.toString()).toBe(SQL)
        })
      }
    }
  })
})
