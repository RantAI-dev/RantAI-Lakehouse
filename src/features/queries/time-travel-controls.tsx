"use client"

import * as React from "react"

import { Button } from "@/components/ui/button"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useService } from "@/hooks/use-service"
import { formatDateTime } from "@/lib/format"
import { msToIso, snapshotsNewestFirst } from "@/lib/lakehouse-view"
import { pinSnapshot, snapshotRetentionText } from "@/lib/snapshot-picker"
import { lakehouseService } from "@/services"

/**
 * Iceberg time-travel controls: the user picks a namespace, then a table
 * (from `lakehouseService.listTables`), and that table's own snapshots
 * (from `getTableDetail`) feed the picker. The table only selects whose
 * versions are listed: the pin is a query-wide ClickHouse setting
 * (`pinSnapshot`), so it is not written next to a table name and applies to
 * every raw table the query reads (`DATA-16` F7). Past versions are
 * queried on ClickHouse only; on Trino the controls are disabled and say
 * why (`DATA-16` F5, `DATA-21`).
 */
export function IcebergTimeTravelControls({
  sql,
  engine,
  onApply,
}: {
  sql: string
  engine: string
  onApply: (nextSql: string) => void
}) {
  const [namespace, setNamespace] = React.useState<string | null>(null)
  const [table, setTable] = React.useState<string | null>(null)
  const [snapshotId, setSnapshotId] = React.useState<string | null>(null)
  const supported = engine === "clickhouse"

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
  const snapshots =
    detailState.status === "success" && detailState.data
      ? snapshotsNewestFirst(detailState.data.snapshots)
      : []

  function handleApply() {
    if (!snapshotId) return
    const next = pinSnapshot(sql, engine, snapshotId)
    if (next !== null) onApply(next)
  }

  return (
    <div className="flex flex-col gap-1.5 rounded-md border border-dashed border-border p-2 text-xs">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium text-muted-foreground">Time travel</span>
        <Select
          value={namespace ?? ""}
          onValueChange={(v) => {
            setNamespace(v || null)
            setTable(null)
            setSnapshotId(null)
          }}
          disabled={!supported}
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
          disabled={!supported || !namespace}
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
        <Select
          value={snapshotId ?? ""}
          onValueChange={(v) => setSnapshotId(v || null)}
          disabled={!supported || snapshots.length === 0}
        >
          <SelectTrigger size="sm">
            <SelectValue placeholder="Version" />
          </SelectTrigger>
          <SelectContent>
            {snapshots.map((snap) => (
              <SelectItem key={snap.id} value={snap.id} title={snap.id}>
                {formatDateTime(msToIso(snap.timestampMs))} · {snap.operation}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button size="sm" variant="outline" onClick={handleApply} disabled={!supported || !snapshotId}>
          Query this version
        </Button>
      </div>
      <p className="text-muted-foreground">
        {supported
          ? "One version applies to every raw table in the query."
          : "Past versions can be queried on ClickHouse only."}
        {supported && snapshots.length > 0 ? ` ${snapshotRetentionText(snapshots)}` : ""}
      </p>
    </div>
  )
}
