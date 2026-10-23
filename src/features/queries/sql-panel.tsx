"use client"

import * as React from "react"
import { BookOpen, LayoutDashboard, Play, Save, Settings, Square, WandSparkles } from "lucide-react"

import { SqlEditor } from "@/components/sql-editor"
import { formatSql, type KeywordCase } from "@/lib/sql-format"
import { Button } from "@/components/ui/button"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useFormatSettings } from "./use-format-settings"
import type { useQueryStudio } from "./use-query-studio"

/** What the keyword case is called in the settings control. */
const KEYWORD_CASE_LABELS: Record<KeywordCase, string> = {
  upper: "Upper case",
  lower: "Lower case",
  preserve: "As typed",
}

/** What the indent is called in the settings control. */
const INDENT_LABELS: Record<"0" | "2" | "4", string> = {
  "0": "No indent",
  "2": "2 spaces",
  "4": "4 spaces",
}

/** The indent the control picked, or none when it picked something else. */
function indentOf(value: string | null): 0 | 2 | 4 {
  if (value === "2") return 2
  if (value === "4") return 4
  return 0
}

/**
 * The SQL editor and everything you can do to what is in it.
 *
 * Run is bound to ⌘/Ctrl+Enter as well as the button — in an editor, the
 * keyboard is where the hands already are.
 */
export function SqlPanel({
  studio,
  onSave,
  onSaveAsSource,
}: {
  readonly studio: ReturnType<typeof useQueryStudio>
  readonly onSave: () => void
  /** Present only when the principal may author dashboard SQL sources. */
  readonly onSaveAsSource?: () => void
}) {
  const {
    sql,
    setSql,
    engine,
    setEngine,
    completionExtensions,
    runAct,
    estimateAct,
    estimatable,
    runQuery,
    reEstimate,
    explain,
  } = studio
  const running = runAct.status === "pending"
  const explainBusy = explain.busy
  const explainRun = explain.run
  const { settings, setSettings } = useFormatSettings()

  React.useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      // Either modifier, like the command palette and the sidebar
      // shortcut: which one a keyboard has is not worth detecting.
      if (!(event.metaKey || event.ctrlKey) || event.key !== "Enter") return
      event.preventDefault()
      if (!running) void runQuery()
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [runQuery, running])

  return (
    <div className="space-y-3">
      {/* The editor gets the console's own completion source, not the
          language extension's schema: the tables and columns come from the
          catalog, under the names a query can actually use. */}
      <SqlEditor value={sql} onChange={setSql} extensions={completionExtensions} />
      <div className="flex flex-wrap items-center gap-2">
        <Select value={engine} onValueChange={(v) => setEngine(v as typeof engine)}>
          <SelectTrigger size="sm" className="w-32">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="clickhouse">ClickHouse</SelectItem>
            <SelectItem value="trino">Trino</SelectItem>
          </SelectContent>
        </Select>
        <Button
          size="sm"
          onClick={() => void runQuery()}
          disabled={running || !estimatable}
        >
          <Play className="size-4" aria-hidden />
          {running ? "Running…" : "Run query"}
        </Button>
        {running ? (
          <Button size="sm" variant="outline" onClick={runAct.cancel}>
            <Square className="size-4" aria-hidden />
            Stop
          </Button>
        ) : null}
        <Button size="sm" variant="outline" onClick={onSave} disabled={!estimatable}>
          <Save className="size-4" aria-hidden />
          Save query
        </Button>
        {onSaveAsSource ? (
          <Button size="sm" variant="outline" onClick={onSaveAsSource} disabled={!estimatable}>
            <LayoutDashboard className="size-4" aria-hidden />
            Save as SQL source
          </Button>
        ) : null}
        <Button
          size="sm"
          variant="outline"
          onClick={() => setSql(formatSql(sql, settings))}
          disabled={!estimatable}
        >
          <WandSparkles className="size-4" aria-hidden />
          Format
        </Button>
        <Popover>
          <PopoverTrigger asChild>
            <Button size="sm" variant="outline">
              <Settings className="size-4" aria-hidden />
              Format settings
            </Button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-64 gap-3">
            <div className="space-y-1">
              <p className="text-xs font-medium text-muted-foreground">Keyword case</p>
              <Select
                value={settings.keywordCase}
                items={KEYWORD_CASE_LABELS}
                onValueChange={(v) =>
                  setSettings({ ...settings, keywordCase: (v ?? "upper") as KeywordCase })
                }
              >
                <SelectTrigger size="sm" className="w-full" aria-label="Keyword case">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="upper">Upper case</SelectItem>
                  <SelectItem value="lower">Lower case</SelectItem>
                  <SelectItem value="preserve">As typed</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1">
              <p className="text-xs font-medium text-muted-foreground">Indent</p>
              <Select
                value={String(settings.indent)}
                items={INDENT_LABELS}
                onValueChange={(v) => setSettings({ ...settings, indent: indentOf(v) })}
              >
                <SelectTrigger size="sm" className="w-full" aria-label="Indent">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="0">No indent</SelectItem>
                  <SelectItem value="2">2 spaces</SelectItem>
                  <SelectItem value="4">4 spaces</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </PopoverContent>
        </Popover>
        <Button
          size="sm"
          variant="ghost"
          onClick={reEstimate}
          disabled={estimateAct.status === "pending" || !estimatable}
        >
          Estimate
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={() => void explainRun()}
          disabled={explainBusy || !estimatable}
        >
          <BookOpen className="size-4" aria-hidden />
          {explainBusy ? "Explaining…" : "Explain"}
        </Button>
        <span className="ml-auto text-xs text-muted-foreground">
          ⌘/Ctrl+Enter to run
        </span>
      </div>
    </div>
  )
}
