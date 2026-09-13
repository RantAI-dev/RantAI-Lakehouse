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
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import {
  AUTONOMY_LABEL,
  type AutonomyLevel,
} from "@/lib/status"
import { agentService } from "@/services"
import { getEmployeeColumns } from "./employee-columns"

const AUTONOMY_OPTIONS = (
  Object.keys(AUTONOMY_LABEL) as AutonomyLevel[]
).map((level) => ({ value: level, label: AUTONOMY_LABEL[level] }))

const selectClassName =
  "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

export function EmployeesPage() {
  const state = useService((s) => agentService.listEmployees(s), [])
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [purpose, setPurpose] = React.useState("")
  const [formAutonomy, setFormAutonomy] = React.useState<AutonomyLevel>("L1")
  const [allowedTools, setAllowedTools] = React.useState("")
  const [dataScope, setDataScope] = React.useState("")
  const [budgetLimit, setBudgetLimit] = React.useState("")

  const columns = React.useMemo(() => getEmployeeColumns(), [])

  const data = state.data ?? []

  const { table } = useDataTable({
    data,
    columns,
    pageCount: 1,
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (originalRow) => originalRow.id,
    shallow: false,
    clearOnDefault: true,
  })

  const create = useServiceAction(
    withNotify(
      { success: "Employee created", error: "Failed to create employee" },
      (signal, input: Parameters<typeof agentService.createEmployee>[0]) =>
        agentService.createEmployee(input, signal)
    )
  )

  function resetForm() {
    setName("")
    setPurpose("")
    setFormAutonomy("L1")
    setAllowedTools("")
    setDataScope("")
    setBudgetLimit("")
  }

  async function handleCreate() {
    const tools = allowedTools
      .split(",")
      .map((t) => t.trim())
      .filter(Boolean)
    const budget = Number(budgetLimit)
    const result = await create.run({
      name: name.trim(),
      purpose: purpose.trim(),
      autonomy: formAutonomy,
      allowedTools: tools,
      dataScope: dataScope.trim(),
      budgetLimit: budget,
    })
    if (result) {
      setCreateOpen(false)
      resetForm()
      state.reload()
    }
  }

  const canSubmit = Boolean(
    name.trim() &&
      purpose.trim() &&
      allowedTools.trim() &&
      dataScope.trim() &&
      budgetLimit.trim() &&
      !Number.isNaN(Number(budgetLimit))
  )

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Digital Employees"
        description="Governed agents with autonomy levels, budgets, tools, and approval rates."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <PlusIcon data-icon="inline-start" />
            Create Employee
          </Button>
        }
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch placeholder="Search employees..." />
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
        title="Create Employee"
        description="Define purpose, autonomy, tools, data scope, and budget."
        canSubmit={canSubmit}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
      >
        <div className="space-y-1.5">
          <Label htmlFor="emp-name">Name</Label>
          <Input id="emp-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="emp-purpose">Purpose</Label>
          <Input
            id="emp-purpose"
            value={purpose}
            onChange={(e) => setPurpose(e.target.value)}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="emp-autonomy">Autonomy</Label>
          <select
            id="emp-autonomy"
            className={selectClassName}
            value={formAutonomy}
            onChange={(e) => setFormAutonomy(e.target.value as AutonomyLevel)}
          >
            {AUTONOMY_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>{o.label}</option>
            ))}
          </select>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="emp-tools">Allowed tools (comma-separated)</Label>
          <Input
            id="emp-tools"
            value={allowedTools}
            onChange={(e) => setAllowedTools(e.target.value)}
            placeholder="sql.query, catalog.read"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="emp-scope">Data scope</Label>
          <Input
            id="emp-scope"
            value={dataScope}
            onChange={(e) => setDataScope(e.target.value)}
            placeholder="tenant:acme"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="emp-budget">Budget limit</Label>
          <Input
            id="emp-budget"
            type="number"
            min={0}
            value={budgetLimit}
            onChange={(e) => setBudgetLimit(e.target.value)}
          />
        </div>
      </CreateSheet>
    </div>
  )
}
