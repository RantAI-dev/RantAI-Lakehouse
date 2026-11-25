"use client"

import * as React from "react"

/**
 * Marks which edges of a scroll area hide content, so the area fades there
 * and only there (`BI-61` review round 1, SHOULD-FIX 1; DESIGN.md §3, "an area
 * that scrolls fades at the edge where it is cut"): `data-fade-top` while
 * something is scrolled out above, `data-fade-bottom` while something is still
 * below. The `.scroll-fade` class in `globals.css` turns them into the mask.
 *
 * The attributes are written on the element directly, not through React
 * state, so scrolling never re-renders the page. They follow the scroll, a
 * change of the area's size, and a change of its children's size (tiles
 * loading, a tab switch).
 *
 * Returns a callback ref for the scrolling element.
 */
export function useScrollFade<T extends HTMLElement>(): (el: T | null) => void {
  const [el, setEl] = React.useState<T | null>(null)
  React.useEffect(() => {
    if (!el) return
    const update = () => {
      // A pixel of slack: a fractional scroll position never reaches the end exactly.
      const above = el.scrollTop > 1
      const below = el.scrollTop + el.clientHeight < el.scrollHeight - 1
      el.toggleAttribute("data-fade-top", above)
      el.toggleAttribute("data-fade-bottom", below)
    }
    update()
    el.addEventListener("scroll", update, { passive: true })
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update)
    const watchChildren = () => {
      if (!resize) return
      resize.disconnect()
      resize.observe(el)
      for (const child of Array.from(el.children)) resize.observe(child)
    }
    watchChildren()
    const children = typeof MutationObserver === "undefined" ? null : new MutationObserver(() => { watchChildren(); update() })
    children?.observe(el, { childList: true })
    return () => {
      el.removeEventListener("scroll", update)
      resize?.disconnect()
      children?.disconnect()
    }
  }, [el])
  return setEl
}
