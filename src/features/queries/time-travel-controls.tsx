"use client"

import * as React from "react"

import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useService } from "@/hooks/use-service"
import { formatDateTime } from "@/lib/format"
import { msToIso, snapshotsNewestFirst } from "@/lib/lakehouse-view"
import { pinSnapshot, snapshotRetentionText } from "@/lib/snapshot-picker"
import { lakehouseService } from "@/services"

/** What the picker pinned, for the chip that shows it. */
export type PinnedVersion = {
  readonly snapshotId: string
  /** `namespace.table`, as picked. */
  readonly table: string
  /** When the version was written, as an ISO time. */
  readonly at: string
}

/**
 * Iceberg time-travel controls: the user picks a namespace, then a table
 * (from `lakehouseService.listTables`), and that table's own snapshots
 * (from `getTableDetail`) feed the picker. The table only selects whose
 * versions are listed. On ClickHouse the pin is a query-wide setting
 * (`pinSnapshot`), so it is not written next to a table name and applies
 * to every raw table the query reads (`DATA-16` F7). On Trino a version
 * belongs to one table, so the pin is written on that table's own
 * reference — `FROM bronze.orders FOR VERSION AS OF <id>` (`DATA-21` D5).
 *
 * `UX-9`: the controls are the body of Query Studio's "Time travel…" dialog
 * (the ⋯ menu); the pin shows as a chip by the editor once it is in the SQL.
 * Time travel works on both engines (`supported` below); the page comment
 * that said Trino was switched off was stale and went with the old layout.
 */
export function IcebergTimeTravelControls({
  sql,
  engine,
  onApply,
}: {
  sql: string
  engine: string
  onApply: (nextSql: string, pinned: PinnedVersion) => void
}) {
  const [namespace, setNamespace] = React.useState<string | null>(null)
  const [table, setTable] = React.useState<string | null>(null)
  const [snapshotId, setSnapshotId] = React.useState<string | null>(null)
  // A Trino statement that does not read the picked table cannot be pinned;
  // the button says so instead of doing nothing.
  const [notRead, setNotRead] = React.useState<string | null>(null)
  const id = React.useId()
  const supported = engine === "clickhouse" || engine === "trino"
  // The table the version belongs to, as the statement names it: the
  // namespace and the table the picker holds. On Trino the pin is
  // written on that reference (`DATA-21` D5).
  const tableRef = namespace && table ? `${namespace}.${table}` : undefined

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
    if (!snapshotId || !tableRef) return
    const next = pinSnapshot(sql, engine, snapshotId, tableRef)
    if (next === null) {
      setNotRead(tableRef)
      return
    }
    setNotRead(null)
    const snap = snapshots.find((s) => s.id === snapshotId)
    onApply(next, {
      snapshotId,
      table: tableRef,
      at: snap ? msToIso(snap.timestampMs) : "",
    })
  }

  return (
    <div role="group" aria-label="Time travel" className="grid gap-4 text-sm">
      <div className="grid gap-1.5">
        <Label htmlFor={`${id}-namespace`}>Namespace</Label>
        <Select
          value={namespace ?? ""}
          onValueChange={(v) => {
            setNamespace(v || null)
            setTable(null)
            setSnapshotId(null)
            setNotRead(null)
          }}
          disabled={!supported}
        >
          <SelectTrigger id={`${id}-namespace`} className="w-full">
            <SelectValue placeholder="Choose a namespace" />
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
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor={`${id}-table`}>Table</Label>
        <Select
          value={table ?? ""}
          onValueChange={(v) => {
            setTable(v || null)
            setSnapshotId(null)
            setNotRead(null)
          }}
          disabled={!supported || !namespace}
        >
          <SelectTrigger id={`${id}-table`} className="w-full">
            <SelectValue placeholder="Choose a table" />
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
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor={`${id}-version`}>Version</Label>
        <Select
          value={snapshotId ?? ""}
          onValueChange={(v) => {
            setSnapshotId(v || null)
            setNotRead(null)
          }}
          disabled={!supported || snapshots.length === 0}
        >
          <SelectTrigger id={`${id}-version`} className="w-full">
            <SelectValue placeholder="Choose a version" />
          </SelectTrigger>
          <SelectContent>
            {snapshots.map((snap) => (
              <SelectItem key={snap.id} value={snap.id} title={snap.id}>
                {formatDateTime(msToIso(snap.timestampMs))} · {snap.operation}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <p className="text-xs text-muted-foreground">
        {supported
          ? engine === "clickhouse"
            ? "One version applies to every raw table in the query."
            : "The version applies to the table you picked."
          : "Past versions can be queried on ClickHouse and Trino."}
        {supported && snapshots.length > 0 ? ` ${snapshotRetentionText(snapshots)}` : ""}
      </p>
      {notRead ? (
        <p role="alert" className="text-xs text-destructive">
          The query does not read {notRead}: write it in the SQL first.
        </p>
      ) : null}
      <div className="flex justify-end">
        <Button onClick={handleApply} disabled={!supported || !snapshotId}>
          Query this version
        </Button>
      </div>
    </div>
  )
}
