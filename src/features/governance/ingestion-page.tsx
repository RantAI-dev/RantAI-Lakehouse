"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { FilterToolbar, SearchField } from "@/components/patterns/filter-toolbar"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Pill } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatBytes, formatRelativeTime } from "@/lib/format"
import { governanceService } from "@/services"
import type { ReplicationSlot } from "@/services/contracts/governance"

const STATUS_TONE: Record<string, "success" | "warning" | "destructive" | "neutral"> = {
  ok: "success",
  warning: "warning",
  critical: "destructive",
}

const columns: ColumnDef<ReplicationSlot>[] = [
  { key: "connector", header: "Connector", render: (r) => r.connectorId },
  { key: "slot", header: "Replication slot", className: "font-mono text-xs", render: (r) => r.slotName },
  {
    key: "active",
    header: "Active",
    render: (r) => (
      <Pill tone={r.active ? "success" : "destructive"}>
        {r.active ? "connected" : "disconnected"}
      </Pill>
    ),
  },
  {
    key: "wal",
    header: "WAL retained",
    render: (r) => formatBytes(Number(r.walRetainedBytes) || 0),
  },
  {
    key: "lag",
    header: "Flush lag",
    render: (r) => formatBytes(Number(r.confirmedFlushLagBytes) || 0),
  },
  {
    key: "status",
    header: "Status",
    render: (r) => <Pill tone={STATUS_TONE[r.status] ?? "neutral"}>{r.status}</Pill>,
  },
  { key: "checked", header: "Checked", render: (r) => formatRelativeTime(r.checkedAt) },
]

/**
 * CDC ingestion health (P5/P6): one row per Postgres logical-replication
 * slot backing a Debezium Server connector into Bronze. Reads `GET
 * /api/governance/replication`, which surfaces `lake.bronze_meta.
 * replication_slot` — written every 15 minutes by `dagster/
 * dispar_orchestrate/replication_metrics.py`'s `replication_slot_check_job`
 * (R5: a lagging or disconnected slot pins WAL and can fill the source
 * database's disk; a disconnected slot is flagged "critical" regardless of
 * byte thresholds, because it still pins WAL indefinitely).
 *
 * This is deliberately not called "Streaming" — there is no Kafka/Flink/
 * streaming engine in this stack. CDC is a batch-of-one-per-transaction
 * replication pipe into Bronze Iceberg, which is what this page monitors.
 */
export function IngestionPage() {
  const state = useService((s) => governanceService.listReplicationSlots(s), [])
  const [search, setSearch] = React.useState("")

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q) return state.data ?? []
    return (state.data ?? []).filter((r) =>
      [r.connectorId, r.slotName].some((v) => v.toLowerCase().includes(q))
    )
  }, [state.data, search])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Ingestion (CDC)"
        description="Postgres logical-replication slot health per CDC connector — WAL retention, flush lag, and connection state. Not a streaming engine: this is Debezium Server capturing row-level changes into Bronze Iceberg."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search connector or slot..."
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <DataTable columns={columns} rows={filtered} rowKey={(r) => `${r.connectorId}-${r.slotName}`} />
      ) : null}
    </div>
  )
}
