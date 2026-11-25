"use client"

/**
 * A chart in the report editor (`BI-62`, decision D5): a Tiptap atom block
 * that holds only which chart and how wide, drawn live by the reader's own
 * card (`ReportChart`), so what the writer sees is what the reader gets. A
 * small toolbar on hover, on focus or when the chart is selected: Swap, Text
 * width / Wide, Remove. A tap on a phone selects it, which shows the toolbar.
 */
import * as React from "react"
import { Node } from "@tiptap/core"
import { NodeViewWrapper, ReactNodeViewRenderer, type ReactNodeViewProps } from "@tiptap/react"
import { useTheme } from "next-themes"
import { RectangleHorizontal, Replace, StretchHorizontal, Trash2 } from "lucide-react"

import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"
import { ReportChart, ReportChartBlock, reportChartHeight } from "./report-blocks"
import { chartWaiting, useReportCharts } from "./report-charts"
import { CHART_NODE_SPEC } from "./report-editor-extensions"
import { EASE, FOCUS, IconTip } from "./report-editor-ui"

export type PickerRequest = { mode: "insert"; at?: number } | { mode: "swap"; pos: number; current: string }

/** What a chart's view needs from the page around it (node views render inside the page's tree). */
export type ReportEditorUi = {
  readOnly: boolean
  openPicker: (req: PickerRequest) => void
}

export const ReportEditorUiContext = React.createContext<ReportEditorUi>({ readOnly: true, openPicker: () => {} })

const TOOL = cn(
  "grid size-7 place-items-center rounded-md text-muted-foreground hover:bg-background hover:text-foreground active:scale-[0.96] dark:hover:bg-muted",
  EASE,
  FOCUS,
)

function ChartView({ node, selected, updateAttributes, deleteNode, getPos, editor }: ReactNodeViewProps) {
  const ui = React.useContext(ReportEditorUiContext)
  const charts = useReportCharts()
  const { resolvedTheme } = useTheme()
  const id = typeof node.attrs.chart === "string" ? node.attrs.chart : ""
  const wide = node.attrs.width === "wide"
  const entry = charts.entry(id)
  const { ensure } = charts
  React.useEffect(() => {
    if (!entry && id) ensure(id)
  }, [entry, id, ensure])
  const pos = () => (typeof getPos === "function" ? (getPos() ?? 0) : 0)

  const toolbar = ui.readOnly ? null : (
    <div
      // A mouse click on a tool keeps the focus in the text (the keyboard can still Tab to them).
      onMouseDown={(e) => e.preventDefault()}
      className={cn(
        "absolute top-2.5 right-2.5 flex items-center gap-0.5 rounded-lg bg-muted/90 p-0.5 opacity-0 shadow-xs backdrop-blur-sm group-hover/chart:opacity-100 focus-within:opacity-100 dark:bg-background/80",
        EASE,
        selected && "opacity-100",
      )}
    >
      <IconTip label="Swap chart">
        <button type="button" aria-label="Swap chart" className={TOOL} onClick={() => ui.openPicker({ mode: "swap", pos: pos(), current: id })}>
          <Replace className="size-3.5" aria-hidden />
        </button>
      </IconTip>
      <div role="group" aria-label="Width" className="flex items-center">
        <IconTip label="Text width">
          <button
            type="button"
            aria-label="Text width"
            aria-pressed={!wide}
            onClick={() => updateAttributes({ width: "text" })}
            className={cn(TOOL, !wide && "bg-background text-foreground dark:bg-muted")}
          >
            <RectangleHorizontal className="size-3.5" aria-hidden />
          </button>
        </IconTip>
        <IconTip label="Wide">
          <button
            type="button"
            aria-label="Wide"
            aria-pressed={wide}
            onClick={() => updateAttributes({ width: "wide" })}
            className={cn(TOOL, wide && "bg-background text-foreground dark:bg-muted")}
          >
            <StretchHorizontal className="size-3.5" aria-hidden />
          </button>
        </IconTip>
      </div>
      <IconTip label="Remove (Delete)">
        <button
          type="button"
          aria-label="Remove chart"
          className={cn(TOOL, "hover:text-destructive")}
          onClick={() => {
            deleteNode()
            editor.commands.focus()
          }}
        >
          <Trash2 className="size-3.5" aria-hidden />
        </button>
      </IconTip>
    </div>
  )

  const waiting = entry ? null : chartWaiting(charts, id)
  return (
    <NodeViewWrapper data-chart-id={id}>
      <ReportChartBlock wide={wide} contentEditable={false}>
        {waiting === "loading" ? (
          <figure
            aria-busy="true"
            aria-label="Drawing the chart"
            className={cn("relative m-0 rounded-2xl bg-muted/50 px-4 pt-3.5 pb-3 dark:bg-card", selected && !ui.readOnly && "ring-2 ring-primary/35")}
          >
            <Skeleton className="h-4 w-48" />
            <Skeleton className={cn("mt-3", reportChartHeight(charts.choice(id)?.kind, wide))} />
            {toolbar}
          </figure>
        ) : (
          <ReportChart
            entry={entry ?? (waiting ? { unavailable: waiting.sentence } : undefined)}
            dark={resolvedTheme === "dark"}
            wide={wide}
            selected={selected && !ui.readOnly}
            toolbar={toolbar}
          />
        )}
      </ReportChartBlock>
    </NodeViewWrapper>
  )
}

export const ChartNode = Node.create({
  ...CHART_NODE_SPEC,
  addNodeView() {
    return ReactNodeViewRenderer(ChartView)
  },
})
