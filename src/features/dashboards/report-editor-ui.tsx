"use client"

/**
 * Small pieces the report editor's parts share (`BI-62`): the one easing and
 * focus ring (DESIGN.md §3: 150 to 250 ms, never linear, a visible focus), and
 * a tooltip for icon-only buttons (a labelled button gets none).
 */
import * as React from "react"

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"

export const EASE = "transition-[background-color,border-color,color,box-shadow,opacity,transform] duration-200 ease-ui"
export const FOCUS = "outline-none focus-visible:ring-3 focus-visible:ring-ring/50"

export function IconTip({
  label,
  children,
  side,
}: {
  readonly label: string
  readonly children: React.ReactElement
  readonly side?: "top" | "bottom" | "left" | "right"
}) {
  return (
    <Tooltip>
      <TooltipTrigger render={children} />
      <TooltipContent side={side}>{label}</TooltipContent>
    </Tooltip>
  )
}

/** Whether a picture of the element is on screen (lazy thumbnails); true where the browser cannot tell. */
export function useOnScreen<T extends Element>(): [React.RefCallback<T>, boolean] {
  const [seen, setSeen] = React.useState(false)
  const [el, setEl] = React.useState<T | null>(null)
  React.useEffect(() => {
    if (!el || seen) return
    if (typeof IntersectionObserver === "undefined") {
      setSeen(true)
      return
    }
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) setSeen(true)
    }, { rootMargin: "200px" })
    io.observe(el)
    return () => io.disconnect()
  }, [el, seen])
  return [setEl, seen]
}
