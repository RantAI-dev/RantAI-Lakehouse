import { cn } from "@/lib/utils"

/**
 * Compact KPI card used in summary strips.
 * `hint` explains the metric; `trend` is short delta copy like "+4.2% 7d".
 */
export function MetricCard({
  label,
  value,
  hint,
  trend,
  trendTone = "neutral",
  icon,
  className,
}: {
  label: string
  value: React.ReactNode
  hint?: string
  trend?: string
  trendTone?: "positive" | "negative" | "neutral"
  icon?: React.ReactNode
  className?: string
}) {
  return (
    <div
      className={cn(
        "rounded-lg border border-border bg-card p-4 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]",
        className
      )}
    >
      <div className="flex items-center justify-between gap-2">
        <p className="truncate text-xs font-medium text-muted-foreground">
          {label}
        </p>
        {icon ? <span className="text-muted-foreground">{icon}</span> : null}
      </div>
      <div className="mt-1.5 flex items-baseline gap-2">
        <span className="text-2xl font-semibold leading-8 tracking-[-0.02em] text-foreground">
          {value}
        </span>
        {trend ? (
          <span
            className={cn(
              "text-xs font-medium",
              trendTone === "positive" &&
                "text-emerald-600 dark:text-emerald-400",
              trendTone === "negative" && "text-destructive",
              trendTone === "neutral" && "text-muted-foreground"
            )}
          >
            {trend}
          </span>
        ) : null}
      </div>
      {hint ? (
        <p className="mt-1 truncate text-xs text-muted-foreground">{hint}</p>
      ) : null}
    </div>
  )
}

/** Responsive grid wrapper for MetricCards. */
export function MetricGrid({
  children,
  className,
}: {
  children: React.ReactNode
  className?: string
}) {
  return (
    <div className={cn("grid gap-3 sm:grid-cols-2 lg:grid-cols-4", className)}>
      {children}
    </div>
  )
}
