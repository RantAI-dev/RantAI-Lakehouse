import { parseTimestamp } from "@/lib/format"

/**
 * Which dashboard Home draws in its focal widget (`WEB-17` D1). Pure, so the
 * order can be tested without a page.
 *
 * Only a board with charts can be drawn, so every rule below skips the ones
 * without. In order:
 * 1. the board chosen in Customize (`previewBoardId`), while it exists;
 * 2. the person's first favourite dashboard (`/api/dashboard/shortcuts`),
 *    when nothing was chosen or the chosen one cannot be drawn;
 * 3. the board last opened in this browser (`last-board.ts`);
 * 4. the most recently updated.
 *
 * A chosen board that was deleted or emptied falls through to the rest
 * rather than leaving the widget empty, as Home always did.
 */
export type FocalBoardLike = {
  id: string
  name: string
  chartCount?: number
  updatedAt?: string
}

/** Why this board is the one shown: a few honest words for the widget's meta line. */
export type FocalReason = "chosen" | "favourite" | "last-opened" | "recent"

export type FocalPick<T extends FocalBoardLike> = { board: T; reason: FocalReason }

function time(iso: string | undefined): number {
  if (!iso) return 0
  const t = parseTimestamp(iso).getTime()
  return Number.isNaN(t) ? 0 : t
}

const drawable = (b: FocalBoardLike) => (b.chartCount ?? 0) > 0

export function focalBoard<T extends FocalBoardLike>(input: {
  boards: readonly T[]
  chosenId: string | null
  /** Favourite dashboard ids, in the order the person's favourites list has them. */
  favouriteIds: readonly string[]
  lastOpenedId: string | null
}): FocalPick<T> | null {
  const { boards, chosenId, favouriteIds, lastOpenedId } = input
  const byId = (id: string | null) => (id ? boards.find((b) => b.id === id && drawable(b)) : undefined)

  const chosen = byId(chosenId)
  if (chosen) return { board: chosen, reason: "chosen" }
  for (const id of favouriteIds) {
    const fav = byId(id)
    if (fav) return { board: fav, reason: "favourite" }
  }
  const last = byId(lastOpenedId)
  if (last) return { board: last, reason: "last-opened" }
  const recent = [...boards].filter(drawable).sort((a, b) => time(b.updatedAt) - time(a.updatedAt))[0]
  return recent ? { board: recent, reason: "recent" } : null
}

export const FOCAL_REASON_LABEL: Record<FocalReason, string> = {
  chosen: "Chosen for Home",
  favourite: "Favourite",
  "last-opened": "Last opened",
  recent: "Recently updated",
}
