"use client"

import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

/** Stands for "no limit" among a toggle's options; shown as "All". */
export const ALL = Number.POSITIVE_INFINITY

/**
 * How many to show: a labelled row of buttons, the same ones as the list
 * page's Layer filter. Labelled because bare numbers in a thin border were
 * not read as a control.
 */
export function CountToggle({
  label,
  ariaLabel,
  options,
  value,
  onChange,
}: {
  /** The visible word before the numbers, e.g. "Rows". */
  label: string
  ariaLabel: string
  options: readonly number[]
  value: number
  onChange: (next: number) => void
}) {
  return (
    <div className="flex items-center gap-1" role="group" aria-label={ariaLabel}>
      <span className="mr-1 text-xs text-muted-foreground">{label}</span>
      {options.map((n) => (
        <Button
          key={n}
          type="button"
          size="sm"
          variant={value === n ? "secondary" : "ghost"}
          aria-pressed={value === n}
          onClick={() => onChange(n)}
          className={cn("tabular-nums", value === n && "font-semibold")}
        >
          {n === ALL ? "All" : n}
        </Button>
      ))}
    </div>
  )
}
