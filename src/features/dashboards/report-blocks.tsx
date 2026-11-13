"use client"

import * as React from "react"
import Link from "next/link"
import { LayoutGrid, Lock } from "lucide-react"

import { cn } from "@/lib/utils"
import { safeReportUrl } from "@/lib/report-blocks"
import type { ReportBlock, ReportChartEntry } from "@/services/contracts/dashboard-reports"
import { TileBody } from "./tile-body"

/**
 * A report's blocks, in reading order (`BI-18` part D, R1): the reader page and
 * the PDF's print view both draw a report with this, so the PDF shows what the
 * page shows.
 *
 * Every string is rendered as text. React escapes it, and nothing here parses
 * markup, so `<script>` typed into a paragraph is shown as those characters. A
 * link's address is checked again here (`safeReportUrl`, the server's rule): one
 * that is not `https://`, `http://`, `mailto:` or a console path is shown as
 * plain words, never as a link.
 *
 * A chart block draws its chart with `TileBody`, the dashboard tile's own body,
 * from the cell the server read for this person (their masks and row filters).
 * A chart the person may not see is the server's fixed sentence and nothing
 * else: no title, no data, no link to its dashboard.
 */
export function ReportBlocks({
  blocks,
  charts,
  dark,
  print = false,
}: {
  readonly blocks: readonly ReportBlock[]
  readonly charts: Readonly<Record<string, ReportChartEntry>>
  readonly dark: boolean
  /** The PDF's print view: no links out of the page, charts drawn without animation by `RenderMode`. */
  readonly print?: boolean
}) {
  return (
    <div className="flex flex-col gap-4" data-report-blocks>
      {blocks.map((block, i) => (
        <ReportBlockView key={i} block={block} charts={charts} dark={dark} print={print} />
      ))}
    </div>
  )
}

const HEADING_CLASS: Record<1 | 2 | 3, string> = {
  1: "text-xl font-semibold tracking-tight",
  2: "text-lg font-semibold",
  3: "text-base font-semibold",
}

function ReportBlockView({
  block,
  charts,
  dark,
  print,
}: {
  readonly block: ReportBlock
  readonly charts: Readonly<Record<string, ReportChartEntry>>
  readonly dark: boolean
  readonly print: boolean
}) {
  switch (block.type) {
    case "heading": {
      // The report's title is the page's `h1`; a level-1 block heading is an `h2`.
      const Tag = block.level === 1 ? "h2" : block.level === 3 ? "h4" : "h3"
      return <Tag className={cn("text-foreground", HEADING_CLASS[block.level] ?? HEADING_CLASS[2])}>{block.text}</Tag>
    }
    case "paragraph":
      return block.text ? (
        <p className="whitespace-pre-wrap text-sm leading-relaxed text-foreground">{block.text}</p>
      ) : null
    case "list": {
      const Tag = block.ordered ? "ol" : "ul"
      return (
        <Tag className={cn("flex flex-col gap-1 pl-5 text-sm leading-relaxed", block.ordered ? "list-decimal" : "list-disc")}>
          {block.items.map((item, i) => (
            <li key={i}>{item}</li>
          ))}
        </Tag>
      )
    }
    case "link": {
      const words = block.text || block.url
      if (!safeReportUrl(block.url)) return <p className="text-sm">{words}</p>
      return (
        <p className="text-sm">
          <a
            href={block.url}
            target="_blank"
            rel="noopener noreferrer"
            className="text-primary underline-offset-2 hover:underline focus-visible:rounded-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
          >
            {words}
          </a>
          {print && block.text ? <span className="text-muted-foreground"> ({block.url})</span> : null}
        </p>
      )
    }
    case "chart":
      return <ReportChart entry={charts[block.chart]} dark={dark} print={print} />
    default:
      return null
  }
}

/** One chart block: its card, title, source dashboard and live body. */
export function ReportChart({
  entry,
  dark,
  print = false,
}: {
  readonly entry: ReportChartEntry | undefined
  readonly dark: boolean
  readonly print?: boolean
}) {
  if (!entry || entry.unavailable !== undefined) {
    return (
      <figure
        role="status"
        className="flex items-center gap-2 rounded-xl border border-dashed bg-muted/30 px-4 py-6 text-sm text-muted-foreground"
        data-report-chart="unavailable"
      >
        <Lock className="size-4 shrink-0" aria-hidden />
        {entry?.unavailable ?? "This chart is not available."}
      </figure>
    )
  }
  const { spec, cell, board, boardName } = entry
  return (
    <figure
      className="flex flex-col gap-2 rounded-xl border bg-card p-4 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)] break-inside-avoid"
      data-report-chart={spec.id}
    >
      <figcaption className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <span className="min-w-0">
          <span className="block truncate font-medium text-foreground">{spec.title}</span>
          {spec.subtitle ? <span className="block text-xs text-muted-foreground">{spec.subtitle}</span> : null}
        </span>
        {boardName && !print ? (
          <Link
            href={`/dashboards/${encodeURIComponent(board)}`}
            className="inline-flex shrink-0 items-center gap-1 rounded-sm text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
          >
            <LayoutGrid className="size-3" aria-hidden />
            {boardName}
          </Link>
        ) : boardName ? (
          <span className="text-xs text-muted-foreground">{boardName}</span>
        ) : null}
      </figcaption>
      <div className={cn(spec.kind === "kpi" || spec.kind === "gauge" || spec.kind === "progress" ? "h-40" : "h-80")}>
        <TileBody spec={spec} cell={cell ?? undefined} dark={dark} loading={false} />
      </div>
    </figure>
  )
}
