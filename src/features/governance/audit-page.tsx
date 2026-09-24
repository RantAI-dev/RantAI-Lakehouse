"use client"

import * as React from "react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { OutcomeBadge, Pill } from "@/components/patterns/status-badge"
import { useDataTable } from "@/hooks/use-data-table"
import { useService } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { formatCost, formatDateTime } from "@/lib/format"
import {
  ACTOR_KIND_LABEL,
  ENGINE_CATEGORY_LABEL,
} from "@/lib/status"
import { governanceService } from "@/services"
import type { AuditEvent } from "@/services/contracts/governance"
import { getAuditColumns } from "./audit-columns"

function EventDetail({ event }: { readonly event: AuditEvent }) {
  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <OutcomeBadge outcome={event.outcome} />
        <span className="text-xs text-muted-foreground">
          {formatDateTime(event.at)}
        </span>
      </div>
      <MetadataList
        items={[
          {
            label: "Actor",
            value: `${event.actor} (${ACTOR_KIND_LABEL[event.actorKind]})`,
          },
          {
            label: "Delegated actor",
            value: event.delegatedActor ?? "—",
          },
          {
            label: "Tenant",
            value: <span className="font-mono text-xs">{event.tenant}</span>,
          },
          { label: "Action", value: event.action },
          { label: "Resource", value: event.resource },
          { label: "Policy decision", value: event.policyDecision },
          ...(event.engineCategory
            ? [
                {
                  label: "Engine",
                  value: ENGINE_CATEGORY_LABEL[event.engineCategory],
                },
              ]
            : []),
          {
            label: "Estimated cost",
            value: event.estimatedCost != null ? formatCost(event.estimatedCost) : "—",
          },
          {
            label: "Actual cost",
            value: event.actualCost != null ? formatCost(event.actualCost) : "—",
          },
          { label: "Approval", value: event.approvalId ?? "—" },
          { label: "Timestamp", value: formatDateTime(event.at) },
        ]}
      />
      {event.obligations.length > 0 ? (
        <div>
          <p className="text-xs font-medium text-muted-foreground">Obligations</p>
          <div className="mt-1.5 flex flex-wrap gap-1.5">
            {event.obligations.map((o) => (
              <Pill key={o} tone="neutral">{o}</Pill>
            ))}
          </div>
        </div>
      ) : null}
    </>
  )
}

export function AuditPage() {
  const state = useService((s) => governanceService.listAudit(s), [])
  const [selected, setSelected] = React.useState<AuditEvent | null>(null)
  const tableUrlState = useTableUrlState()

  // Correlate ?event=<id> links from other pages: auto-open the drawer once
  // the list has loaded. Read via window.location to stay build-safe.
  React.useEffect(() => {
    if (state.status !== "success") return
    const eventId = new URLSearchParams(window.location.search).get("event")
    if (!eventId) return
    const match = state.data.find((e) => e.id === eventId)
    if (match) setSelected(match)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.status])

  const columns = React.useMemo(
    () => getAuditColumns({ onSelect: setSelected }),
    []
  )

  const filteredData = React.useMemo(() => {
    if (state.status !== "success" || !state.data) return []
    return filterDataClientSide(state.data, {
      search: tableUrlState.search,
      searchFields: [
        (e) => e.actor,
        (e) => e.action,
        (e) => e.resource,
        (e) => e.tenant,
        (e) => e.policyDecision,
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
    persistKey: "/audit",
    initialState: {
      sorting: [{ id: "at", desc: true }],
      columnPinning: { right: ["actions"] },
    },
  })

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Audit"
        description="Immutable events with actor chains, policy decisions, cost, and approvals."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <DataTable table={table}>
          <DataTableAdvancedToolbar table={table}>
            <DataTableSearch placeholder="Search actor, action, resource, tenant…" />
          </DataTableAdvancedToolbar>
        </DataTable>
      ) : null}
      <DetailDrawer
        open={selected != null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected ? selected.action : ""}
        description={selected ? `Audit event ${selected.id}` : undefined}
        wide
      >
        {selected ? <EventDetail event={selected} /> : null}
      </DetailDrawer>
    </div>
  )
}
