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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useAuth } from "@/features/auth/auth-provider"
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { identityService } from "@/services"
import { getServiceIdentityColumns } from "./service-identities-columns"

export function ServiceIdentitiesPage() {
  const { hasPermission } = useAuth()
  const canWrite = hasPermission("identity:write")
  const state = useService((s) => identityService.listServiceIdentities(s), [])
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [scopes, setScopes] = React.useState("")
  const [environment, setEnvironment] = React.useState("")
  // The rotated secret is shown in the
  // dialog below exactly once and is NEVER persisted to localStorage,
  // sessionStorage, or any service-layer cache entry. Kept as local
  // component state, cleared when the dialog closes, so re-renders of the
  // table (filter/search input, a refetch landing) cannot leak it via any
  // hook that stores its argument.
  const [revealedSecret, setRevealedSecret] = React.useState<string | null>(null)
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
  const rotate = useServiceAction(
    (signal, id: string) => identityService.rotateServiceIdentity(id, signal),
  )

  const handleRotate = React.useCallback(
    async (id: string) => {
      // `useServiceAction.run` resolves to `null` on any thrown error (the
      // service layer classifies upstream failures into ServiceError; see
      // use-service.ts:99-105). A failed rotate MUST NOT open the dialog
      // and MUST NOT refetch the list — a stale-row-refetch here would make
      // a 5xx look like a success, so the `if (result)` guard is the
      // single source of truth for "the rotation actually returned a
      // secret".
      const result = await rotate.run(id)
      if (!result) return
      setRevealedSecret(result.secret)
      // Pull the freshly-rotated row (new expiresAt + rotationStatus reset
      // back to "current") into view immediately, so the table reflects the
      // backend's three writes without a manual page reload.
      state.reload()
    },
    [rotate, state]
  )

  const columns = React.useMemo(
    () =>
      getServiceIdentityColumns({
        canWrite,
        isRotating: rotate.status === "pending",
        onRotate: (id) => void handleRotate(id),
      }),
    [canWrite, rotate.status, handleRotate]
  )

  const tableUrlState = useTableUrlState()
  const filteredData = React.useMemo(
    () =>
      filterDataClientSide(state.data ?? [], {
        search: tableUrlState.search,
        searchFields: [
          (r) => r.name,
          (r) => r.scopes.join(" "),
          (r) => r.environment,
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
    persistKey: "/admin/service-identities",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
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
              placeholder="Search identity, scopes..."
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
      <Dialog
        open={revealedSecret !== null}
        onOpenChange={(open) => {
          if (!open) setRevealedSecret(null)
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>New credential</DialogTitle>
            <DialogDescription>
              This secret will not be shown again. Copy it now and store it in your secret
              manager.
            </DialogDescription>
          </DialogHeader>
          <code
            data-testid="revealed-secret"
            className="block break-all rounded bg-muted p-2 text-xs"
          >
            {revealedSecret}
          </code>
          <div className="flex justify-end">
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                if (revealedSecret === null) return
                void navigator.clipboard.writeText(revealedSecret)
              }}
            >
              Copy
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
