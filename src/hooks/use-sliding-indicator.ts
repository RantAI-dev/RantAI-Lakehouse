"use client"

import * as React from "react"

const useIsoLayoutEffect = typeof window === "undefined" ? React.useEffect : React.useLayoutEffect

/**
 * One indicator that travels to the chosen item of a row (a tab's underline,
 * a segmented switch's pill) instead of one indicator per item that blinks
 * off on the old item and on at the new one (micro-interaction audit
 * 2026-11-14, emil: motion that says where the choice went).
 *
 * The row is the list (`listRef`, positioned: `relative`); the item that is
 * chosen carries `data-indicator-active`; the indicator (`indicatorRef`) is an
 * absolutely placed element inside the list at its top-left corner. The hook
 * writes the indicator's `transform` and `width` (and `height` with
 * `box: true`) straight onto it, so a change never re-renders the row, and
 * follows a change of the row's or the items' size.
 *
 * The first placement does not slide (the indicator would fly in from the
 * corner): `data-ready` is set on the indicator one frame later, and the
 * caller's transition classes key off it (`data-ready:transition-…`).
 * Nothing chosen hides the indicator. Under reduced motion the global rule in
 * `globals.css` makes the slide instant.
 */
export function useSlidingIndicator<L extends HTMLElement = HTMLDivElement, I extends HTMLElement = HTMLSpanElement>(
  activeKey: unknown,
  { box = false }: { readonly box?: boolean } = {},
): {
  readonly listRef: React.RefObject<L | null>
  readonly indicatorRef: React.RefObject<I | null>
} {
  const listRef = React.useRef<L | null>(null)
  const indicatorRef = React.useRef<I | null>(null)

  useIsoLayoutEffect(() => {
    const list = listRef.current
    const indicator = indicatorRef.current
    if (!list || !indicator) return
    const place = () => {
      const active = list.querySelector<HTMLElement>("[data-indicator-active]")
      if (!active) {
        indicator.style.opacity = "0"
        return
      }
      indicator.style.opacity = ""
      indicator.style.transform = box
        ? `translate(${active.offsetLeft}px, ${active.offsetTop}px)`
        : `translateX(${active.offsetLeft}px)`
      indicator.style.width = `${active.offsetWidth}px`
      if (box) indicator.style.height = `${active.offsetHeight}px`
    }
    place()
    const raf = typeof requestAnimationFrame === "undefined"
      ? null
      : requestAnimationFrame(() => indicator.setAttribute("data-ready", ""))
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(place)
    if (resize) {
      resize.observe(list)
      for (const child of Array.from(list.querySelectorAll("[data-indicator-item]"))) resize.observe(child)
    }
    return () => {
      if (raf !== null) cancelAnimationFrame(raf)
      resize?.disconnect()
    }
  }, [activeKey, box])

  return { listRef, indicatorRef }
}
