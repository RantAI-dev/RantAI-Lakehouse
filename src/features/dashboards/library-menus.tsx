"use client"

/**
 * The two menus of the Dashboards page header (`BI-59` T1): the one primary
 * button, "New", and the page's ⋯ menu for everything that is neither finding
 * nor making something (metrics, folders, the trash). They replace the
 * toolbar row of eleven controls the owner found busy at QA on 2026-11-14.
 */
import * as React from "react"
import Link from "next/link"
import { FileText, FolderCog, LayoutDashboard, LayoutTemplate, MoreHorizontal, Plus, Sigma, Trash2, Upload } from "lucide-react"

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
import type { DashboardTemplate } from "@/services"

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
      <DropdownMenuTrigger render={<Button className="text-white" />}>
        <Plus data-icon="inline-start" /> New
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
      <DropdownMenuTrigger render={<Button variant="outline" size="icon" aria-label="More actions" />}>
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
