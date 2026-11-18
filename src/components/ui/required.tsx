"use client"

import { cn } from "@/lib/utils"

/**
 * Decorative star for a field that needs a supplied value (UX-1). The glyph
 * is drawn by CSS `::before` on an empty, `aria-hidden` span. `aria-hidden`
 * alone would keep a literal `*` out of a browser's accessible name; the CSS
 * is used because testing-library's `getByLabelText` matches on `textContent`,
 * which would include a text-node star and break the many existing exact
 * label queries. The star is therefore visible without changing label text.
 * Required semantics belong on the control (native `required` or
 * `aria-required`). Markers are opt-in - callers pass `required` to
 * `Label`/`FieldLabel` (or render this for a custom control) - rather than
 * inferred from children, so an optional field can never acquire a star by
 * accident.
 */
export function RequiredMarker({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn("text-destructive leading-none before:content-['*']", className)}
    />
  )
}
