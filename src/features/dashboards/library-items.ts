/**
 * The one list of the Dashboards page (`BI-59` T2): dashboards, reports and
 * templates as one kind of row, so a single table can filter, search and sort
 * them together. Pure: no fetch, no clock.
 *
 * Each row keeps the record it came from (`board`, `report`, `template`), so
 * an action reaches the service with the real object, never a copy rebuilt
 * from the row.
 */
import { parseTimestamp } from "@/lib/format"
import type { Board, DashboardTemplate, ReportSummary } from "@/services"

export type LibraryKind = "dashboard" | "report" | "template"

export const KIND_LABEL: Record<LibraryKind, string> = {
  dashboard: "Dashboard",
  report: "Report",
  template: "Template",
}

export type LibraryItem = {
  /** `kind:id`, unique across the three kinds. */
  readonly key: string
  readonly kind: LibraryKind
  readonly id: string
  readonly name: string
  /** The dashboard folder; `""` is no folder. A template is never filed. */
  readonly folderId: string
  /** Display name of whoever made it; `""` when the record has none. */
  readonly owner: string
  /** When it last changed (a template: when it was saved); `""` for the built-in board. */
  readonly updatedAt: string
  /** Tiles on a dashboard, charts in a report or a template. */
  readonly count: number
  readonly builtin: boolean
  readonly publicLink: boolean
  readonly embedded: boolean
  /** A dashboard in a personal space (it has an owner): only that person and administrators see it. */
  readonly personal: boolean
  readonly description: string
  /** A template's source dashboard, by the name it had when the template was saved. */
  readonly source: string
  readonly board?: Board
  readonly report?: ReportSummary
  readonly template?: DashboardTemplate
}

export function boardItem(b: Board): LibraryItem {
  return {
    key: `dashboard:${b.id}`,
    kind: "dashboard",
    id: b.id,
    name: b.name,
    folderId: b.folderId ?? "",
    owner: b.createdBy ?? "",
    updatedAt: b.builtin ? "" : (b.updatedAt ?? ""),
    count: b.chartCount ?? 0,
    builtin: Boolean(b.builtin),
    publicLink: Boolean(b.publicToken),
    embedded: Boolean(b.embedEnabled),
    personal: Boolean(b.ownerId),
    description: b.description?.trim() ?? "",
    source: "",
    board: b,
  }
}

export function reportItem(r: ReportSummary): LibraryItem {
  return {
    key: `report:${r.id}`,
    kind: "report",
    id: r.id,
    name: r.title,
    folderId: r.folderId,
    owner: r.createdBy,
    updatedAt: r.updatedAt,
    count: r.chartCount,
    builtin: false,
    publicLink: false,
    embedded: false,
    personal: false,
    description: "",
    source: "",
    report: r,
  }
}

export function templateItem(t: DashboardTemplate): LibraryItem {
  return {
    key: `template:${t.id}`,
    kind: "template",
    id: t.id,
    name: t.name,
    folderId: "",
    owner: t.createdBy,
    updatedAt: t.createdAt,
    count: t.chartCount,
    builtin: false,
    publicLink: false,
    embedded: false,
    personal: false,
    description: "",
    source: t.sourceBoardName,
    template: t,
  }
}

/** Where opening a row goes. A template has no page of its own: its row asks for a name (the page handles it). */
export function libraryHref(item: Pick<LibraryItem, "kind" | "id">): string {
  if (item.kind === "report") return `/dashboards/reports/${encodeURIComponent(item.id)}`
  return `/dashboards/${encodeURIComponent(item.id)}`
}

/** "13 tiles", "4 charts", "No tiles yet": the count in the item's own unit. */
export function countLabel(item: Pick<LibraryItem, "kind" | "count">): string {
  const unit = item.kind === "dashboard" ? "tile" : "chart"
  if (item.count === 0) return item.kind === "dashboard" ? "No tiles yet" : "No charts yet"
  return `${item.count.toLocaleString("en-US")} ${unit}${item.count === 1 ? "" : "s"}`
}

