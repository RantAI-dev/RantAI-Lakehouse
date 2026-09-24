import { parseTimestamp } from "@/lib/format"

/**
 * A chat message's markdown as one plain line, for a list preview: code
 * blocks, emphasis, headings, list markers and link targets stripped.
 */
export function plainPreview(markdown: string, max = 160): string {
  const text = markdown
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/`([^`]*)`/g, "$1")
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/^\s{0,3}(#{1,6}|>|[-*+]|\d+\.)\s+/gm, "")
    .replace(/(\*\*|__|\*|_|~~)/g, "")
    .replace(/\s+/g, " ")
    .trim()
  return text.length > max ? `${text.slice(0, max - 1).trimEnd()}…` : text
}

export const SESSION_GROUPS = [
  "Today",
  "Yesterday",
  "Previous 7 days",
  "Previous 30 days",
  "Earlier",
] as const

export type SessionGroup = (typeof SESSION_GROUPS)[number]

/** Which recency bucket a timestamp falls in, by local calendar day. */
export function sessionGroup(updatedAt: string | undefined, now = new Date()): SessionGroup {
  if (!updatedAt) return "Earlier"
  const t = parseTimestamp(updatedAt)
  if (Number.isNaN(t.getTime())) return "Earlier"
  const startOf = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()
  const days = Math.round((startOf(now) - startOf(t)) / 86_400_000)
  if (days <= 0) return "Today"
  if (days === 1) return "Yesterday"
  if (days < 7) return "Previous 7 days"
  if (days < 30) return "Previous 30 days"
  return "Earlier"
}

/** Items bucketed by recency, buckets in `SESSION_GROUPS` order, empty ones dropped. */
export function groupSessions<T extends { updatedAt?: string }>(
  items: T[],
  now = new Date()
): { group: SessionGroup; items: T[] }[] {
  const byGroup = new Map<SessionGroup, T[]>()
  for (const item of items) {
    const g = sessionGroup(item.updatedAt, now)
    byGroup.set(g, [...(byGroup.get(g) ?? []), item])
  }
  return SESSION_GROUPS.filter((g) => byGroup.has(g)).map((group) => ({
    group,
    items: byGroup.get(group) ?? [],
  }))
}
