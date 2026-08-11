import { cn } from "@/lib/utils"
import { formatLagSeconds } from "@/lib/format"

/**
 * Freshness readout backed by table watermarks.
 * Thresholds: fresh ≤ 60 s, lagging ≤ 1 h, stale beyond that.
 * Always includes text so color is never the only signal.
 */
export function FreshnessIndicator({
  lagSeconds,
  className,
}: {
  lagSeconds: number
  className?: string
}) {
  const level =
    lagSeconds <= 60 ? "fresh" : lagSeconds <= 3600 ? "lagging" : "stale"
  const label =
    level === "fresh" ? "Fresh" : level === "lagging" ? "Lagging" : "Stale"
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 whitespace-nowrap text-xs font-medium",
        level === "fresh" && "text-emerald-600 dark:text-emerald-400",
        level === "lagging" && "text-amber-600 dark:text-amber-400",
        level === "stale" && "text-destructive",
        className
      )}
      title={`Watermark lag: ${formatLagSeconds(lagSeconds)}`}
    >
      <span className="size-1.5 rounded-full bg-current" aria-hidden />
      {label} · {formatLagSeconds(lagSeconds)}
    </span>
  )
}
