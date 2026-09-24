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
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { useAuth } from "@/features/auth/auth-provider"
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService, useServiceAction } from "@/hooks/use-service"
import { isOwnAccessRequest } from "@/lib/access-requests"
import { withNotify } from "@/lib/notify"
import { formatCost, formatRelativeTime } from "@/lib/format"
import { agentService, assetService } from "@/services"
import type { ApprovalItem } from "@/services/contracts/agents"
import { accessPermission, getApprovalColumns, statusCell } from "./approval-columns"

interface DrawerContentProps {
  readonly selected: ApprovalItem
  readonly executionResult: { readonly executed: boolean; readonly result?: unknown } | null
  readonly decidedGrants: Record<string, string>
  readonly selfApproval: boolean
  readonly onDecide: (decision: "approved" | "rejected") => void
}

function ApprovalDrawerContent({
  selected,
  executionResult,
  decidedGrants,
  selfApproval,
  onDecide,
}: DrawerContentProps) {
  const costText = selected.costEstimate != null ? formatCost(selected.costEstimate) : "—"
  const decisionText = selected.decidedAt
    ? `${selected.status} · ${formatRelativeTime(selected.decidedAt)}`
    : "Pending"
  // A grant's real expiry is only ever known for a row THIS session just
  // decided (see `statusCell`'s doc comment); an older `expiresAt` on a
  // tool-call approval is a real backend field either way.
  const expiresValue =
    selected.kind === "access"
      ? decidedGrants[selected.id]
      : selected.expiresAt

  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        {statusCell(selected, decidedGrants)}
        {selected.status === "pending" && selfApproval ? (
          <span title="You requested this access; a different Governance Admin must decide it">
            <Button size="sm" disabled>
              <CheckIcon data-icon="inline-start" />
              Approve
            </Button>
          </span>
        ) : null}
        {selected.status === "pending" && !selfApproval ? (
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
          ...(selected.kind === "access"
            ? []
            : [
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
              ]),
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
            // The Agent Workflows page was removed (no mock consumer left
            // for it), so this renders the workflow id as plain text
            // instead of a link that would land on a missing route.
            label: "Workflow",
            value: selected.workflowId ? (
              <span className="font-mono text-xs">{selected.workflowId}</span>
            ) : (
              "—"
            ),
          },
          {
            label: selected.kind === "access" ? "Catalog entry" : "Resource",
            value: selected.resource ?? "—",
          },
          { label: "Reason", value: selected.reason ?? "—" },
          { label: "Impact", value: selected.impact ?? "—" },
          { label: "Risk", value: selected.risk || "—" },
          { label: "Policy", value: selected.policy ?? "—" },
          { label: "Cost estimate", value: costText },
          {
            label: "Requested",
            value: formatRelativeTime(selected.requestedAt),
          },
          {
            label: "Expires",
            value: expiresValue ? formatRelativeTime(expiresValue) : "—",
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
  const label = selected.kind === "access" ? `Access: ${accessPermission(selected.action)}` : selected.action
  return `${verb} "${label}"?`
}

export function ApprovalsPage() {
  const state = useService((s) => agentService.listApprovals(undefined, s), [])
  const { user } = useAuth()
  const [selected, setSelected] = React.useState<ApprovalItem | null>(null)
  const [decision, setDecision] = React.useState<"approved" | "rejected" | null>(
    null
  )
  const [comment, setComment] = React.useState("")
  const [executionResult, setExecutionResult] = React.useState<{
    readonly executed: boolean
    readonly result?: unknown
  } | null>(null)
  // Grants this session decided, keyed by approval id — see `statusCell`'s
  // doc comment on why an already-decided access request otherwise has no
  // expiry to show.
  const [decidedGrants, setDecidedGrants] = React.useState<Record<string, string>>({})

  // A `kind = "tool_call"` decision and a `kind = "access"` decision are
  // two different routes with two different server-side authorities
  // (`agent:approve` vs `access:approve`) — never one call that guesses
  // which endpoint to hit.
  const decideToolCall = useServiceAction(
    withNotify(
      { success: "Decision recorded", error: "Failed to record decision" },
      (signal, id: string, input: { decision: "approved" | "rejected"; comment?: string }) =>
        agentService.decideApproval(id, input, signal)
    )
  )
  const decideAccess = useServiceAction(
    withNotify(
      { success: "Decision recorded", error: "Failed to record decision" },
      (signal, id: string, dec: "approved" | "rejected", cmt: string | undefined) => {
        // `decideAccessRequest` is optional on `AssetService` only so the
        // dead `mock/assets.ts` fixture still satisfies the interface —
        // see that contract's own doc comment. Fail closed rather than
        // silently no-op if that assumption is ever wrong.
        if (!assetService.decideAccessRequest) {
          return Promise.reject(new Error("This deployment cannot decide access requests."))
        }
        return assetService.decideAccessRequest(id, dec, cmt, signal)
      }
    )
  )
  const deciding = decideToolCall.status === "pending" || decideAccess.status === "pending"
  const decideError = decideToolCall.error ?? decideAccess.error

  const selfApprovalOf = React.useCallback(
    (item: ApprovalItem) => user !== null && isOwnAccessRequest(item, user.id),
    [user]
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
        decidedGrants,
        isSelfApproval: selfApprovalOf,
      }),
    [decidedGrants, selfApprovalOf]
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
    const trimmedComment = comment.trim() || undefined
    if (selected.kind === "access") {
      const outcome = await decideAccess.run(selected.id, decision, trimmedComment)
      if (outcome) {
        setDecision(null)
        setComment("")
        if (outcome.grant) {
          setDecidedGrants((prev) => ({ ...prev, [selected.id]: outcome.grant!.expiresAt }))
        }
        // Access requests never execute a tool — nothing to show there.
        setExecutionResult(null)
        setSelected({
          ...selected,
          status: outcome.status,
          decidedAt: new Date().toISOString(),
          comment: trimmedComment,
        })
        state.reload()
      }
      return
    }
    const outcome = await decideToolCall.run(selected.id, {
      decision,
      comment: trimmedComment,
    })
    if (outcome) {
      setDecision(null)
      setComment("")
      setSelected(outcome.approval)
      setExecutionResult({ executed: outcome.executed, result: outcome.result })
      state.reload()
    }
  }

  const selfApproval = selected !== null && selfApprovalOf(selected)
  const dialogDescription = getDecisionDescription(selected, decision)

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Approvals"
        description="Human review gate for higher-risk agent actions and catalog access requests. Approve or reject with impact context before anything takes effect."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" && data.length === 0 ? (
        <EmptyState
          title="No approvals"
          description="A request lands here whenever a run — from the copilot chat, Run now, or a schedule — hits a high-risk (WriteHigh) tool call, or a catalog viewer requests a permission they don't hold. Nothing is waiting on you right now."
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
        title={selected?.kind === "access" ? "Access request" : "Approval request"}
        description={
          selected?.kind === "access"
            ? `Access: ${accessPermission(selected.action)}`
            : selected?.action
        }
      >
        {selected ? (
          <ApprovalDrawerContent
            selected={selected}
            executionResult={executionResult}
            decidedGrants={decidedGrants}
            selfApproval={selfApproval}
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
          (selected?.kind === "access"
            ? `The requester ${
                decision === "approved" ? "gains" : "does not gain"
              } this permission. The choice is audited.`
            : "The agent will proceed or stop based on your decision. The choice is audited.")
        }
        confirmLabel={decision === "approved" ? "Approve" : "Reject"}
        confirming={deciding}
        onConfirm={confirmDecision}
      >
        <Textarea
          value={comment}
          onChange={(e) => setComment(e.target.value)}
          placeholder="Optional comment for the audit trail"
          rows={3}
        />
        {decideError ? (
          // Surfaces the backend's own message verbatim — e.g. the 403 a
          // self-approval attempt gets refused with — rather than a
          // generic "something went wrong". The UI does not normally
          // offer this dialog for a self-approval (see the disabled,
          // tooltipped Approve button above); this is the fallback if it
          // ever happens anyway (e.g. a stale row from before a reload).
          <p className="text-sm text-destructive">{decideError.message}</p>
        ) : null}
      </ConfirmActionDialog>
    </div>
  )
}

