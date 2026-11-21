// Keyboard shortcuts dialog: that every listed shortcut is bound in the
// editor that the console actually runs, and that the dialog lists a row
// for each shortcut when the button is pressed. The services are stubbed;
// nothing here reaches the network.
import { cleanup, render, screen, waitFor } from "@testing-library/react"
import type { EditorState } from "@codemirror/state"
import { EditorView, keymap, type KeyBinding } from "@codemirror/view"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { assetService, queryService } from "@/services"
import { EDITOR_SHORTCUTS } from "@/lib/editor-shortcuts"
import { KeyboardShortcutsDialog } from "./keyboard-shortcuts-dialog"
import { SqlPanel } from "./sql-panel"
import { queryDraftKey } from "@/lib/query-draft"
import { useQueryStudio } from "./use-query-studio"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
})

function stubServices() {
  spyOn(queryService, "listSaved").mockResolvedValue([])
  spyOn(queryService, "listHistory").mockResolvedValue([])
  spyOn(assetService, "listAssets").mockResolvedValue([])
  spyOn(assetService, "getAsset").mockResolvedValue({ schema: [] } as never)
}

/** Wraps the studio hook with the SQL panel, the same way the page does. */
function Harness() {
  const studio = useQueryStudio(queryDraftKey("u1", null))
  return <SqlPanel studio={studio} />
}

/** Finds the editor view from the DOM, the way the editor's own API exposes it. */
function editorView(container: HTMLElement): EditorView {
  const content = container.querySelector(".cm-content")
  const view = content ? EditorView.findFromDOM(content as HTMLElement) : null
  if (!view) throw new Error("the editor did not render")
  return view
}

/**
 * Returns every key binding installed across the editor state's keymap facets.
 */
function getKeymap(state: EditorState): readonly KeyBinding[] {
  return state.facet(keymap).flat()
}

describe("keyboard shortcut bindings", () => {
  it("every listed shortcut is bound in the editor's installed keymap", async () => {
    stubServices()
    const { container } = render(<Harness />)

    const view = editorView(container)
    await waitFor(() => {
      // Wait for the editor to have finished setting up its extensions
      expect(view.state.facet(keymap).length).toBeGreaterThan(0)
    })

    const bindings = getKeymap(view.state)

    for (const shortcut of EDITOR_SHORTCUTS) {
      if (shortcut.macKey !== undefined) {
        const found = bindings.find(
          (b) => b.key === shortcut.key && b.mac === shortcut.macKey
        )
        expect(found).toBeDefined()
      } else {
        const found = bindings.find((b) => b.key === shortcut.key)
        expect(found).toBeDefined()
      }
    }
  })
})

describe("KeyboardShortcutsDialog", () => {
  // UX-9: the dialog opens from the ⋯ menu and the command menu, so the
  // page holds whether it is open; the menu path is in the page tests.
  it("lists a row for every shortcut the editor binds, and the command menu's key", async () => {
    render(<KeyboardShortcutsDialog open onOpenChange={() => {}} />)

    for (const shortcut of EDITOR_SHORTCUTS) {
      expect(await screen.findByText(shortcut.action)).toBeTruthy()
    }
    expect(screen.getByText("Open the command menu")).toBeTruthy()
  })
})
