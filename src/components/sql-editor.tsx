"use client"

import * as React from "react"
import { useTheme } from "next-themes"
import CodeMirror, { EditorView } from "@uiw/react-codemirror"
import { sql } from "@codemirror/lang-sql"
import { foldable, foldCode, unfoldCode } from "@codemirror/language"
import { Prec, type Extension } from "@codemirror/state"
import { cn } from "@/lib/utils"

/**
 * Skema yang dipakai untuk autocomplete: nama tabel beserta kolomnya.
 * Bentuknya sengaja sama dengan yang diminta `@codemirror/lang-sql`.
 */
export type SqlSchema = Record<string, string[]>

/**
 * Fold the block the cursor is in. `foldCode` only folds a block that starts on
 * the cursor's own line; `@codemirror/lang-sql` folds a whole multi-line
 * statement from its first line, so with the cursor on any later line the key
 * did nothing. This walks up to the nearest line whose block holds the cursor.
 */
function foldEnclosing(view: EditorView): boolean {
  if (foldCode(view)) return true
  const { state } = view
  const head = state.selection.main.head
  for (let n = state.doc.lineAt(head).number - 1; n >= 1; n -= 1) {
    const line = state.doc.line(n)
    const range = foldable(state, line.from, line.to)
    if (range && range.to >= head) {
      // The cursor goes to the fold's own line: a fold that covers the cursor
      // is opened again by the next selection change.
      view.dispatch({ selection: { anchor: range.from } })
      return foldCode(view)
    }
  }
  return false
}

/**
 * Ctrl+Shift+[ folds and Ctrl+Shift+] unfolds (Cmd+Alt on a Mac), the keys
 * `editor-shortcuts.ts` lists. They are matched on the physical bracket key
 * (`keyCode` 219 and 221), not on the character, before any other binding.
 * A browser or layout that reports `[` for Ctrl+Shift+[ otherwise looks like
 * Ctrl+[ to CodeMirror, which is `indentLess`, and the key re-indents the line
 * instead of folding it (QS-1 QA of 2026-11-05).
 */
const foldKeys = Prec.highest(
  EditorView.domEventHandlers({
    keydown(event, view) {
      const bracket = event.keyCode === 219 ? "[" : event.keyCode === 221 ? "]" : null
      const chord =
        (event.ctrlKey && event.shiftKey && !event.altKey && !event.metaKey) ||
        (event.metaKey && event.altKey && !event.ctrlKey && !event.shiftKey)
      if (!bracket || !chord) return false
      if (bracket === "[") foldEnclosing(view)
      else unfoldCode(view)
      // Handled even when there was nothing to fold: the key must never fall
      // through to indentLess.
      return true
    },
  })
)

/**
 * SQL code editor used by Query Studio (and any future SQL-editing surfaces).
 *
 * Wraps CodeMirror with the SQL language extension and follows the active
 * `next-themes` theme so colors stay consistent with the rest of the app.
 *
 * Ketika `schema` diberikan, CodeMirror menyalakan autocomplete untuk nama
 * tabel dan kolom. Tanpa itu editor tetap berjalan seperti sebelumnya —
 * daftar skema diambil asinkron dari katalog, jadi editor tidak boleh
 * menunggu data itu untuk bisa dipakai.
 *
 * @param value Current SQL text
 * @param onChange Called with the new text on each user edit
 * @param schema Tabel dan kolom untuk autocomplete
 * @param className Optional extra wrapper classes
 * @param minHeight Editor min-height (CSS string), default `220px`
 * @param wrap Soft-wrap long lines instead of scrolling sideways; off by
 *   default so Query Studio is unchanged
 * @param onView Called with the editor's view when the editor is created,
 *   so the caller can read the selection and insert text at the cursor
 * @param extensions Extra CodeMirror extensions, appended to the ones this
 *   component builds. The caller decides their order, and keeps the array's
 *   identity: a new array on every render reconfigures the editor on every
 *   keystroke
 */
export function SqlEditor({
  value,
  onChange,
  schema,
  className,
  minHeight = "220px",
  wrap = false,
  onView,
  extensions,
}: {
  value: string
  onChange: (v: string) => void
  schema?: SqlSchema
  className?: string
  minHeight?: string
  wrap?: boolean
  onView?: (view: EditorView) => void
  extensions?: Extension[]
}) {
  const { resolvedTheme } = useTheme()
  const cmTheme = resolvedTheme === "dark" ? "dark" : "light"

  // Ekstensi dibuat ulang hanya saat skema berubah. Membuat array baru tiap
  // render akan memaksa CodeMirror me-rekonfigurasi editor pada setiap
  // ketikan. Extensi dari pemanggil ikut di sini, dalam urutan yang
  // pemanggil pilih.
  const editorExtensions = React.useMemo(
    () => [
      sql(schema ? { schema, upperCaseKeywords: true } : undefined),
      foldKeys,
      ...(wrap ? [EditorView.lineWrapping] : []),
      ...(extensions ?? []),
    ],
    [schema, wrap, extensions]
  )

  return (
    <div
      className={cn(
        "overflow-hidden rounded-md border border-border bg-background [&_.cm-editor]:outline-none [&_.cm-scroller]:font-mono [&_.cm-scroller]:text-sm",
        className
      )}
    >
      <CodeMirror
        value={value}
        height={minHeight}
        extensions={editorExtensions}
        onChange={onChange}
        onCreateEditor={(view) => onView?.(view)}
        theme={cmTheme}
        basicSetup={{
          lineNumbers: true,
          foldGutter: true,
          highlightActiveLine: true,
          autocompletion: true,
        }}
      />
    </div>
  )
}
