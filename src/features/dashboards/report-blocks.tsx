"use client"

import * as React from "react"
import Link from "next/link"
import { LayoutGrid, Lock } from "lucide-react"

import { cn } from "@/lib/utils"
import { safeReportUrl } from "@/lib/report-blocks"
import { parseInline, type InlineRun } from "@/lib/report-inline"
import type { ReportBlock, ReportChartEntry } from "@/services/contracts/dashboard-reports"
import type { MapLoader } from "./echarts-maps"
import { TileBody } from "./tile-body"
import "./report-document.css"

/**
 * A report's blocks, in reading order (`BI-18` part D, R1; `BI-62`, D7): the
 * one renderer the reader page, the editor's Preview and the PDF's print view
 * all draw a report with, so the PDF shows what the page shows. The editor's
 * own document uses the same stylesheet (`report-document.css`), so writing
 * looks like reading.
 *
 * Every string is rendered as text. The inline marks of `BI-62` (`**bold**`,
 * `*italic*`, `[words](address)`) are parsed by `parseInline` into runs and
 * built here as React nodes, never through `dangerouslySetInnerHTML`, so
 * `<script>` typed into a paragraph is shown as those characters. A link's
 * address is checked again here (`safeReportUrl`, the server's rule): one
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
  title,
  className,
  loadMap,
}: {
  readonly blocks: readonly ReportBlock[]
  readonly charts: Readonly<Record<string, ReportChartEntry>>
  readonly dark: boolean
  /** The PDF's print view: no links out of the page, charts drawn without animation by `RenderMode`. */
  readonly print?: boolean
  /** The report's title as the document's first line (the reader page, Preview). */
  readonly title?: string
  readonly className?: string
  /** Map boundary loader for headless print views with no session (`PR #528 review BLOCKER`). */
  readonly loadMap?: MapLoader
}) {
  return (
    <div className={cn("rd-doc", print && "rd-print", className)} data-report-blocks>
      {title !== undefined ? <h1 className="rd-title">{title || "Untitled report"}</h1> : null}
      {blocks.map((block, i) => (
        <ReportBlockView key={i} block={block} charts={charts} dark={dark} print={print} loadMap={loadMap} />
      ))}
    </div>
  )
}

/** One run's words inside its bold and italic. */
function Marked({ run }: { readonly run: InlineRun }) {
  let node: React.ReactNode = run.text
  if (run.italic) node = <em>{node}</em>
  if (run.bold) node = <strong>{node}</strong>
  return <>{node}</>
}

/**
 * A text in the inline subset as React nodes: runs that share a link are one
 * `<a>`, an address leaves the console in a new tab, and a console path stays
 * in it.
 */
export function InlineText({ source }: { readonly source: string }) {
  const runs = parseInline(source)
  const out: React.ReactNode[] = []
  for (let i = 0; i < runs.length; ) {
    const href = runs[i].href
    let j = i
    while (j < runs.length && runs[j].href === href) j += 1
    const words = runs.slice(i, j).map((r, k) => <Marked key={k} run={r} />)
    if (href !== undefined && safeReportUrl(href)) {
      const external = !href.trim().startsWith("/")
      out.push(
        <a
          key={i}
          href={href.trim()}
          {...(external ? { target: "_blank", rel: "noopener noreferrer" } : {})}
        >
          {words}
        </a>,
      )
    } else {
      out.push(<React.Fragment key={i}>{words}</React.Fragment>)
    }
    i = j
  }
  return <>{out}</>
}

function ReportBlockView({
  block,
  charts,
  dark,
  print,
  loadMap,
}: {
  readonly block: ReportBlock
  readonly charts: Readonly<Record<string, ReportChartEntry>>
  readonly dark: boolean
  readonly print: boolean
  readonly loadMap?: MapLoader
}) {
  switch (block.type) {
    case "heading": {
      // The report's title is the page's `h1`; a level-1 block heading is an `h2`.
      const level = block.level === 1 || block.level === 3 ? block.level : 2
      const Tag = level === 1 ? "h2" : level === 3 ? "h4" : "h3"
      return (
        <Tag data-level={level}>
          <InlineText source={block.text} />
        </Tag>
      )
    }
    case "paragraph":
      return block.text ? (
        <p>
          <InlineText source={block.text} />
        </p>
      ) : null
    case "list": {
      const Tag = block.ordered ? "ol" : "ul"
      return (
        <Tag>
          {block.items.map((item, i) => (
            <li key={i}>
              <InlineText source={item} />
            </li>
          ))}
        </Tag>
      )
    }
    case "quote":
      return (
        <blockquote>
          <p>
            <InlineText source={block.text} />
          </p>
        </blockquote>
      )
    case "divider":
      return <hr />
    case "link": {
      // A report made before BI-62: its words are plain text, as they always were.
      const words = block.text || block.url
      if (!safeReportUrl(block.url)) return <p>{words}</p>
      return (
        <p>
          <a href={block.url} target="_blank" rel="noopener noreferrer">
            {words}
          </a>
          {print && block.text ? <span className="text-muted-foreground"> ({block.url})</span> : null}
        </p>
      )
    }
    case "chart":
      return (
        <ReportChartBlock wide={block.width === "wide"}>
          <ReportChart entry={charts[block.chart]} dark={dark} print={print} wide={block.width === "wide"} loadMap={loadMap} />
        </ReportChartBlock>
      )
    default:
      return null
  }
}

