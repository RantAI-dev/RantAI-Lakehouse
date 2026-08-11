import { cn } from "@/lib/utils"

export type MetadataItem = {
  label: string
  value: React.ReactNode
}

/** Two-column definition list for entity metadata panels and drawers. */
export function MetadataList({
  items,
  columns = 2,
  className,
}: {
  items: MetadataItem[]
  columns?: 1 | 2 | 3
  className?: string
}) {
  return (
    <dl
      className={cn(
        "grid gap-x-6 gap-y-3",
        columns === 1 && "grid-cols-1",
        columns === 2 && "grid-cols-1 sm:grid-cols-2",
        columns === 3 && "grid-cols-1 sm:grid-cols-3",
        className
      )}
    >
      {items.map((item) => (
        <div key={item.label} className="min-w-0">
          <dt className="text-xs font-medium text-muted-foreground">
            {item.label}
          </dt>
          <dd className="mt-0.5 text-sm text-foreground">{item.value}</dd>
        </div>
      ))}
    </dl>
  )
}
