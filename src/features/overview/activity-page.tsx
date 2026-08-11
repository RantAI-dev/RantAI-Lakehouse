"use client"

import { useMemo, useState } from "react"
import Link from "next/link"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { ACTOR_KIND_LABEL, type ActorKind } from "@/lib/status"
import { overviewService } from "@/services"
import type { ActivityCategory, ActivityItem } from "@/services/contracts/overview"

const CATEGORY_LABEL: Record<ActivityCategory, string> = {
  pipeline: "Pipeline",
  query: "Query",
  schema: "Schema",
  policy: "Policy",
  connector: "Connector",
  agent: "Agent",
  approval: "Approval",
  incident: "Incident",
}

const columns: ColumnDef<ActivityItem>[] = [
  {
    key: "at",
    header: "When",
    render: (r) => (
      <span className="text-muted-foreground">{formatRelativeTime(r.at)}</span>
    ),
  },
  {
    key: "actor",
    header: "Actor",
    render: (r) => (
      <div>
        <p className="font-medium">{r.actor}</p>
        <p className="text-xs text-muted-foreground">
          {ACTOR_KIND_LABEL[r.actorKind]}
        </p>
      </div>
    ),
  },
  { key: "action", header: "Action", render: (r) => r.action },
  {
    key: "target",
    header: "Target",
    render: (r) =>
      r.targetHref ? (
        <Link href={r.targetHref} className="text-primary hover:underline">
          {r.target}
        </Link>
      ) : (
        r.target
      ),
  },
  {
    key: "category",
    header: "Category",
    render: (r) => (
      <span className="text-muted-foreground">{CATEGORY_LABEL[r.category]}</span>
    ),
  },
  {
    key: "audit",
    header: "",
    render: (r) =>
      r.auditEventId ? (
        <Link
          href={`/audit?event=${r.auditEventId}`}
          className="text-sm text-primary hover:underline"
        >
          View audit
        </Link>
      ) : null,
  },
]

/** Unified activity feed across pipelines, queries, agents, and policies. */
export function ActivityPage() {
  const state = useService((s) => overviewService.listActivity(s), [])
  const [search, setSearch] = useState("")
  const [category, setCategory] = useState<ActivityCategory | "all">("all")
  const [actorKind, setActorKind] = useState<ActorKind | "all">("all")

  const rows = useMemo(() => {
    if (state.status !== "success") return []
    const q = search.trim().toLowerCase()
    return state.data.filter((item) => {
      if (category !== "all" && item.category !== category) return false
      if (actorKind !== "all" && item.actorKind !== actorKind) return false
      if (q) {
        const hay = `${item.actor} ${item.action} ${item.target}`.toLowerCase()
        if (!hay.includes(q)) return false
      }
      return true
    })
  }, [state.status, state.data, search, category, actorKind])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Activity"
        description="Recent actions from pipelines, queries, schema changes, policies, connectors, agents, and approvals."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search actor, action, target..."
        />
        <FilterSelect
          ariaLabel="Filter by category"
          allLabel="All categories"
          value={category}
          onChange={(v) => setCategory(v as ActivityCategory | "all")}
          options={Object.entries(CATEGORY_LABEL).map(([value, label]) => ({
            value,
            label,
          }))}
        />
        <FilterSelect
          ariaLabel="Filter by actor kind"
          allLabel="All actors"
          value={actorKind}
          onChange={(v) => setActorKind(v as ActorKind | "all")}
          options={Object.entries(ACTOR_KIND_LABEL).map(([value, label]) => ({
            value,
            label,
          }))}
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <DataTable columns={columns} rows={rows} rowKey={(r) => r.id} />
      ) : null}
    </div>
  )
}
