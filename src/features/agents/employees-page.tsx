"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { useRouter } from "next/navigation"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { AutonomyBadge, StatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatCost, formatPercent } from "@/lib/format"
import {
  AUTONOMY_LABEL,
  ENTITY_STATUS_LABEL,
  type AutonomyLevel,
} from "@/lib/status"
import { agentService } from "@/services"
import type { DigitalEmployee } from "@/services/contracts/agents"

const AUTONOMY_OPTIONS = (
  Object.keys(AUTONOMY_LABEL) as AutonomyLevel[]
).map((level) => ({ value: level, label: AUTONOMY_LABEL[level] }))

const selectClassName =
  "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

const columns: ColumnDef<DigitalEmployee>[] = [
  { key: "name", header: "Employee", render: (r) => (
    <div><p className="font-medium">{r.name}</p><p className="text-xs text-muted-foreground">{r.purpose}</p></div>
  )},
  { key: "autonomy", header: "Autonomy", render: (r) => <AutonomyBadge level={r.autonomy} /> },
  { key: "status", header: "Status", render: (r) => <StatusBadge status={r.status} /> },
  { key: "budget", header: "Budget", render: (r) => {
    const used = r.budgetSpent + r.budgetReserved
    return (
      <span className="tabular-nums">
        {formatCost(used)} / {formatCost(r.budgetLimit)}{" "}
        <span className="text-xs text-muted-foreground">
          ({formatPercent(r.budgetLimit > 0 ? used / r.budgetLimit : 0)})
        </span>
      </span>
    )
  }},
  { key: "success", header: "Success", render: (r) => formatPercent(r.successRate) },
  { key: "approval", header: "Approval rate", render: (r) => formatPercent(r.approvalRate) },
  { key: "owner", header: "Owner", render: (r) => r.owner },
]

export function EmployeesPage() {
  const router = useRouter()
  const state = useService((s) => agentService.listEmployees(s), [])
  const [search, setSearch] = React.useState("")
  const [status, setStatus] = React.useState("all")
  const [autonomy, setAutonomy] = React.useState("all")
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [purpose, setPurpose] = React.useState("")
  const [formAutonomy, setFormAutonomy] = React.useState<AutonomyLevel>("L1")
  const [allowedTools, setAllowedTools] = React.useState("")
  const [dataScope, setDataScope] = React.useState("")
  const [budgetLimit, setBudgetLimit] = React.useState("")
  const create = useServiceAction(
    (signal, input: Parameters<typeof agentService.createEmployee>[0]) =>
      agentService.createEmployee(input, signal)
  )

  const statusOptions = React.useMemo(() => {
    const present = new Set(state.data?.map((r) => r.status) ?? [])
    return [...present].map((s) => ({ value: s, label: ENTITY_STATUS_LABEL[s] }))
  }, [state.data])

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((r) => {
      if (status !== "all" && r.status !== status) return false
      if (autonomy !== "all" && r.autonomy !== autonomy) return false
      if (!q) return true
      return [r.name, r.purpose, r.owner].some((v) => v.toLowerCase().includes(q))
    })
  }, [state.data, search, status, autonomy])

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
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search name, purpose, owner..."
        />
        <FilterSelect
          value={status}
          onChange={setStatus}
          options={statusOptions}
          allLabel="All statuses"
          ariaLabel="Filter by status"
        />
        <FilterSelect
          value={autonomy}
          onChange={setAutonomy}
          options={AUTONOMY_OPTIONS}
          allLabel="All autonomy levels"
          ariaLabel="Filter by autonomy level"
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" ? (
        <DataTable
          columns={columns}
          rows={filtered}
          rowKey={(r) => r.id}
          onRowClick={(r) => router.push(`/agents/employees/${r.id}`)}
        />
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
