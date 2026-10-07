import type { CSSProperties } from "react"
import { cn } from "@/lib/utils"

/**
 * A product mark of one or more paths on a view box (24 x 24 by default). The
 * SVG's default `xMidYMid meet` centres a non-square box without stretching.
 * Data and licence: `@/lib/connectors/brand-marks`. Decoration beside a name
 * that is always
 * shown, so it is hidden from assistive technology.
 *
 * One colour only. `tone="brand"` draws it in `hex` (or, with
 * `foregroundInDark`, in the foreground colour in the dark theme, for a
 * brand colour too dark to see there); `tone="inherit"` takes the parent's
 * text colour, for a chip that already sets one (the selected state).
 */
export function BrandMark({
  paths,
  viewBox = "0 0 24 24",
  hex,
  foregroundInDark = false,
  tone = "brand",
  className,
}: {
  paths: readonly string[]
  viewBox?: string
  hex: string
  foregroundInDark?: boolean
  tone?: "brand" | "inherit"
  className?: string
}) {
  const brand = tone === "brand"
  return (
    <svg
      viewBox={viewBox}
      fill="currentColor"
      aria-hidden="true"
      focusable="false"
      data-brand-mark=""
      data-tone={tone}
      className={cn(brand && "text-(--brand-mark)", brand && foregroundInDark && "dark:text-foreground", className)}
      style={brand ? ({ "--brand-mark": hex } as CSSProperties) : undefined}
    >
      {paths.map((d) => (
        <path key={d} d={d} />
      ))}
    </svg>
  )
}
