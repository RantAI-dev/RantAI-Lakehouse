"use client"

import { useTheme } from "next-themes"
import CodeMirror from "@uiw/react-codemirror"
import { sql } from "@codemirror/lang-sql"
import { cn } from "@/lib/utils"

/**
 * SQL code editor used by Query Studio (and any future SQL-editing surfaces).
 *
 * Wraps CodeMirror with the SQL language extension and follows the active
 * `next-themes` theme so colors stay consistent with the rest of the app.
 *
 * @param value Current SQL text
 * @param onChange Called with the new text on each user edit
 * @param className Optional extra wrapper classes
 * @param minHeight Editor min-height (CSS string), default `220px`
 */
export function SqlEditor({
  value,
  onChange,
  className,
  minHeight = "220px",
}: {
  value: string
  onChange: (v: string) => void
  className?: string
  minHeight?: string
}) {
  const { resolvedTheme } = useTheme()
  const cmTheme = resolvedTheme === "dark" ? "dark" : "light"

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
        extensions={[sql()]}
        onChange={onChange}
        theme={cmTheme}
        basicSetup={{
          lineNumbers: true,
          foldGutter: true,
          highlightActiveLine: true,
        }}
      />
    </div>
  )
}
