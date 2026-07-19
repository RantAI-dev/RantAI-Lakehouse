"use client"

import { useCallback, useEffect, useRef, useState } from "react"
import { useTheme } from "next-themes"
import CodeMirror from "@uiw/react-codemirror"
import { sql as sqlLang } from "@codemirror/lang-sql"
import {
  Check,
  Copy,
  Loader2,
  Play,
  RefreshCw,
  SquareArrowOutUpRight,
  Terminal,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

/**
 * Read-only, syntax-highlighted SQL block used inside the Natural Language
 * chat. Renders the generated query with a header (label + copy) and an
 * action toolbar: Run Query, Open in SQL Editor, Copy, and Regenerate.
 *
 * Highlighting uses the same CodeMirror setup as the main `SqlEditor`, so the
 * generated SQL looks identical to what the user will see after opening it in
 * the editor tab.
 */
export function SqlCodeBlock({
  sql,
  onRun,
  onOpenInEditor,
  onRegenerate,
  running = false,
  disabled = false,
  className,
}: {
  sql: string
  onRun: () => void
  onOpenInEditor: () => void
  onRegenerate: () => void
  /** True while this query is being executed from the chat. */
  running?: boolean
  /** Disables all actions (e.g. while the assistant is generating). */
  disabled?: boolean
  className?: string
}) {
  const { resolvedTheme } = useTheme()
  const [copied, setCopied] = useState(false)
  const copiedTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    return () => {
      if (copiedTimeoutRef.current) clearTimeout(copiedTimeoutRef.current)
    }
  }, [])

  const copySql = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(sql)
      setCopied(true)
      if (copiedTimeoutRef.current) clearTimeout(copiedTimeoutRef.current)
      copiedTimeoutRef.current = setTimeout(() => setCopied(false), 1500)
    } catch {
      /* clipboard unavailable — ignore */
    }
  }, [sql])

  return (
    <div
      className={cn(
        "overflow-hidden rounded-lg border border-border bg-card shadow-sm",
        className
      )}
      data-name="sqlCodeBlock"
    >
      <div className="flex items-center justify-between gap-2 border-b border-border bg-muted/40 py-1.5 pl-3 pr-1.5">
        <div className="flex min-w-0 items-center gap-2">
          <Terminal className="size-3.5 shrink-0 text-primary" aria-hidden />
          <span className="truncate text-xs font-medium tracking-wide text-primary">
            Generated SQL
          </span>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label={copied ? "Copied" : "Copy SQL"}
          onClick={copySql}
        >
          {copied ? (
            <Check className="size-3.5 text-primary" />
          ) : (
            <Copy className="size-3.5" />
          )}
        </Button>
      </div>

      <div className="[&_.cm-editor]:bg-transparent [&_.cm-editor]:outline-none [&_.cm-scroller]:font-mono [&_.cm-scroller]:text-xs [&_.cm-scroller]:leading-relaxed">
        <CodeMirror
          value={sql}
          editable={false}
          extensions={[sqlLang()]}
          theme={resolvedTheme === "dark" ? "dark" : "light"}
          maxHeight="280px"
          basicSetup={{
            lineNumbers: true,
            foldGutter: false,
            highlightActiveLine: false,
            highlightActiveLineGutter: false,
          }}
        />
      </div>

      <div className="flex flex-wrap items-center gap-1.5 border-t border-border bg-muted/40 px-2 py-1.5">
        <Button
          type="button"
          size="sm"
          onClick={onRun}
          disabled={disabled || running}
        >
          {running ? (
            <Loader2 className="animate-spin" aria-hidden />
          ) : (
            <Play aria-hidden />
          )}
          Run Query
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={onOpenInEditor}
          disabled={disabled}
        >
          <SquareArrowOutUpRight aria-hidden />
          Open in SQL Editor
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          onClick={copySql}
          disabled={disabled}
        >
          {copied ? <Check aria-hidden /> : <Copy aria-hidden />}
          {copied ? "Copied" : "Copy"}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          onClick={onRegenerate}
          disabled={disabled || running}
        >
          <RefreshCw aria-hidden />
          Regenerate
        </Button>
      </div>
    </div>
  )
}
