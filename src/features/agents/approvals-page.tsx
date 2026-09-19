"use client"

import * as React from "react"
import Link from "next/link"
import { CheckIcon, XIcon } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
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
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { formatCost, formatRelativeTime } from "@/lib/format"
import { agentService } from "@/services"
import type { ApprovalItem } from "@/services/contracts/agents"
import { getApprovalColumns } from "./approval-columns"

interface DrawerContentProps {
  readonly selected: ApprovalItem
  readonly executionResult: { readonly executed: boolean; readonly result?: unknown } | null
  readonly onDecide: (decision: "approved" | "rejected") => void
}

function ApprovalDrawerContent({
  selected,
  executionResult,
  onDecide,
}: DrawerContentProps) {
  const costText = selected.costEstimate != null ? formatCost(selected.costEstimate) : "—"
  const decisionText = selected.decidedAt
    ? `${selected.status} · ${formatRelativeTime(selected.decidedAt)}`
    : "Pending"

  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <ApprovalBadge status={selected.status} />
        {selected.status === "pending" ? (
          <>
            <Button size="sm" onClick={() => onDecide("approved")}>
              <CheckIcon data-icon="inline-start" />
              Approve
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => onDecide("rejected")}
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
              <Link
                href={`/agents/runs/${encodeURIComponent(selected.runId)}`}
                className="font-mono text-xs text-primary hover:underline"
              >
                {selected.runId}
              </Link>
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
          { label: "Cost estimate", value: costText },
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
            value: decisionText,
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
      {executionResult ? (
        <div>
          <p className="text-xs font-medium text-muted-foreground">
            {executionResult.executed
              ? "Execution result"
              : "Decision recorded — not executed"}
          </p>
          <pre className="mt-1 max-h-64 overflow-auto rounded bg-muted/60 px-2 py-1.5 font-mono text-[11px] text-muted-foreground">
            {executionResult.executed
              ? JSON.stringify(executionResult.result, null, 2)
              : "The tool was never executed — either the action was rejected, or the approver lacked the underlying tool's own permission."}
          </pre>
        </div>
      ) : null}
    </>
  )
}

function getDecisionDescription(
  selected: ApprovalItem | null,
  decision: "approved" | "rejected" | null
): string {
  if (!selected) {
    return "Confirm this approval decision."
  }
  const verb = decision === "approved" ? "Approve" : "Reject"
  return `${verb} “${selected.action}”?`
}

export function ApprovalsPage() {
  const state = useService((s) => agentService.listApprovals(undefined, s), [])
  const [selected, setSelected] = React.useState<ApprovalItem | null>(null)
  const [decision, setDecision] = React.useState<"approved" | "rejected" | null>(
    null
  )
  const [comment, setComment] = React.useState("")
  const [executionResult, setExecutionResult] = React.useState<{
    readonly executed: boolean
    readonly result?: unknown
  } | null>(null)

  const decide = useServiceAction(
    withNotify(
      { success: "Decision recorded", error: "Failed to record decision" },
      (signal, id: string, input: { decision: "approved" | "rejected"; comment?: string }) =>
        agentService.decideApproval(id, input, signal)
    )
  )

  const columns = React.useMemo(
    () =>
      getApprovalColumns({
        onSelect: (item) => {
          setExecutionResult(null)
          setSelected(item)
        },
        onDecide: (item, d) => {
          setSelected(item)
          setDecision(d)
        },
      }),
    []
  )

  const data = state.data ?? []

  const tableUrlState = useTableUrlState()
  const filteredData = React.useMemo(
    () =>
      filterDataClientSide(state.data ?? [], {
        search: tableUrlState.search,
        searchFields: [
          (r) => r.action,
          (r) => r.employeeName,
          (r) => r.resource,
          (r) => r.risk,
        ],
        filters: tableUrlState.filters,
        joinOperator: tableUrlState.joinOperator,
      }),
    [state.data, tableUrlState.search, tableUrlState.filters, tableUrlState.joinOperator]
  )

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/agents/approvals",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

  async function confirmDecision() {
    if (!selected || !decision) return
    const outcome = await decide.run(selected.id, {
      decision,
      comment: comment.trim() || undefined,
    })
    if (outcome) {
      setDecision(null)
      setComment("")
      setSelected(outcome.approval)
      setExecutionResult({ executed: outcome.executed, result: outcome.result })
      state.reload()
    }
  }

  const dialogDescription = getDecisionDescription(selected, decision)

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Approvals"
        description="Human review gate for higher-risk agent actions. Approve or reject with impact context before execution."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" && data.length === 0 ? (
        <EmptyState
          title="No approvals"
          description="A request lands here whenever a run — from the copilot chat, Run now, or a schedule — hits a high-risk (WriteHigh) tool call. Nothing is waiting on you right now."
          action={
            <Button size="sm" variant="outline" render={<Link href="/agents/runs" />}>
              View agent runs
            </Button>
          }
        />
      ) : null}
      {state.status === "success" && data.length > 0 ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch placeholder="Search approvals..." />
          </DataTableAdvancedToolbar>
          <div className="rounded-md border">
            <DataTable table={table} />
          </div>
        </div>
      ) : null}

      <DetailDrawer
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) {
            setSelected(null)
            setExecutionResult(null)
          }
        }}
        title="Approval request"
        description={selected?.action}
      >
        {selected ? (
          <ApprovalDrawerContent
            selected={selected}
            executionResult={executionResult}
            onDecide={setDecision}
          />
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
        description={dialogDescription}
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

