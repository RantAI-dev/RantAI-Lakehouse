"use client"

import { cn } from "@/lib/utils"

/**
 * Decorative star for a field that needs a supplied value (UX-1). The star is
 * drawn by CSS `::before` on an empty, `aria-hidden` span rather than as a
 * text node: it stays visible, but does not enter the label's accessible name
 * or `textContent`, so existing `getByLabelText("Name")` queries and screen
 * readers both keep hearing just "Name". Required semantics belong on the
 * control (native `required` or `aria-required`). Markers are opt-in - callers
 * pass `required` to `Label`/`FieldLabel` (or render this for a custom
 * control) - rather than inferred from children, so an optional field can
 * never acquire a star by accident.
 */
export function RequiredMarker({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn("text-destructive leading-none before:content-['*']", className)}
    />
  )
}
