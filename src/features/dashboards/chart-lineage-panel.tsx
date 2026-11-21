"use client"

import * as React from "react"
import Link from "next/link"
import {
  ArrowUpRight, ChartColumn, Circle, Clock, Database, FileCode, Landmark, Plug, SquareSigma, Table2, Upload, Workflow, type LucideIcon,
} from "lucide-react"
import { HelpHint } from "@/components/patterns/help-hint"
import { ErrorState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { Skeleton } from "@/components/ui/skeleton"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useAuth } from "@/features/auth/auth-provider"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { hasRefreshTime, LINEAGE_REFUSED, missingWords, refreshWords, STEP_KIND_LABEL } from "@/lib/lineage-words"
import { cn } from "@/lib/utils"
import { dashboardService } from "@/services"
import type { ChartLineage, FieldColumns, LineageStep } from "@/services/contracts/dashboards"
import { useReporting } from "./reporting-context"

/**
 * "Where does this come from" (`BI-14` feature decision 6, plan D7; look
 * `BI-71` decision 1, the owner's mockup A "Route"): a side panel with the
 * chart's path read top to bottom, from the source down to this chart, as one
 * line with a small kind icon per step. A step says its name and kind, and a
 * refresh time only where one is on record. The chart's fields sit as chips
 * on the last step, a recorded column written `field ← column`. What the path
 * leaves out is said once, in one quiet line under it, with the server's
 * `coverage` sentences in a help hint (owner QA of `BI-14`, 2026-11-20: five
 * bordered cards, each saying "Not recorded", read as "just cards"). A link
 * to the full lineage page focused on the chart is the one action.
 *
 * Needs `lineage:read` (owner decision Q6): the menu item is shown to everyone
 * who can see the chart, and a person without the permission gets the sentence
 * instead of the path; nothing is asked for.
 */
export function ChartLineagePanel({
  chart, onOpenChange,
}: {
  /** The chart whose path is shown; `null` keeps the panel closed. */
  readonly chart: { id: string; title: string } | null
  readonly onOpenChange: (open: boolean) => void
}) {
  const { hasPermission } = useAuth()
  const allowed = hasPermission("lineage:read")
  const id = chart?.id ?? ""
  const state = useService((s) => (allowed && id ? dashboardService.chartLineage(id, s) : Promise.resolve(null)), [allowed, id])
  const refused = !allowed || (state.status === "error" && state.error.code === "permission_denied")
  const lineage = !refused && state.status === "success" && state.data?.supported ? state.data : null

  return (
    <Sheet open={chart !== null} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="w-full gap-0 sm:max-w-md">
        <SheetHeader className="pb-4">
          <SheetTitle>Where does this come from</SheetTitle>
          <SheetDescription>{chart?.title}</SheetDescription>
        </SheetHeader>
        <div className="min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-6">
          {refused ? (
            <p role="status" className="text-sm text-muted-foreground">{LINEAGE_REFUSED}</p>
          ) : state.status === "loading" ? (
            <RouteSkeleton />
          ) : state.status === "error" ? (
            <ErrorState error={state.error} onRetry={state.reload} />
          ) : state.data && !state.data.supported ? (
            <p role="status" className="text-sm text-muted-foreground">Lineage is not recorded on this deployment.</p>
          ) : lineage ? (
            <Route lineage={lineage} />
          ) : null}
        </div>
        {lineage ? (
          <div className="shrink-0 px-6 pt-2 pb-5">
            <Button
              variant="outline"
              className="w-full sm:w-fit"
              render={<Link href={`/lineage?focus=${encodeURIComponent(`chart:${lineage.chart.id}`)}`} />}
            >
              Open in lineage <ArrowUpRight data-icon="inline-end" />
            </Button>
          </div>
        ) : null}
      </SheetContent>
    </Sheet>
  )
}

/** A step's kind as an icon; a kind this console does not know yet gets a plain dot, not a guess. */
export const KIND_ICON: Record<string, LucideIcon> = {
  chart: ChartColumn,
  sql_source: FileCode,
  metric: SquareSigma,
  gold: Table2,
  silver: Table2,
  bronze: Table2,
  connector: Plug,
  upload: Upload,
  pipeline: Workflow,
  source: Landmark,
  dataset: Database,
  iceberg: Table2,
}

/** The delay between two steps' entrances, so the route draws from the source down. */
const STAGGER_MS = 70

/**
 * The route itself. The API sends the steps chart first (`ChartLineage`);
 * they are read here source first, in the direction the data flows.
 */
