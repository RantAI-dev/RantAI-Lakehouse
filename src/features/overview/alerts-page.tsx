"use client"

import * as React from "react"
import Link from "next/link"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import {
  AlertStatusBadge,
  SeverityBadge,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { formatDateTime } from "@/lib/format"
import { overviewService } from "@/services"
import type { AlertItem } from "@/services/contracts/overview"
import { getAlertColumns } from "./alerts-columns"

type AlertResolutionProps = {
  readonly selected: AlertItem
  readonly note: string
  readonly onNoteChange: (val: string) => void
  readonly ackPending: boolean
  readonly resolvePending: boolean
  readonly silencePending: boolean
  readonly actionError: Error | null
  readonly onAcknowledge: (alert: AlertItem) => void
  readonly onResolve: (alert: AlertItem) => void
  readonly onSilence: (alert: AlertItem) => void
}

function AlertResolutionForm(props: AlertResolutionProps) {
  const {
    selected,
    note,
    onNoteChange,
    ackPending,
    resolvePending,
    silencePending,
    actionError,
    onAcknowledge,
    onResolve,
    onSilence,
  } = props

  if (selected.status === "resolved") return null

  return (
    <div className="flex flex-col gap-2">
      <Textarea
        value={note}
        onChange={(e) => onNoteChange(e.target.value)}
        placeholder="Resolution note..."
        aria-label="Resolution note"
      />
      <div className="flex flex-wrap items-center gap-2">
        {selected.status === "open" ? (
          <Button
            variant="outline"
            size="sm"
            disabled={ackPending || resolvePending || silencePending}
            onClick={() => onAcknowledge(selected)}
          >
            {ackPending ? "Acknowledging..." : "Acknowledge"}
          </Button>
        ) : null}
        <Button
          size="sm"
          disabled={ackPending || resolvePending || silencePending || note.trim().length === 0}
          onClick={() => onResolve(selected)}
        >
          {resolvePending ? "Resolving..." : "Resolve"}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={ackPending || resolvePending || silencePending}
          onClick={() => onSilence(selected)}
        >
          {silencePending ? "Silencing..." : "Silence 60m"}
        </Button>
      </div>
      {actionError ? (
        <p className="text-xs text-destructive">{actionError.message}</p>
      ) : null}
    </div>
  )
}

type AlertDrawerProps = {
  readonly selected: AlertItem | null
  readonly onOpenChange: (open: boolean) => void
  readonly note: string
  readonly onNoteChange: (val: string) => void
  readonly ackPending: boolean
  readonly resolvePending: boolean
  readonly silencePending: boolean
  readonly actionError: Error | null
  readonly onAcknowledge: (alert: AlertItem) => void
  readonly onResolve: (alert: AlertItem) => void
  readonly onSilence: (alert: AlertItem) => void
}

function AlertDetailDrawer(props: AlertDrawerProps) {
  const {
    selected,
    onOpenChange,
    note,
    onNoteChange,
    ackPending,
    resolvePending,
    silencePending,
    actionError,
    onAcknowledge,
    onResolve,
    onSilence,
  } = props

  if (!selected) return null

  return (
    <DetailDrawer
      open={selected !== null}
      onOpenChange={onOpenChange}
      title={selected.title}
    >
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
            value: selected.assignee ?? (
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
      <AlertResolutionForm
        selected={selected}
        note={note}
        onNoteChange={onNoteChange}
        ackPending={ackPending}
        resolvePending={resolvePending}
        silencePending={silencePending}
        actionError={actionError}
        onAcknowledge={onAcknowledge}
        onResolve={onResolve}
        onSilence={onSilence}
      />
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
    </DetailDrawer>
  )
}

export function AlertsPage() {
  const state = useService((s) => overviewService.listAlerts(s), [])
  const [selected, setSelected] = React.useState<AlertItem | null>(null)
  const [note, setNote] = React.useState("")
  const tableUrlState = useTableUrlState()

  const ack = useServiceAction((signal, id: string) =>
    overviewService.acknowledgeAlert(id, signal)
  )
  const resolve = useServiceAction((signal, id: string, resolutionNote: string) =>
    overviewService.resolveAlert(id, resolutionNote, signal)
  )
  const silence = useServiceAction((signal, id: string, untilMinutes: number) =>
    overviewService.silenceAlert(id, untilMinutes, signal)
  )

  const openAlert = React.useCallback(
    (alert: AlertItem) => {
      ack.reset()
      resolve.reset()
      silence.reset()
      setNote("")
      setSelected(alert)
    },
    [ack, resolve, silence]
  )

  const onAcknowledge = React.useCallback(
    async (alert: AlertItem) => {
      const updated = await ack.run(alert.id)
      if (updated) {
        setSelected(updated)
        state.reload()
      }
    },
    [ack, state]
  )

  const onResolve = React.useCallback(
    async (alert: AlertItem) => {
      const updated = await resolve.run(alert.id, note.trim())
      if (updated) {
        setSelected(updated)
        state.reload()
      }
    },
    [note, resolve, state]
  )

  const columns = React.useMemo(
    () =>
      getAlertColumns({
        onOpenDetail: openAlert,
        onAcknowledge,
        onResolve: openAlert,
      }),
    [openAlert, onAcknowledge]
  )

  const filteredData = React.useMemo(() => {
    if (state.status !== "success") return []
    return filterDataClientSide(state.data, {
      search: tableUrlState.search,
      searchFields: [
        (r) => r.title,
        (r) => r.source,
        (r) => r.affected,
        (r) => r.assignee ?? "",
      ],
      filters: tableUrlState.filters,
      joinOperator: tableUrlState.joinOperator,
    })
  }, [
    state.status,
    state.data,
    tableUrlState.search,
    tableUrlState.filters,
    tableUrlState.joinOperator,
  ])

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/alerts",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  async function onSilence(alert: AlertItem) {
    const updated = await silence.run(alert.id, 60)
    if (updated) {
      setSelected(updated)
      state.reload()
    }
  }

  const actionError = ack.error ?? resolve.error ?? silence.error

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Alerts"
        description="Acknowledge and investigate platform incidents across ingestion, residency, and agents."
      />

      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="flex flex-col gap-4">
          <DataTableAdvancedToolbar table={table}>
            <DataTableSearch
              placeholder="Search alerts by title, source, or target..."
            />
          </DataTableAdvancedToolbar>
          <DataTable table={table} />
        </div>
      ) : null}

      <AlertDetailDrawer
        selected={selected}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        note={note}
        onNoteChange={setNote}
        ackPending={ack.status === "pending"}
        resolvePending={resolve.status === "pending"}
        silencePending={silence.status === "pending"}
        actionError={actionError}
        onAcknowledge={onAcknowledge}
        onResolve={onResolve}
        onSilence={onSilence}
      />
    </div>
  )
}

export { AlertsPage as PlatformAlertsPage }
