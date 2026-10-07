"use client"

import * as React from "react"
import { useTheme } from "next-themes"
import CodeMirror, { EditorView } from "@uiw/react-codemirror"
import { sql } from "@codemirror/lang-sql"
import { cn } from "@/lib/utils"

/**
 * Skema yang dipakai untuk autocomplete: nama tabel beserta kolomnya.
 * Bentuknya sengaja sama dengan yang diminta `@codemirror/lang-sql`.
 */
export type SqlSchema = Record<string, string[]>

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
 */
export function SqlEditor({
  value,
  onChange,
  schema,
  className,
  minHeight = "220px",
  wrap = false,
}: {
  value: string
  onChange: (v: string) => void
  schema?: SqlSchema
  className?: string
  minHeight?: string
  wrap?: boolean
}) {
  const { resolvedTheme } = useTheme()
  const cmTheme = resolvedTheme === "dark" ? "dark" : "light"

  // Ekstensi dibuat ulang hanya saat skema berubah. Membuat array baru tiap
  // render akan memaksa CodeMirror me-rekonfigurasi editor pada setiap
  // ketikan.
  const extensions = React.useMemo(
    () => [
      sql(schema ? { schema, upperCaseKeywords: true } : undefined),
      ...(wrap ? [EditorView.lineWrapping] : []),
    ],
    [schema, wrap]
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
        extensions={extensions}
        onChange={onChange}
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