function Route({ lineage }: { lineage: ChartLineage }) {
  const { timeZone } = useReporting()
  const steps = [...lineage.steps].reverse()
  const last = steps.length - 1
  const missing = missingWords(lineage)
  return (
    <div className="grid gap-5">
      <ol aria-label="Path from the source to this chart" className="grid">
        {steps.map((step, i) => {
          const here = step.kind === "chart"
          const delay = { animationDelay: `${i * STAGGER_MS}ms` }
          return (
            <li
              key={step.id}
              data-step-kind={step.kind}
              className={cn("relative grid grid-cols-[2rem_minmax(0,1fr)] gap-x-3", i < last && "pb-4")}
            >
              {i < last ? (
                <span
                  aria-hidden
                  className="absolute top-10 bottom-2 left-[calc(1rem-0.5px)] w-px origin-top animate-route-draw bg-border"
                  style={{ animationDelay: `${i * STAGGER_MS + 120}ms` }}
                />
              ) : null}
              <StepIcon kind={step.kind} style={delay} />
              <div className="min-w-0 animate-route-rise" style={delay}>
                <p className={cn("truncate text-sm leading-5", here ? "font-semibold" : "font-medium")} title={here ? undefined : step.label}>
                  {here ? "This chart" : step.label}
                </p>
                <p className="flex flex-wrap items-center gap-x-1.5 text-xs leading-5 text-muted-foreground">
                  {STEP_KIND_LABEL[step.kind] ?? step.kind}
                  {hasRefreshTime(step) ? (
                    <>
                      <span aria-hidden>·</span>
                      <RefreshTag step={step} words={refreshWords(step, timeZone)} />
                    </>
                  ) : null}
                </p>
                {here && lineage.fields.length > 0 ? (
                  <ul aria-label="Fields of this chart" className="mt-2 flex flex-wrap gap-1.5">
                    {lineage.fields.map((f) => <li key={f.field} className="max-w-full min-w-0"><FieldChip field={f} /></li>)}
                  </ul>
                ) : null}
              </div>
            </li>
          )
        })}
      </ol>
      {missing ? (
        <p className="flex items-center gap-1.5 text-xs leading-5 text-muted-foreground">
          {missing}
          {lineage.coverage.length > 0 ? <HelpHint>{lineage.coverage.join(" ")}</HelpHint> : null}
        </p>
      ) : null}
    </div>
  )
}

/** A step's kind as an icon in a soft circle; the chart, where the reader is, in the accent. */
function StepIcon({ kind, style }: { kind: string; style?: React.CSSProperties }) {
  const Icon = KIND_ICON[kind] ?? Circle
  return (
    <span
      aria-hidden
      style={style}
      className={cn(
        "inline-flex size-8 shrink-0 animate-route-pop items-center justify-center rounded-full",
        kind === "chart" ? "bg-primary/12 text-primary" : "bg-muted text-foreground/75",
      )}
    >
      <Icon className={KIND_ICON[kind] ? "size-4" : "size-2 fill-current"} />
    </span>
  )
}

const FOCUS = "outline-none focus-visible:ring-2 focus-visible:ring-ring"

/** When a step was refreshed: how long ago in the line, the exact time and its record (`refreshWords`) in the tooltip. */
function RefreshTag({ step, words }: { step: LineageStep; words: string }) {
  return (
    <Tooltip>
      <TooltipTrigger render={<span tabIndex={0} className={cn("inline-flex items-center gap-1 rounded-sm text-foreground/80 tabular-nums", FOCUS)} />}>
        <Clock aria-hidden className="size-3 shrink-0" />
        <span aria-hidden>{formatRelativeTime(step.refreshedAt)}</span>
        <span className="sr-only">{words}</span>
      </TooltipTrigger>
      <TooltipContent>{words}</TooltipContent>
    </Tooltip>
  )
}

/**
 * One field of the chart. A recorded mapping reads `field ← column` with the
 * nearest column (the first the server lists); every recorded `table.column`
 * and the step it came through are in the tooltip.
 */
function FieldChip({ field }: { field: FieldColumns }) {
  const from = field.columns[0]
  const chip = (
    <span
      className={cn(
        "inline-flex h-6 max-w-full min-w-0 items-center gap-1 rounded-md px-2 text-xs leading-none",
        from ? "bg-primary/10 text-foreground" : "bg-muted text-foreground/80",
      )}
    >
      <span className="truncate">{field.field}</span>
      {from ? (
        <>
          <span aria-hidden className="text-muted-foreground">←</span>
          <span className="truncate text-muted-foreground">{from.column}</span>
        </>
      ) : null}
    </span>
  )
  if (!from) return chip
  const detail = field.columns.map((c) => `${c.table}.${c.column} · ${c.via}`).join(", ")
  return (
    <Tooltip>
      <TooltipTrigger render={<span tabIndex={0} className={cn("inline-flex max-w-full min-w-0 rounded-md", FOCUS)} />}>
        {chip}
        <span className="sr-only">, from {detail}</span>
      </TooltipTrigger>
      <TooltipContent>{detail}</TooltipContent>
    </Tooltip>
  )
}

/** The loading look takes the route's shape (DESIGN.md §3: one page, one loading look). */
function RouteSkeleton() {
  return (
    <div role="status" aria-label="Loading the path">
      <ol aria-hidden className="grid">
        {[0, 1, 2, 3].map((i) => (
          <li key={i} className={cn("relative grid grid-cols-[2rem_minmax(0,1fr)] gap-x-3", i < 3 && "pb-4")}>
            {i < 3 ? <span className="absolute top-10 bottom-2 left-[calc(1rem-0.5px)] w-px bg-border" /> : null}
            <Skeleton className="size-8 rounded-full" />
            <div className="grid content-start gap-1.5 pt-0.5">
              <Skeleton className="h-4 w-40 max-w-full" />
              <Skeleton className="h-3 w-16" />
            </div>
          </li>
        ))}
      </ol>
    </div>
  )
}
