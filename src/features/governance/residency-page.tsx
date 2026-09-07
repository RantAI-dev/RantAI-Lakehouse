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
import {
  ClassificationBadge,
  Pill,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { CLASSIFICATION_LABEL, type Classification } from "@/lib/status"
import { governanceService } from "@/services"
import type { ResidencyRule } from "@/services/contracts/governance"

const CLASSIFICATION_OPTIONS = (
  Object.keys(CLASSIFICATION_LABEL) as Classification[]
).map((c) => ({ value: c, label: CLASSIFICATION_LABEL[c] }))

const selectClassName =
  "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

const columns: ColumnDef<ResidencyRule>[] = [
  { key: "tenant", header: "Tenant", render: (r) => r.tenant },
  { key: "class", header: "Classification", render: (r) => <ClassificationBadge classification={r.classification} /> },
  { key: "sites", header: "Approved sites", render: (r) => (
    <div className="flex flex-wrap gap-1">
      {r.approvedSites.map((s) => (
        <Pill key={s} tone="neutral">{s}</Pill>
      ))}
    </div>
  )},
  { key: "cross", header: "Cross-site", render: (r) =>
    r.crossSiteAllowed ? (
      <Pill tone="neutral">Cross-site allowed</Pill>
    ) : (
      <Pill tone="warning">Single site</Pill>
    ),
  },
  { key: "out", header: "Allowed output", render: (r) => r.allowedOutput },
  { key: "viol", header: "Violations 7d", render: (r) =>
    r.violations7d > 0 ? (
      <span className="font-medium text-destructive">{r.violations7d}</span>
    ) : (
      r.violations7d
    ),
  },
]

export function ResidencyPage() {
  const state = useService((s) => governanceService.listResidency(s), [])
  const [search, setSearch] = React.useState("")
  const [classification, setClassification] = React.useState("all")
  const [createOpen, setCreateOpen] = React.useState(false)
  const [tenant, setTenant] = React.useState("")
  const [formClassification, setFormClassification] =
    React.useState<Classification>("internal")
  const [approvedSites, setApprovedSites] = React.useState("")
  const [crossSiteAllowed, setCrossSiteAllowed] = React.useState("no")
  const [allowedOutput, setAllowedOutput] = React.useState("")
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

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((r) => {
      if (classification !== "all" && r.classification !== classification)
        return false
      if (!q) return true
      return r.tenant.toLowerCase().includes(q)
    })
  }, [state.data, search, classification])

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
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search tenant..."
        />
        <FilterSelect
          value={classification}
          onChange={setClassification}
          options={CLASSIFICATION_OPTIONS}
          allLabel="All classifications"
          ariaLabel="Filter by classification"
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" ? (
        <DataTable columns={columns} rows={filtered} rowKey={(r) => r.id} />
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
              <option key={o.value} value={o.value}>{o.label}</option>
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
