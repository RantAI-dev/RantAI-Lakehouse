"use client"

import * as React from "react"
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
import { OutcomeBadge, Pill } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatCost, formatDateTime, formatRelativeTime } from "@/lib/format"
import {
  ACTOR_KIND_LABEL,
  AUDIT_OUTCOME_LABEL,
  ENGINE_CATEGORY_LABEL,
  type ActorKind,
  type AuditOutcome,
} from "@/lib/status"
import { governanceService } from "@/services"
import type { AuditEvent } from "@/services/contracts/governance"

const ACTOR_KIND_OPTIONS = (Object.keys(ACTOR_KIND_LABEL) as ActorKind[]).map(
  (k) => ({ value: k, label: ACTOR_KIND_LABEL[k] })
)

const OUTCOME_OPTIONS = (Object.keys(AUDIT_OUTCOME_LABEL) as AuditOutcome[]).map(
  (o) => ({ value: o, label: AUDIT_OUTCOME_LABEL[o] })
)

const columns: ColumnDef<AuditEvent>[] = [
  { key: "at", header: "When", render: (r) => formatRelativeTime(r.at) },
  { key: "actor", header: "Actor", render: (r) => (
    <div>
      <p>{r.actor}</p>
      <p className="text-xs text-muted-foreground">
        {ACTOR_KIND_LABEL[r.actorKind]}
        {r.delegatedActor ? ` · on behalf of ${r.delegatedActor}` : ""}
      </p>
    </div>
  )},
  { key: "tenant", header: "Tenant", className: "font-mono text-xs", render: (r) => r.tenant },
  { key: "action", header: "Action", render: (r) => r.action },
  { key: "resource", header: "Resource", render: (r) => r.resource },
  { key: "outcome", header: "Outcome", render: (r) => <OutcomeBadge outcome={r.outcome} /> },
  { key: "policy", header: "Policy", render: (r) => r.policyDecision },
  { key: "cost", header: "Cost", render: (r) => (r.actualCost != null ? formatCost(r.actualCost) : "—") },
]

function EventDetail({ event }: { event: AuditEvent }) {
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
  const [search, setSearch] = React.useState("")
  const [actorKind, setActorKind] = React.useState("all")
  const [outcome, setOutcome] = React.useState("all")
  const [selected, setSelected] = React.useState<AuditEvent | null>(null)

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

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((r) => {
      if (actorKind !== "all" && r.actorKind !== actorKind) return false
      if (outcome !== "all" && r.outcome !== outcome) return false
      if (!q) return true
      return [r.actor, r.action, r.resource].some((v) =>
        v.toLowerCase().includes(q)
      )
    })
  }, [state.data, search, actorKind, outcome])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Audit"
        description="Immutable events with actor chains, policy decisions, cost, and approvals."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search actor, action, resource..."
        />
        <FilterSelect
          value={actorKind}
          onChange={setActorKind}
          options={ACTOR_KIND_OPTIONS}
          allLabel="All actor kinds"
          ariaLabel="Filter by actor kind"
        />
        <FilterSelect
          value={outcome}
          onChange={setOutcome}
          options={OUTCOME_OPTIONS}
          allLabel="All outcomes"
          ariaLabel="Filter by outcome"
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" ? (
        <DataTable
          columns={columns}
          rows={filtered}
          rowKey={(r) => r.id}
          onRowClick={setSelected}
        />
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
