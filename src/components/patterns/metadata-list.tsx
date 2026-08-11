import { cn } from "@/lib/utils"

export type MetadataItem = {
  label: string
  value: React.ReactNode
}

/** Two-column definition list for entity metadata panels and drawers. */
export function MetadataList({
  items,
  columns = 2,
  density = "default",
  className,
}: {
  items: MetadataItem[]
  columns?: 1 | 2 | 3 | 4
  /** Compact = tighter gaps and inline label/value for denser detail headers. */
  density?: "default" | "compact"
  className?: string
}) {
  const compact = density === "compact"

  return (
    <dl
      className={cn(
        "grid",
        compact ? "gap-x-6 gap-y-2" : "gap-x-6 gap-y-3",
        columns === 1 && "grid-cols-1",
        columns === 2 && "grid-cols-1 sm:grid-cols-2",
        columns === 3 && "grid-cols-1 sm:grid-cols-2 lg:grid-cols-3",
        columns === 4 && "grid-cols-1 sm:grid-cols-2 lg:grid-cols-4",
        className
      )}
    >
      {items.map((item) => (
        <div
          key={item.label}
          className={cn(
            "min-w-0",
            compact && "flex items-baseline gap-2"
          )}
        >
          <dt
            className={cn(
              "text-xs font-medium text-muted-foreground",
              compact && "shrink-0"
            )}
          >
            {item.label}
          </dt>
          <dd
            className={cn(
              "text-sm text-foreground",
              compact ? "min-w-0 truncate" : "mt-0.5"
            )}
          >
            {item.value}
          </dd>
        </div>
      ))}
    </dl>
  )
}
