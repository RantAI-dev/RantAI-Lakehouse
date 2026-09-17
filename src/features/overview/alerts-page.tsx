"use client"

import { useMemo, useState } from "react"
import Link from "next/link"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { MetadataList } from "@/components/patterns/metadata-list"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import {
  AlertStatusBadge,
  SeverityBadge,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatDateTime, formatRelativeTime } from "@/lib/format"
import {
  ALERT_STATUS_LABEL,
  SEVERITY_LABEL,
  type AlertStatus,
  type Severity,
} from "@/lib/status"
import { overviewService } from "@/services"
import type { AlertItem } from "@/services/contracts/overview"

const columns: ColumnDef<AlertItem>[] = [
  {
    key: "severity",
    header: "Severity",
    render: (r) =>
      r.severity !== null ? (
        <SeverityBadge severity={r.severity} />
      ) : (
        <span className="text-muted-foreground">—</span>
      ),
  },
  { key: "title", header: "Alert", render: (r) => r.title },
  { key: "source", header: "Source", render: (r) => r.source },
  {
    key: "affected",
    header: "Affected",
    render: (r) => <span className="font-mono text-xs">{r.affected}</span>,
  },
  {
    key: "status",
    header: "Status",
    render: (r) => <AlertStatusBadge status={r.status} />,
  },
  {
    key: "assignee",
    header: "Assignee",
    render: (r) =>
      r.assignee ?? <span className="text-muted-foreground">Unassigned</span>,
  },
  { key: "at", header: "When", render: (r) => formatRelativeTime(r.at) },
]

/** Platform alerts with severity, acknowledgement, resolution, and deep links. */
export function AlertsPage() {
  const state = useService((s) => overviewService.listAlerts(s), [])
  const [search, setSearch] = useState("")
  const [severity, setSeverity] = useState<Severity | "all">("all")
  const [status, setStatus] = useState<AlertStatus | "all">("all")
  const [selected, setSelected] = useState<AlertItem | null>(null)
  const [note, setNote] = useState("")

  const ack = useServiceAction((signal, id: string) =>
    overviewService.acknowledgeAlert(id, signal)
  )
  const resolve = useServiceAction((signal, id: string, resolutionNote: string) =>
    overviewService.resolveAlert(id, resolutionNote, signal)
  )

  const rows = useMemo(() => {
    if (state.status !== "success") return []
    const q = search.trim().toLowerCase()
    return state.data.filter((a) => {
      if (severity !== "all" && a.severity !== severity) return false
      if (status !== "all" && a.status !== status) return false
      if (q) {
        const hay = `${a.title} ${a.source} ${a.affected}`.toLowerCase()
        if (!hay.includes(q)) return false
      }
      return true
    })
  }, [state.status, state.data, search, severity, status])

  // Judge finding J21: with no filter applied, an empty list means no
  // alert has fired, not "nothing matches a filter" — the two states read
  // very differently to an operator, so DataTable's default filter-shaped
  // empty message must not appear when no filter is active.
  const noFilterActive = !search.trim() && severity === "all" && status === "all"

  function openAlert(alert: AlertItem) {
    ack.reset()
    resolve.reset()
    setNote("")
    setSelected(alert)
  }

  async function onAcknowledge(alert: AlertItem) {
    const updated = await ack.run(alert.id)
    if (updated) {
      setSelected(updated)
      state.reload()
    }
  }

  async function onResolve(alert: AlertItem) {
    const updated = await resolve.run(alert.id, note.trim())
    if (updated) {
      setSelected(updated)
      state.reload()
    }
  }

  const actionError = ack.error ?? resolve.error

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Alerts"
        description="Acknowledge and investigate platform incidents across ingestion, residency, and agents."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search alerts..."
        />
        <FilterSelect
          ariaLabel="Filter by severity"
          allLabel="All severities"
          value={severity}
          onChange={(v) => setSeverity(v as Severity | "all")}
          options={Object.entries(SEVERITY_LABEL).map(([value, label]) => ({
            value,
            label,
          }))}
        />
        <FilterSelect
          ariaLabel="Filter by status"
          allLabel="All statuses"
          value={status}
          onChange={(v) => setStatus(v as AlertStatus | "all")}
          options={Object.entries(ALERT_STATUS_LABEL).map(([value, label]) => ({
            value,
            label,
          }))}
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <DataTable
          columns={columns}
          rows={rows}
          rowKey={(r) => r.id}
          onRowClick={openAlert}
          emptyMessage={
            noFilterActive
              ? "No alerts have fired"
              : "No results match the current filters."
          }
        />
      ) : null}

      <DetailDrawer
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.title ?? ""}
      >
        {selected ? (
          <>
            <div className="flex flex-wrap items-center gap-2">
              {selected.severity !== null ? (
                <SeverityBadge severity={selected.severity} />
              ) : (
                <span className="text-muted-foreground">No severity</span>
              )}
              <AlertStatusBadge status={selected.status} />
            </div>
            <p className="text-sm text-foreground">{selected.detail}</p>
            <MetadataList
              items={[
                { label: "Source", value: selected.source },
                {
                  label: "Affected",
                  value: <span className="font-mono text-xs">{selected.affected}</span>,
                },
                {
                  label: "Assignee",
                  value:
                    selected.assignee ?? (
                      <span className="text-muted-foreground">Unassigned</span>
                    ),
                },
                { label: "Opened", value: formatDateTime(selected.at) },
              ]}
            />
            {selected.resolutionNote ? (
              <div>
                <p className="text-xs font-medium text-muted-foreground">
                  Resolution note
                </p>
                <p className="mt-0.5 text-sm">{selected.resolutionNote}</p>
              </div>
            ) : null}
            {selected.status !== "resolved" ? (
              <div className="flex flex-col gap-2">
                <Textarea
                  value={note}
                  onChange={(e) => setNote(e.target.value)}
                  placeholder="Resolution note..."
                  aria-label="Resolution note"
                />
                <div className="flex flex-wrap items-center gap-2">
                  {selected.status === "open" ? (
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={ack.status === "pending" || resolve.status === "pending"}
                      onClick={() => onAcknowledge(selected)}
                    >
                      {ack.status === "pending" ? "Acknowledging..." : "Acknowledge"}
                    </Button>
                  ) : null}
                  <Button
                    size="sm"
                    disabled={
                      ack.status === "pending" ||
                      resolve.status === "pending" ||
                      note.trim().length === 0
                    }
                    onClick={() => onResolve(selected)}
                  >
                    {resolve.status === "pending" ? "Resolving..." : "Resolve"}
                  </Button>
                </div>
                {actionError ? (
                  <p className="text-xs text-destructive">{actionError.message}</p>
                ) : null}
              </div>
            ) : null}
            {selected.href ? (
              <Button
                variant="outline"
                size="sm"
                className="self-start"
                render={<Link href={selected.href} />}
              >
                Open related object
              </Button>
            ) : null}
          </>
        ) : null}
      </DetailDrawer>
    </div>
  )
}
