"use client"

/**
 * The two menus of the Dashboards page header (`BI-59` T1): the one primary
 * button, "New", and the page's ⋯ menu for everything that is neither finding
 * nor making something (metrics, folders, the trash). They replace the
 * toolbar row of eleven controls the owner found busy at QA on 2026-11-14.
 */
import * as React from "react"
import Link from "next/link"
import {
  Check, ChevronDown, Copy, ExternalLink, Folder as FolderIcon, Lock, FileText, FolderCog, FolderInput, LayoutDashboard, LayoutTemplate, Link2, MoreHorizontal, Pencil, Plus,
  Sigma, Star, Trash2, Upload,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuGroupLabel,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { notifyFailure, notifySuccess } from "@/lib/notify"
import { cn } from "@/lib/utils"
import type { DashboardTemplate } from "@/services"
import { libraryHref, type FolderFilter, type LibraryItem } from "./library-items"

/**
 * "New": a dashboard, a report, a dashboard from a template, or a dashboard
 * file. Templates are listed with their picture (owner taste: a choice of
 * visual things is shown, not listed as words); `templates` is `null` while
 * they load or when they could not be read, and the group then says so
 * rather than claiming there are none.
 */
export function NewMenu({
  templates,
  renderPicture,
  onDashboard,
  onTemplate,
  onImport,
}: {
  readonly templates: readonly DashboardTemplate[] | null
  /** The small picture of a template (T3); absent, a neutral glyph. */
  readonly renderPicture?: (template: DashboardTemplate) => React.ReactNode
  readonly onDashboard: () => void
  readonly onTemplate: (template: DashboardTemplate) => void
  readonly onImport: () => void
}) {
  return (
    <DropdownMenu>
      {/* BI-59 review round 2, SHOULD-FIX 3: 36 px like the search beside it (the console's h-9); on
          a phone the word is for screen readers only and the button is the plus alone. */}
      <DropdownMenuTrigger render={<Button size="lg" className="text-white max-sm:w-9 max-sm:px-0! sm:px-3" />}>
        <Plus data-icon="inline-start" className="max-sm:mx-auto" /> <span className="max-sm:sr-only">New</span>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-72">
        <DropdownMenuItem className="items-start py-2" onClick={onDashboard}>
          <LayoutDashboard className="mt-0.5" />
          <span className="grid gap-0.5">
            <span className="font-medium">Dashboard</span>
            <span className="text-xs text-muted-foreground">A canvas of charts, KPIs and tables</span>
          </span>
        </DropdownMenuItem>
        <DropdownMenuItem className="items-start py-2" render={<Link href="/dashboards/reports/new" />}>
          <FileText className="mt-0.5" />
          <span className="grid gap-0.5">
            <span className="font-medium">Report</span>
            <span className="text-xs text-muted-foreground">A written page with live charts</span>
          </span>
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuGroupLabel>From a template</DropdownMenuGroupLabel>
          {templates === null ? (
            <p className="px-2 pb-2 text-xs text-muted-foreground">Templates could not be listed right now.</p>
          ) : templates.length === 0 ? (
            <p className="px-2 pb-2 text-xs text-muted-foreground">No templates yet. Save a dashboard as a template from its ⋯ menu.</p>
          ) : (
            templates.slice(0, 5).map((t) => (
              <DropdownMenuItem key={t.id} className="gap-3 py-2" onClick={() => onTemplate(t)}>
                {renderPicture ? (
                  renderPicture(t)
                ) : (
                  <span aria-hidden className="grid h-10 w-16 shrink-0 place-items-center rounded-md bg-muted/70 text-muted-foreground dark:bg-muted/40">
                    <LayoutTemplate className="size-4" />
                  </span>
                )}
                <span className="grid min-w-0 gap-0.5">
                  <span className="truncate font-medium">{t.name}</span>
                  <span className="truncate text-xs text-muted-foreground">
                    {t.chartCount.toLocaleString("en-US")} {t.chartCount === 1 ? "chart" : "charts"}
                    {t.sourceBoardName ? ` · from ${t.sourceBoardName}` : ""}
                  </span>
                </span>
              </DropdownMenuItem>
            ))
          )}
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={onImport}>
          <Upload /> Import a dashboard file…
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/**
 * Everything that is not "find" or "new", behind one ⋯. Folders and the trash
 * are for a holder of `dashboard:write`, as before (BI-18 part C review fix
 * (SHOULD-FIX) R18: the trash is for those who can restore), so a reader's
 * menu holds Metrics alone.
 */
export function PageMenu({
  onFolders,
  onTrash,
}: {
  /** Absent without `dashboard:write`. */
  readonly onFolders?: () => void
  /** Absent without `dashboard:write`. */
  readonly onTrash?: () => void
}) {
  return (
    <DropdownMenu>
      {/* BI-59 review round 2, SHOULD-FIX 3: 36 px and filled like the search and the view switch
          beside it, no border, so the row reads as one set rather than a default outline button. */}
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="icon-lg"
            aria-label="More actions"
            className="bg-muted/70 text-muted-foreground hover:bg-muted hover:text-foreground aria-expanded:bg-muted dark:bg-input/30 dark:hover:bg-input/50"
          />
        }
      >
        <MoreHorizontal />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuItem render={<Link href="/dashboards/metrics" />}>
          <Sigma /> Metrics
        </DropdownMenuItem>
        {onFolders ? (
          <DropdownMenuItem onClick={onFolders}>
            <FolderCog /> Manage folders…
          </DropdownMenuItem>
        ) : null}
        {onTrash ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onClick={onTrash}>
              <Trash2 /> Trash
            </DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/**
 * What a row's ⋯ menu can do. The write actions are absent without
 * `dashboard:write` (BI-18 part C review fix (SHOULD-FIX) R27), and the menu
 * then offers only what the reader can do.
 */
export type ItemActions = {
  readonly isFavourite: (item: LibraryItem) => boolean
  /** Dashboards only: the favourites list holds dashboards and charts, not reports. */
  readonly onToggleFavourite: (item: LibraryItem) => void
  readonly onRename?: (item: LibraryItem) => void
  readonly onDuplicate?: (item: LibraryItem) => void
  readonly onMove?: (item: LibraryItem) => void
  readonly onTrash?: (item: LibraryItem) => void
  readonly onUseTemplate?: (item: LibraryItem) => void
  readonly onDeleteTemplate?: (item: LibraryItem) => void
}

/** The page's address of an item, for "Copy link". */
async function copyLink(item: LibraryItem) {
  try {
    await navigator.clipboard.writeText(new URL(libraryHref(item), window.location.origin).toString())
    notifySuccess("Link copied")
  } catch {
    notifyFailure("The link could not be copied", "The browser did not allow it. Open the item and copy the address instead.")
  }
}

/** What the built-in board's menu says instead of hiding the entries it cannot offer. */
export const BUILTIN_NOTE = "Built-in: it cannot be renamed, moved or deleted. Duplicate it for a copy you can change."

/**
 * The ⋯ menu of one row or card, in the same place for every item, the
 * built-in Main included (the old list gave Main none). Main's entries that
 * cannot apply are shown disabled, with the reason under them.
 */
export function ItemMenu({ item, actions, className }: {
  readonly item: LibraryItem
  readonly actions: ItemActions
  readonly className?: string
}) {
  const fav = actions.isFavourite(item)
  const href = libraryHref(item)
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={`Actions for ${item.name}`}
            className={cn("relative z-10 text-muted-foreground", className)}
          />
        }
      >
        <MoreHorizontal />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-60">
        {item.kind === "template" ? (
          <>
            {actions.onUseTemplate ? (
              <DropdownMenuItem onClick={() => actions.onUseTemplate?.(item)}>
                <Plus /> Use template…
              </DropdownMenuItem>
            ) : null}
            {actions.onDeleteTemplate ? (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem variant="destructive" onClick={() => actions.onDeleteTemplate?.(item)}>
                  <Trash2 /> Delete template…
                </DropdownMenuItem>
              </>
            ) : null}
          </>
        ) : (
          <>
            <DropdownMenuItem render={<Link href={href} />}>
              <ExternalLink /> Open
            </DropdownMenuItem>
            {item.kind === "dashboard" ? (
              <DropdownMenuItem onClick={() => actions.onToggleFavourite(item)}>
                <Star className={cn(fav && "fill-amber-400 text-amber-500")} />
                {fav ? "Remove from favourites" : "Add to favourites"}
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuItem onClick={() => void copyLink(item)}>
              <Link2 /> Copy link
            </DropdownMenuItem>
            {actions.onRename || actions.onDuplicate || actions.onMove || actions.onTrash ? <DropdownMenuSeparator /> : null}
            {item.kind === "report" && actions.onRename ? (
              <DropdownMenuItem render={<Link href={`${href}/edit`} />}>
                <Pencil /> Edit
              </DropdownMenuItem>
            ) : null}
            {item.kind === "dashboard" && actions.onRename ? (
              <DropdownMenuItem disabled={item.builtin} onClick={() => actions.onRename?.(item)}>
                <Pencil /> Rename…
              </DropdownMenuItem>
            ) : null}
            {item.kind === "dashboard" && actions.onDuplicate ? (
              <DropdownMenuItem onClick={() => actions.onDuplicate?.(item)}>
                <Copy /> Duplicate
              </DropdownMenuItem>
            ) : null}
            {actions.onMove ? (
              <DropdownMenuItem disabled={item.builtin} onClick={() => actions.onMove?.(item)}>
                <FolderInput /> Move to folder…
              </DropdownMenuItem>
            ) : null}
            {actions.onTrash ? (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem variant="destructive" disabled={item.builtin} onClick={() => actions.onTrash?.(item)}>
                  <Trash2 /> Move to trash
                </DropdownMenuItem>
              </>
            ) : null}
            {item.builtin && (actions.onRename || actions.onMove || actions.onTrash) ? (
              <p className="max-w-56 px-2 pt-1.5 pb-1 text-xs text-muted-foreground">{BUILTIN_NOTE}</p>
            ) : null}
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** One entry of the folder picker: its id, its path and whether it is a personal space. */
export type FolderChoice = { readonly id: string; readonly path: string; readonly personal: boolean; readonly count: number }

/**
 * The folder picker beside the type filter: every folder, no folder, or one
 * folder (with what it holds). Folders are listed in tree order, by path.
 */
export function FolderPicker({ value, choices, noFolderCount, onChange }: {
  readonly value: FolderFilter
  readonly choices: readonly FolderChoice[]
  readonly noFolderCount: number
  readonly onChange: (value: FolderFilter) => void
}) {
  const label = value === null ? "All folders" : value === "" ? "No folder" : (choices.find((c) => c.id === value)?.path ?? "All folders")
  const tick = (on: boolean) => (on ? <Check className="size-4 text-foreground" /> : <span aria-hidden className="size-4" />)
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="sm"
            aria-label={`Folder: ${label}`}
            className={cn("max-w-56 text-muted-foreground", value !== null && "bg-muted text-foreground")}
          />
        }
      >
        <FolderIcon data-icon="inline-start" />
        <span className="truncate">{label}</span>
        <ChevronDown data-icon="inline-end" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <DropdownMenuItem onClick={() => onChange(null)}>
          All folders <span className="ml-auto">{tick(value === null)}</span>
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={() => onChange("")}>
          No folder
          <span className="ml-auto flex items-center gap-2 text-xs text-muted-foreground tabular-nums">
            {noFolderCount.toLocaleString("en-US")} {tick(value === "")}
          </span>
        </DropdownMenuItem>
        {choices.map((c) => (
          <DropdownMenuItem key={c.id} onClick={() => onChange(c.id)}>
            {c.personal ? <Lock aria-hidden /> : <FolderIcon aria-hidden />}
            <span className="truncate">{c.path}</span>
            <span className="ml-auto flex items-center gap-2 text-xs text-muted-foreground tabular-nums">
              {c.count.toLocaleString("en-US")} {tick(value === c.id)}
            </span>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
