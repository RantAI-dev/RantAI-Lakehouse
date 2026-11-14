"use client"

/**
 * The Table view of the Dashboards page (`BI-59` T2): one table of
 * dashboards, reports and templates with the columns name, type, folder,
 * owner, updated and sharing, every one of them sortable, and the ⋯ menu at
 * the end of every row.
 */
import * as React from "react"
import Link from "next/link"
import { ArrowDown, ArrowUp, Folder as FolderIcon, Lock } from "lucide-react"

import { formatRelativeTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import { KIND_LABEL, libraryHref, type LibraryItem, type SortColumn, type SortState } from "./library-items"
import { ItemMenu, type ItemActions } from "./library-menus"
import { Avatar, EASE, FOCUS, ItemBadges, KindIcon, StarButton, ownerLabel } from "./library-parts"

/** What the table needs to know about folders: a path for a folder id, and whether it is a personal space. */
export type FolderLookup = {
  readonly pathOf: (folderId: string) => string
  readonly isPersonal: (folderId: string) => boolean
}

type RowProps = {
  readonly item: LibraryItem
  readonly actions: ItemActions
  readonly folders: FolderLookup
  readonly myName: string | undefined
  /** Opening a template asks for a name first; the page owns that dialog. */
  readonly onOpenTemplate: (item: LibraryItem) => void
}

export function LibraryTable({
  items, sort, onSort, actions, folders, myName, onOpenTemplate,
}: {
  readonly items: readonly LibraryItem[]
  readonly sort: SortState
  readonly onSort: (column: SortColumn) => void
  readonly actions: ItemActions
  readonly folders: FolderLookup
  readonly myName: string | undefined
  readonly onOpenTemplate: (item: LibraryItem) => void
}) {
  return (
    <div className="rounded-2xl bg-card ring-1 ring-foreground/5">
      <table className="w-full table-fixed text-sm">
        <thead>
          <tr className="text-left text-xs text-muted-foreground">
            <Th column="name" sort={sort} onSort={onSort} className="w-[30%] pl-4">Name</Th>
            <Th column="kind" sort={sort} onSort={onSort} className="w-28">Type</Th>
            <Th column="folder" sort={sort} onSort={onSort} className="hidden w-40 xl:table-cell">Folder</Th>
            <Th column="owner" sort={sort} onSort={onSort} className="w-36">Owner</Th>
            <Th column="updated" sort={sort} onSort={onSort} className="w-24">Updated</Th>
            <Th column="sharing" sort={sort} onSort={onSort}>Sharing</Th>
            <th className="w-20 py-2.5 pr-2"><span className="sr-only">Actions</span></th>
          </tr>
        </thead>
        <tbody>
          {items.map((item) => (
            <TableRow key={item.key} item={item} actions={actions} folders={folders} myName={myName} onOpenTemplate={onOpenTemplate} />
          ))}
        </tbody>
      </table>
    </div>
  )
}

function Th({ column, sort, onSort, className, children }: {
  readonly column: SortColumn
  readonly sort: SortState
  readonly onSort: (column: SortColumn) => void
  readonly className?: string
  readonly children: React.ReactNode
}) {
  const on = sort.column === column
  const Arrow = on && sort.dir === "asc" ? ArrowUp : ArrowDown
  return (
    <th className={cn("py-2.5 pr-3 font-normal", className)} aria-sort={on ? (sort.dir === "asc" ? "ascending" : "descending") : "none"}>
      <button
        type="button"
        onClick={() => onSort(column)}
        className={cn("-ml-1.5 inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 hover:bg-muted hover:text-foreground", on && "text-foreground", EASE, FOCUS)}
      >
        {children}
        <Arrow aria-hidden className={cn("size-3", EASE, on ? "opacity-100" : "opacity-0")} />
      </button>
    </th>
  )
}

/** The item's name as the row's one link (stretched over the row); a template's opens the Use template dialog. */
export function ItemName({ item, onOpenTemplate, className }: {
  readonly item: LibraryItem
  readonly onOpenTemplate: (item: LibraryItem) => void
  readonly className?: string
}) {
  const cls = cn(
    "min-w-0 truncate text-left font-medium outline-none after:absolute after:inset-0 focus-visible:after:ring-2 focus-visible:after:ring-inset focus-visible:after:ring-ring/50",
    className,
  )
  if (item.kind === "template") {
    return (
      <button type="button" className={cls} onClick={() => onOpenTemplate(item)}>
        {item.name}
      </button>
    )
  }
  return (
    <Link href={libraryHref(item)} className={cls}>
      {item.name}
    </Link>
  )
}

/** A folder's path with its glyph: a lock for a personal space, a folder otherwise; a dash for none. */
export function FolderCell({ item, folders }: { readonly item: LibraryItem; readonly folders: FolderLookup }) {
  const path = item.folderId ? folders.pathOf(item.folderId) : ""
  if (!path) return <span aria-label="No folder" className="text-muted-foreground/60">—</span>
  const Glyph = folders.isPersonal(item.folderId) ? Lock : FolderIcon
  return (
    <span className="inline-flex min-w-0 max-w-full items-center gap-1.5">
      <Glyph aria-hidden className="size-3.5 shrink-0" />
      <span className="truncate">{path}</span>
    </span>
  )
}

function TableRow({ item, actions, folders, myName, onOpenTemplate }: RowProps) {
  const fav = item.kind === "dashboard" && actions.isFavourite(item)
  return (
    <tr className={cn("group relative border-t border-border/60 hover:bg-muted/40", EASE)}>
      <td className="py-2 pr-3 pl-4">
        <div className="flex min-w-0 items-center gap-3">
          <span aria-hidden className="grid h-8 w-11 shrink-0 place-items-center rounded-md bg-muted/70 dark:bg-muted/40">
            <KindIcon kind={item.kind} />
          </span>
          <ItemName item={item} onOpenTemplate={onOpenTemplate} />
        </div>
      </td>
      <td className="py-2 pr-3 text-muted-foreground">
        <span className="inline-flex items-center gap-1.5">
          <KindIcon kind={item.kind} className="size-3.5" /> {KIND_LABEL[item.kind]}
        </span>
      </td>
      <td className="hidden py-2 pr-3 text-muted-foreground xl:table-cell">
        <FolderCell item={item} folders={folders} />
      </td>
      <td className="py-2 pr-3">
        <span className="inline-flex min-w-0 max-w-full items-center gap-2 text-muted-foreground">
          <Avatar owner={item.owner} builtin={item.builtin} />
          <span className="truncate">{ownerLabel(item, myName)}</span>
        </span>
      </td>
      <td className="py-2 pr-3 text-muted-foreground tabular-nums">
        {item.updatedAt ? formatRelativeTime(item.updatedAt) : "—"}
      </td>
      <td className="py-2 pr-3"><ItemBadges item={item} /></td>
      <td className="py-2 pr-2">
        <div className="flex items-center justify-end">
          {item.kind === "dashboard" ? (
            <StarButton
              on={fav}
              name={item.name}
              onToggle={() => actions.onToggleFavourite(item)}
              className={cn(!fav && "opacity-0 group-hover:opacity-100 focus-visible:opacity-100 [@media(hover:none)]:opacity-100")}
            />
          ) : null}
          <ItemMenu item={item} actions={actions} />
        </div>
      </td>
    </tr>
  )
}
