"use client"

/**
 * The small pieces of the Dashboards page (`BI-59`, layout C of the
 * 2026-11-14 mockups): the page frame that ends above the assistant bar, the
 * large search, and (later tasks) the badges, the owner avatar, the segmented
 * type filter and the designed empty states. Kept apart from the page so the
 * table and the gallery draw them the same way.
 */
import * as React from "react"
import { Search, X } from "lucide-react"

import { useAuth } from "@/features/auth/auth-provider"
import { cn } from "@/lib/utils"

/** One easing and duration for every hover, press and change (owner taste: 150–250 ms, eased, never linear). */
export const EASE = "transition-[background-color,border-color,color,box-shadow,opacity,transform] duration-200 ease-[cubic-bezier(0.2,0,0,1)]"
/** The keyboard focus ring every custom control here uses. */
export const FOCUS = "outline-none focus-visible:ring-3 focus-visible:ring-ring/50"

/**
 * Whether the assistant bar is on this page. It is the same test the app
 * frame makes (`app-frame.tsx`): the bar shows for a holder of
 * `assistant:use` or `assistant:ask`, and `/dashboards/browse` is not one of
 * the paths that hide it.
 */
function useHasDock(): boolean {
  const { hasPermission } = useAuth()
  return hasPermission("assistant:use") || hasPermission("assistant:ask")
}

/**
 * The page as a frame that ends above the assistant bar, so the bar never
 * covers a row or a card (spec `BI-59` T5): the list scrolls inside the frame
 * instead of under the bar.
 *
 * With the bar, the height leaves the navbar (4rem), the shell's top padding
 * (1, 1.25 or 1.5rem) and 5rem for the bar, and the negative bottom margin
 * gives back the bottom padding the app frame adds for the bar (`pb-24` /
 * `lg:pb-28`), so the window itself does not scroll. Without the bar the
 * frame is the viewport less the navbar and the shell's own padding.
 *
 * Below `lg` the whole frame scrolls, header included, so a phone keeps the
 * room for the rows; from `lg` only the list scrolls (`ScrollRegion`).
 */
export function PageFrame({ children, className }: { readonly children: React.ReactNode; readonly className?: string }) {
  const dock = useHasDock()
  return (
    <div
      className={cn(
        "flex min-h-0 flex-col overflow-y-auto lg:overflow-hidden",
        dock
          ? "-mb-24 h-[calc(100dvh-10rem)] sm:h-[calc(100dvh-10.25rem)] lg:-mb-28 lg:h-[calc(100dvh-10.5rem)]"
          : "h-[calc(100dvh-6rem)] sm:h-[calc(100dvh-6.5rem)] lg:h-[calc(100dvh-7rem)]",
        className,
      )}
    >
      {children}
    </div>
  )
}

/** The part of the frame that scrolls on a laptop; its bottom edge fades instead of cutting a row in half. */
export function ScrollRegion({ children, className }: { readonly children: React.ReactNode; readonly className?: string }) {
  return (
    <div
      className={cn(
        "lg:min-h-0 lg:flex-1 lg:overflow-y-auto lg:[mask-image:linear-gradient(to_bottom,black_calc(100%-20px),transparent)] lg:[scrollbar-gutter:stable]",
        className,
      )}
    >
      {children}
    </div>
  )
}

/**
 * The page's search: large, with a clear button. "/" focuses it from
 * anywhere on the page (unless the person is typing somewhere else), and
 * Escape clears it.
 */
export function SearchField({
  value, onChange, placeholder = "Search everything", className,
}: {
  readonly value: string
  readonly onChange: (value: string) => void
  readonly placeholder?: string
  readonly className?: string
}) {
  const ref = React.useRef<HTMLInputElement>(null)
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "/" || e.metaKey || e.ctrlKey || e.altKey) return
      const target = e.target
      if (target instanceof Element && target.closest("input,textarea,select,[contenteditable=true],[role=dialog],[role=menu]")) return
      e.preventDefault()
      ref.current?.focus()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [])
  return (
    <div className={cn("relative", className)}>
      <Search aria-hidden className="pointer-events-none absolute top-1/2 left-3 size-4.5 -translate-y-1/2 text-muted-foreground" />
      <input
        ref={ref}
        type="search"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape" && value) {
            e.preventDefault()
            onChange("")
          }
        }}
        placeholder={placeholder}
        aria-label="Search dashboards, reports and templates"
        className={cn(
          "h-11 w-full min-w-0 rounded-lg bg-muted/70 pr-16 pl-9 text-[15px] placeholder:text-muted-foreground hover:bg-muted focus:bg-background focus:ring-1 focus:ring-ring/60 dark:bg-input/30 dark:focus:bg-input/40 [&::-webkit-search-cancel-button]:hidden",
          EASE,
          "outline-none focus-visible:ring-3 focus-visible:ring-ring/40",
        )}
      />
      {value ? (
        <button
          type="button"
          aria-label="Clear search"
          onClick={() => {
            onChange("")
            ref.current?.focus()
          }}
          className={cn("absolute top-1/2 right-2 grid size-7 -translate-y-1/2 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground", EASE, FOCUS)}
        >
          <X className="size-3.5" aria-hidden />
        </button>
      ) : (
        <kbd aria-hidden className="pointer-events-none absolute top-1/2 right-3 hidden h-5 -translate-y-1/2 items-center rounded bg-background px-1.5 text-[11px] text-muted-foreground shadow-xs sm:inline-flex dark:bg-input/40">
          /
        </kbd>
      )}
    </div>
  )
}
