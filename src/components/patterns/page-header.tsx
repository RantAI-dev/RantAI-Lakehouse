import { cn } from "@/lib/utils"

/**
 * Standard page header: title + concise purpose on the left, primary and
 * contextual actions on the right. Every primary page starts with this.
 */
export function PageHeader({
  title,
  description,
  actions,
  className,
}: {
  /**
   * Usually a plain string. Accepts a node so a page whose title IS the
   * thing being chosen can put a picker here - the Dashboards header
   * renders its board switcher as the title rather than bolting a second
   * control onto the side.
   */
  title: React.ReactNode
  description?: string
  actions?: React.ReactNode
  className?: string
}) {
  return (
    <div
      className={cn(
        "flex flex-col gap-3 border-b border-border pb-4 sm:flex-row sm:items-end sm:justify-between",
        className
      )}
    >
      <div className="min-w-0">
        <h1 className="text-2xl font-semibold leading-8 tracking-[-0.02em] text-foreground">
          {title}
        </h1>
        {description ? (
          <p className="mt-1 max-w-3xl text-sm leading-5 text-muted-foreground">
            {description}
          </p>
        ) : null}
      </div>
      {actions ? (
        <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>
      ) : null}
    </div>
  )
}

/**
 * Header block for entity detail pages: back-link slot, entity name,
 * identifier, badges, and actions.
 */
export function EntityHeader({
  eyebrow,
  title,
  titleAccessory,
  description,
  actions,
  className,
}: {
  eyebrow?: React.ReactNode
  title: string
  titleAccessory?: React.ReactNode
  description?: string
  actions?: React.ReactNode
  className?: string
}) {
  return (
    <div
      className={cn(
        "flex flex-col gap-3 border-b border-border pb-4 sm:flex-row sm:items-end sm:justify-between",
        className
      )}
    >
      <div className="min-w-0">
        {eyebrow ? (
          <div className="mb-1 flex items-center gap-2 text-xs text-muted-foreground">
            {eyebrow}
          </div>
        ) : null}
        <div className="flex flex-wrap items-center gap-2">
          <h1 className="truncate text-2xl font-semibold leading-8 tracking-[-0.02em] text-foreground">
            {title}
          </h1>
          {titleAccessory}
        </div>
        {description ? (
          <p className="mt-1 max-w-3xl text-sm leading-5 text-muted-foreground">
            {description}
          </p>
        ) : null}
      </div>
      {actions ? (
        <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>
      ) : null}
    </div>
  )
}
