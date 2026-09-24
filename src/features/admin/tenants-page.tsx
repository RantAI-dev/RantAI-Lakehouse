"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useAuth } from "@/features/auth/auth-provider"
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService, useServiceAction } from "@/hooks/use-service"
import {
  formatBytes,
  formatCompactNumber,
  formatNumber,
  formatPercent,
} from "@/lib/format"
import { withNotify } from "@/lib/notify"
import { identityService } from "@/services"
import type { Tenant } from "@/services/contracts/identity"
import { getTenantColumns } from "./tenants-columns"

export function TenantsPage() {
  const { hasPermission } = useAuth()
  const canWrite = hasPermission("identity:write")
  const state = useService((s) => identityService.listTenants(s), [])
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

  const columns = React.useMemo(
    () => getTenantColumns({ onSelect: setSelected }),
    []
  )

  const tableUrlState = useTableUrlState()
  const filteredData = React.useMemo(
    () =>
      filterDataClientSide(state.data ?? [], {
        search: tableUrlState.search,
        searchFields: [
          (r) => r.name,
          (r) => r.slug,
          (r) => r.plan,
        ],
        filters: tableUrlState.filters,
        joinOperator: tableUrlState.joinOperator,
      }),
    [state.data, tableUrlState.search, tableUrlState.filters, tableUrlState.joinOperator]
  )

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/admin/tenants",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

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
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch
              placeholder="Search name, slug, plan..."
            />
          </DataTableAdvancedToolbar>
          <div className="rounded-md border">
            <DataTable table={table} />
          </div>
        </div>
      ) : null}
      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="Create Tenant"
        description="Provision a tenant with residency and quota."
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
            placeholder="ap-southeast-1"
          />
        </div>
      </CreateSheet>
      <DetailDrawer
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
        description={selected?.slug}
      >
        {selected ? (
          <MetadataList
            items={[
              { label: "Plan", value: selected.plan },
              { label: "Residency", value: selected.residency },
              { label: "Users", value: formatNumber(selected.users) },
              { label: "Agents", value: formatNumber(selected.agents) },
              { label: "Storage", value: formatBytes(selected.storageBytes) },
              {
                label: "Compute used",
                value: formatCompactNumber(selected.usedCompute),
              },
              {
                label: "Compute quota",
                value: formatCompactNumber(selected.quotaCompute),
              },
              {
                label: "Utilization",
                value:
                  selected.quotaCompute > 0
                    ? formatPercent(selected.usedCompute / selected.quotaCompute)
                    : "—",
              },
            ]}
          />
        ) : null}
      </DetailDrawer>
    </div>
  )
}
