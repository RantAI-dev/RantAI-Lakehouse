"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import {
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { MetadataList } from "@/components/patterns/metadata-list"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useService } from "@/hooks/use-service"
import { formatBytes, formatCompactNumber, formatNumber, formatPercent } from "@/lib/format"
import { identityService } from "@/services"
import type { Tenant } from "@/services/contracts/identity"

function computeQuota(r: Tenant): string {
  const used = formatCompactNumber(r.usedCompute)
  const quota = formatCompactNumber(r.quotaCompute)
  const utilization =
    r.quotaCompute > 0 ? formatPercent(r.usedCompute / r.quotaCompute) : "—"
  return `${used} / ${quota} (${utilization})`
}

const columns: ColumnDef<Tenant>[] = [
  {
    key: "name",
    header: "Tenant",
    render: (r) => (
      <div>
        <p className="font-medium">{r.name}</p>
        <p className="font-mono text-xs text-muted-foreground">{r.slug}</p>
      </div>
    ),
  },
  { key: "plan", header: "Plan", render: (r) => r.plan },
  { key: "res", header: "Residency", render: (r) => r.residency },
  { key: "users", header: "Users", render: (r) => formatNumber(r.users) },
  { key: "agents", header: "Agents", render: (r) => formatNumber(r.agents) },
  {
    key: "storage",
    header: "Storage",
    render: (r) => formatBytes(r.storageBytes),
  },
  {
    key: "compute",
    header: "Compute quota",
    render: (r) => computeQuota(r),
  },
]

export function TenantsPage() {
  const state = useService((s) => identityService.listTenants(s), [])
  const [search, setSearch] = React.useState("")
  const [selected, setSelected] = React.useState<Tenant | null>(null)

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q) return state.data ?? []
    return (state.data ?? []).filter((t) =>
      [t.name, t.slug, t.plan].some((v) => v.toLowerCase().includes(q))
    )
  }, [state.data, search])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Tenants"
        description="Tenant identity, residency, quotas, agents, and storage posture."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search name, slug, plan..."
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
        description={selected ? `${selected.plan} plan` : undefined}
      >
        {selected ? (
          <MetadataList
            items={[
              {
                label: "Slug",
                value: (
                  <span className="font-mono text-xs">{selected.slug}</span>
                ),
              },
              { label: "Plan", value: selected.plan },
              { label: "Residency", value: selected.residency },
              { label: "Users", value: formatNumber(selected.users) },
              { label: "Agents", value: formatNumber(selected.agents) },
              { label: "Storage", value: formatBytes(selected.storageBytes) },
              { label: "Compute used vs quota", value: computeQuota(selected) },
            ]}
          />
        ) : null}
      </DetailDrawer>
    </div>
  )
}
