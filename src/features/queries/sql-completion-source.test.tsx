// The editor's list after a `{{` (`QS-5`): the tab's own parameter names,
// and nothing else. The SQL language offers the keywords everywhere else,
// and a placeholder is the one place where a keyword cannot be written, so
// the keywords stay out of it.
//
// The editor is a real one here: the list is what CodeMirror shows, with
// the console's own source and the language's keywords beside each other.
// The last test opens the editor the console shows, so the list is read
// through the wiring the studio uses.

import { afterEach, describe, expect, it } from "bun:test"
import { cleanup, render } from "@testing-library/react"
import {
  autocompletion,
  currentCompletions,
  startCompletion,
} from "@codemirror/autocomplete"
import { sql } from "@codemirror/lang-sql"
import { EditorState } from "@codemirror/state"
import { EditorView } from "@codemirror/view"
import { SqlEditor } from "@/components/sql-editor"
import {
  createSqlCompletionSource,
  sqlCompletionExtension,
} from "./sql-completion-source"

/** The names the open tab defines. */
const PARAMETERS = ["region", "day"]

/** The editors the tests opened, so each is closed again. */
const views: EditorView[] = []

afterEach(() => {
  for (const view of views.splice(0)) view.destroy()
  cleanup()
})

/**
 * The console's own source, over the names the open tab defines.
 */
function consoleSource() {
  return createSqlCompletionSource({
    tables: [],
    columns: () => ({}),
    parameters: () => PARAMETERS,
  })
}

/**
 * An editor with the SQL language and the console's suggestions, the way
 * Query Studio builds it: the language first, the console's own source
 * last, and the cursor at the end of the text.
 */
function editor(doc: string): EditorView {
  const view = new EditorView({
    state: EditorState.create({
      doc,
      selection: { anchor: doc.length },
      extensions: [
        sql(),
        // The editor's own basic setup brings the autocomplete plugin; the
        // console's extension then says which sources answer.
        autocompletion(),
        sqlCompletionExtension(consoleSource()),
      ],
    }),
    parent: document.body,
  })
  views.push(view)
  return view
}

/** The labels the editor's list holds, once it has opened. */
async function shownLabels(view: EditorView): Promise<string[]> {
  startCompletion(view)
  // The editor waits a moment before it asks its sources.
  await new Promise((resolve) => setTimeout(resolve, 200))
  return currentCompletions(view.state).map((completion) => completion.label)
}

describe("the list of the SQL editor", () => {
  it("holds the tab's parameter names after a {{ and no keyword", async () => {
    const view = editor("SELECT {{")

    const labels = await shownLabels(view)

    expect(labels.slice().sort()).toEqual(["day", "region"])
  })

  it("holds only the names that match after a {{ with a half typed name", async () => {
    const view = editor("SELECT {{reg")

    const labels = await shownLabels(view)

    expect(labels).toEqual(["region"])
  })

  it("still holds the keywords where a placeholder cannot be", async () => {
    const view = editor("SELECT ")

    const labels = await shownLabels(view)

    expect(labels).toContain("select")
  })

  it("holds no parameter name after a {{ inside a quoted run", async () => {
    const view = editor("SELECT 'a {{")

    const labels = await shownLabels(view)

    expect(labels).toEqual([])
  })

  it("holds no parameter name after a {{ inside a comment", async () => {
    const view = editor("-- a note with a {{")

    const labels = await shownLabels(view)

    expect(labels).toEqual([])
  })
})

describe("the list of the editor the console shows", () => {
  it("holds the tab's parameter names after a {{ and no keyword", async () => {
    const opened: EditorView[] = []
    render(
      <SqlEditor
        value="SELECT {{"
        onChange={() => {}}
        onView={(view) => opened.push(view)}
        extensions={[sqlCompletionExtension(consoleSource())]}
      />
    )
    const view = opened[0]
    if (!view) throw new Error("the editor did not open")
    views.push(view)
    // The person typed at the end of the statement, so the cursor is there.
    view.dispatch({ selection: { anchor: view.state.doc.length } })

    const labels = await shownLabels(view)

    expect(labels.slice().sort()).toEqual(["day", "region"])
  })
})
