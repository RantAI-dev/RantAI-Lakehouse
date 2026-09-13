"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useAuth } from "@/features/auth/auth-provider"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { identityService } from "@/services"
import type { ServiceIdentity } from "@/services/contracts/identity"
import type { DataTableFilterField } from "@/types/data-table"
import { getServiceIdentityColumns } from "./service-identities-columns"

const filterFields: DataTableFilterField<ServiceIdentity>[] = [
  {
    id: "rotationStatus",
    label: "Rotation",
    options: [
      { value: "current", label: "Current" },
      { value: "due", label: "Rotation due" },
      { value: "expired", label: "Expired" },
    ],
  },
]

export function ServiceIdentitiesPage() {
  const { hasPermission } = useAuth()
  const canWrite = hasPermission("identity:write")
  const state = useService((s) => identityService.listServiceIdentities(s), [])
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [scopes, setScopes] = React.useState("")
  const [environment, setEnvironment] = React.useState("")
  const create = useServiceAction(
    withNotify(
      {
        success: "Service identity created",
        error: "Failed to create service identity",
      },
      (
        signal,
        input: Parameters<typeof identityService.createServiceIdentity>[0]
      ) => identityService.createServiceIdentity(input, signal)
    )
  )

  const columns = React.useMemo(() => getServiceIdentityColumns(), [])

  const { table } = useDataTable({
    data: state.data ?? [],
    columns,
    pageCount: 1,
    filterFields,
    enableRowSelection: false,
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
    shallow: false,
    clearOnDefault: true,
  })

  function resetForm() {
    setName("")
    setScopes("")
    setEnvironment("")
  }

  async function handleCreate() {
    const result = await create.run({
      name: name.trim(),
      scopes: scopes
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
      environment: environment.trim(),
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
        title="Service Identities"
        description="Non-human identity, mTLS certs, scoped tokens, and rotation posture."
        actions={
          <Button
            size="sm"
            onClick={() => setCreateOpen(true)}
            disabled={!canWrite}
            title={canWrite ? undefined : "You don't have permission to create service identities."}
          >
            <PlusIcon data-icon="inline-start" />
            Create Service Identity
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
              table={table}
              placeholder="Search identity, scopes..."
              className="h-8 w-40 lg:w-64"
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
        title="Create Service Identity"
        description="Issue a new non-human service identity."
        canSubmit={Boolean(name.trim() && scopes.trim() && environment.trim())}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
      >
        <div className="space-y-1.5">
          <Label htmlFor="si-name">Name</Label>
          <Input id="si-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="si-scopes">Scopes (comma-separated)</Label>
          <Input
            id="si-scopes"
            value={scopes}
            onChange={(e) => setScopes(e.target.value)}
            placeholder="query:read, storage:write"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="si-env">Environment</Label>
          <Input
            id="si-env"
            value={environment}
            onChange={(e) => setEnvironment(e.target.value)}
            placeholder="production"
          />
        </div>
      </CreateSheet>
    </div>
  )
}
    const result = await create.run({
      name: name.trim(),
      scopes: scopes
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
      environment: environment.trim(),
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
        title="Service Identities"
        description="Machine clients, scopes, rotation, and recent use."
        actions={
          <Button
            size="sm"
            onClick={() => setCreateOpen(true)}
            disabled={!canWrite}
            title={canWrite ? undefined : "You don't have permission to create service identities."}
          >
            <PlusIcon data-icon="inline-start" />
            Create Service Identity
          </Button>
        }
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search name, scopes..."
        />
        <FilterSelect
          value={rotation}
          onChange={setRotation}
          options={ROTATION_OPTIONS}
          allLabel="All rotation states"
          ariaLabel="Filter by rotation status"
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <DataTable columns={columns} rows={filtered} rowKey={(r) => r.id} />
      ) : null}
      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="Create Service Identity"
        description="Register a machine client with scopes and environment."
        canSubmit={Boolean(name.trim() && scopes.trim() && environment.trim())}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
      >
        <div className="space-y-1.5">
          <Label htmlFor="si-name">Name</Label>
          <Input id="si-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="si-scopes">Scopes (comma-separated)</Label>
          <Input
            id="si-scopes"
            value={scopes}
            onChange={(e) => setScopes(e.target.value)}
            placeholder="pipelines:write, catalog:read"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="si-env">Environment</Label>
          <Input
            id="si-env"
            value={environment}
            onChange={(e) => setEnvironment(e.target.value)}
            placeholder="production"
          />
        </div>
      </CreateSheet>
    </div>
  )
}
