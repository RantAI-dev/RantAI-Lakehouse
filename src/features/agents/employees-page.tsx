"use client"

import * as React from "react"
import { useRouter } from "next/navigation"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { AutonomyBadge, StatusBadge } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
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

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Digital Employees"
        description="Governed agents with autonomy levels, budgets, tools, and approval rates."
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
    </div>
  )
}
