"use client"

import type * as React from "react"
import { ChevronDown, Database, Gauge } from "lucide-react"

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuGroupLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { formatBytes, formatCost, formatLagSeconds } from "@/lib/format"
import { queryEngineLabel } from "@/lib/status"
import { cn } from "@/lib/utils"
import type { QueryEngine, QueryEstimate } from "@/services/contracts/queries"
import { useStudioKeys } from "./studio-toolbar"
import type { useQueryStudio } from "./use-query-studio"

type Studio = ReturnType<typeof useQueryStudio>

/** What each engine is for, in the engine menu. */
const ENGINES: ReadonlyArray<{ value: QueryEngine; note: string }> = [
  { value: "clickhouse", note: "The hot analytical store, with a pre-run estimate" },
  { value: "trino", note: "Iceberg tables, Bronze included" },
]

const SEGMENT =
  "inline-flex h-6 shrink-0 items-center gap-1.5 rounded px-1.5 outline-none transition-colors duration-150 ease-ui focus-visible:ring-3 focus-visible:ring-ring/50"

/**
 * The engine, as an app menu (not a native select), named the way the
 * engines name themselves.
 */
function EngineMenu({
  studio,
  availableEngines,
}: {
  readonly studio: Studio
  readonly availableEngines: readonly QueryEngine[]
}) {
  // BI-44: a `bi-only` install runs one engine. The page keeps the tab on it,
  // but a stale draft can still name the other one before its effect lands;
  // the menu never offers an engine the API would refuse with a 400.
  const engines = ENGINES.filter((engine) => availableEngines.includes(engine.value))
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <button
            type="button"
            aria-label={`Engine: ${queryEngineLabel(studio.engine)}`}
            className={cn(SEGMENT, "text-foreground/85 hover:bg-muted aria-expanded:bg-muted")}
          />
        }
      >
        <Database className="size-3.5 text-muted-foreground" aria-hidden />
        {queryEngineLabel(studio.engine)}
        <ChevronDown className="size-3 opacity-60" aria-hidden />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-72">
        <DropdownMenuGroup>
          <DropdownMenuGroupLabel>Run on</DropdownMenuGroupLabel>
          <DropdownMenuRadioGroup
            value={studio.engine}
            onValueChange={(value) => studio.setEngine(value as QueryEngine)}
          >
            {engines.map((engine) => (
              <DropdownMenuRadioItem key={engine.value} value={engine.value} className="items-start">
                <span className="grid">
                  <span>{queryEngineLabel(engine.value)}</span>
                  <span className="text-xs text-muted-foreground">{engine.note}</span>
                </span>
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** The cost range of a ready estimate, or `null` when there is none worth saying. */
function costRange(data: QueryEstimate): string | null {
  if (data.estimatedCostMin == null || data.estimatedCostMax == null) return null
  if ((data.estimatedBytes ?? 0) <= 0) return null
  return data.estimatedCostMin === data.estimatedCostMax
    ? formatCost(data.estimatedCostMin)
    : `${formatCost(data.estimatedCostMin)}–${formatCost(data.estimatedCostMax)}`
}

/**
 * The estimate in one line. Every state of the pre-run estimate has its own
 * short sentence here; the reasons and the details are the Plan tab's, which
 * a click on the line opens.
 */
function EstimateSummary({
  studio,
  onOpenPlan,
}: {
  readonly studio: Studio
  readonly onOpenPlan: () => void
}) {
  const { estimateAct: state, estimateSupported, estimatable, estimateWaitsFor } = studio
  const quiet = (text: string) => <span className="shrink-0 px-1.5">{text}</span>
  // LH-QS-4-08: the same sentence the Plan tab says.
  if (!estimateSupported) return quiet("No pre-run estimate for Trino")
  if (estimateWaitsFor !== null) return quiet(`Fill in {{${estimateWaitsFor}}} for an estimate`)
  if (state.status === "idle" || !estimatable) return quiet("Type SQL for an estimate")
  if (state.status === "error") {
    return (
      <span className="inline-flex shrink-0 items-center gap-1 px-1.5">
        Estimate unavailable
        <button
          type="button"
          onClick={studio.reEstimate}
          className={cn(SEGMENT, "h-5 px-1 text-primary hover:bg-muted dark:text-sky-300")}
        >
          Retry
        </button>
      </span>
    )
  }
  if (state.status === "pending" && !state.data) {
    return (
      <span role="status" aria-label="Estimating" className="shrink-0 px-1.5">
        Estimating…
      </span>
    )
  }
  const data = state.data
  if (!data) return null
  const open = (children: React.ReactNode, label: string) => (
    <button
      type="button"
      onClick={onOpenPlan}
      title="Open the plan"
      aria-label={label}
      className={cn(SEGMENT, "hover:bg-muted hover:text-foreground", state.status === "pending" && "opacity-60")}
    >
      <Gauge className="size-3.5" aria-hidden />
      {children}
    </button>
  )
  if (data.supported === false) return open("No pre-run estimate", "No pre-run estimate: open the plan")
  if (data.error) return open("Cannot be planned", "This query cannot be planned: open the plan")
  const cost = costRange(data)
  const parts = [`≈ ${formatBytes(data.estimatedBytes)}`, ...(cost ? [cost] : [])]
  // Cache and data age join the line only when they were measured: the Plan
  // tab says "not measured" once, with the rest of the details.
  const extra: string[] = []
  if (data.cacheEligible != null) extra.push(data.cacheEligible ? "cache eligible" : "not cacheable")
  if (!data.catalogHidden && data.freshnessLagSeconds != null) {
    extra.push(`data ${formatLagSeconds(data.freshnessLagSeconds)} old`)
  }
  const text = [parts.join(" · "), ...extra].join(" · ")
  return open(<span className="tabular-nums">{text}</span>, `Estimate ${text}: open the plan`)
}

/**
 * The line under the editor (`UX-9`, mockup B's status bar): which engine
 * runs the query, what it will cost before it runs, and the two keys worth
 * knowing. The engine and the estimate are shown here and nowhere else on
 * the page; a run's own metrics are the result's.
 */
export function StudioStatusBar({
  studio,
  onOpenPlan,
  availableEngines = ["clickhouse", "trino"],
}: {
  readonly studio: Studio
  readonly onOpenPlan: () => void
  /** The engines this installation can run (`BI-44`). */
  readonly availableEngines?: readonly QueryEngine[]
}) {
  const keys = useStudioKeys()
  return (
    <div className="flex h-8 items-center gap-1 overflow-x-auto border-t border-border/60 px-2 text-xs text-muted-foreground [scrollbar-width:none]">
      <EngineMenu studio={studio} availableEngines={availableEngines} />
      <span aria-hidden className="text-border">
        |
      </span>
      <EstimateSummary studio={studio} onOpenPlan={onOpenPlan} />
      {keys ? (
        <span className="ml-auto hidden shrink-0 px-1.5 sm:inline">
          {keys.run} run · {keys.commands} commands
        </span>
      ) : null}
    </div>
  )
}
