import { cn } from "@/lib/utils"

/**
 * One tile of a strip of facts at the top of an entity's overview (the asset
 * page's health strip, the connector page's). A button, not a link: every
 * tile that has somewhere to go opens a tab on the same page, where the
 * detail behind the number lives; one without `onClick` is plain text.
 * Lay a strip out as `grid gap-2 sm:grid-cols-2 lg:grid-cols-4`.
 */
export function HealthTile({
  label,
  children,
  hint,
  onClick,
}: {
  label: string
  children: React.ReactNode
  hint?: React.ReactNode
  onClick?: () => void
}) {
  const className = cn(
    "flex flex-col items-start gap-1.5 rounded-lg border border-border bg-card p-3 text-left",
    onClick && "transition-colors hover:border-primary/40 hover:bg-muted/30"
  )
  const body = (
    <>
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      <span className="flex min-h-6 flex-wrap items-center gap-1.5 text-sm font-medium">
        {children}
      </span>
      {hint ? <span className="text-xs text-muted-foreground">{hint}</span> : null}
    </>
  )
  return onClick ? (
    <button type="button" className={className} onClick={onClick}>
      {body}
    </button>
  ) : (
    <div className={className}>{body}</div>
  )
}
