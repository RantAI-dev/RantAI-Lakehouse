import { parseTimestamp } from "@/lib/format"
import { folderPath, type FolderLike } from "@/lib/folder-tree"
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

/** Said for a chart deleted on its own whose dashboard is in the trash too (BI-18 part C review fix R10); the server refuses that restore with the same words. */
export const DASHBOARD_IN_TRASH = "Its dashboard is in the trash too: restore the dashboard first."

/** Whether a trash entry cannot be restored yet. */
export function restoreBlocked(item: Pick<TrashItem, "kind" | "boardState">): boolean {
  return item.kind === "chart" && item.boardState === "trash"
}

/** The words after a revert about the charts it took out of the trash and the ones that could not come back (BI-18 part C review fix R7). */
export function revertNote(done: { chartsRemoved: number; chartsRestored: string[]; chartsGone: string[] }): string {
  const plural = (n: number) => (n === 1 ? "chart" : "charts")
  const parts: string[] = []
  if (done.chartsRestored.length) parts.push(`${done.chartsRestored.length} ${plural(done.chartsRestored.length)} came back from the trash.`)
  if (done.chartsRemoved) parts.push(`${done.chartsRemoved} ${plural(done.chartsRemoved)} added since went to the trash.`)
  if (done.chartsGone.length) {
    parts.push(`${done.chartsGone.length} ${plural(done.chartsGone.length)} could not come back: removed from the trash for good: ${done.chartsGone.join(", ")}.`)
  }
  return parts.join(" ")
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
      return DASHBOARD_IN_TRASH
    default:
      return "Its dashboard is gone: on no dashboard"
  }
}

/** Where opening a favourite, a recent item or a search hit goes. A chart opens its dashboard. */
export function itemHref(item: { kind: ItemKind; id: string; board?: string }): string {
  if (item.kind === "board") return `/dashboards/${encodeURIComponent(item.id)}`
  return `/dashboards/${encodeURIComponent(item.board || "default")}`
}

/** Whether `id` names a personal space among `folders` (the New dashboard dialog hints that it is private). */
export function isPersonalFolder(folders: readonly FolderLike[], id: string): boolean {
  return Boolean(folders.find((f) => f.id === id)?.ownerId)
}

/** What a folder picker calls the caller's own personal space (BI-18 part C review fix (SHOULD-FIX) R17). */
export const PERSONAL_OPTION_LABEL = "Personal space (only you)"

/**
 * The folders of a picker in the order it shows them: the personal space (and
 * what is filed under it) first, apart from the shared folders, which keep
 * their tree order. The caller adds the "no folder" entry between the two.
 */
export function splitFolderOptions<T extends { folder: FolderLike }>(flat: readonly T[], folders: readonly FolderLike[]): { personal: T[]; shared: T[] } {
  const inPersonal = (f: FolderLike): boolean => {
    const seen = new Set<string>()
    let at: FolderLike | undefined = f
    while (at && !seen.has(at.id)) {
      if (at.ownerId) return true
      seen.add(at.id)
      at = at.parentId ? folders.find((x) => x.id === at!.parentId) : undefined
    }
    return false
  }
  return { personal: flat.filter((o) => inPersonal(o.folder)), shared: flat.filter((o) => !inPersonal(o.folder)) }
}

/** The text a picker shows for the chosen value: the label of the personal space, the path of a folder, else `none`. */
export function folderOptionText(folders: FolderLike[], id: string, none: string): string {
  const folder = folders.find((f) => f.id === id)
  if (!folder) return none
  return folder.ownerId ? PERSONAL_OPTION_LABEL : folderPath(folders, id)
}
