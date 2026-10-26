/**
 * The keyboard shortcuts the SQL editor binds, as a single authoritative list.
 *
 * These are the keys that CodeMirror actually installs through the
 * extensions in `use-query-studio.ts`: the `Mod-Enter` binding from the
 * hook itself, and the bindings from `basicSetup` in `sql-editor.tsx`
 * (`defaultKeymap`, `searchKeymap`, `foldKeymap`, `completionKeymap`).
 * The test in `keyboard-shortcuts.test.tsx` checks every entry here against
 * the keymap that is actually installed in the editor, so a typo or drift
 * is caught at test time.
 */

export type EditorShortcut = {
  /** What the key does, in the words a person would use. */
  readonly action: string
  /** The key as CodeMirror names it ("Mod-f"). The editor's keymap is checked against this. */
  readonly key: string
  /** The mac variant, when the binding has one. */
  readonly macKey?: string
  /** How the key is shown to a person. */
  readonly label: string
}

// The search panel holds both the find and the replace fields — there is no
// separate replace key. "Find and replace" therefore names one entry, not two.
export const EDITOR_SHORTCUTS: readonly EditorShortcut[] = [
  {
    action: "Run the query, or the selection when there is one",
    key: "Mod-Enter",
    label: "⌘/Ctrl+Enter",
  },
  {
    action: "Find and replace",
    key: "Mod-f",
    label: "⌘/Ctrl+F",
  },
  {
    action: "Select the next occurrence",
    key: "Mod-d",
    label: "⌘/Ctrl+D",
  },
  {
    action: "Go to line",
    key: "Mod-Alt-g",
    label: "⌘/Ctrl+Alt+G",
  },
  {
    action: "Toggle the comment",
    key: "Mod-/",
    label: "⌘/Ctrl+/",
  },
  {
    action: "Fold",
    key: "Ctrl-Shift-[",
    macKey: "Cmd-Alt-[",
    label: "Ctrl+Shift+[ (⌘+Alt+[ on a Mac)",
  },
  {
    action: "Unfold",
    key: "Ctrl-Shift-]",
    macKey: "Cmd-Alt-]",
    label: "Ctrl+Shift+] (⌘+Alt+] on a Mac)",
  },
  {
    action: "Fold everything",
    key: "Ctrl-Alt-[",
    label: "Ctrl+Alt+[",
  },
  {
    action: "Unfold everything",
    key: "Ctrl-Alt-]",
    label: "Ctrl+Alt+]",
  },
  {
    action: "Complete",
    key: "Ctrl-Space",
    label: "Ctrl+Space",
  },
  {
    action: "Undo",
    key: "Mod-z",
    label: "⌘/Ctrl+Z",
  },
  {
    action: "Redo",
    key: "Mod-y",
    macKey: "Mod-Shift-z",
    label: "⌘/Ctrl+Y (⌘+Shift+Z on a Mac)",
  },
  {
    action: "Select all",
    key: "Mod-a",
    label: "⌘/Ctrl+A",
  },
  {
    action: "Match the bracket",
    key: "Shift-Mod-\\",
    label: "Shift+⌘/Ctrl+\\",
  },
]
