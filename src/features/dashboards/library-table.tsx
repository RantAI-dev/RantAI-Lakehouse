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

import { Checkbox } from "@/components/ui/checkbox"
import { formatRelativeTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import { KIND_LABEL, countLabel, libraryHref, type LibraryItem, type SortColumn, type SortState } from "./library-items"
import { pickable } from "./library-bulk"
import { ItemMenu, type ItemActions } from "./library-menus"
import { Avatar, EASE, FOCUS, ItemBadges, KindIcon, StarButton, ownerLabel } from "./library-parts"
import { ItemPicture, chartsOf, type PictureSource } from "./library-picture"

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
  readonly pictures: PictureSource
  /** Ticking rows for the bulk actions; absent without `dashboard:write`. */
  readonly selection?: Selection
}

/** The ticked rows, by key, and how to change them. */
export type Selection = {
  readonly selected: ReadonlySet<string>
  readonly onToggle: (item: LibraryItem) => void
  /** Tick every pickable row shown, or untick them all. */
  readonly onToggleAll: (items: readonly LibraryItem[], on: boolean) => void
}

export function LibraryTable({
  items, sort, onSort, actions, folders, myName, onOpenTemplate, pictures, selection, phone,
}: {
  readonly items: readonly LibraryItem[]
  readonly sort: SortState
  readonly onSort: (column: SortColumn) => void
  readonly actions: ItemActions
  readonly folders: FolderLookup
  readonly myName: string | undefined
  readonly onOpenTemplate: (item: LibraryItem) => void
  readonly pictures: PictureSource
  readonly selection?: Selection
  /**
   * Below 768 px the rows are stacked instead of a table (T6). Chosen by the
   * page from the viewport, not by CSS alone, so the rows are not in the
   * page twice for a screen reader.
   */
  readonly phone?: boolean
}) {
  const choosable = items.filter(pickable)
  const allOn = choosable.length > 0 && choosable.every((i) => selection?.selected.has(i.key))
  const someOn = !allOn && choosable.some((i) => selection?.selected.has(i.key))
  return (
    <div className="rounded-2xl bg-card ring-1 ring-foreground/5">
      {phone ? (
        <ul className="divide-y divide-border/60" aria-label="Dashboards, reports and templates">
          {items.map((item) => (
            <PhoneRow key={item.key} item={item} actions={actions} folders={folders} myName={myName} onOpenTemplate={onOpenTemplate} pictures={pictures} selection={selection} />
          ))}
        </ul>
      ) : (
        <table className="w-full table-fixed text-sm">
          <thead>
            <tr className="text-left text-xs text-muted-foreground">
              {selection ? (
                <th className="w-11 py-2.5 pl-4 font-normal">
                  {choosable.length > 0 ? (
                    <Checkbox
                      aria-label="Select all"
                      checked={allOn ? true : someOn ? "indeterminate" : false}
                      onCheckedChange={() => selection.onToggleAll(choosable, !allOn)}
                    />
                  ) : null}
                </th>
              ) : null}
              <Th column="name" sort={sort} onSort={onSort} className={cn("w-[30%]", !selection && "pl-4")}>Name</Th>
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
              <TableRow key={item.key} item={item} actions={actions} folders={folders} myName={myName} onOpenTemplate={onOpenTemplate} pictures={pictures} selection={selection} />
            ))}
          </tbody>
        </table>
      )}
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

function TableRow({ item, actions, folders, myName, onOpenTemplate, pictures, selection }: RowProps) {
  const fav = item.kind === "dashboard" && actions.isFavourite(item)
  const ticked = selection?.selected.has(item.key) ?? false
  return (
    <tr className={cn("group relative border-t border-border/60", ticked ? "bg-primary/6 dark:bg-primary/10" : "hover:bg-muted/40", EASE)}>
      {selection ? (
        <td className="py-2 pl-4">
          {pickable(item) ? (
            <Checkbox aria-label={`Select ${item.name}`} checked={ticked} onCheckedChange={() => selection.onToggle(item)} className="relative z-10" />
          ) : null}
        </td>
      ) : null}
      <td className={cn("py-2 pr-3", !selection && "pl-4")}>
        <div className="flex min-w-0 items-center gap-3">
          <ItemPicture item={item} charts={chartsOf(pictures, item.id)} size="sm" className="h-8 w-11 shrink-0 rounded-md" />
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

/**
 * A row on a phone: the tick, the picture, the name over one line of what the
 * table's columns say (type, size, folder, when), then the star and the ⋯.
 * Nothing scrolls sideways.
 */
function PhoneRow({ item, actions, folders, myName, onOpenTemplate, pictures, selection }: RowProps) {
  const fav = item.kind === "dashboard" && actions.isFavourite(item)
  const ticked = selection?.selected.has(item.key) ?? false
  const folder = item.folderId ? folders.pathOf(item.folderId) : ""
  const owner = ownerLabel(item, myName)
  const facts = [
    KIND_LABEL[item.kind],
    countLabel(item),
    folder || null,
    item.builtin ? null : owner === "—" ? null : owner,
    item.updatedAt ? formatRelativeTime(item.updatedAt) : null,
  ].filter(Boolean)
  return (
    <li className={cn("relative flex items-center gap-3 px-3 py-2.5", ticked && "bg-primary/6 dark:bg-primary/10", EASE)}>
      {selection ? (
        pickable(item) ? (
          <Checkbox aria-label={`Select ${item.name}`} checked={ticked} onCheckedChange={() => selection.onToggle(item)} className="relative z-10" />
        ) : (
          <span aria-hidden className="size-4 shrink-0" />
        )
      ) : null}
      <ItemPicture item={item} charts={chartsOf(pictures, item.id)} size="sm" className="h-9 w-12 shrink-0 rounded-md" />
      <div className="grid min-w-0 flex-1 gap-0.5">
        <ItemName item={item} onOpenTemplate={onOpenTemplate} className="block max-w-full text-sm" />
        <p className="truncate text-xs text-muted-foreground">{facts.join(" · ")}</p>
        <ItemBadges item={item} max={2} />
      </div>
      {item.kind === "dashboard" ? <StarButton on={fav} name={item.name} onToggle={() => actions.onToggleFavourite(item)} /> : null}
      <ItemMenu item={item} actions={actions} />
    </li>
  )
}
