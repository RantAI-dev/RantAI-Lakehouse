"use client"

import * as React from "react"
import Link from "next/link"
import { useParams, useRouter } from "next/navigation"
import { BanIcon, PauseIcon, PlayIcon, PlayCircleIcon } from "lucide-react"
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
  AgentRunStatusBadge,
  ApprovalBadge,
  AutonomyBadge,
  StatusBadge,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
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
        description="This employee has not executed any runs. Trigger one with Run now above, or set a schedule so it runs on its own."
        action={
          <Button size="sm" variant="outline" render={<Link href="/agents/runs" />}>
            View all runs
          </Button>
        }
      />
    )
  }
  return (
    <ul className="space-y-3">
      {runs.map((r) => (
        <li key={r.id} className="rounded-md border border-border p-3 text-sm">
          <div className="flex flex-wrap items-center gap-2">
            <Link
              href={`/agents/runs/${encodeURIComponent(r.id)}`}
              className="font-mono text-xs font-medium hover:underline"
            >
              {r.id}
            </Link>
            <AgentRunStatusBadge status={r.status} />
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
          {r.status === "waiting_approval" && r.approvals.length > 0 ? (
            <p className="mt-2 rounded border border-amber-500/30 bg-amber-500/10 px-2 py-1.5 text-xs text-amber-700 dark:text-amber-400">
              Blocked on a human decision.{" "}
              <Link
                href={`/agents/approvals?id=${encodeURIComponent(r.approvals[r.approvals.length - 1].id)}`}
                className="font-medium underline underline-offset-2"
              >
                Review the approval →
              </Link>
            </p>
          ) : null}
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

/** `PermissionSet`-formatted `resource:action` string, split for legible
 * display in the run-configuration panel. */
function formatPermissionCeiling(permissions: string): string[] {
  return permissions
    .split(",")
    .map((p) => p.trim())
    .filter(Boolean)
}

export function EmployeeDetailPage() {
  const { employeeId } = useParams<{ employeeId: string }>()
  const router = useRouter()
  const { hasPermission } = useAuth()
  const canRun = hasPermission("agent:manage")
  const employee = useService((s) => agentService.getEmployee(employeeId, s), [employeeId])
  const runs = useService((s) => agentService.listRuns(employeeId, s), [employeeId])
  const approvals = useService((s) => agentService.listApprovals(employeeId, s), [employeeId])
  const [confirm, setConfirm] = React.useState<"suspend" | "revoke" | null>(null)
  const [runDialogOpen, setRunDialogOpen] = React.useState(false)
  const [promptOverride, setPromptOverride] = React.useState("")
  const suspendAction = useServiceAction((signal, id: string) =>
    agentService.suspendEmployee(id, signal)
  )
  const resumeAction = useServiceAction((signal, id: string) =>
    agentService.resumeEmployee(id, signal)
  )
  const revokeAction = useServiceAction((signal, id: string) =>
    agentService.revokeEmployee(id, signal)
  )
  const runAction = useServiceAction((signal, id: string, prompt: string) =>
    agentService.runEmployee(id, prompt ? { prompt } : undefined, signal)
  )

  if (employee.status === "loading") return <LoadingSkeleton rows={8} />
  if (employee.status === "error") return <ErrorState error={employee.error} onRetry={employee.reload} />
  const e = employee.data
  const isPaused = e.status === "paused"
  const isRevoked = e.status === "cancelled"
  const isCopilot = employeeId === "emp-copilot"
  const hasPrompt = Boolean(e.prompt?.trim())
  const runDisabledReason = isCopilot
    ? "emp-copilot is the interactive-chat row and can't be run headlessly."
    : !canRun
      ? "You don't have permission to run digital employees (requires agent:manage)."
      : isRevoked
        ? "This employee is revoked and can't run again until re-provisioned."
        : isPaused
          ? "This employee is suspended. Resume it before running."
          : undefined

  return (
    <div className="flex flex-col gap-4">
      <EntityHeader
        eyebrow={<Link href="/agents/employees" className="hover:underline">Digital Employees</Link>}
        title={e.name}
        titleAccessory={<><AutonomyBadge level={e.autonomy} /><StatusBadge status={e.status} /></>}
        description={e.purpose}
        actions={
          <>
            {!isCopilot ? (
              <Button
                size="sm"
                disabled={Boolean(runDisabledReason)}
                title={runDisabledReason}
                onClick={() => {
                  setPromptOverride("")
                  runAction.reset()
                  setRunDialogOpen(true)
                }}
              >
                <PlayCircleIcon data-icon="inline-start" />
                Run now
              </Button>
            ) : null}
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
      <ConfirmActionDialog
        open={runDialogOpen}
        onOpenChange={(open) => {
          setRunDialogOpen(open)
          if (!open) runAction.reset()
        }}
        title="Run now"
        description={`Run ${e.name} headlessly, right now, in ${e.mode === "build" ? "build" : "ask"} mode.`}
        impact={
          e.permissions
            ? `This run is capped by the employee's permission ceiling: ${e.permissions}. It cannot do anything outside that, no matter who triggers it.`
            : "This employee has no permissions granted, so its run is limited to authenticated read-only tools."
        }
        confirmLabel="Run now"
        confirming={runAction.status === "pending"}
        onConfirm={async () => {
          const run = await runAction.run(employeeId, promptOverride)
          if (run) {
            setRunDialogOpen(false)
            runs.reload()
            router.push(`/agents/runs/${encodeURIComponent(run.id)}`)
          }
        }}
      >
        <div className="space-y-2">
          {hasPrompt ? (
            <p className="text-xs text-muted-foreground">
              Runs with this employee&apos;s own prompt unless you override it below.
            </p>
          ) : (
            <p className="text-xs text-amber-600 dark:text-amber-400">
              This employee has no prompt configured — you must supply one below or the run
              will be refused.
            </p>
          )}
          <Textarea
            value={promptOverride}
            onChange={(ev) => setPromptOverride(ev.target.value)}
            placeholder={
              hasPrompt
                ? "Optional prompt override for this run only"
                : "Prompt for this run (required — no prompt is configured)"
            }
            rows={3}
          />
          {runAction.status === "error" ? (
            <p className="text-sm text-destructive">{runAction.error.message}</p>
          ) : null}
        </div>
      </ConfirmActionDialog>
      <SectionCard
        title="Run configuration"
        description="What a headless run (Run now, or a schedule) sends to the copilot and what it's allowed to do."
      >
        <div className="grid gap-4 sm:grid-cols-2">
          <div>
            <p className="text-xs font-medium text-muted-foreground">Mode</p>
            <p className="mt-0.5 text-sm">
              {e.mode === "build" ? "Build — may call write tools" : "Ask — read-only"}
            </p>
          </div>
          <div>
            <p className="text-xs font-medium text-muted-foreground">Schedule</p>
            <p className="mt-0.5 text-sm">
              {e.scheduleCron ? (
                <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs">
                  {e.scheduleCron}
                </code>
              ) : (
                "Manual only — no schedule is configured; this employee only runs from Run now."
              )}
            </p>
          </div>
          <div className="sm:col-span-2">
            <p className="text-xs font-medium text-muted-foreground">Permission ceiling</p>
            {formatPermissionCeiling(e.permissions).length > 0 ? (
              <div className="mt-1 flex flex-wrap gap-1.5">
                {formatPermissionCeiling(e.permissions).map((p) => (
                  <code
                    key={p}
                    className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs"
                  >
                    {p}
                  </code>
                ))}
              </div>
            ) : (
              <p className="mt-0.5 text-sm text-muted-foreground">
                Empty — authenticated read-only only, no write tools.
              </p>
            )}
            <p className="mt-1.5 text-xs text-muted-foreground">
              This is a CEILING on what this employee&apos;s own runs may do — it never grants
              anything to whoever or whatever triggers the run (a schedule, or a person clicking
              Run now); it only narrows what the run itself can call.
            </p>
          </div>
          <div className="sm:col-span-2">
            <p className="text-xs font-medium text-muted-foreground">Prompt</p>
            {hasPrompt ? (
              <pre className="mt-1 overflow-x-auto whitespace-pre-wrap rounded border border-border bg-muted/40 px-2.5 py-2 text-xs text-foreground">
                {e.prompt}
              </pre>
            ) : (
              <p className="mt-0.5 text-sm text-muted-foreground">
                No prompt configured — a run needs an override supplied at run time, or it&apos;s
                refused.
              </p>
            )}
            <p className="mt-1.5 text-xs text-muted-foreground">
              Prompt, schedule, mode, and permission ceiling are set when the employee is
              created; editing them for an existing employee isn&apos;t available in this
              console yet.
            </p>
          </div>
        </div>
      </SectionCard>
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
