"use client"

import * as React from "react"

import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState } from "@/components/patterns/page-states"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import { insertAsOfClause } from "@/lib/snapshot-picker"
import { lakehouseService } from "@/services"
import { HistoryQuickList, SavedQuickList } from "./query-context-lists"
import { NaturalLanguagePanel } from "./nl-panel"
import { QueryResultsSection } from "./query-results-section"
import { QueryStudioTabs } from "./query-studio-tabs"
import { QueryTransparencyPanel } from "./query-transparency-panel"
import { SaveQuerySheet } from "./save-query-sheet"
import { SqlPanel } from "./sql-panel"
import { useQueryStudio } from "./use-query-studio"

/**
 * Iceberg time-travel controls: the user picks a namespace, then a table
 * (from `lakehouseService.listTables`), and that table's own snapshots
 * (from `getTableDetail`) feed the picker — the table is never inferred
 * from the SQL text itself. Only rendered when the engine is `trino`,
 * since `FOR VERSION AS OF` is meaningless against ClickHouse.
 */
function IcebergTimeTravelControls({
  sql,
  onApply,
}: {
  sql: string
  onApply: (nextSql: string) => void
}) {
  const [namespace, setNamespace] = React.useState<string | null>(null)
  const [table, setTable] = React.useState<string | null>(null)
  const [snapshotId, setSnapshotId] = React.useState<string | null>(null)

  const namespacesState = useService((s) => lakehouseService.listNamespaces(undefined, s), [])
  const tablesState = useService(
    (s) => (namespace ? lakehouseService.listTables(namespace, undefined, s) : Promise.resolve([])),
    [namespace]
  )
  const detailState = useService(
    (s) =>
      namespace && table
        ? lakehouseService.getTableDetail(namespace, table, s)
        : Promise.resolve(null),
    [namespace, table]
  )
  const snapshots = detailState.status === "success" && detailState.data ? detailState.data.snapshots : []

  function handleApply() {
    if (!namespace || !table || !snapshotId) return
    onApply(insertAsOfClause(sql, `${namespace}.${table}`, snapshotId))
  }

  return (
    <div className="flex flex-wrap items-center gap-2 rounded-md border border-dashed border-border p-2 text-xs">
      <span className="font-medium text-muted-foreground">Time travel</span>
      <Select
        value={namespace ?? ""}
        onValueChange={(v) => {
          setNamespace(v || null)
          setTable(null)
          setSnapshotId(null)
        }}
      >
        <SelectTrigger size="sm">
          <SelectValue placeholder="Namespace" />
        </SelectTrigger>
        <SelectContent>
          {namespacesState.status === "success"
            ? namespacesState.data.map((n) => (
                <SelectItem key={n.name} value={n.name}>
                  {n.name}
                </SelectItem>
              ))
            : null}
        </SelectContent>
      </Select>
      <Select
        value={table ?? ""}
        onValueChange={(v) => {
          setTable(v || null)
          setSnapshotId(null)
        }}
        disabled={!namespace}
      >
        <SelectTrigger size="sm">
          <SelectValue placeholder="Table" />
        </SelectTrigger>
        <SelectContent>
          {tablesState.status === "success"
            ? tablesState.data.map((t) => (
                <SelectItem key={t.name} value={t.name}>
                  {t.name}
                </SelectItem>
              ))
            : null}
        </SelectContent>
      </Select>
      <Select value={snapshotId ?? ""} onValueChange={(v) => setSnapshotId(v || null)} disabled={snapshots.length === 0}>
        <SelectTrigger size="sm">
          <SelectValue placeholder="Snapshot" />
        </SelectTrigger>
        <SelectContent>
          {snapshots.map((snap) => (
            <SelectItem key={snap.id} value={snap.id}>
              {snap.id} · {snap.operation}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Button size="sm" variant="outline" onClick={handleApply} disabled={!namespace || !table || !snapshotId}>
        Insert AS OF
      </Button>
    </div>
  )
}

/** Query Studio: natural-language ↔ SQL workspace with execution transparency. */
export function QueryStudioPage() {
  // All the state lives in the hook; this file is the layout.
  const studio = useQueryStudio()
  const [saveOpen, setSaveOpen] = React.useState(false)

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Query Studio"
        description="Ask questions or write SQL, then run it against the hot analytical store."
      />
      <QueryStudioTabs />
      <div className="grid gap-4 xl:grid-cols-[1fr_320px]">
        <div className="min-w-0 space-y-4">
          <Tabs
            value={studio.tab}
            onValueChange={(v) => studio.setTab(v === "sql" ? "sql" : "nl")}
          >
            <TabsList>
              <TabsTrigger value="nl">Natural language</TabsTrigger>
              <TabsTrigger value="sql">SQL</TabsTrigger>
            </TabsList>
            <TabsContent value="nl" className="mt-3">
              <NaturalLanguagePanel studio={studio} />
            </TabsContent>
            <TabsContent value="sql" className="mt-3 space-y-3">
              <SqlPanel studio={studio} onSave={() => setSaveOpen(true)} />
              {/*
               * Iceberg time-travel: inserts a `FOR VERSION AS OF` clause
               * into the SQL text. Kept unconditional even when the
               * engine picker (`SqlPanel`) is set to ClickHouse — the
               * clause is meaningless there, so running it is on the
               * author, the same way writing any Trino-only syntax into
               * the editor is.
               */}
              <IcebergTimeTravelControls sql={studio.sql} onApply={studio.setSql} />
            </TabsContent>
          </Tabs>
          {studio.runAct.status === "error" ? (
            <ErrorState
              error={studio.runAct.error}
              onRetry={() => void studio.runQuery()}
            />
          ) : null}
          {studio.runAct.data ? (
            <QueryResultsSection result={studio.runAct.data} />
          ) : null}
        </div>
        <div className="space-y-3">
          <QueryTransparencyPanel
            state={studio.estimateAct}
            estimatable={studio.estimatable}
            onRetry={studio.reEstimate}
          />
          <SavedQuickList state={studio.savedState} onLoadSql={studio.loadSql} />
          <HistoryQuickList
            state={studio.historyState}
            onLoadSql={studio.loadSql}
          />
        </div>
      </div>

      <SaveQuerySheet
        open={saveOpen}
        onOpenChange={setSaveOpen}
        sql={studio.sql}
        saving={studio.saveAct.status === "pending"}
        error={studio.saveAct.error?.message ?? null}
        onSave={studio.save}
      />
    </div>
  )
}
