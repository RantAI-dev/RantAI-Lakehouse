"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { PlayCircleIcon } from "lucide-react"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { PageHeader } from "@/components/patterns/page-header"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { AgentRunStatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import { formatCost, formatRelativeTime } from "@/lib/format"
import { AGENT_RUN_STATUS_LABEL, type AgentRunStatus } from "@/lib/status"
import { agentService } from "@/services"
import type { AgentRun } from "@/services/contracts/agents"

const columns: ColumnDef<AgentRun>[] = [
  {
    key: "id",
    header: "Run",
    render: (r) => (
      <div>
        <p className="font-mono text-xs font-medium">{r.id}</p>
        <p className="text-xs text-muted-foreground">{r.employeeId}</p>
      </div>
    ),
  },
  { key: "status", header: "Status", render: (r) => <AgentRunStatusBadge status={r.status} /> },
  { key: "trigger", header: "Trigger", render: (r) => r.trigger },
  { key: "actor", header: "Actor", render: (r) => r.actor },
  {
    key: "started",
    header: "Started",
    render: (r) => (
      <span className="text-muted-foreground">{formatRelativeTime(r.startedAt)}</span>
    ),
  },
  {
    key: "ended",
    header: "Ended",
    render: (r) => (
      <span className="text-muted-foreground">
        {r.endedAt ? formatRelativeTime(r.endedAt) : "—"}
      </span>
    ),
  },
  {
    key: "cost",
    header: "Cost",
    render: (r) => <span className="tabular-nums">{formatCost(r.budgetConsumed)}</span>,
  },
]

/**
 * `/agents/runs` — every digital-employee run, real ones only: seeded
 * fixtures were dropped (migration `0025_drop_seeded_agent_runs.sql`), so
 * this list is legitimately empty until a Run now click or a schedule
 * produces the first row. The empty state says so explicitly rather than
 * reading as broken.
 */
export function RunsPage() {
  const router = useRouter()
  const state = useService((s) => agentService.listRuns(undefined, s), [])
  const [search, setSearch] = React.useState("")
  const [status, setStatus] = React.useState<AgentRunStatus | "all">("all")

  const rows = React.useMemo(() => {
    if (state.status !== "success") return []
    const q = search.trim().toLowerCase()
    return state.data.filter((r) => {
      if (status !== "all" && r.status !== status) return false
      if (!q) return true
      return [r.id, r.employeeId, r.trigger, r.actor].join(" ").toLowerCase().includes(q)
    })
  }, [state.status, state.data, search, status])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Agent Runs"
        description="Every headless execution of a digital employee — triggered manually with Run now, or by a schedule."
        actions={
          <Button size="sm" variant="outline" render={<Link href="/agents/employees" />}>
            <PlayCircleIcon data-icon="inline-start" />
            Run an employee
          </Button>
        }
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search run id, employee, trigger, actor..."
        />
        <FilterSelect
          ariaLabel="Filter by status"
          allLabel="All statuses"
          value={status}
          onChange={(v) => setStatus(v as AgentRunStatus | "all")}
          options={Object.entries(AGENT_RUN_STATUS_LABEL).map(([value, label]) => ({
            value,
            label,
          }))}
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" && rows.length === 0 ? (
        <EmptyState
          title="No runs yet"
          description="Runs appear here once a digital employee actually executes — click Run now on an employee's page, or set a schedule so it runs on its own. There's nothing broken; the seeded demo history was intentionally removed."
          action={
            <Button size="sm" render={<Link href="/agents/employees" />}>
              Go to Digital Employees
            </Button>
          }
        />
      ) : null}
      {state.status === "success" && rows.length > 0 ? (
        <DataTable
          columns={columns}
          rows={rows}
          rowKey={(r) => r.id}
          onRowClick={(r) => router.push(`/agents/runs/${encodeURIComponent(r.id)}`)}
        />
      ) : null}
    </div>
  )
}
