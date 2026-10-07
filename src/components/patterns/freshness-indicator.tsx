import { cn } from "@/lib/utils"
import { formatLagSeconds } from "@/lib/format"

const BASE = "inline-flex items-center gap-1.5 whitespace-nowrap text-xs font-medium"

/** Whose word a freshness target is, as the tooltip puts it. */
const TARGET_SOURCE_LABEL = {
  sla: "freshness SLA",
  frequency: "refresh frequency",
} as const

/**
 * How old an asset's data is and, when something says how old is too old,
 * whether that is on time. Always includes text, so color is never the
 * only signal.
 *
 * `lagSeconds === null` means the backend has not measured freshness for
 * this asset. It renders as a neutral "Not measured" state, never as a
 * verdict — a null lag is not evidence of freshness.
 *
 * `targetSeconds` picks how a measured age is judged:
 * - a number — the asset's own target: an authored freshness SLA, or the
 *   registry's refresh cadence (`targetSource`). On time at or under it,
 *   late past it; the rule the overview's freshness strip applies.
 * - `null` — nothing says how often this asset should refresh, so the age
 *   is shown with no verdict. A table refreshed once a day is not "stale"
 *   at three hours old, and nothing here knows that it is not.
 * - omitted — a streaming watermark with no per-asset target, judged on
 *   fixed thresholds: fresh ≤ 60 s, lagging ≤ 1 h, stale beyond that.
 */
export function FreshnessIndicator({
  lagSeconds,
  targetSeconds,
  targetSource,
  className,
}: {
  lagSeconds: number | null
  targetSeconds?: number | null
  targetSource?: "sla" | "frequency" | null
  className?: string
}) {
  if (lagSeconds === null) {
    return (
      <span className={cn(BASE, "text-muted-foreground", className)} title="Freshness not measured">
        <span className="size-1.5 rounded-full bg-current" aria-hidden />
        Not measured
      </span>
    )
  }
  const age = formatLagSeconds(lagSeconds)

  if (targetSeconds === null) {
    return (
      <span
        className={cn(BASE, "text-muted-foreground", className)}
        title={`Last written ${age} ago. No refresh interval is set for this asset, so its age is not judged.`}
      >
        <span className="size-1.5 rounded-full bg-current" aria-hidden />
        {age} ago
      </span>
    )
  }

  if (targetSeconds !== undefined) {
    const onTime = lagSeconds <= targetSeconds
    const source = targetSource ? ` (${TARGET_SOURCE_LABEL[targetSource]})` : ""
    return (
      <span
        className={cn(
          BASE,
          onTime ? "text-emerald-600 dark:text-emerald-400" : "text-destructive",
          className
        )}
        title={`Last written ${age} ago. Expected within ${formatLagSeconds(targetSeconds)}${source}.`}
      >
        <span className="size-1.5 rounded-full bg-current" aria-hidden />
        {onTime ? "On time" : "Late"} · {age}
      </span>
    )
  }

  const level =
    lagSeconds <= 60 ? "fresh" : lagSeconds <= 3600 ? "lagging" : "stale"
  const label =
    level === "fresh" ? "Fresh" : level === "lagging" ? "Lagging" : "Stale"
  return (
    <span
      className={cn(
        BASE,
        level === "fresh" && "text-emerald-600 dark:text-emerald-400",
        level === "lagging" && "text-amber-600 dark:text-amber-400",
        level === "stale" && "text-destructive",
        className
      )}
      title={`Watermark lag: ${age}`}
    >
      <span className="size-1.5 rounded-full bg-current" aria-hidden />
      {label} · {age}
    </span>
  )
}
