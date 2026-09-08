"use client"

import Link from "next/link"
import { useParams } from "next/navigation"
import { EntityHeader } from "@/components/patterns/page-header"
import {
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { RunTimeline } from "@/components/patterns/run-timeline"
import { MetadataList } from "@/components/patterns/metadata-list"
import { SectionCard } from "@/components/patterns/section-card"
import { AgentRunStatusBadge, ApprovalBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import { formatCost, formatDateTime, formatRelativeTime } from "@/lib/format"
import { agentService } from "@/services"

/**
 * `/agents/runs/[runId]` — full trace of one `agent_run`: status, trigger,
 * actor, timings, its step-by-step tool-call history, and any linked
 * approval. Previously this route did not exist (T3.4 of the
 * copilot-operations-handover plan) — the approvals drawer showed a run id
 * as plain text rather than link to a page that would 404; it now links
 * here instead.
 */
export function RunDetailPage() {
  const { runId } = useParams<{ runId: string }>()
  const run = useService((s) => agentService.getRun(runId, s), [runId])
  const employeeId = run.data?.employeeId
  const employee = useService(
    (s) => {
      // No employee to look up yet (run still loading) — reject with an
      // AbortError so `useService` treats it as a no-op, not a shown error
      // (see its "aborted requests never surface as errors" contract).
      if (!employeeId) return Promise.reject(new DOMException("no run yet", "AbortError"))
      return agentService.getEmployee(employeeId, s)
    },
    [employeeId]
  )

  if (run.status === "loading") return <LoadingSkeleton rows={8} />
  if (run.status === "error") return <ErrorState error={run.error} onRetry={run.reload} />
  const r = run.data
  const employeeName =
    employee.status === "success" ? employee.data.name : r.employeeId
  const lastApproval = r.approvals.at(-1)

  return (
    <div className="flex flex-col gap-4">
      <EntityHeader
        eyebrow={<Link href="/agents/runs" className="hover:underline">Agent Runs</Link>}
        title={r.id}
        titleAccessory={<AgentRunStatusBadge status={r.status} />}
        description={`${r.trigger} run of ${employeeName}`}
        actions={
          <Button size="sm" variant="outline" render={<Link href={`/agents/employees/${encodeURIComponent(r.employeeId)}`} />}>
            View employee
          </Button>
        }
      />

      {r.status === "waiting_approval" ? (
        <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm">
          <p className="font-medium text-amber-800 dark:text-amber-300">
            Blocked on a human decision
          </p>
          <p className="mt-1 text-amber-700 dark:text-amber-400">
            This run paused before a high-risk tool call and cannot continue until it&apos;s
            approved or rejected.
          </p>
          <Link
            href={
              lastApproval
                ? `/agents/approvals?id=${encodeURIComponent(lastApproval.id)}`
                : "/agents/approvals"
            }
            className="mt-2 inline-flex items-center gap-1 font-medium text-amber-800 underline underline-offset-2 dark:text-amber-300"
          >
            Review the approval →
          </Link>
        </div>
      ) : null}

      <SectionCard title="Summary">
        <MetadataList
          items={[
            {
              label: "Employee",
              value: (
                <Link
                  href={`/agents/employees/${encodeURIComponent(r.employeeId)}`}
                  className="text-primary hover:underline"
                >
                  {employeeName}
                </Link>
              ),
            },
            { label: "Trigger", value: r.trigger },
            { label: "Actor", value: r.actor },
            { label: "Delegated user", value: r.delegatedUser ?? "—" },
            {
              label: "Started",
              value: `${formatDateTime(r.startedAt)} (${formatRelativeTime(r.startedAt)})`,
            },
            {
              label: "Ended",
              value: r.endedAt
                ? `${formatDateTime(r.endedAt)} (${formatRelativeTime(r.endedAt)})`
                : "Still running / awaiting a decision",
            },
            { label: "Budget consumed", value: formatCost(r.budgetConsumed) },
            {
              label: "Audit event",
              value: r.auditEventId ? (
                <Link
                  href={`/audit?event=${encodeURIComponent(r.auditEventId)}`}
                  className="text-primary hover:underline"
                >
                  {r.auditEventId}
                </Link>
              ) : (
                "—"
              ),
            },
          ]}
        />
      </SectionCard>

      {r.approvals.length > 0 ? (
        <SectionCard title="Linked approvals">
          <ul className="space-y-2 text-sm">
            {r.approvals.map((a) => (
              <li
                key={a.id}
                className="flex flex-wrap items-center justify-between gap-2 border-b border-border py-2 last:border-b-0"
              >
                <Link
                  href={`/agents/approvals?id=${encodeURIComponent(a.id)}`}
                  className="font-mono text-xs font-medium hover:underline"
                >
                  {a.id}
                </Link>
                <div className="flex items-center gap-2">
                  <ApprovalBadge status={a.status} />
                  {a.at ? (
                    <span className="text-xs text-muted-foreground">
                      {formatRelativeTime(a.at)}
                    </span>
                  ) : null}
                </div>
              </li>
            ))}
          </ul>
        </SectionCard>
      ) : null}

      <SectionCard title="Steps" description="The full tool-call trace for this run, in order.">
        {r.steps.length === 0 ? (
          <p className="text-sm text-muted-foreground">No steps recorded yet.</p>
        ) : (
          <RunTimeline
            steps={r.steps.map((s) => ({
              id: s.id,
              label: s.label,
              status: s.status,
              description: s.detail,
            }))}
          />
        )}
      </SectionCard>
    </div>
  )
}
