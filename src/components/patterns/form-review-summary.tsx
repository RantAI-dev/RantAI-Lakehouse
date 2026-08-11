import { cn } from "@/lib/utils"

export type ReviewSection = {
  title: string
  items: { label: string; value: React.ReactNode }[]
}

/** Read-only summary blocks for the final step of a create wizard. */
export function FormReviewSummary({
  sections,
  className,
}: {
  sections: ReviewSection[]
  className?: string
}) {
  return (
    <div className={cn("space-y-5", className)}>
      {sections.map((section) => (
        <section key={section.title} className="space-y-2">
          <h3 className="text-sm font-medium text-foreground">{section.title}</h3>
          <dl className="grid gap-x-6 gap-y-2 rounded-lg border border-border bg-muted/30 p-3 sm:grid-cols-2">
            {section.items.map((item) => (
              <div key={item.label} className="min-w-0">
                <dt className="text-xs text-muted-foreground">{item.label}</dt>
                <dd className="mt-0.5 text-sm text-foreground">{item.value || "—"}</dd>
              </div>
            ))}
          </dl>
        </section>
      ))}
    </div>
  )
}
