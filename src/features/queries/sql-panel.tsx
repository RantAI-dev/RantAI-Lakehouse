"use client"

import * as React from "react"
import {
  BookOpen,
  Database,
  LayoutDashboard,
  Play,
  Save,
  Settings,
  Square,
  WandSparkles,
} from "lucide-react"

import { SqlEditor } from "@/components/sql-editor"
import { formatSql, type KeywordCase } from "@/lib/sql-format"
import { formatDuration } from "@/lib/format"
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
import { KeyboardShortcutsDialog } from "./keyboard-shortcuts-dialog"
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
  onToggleSchema,
  schemaOpen = false,
}: {
  readonly studio: ReturnType<typeof useQueryStudio>
  readonly onSave: () => void
  /** Present only when the principal may author dashboard SQL sources. */
  readonly onSaveAsSource?: () => void
  /** Present when the page has a schema browser to open and close. */
  readonly onToggleSchema?: () => void
  readonly schemaOpen?: boolean
}) {
  const {
    sql,
    setSql,
    engine,
    setEngine,
    editorExtensions,
    statements,
    running,
    stopRun,
    elapsedMs,
    download,
    estimateAct,
    runnable,
    targetStatement,
    runQuery,
    runSelectionOrAll,
    reEstimate,
    explain,
  } = studio
  const explainBusy = explain.busy
  const explainRun = explain.run
  const { settings, setSettings } = useFormatSettings()

  // The actions that work on one statement are disabled without one: the
  // button's title says what to do about it.
  const oneStatement = targetStatement !== null
  const manyStatements = statements.length > 1

  React.useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      // Either modifier, like the command palette and the sidebar
      // shortcut: which one a keyboard has is not worth detecting.
      if (!(event.metaKey || event.ctrlKey) || event.key !== "Enter") return
      // The editor's own binding runs the selection and marks the key as
      // handled. Running the batch here as well would run it twice.
      if (event.defaultPrevented) return
      event.preventDefault()
      if (!running) void runSelectionOrAll()
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [runSelectionOrAll, running])

  return (
    <div className="space-y-3">
      {/* The editor gets the console's own completion source, not the
          language extension's schema: the tables and columns come from the
          catalog, under the names a query can actually use. */}
      <SqlEditor
        value={sql}
        onChange={setSql}
        extensions={editorExtensions}
        onView={(view) => studio.setEditorView(view)}
      />
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
          disabled={running || !runnable}
        >
          <Play className="size-4" aria-hidden />
          {running ? "Running…" : manyStatements ? "Run all" : "Run query"}
        </Button>
        {/* Stop is here for a run and for a download: a download is a query
            on the engine as well, and the person who presses Stop means it
            either way. */}
        {running || download.busy !== null ? (
          <Button
            size="sm"
            variant="outline"
            onClick={stopRun}
            title={
              running
                ? engine === "trino"
                  ? "Stops the page waiting; the query keeps running on Trino."
                  : "Ends the query on the engine and stops the page waiting."
                : "Stops the download and ends the query on the engine."
            }
          >
            <Square className="size-4" aria-hidden />
            Stop
          </Button>
        ) : null}
        {running ? (
          <span
            role="timer"
            aria-label="Elapsed time"
            className="text-xs tabular-nums text-muted-foreground"
          >
            {formatDuration(elapsedMs ?? 0)}
          </span>
        ) : null}
        <Button
          size="sm"
          variant="outline"
          onClick={onSave}
          disabled={!oneStatement}
          title={oneStatement ? undefined : "Select one statement"}
        >
          <Save className="size-4" aria-hidden />
          Save query
        </Button>
        {onSaveAsSource ? (
          <Button
            size="sm"
            variant="outline"
            onClick={onSaveAsSource}
            disabled={!oneStatement}
            title={oneStatement ? undefined : "Select one statement"}
          >
            <LayoutDashboard className="size-4" aria-hidden />
            Save as SQL source
          </Button>
        ) : null}
        <Button
          size="sm"
          variant="outline"
          onClick={() => setSql(formatSql(sql, settings))}
          disabled={!runnable}
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
        <KeyboardShortcutsDialog />
        {onToggleSchema ? (
          <Button
            size="sm"
            variant="outline"
            onClick={onToggleSchema}
            aria-pressed={schemaOpen}
          >
            <Database className="size-4" aria-hidden />
            Schema
          </Button>
        ) : null}
        <Button
          size="sm"
          variant="ghost"
          onClick={reEstimate}
          disabled={estimateAct.status === "pending" || !oneStatement}
          title={oneStatement ? undefined : "Select one statement"}
        >
          Estimate
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={() => void explainRun()}
          disabled={explainBusy || !oneStatement}
          title={oneStatement ? undefined : "Select one statement"}
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
