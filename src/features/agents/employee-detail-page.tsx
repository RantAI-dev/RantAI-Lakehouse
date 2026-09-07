"use client"

import * as React from "react"
import Link from "next/link"
import { useParams } from "next/navigation"
import { BanIcon, PauseIcon, PlayIcon } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { EntityHeader } from "@/components/patterns/page-header"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { RunTimeline } from "@/components/patterns/run-timeline"
import { SectionCard } from "@/components/patterns/section-card"
import {
  ApprovalBadge,
  AutonomyBadge,
  StatusBadge,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import {
  formatCost,
  formatPercent,
  formatRelativeTime,
} from "@/lib/format"
import { agentService } from "@/services"
import type { AgentRun, ApprovalItem } from "@/services/contracts/agents"

function ApprovalsSection({ approvals }: { approvals: ApprovalItem[] }) {
  const hasPending = approvals.some((a) => a.status === "pending")

  if (approvals.length === 0) {
    return (
      <EmptyState
        title="No approvals"
        description="This employee has no pending or resolved approval requests."
      />
    )
  }
  return (
    <div className="space-y-3">
      {hasPending ? (
        <p className="text-xs text-muted-foreground">
          Pending requests can be approved or rejected in the{" "}
          <Link href="/agents/approvals" className="font-medium hover:underline">
            Approvals inbox
          </Link>
          .
        </p>
      ) : null}
      <ul className="space-y-2 text-sm">
        {approvals.map((a) => (
          <li
            key={a.id}
            className="flex flex-wrap items-center justify-between gap-2 border-b border-border py-2 last:border-b-0"
          >
            <div className="min-w-0">
              <p>
                <Link
                  href="/agents/approvals"
                  className="font-medium hover:underline"
                >
                  {a.action}
                </Link>
              </p>
              <p className="text-xs text-muted-foreground">
                {a.id} · Risk: {a.risk}
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <ApprovalBadge status={a.status} />
              <span className="text-xs text-muted-foreground">
                {formatRelativeTime(a.requestedAt)}
              </span>
            </div>
          </li>
        ))}
      </ul>
    </div>
  )
}

function RunsSection({ runs }: { runs: AgentRun[] }) {
  if (runs.length === 0) {
    return (
      <EmptyState
        title="No runs yet"
        description="This employee has not executed any runs."
      />
    )
  }
  return (
    <ul className="space-y-3">
      {runs.map((r) => (
        <li key={r.id} className="rounded-md border border-border p-3 text-sm">
          <div className="flex flex-wrap items-center gap-2">
            <StatusBadge status={r.status} />
            <span>{r.trigger}</span>
            {r.delegatedUser ? (
              <span className="text-xs text-muted-foreground">
                on behalf of {r.delegatedUser}
              </span>
            ) : null}
            {r.auditEventId ? (
              <Button
                size="sm"
                variant="ghost"
                className="ml-auto"
                render={<Link href={`/audit?event=${r.auditEventId}`} />}
              >
                Audit
              </Button>
            ) : null}
            <span
              className={`text-xs text-muted-foreground ${r.auditEventId ? "" : "ml-auto"}`}
            >
              {formatCost(r.budgetConsumed)} · started{" "}
              {formatRelativeTime(r.startedAt)}
              {r.endedAt ? ` · ended ${formatRelativeTime(r.endedAt)}` : ""}
            </span>
          </div>
          <RunTimeline
            className="mt-3"
            steps={r.steps.map((s) => ({
              id: s.id,
              label: s.label,
              status: s.status,
              description: s.detail,
            }))}
          />
        </li>
      ))}
    </ul>
  )
}

export function EmployeeDetailPage() {
  const { employeeId } = useParams<{ employeeId: string }>()
  const employee = useService((s) => agentService.getEmployee(employeeId, s), [employeeId])
  const runs = useService((s) => agentService.listRuns(employeeId, s), [employeeId])
  const approvals = useService((s) => agentService.listApprovals(employeeId, s), [employeeId])
  const [confirm, setConfirm] = React.useState<"suspend" | "revoke" | null>(null)
  const suspendAction = useServiceAction(
    withNotify(
      { success: "Employee suspended", error: "Failed to suspend employee" },
      (signal, id: string) => agentService.suspendEmployee(id, signal)
    )
  )
  const resumeAction = useServiceAction(
    withNotify(
      { success: "Employee resumed", error: "Failed to resume employee" },
      (signal, id: string) => agentService.resumeEmployee(id, signal)
    )
  )
  const revokeAction = useServiceAction(
    withNotify(
      { success: "Employee revoked", error: "Failed to revoke employee" },
      (signal, id: string) => agentService.revokeEmployee(id, signal)
    )
  )

  if (employee.status === "loading") return <LoadingSkeleton rows={8} />
  if (employee.status === "error") return <ErrorState error={employee.error} onRetry={employee.reload} />
  const e = employee.data
  const isPaused = e.status === "paused"
  const isRevoked = e.status === "cancelled"

  return (
    <div className="flex flex-col gap-4">
      <EntityHeader
        eyebrow={<Link href="/agents/employees" className="hover:underline">Digital Employees</Link>}
        title={e.name}
        titleAccessory={<><AutonomyBadge level={e.autonomy} /><StatusBadge status={e.status} /></>}
        description={e.purpose}
        actions={
          <>
            {isPaused ? (
              <Button
                variant="outline"
                size="sm"
                disabled={resumeAction.status === "pending"}
                onClick={async () => {
                  const updated = await resumeAction.run(employeeId)
                  if (updated) employee.reload()
                }}
              >
                <PlayIcon data-icon="inline-start" />
                {resumeAction.status === "pending" ? "Resuming…" : "Resume"}
              </Button>
            ) : (
              <Button
                variant="outline"
                size="sm"
                disabled={isRevoked}
                onClick={() => setConfirm("suspend")}
              >
                <PauseIcon data-icon="inline-start" />
                Suspend
              </Button>
            )}
            <Button
              variant="destructive"
              size="sm"
              disabled={isRevoked}
              onClick={() => setConfirm("revoke")}
            >
              <BanIcon data-icon="inline-start" />
              Revoke
            </Button>
          </>
        }
      />
      <ConfirmActionDialog
        open={confirm !== null}
        onOpenChange={(open) => {
          if (!open) setConfirm(null)
        }}
        title={confirm === "revoke" ? "Revoke employee" : "Suspend employee"}
        description={
          confirm === "revoke"
            ? `Revoke ${e.name}? The employee cannot run again until re-provisioned.`
            : `Suspend ${e.name}? New runs are blocked until resumed.`
        }
        impact={
          confirm === "revoke"
            ? "Active runs are cancelled; credentials and tool grants are revoked."
            : "In-flight runs finish; scheduled triggers are held."
        }
        confirmLabel={confirm === "revoke" ? "Revoke" : "Suspend"}
        destructive={confirm === "revoke"}
        confirming={
          suspendAction.status === "pending" || revokeAction.status === "pending"
        }
        onConfirm={async () => {
          const updated =
            confirm === "revoke"
              ? await revokeAction.run(employeeId)
              : await suspendAction.run(employeeId)
          if (updated) {
            setConfirm(null)
            employee.reload()
          }
        }}
      />
      <div className="grid gap-3 lg:grid-cols-3">
        <SectionCard title="Budget">
          <p className="text-2xl font-semibold tabular-nums">
            {formatCost(e.budgetSpent + e.budgetReserved)}
            <span className="text-base text-muted-foreground">
              {" "}/ {formatCost(e.budgetLimit)}
            </span>
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            Spent {formatCost(e.budgetSpent)} · Reserved {formatCost(e.budgetReserved)}
          </p>
        </SectionCard>
        <SectionCard title="Scope">
          <p className="text-sm">{e.dataScope}</p>
          <p className="mt-2 text-xs text-muted-foreground">Tools: {e.allowedTools.join(", ")}</p>
        </SectionCard>
        <SectionCard title="Outcomes">
          <p className="text-sm">Success {formatPercent(e.successRate)} · Approval {formatPercent(e.approvalRate)}</p>
          <p className="mt-1 text-xs text-muted-foreground">{e.recentRuns} recent runs</p>
        </SectionCard>
      </div>
      <SectionCard title="Approval queue">
        {approvals.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
        {approvals.status === "error" ? (
          <ErrorState error={approvals.error} onRetry={approvals.reload} />
        ) : null}
        {approvals.status === "success" ? (
          <ApprovalsSection approvals={approvals.data} />
        ) : null}
      </SectionCard>
      <SectionCard title="Recent runs">
        {runs.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
        {runs.status === "error" ? (
          <ErrorState error={runs.error} onRetry={runs.reload} />
        ) : null}
        {runs.status === "success" ? <RunsSection runs={runs.data} /> : null}
      </SectionCard>
    </div>
  )
}
