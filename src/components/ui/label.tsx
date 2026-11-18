"use client"

import * as React from "react"

import { cn } from "@/lib/utils"
import { RequiredMarker } from "@/components/ui/required"

/**
 * A field label with an opt-in required marker (UX-1). `required` is consumed
 * here rather than forwarded to the DOM: `<label>` has no native `required`
 * attribute, and the semantic state belongs on the control (native `required`
 * or `aria-required`). The star stays decorative and `aria-hidden`.
 */
function Label({
  className,
  required,
  children,
  ...props
}: React.ComponentProps<"label"> & { required?: boolean }) {
  return (
    <label
      data-slot="label"
      data-required={required ? "" : undefined}
      className={cn(
        "flex items-center gap-2 text-sm leading-none font-medium select-none group-data-[disabled=true]:pointer-events-none group-data-[disabled=true]:opacity-50 peer-disabled:cursor-not-allowed peer-disabled:opacity-50",
        className
      )}
      {...props}
    >
      {children}
      {required ? <RequiredMarker /> : null}
    </label>
  )
}

export { Label }
