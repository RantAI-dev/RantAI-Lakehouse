"use client"

import { useMemo, useState } from "react"
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
import { HealthBadge, Pill } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { HEALTH_LABEL, type Health } from "@/lib/status"
import { connectorService } from "@/services"
import type { Connector } from "@/services/contracts/connectors"

type Direction = Connector["direction"]

const DIRECTION_LABEL: Record<Direction, string> = {
  source: "Source",
  sink: "Sink",
  bidirectional: "Bidirectional",
}

const columns: ColumnDef<Connector>[] = [
  {
    key: "name",
    header: "Connector",
    render: (r) => (
      <div>
        <p className="font-medium">{r.name}</p>
        <p className="text-xs text-muted-foreground">{r.type}</p>
      </div>
    ),
  },
  {
    key: "dir",
    header: "Direction",
    render: (r) => DIRECTION_LABEL[r.direction],
  },
  {
    key: "health",
    header: "Health",
    render: (r) => <HealthBadge health={r.health} />,
  },
  { key: "env", header: "Environment", render: (r) => r.environment },
  { key: "tenant", header: "Tenant", render: (r) => r.tenant },
  {
    key: "test",
    header: "Last test",
    render: (r) => (
      <span className="text-muted-foreground">
        {formatRelativeTime(r.lastTestAt)}
      </span>
    ),
  },
  {
    key: "activity",
    header: "Last activity",
    render: (r) => (
      <span className="text-muted-foreground">
        {formatRelativeTime(r.lastActivityAt)}
      </span>
    ),
  },
]

/** Drawer body — fetches full connector detail for the selected row. */
function ConnectorDetail({ id }: { id: string }) {
  const state = useService((s) => connectorService.getConnector(id, s), [id])

  if (state.status === "loading") return <LoadingSkeleton rows={4} />
  if (state.status === "error")
    return <ErrorState error={state.error} onRetry={state.reload} />
  const c = state.data

  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <HealthBadge health={c.health} />
        <Pill tone="neutral">{DIRECTION_LABEL[c.direction]}</Pill>
      </div>
      <MetadataList
        items={[
          { label: "Type", value: c.type },
          { label: "Direction", value: DIRECTION_LABEL[c.direction] },
          { label: "Environment", value: c.environment },
          { label: "Tenant", value: c.tenant },
          { label: "Owner", value: c.owner },
          { label: "Last test", value: formatRelativeTime(c.lastTestAt) },
          { label: "Last activity", value: formatRelativeTime(c.lastActivityAt) },
          { label: "Discovered assets", value: c.discoveredAssets },
        ]}
      />
      <div>
        <p className="text-xs font-medium text-muted-foreground">Capabilities</p>
        <div className="mt-1.5 flex flex-wrap gap-1.5">
          {c.capabilities.map((cap) => (
            <Pill key={cap} tone="neutral">
              {cap}
            </Pill>
          ))}
        </div>
      </div>
      <div>
        <p className="text-xs font-medium text-muted-foreground">Recent errors</p>
        {c.recentErrors.length === 0 ? (
          <p className="mt-1 text-sm text-muted-foreground">No recent errors.</p>
        ) : (
          <ul className="mt-1 space-y-1.5">
            {c.recentErrors.map((err, i) => (
              <li key={i} className="text-sm text-destructive">
                {err.message}
                <span className="ml-2 text-xs text-muted-foreground">
                  {formatRelativeTime(err.at)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
      <div>
        <p className="text-xs font-medium text-muted-foreground">
          Dependent pipelines
        </p>
        {c.dependentPipelines.length === 0 ? (
          <p className="mt-1 text-sm text-muted-foreground">
            No dependent pipelines.
          </p>
        ) : (
          <ul className="mt-1 space-y-1">
            {c.dependentPipelines.map((p) => (
              <li key={p} className="font-mono text-sm">
                {p}
              </li>
            ))}
          </ul>
        )}
      </div>
    </>
  )
}

export function ConnectorsPage() {
  const state = useService((s) => connectorService.listConnectors(s), [])
  const [search, setSearch] = useState("")
  const [direction, setDirection] = useState<Direction | "all">("all")
  const [health, setHealth] = useState<Health | "all">("all")
  const [selected, setSelected] = useState<Connector | null>(null)

  const rows = useMemo(() => {
    if (state.status !== "success") return []
    const q = search.trim().toLowerCase()
    return state.data.filter((c) => {
      if (direction !== "all" && c.direction !== direction) return false
      if (health !== "all" && c.health !== health) return false
      if (q) {
        const hay = `${c.name} ${c.type} ${c.tenant} ${c.owner}`.toLowerCase()
        if (!hay.includes(q)) return false
      }
      return true
    })
  }, [state.status, state.data, search, direction, health])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Connectors"
        description="Sources and sinks for CDC, messaging, object storage, SaaS, and federation."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search connectors..."
        />
        <FilterSelect
          ariaLabel="Filter by direction"
          allLabel="All directions"
          value={direction}
          onChange={(v) => setDirection(v as Direction | "all")}
          options={Object.entries(DIRECTION_LABEL).map(([value, label]) => ({
            value,
            label,
          }))}
        />
        <FilterSelect
          ariaLabel="Filter by health"
          allLabel="All health"
          value={health}
          onChange={(v) => setHealth(v as Health | "all")}
          options={Object.entries(HEALTH_LABEL).map(([value, label]) => ({
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
          onRowClick={setSelected}
        />
      ) : null}

      <DetailDrawer
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
        description={selected?.type}
      >
        {selected ? <ConnectorDetail id={selected.id} /> : null}
      </DetailDrawer>
    </div>
  )
}
