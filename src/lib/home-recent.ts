import { parseTimestamp } from "@/lib/format"

/**
 * Home's single "Recent" list: dashboards, conversations and saved queries
 * in one list by time, instead of a card for each. One list reads as one
 * thing to scan; three cards with two-line rows read as a wall.
 */
export type RecentKind = "dashboard" | "conversation" | "query"

export type RecentSource = {
  id: string
  title: string
  href: string
  /** When it was last changed; missing or unparseable sorts last. */
  at?: string | null
}

export type RecentItem = RecentSource & {
  kind: RecentKind
  /** The dashboard this browser had open last. */
  lastOpened: boolean
}

function ms(iso: string | null | undefined): number {
  if (!iso) return 0
  const t = parseTimestamp(iso).getTime()
  return Number.isNaN(t) ? 0 : t
}

/**
 * Merge the three sources, newest first. The dashboard last opened in this
 * browser leads regardless of its timestamp: "where I left off" is about
 * what was open, and a board's time only says when it was last written.
 * Each kind is capped (`perKind`) so one busy kind cannot crowd out the
 * other two.
 */
export function recentItems(
  sources: Record<RecentKind, RecentSource[]>,
  lastBoardId: string | null,
  limit = 7,
  perKind = 3,
): RecentItem[] {
  const take = (kind: RecentKind): RecentItem[] =>
    [...sources[kind]]
      .map((s) => ({ ...s, kind, lastOpened: kind === "dashboard" && s.id === lastBoardId }))
      .sort((a, b) => Number(b.lastOpened) - Number(a.lastOpened) || ms(b.at) - ms(a.at))
      .slice(0, perKind)

  return [...take("dashboard"), ...take("conversation"), ...take("query")]
    .sort((a, b) => Number(b.lastOpened) - Number(a.lastOpened) || ms(b.at) - ms(a.at))
    .slice(0, limit)
}
