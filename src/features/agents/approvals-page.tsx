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
import { ApprovalBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { accessGrantState, isOwnAccessRequest } from "@/lib/access-requests"
import { formatCost, formatRelativeTime } from "@/lib/format"
import { APPROVAL_STATUS_LABEL, type ApprovalStatus } from "@/lib/status"
import { agentService, assetService } from "@/services"
import type { ApprovalItem } from "@/services/contracts/agents"

/** `"access:catalog:write"` → `"catalog:write"` — the permission a `kind = "access"` row's `action` names. */
function accessPermission(action: string): string {
  return action.startsWith("access:") ? action.slice("access:".length) : action
}

/**
 * The row's own known grant expiry — only ever available for a request
 * THIS session just decided (`decidedGrants`, keyed by approval id). A
 * row read back from `GET /api/agents/approvals` never carries it (that
 * route lists `approval_item` rows only; the grant's `expires_at` lives
 * in a separate `access_grant` row nothing joins in — see
 * `@/lib/access-requests`'s `accessGrantState` doc comment). Showing
 * nothing here for an older approved request is the honest choice, never
 * a fabricated date.
 */
function statusCell(r: ApprovalItem, decidedGrants: Record<string, string>) {
  if (r.kind !== "access") return <ApprovalBadge status={r.status} />
  const expiresAt = decidedGrants[r.id]
  const grantState = accessGrantState(r.status, expiresAt, Date.now())
  if (grantState === "approved-expired") {
    // Deliberately NOT the "Approved" tone — an expired grant must never
    // read as still active.
    return <Pill tone="neutral">Expired</Pill>
  }
  return (
    <span className="flex flex-wrap items-center gap-1.5">
      <ApprovalBadge status={r.status} />
      {grantState === "approved-active" && expiresAt ? (
        <span className="text-xs text-muted-foreground">
          until {formatRelativeTime(expiresAt)}
        </span>
      ) : null}
    </span>
  )
}

export function ApprovalsPage() {
  const state = useService((s) => agentService.listApprovals(undefined, s), [])
  const { user } = useAuth()
  const [search, setSearch] = React.useState("")
  const [status, setStatus] = React.useState<ApprovalStatus | "all">("pending")
  const [selected, setSelected] = React.useState<ApprovalItem | null>(null)
  const [decision, setDecision] = React.useState<"approved" | "rejected" | null>(
    null
  )
  const [comment, setComment] = React.useState("")
  const [executionResult, setExecutionResult] = React.useState<{
    executed: boolean
    result?: unknown
  } | null>(null)
  // Grants this session decided, keyed by approval id — see `statusCell`'s
  // doc comment on why an already-decided access request otherwise has no
  // expiry to show.
  const [decidedGrants, setDecidedGrants] = React.useState<Record<string, string>>({})

  // N1 (WS7 plan): a `kind = "tool_call"` decision and a `kind = "access"`
  // decision are two different routes with two different server-side
  // authorities (`agent:approve` vs `access:approve`) — never one call
  // that guesses which endpoint to hit.
  const decideToolCall = useServiceAction(
    (signal, id: string, input: { decision: "approved" | "rejected"; comment?: string }) =>
      agentService.decideApproval(id, input, signal)
  )
  const decideAccess = useServiceAction(
    (signal, id: string, dec: "approved" | "rejected", cmt: string | undefined) => {
      // `decideAccessRequest` is optional on `AssetService` only so the
      // dead `mock/assets.ts` fixture still satisfies the interface — see
      // that contract's own doc comment. Fail closed rather than
      // silently no-op if that assumption is ever wrong.
      if (!assetService.decideAccessRequest) {
        return Promise.reject(new Error("This deployment cannot decide access requests."))
      }
      return assetService.decideAccessRequest(id, dec, cmt, signal)
    }
  )
  const deciding = decideToolCall.status === "pending" || decideAccess.status === "pending"
  const decideError = decideToolCall.error ?? decideAccess.error

  const columns: ColumnDef<ApprovalItem>[] = [
    {
      key: "action",
      header: "Requested action",
      render: (r) => (
        <div>
          <p className="font-medium">
            {r.kind === "access" ? `Access: ${accessPermission(r.action)}` : r.action}
          </p>
          <p className="text-xs text-muted-foreground">
            {r.kind === "access" ? (r.reason || "—") : r.employeeName}
          </p>
        </div>
      ),
    },
    {
      key: "status",
      header: "Status",
      render: (r) => statusCell(r, decidedGrants),
    },
    { key: "risk", header: "Risk", render: (r) => r.risk || "—" },
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

  const rows = React.useMemo(() => {
    if (state.status !== "success") return []
    const q = search.trim().toLowerCase()
    return state.data.filter((a) => {
      if (status !== "all" && a.status !== status) return false
      if (!q) return true
      return [a.action, a.employeeName, a.risk, a.resource ?? "", a.reason ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(q)
    })
  }, [state.status, state.data, search, status])

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

  const selfApproval =
    selected !== null && user !== null && isOwnAccessRequest(selected, user.id)

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Approvals"
        description="Human review gate for higher-risk agent actions and catalog access requests. Approve or reject with impact context before anything takes effect."
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
          description="A request lands here whenever a run — from the copilot chat, Run now, or a schedule — hits a high-risk (WriteHigh) tool call, or a catalog viewer requests a permission they don't hold. Nothing is waiting on you right now."
          action={
            <Button size="sm" variant="outline" render={<Link href="/agents/runs" />}>
              View agent runs
            </Button>
          }
        />
      ) : null}
      {state.status === "success" && rows.length > 0 ? (
        <DataTable
          columns={columns}
          rows={rows}
          rowKey={(r) => r.id}
          onRowClick={(r) => {
            setExecutionResult(null)
            setSelected(r)
          }}
        />
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
          <>
            <div className="flex flex-wrap items-center gap-2">
              {statusCell(selected, decidedGrants)}
              {selected.status === "pending" && selfApproval ? (
                <Tooltip>
                  <TooltipTrigger
                    render={
                      <span>
                        <Button size="sm" disabled>
                          <CheckIcon data-icon="inline-start" />
                          Approve
                        </Button>
                      </span>
                    }
                  />
                  <TooltipContent>
                    You requested this access; a different Governance Admin must
                    decide it
                  </TooltipContent>
                </Tooltip>
              ) : null}
              {selected.status === "pending" && !selfApproval ? (
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
                  // T3.4 of the copilot-operations-handover plan added
                  // `/agents/runs/[id]`, so this links there now instead of
                  // showing the run id as plain text.
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
                  // WS1 T11 removed the Agent Workflows page, so this
                  // renders the workflow id as plain text instead of a
                  // link that would land on a missing route.
                  label: "Workflow",
                  value: selected.workflowId ? (
                    <span className="font-mono text-xs">
                      {selected.workflowId}
                    </span>
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
                  value: (() => {
                    const grantExpiry =
                      selected.kind === "access"
                        ? decidedGrants[selected.id]
                        : selected.expiresAt
                    return grantExpiry ? formatRelativeTime(grantExpiry) : "—"
                  })(),
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
            ? `${decision === "approved" ? "Approve" : "Reject"} “${
                selected.kind === "access"
                  ? `Access: ${accessPermission(selected.action)}`
                  : selected.action
              }”?`
            : "Confirm this approval decision."
        }
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
