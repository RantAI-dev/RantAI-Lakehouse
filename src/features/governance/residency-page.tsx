"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { withNotify } from "@/lib/notify"
import { type Classification } from "@/lib/status"
import { governanceService } from "@/services"
import { CLASSIFICATION_OPTIONS, getResidencyColumns } from "./residency-columns"

const selectClassName =
  "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

export function ResidencyPage() {
  const state = useService((s) => governanceService.listResidency(s), [])
  const [createOpen, setCreateOpen] = React.useState(false)
  const [tenant, setTenant] = React.useState("")
  const [formClassification, setFormClassification] =
    React.useState<Classification>("internal")
  const [approvedSites, setApprovedSites] = React.useState("")
  const [crossSiteAllowed, setCrossSiteAllowed] = React.useState("no")
  const [allowedOutput, setAllowedOutput] = React.useState("")
  const tableUrlState = useTableUrlState()

  const columns = React.useMemo(() => getResidencyColumns(), [])

  const filteredData = React.useMemo(() => {
    if (state.status !== "success" || !state.data) return []
    return filterDataClientSide(state.data, {
      search: tableUrlState.search,
      searchFields: [
        (r) => r.tenant,
        (r) => r.classification,
        (r) => r.approvedSites.join(", "),
        (r) => r.allowedOutput,
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
    persistKey: "/governance/residency",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  const create = useServiceAction(
    withNotify(
      {
        success: "Residency rule created",
        error: "Failed to create residency rule",
      },
      (
        signal,
        input: Parameters<typeof governanceService.createResidencyRule>[0]
      ) => governanceService.createResidencyRule(input, signal)
    )
  )

  function resetForm() {
    setTenant("")
    setFormClassification("internal")
    setApprovedSites("")
    setCrossSiteAllowed("no")
    setAllowedOutput("")
  }

  async function handleCreate() {
    const sites = approvedSites
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean)
    const result = await create.run({
      tenant: tenant.trim(),
      classification: formClassification,
      approvedSites: sites,
      crossSiteAllowed: crossSiteAllowed === "yes",
      allowedOutput: allowedOutput.trim(),
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
        title="Residency"
        description="Approved sites, classification rules, and boundary-crossing constraints."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <PlusIcon data-icon="inline-start" />
            Create Residency Rule
          </Button>
        }
      />

      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="flex flex-col gap-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch placeholder="Search tenant, sites, classification…" />
          </DataTableAdvancedToolbar>
          <DataTable table={table} />
        </div>
      ) : null}

      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="Create Residency Rule"
        description="Set approved sites and cross-site output constraints."
        canSubmit={Boolean(
          tenant.trim() && approvedSites.trim() && allowedOutput.trim()
        )}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
      >
        <div className="space-y-1.5">
          <Label htmlFor="rr-tenant">Tenant</Label>
          <Input
            id="rr-tenant"
            value={tenant}
            onChange={(e) => setTenant(e.target.value)}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="rr-class">Classification</Label>
          <select
            id="rr-class"
            className={selectClassName}
            value={formClassification}
            onChange={(e) =>
              setFormClassification(e.target.value as Classification)
            }
          >
            {CLASSIFICATION_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="rr-sites">Approved sites (comma-separated)</Label>
          <Input
            id="rr-sites"
            value={approvedSites}
            onChange={(e) => setApprovedSites(e.target.value)}
            placeholder="id-jkt, sg-1"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="rr-cross">Cross-site allowed</Label>
          <select
            id="rr-cross"
            className={selectClassName}
            value={crossSiteAllowed}
            onChange={(e) => setCrossSiteAllowed(e.target.value)}
          >
            <option value="no">No</option>
            <option value="yes">Yes</option>
          </select>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="rr-output">Allowed output</Label>
          <Input
            id="rr-output"
            value={allowedOutput}
            onChange={(e) => setAllowedOutput(e.target.value)}
            placeholder="aggregates only"
          />
        </div>
      </CreateSheet>
    </div>
  )
}

