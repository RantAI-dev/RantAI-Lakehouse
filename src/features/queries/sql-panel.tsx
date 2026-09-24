"use client"

import * as React from "react"
import { Play, Save, Square, WandSparkles } from "lucide-react"

import { SqlEditor } from "@/components/sql-editor"
import { formatSql } from "@/lib/sql-format"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { useQueryStudio } from "./use-query-studio"

/**
 * The SQL editor and everything you can do to what is in it.
 *
 * Run is bound to ⌘/Ctrl+Enter as well as the button — in an editor, the
 * keyboard is where the hands already are.
 */
export function SqlPanel({
  studio,
  onSave,
}: {
  readonly studio: ReturnType<typeof useQueryStudio>
  readonly onSave: () => void
}) {
  const {
    sql,
    setSql,
    engine,
    setEngine,
    sqlSchema,
    runAct,
    estimateAct,
    estimatable,
    runQuery,
    reEstimate,
  } = studio
  const running = runAct.status === "pending"

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
      <SqlEditor value={sql} onChange={setSql} schema={sqlSchema} />
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
        <Button
          size="sm"
          variant="outline"
          onClick={() => setSql(formatSql(sql))}
          disabled={!estimatable}
        >
          <WandSparkles className="size-4" aria-hidden />
          Format
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={reEstimate}
          disabled={estimateAct.status === "pending" || !estimatable}
        >
          Estimate
        </Button>
        <span className="ml-auto text-xs text-muted-foreground">
          ⌘/Ctrl+Enter to run
        </span>
      </div>
    </div>
  )
}
