"use client"

/**
 * The loading placeholders of the Dashboards page (`/dashboards/browse`,
 * `BI-59`), shaped like the page itself: the header row (title, then the
 * search, the Table / Gallery switch, ⋯ and New, every control 32 px), the
 * Recent cards, the type filter row, and the table rows or gallery cards.
 *
 * One component for both loading phases (owner QA 2026-11-14, "kenapa kita
 * punya dua skeleton page?"):
 * - while `AuthProvider` checks the session, `AppShellSkeleton` draws
 *   `DashboardListPageSkeleton` inside the shell's frame;
 * - once the page mounts, it draws its real header and filter row and the
 *   same `DashboardListRecentSkeleton` and `DashboardListRowsSkeleton`
 *   below them until its reads land.
 * So the handoff between the two is real controls replacing grey shapes in
 * the same places, never a second, different placeholder.
 *
 * Nothing here depends on the person's permissions (the tick column and the
 * Templates choice do): the session is not known in the first phase, and a
 * shape that changed between the phases is the jump this file exists to
 * remove. The Table / Gallery choice is per browser, not per person, so both
 * phases read it from the same key.
 */
import * as React from "react"

import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"

export type DashboardListView = "table" | "gallery"

/**
 * Where the Table / Gallery choice is kept (decision P2 of `BI-59`: Table is
 * the default, the choice is kept per browser).
 */
export const DASHBOARDS_VIEW_STORAGE_KEY = "dashboards:browse-view"

/** The stored choice; Table when there is none or storage is blocked (a private window). */
export function readStoredListView(): DashboardListView {
  try {
    return window.localStorage.getItem(DASHBOARDS_VIEW_STORAGE_KEY) === "gallery" ? "gallery" : "table"
  } catch {
    return "table"
  }
}

function subscribeToStorage(onChange: () => void): () => void {
  window.addEventListener("storage", onChange)
  return () => window.removeEventListener("storage", onChange)
}

/**
 * The stored choice, read during render on the client and as Table on the
 * server, so a server-rendered shell hydrates without a mismatch and then
 * shows the right placeholder before the first paint after hydration.
 */
export function useStoredListView(): DashboardListView {
  return React.useSyncExternalStore(subscribeToStorage, readStoredListView, () => "table")
}

/** The card and table surface of the page (`library-table.tsx`, `library-gallery.tsx`). */
const SURFACE = "rounded-2xl bg-card ring-1 ring-foreground/5"

/** Name widths that vary like real names do, so the rows do not read as a pattern. */
const NAME_WIDTHS = ["58%", "44%", "66%", "38%", "52%", "47%", "61%", "35%"]

/**
 * The four Recent cards (`library-recent.tsx`): a 96 px picture over the name
 * and one line of what it is.
 */
export function DashboardListRecentSkeleton() {
  return (
    <section aria-hidden className="grid gap-3">
      <h2 className="text-sm font-medium">Recent</h2>
      <ul className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        {[0, 1, 2, 3].map((i) => (
          <li key={i} className={cn("grid min-w-0 gap-2 p-2", SURFACE)}>
            <Skeleton className="h-24 rounded-lg" />
            <div className="grid min-w-0 gap-0.5 px-1.5 pb-1">
              <span className="flex h-5 items-center"><Skeleton className="h-3.5" style={{ width: NAME_WIDTHS[i + 2] }} /></span>
              <span className="flex h-4 items-center"><Skeleton className="h-3 w-24" /></span>
            </div>
          </li>
        ))}
      </ul>
    </section>
  )
}

/**
 * The list while it loads: table rows as tall as the real ones (a 32 px
 * picture in 8 px padding: 48 px), stacked rows on a phone as the real
 * table does below 768 px, or the gallery's cards.
 */
