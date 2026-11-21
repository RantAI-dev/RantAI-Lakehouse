"use client"

/**
 * The Charts tab of the one side panel on the report editor (`BI-62`,
 * decision D6): every dashboard's charts as live pictures, searched by title,
 * dragged into the document or clicked in at the cursor. It is a tab of the
 * shell's panel (`useSidePanelTabs`, `BI-61` T6), beside AI Copilot, never a
 * second panel; on a phone the shell draws that panel as a sheet.
 *
 * `useReportChartsTab` registers the tab and makes its first state: on a wide
 * screen the panel opens on Charts for an empty report, and otherwise comes
 * back the way the person left it last time (kept in this browser only).
 */
import * as React from "react"
import { createPortal } from "react-dom"
import { Search } from "lucide-react"

import { useSidePanel, useSidePanelTabs, type SidePanelTab } from "@/components/app-shell/side-panel"
import { MAX_REPORT_CHARTS } from "@/lib/report-blocks"
import { cn } from "@/lib/utils"
import { ChartChoices } from "./report-chart-picker"
import type { ReportChartChoice } from "./report-charts"
import { EASE } from "./report-editor-ui"

export const CHARTS_TAB = "charts"
const TABS: readonly SidePanelTab[] = [{ id: CHARTS_TAB, label: "Charts" }]

/** Where the editor remembers whether Charts was open (a per-browser convenience). */
export const CHARTS_TAB_KEY = "lakehouse.report-editor.charts-open"

function wideScreen(): boolean {
  return typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia("(min-width: 1024px)").matches
}

function remembered(): boolean {
  try {
    return window.localStorage.getItem(CHARTS_TAB_KEY) === "1"
  } catch {
    return false
  }
}

function remember(open: boolean) {
  try {
    window.localStorage.setItem(CHARTS_TAB_KEY, open ? "1" : "0")
  } catch {
    // Storage blocked: the panel just starts closed next time.
  }
}

/**
 * Register the Charts tab while the editor is mounted. `ready` is false until
 * the document is known, so the first state can depend on whether it is empty.
 */
export function useReportChartsTab({ ready, empty }: { readonly ready: boolean; readonly empty: boolean }) {
  useSidePanelTabs(TABS)
  const panel = useSidePanel()
  const [decided, setDecided] = React.useState(false)
  const { open } = panel
  React.useEffect(() => {
    if (decided || !ready) return
    if (wideScreen() && (empty || remembered())) open(CHARTS_TAB)
    setDecided(true)
  }, [decided, ready, empty, open])
  const shown = panel.active === CHARTS_TAB
  React.useEffect(() => {
    if (decided) remember(shown)
  }, [decided, shown])
  return { shown, toggle: () => panel.toggle(CHARTS_TAB), host: panel.host }
}

export function ChartShelf({
  used,
  count,
  onPick,
}: {
  readonly used: ReadonlySet<string>
  readonly count: number
  readonly onPick: (chart: ReportChartChoice) => void
}) {
  const [query, setQuery] = React.useState("")
  const full = count >= MAX_REPORT_CHARTS
  return (
    <div className="flex min-h-0 flex-1 flex-col max-md:max-h-[60dvh]">
      <div className="grid shrink-0 gap-2 px-3 pt-3 pb-2">
        <label className={cn("flex h-8 items-center gap-2 rounded-lg bg-muted/70 px-2.5 text-sm focus-within:ring-3 focus-within:ring-ring/40 dark:bg-input/30", EASE)}>
          <Search className="size-4 shrink-0 text-muted-foreground" aria-hidden />
          <span className="sr-only">Search charts</span>
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search charts"
            className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-muted-foreground"
          />
        </label>
        <p className="truncate px-0.5 text-xs text-muted-foreground">
          {full ? `${MAX_REPORT_CHARTS} of ${MAX_REPORT_CHARTS} charts · remove one to add another` : "Drag into the report, or click to add at the cursor"}
        </p>
      </div>
      <div className="rd-fade-y min-h-0 flex-1 overflow-y-auto px-3 pb-4">
        <ChartChoices compact query={query} onClearQuery={() => setQuery("")} used={used} full={full} onPick={onPick} />
      </div>
    </div>
  )
}

/** The shelf drawn into the shared panel's body while Charts is the tab shown. */
export function ChartShelfPortal({
  host,
  shown,
  ...props
}: {
  readonly host: HTMLElement | null
  readonly shown: boolean
} & React.ComponentProps<typeof ChartShelf>) {
  return host && shown ? createPortal(<ChartShelf {...props} />, host) : null
}
