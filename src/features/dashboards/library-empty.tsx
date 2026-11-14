"use client"

/**
 * The designed empty states of the Dashboards page (`BI-59` T5): each says
 * what is missing, why, and the next step, in words that fit the filter. They
 * replace the generic inbox state and the old cards' "No description.".
 */
import type * as React from "react"
import Link from "next/link"
import { FileText, FolderInput, LayoutDashboard, LayoutTemplate, Lock, Plus, Search, type LucideIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import type { KindFilter } from "./library-items"
import { EASE, FOCUS, useHasDock } from "./library-parts"

/** An icon in a soft square, a title, a sentence, and an optional next step. */
export function LibraryEmpty({ icon: Icon, title, body, action, compact, className }: {
  readonly icon: LucideIcon
  readonly title: string
  readonly body: React.ReactNode
  readonly action?: React.ReactNode
  /** The smaller version, inside a folder's section of the gallery. */
  readonly compact?: boolean
  readonly className?: string
}) {
  return (
    <div
      className={cn(
        "grid justify-items-center gap-3 rounded-2xl bg-muted/40 text-center",
        compact ? "px-6 py-6 sm:max-w-md sm:justify-items-start sm:text-left" : "px-6 py-14",
        className,
      )}
    >
      <span className="grid size-10 place-items-center rounded-xl bg-background text-muted-foreground shadow-xs dark:bg-input/30">
        <Icon className="size-5" aria-hidden />
      </span>
      <div className={cn("grid max-w-sm gap-1", compact && "sm:justify-items-start")}>
        <p className="text-sm font-medium text-foreground">{title}</p>
        <p className="text-sm text-muted-foreground">{body}</p>
      </div>
      {action}
    </div>
  )
}

/** A folder with nothing in it: who sees what is filed there, and how to file something. */
export function EmptyFolder({ path, personal, compact, onShowAll }: {
  readonly path: string
  readonly personal: boolean
  readonly compact?: boolean
  /** Offered when the folder picker chose this folder. */
  readonly onShowAll?: () => void
}) {
  return (
    <LibraryEmpty
      compact={compact}
      icon={personal ? Lock : FolderInput}
      title={`Nothing in ${path || "this folder"} yet`}
      body={
        personal
          ? "Only you and administrators see what you file here. Move a dashboard in from its ⋯ menu."
          : "Move a dashboard or a report in from its ⋯ menu, or tick several rows and choose Move to folder."
      }
      action={onShowAll ? <Button variant="outline" size="sm" onClick={onShowAll}>Show all folders</Button> : undefined}
    />
  )
}

/**
 * Nothing matches the search: say what was searched. No "Clear search"
 * button here: the search box's ✕ right above does that, and DESIGN.md
 * §3 "Satu aksi, satu tombol" (owner, 2026-11-14) asks for one control
 * per action.
 */
export function NoMatch({ query }: { readonly query: string }) {
  return (
    <LibraryEmpty
      icon={Search}
      title={`Nothing matches “${query}”`}
      body="Search looks at names, owners, folders and types. Clear it with ✕ in the search box, or press Esc."
    />
  )
}

/** No row of one kind at all. A reader is told reports are shared with them; a writer how to make one. */
export function NoneOfKind({ kind, canWrite }: { readonly kind: KindFilter; readonly canWrite: boolean }) {
  if (kind === "template") {
    return (
      <LibraryEmpty
        icon={LayoutTemplate}
        title="No templates yet"
        body="Open a dashboard and choose Save as template in its ⋯ menu. Its layout, charts and filters are saved, without data."
      />
    )
  }
  if (kind === "report") {
    return (
      <LibraryEmpty
        icon={FileText}
        title="No reports yet"
        body={
          canWrite
            ? "A report is a written page with live charts between the paragraphs. Pick the charts from your dashboards."
            : "Nobody has shared a report with you yet."
        }
        action={
          canWrite ? (
            <Button variant="outline" size="sm" render={<Link href="/dashboards/reports/new" />}>
              <Plus data-icon="inline-start" /> New report
            </Button>
          ) : undefined
        }
      />
    )
  }
  return <LibraryEmpty icon={LayoutDashboard} title="No dashboards yet" body="A dashboard collects charts, KPIs and tables on one canvas." />
}

/**
 * The workspace holds nothing but the built-in Main: ways to start, above the
 * list. A reader is told what will show up here instead. The assistant is
 * named only where its bar is on the page.
 */
export function EmptyWorkspace({ canWrite, onNewDashboard }: { readonly canWrite: boolean; readonly onNewDashboard: () => void }) {
  const dock = useHasDock()
  if (!canWrite) {
    return (
      <LibraryEmpty
        icon={LayoutDashboard}
        title="Nothing shared with you yet"
        body="Dashboards and reports people share in this workspace show up here. The built-in Main is below."
      />
    )
  }
  const card = cn(
    "grid h-full w-full justify-items-start gap-2 rounded-xl bg-card p-4 text-left shadow-xs hover:-translate-y-0.5 hover:shadow-md active:translate-y-0",
    EASE, FOCUS,
  )
  return (
    <div className="grid justify-items-center gap-6 rounded-2xl bg-muted/40 px-6 py-10 text-center">
      <div className="grid max-w-md gap-1">
        <p className="text-base font-semibold text-foreground">Nothing made here yet</p>
        <p className="text-sm text-muted-foreground">Dashboards, reports and templates made in this workspace show up here. Start with one of these.</p>
      </div>
      <ul className="grid w-full max-w-xl gap-3 sm:grid-cols-2">
        <li>
          <button type="button" onClick={onNewDashboard} className={card}>
            <LayoutDashboard className="size-5 text-primary dark:text-sky-300" aria-hidden />
            <span className="text-sm font-medium">Blank dashboard</span>
            <span className="text-xs text-muted-foreground">Add charts one by one</span>
          </button>
        </li>
        <li>
          <Link href="/dashboards/reports/new" className={card}>
            <FileText className="size-5 text-primary dark:text-sky-300" aria-hidden />
            <span className="text-sm font-medium">Report</span>
            <span className="text-xs text-muted-foreground">Words with live charts</span>
          </Link>
        </li>
      </ul>
      {dock ? (
        <p className="max-w-md text-xs text-muted-foreground">
          Or ask the assistant below, for example “Build a dashboard of hotel occupancy”.
        </p>
      ) : null}
    </div>
  )
}