export function DashboardListRowsSkeleton({ view, rows = 8 }: { readonly view: DashboardListView; readonly rows?: number }) {
  if (view === "gallery") return <GallerySkeleton />
  const list = Array.from({ length: rows }, (_, i) => i)
  return (
    <div role="status" aria-label="Loading dashboards" className={SURFACE}>
      {/* A phone: the stacked rows of `PhoneRow`. */}
      <ul aria-hidden className="divide-y divide-border/60 md:hidden">
        {list.map((i) => (
          <li key={i} className="flex items-center gap-3 px-3 py-2.5">
            <Skeleton className="h-9 w-12 shrink-0 rounded-md" />
            <div className="grid min-w-0 flex-1 gap-0.5">
              <span className="flex h-5 items-center"><Skeleton className="h-3.5" style={{ width: NAME_WIDTHS[i % NAME_WIDTHS.length] }} /></span>
              <span className="flex h-4 items-center"><Skeleton className="h-3 w-3/4" /></span>
            </div>
          </li>
        ))}
      </ul>
      {/* A laptop: the table, with the real table's column widths. */}
      <table aria-hidden className="hidden w-full table-fixed text-sm md:table">
        <thead>
          <tr className="h-12">
            <th className="w-[30%] py-2.5 pr-3 pl-4"><Skeleton className="h-3 w-12" /></th>
            <th className="w-28 py-2.5 pr-3"><Skeleton className="h-3 w-10" /></th>
            <th className="hidden w-40 py-2.5 pr-3 xl:table-cell"><Skeleton className="h-3 w-12" /></th>
            <th className="w-36 py-2.5 pr-3"><Skeleton className="h-3 w-12" /></th>
            <th className="w-24 py-2.5 pr-3"><Skeleton className="h-3 w-14" /></th>
            <th className="py-2.5 pr-3"><Skeleton className="h-3 w-14" /></th>
            <th className="w-20 py-2.5 pr-2" />
          </tr>
        </thead>
        <tbody>
          {list.map((i) => (
            <tr key={i} className="border-t border-border/60">
              <td className="py-2 pr-3 pl-4">
                <div className="flex min-w-0 items-center gap-3">
                  <Skeleton className="h-8 w-11 shrink-0 rounded-md" />
                  <Skeleton className="h-3.5" style={{ width: NAME_WIDTHS[i % NAME_WIDTHS.length] }} />
                </div>
              </td>
              <td className="py-2 pr-3"><Skeleton className="h-3 w-20" /></td>
              <td className="hidden py-2 pr-3 xl:table-cell"><Skeleton className="h-3 w-24" /></td>
              <td className="py-2 pr-3">
                <div className="flex items-center gap-2">
                  <Skeleton className="size-5 shrink-0 rounded-full" />
                  <Skeleton className="h-3 w-16" />
                </div>
              </td>
              <td className="py-2 pr-3"><Skeleton className="h-3 w-14" /></td>
              <td className="py-2 pr-3" />
              <td className="py-2 pr-2" />
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

/** One folder section of the gallery (`library-gallery.tsx`): its header, then cards with a 144 px picture. */
function GallerySkeleton() {
  return (
    <div role="status" aria-label="Loading dashboards" className="grid grid-cols-[minmax(0,1fr)] gap-3">
      <div aria-hidden className="flex h-6 items-center gap-2">
        <Skeleton className="size-4" />
        <Skeleton className="h-3.5 w-24" />
      </div>
      <ul aria-hidden className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">
        {[0, 1, 2, 3, 4, 5].map((i) => (
          <li key={i} className={cn("flex flex-col p-2 shadow-[0_1px_2px_rgba(0,0,0,0.04)]", SURFACE)}>
            <Skeleton className="h-36 rounded-lg" />
            <div className="grid gap-1 px-2 pt-3 pb-1.5">
              <span className="flex h-5 items-center"><Skeleton className="h-3.5" style={{ width: NAME_WIDTHS[i] }} /></span>
              <span className="flex h-4 items-center gap-1.5">
                <Skeleton className="size-4 shrink-0 rounded-full" />
                <Skeleton className="h-3 w-32" />
              </span>
              {/* The card's one fact (its description, or what it holds); a row of cards takes the tallest. */}
              <span className="flex h-4 items-center"><Skeleton className="h-3 w-3/5" /></span>
            </div>
          </li>
        ))}
      </ul>
    </div>
  )
}

/**
 * The whole page while the session is checked, inside the shell's padded
 * `main`. The title is the page's own fixed name; every control in the header
 * and the filter row is a grey block of its size, because which of them a
 * person gets (New, the Templates choice) depends on permissions not known
 * yet. The frame mirrors `PageFrame` and `ScrollRegion`, whose negative
 * margins and paddings cancel out on screen.
 */
export function DashboardListPageSkeleton() {
  const view = useStoredListView()
  return (
    <div className="flex min-h-0 flex-col">
      <header className="flex shrink-0 items-center gap-2 max-md:flex-wrap max-md:gap-y-3">
        <h1 className="mr-2 shrink-0 text-xl leading-8 font-semibold tracking-tight max-md:w-full">Dashboards</h1>
        <div aria-hidden className="flex-1 max-md:hidden" />
        {/* The search (a 32 px square until opened), the view switch, ⋯ and New. */}
        <Skeleton aria-hidden className="size-8 rounded-lg" />
        <Skeleton aria-hidden className="h-8 w-[60px] rounded-lg max-md:hidden" />
        <Skeleton aria-hidden className="size-8 rounded-lg" />
        <Skeleton aria-hidden className="h-8 w-8 rounded-lg sm:w-[74px]" />
      </header>
      {/* `ScrollRegion` keeps a scrollbar's width on the right from `lg`; so does this, or the cards
          and rows would be that much wider here than on the page a moment later. */}
      <div className="mt-3 pt-1 pb-6 lg:overflow-y-auto lg:[scrollbar-gutter:stable]">
        <div className="grid grid-cols-[minmax(0,1fr)] gap-7">
          <DashboardListRecentSkeleton />
          <section className="grid grid-cols-[minmax(0,1fr)] gap-3">
            <div aria-hidden className="flex min-h-9 flex-wrap items-center justify-between gap-2">
              {/* The type filter (a 36 px segmented group) and the folder picker (a 28 px button). */}
              <Skeleton className="h-9 w-full rounded-lg md:w-[19rem]" />
              <Skeleton className="h-7 w-28 rounded-lg" />
            </div>
            <DashboardListRowsSkeleton view={view} />
          </section>
        </div>
      </div>
    </div>
  )
}