/**
 * One useful fact where the old card said "No description." (`BI-59` T5): the
 * description when there is one, a template's source dashboard, or nothing.
 *
 * Two facts the mockup showed are not offered because the list does not carry
 * them: a dashboard does not record the template it was made from, and a
 * report's summary carries no blocks, so its first sentence is not known here.
 * Reading every report to find it would cost one request per report.
 */
export function factOf(item: LibraryItem): string | null {
  if (item.description) return item.description
  if (item.kind === "template" && item.source) return `Saved from ${item.source}`
  return null
}

/** The sharing labels of a row, in the one order the badges use (P4). */
export function sharingLabels(item: LibraryItem): string[] {
  const out: string[] = []
  if (item.builtin) out.push("Built-in")
  if (item.publicLink) out.push("Public link")
  if (item.embedded) out.push("Embedded")
  if (item.personal) out.push("Private")
  return out
}

/** Whether `item` matches the search: name, owner, folder, kind, description and a template's source. */
export function matchesQuery(item: LibraryItem, query: string, folderPathOf: (folderId: string) => string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return [item.name, item.owner, folderPathOf(item.folderId), KIND_LABEL[item.kind], item.description, item.source]
    .some((field) => field.toLowerCase().includes(q))
}

export type KindFilter = "all" | LibraryKind
/** `null` is every folder, `""` no folder, else a folder id. */
export type FolderFilter = string | null

/**
 * The rows a filter keeps. A folder filter (no folder included) keeps only
 * what can be filed, so templates, which never are, drop out under it.
 */
export function filterItems(
  items: readonly LibraryItem[],
  filter: { kind: KindFilter; folder: FolderFilter; query: string },
  folderPathOf: (folderId: string) => string,
): LibraryItem[] {
  return items.filter((i) =>
    (filter.kind === "all" || i.kind === filter.kind)
    && (filter.folder === null || (i.kind !== "template" && i.folderId === filter.folder))
    && matchesQuery(i, filter.query, folderPathOf),
  )
}

export type SortColumn = "name" | "kind" | "folder" | "owner" | "updated" | "sharing"
export type SortState = { readonly column: SortColumn; readonly dir: "asc" | "desc" }

/** The direction a column sorts in when first chosen: newest first for Updated, A to Z for the rest. */
export function firstDirection(column: SortColumn): "asc" | "desc" {
  return column === "updated" ? "desc" : "asc"
}

function time(value: string): number {
  if (!value) return Number.NaN
  return parseTimestamp(value).getTime()
}

/**
 * Sorts by one column, the name breaking ties. An empty value (no folder, no
 * owner, no sharing, no update time) goes last whichever way the column
 * sorts, so "newest first" never opens on the built-in board's missing time.
 */
export function sortItems(
  items: readonly LibraryItem[],
  sort: SortState,
  folderPathOf: (folderId: string) => string,
): LibraryItem[] {
  const sign = sort.dir === "asc" ? 1 : -1
  const text = (i: LibraryItem): string => {
    switch (sort.column) {
      case "name": return i.name
      case "kind": return KIND_LABEL[i.kind]
      case "folder": return i.folderId ? folderPathOf(i.folderId) : ""
      case "owner": return i.builtin ? "" : i.owner
      case "sharing": return sharingLabels(i).join(" ")
      default: return ""
    }
  }
  const byName = (a: LibraryItem, b: LibraryItem) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" })
  return [...items].sort((a, b) => {
    if (sort.column === "updated") {
      const x = time(a.updatedAt)
      const y = time(b.updatedAt)
      const xa = Number.isNaN(x)
      const ya = Number.isNaN(y)
      if (xa !== ya) return xa ? 1 : -1
      if (!xa && x !== y) return sign * (x - y)
      return byName(a, b)
    }
    const x = text(a)
    const y = text(b)
    if (!x !== !y) return x ? -1 : 1
    const c = x.localeCompare(y, undefined, { sensitivity: "base", numeric: true })
    return c !== 0 ? sign * c : byName(a, b)
  })
}
