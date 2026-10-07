"use client"

import * as React from "react"
import type { LucideIcon } from "lucide-react"
import { TabsList, TabsTrigger } from "@/components/ui/tabs"
import { cn } from "@/lib/utils"

export type TabCountTone = "danger" | "warning"

/** The number a tab carries, and whether it should read as trouble. */
export type LineTabCount = { count: number; tone?: TabCountTone }

export type LineTab = {
  value: string
  label: string
  icon: LucideIcon
  /** `null` or absent for a tab with nothing to count. */
  count?: LineTabCount | null
}

/**
 * How much is behind a tab, so the empty ones can be skipped without a
 * click. A zero is dimmed; a tone turns the count red or amber.
 */
export function TabCount({ count, tone }: { count: number; tone?: TabCountTone }) {
  return (
    <span
      className={cn(
        "min-w-4 rounded-full px-1 text-center text-[11px] leading-4 font-medium tabular-nums",
        tone === "danger"
          ? "bg-destructive/10 text-destructive"
          : tone === "warning"
            ? "bg-amber-500/15 text-amber-700 dark:text-amber-400"
            : count === 0
              ? "text-muted-foreground/50"
              : "bg-muted text-muted-foreground group-data-active/tab:bg-primary/10 group-data-active/tab:text-primary"
      )}
    >
      {count}
    </span>
  )
}

/**
 * The app navbar's height (`h-16` in `app-navbar.tsx`): the tab strip
 * sticks right under it.
 */
const STICKY_TOP_PX = 64

/**
 * After a tab switch, scrolled past the strip, the new tab would open
 * mid-page: start it at the top instead, just under the stuck strip. `root`
 * is the element that wraps the `Tabs` the strip belongs to.
 */
export function keepStripInView(root: HTMLElement | null) {
  const top = root?.getBoundingClientRect().top
  if (top !== undefined && top < STICKY_TOP_PX && window.scrollY > 0) {
    window.scrollTo({ top: window.scrollY + top - STICKY_TOP_PX })
  }
}

/**
 * The line-style tab strip of an entity's detail page (the asset page, the
 * connector page): an icon, a label and an optional count per tab. Render it
 * inside a `Tabs` whose `value` is `active`; the panels stay with the page.
 */
export function LineTabsList({ tabs, active }: { tabs: LineTab[]; active: string }) {
  const stripRef = React.useRef<HTMLDivElement>(null)

  // On a narrow screen the strip scrolls sideways; keep the open tab in
  // it, so a link to `?tab=activity` does not open a tab you cannot see.
  // Sideways only: the page itself never moves for this.
  React.useEffect(() => {
    const strip = stripRef.current
    const open = strip?.querySelector<HTMLElement>("[role=tab][data-active]")
    if (!strip || !open) return
    const s = strip.getBoundingClientRect()
    const t = open.getBoundingClientRect()
    if (t.left < s.left) strip.scrollLeft += t.left - s.left - 16
    else if (t.right > s.right) strip.scrollLeft += t.right - s.right + 16
  }, [active])

  return (
    // Sticky under the navbar, bled to the edges of `<main>`'s padding
    // (`app-frame.tsx`) so content scrolling underneath never shows
    // beside it. Two layers repeat the page's own background: the
    // inset's `bg-muted/25` over the body's `bg-background`.
    <div className="sticky top-16 z-10 -mx-4 bg-background sm:-mx-5 lg:-mx-6">
      <div
        ref={stripRef}
        className="overflow-x-auto border-b border-border bg-muted/25 px-4 [scrollbar-width:none] sm:px-5 lg:px-6"
      >
        <TabsList
          variant="line"
          className="w-max justify-start gap-0.5 p-0 group-data-horizontal/tabs:h-10"
        >
          {tabs.map(({ value, label, icon: Icon, count }) => (
            <TabsTrigger
              key={value}
              value={value}
              className="group/tab h-full flex-none px-2.5 after:bg-primary group-data-horizontal/tabs:after:bottom-0"
            >
              <Icon
                className="size-3.5 text-muted-foreground group-data-active/tab:text-primary"
                aria-hidden
              />
              {label}
              {count ? <TabCount count={count.count} tone={count.tone} /> : null}
            </TabsTrigger>
          ))}
        </TabsList>
      </div>
    </div>
  )
}
