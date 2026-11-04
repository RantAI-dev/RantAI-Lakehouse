"use client"

import Link from "next/link"

import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatBytes, formatDateTime, formatRelativeTime } from "@/lib/format"
import { latestSlotReadings } from "@/lib/system-status"
import { governanceService } from "@/services"
import type { ReplicationSlot } from "@/services/contracts/governance"

const STATUS_TONE: Record<string, "success" | "warning" | "destructive"> = {
  ok: "success",
  warning: "warning",
  critical: "destructive",
}

/**
 * A byte count the API sends as a string. One that does not parse is shown
 * as "Not measured", never as `0 B`: a zero WAL backlog is a good reading,
 * and an unreadable one must not pass for it.
 */
function BytesCell({ value }: { value: string }) {
  const n = Number(value)
  if (value === "" || !Number.isFinite(n)) {
    return <span className="text-xs text-muted-foreground">Not measured</span>
  }
  return <span className="font-mono text-xs">{formatBytes(n)}</span>
}

function columns(linkConnector: boolean): ColumnDef<ReplicationSlot>[] {
  const cols: ColumnDef<ReplicationSlot>[] = [
    {
      key: "slotName",
      header: "Slot",
      render: (s) => <span className="font-mono text-xs">{s.slotName}</span>,
    },
    {
      key: "active",
      header: "Connection",
      render: (s) => (
        <Pill tone={s.active ? "success" : "destructive"}>
          {s.active ? "Connected" : "Disconnected"}
        </Pill>
      ),
    },
    {
      key: "walRetainedBytes",
      header: "WAL retained",
      className: "text-right",
      render: (s) => (
        <div className="text-right">
          <BytesCell value={s.walRetainedBytes} />
        </div>
      ),
    },
    {
      key: "confirmedFlushLagBytes",
      header: "Flush lag",
      className: "text-right",
      render: (s) => (
        <div className="text-right">
          <BytesCell value={s.confirmedFlushLagBytes} />
        </div>
      ),
    },
    {
      key: "status",
      header: "Status",
      render: (s) => <Pill tone={STATUS_TONE[s.status] ?? "neutral"}>{s.status || "unknown"}</Pill>,
    },
    {
      key: "checkedAt",
      header: "Checked",
      render: (s) => (
        <span className="text-xs text-muted-foreground" title={formatDateTime(s.checkedAt)}>
          {formatRelativeTime(s.checkedAt)}
        </span>
      ),
    },
  ]
  if (!linkConnector) return cols
  return [
    {
      key: "connectorId",
      header: "Connector",
      render: (s) => (
        <Link
          href={`/connectors/${encodeURIComponent(s.connectorId)}`}
          className="font-mono text-sm text-primary hover:underline"
        >
          {s.connectorId}
        </Link>
      ),
    },
    ...cols,
  ]
}

/**
 * CDC replication-slot health (`WEB-3` S2-D2 section 4 and S2-D3): the one
 * component behind both the System status page (every connector) and a CDC
 * connector's own page (`connectorId` set, its slots only), so the two
 * cannot drift apart (rule 4).
 *
 * Reads `GET /api/governance/replication` (`lake.bronze_meta.
 * replication_slot`, written every 15 minutes by `replication_metrics.py`).
 * That route returns every check, newest first, so each slot is shown once,
 * as last seen (`latestSlotReadings`). A disconnected slot reads "critical"
 * whatever its byte counts, because it still pins WAL on the source (R5).
 *
 * With no slot to show, the section shrinks to `emptyLine`, one muted line,
 * rather than a card around an empty table. On a deployment where no CDC
 * connector has ever run the slot table does not exist; the route answers
 * that with an empty list (`routes/governance.rs`, `WEB-3 review R1`), so
 * it shows the same muted line. The error, with its Retry, is left for a
 * real failure to read the slots.
 */
export function ReplicationSlotHealth({
  connectorId,
  title,
  description,
  emptyLine,
  refreshKey = 0,
}: {
  /** Show this connector's slots only. Unset: every connector, each linked. */
  connectorId?: string
  title: string
  description?: string
  emptyLine: string
  /** Changing it reloads, for a page-level Refresh. */
  refreshKey?: number
}) {
  const state = useService(
    (s) => governanceService.listReplicationSlots(s),
    [refreshKey]
  )

  if (state.status === "success") {
    const rows = latestSlotReadings(state.data).filter(
      (slot) => connectorId === undefined || slot.connectorId === connectorId
    )
    if (rows.length === 0) {
      return <p className="text-sm text-muted-foreground">{emptyLine}</p>
    }
    return (
      <SectionCard title={title} description={description}>
        <DataTable
          columns={columns(connectorId === undefined)}
          rows={rows}
          rowKey={(s) => `${s.connectorId}/${s.slotName}`}
        />
      </SectionCard>
    )
  }

  return (
    <SectionCard title={title} description={description}>
      {state.status === "loading" ? <LoadingSkeleton rows={2} /> : null}
      {state.status === "error" ? (
        <div className="flex flex-col gap-2">
          <ErrorState error={state.error} onRetry={state.reload} className="py-6" />
          <p className="text-xs text-muted-foreground">
            The slot readings could not be read from ClickHouse.
          </p>
        </div>
      ) : null}
    </SectionCard>
  )
}