/** The box a chart block takes in the document: the text column, or wider (`BI-62`). */
export function ReportChartBlock({
  wide,
  children,
  className,
  ...rest
}: {
  readonly wide: boolean
  readonly children: React.ReactNode
  readonly className?: string
} & Omit<React.HTMLAttributes<HTMLDivElement>, "className" | "children">) {
  return (
    <div {...rest} className={cn("rd-chart-block", className)} data-width={wide ? "wide" : "text"}>
      {children}
    </div>
  )
}

/** How tall a chart is in the document, by kind: a number needs little room, a table or an axis more. */
export function reportChartHeight(kind: string | undefined, wide: boolean): string {
  if (kind === "kpi" || kind === "gauge" || kind === "progress") return "h-32"
  if (kind === "table" || kind === "pivot") return wide ? "h-80" : "h-72"
  return wide ? "h-80" : "h-64"
}

/**
 * One chart block's card (DESIGN.md §3: no border, a soft fill): its title and
 * "from <dashboard>" small above the live body. The editor passes `toolbar`
 * and `selected`; the reader and the PDF pass neither.
 */
export function ReportChart({
  entry,
  dark,
  print = false,
  wide = false,
  selected = false,
  toolbar,
  loadMap,
}: {
  readonly entry: ReportChartEntry | undefined
  readonly dark: boolean
  readonly print?: boolean
  readonly wide?: boolean
  readonly selected?: boolean
  readonly toolbar?: React.ReactNode
  readonly loadMap?: MapLoader
}) {
  const card = cn(
    "group/chart relative m-0 rounded-2xl bg-muted/50 px-4 pt-3.5 pb-3 break-inside-avoid transition-[box-shadow,background-color] duration-200 ease-[cubic-bezier(0.2,0,0,1)] dark:bg-card",
    selected && "ring-2 ring-primary/35 dark:ring-sky-400/40",
  )
  if (!entry || entry.unavailable !== undefined) {
    return (
      <figure role="status" className={card} data-report-chart="unavailable">
        <span className="flex items-center gap-2.5 py-5 text-sm text-muted-foreground">
          <Lock className="size-4 shrink-0" aria-hidden />
          {entry?.unavailable ?? "This chart is not available."}
        </span>
        {toolbar}
      </figure>
    )
  }
  const { spec, cell, board, boardName } = entry
  return (
    <figure className={card} data-report-chart={spec.id}>
      <figcaption className={cn("flex min-w-0 items-baseline gap-2", toolbar ? "pr-1 sm:pr-36" : "")}>
        <span className="truncate text-sm leading-5 font-medium text-foreground" title={spec.title}>
          {spec.title}
        </span>
        {boardName && !print ? (
          <Link
            href={`/dashboards/${encodeURIComponent(board)}`}
            className="hidden shrink-0 items-center gap-1 rounded-sm text-xs leading-4 text-muted-foreground transition-colors duration-150 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50 sm:inline-flex"
          >
            <LayoutGrid className="size-3" aria-hidden />
            from {boardName}
          </Link>
        ) : boardName ? (
          <span className="shrink-0 text-xs leading-4 text-muted-foreground">from {boardName}</span>
        ) : null}
      </figcaption>
      {spec.subtitle ? <p className="mt-0.5 text-xs leading-4 text-muted-foreground">{spec.subtitle}</p> : null}
      <div className={cn("relative mt-2 min-h-0", reportChartHeight(spec.kind, wide))}>
        <TileBody spec={spec} cell={cell ?? undefined} dark={dark} loading={false} loadMap={loadMap} />
      </div>
      {toolbar}
    </figure>
  )
}
