"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { CreateSheet } from "@/components/patterns/create-sheet"
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
import {
  ApprovalBadge,
  HealthBadge,
  Pill,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { formatCompactNumber } from "@/lib/format"
import {
  APPROVAL_STATUS_LABEL,
  HEALTH_LABEL,
  type ApprovalStatus,
  type Health,
} from "@/lib/status"
import { agentService } from "@/services"
import type { AgentTool } from "@/services/contracts/agents"

const APPROVAL_OPTIONS = (
  Object.keys(APPROVAL_STATUS_LABEL) as ApprovalStatus[]
).map((s) => ({ value: s, label: APPROVAL_STATUS_LABEL[s] }))

const HEALTH_OPTIONS = (Object.keys(HEALTH_LABEL) as Health[]).map((h) => ({
  value: h,
  label: HEALTH_LABEL[h],
}))

const columns: ColumnDef<AgentTool>[] = [
  { key: "name", header: "Tool", render: (r) => (
    <div>
      <div className="flex items-center gap-2">
        <p className="font-mono font-medium">{r.name}</p>
        {r.deprecated ? <Pill tone="neutral">Deprecated</Pill> : null}
      </div>
      <p className="text-xs text-muted-foreground">v{r.version} · {r.publisher}</p>
    </div>
  )},
  { key: "perm", header: "Permission", render: (r) => r.permission },
  { key: "health", header: "Health", render: (r) => <HealthBadge health={r.health} /> },
  { key: "approval", header: "Approval", render: (r) => <ApprovalBadge status={r.approvalStatus} /> },
  { key: "rate", header: "Rate limit", render: (r) => r.rateLimit },
  { key: "usage", header: "Usage 30d", render: (r) => formatCompactNumber(r.usage30d) },
]

export function ToolsPage() {
  const state = useService((s) => agentService.listTools(s), [])
  const [search, setSearch] = React.useState("")
  const [approval, setApproval] = React.useState("all")
  const [health, setHealth] = React.useState("all")
  const [selected, setSelected] = React.useState<AgentTool | null>(null)
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [version, setVersion] = React.useState("")
  const [publisher, setPublisher] = React.useState("")
  const [permission, setPermission] = React.useState("")
  const [rateLimit, setRateLimit] = React.useState("")
  const create = useServiceAction(
    withNotify(
      { success: "Tool registered", error: "Failed to register tool" },
      (signal, input: Parameters<typeof agentService.registerTool>[0]) =>
        agentService.registerTool(input, signal)
    )
  )

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((r) => {
      if (approval !== "all" && r.approvalStatus !== approval) return false
      if (health !== "all" && r.health !== health) return false
      if (!q) return true
      return [r.name, r.publisher, r.permission].some((v) =>
        v.toLowerCase().includes(q)
      )
    })
  }, [state.data, search, approval, health])

  function resetForm() {
    setName("")
    setVersion("")
    setPublisher("")
    setPermission("")
    setRateLimit("")
  }

  async function handleCreate() {
    const result = await create.run({
      name: name.trim(),
      version: version.trim(),
      publisher: publisher.trim(),
      permission: permission.trim(),
      rateLimit: rateLimit.trim(),
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
        title="Tool Registry"
        description="Governed tool inventory with permissions, health, and usage."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <PlusIcon data-icon="inline-start" />
            Register Tool
          </Button>
        }
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search name, publisher, permission..."
        />
        <FilterSelect
          value={approval}
          onChange={setApproval}
          options={APPROVAL_OPTIONS}
          allLabel="All approvals"
          ariaLabel="Filter by approval status"
        />
        <FilterSelect
          value={health}
          onChange={setHealth}
          options={HEALTH_OPTIONS}
          allLabel="All health"
          ariaLabel="Filter by health"
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" ? (
        <DataTable
          columns={columns}
          rows={filtered}
          rowKey={(r) => r.id}
          onRowClick={setSelected}
        />
      ) : null}
      <DetailDrawer
        open={selected != null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
        description="Tool detail"
      >
        {selected ? (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <HealthBadge health={selected.health} />
              <ApprovalBadge status={selected.approvalStatus} />
              {selected.deprecated ? <Pill tone="neutral">Deprecated</Pill> : null}
            </div>
            <MetadataList
              items={[
                { label: "Version", value: `v${selected.version}` },
                { label: "Publisher", value: selected.publisher },
                { label: "Permission", value: selected.permission },
                { label: "Rate limit", value: selected.rateLimit },
                {
                  label: "Usage 30d",
                  value: formatCompactNumber(selected.usage30d),
                },
              ]}
            />
          </>
        ) : null}
      </DetailDrawer>
      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="Register Tool"
        description="Add a governed tool with permission and rate limits."
        canSubmit={Boolean(
          name.trim() &&
            version.trim() &&
            publisher.trim() &&
            permission.trim() &&
            rateLimit.trim()
        )}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
        submitLabel="Register"
      >
        <div className="space-y-1.5">
          <Label htmlFor="tool-name">Name</Label>
          <Input id="tool-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="tool-version">Version</Label>
          <Input
            id="tool-version"
            value={version}
            onChange={(e) => setVersion(e.target.value)}
            placeholder="1.0.0"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="tool-publisher">Publisher</Label>
          <Input
            id="tool-publisher"
            value={publisher}
            onChange={(e) => setPublisher(e.target.value)}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="tool-permission">Permission</Label>
          <Input
            id="tool-permission"
            value={permission}
            onChange={(e) => setPermission(e.target.value)}
            placeholder="tools.invoke"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="tool-rate">Rate limit</Label>
          <Input
            id="tool-rate"
            value={rateLimit}
            onChange={(e) => setRateLimit(e.target.value)}
            placeholder="60/min"
          />
        </div>
      </CreateSheet>
    </div>
  )
}
