"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { PageHeader } from "@/components/patterns/page-header"
import {
  ApprovalBadge,
  HealthBadge,
  Pill,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatCompactNumber } from "@/lib/format"
import { withNotify } from "@/lib/notify"
import { agentService } from "@/services"
import type { AgentTool } from "@/services/contracts/agents"
import { getToolColumns } from "./tool-columns"

function ToolDrawerContent({ tool }: { readonly tool: AgentTool }) {
  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <HealthBadge health={tool.health} />
        <ApprovalBadge status={tool.approvalStatus} />
        {tool.deprecated ? <Pill tone="neutral">Deprecated</Pill> : null}
      </div>
      <MetadataList
        items={[
          { label: "Version", value: `v${tool.version}` },
          { label: "Publisher", value: tool.publisher },
          { label: "Permission", value: tool.permission },
          { label: "Rate limit", value: tool.rateLimit },
          {
            label: "Usage 30d",
            value: formatCompactNumber(tool.usage30d),
          },
        ]}
      />
    </>
  )
}

export function ToolsPage() {
  const state = useService((s) => agentService.listTools(s), [])
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

  const columns = React.useMemo(
    () =>
      getToolColumns({
        onInspect: (tool) => setSelected(tool),
      }),
    []
  )

  const rawData = React.useMemo(() => state.data ?? [], [state.data])

  const { table } = useDataTable({
    data: rawData,
    columns,
    pageCount: 1,
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
    shallow: false,
    clearOnDefault: true,
  })

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

  const canSubmit = Boolean(
    name.trim() &&
      version.trim() &&
      publisher.trim() &&
      permission.trim() &&
      rateLimit.trim()
  )

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
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch placeholder="Search tools, permissions..." />
          </DataTableAdvancedToolbar>
          <div className="rounded-md border">
            <DataTable table={table} />
          </div>
        </div>
      ) : null}
      <DetailDrawer
        open={selected != null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
        description="Tool detail"
      >
        {selected ? <ToolDrawerContent tool={selected} /> : null}
      </DetailDrawer>
      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="Register Tool"
        description="Add a governed tool with permission and rate limits."
        canSubmit={canSubmit}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
        submitLabel="Register"
      >
        <div className="space-y-1.5">
          <Label htmlFor="tool-name">Name</Label>
          <Input
            id="tool-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
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
