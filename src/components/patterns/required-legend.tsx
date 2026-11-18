"use client"

import { RequiredMarker } from "@/components/ui/required"
import { cn } from "@/lib/utils"

/**
 * One-per-form explanation of the shared required marker (UX-1). Placed at
 * the top of a form or wizard step, not next to every star: the convention is
 * stated once where the reader looks. The star is decorative and the words
 * carry the meaning for screen readers.
 */
export function RequiredLegend({ className }: { className?: string }) {
  return (
    <p className={cn("flex items-center gap-1 text-xs text-muted-foreground", className)}>
      <RequiredMarker /> Required
    </p>
  )
}
