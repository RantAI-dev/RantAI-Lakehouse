import { parseTimestamp } from "@/lib/format"
import type { FolderLike } from "@/lib/folder-tree"
import type { ItemKind, TrashItem } from "@/services/contracts/dashboards"

/**
 * Helpers for the trash, the personal space, favourites and search (BI-18
 * part C, part 2). Pure: no fetch, no clock (the caller passes `now`).
 */

/** The label of a folder in lists: the caller's own personal space is just "Personal space". */
export function folderLabel(folder: FolderLike, meId: string | undefined): string {
  if (folder.ownerId && folder.ownerId === meId) return "Personal space"
  return folder.name
}

/** The folders with the caller's own personal space renamed for display; others unchanged. */
export function labelledFolders<F extends FolderLike>(folders: F[], meId: string | undefined): F[] {
  return folders.map((f) => (f.ownerId ? { ...f, name: folderLabel(f, meId) } : f))
}

/**
 * The folders a dashboard may be filed in by this caller: every ordinary folder
 * and the caller's own personal space. An administrator sees other people's
 * personal spaces in the list, but files nothing into them.
 */
export function folderChoices<F extends FolderLike>(folders: F[], meId: string | undefined): F[] {
  return folders.filter((f) => !f.ownerId || f.ownerId === meId)
}

/** The folders the "Folders" manager may change: a personal space cannot be renamed, moved or deleted. */
export function manageableFolders<F extends FolderLike>(folders: F[]): F[] {
  return folders.filter((f) => !f.ownerId)
}

/** Whole days left before a trash entry is removed for good; 0 once it is due. */
export function daysLeft(expiresAt: string, now: number): number {
  const t = parseTimestamp(expiresAt).getTime()
  if (Number.isNaN(t)) return 0
  return Math.max(0, Math.ceil((t - now) / 86_400_000))
}

/** "3 days left", "1 day left", "Due today". */
export function daysLeftLabel(expiresAt: string, now: number): string {
  const n = daysLeft(expiresAt, now)
  if (n <= 0) return "Due today"
  return `${n} ${n === 1 ? "day" : "days"} left`
}

/**
 * What restoring a chart will do, said once for the row: it goes back to its
 * dashboard, comes back with the dashboard when that is in the trash too, or
 * is left on no dashboard when the dashboard is gone.
 */
export function restoreNote(item: Pick<TrashItem, "kind" | "boardState" | "boardName">): string {
  if (item.kind === "board") return "With its charts"
  switch (item.boardState) {
    case "live":
      return item.boardName ? `Back on ${item.boardName}` : "Back on its dashboard"
    case "trash":
      return "Comes back with its dashboard"
    default:
      return "Its dashboard is gone: on no dashboard"
  }
}

/** Where opening a favourite, a recent item or a search hit goes. A chart opens its dashboard. */
export function itemHref(item: { kind: ItemKind; id: string; board?: string }): string {
  if (item.kind === "board") return `/dashboards/${encodeURIComponent(item.id)}`
  return `/dashboards/${encodeURIComponent(item.board || "default")}`
}
