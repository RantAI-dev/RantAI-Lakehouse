"use client"

import Link from "next/link"

import { itemHref } from "@/lib/dashboard-organise"
import { cn } from "@/lib/utils"
import type { ShortcutItem } from "@/services"
import { countLabel, type LibraryItem } from "./library-items"
import { EASE } from "./library-parts"
import { ChartPicture, ItemPicture, chartsOf, type PictureSource } from "./library-picture"

/** At most this many recent items: one row of small cards on a laptop. */
export const RECENT_MAX = 4

/**
 * "Recent" (`BI-59` T3): the four items the person opened last, as small
 * picture cards, from the recent list the page already reads
 * (`GET /api/dashboard/shortcuts`, newest first). It replaces the Favourites
 * and Recent chip rows. A recent dashboard that is no longer in the list (in
 * the trash, or no longer visible) is left out; a recent chart opens its
 * dashboard.
 */
export function RecentRow({ recent, boards, source }: {
  readonly recent: readonly ShortcutItem[]
  /** The dashboards of the list, by id, so a card shows what the list shows. */
  readonly boards: ReadonlyMap<string, LibraryItem>
  readonly source: PictureSource
}) {
  const cards = recent
    .filter((r) => (r.kind === "board" ? boards.has(r.id) : true))
    .slice(0, RECENT_MAX)
  if (cards.length === 0) return null
  return (
    <section aria-labelledby="dashboards-recent" className="grid gap-3">
      <h2 id="dashboards-recent" className="text-sm font-medium">Recent</h2>
      <ul className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        {cards.map((r) => {
          const board = r.kind === "board" ? boards.get(r.id) : undefined
          const what = board ? `Dashboard · ${countLabel(board)}` : `Chart${r.boardName ? ` on ${r.boardName}` : ""}`
          return (
            <li
              key={`${r.kind}:${r.id}`}
              className={cn("group relative grid min-w-0 gap-2 rounded-2xl bg-card p-2 ring-1 ring-foreground/5 hover:-translate-y-0.5 hover:shadow-md hover:ring-foreground/10", EASE)}
            >
              {board ? (
                <ItemPicture item={board} charts={chartsOf(source, board.id)} size="md" className="h-24" />
              ) : (
                <ChartPicture chart={source.byId?.get(r.id) ?? null} size="md" className="h-24" />
              )}
              <div className="grid min-w-0 gap-0.5 px-1.5 pb-1">
                <Link
                  href={itemHref(r)}
                  className="truncate text-sm font-medium outline-none after:absolute after:inset-0 after:rounded-2xl focus-visible:after:ring-2 focus-visible:after:ring-ring/50"
                >
                  {board?.name ?? r.name}
                </Link>
                <p className="truncate text-xs text-muted-foreground">{what}</p>
              </div>
            </li>
          )
        })}
      </ul>
    </section>
  )
}
