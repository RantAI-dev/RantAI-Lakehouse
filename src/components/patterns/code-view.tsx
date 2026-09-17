"use client"

import { useTheme } from "next-themes"
import CodeMirror from "@uiw/react-codemirror"
import { python } from "@codemirror/lang-python"
import { cn } from "@/lib/utils"

/**
 * Read-only Python source viewer for `GET /api/pipelines/{id}/source` —
 * `SqlEditor`'s (`src/components/sql-editor.tsx`) read-only, Python twin:
 * same bundled CodeMirror, same `next-themes` wiring, same wrapper classes.
 * No CDN dependency: this repo's on-prem/possibly-offline deployment
 * posture rules out a `highlight.js` CDN script (AGENTS.md, WS4 grand plan
 * §6 Correction 2) — `@codemirror/lang-python` ships as a bundled npm
 * dependency alongside `@codemirror/lang-sql`, which `SqlEditor` already
 * uses the identical way.
 *
 * @param text The op's source text (already resolved server-side).
 * @param className Optional extra wrapper classes.
 */
export function CodeView({ text, className }: { text: string; className?: string }) {
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
        value={text}
        extensions={[python()]}
        editable={false}
        theme={cmTheme}
        basicSetup={{ lineNumbers: true, foldGutter: true, highlightActiveLine: false }}
      />
    </div>
  )
}
