"use client"

import * as React from "react"
import Link from "next/link"
import { CheckIcon, XIcon } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { ApprovalBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { formatCost, formatRelativeTime } from "@/lib/format"
import { APPROVAL_STATUS_LABEL, type ApprovalStatus } from "@/lib/status"
import { agentService } from "@/services"
import type { ApprovalItem } from "@/services/contracts/agents"

const columns: ColumnDef<ApprovalItem>[] = [
  {
    key: "action",
    header: "Requested action",
    render: (r) => (
      <div>
        <p className="font-medium">{r.action}</p>
        <p className="text-xs text-muted-foreground">{r.employeeName}</p>
      </div>
    ),
  },
  {
    key: "status",
    header: "Status",
    render: (r) => <ApprovalBadge status={r.status} />,
  },
  { key: "risk", header: "Risk", render: (r) => r.risk },
  {
    key: "requested",
    header: "Requested",
    render: (r) => (
      <span className="text-muted-foreground">
        {formatRelativeTime(r.requestedAt)}
      </span>
    ),
  },
]

export function ApprovalsPage() {
  const state = useService((s) => agentService.listApprovals(undefined, s), [])
  const [search, setSearch] = React.useState("")
  const [status, setStatus] = React.useState<ApprovalStatus | "all">("pending")
  const [selected, setSelected] = React.useState<ApprovalItem | null>(null)
  const [decision, setDecision] = React.useState<"approved" | "rejected" | null>(
    null
  )
  const [comment, setComment] = React.useState("")
  const decide = useServiceAction(
    withNotify(
      { success: "Decision recorded", error: "Failed to record decision" },
      (signal, id: string, input: { decision: "approved" | "rejected"; comment?: string }) =>
        agentService.decideApproval(id, input, signal)
    )
  )

  const rows = React.useMemo(() => {
    if (state.status !== "success") return []
    const q = search.trim().toLowerCase()
    return state.data.filter((a) => {
      if (status !== "all" && a.status !== status) return false
      if (!q) return true
      return [a.action, a.employeeName, a.risk, a.resource ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(q)
    })
  }, [state.status, state.data, search, status])

  async function confirmDecision() {
    if (!selected || !decision) return
    const updated = await decide.run(selected.id, {
      decision,
      comment: comment.trim() || undefined,
    })
    if (updated) {
      setDecision(null)
      setComment("")
      setSelected(updated)
      state.reload()
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Approvals"
        description="Human review gate for higher-risk agent actions. Approve or reject with impact context before execution."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search approvals..."
        />
        <FilterSelect
          ariaLabel="Filter by status"
          allLabel="All statuses"
          value={status}
          onChange={(v) => setStatus(v as ApprovalStatus | "all")}
          options={Object.entries(APPROVAL_STATUS_LABEL).map(([value, label]) => ({
            value,
            label,
          }))}
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" && rows.length === 0 ? (
        <EmptyState
          title="No approvals"
          description="Pending and resolved approval requests appear here."
        />
      ) : null}
      {state.status === "success" && rows.length > 0 ? (
        <DataTable
          columns={columns}
          rows={rows}
          rowKey={(r) => r.id}
          onRowClick={setSelected}
        />
      ) : null}

      <DetailDrawer
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title="Approval request"
        description={selected?.action}
      >
        {selected ? (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <ApprovalBadge status={selected.status} />
              {selected.status === "pending" ? (
                <>
                  <Button size="sm" onClick={() => setDecision("approved")}>
                    <CheckIcon data-icon="inline-start" />
                    Approve
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => setDecision("rejected")}
                  >
                    <XIcon data-icon="inline-start" />
                    Reject
                  </Button>
                </>
              ) : null}
              {selected.auditEventId ? (
                <Button
                  size="sm"
                  variant="ghost"
                  render={<Link href={`/audit?event=${selected.auditEventId}`} />}
                >
                  Audit
                </Button>
              ) : null}
            </div>
            <MetadataList
              items={[
                {
                  label: "Agent",
                  value: (
                    <Link
                      href={`/agents/employees/${selected.employeeId}`}
                      className="text-primary hover:underline"
                    >
                      {selected.employeeName}
                    </Link>
                  ),
                },
                {
                  label: "Run",
                  value: selected.runId ? (
                    <span className="font-mono text-xs">{selected.runId}</span>
                  ) : (
                    "—"
                  ),
                },
                {
                  label: "Workflow",
                  value: selected.workflowId ? (
                    <Link
                      href={`/agents/workflows?id=${selected.workflowId}`}
                      className="font-mono text-xs text-primary hover:underline"
                    >
                      {selected.workflowId}
                    </Link>
                  ) : (
                    "—"
                  ),
                },
                { label: "Resource", value: selected.resource ?? "—" },
                { label: "Reason", value: selected.reason ?? "—" },
                { label: "Impact", value: selected.impact ?? "—" },
                { label: "Risk", value: selected.risk },
                { label: "Policy", value: selected.policy ?? "—" },
                {
                  label: "Cost estimate",
                  value:
                    selected.costEstimate != null
                      ? formatCost(selected.costEstimate)
                      : "—",
                },
                {
                  label: "Requested",
                  value: formatRelativeTime(selected.requestedAt),
                },
                {
                  label: "Expires",
                  value: selected.expiresAt
                    ? formatRelativeTime(selected.expiresAt)
                    : "—",
                },
                {
                  label: "Decision",
                  value: selected.decidedAt
                    ? `${selected.status} · ${formatRelativeTime(selected.decidedAt)}`
                    : "Pending",
                },
                {
                  label: "Comment",
                  value: selected.comment ?? "—",
                },
              ]}
            />
            {selected.evidence && selected.evidence.length > 0 ? (
              <div>
                <p className="text-xs font-medium text-muted-foreground">
                  Supporting evidence
                </p>
                <ul className="mt-1 list-disc space-y-1 pl-4 text-sm">
                  {selected.evidence.map((e) => (
                    <li key={e}>{e}</li>
                  ))}
                </ul>
              </div>
            ) : null}
          </>
        ) : null}
      </DetailDrawer>

      <ConfirmActionDialog
        open={decision !== null}
        onOpenChange={(open) => {
          if (!open) {
            setDecision(null)
            setComment("")
          }
        }}
        title={decision === "approved" ? "Approve action" : "Reject action"}
        description={
          selected
            ? `${decision === "approved" ? "Approve" : "Reject"} “${selected.action}”?`
            : "Confirm this approval decision."
        }
        impact={
          selected?.impact ??
          "The agent will proceed or stop based on your decision. The choice is audited."
        }
        confirmLabel={decision === "approved" ? "Approve" : "Reject"}
        confirming={decide.status === "pending"}
        onConfirm={confirmDecision}
      >
        <Textarea
          value={comment}
          onChange={(e) => setComment(e.target.value)}
          placeholder="Optional comment for the audit trail"
          rows={3}
        />
      </ConfirmActionDialog>
    </div>
  )
}
