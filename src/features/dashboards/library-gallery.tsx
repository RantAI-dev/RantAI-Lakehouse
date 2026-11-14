"use client"

/**
 * The Gallery view of the Dashboards page (`BI-59` T4, decision P2): the same
 * rows as the table, as cards topped by the picture of each item, grouped by
 * folder (mockup B's cards). Folders fold; templates, which are never filed,
 * have a section of their own at the end.
 */
import * as React from "react"
import { ChevronDown, Folder as FolderIcon, LayoutTemplate, Lock } from "lucide-react"

import { Button } from "@/components/ui/button"
import { formatRelativeTime } from "@/lib/format"
import { groupBoardsByFolder, type FolderLike } from "@/lib/folder-tree"
import { cn } from "@/lib/utils"
import { countLabel, factOf, type LibraryItem } from "./library-items"
import { ItemMenu, type ItemActions } from "./library-menus"
import { Avatar, EASE, FOCUS, ItemBadges, StarButton, ownerLabel } from "./library-parts"
import { ItemPicture, chartsOf, type PictureSource } from "./library-picture"
import { ItemName } from "./library-table"

type Section = { id: string; title: string; icon: typeof FolderIcon | null; items: LibraryItem[] }

/**
 * The sections in their order: no folder first, then each folder in tree
 * order (an empty one only while nothing narrows the list, `showEmpty`), then
 * the templates.
 */
export function gallerySections(items: readonly LibraryItem[], folders: FolderLike[], showEmpty: boolean): Section[] {
  const filed = items.filter((i) => i.kind !== "template")
  const templates = items.filter((i) => i.kind === "template")
  const out: Section[] = groupBoardsByFolder(folders, filed, showEmpty).map((s) => ({
    id: s.folderId || "none",
    title: s.path || "No folder",
    icon: s.folderId ? (folders.find((f) => f.id === s.folderId)?.ownerId ? Lock : FolderIcon) : null,
    items: s.boards,
  }))
  if (templates.length > 0) out.push({ id: "templates", title: "Templates", icon: LayoutTemplate, items: templates })
  return out
}

export function LibraryGallery({
  items, folders, showEmpty, actions, myName, pictures, onOpenTemplate, emptyFolder,
}: {
  readonly items: readonly LibraryItem[]
  readonly folders: FolderLike[]
  /** Show folders that hold nothing (only while no search or filter narrows the list). */
  readonly showEmpty: boolean
  readonly actions: ItemActions
  readonly myName: string | undefined
  readonly pictures: PictureSource
  readonly onOpenTemplate: (item: LibraryItem) => void
  /** What an empty folder's section says. */
  readonly emptyFolder: (folderId: string) => React.ReactNode
}) {
  const [collapsed, setCollapsed] = React.useState<ReadonlySet<string>>(new Set())
  const sections = gallerySections(items, folders, showEmpty)
  const toggle = (id: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (!next.delete(id)) next.add(id)
      return next
    })
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-8">
      {sections.map((s) => {
        const open = !collapsed.has(s.id)
        const Icon = s.icon
        return (
          <section key={s.id} aria-label={s.title} className="grid grid-cols-[minmax(0,1fr)] gap-3">
            <button
              type="button"
              aria-expanded={open}
              onClick={() => toggle(s.id)}
              className={cn("-ml-1.5 flex w-fit items-center gap-2 rounded-md px-1.5 py-0.5 text-sm font-medium hover:bg-muted/60", EASE, FOCUS)}
            >
              <ChevronDown aria-hidden className={cn("size-4 text-muted-foreground", EASE, !open && "-rotate-90")} />
              {Icon ? <Icon aria-hidden className="size-4 text-muted-foreground" /> : null}
              {s.title}
              <span className="text-xs font-normal text-muted-foreground tabular-nums">{s.items.length.toLocaleString("en-US")}</span>
            </button>
            {open ? (
              s.items.length === 0 ? (
                emptyFolder(s.id)
              ) : (
                <ul className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">
                  {s.items.map((item) => (
                    <Card key={item.key} item={item} actions={actions} myName={myName} pictures={pictures} onOpenTemplate={onOpenTemplate} />
                  ))}
                </ul>
              )
            ) : null}
          </section>
        )
      })}
    </div>
  )
}

/** The card: the picture on top (badges and the star over it), then the name and ⋯, then who, how big and when, then one fact. */
function Card({ item, actions, myName, pictures, onOpenTemplate }: {
  readonly item: LibraryItem
  readonly actions: ItemActions
  readonly myName: string | undefined
  readonly pictures: PictureSource
  readonly onOpenTemplate: (item: LibraryItem) => void
}) {
  const fav = item.kind === "dashboard" && actions.isFavourite(item)
  const fact = factOf(item)
  const owner = ownerLabel(item, myName)
  const when = item.builtin || !item.updatedAt ? null : `${item.kind === "template" ? "Saved" : "Updated"} ${formatRelativeTime(item.updatedAt)}`
  return (
    <li
      className={cn(
        "group relative flex flex-col rounded-2xl bg-card p-2 shadow-[0_1px_2px_rgba(0,0,0,0.04)] ring-1 ring-foreground/5 hover:-translate-y-0.5 hover:shadow-lg hover:ring-foreground/10",
        EASE,
      )}
    >
      <div className="relative">
        <ItemPicture item={item} charts={chartsOf(pictures, item.id)} size="lg" className={cn("h-36", EASE, "group-hover:bg-muted dark:group-hover:bg-muted/60")} />
        <ItemBadges item={item} max={2} className="absolute bottom-2 left-2 [&>span]:bg-card/95 [&>span]:shadow-xs dark:[&>span]:bg-card" />
        {item.kind === "dashboard" ? (
          <StarButton
            on={fav}
            name={item.name}
            onToggle={() => actions.onToggleFavourite(item)}
            className={cn(
              "absolute top-1.5 right-1.5 bg-card/90 shadow-xs hover:bg-card dark:bg-card",
              !fav && "opacity-0 group-hover:opacity-100 focus-visible:opacity-100 [@media(hover:none)]:opacity-100",
            )}
          />
        ) : null}
      </div>
      <div className="grid gap-1 px-2 pt-3 pb-1.5">
        <div className="flex items-center gap-2">
          <ItemName item={item} onOpenTemplate={onOpenTemplate} className="flex-1 text-sm after:rounded-2xl" />
          <ItemMenu item={item} actions={actions} className="-my-1 -mr-1.5" />
        </div>
        <p className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
          <Avatar owner={item.owner} builtin={item.builtin} className="size-4 text-[8px]" />
          <span className="truncate">{[owner, countLabel(item), when].filter(Boolean).join(" · ")}</span>
        </p>
        {fact ? <p className="line-clamp-1 text-xs text-muted-foreground">{fact}</p> : null}
        {item.kind === "template" && actions.onUseTemplate ? (
          <Button variant="outline" size="sm" className="relative z-10 mt-2 justify-self-start" onClick={() => actions.onUseTemplate?.(item)}>
            Use template
          </Button>
        ) : null}
      </div>
    </li>
  )
}
