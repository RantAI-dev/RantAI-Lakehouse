"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { identityService } from "@/services"
import type { ServiceIdentity } from "@/services/contracts/identity"

const ROTATION_OPTIONS = [
  { value: "current", label: "Current" },
  { value: "due", label: "Rotation due" },
  { value: "expired", label: "Expired" },
]

function RotationPill({
  status,
}: {
  status: ServiceIdentity["rotationStatus"]
}) {
  switch (status) {
    case "current":
      return <Pill tone="success">Current</Pill>
    case "due":
      return <Pill tone="warning">Rotation due</Pill>
    case "expired":
      return <Pill tone="destructive">Expired</Pill>
  }
}

const columns: ColumnDef<ServiceIdentity>[] = [
  { key: "name", header: "Identity", render: (r) => r.name },
  {
    key: "scopes",
    header: "Scopes",
    render: (r) => (
      <div className="flex flex-wrap gap-1">
        {r.scopes.map((scope) => (
          <Pill key={scope} tone="neutral" className="font-mono">
            {scope}
          </Pill>
        ))}
      </div>
    ),
  },
  { key: "env", header: "Environment", render: (r) => r.environment },
  {
    key: "rot",
    header: "Rotation",
    render: (r) => <RotationPill status={r.rotationStatus} />,
  },
  {
    key: "exp",
    header: "Expires",
    render: (r) => formatRelativeTime(r.expiresAt),
  },
  {
    key: "used",
    header: "Last used",
    render: (r) => formatRelativeTime(r.lastUsedAt),
  },
]

export function ServiceIdentitiesPage() {
  const state = useService((s) => identityService.listServiceIdentities(s), [])
  const [search, setSearch] = React.useState("")
  const [rotation, setRotation] = React.useState("all")
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [scopes, setScopes] = React.useState("")
  const [environment, setEnvironment] = React.useState("")
  const create = useServiceAction(
    (
      signal,
      input: Parameters<typeof identityService.createServiceIdentity>[0]
    ) => identityService.createServiceIdentity(input, signal)
  )

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((si) => {
      if (rotation !== "all" && si.rotationStatus !== rotation) return false
      if (!q) return true
      return (
        si.name.toLowerCase().includes(q) ||
        si.scopes.some((scope) => scope.toLowerCase().includes(q))
      )
    })
  }, [state.data, search, rotation])

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
        description="Machine clients, scopes, rotation, and recent use."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
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
