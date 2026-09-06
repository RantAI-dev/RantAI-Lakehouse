"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import {
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { MetadataList } from "@/components/patterns/metadata-list"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { formatBytes, formatCompactNumber, formatNumber, formatPercent } from "@/lib/format"
import { identityService } from "@/services"
import type { Tenant } from "@/services/contracts/identity"
import { useAuth } from "@/features/auth/auth-provider"

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
  const { hasPermission } = useAuth()
  const canWrite = hasPermission("identity:write")
  const state = useService((s) => identityService.listTenants(s), [])
  const [search, setSearch] = React.useState("")
  const [selected, setSelected] = React.useState<Tenant | null>(null)
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [slug, setSlug] = React.useState("")
  const [plan, setPlan] = React.useState("")
  const [residency, setResidency] = React.useState("")
  const create = useServiceAction(
    withNotify(
      { success: "Tenant created", error: "Failed to create tenant" },
      (signal, input: Parameters<typeof identityService.createTenant>[0]) =>
        identityService.createTenant(input, signal)
    )
  )

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q) return state.data ?? []
    return (state.data ?? []).filter((t) =>
      [t.name, t.slug, t.plan].some((v) => v.toLowerCase().includes(q))
    )
  }, [state.data, search])

  function resetForm() {
    setName("")
    setSlug("")
    setPlan("")
    setResidency("")
  }

  async function handleCreate() {
    const result = await create.run({
      name: name.trim(),
      slug: slug.trim(),
      plan: plan.trim(),
      residency: residency.trim(),
    })
    if (result) {
      setCreateOpen(false)
      resetForm()
      state.reload()
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Tenants"
        description="Tenant identity, residency, quotas, agents, and storage posture."
        actions={
          <Button
            size="sm"
            onClick={() => setCreateOpen(true)}
            disabled={!canWrite}
            title={canWrite ? undefined : "You don't have permission to create tenants."}
          >
            <PlusIcon data-icon="inline-start" />
            Create Tenant
          </Button>
        }
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
      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="Create Tenant"
        description="Provision a tenant with plan and residency."
        canSubmit={Boolean(
          name.trim() && slug.trim() && plan.trim() && residency.trim()
        )}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
      >
        <div className="space-y-1.5">
          <Label htmlFor="tenant-name">Name</Label>
          <Input
            id="tenant-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="tenant-slug">Slug</Label>
          <Input
            id="tenant-slug"
            value={slug}
            onChange={(e) => setSlug(e.target.value)}
            placeholder="acme"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="tenant-plan">Plan</Label>
          <Input
            id="tenant-plan"
            value={plan}
            onChange={(e) => setPlan(e.target.value)}
            placeholder="Enterprise"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="tenant-residency">Residency</Label>
          <Input
            id="tenant-residency"
            value={residency}
            onChange={(e) => setResidency(e.target.value)}
            placeholder="ID"
          />
        </div>
      </CreateSheet>
    </div>
  )
}
