import { cn } from "@/lib/utils"
import type { EntityStatus } from "@/lib/status"
import { StatusBadge } from "./status-badge"

export type TimelineStep = {
  id: string
  label: string
  status: EntityStatus
  at?: string
  description?: string
  meta?: React.ReactNode
}

/**
 * Vertical status timeline for run details, approvals, and agent steps.
 * Top-to-bottom order matches lifecycle progression.
 */
export function RunTimeline({
  steps,
  className,
}: {
  steps: TimelineStep[]
  className?: string
}) {
  return (
    <ol className={cn("flex flex-col", className)}>
      {steps.map((step, i) => (
        <li key={step.id} className="relative flex gap-3 pb-5 last:pb-0">
          {i < steps.length - 1 ? (
            <span
              className="absolute left-[7px] top-5 h-full w-px bg-border"
              aria-hidden
            />
          ) : null}
          <span
            className={cn(
              "mt-1 size-[15px] shrink-0 rounded-full border-2 border-background ring-1",
              step.status === "completed" && "bg-emerald-500 ring-emerald-500/40",
              step.status === "running" && "animate-pulse bg-primary ring-primary/40",
              step.status === "failed" && "bg-destructive ring-destructive/40",
              step.status === "degraded" && "bg-amber-500 ring-amber-500/40",
              !["completed", "running", "failed", "degraded"].includes(
                step.status
              ) && "bg-muted-foreground/40 ring-border"
            )}
            aria-hidden
          />
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-sm font-medium text-foreground">
                {step.label}
              </span>
              <StatusBadge status={step.status} />
              {step.at ? (
                <span className="text-xs text-muted-foreground">{step.at}</span>
              ) : null}
            </div>
            {step.description ? (
              <p className="mt-0.5 text-sm text-muted-foreground">
                {step.description}
              </p>
            ) : null}
            {step.meta ? <div className="mt-1.5">{step.meta}</div> : null}
          </div>
        </li>
      ))}
    </ol>
  )
}
