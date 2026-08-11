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
import { HealthBadge, Pill } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatPercent } from "@/lib/format"
import { HEALTH_LABEL, type Health } from "@/lib/status"
import { opsService } from "@/services"
import type { PlatformService } from "@/services/contracts/ops"

const HEALTH_OPTIONS = (Object.keys(HEALTH_LABEL) as Health[]).map((h) => ({
  value: h,
  label: HEALTH_LABEL[h],
}))

function DependencyPills({ dependencies }: { dependencies: string[] }) {
  if (dependencies.length === 0) return <span>—</span>
  return (
    <div className="flex flex-wrap gap-1">
      {dependencies.map((dep) => (
        <Pill key={dep} tone="neutral">
          {dep}
        </Pill>
      ))}
    </div>
  )
}

const columns: ColumnDef<PlatformService>[] = [
  {
    key: "name",
    header: "Service",
    render: (r) => (
      <div>
        <p className="font-medium">{r.name}</p>
        <p className="text-xs text-muted-foreground">
          v{r.version} · {r.site}
        </p>
      </div>
    ),
  },
  {
    key: "health",
    header: "Health",
    render: (r) => <HealthBadge health={r.health} />,
  },
  { key: "replicas", header: "Replicas", render: (r) => r.replicas },
  {
    key: "err",
    header: "Error rate",
    render: (r) => formatPercent(r.errorRate),
  },
  {
    key: "lat",
    header: "Latency",
    render: (r) => <span className="font-mono text-xs">{r.latencyMs} ms</span>,
  },
  {
    key: "deps",
    header: "Dependencies",
    render: (r) => <DependencyPills dependencies={r.dependencies} />,
  },
]

export function ServicesPage() {
  const state = useService((s) => opsService.listServices(s), [])
  const [search, setSearch] = React.useState("")
  const [health, setHealth] = React.useState("all")
  const [selected, setSelected] = React.useState<PlatformService | null>(null)

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((svc) => {
      if (health !== "all" && svc.health !== health) return false
      if (!q) return true
      return [svc.name, svc.site].some((v) => v.toLowerCase().includes(q))
    })
  }, [state.data, search, health])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Services"
        description="Platform service health, versions, sites, and dependencies."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search name, site..."
        />
        <FilterSelect
          value={health}
          onChange={setHealth}
          options={HEALTH_OPTIONS}
          allLabel="All health states"
          ariaLabel="Filter by health"
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <DataTable
          columns={columns}
          rows={filtered}
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
        description={selected ? `Platform service on ${selected.site}` : undefined}
      >
        {selected ? (
          <>
            <HealthBadge health={selected.health} className="self-start" />
            <MetadataList
              items={[
                {
                  label: "Version",
                  value: (
                    <span className="font-mono text-xs">v{selected.version}</span>
                  ),
                },
                { label: "Site", value: selected.site },
                { label: "Replicas", value: selected.replicas },
                { label: "Error rate", value: formatPercent(selected.errorRate) },
                {
                  label: "Latency",
                  value: (
                    <span className="font-mono text-xs">
                      {selected.latencyMs} ms
                    </span>
                  ),
                },
                {
                  label: "Dependencies",
                  value: <DependencyPills dependencies={selected.dependencies} />,
                },
              ]}
            />
          </>
        ) : null}
      </DetailDrawer>
    </div>
  )
}
