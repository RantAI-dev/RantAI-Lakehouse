"use client"

import * as React from "react"
import Link from "next/link"
import { createPortal } from "react-dom"
import { BarChart3, ChartColumn, ChevronRight, Maximize2, Minimize2, Minus, Plus, Scan } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { KIND_LABEL, focusIds, nodeHref, nodeKind } from "@/features/governance/lineage-graph"
import { useService } from "@/hooks/use-service"
import { boxHeight, clampView, fitScale, fitView, zoomAt, ZOOM_MAX, ZOOM_STEP, type Size, type View } from "@/lib/pan-zoom"
import { cn } from "@/lib/utils"
import { dashboardService } from "@/services"
import type { Board, ChartOutline } from "@/services/contracts/dashboards"
import type { LineageGraph } from "@/services/contracts/governance"

/** Something drawn beside the recorded graph: a dependent, or a table the catalog relates. */
export type MapExtra = { id: string; name: string; kind: string; href: string | null }

export type MapNode = {
  id: string
  /** The small line above the name: what the node is. */
  caption: string
  label: string
  href: string | null
  /** Columns left (negative) or right (positive) of the asset, which is 0. */
  level: number
  centre: boolean
  /** Part of the recorded lineage, or tied to the asset some other way. */
  recorded: boolean
  /**
   * Set on a node that stands for the charts of one dashboard
   * ([`groupCharts`]): it opens in place to list them.
   */
  group?: { key: string; title: string; count: number; open: boolean }
}

export type MapEdge = {
  id: string
  from: string
  to: string
  /** How the two are connected, in a word; empty when the nodes say it. */
  label: string
  /** The record behind the edge, shown on hover. */
  title?: string
  recorded: boolean
}

/**
 * The authored pipeline a `pipeline` edge was recorded from. The graph
 * draws a pipeline as an edge from its source to its target, and names it
 * only in the edge's `evidence` (`routes/lineage.rs`: "authored pipeline
 * <name> (<id>)").
 */
export function edgePipeline(evidence: string | undefined): { id: string; name: string } | null {
  const match = /^authored pipeline (.+) \((pl-[^()]+)\)$/.exec(evidence ?? "")
  return match ? { id: match[2], name: match[1] } : null
}

const SELF = "self"

function capitalize(word: string) {
  return word.charAt(0).toUpperCase() + word.slice(1)
}

/**
 * The asset's lineage as a mind map: the asset in the middle, what it is
 * built from to its left, what is built from it to its right — each node
 * as many columns away as its longest recorded path — and, as dashed
 * branches, what the catalog knows beyond the recorded edges: what reads
 * the asset (`dependents`) and the tables it relates to it.
 *
 * An asset with no recorded lineage still has a centre, so its dependents
 * and related tables have something to branch from.
 */
export function mindmapModel(input: {
  graph: LineageGraph | null
  /** The asset itself, for the centre when the graph does not hold it. */
  self: { label: string; kind: string }
  dependents: MapExtra[]
  relatedUpstream: MapExtra[]
  relatedDownstream: MapExtra[]
}): { nodes: MapNode[]; edges: MapEdge[] } {
  const graph = input.graph
  const focus = graph ? focusIds(graph) : new Set<string>()
  const ids = new Set((graph?.nodes ?? []).map((n) => n.id))
  // The API draws an authored pipeline as an edge from its source to its
  // target. On the map it is a stop of its own — source, pipeline, target
  // — so it can be seen, and opened.
  const pipelines = new Map<string, { id: string; name: string }>()
  const recordedEdges = (graph?.edges ?? [])
    .filter((e) => ids.has(e.from) && ids.has(e.to))
    .flatMap((e) => {
      const pipeline = edgePipeline(e.evidence)
      if (!pipeline) return [{ id: e.id, from: e.from, to: e.to, label: e.kind, title: e.evidence }]
      const node = `pipeline:${pipeline.id}`
      pipelines.set(node, pipeline)
      return [
        { id: `${e.id}:in`, from: e.from, to: node, label: "", title: e.evidence },
        { id: `${e.id}:out`, from: node, to: e.to, label: "", title: e.evidence },
      ]
    })
  for (const node of pipelines.keys()) ids.add(node)

  // Longest path from the asset, so a table three steps away is not drawn
  // beside one a single step away. Downstream counts up, upstream down.
  const level = new Map<string, number>([...focus].map((id) => [id, 0]))
  for (let pass = 0; pass < ids.size; pass++) {
    let moved = false
    for (const e of recordedEdges) {
      const from = level.get(e.from)
      const to = level.get(e.to)
      if (from !== undefined && from >= 0 && !focus.has(e.to) && (to === undefined || (to > 0 && to < from + 1))) {
        level.set(e.to, from + 1)
        moved = true
      } else if (to !== undefined && to <= 0 && !focus.has(e.from) && (from === undefined || (from < 0 && from > to - 1))) {
        level.set(e.from, to - 1)
        moved = true
      }
    }
    if (!moved) break
  }
  // A node on neither side of the asset hangs off whatever it is tied to.
  for (let pass = 0; pass < ids.size; pass++) {
    for (const e of recordedEdges) {
      const from = level.get(e.from)
      const to = level.get(e.to)
      if (from !== undefined && to === undefined) level.set(e.to, from + 1)
      if (to !== undefined && from === undefined) level.set(e.from, to - 1)
    }
  }

  const nodes: MapNode[] = (graph?.nodes ?? []).map((n) => {
    const kind = nodeKind(n)
    return {
      id: n.id,
      caption: KIND_LABEL[kind] ?? capitalize(kind),
      label: n.label,
      href: focus.has(n.id) ? null : nodeHref(n),
      level: level.get(n.id) ?? 0,
      centre: focus.has(n.id),
      recorded: true,
    }
  })
  for (const [id, pipeline] of pipelines) {
    nodes.push({
      id,
      caption: KIND_LABEL.pipeline,
      label: pipeline.name,
      href: `/pipelines/${encodeURIComponent(pipeline.id)}`,
      level: level.get(id) ?? 0,
      centre: false,
      recorded: true,
    })
  }
  let centreId = nodes.find((n) => n.centre)?.id
  if (centreId === undefined) {
    centreId = SELF
    nodes.push({
      id: SELF,
      caption: KIND_LABEL[input.self.kind] ?? capitalize(input.self.kind),
      label: input.self.label,
      href: null,
      level: 0,
      centre: true,
      recorded: true,
    })
  }

  const seen = new Set<string>()
  const edges: MapEdge[] = recordedEdges
    // Two sources of one pipeline both lead into it: its way out is one line.
    .filter((e) => {
      const key = `${e.from}>${e.to}`
      if (seen.has(key)) return false
      seen.add(key)
      return true
    })
    .map((e) => ({ ...e, recorded: true }))

  const branch = (extra: MapExtra, side: -1 | 1, caption: string, label: string, title: string) => {
    const id = `${side < 0 ? "up" : "down"}:${extra.id}`
    nodes.push({ id, caption, label: extra.name, href: extra.href, level: side, centre: false, recorded: false })
    edges.push({
      id: `edge:${id}`,
      from: side < 0 ? id : centreId,
      to: side < 0 ? centreId : id,
      label,
      title,
      recorded: false,
    })
  }
  for (const d of input.dependents) {
    branch(d, 1, KIND_LABEL[d.kind] ?? capitalize(d.kind), "reads", "Reads this asset")
  }
  const layerOf = (id: string) =>
    id.startsWith("silver.") ? "Silver" : id.startsWith("serving.") ? "Gold" : "Bronze"
  const related = "Tied to this asset by the catalog, not by a recorded lineage edge"
  for (const t of input.relatedUpstream) branch(t, -1, layerOf(t.id), "related", related)
  for (const t of input.relatedDownstream) branch(t, 1, layerOf(t.id), "related", related)

  return { nodes, edges }
}

/**
 * Which dashboard each chart sits on, from the viewer's own readable lists
 * (`BI-71` decision 4): `boardOf` maps a chart id to its board id, `titleOf`
 * a board id to its title. A board the viewer cannot read is not in
 * `listBoards`, so its title never reaches the map.
 */
export type ChartPlacement = { boardOf: Map<string, string>; titleOf: Map<string, string> }

export function chartPlacement(outlines: ChartOutline[], boards: Board[]): ChartPlacement {
  return {
    boardOf: new Map(outlines.map((c) => [c.id, c.board])),
    titleOf: new Map(boards.map((b) => [b.id, b.name || b.id])),
  }
}

const CHART_PREFIX = "chart:"

function chartsWord(count: number): string {
  return `${count.toLocaleString("en-US")} ${count === 1 ? "chart" : "charts"}`
}
const OTHER = "other"

/**
 * The charts that read the asset, one node per dashboard (`BI-71` decision
 * 2): a table read by 111 charts drew 111 nodes, a column many screens tall.
 * Each group reads "<dashboard> · N charts", largest first, then by title;
 * charts the viewer cannot place on a readable dashboard go in one "Other
 * charts" group, last. The `open` group lists its charts again, as the
 * nodes they were, one column to its right. No other kind of node is
 * grouped, and with no placement (it failed to load, or there are no
 * charts) the model is returned as it came.
 */
export function groupCharts(
  model: { nodes: MapNode[]; edges: MapEdge[] },
  placement: ChartPlacement | null,
  open: string | null
): { nodes: MapNode[]; edges: MapEdge[] } {
  const charts = model.nodes.filter((n) => n.id.startsWith(CHART_PREFIX) && !n.centre)
  if (!placement || charts.length === 0) return model

  const groups = new Map<string, { title: string; charts: MapNode[] }>()
  for (const chart of charts) {
    const board = placement.boardOf.get(chart.id.slice(CHART_PREFIX.length))
    const title = board === undefined ? undefined : placement.titleOf.get(board)
    const key = board !== undefined && title !== undefined ? board : OTHER
    const group = groups.get(key) ?? { title: title ?? "Other charts", charts: [] }
    group.charts.push(chart)
    groups.set(key, group)
  }
  const ordered = [...groups.entries()].sort(([ka, a], [kb, b]) =>
    ka === OTHER ? 1 : kb === OTHER ? -1 : b.charts.length - a.charts.length || a.title.localeCompare(b.title)
  )

  const groupOf = new Map<string, string>()
  const grouped = new Set(charts)
  const nodes = model.nodes.filter((n) => !grouped.has(n))
  const extra: MapEdge[] = []
  for (const [key, group] of ordered) {
    const id = `group:${key}`
    const isOpen = open === key
    const level = Math.max(...group.charts.map((c) => c.level))
    for (const chart of group.charts) groupOf.set(chart.id, id)
    const count = group.charts.length
    nodes.push({
      id,
      caption: key === OTHER ? "Charts" : "Dashboard",
      label: `${group.title} · ${chartsWord(count)}`,
      href: null,
      level,
      centre: false,
      recorded: true,
      group: { key, title: group.title, count, open: isOpen },
    })
    if (isOpen) {
      for (const chart of group.charts) {
        nodes.push({ ...chart, level: level + 1 })
        extra.push({ id: `${id}>${chart.id}`, from: id, to: chart.id, label: "", recorded: true })
      }
    }
  }

  // What led into a chart now leads into its group, once.
  const seen = new Set<string>()
  const edges = model.edges
    .map((e) => ({ ...e, from: groupOf.get(e.from) ?? e.from, to: groupOf.get(e.to) ?? e.to }))
    .filter((e) => {
      const key = `${e.from}>${e.to}`
      if (e.from === e.to || seen.has(key)) return false
      seen.add(key)
      return true
    })
  return { nodes, edges: [...edges, ...extra] }
}

/**
 * The placement of the asset's charts, read only when the map has charts
 * to group. Either list failing to load leaves the map ungrouped, quietly
 * (`BI-71` decision 4): grouping is a reading aid, not something to show an
 * error for. `loading` is true while the two reads are out.
 */
export function useChartPlacement(needed: boolean): { loading: boolean; placement: ChartPlacement | null } {
  const state = useService(
    async (s) => {
      if (!needed) return { forNeeded: false as const, placement: null }
      const [outlines, boards] = await Promise.all([
        dashboardService.listChartOutlines(s),
        dashboardService.listBoards(s),
      ])
      return { forNeeded: true as const, placement: chartPlacement(outlines, boards) }
    },
    [needed]
  )
  const current = state.status === "success" && state.data.forNeeded === needed
  return {
    // PR #642 review 1: when `needed` flips false -> true, `state` still holds
    // the prior `forNeeded: false` result until `useService`'s effect resets it
    // to `"loading"`, so treat a mismatched result as loading too.
    loading: needed && (state.status === "loading" || (state.status === "success" && !current)),
    placement: current ? state.data.placement : null,
  }
}

const NODE_W = 192
const NODE_H = 56
const GAP_X = 88
const GAP_Y = 16

/** The schema a caption already names, and so need not be repeated in the name under it. */
const CAPTION_SCHEMA: Record<string, string> = { Bronze: "bronze.", Silver: "silver.", Gold: "serving." }

/**
 * A node's name without the schema its caption already states: three
 * marts cut off at "serving.mart_material_by…" could not be told apart.
 * The full name stays in the tooltip.
 */
export function shortLabel(n: Pick<MapNode, "caption" | "label">): string {
  const schema = CAPTION_SCHEMA[n.caption]
  return schema && n.label.startsWith(schema) ? n.label.slice(schema.length) : n.label
}

type Placed = MapNode & { x: number; y: number }

/**
 * Where each node goes: one column per level, each column centred on the
 * tallest, and — working outward from the asset — each node as near as it
 * can be to the nodes it is tied to, so branches fan out without crossing
 * more than they must.
 */
export function mindmapLayout(model: { nodes: MapNode[]; edges: MapEdge[] }): {
  nodes: Placed[]
  width: number
  height: number
} {
  const levels = [...new Set(model.nodes.map((n) => n.level))].sort((a, b) => a - b)
  const columns = new Map(levels.map((l) => [l, model.nodes.filter((n) => n.level === l)]))
  const rows = Math.max(...[...columns.values()].map((c) => c.length), 1)
  const height = rows * NODE_H + (rows - 1) * GAP_Y
  const placed = new Map<string, Placed>()
  const neighbours = (id: string) =>
    model.edges.flatMap((e) => (e.from === id ? [e.to] : e.to === id ? [e.from] : []))

  // The asset's column first, then outward on each side.
  const outward = [...levels].sort((a, b) => Math.abs(a) - Math.abs(b))
  for (const l of outward) {
    const column = columns.get(l) ?? []
    const pull = (n: MapNode) => {
      const ys = neighbours(n.id).flatMap((id) => {
        const p = placed.get(id)
        return p ? [p.y] : []
      })
      return ys.length === 0 ? Number.POSITIVE_INFINITY : ys.reduce((a, b) => a + b, 0) / ys.length
    }
    const ordered = column
      .map((n, i) => ({ n, i, at: pull(n) }))
      .sort((a, b) => a.at - b.at || a.i - b.i)
      .map(({ n }) => n)
    const top = (height - (ordered.length * NODE_H + (ordered.length - 1) * GAP_Y)) / 2
    ordered.forEach((n, i) => {
      placed.set(n.id, {
        ...n,
        x: levels.indexOf(l) * (NODE_W + GAP_X),
        y: top + i * (NODE_H + GAP_Y),
      })
    })
  }
  const nodes = [...placed.values()].sort((a, b) => a.x - b.x || a.y - b.y)
  return { nodes, width: levels.length * NODE_W + (levels.length - 1) * GAP_X, height }
}

/** A curve from the facing sides of two nodes, and the point halfway along it. */
function curve(from: Placed, to: Placed) {
  const leftToRight = from.x <= to.x
  const x1 = leftToRight ? from.x + NODE_W : from.x
  const x2 = leftToRight ? to.x : to.x + NODE_W
  const y1 = from.y + NODE_H / 2
  const y2 = to.y + NODE_H / 2
  // Two nodes in one column: bow out to the right rather than cut through.
  const sameColumn = from.x === to.x
  const bend = sameColumn ? 60 : (x2 - x1) / 2
  const c1 = x1 + bend
  const c2 = sameColumn ? x2 + NODE_W + bend : x2 - bend
  return {
    d: `M${x1},${y1} C${c1},${y1} ${c2},${y2} ${sameColumn ? x2 + NODE_W : x2},${y2}`,
    mid: { x: sameColumn ? x1 + bend * 0.75 : (x1 + x2) / 2, y: (y1 + y2) / 2 },
  }
}

/**
 * The mind map itself. Solid branches are recorded lineage; dashed ones
 * are what reads the asset and what the catalog relates to it. Nodes are
 * a list, left to right, for a reader who cannot see the lines.
 *
 * With a `placement`, the charts are grouped by dashboard ([`groupCharts`]);
 * a group opens and closes in place, one at a time. The map pans and zooms
 * inside its box ([`PanZoomBox`]) and opens fitted to it.
 */
export function LineageMindmap({
  model: full,
  placement = null,
}: {
  model: { nodes: MapNode[]; edges: MapEdge[] }
  placement?: ChartPlacement | null
}) {
  const [open, setOpen] = React.useState<string | null>(null)
  const model = React.useMemo(() => groupCharts(full, placement, open), [full, placement, open])
  const toggle = React.useCallback((key: string) => setOpen((now) => (now === key ? null : key)), [])
  const layout = React.useMemo(() => mindmapLayout(model), [model])
  const dashed = model.edges.some((e) => !e.recorded)

  return (
    <div className="flex flex-col gap-2">
      <PanZoomBox content={{ width: layout.width, height: layout.height }}>
        <MindmapDrawing edges={model.edges} layout={layout} onToggle={toggle} />
      </PanZoomBox>
      <p className="text-xs text-muted-foreground">
        This asset is in the middle: what it is built from is to its left, what is built from it to
        its right.
        {dashed ? " Dashed branches are not recorded lineage: they are what reads it, and tables the catalog ties to it." : ""}
      </p>
    </div>
  )
}

/**
 * The drawing at zoom 1. Memoised: a pan or zoom moves the box's one
 * transform and must not redraw a hundred nodes on every pointer move.
 */
const MindmapDrawing = React.memo(function MindmapDrawing({
  edges,
  layout,
  onToggle,
}: {
  edges: MapEdge[]
  layout: ReturnType<typeof mindmapLayout>
  onToggle: (key: string) => void
}) {
  const { nodes, width, height } = layout
  const at = new Map(nodes.map((n) => [n.id, n]))
  const centre = nodes.find((n) => n.centre)
  return (
    <div className="relative" style={{ width, height }}>
      <svg
        className="pointer-events-none absolute inset-0 overflow-visible"
        width={width}
        height={height}
        aria-hidden
      >
        {edges.map((e) => {
          const from = at.get(e.from)
          const to = at.get(e.to)
          if (!from || !to) return null
          const touchesCentre = from.id === centre?.id || to.id === centre?.id
          return (
            <path
              key={e.id}
              d={curve(from, to).d}
              fill="none"
              strokeWidth={e.recorded ? 2 : 1.5}
              strokeDasharray={e.recorded ? undefined : "5 5"}
              strokeLinecap="round"
              className={touchesCentre && e.recorded ? "stroke-primary/60" : "stroke-muted-foreground/40"}
            />
          )
        })}
      </svg>
      {edges.map((e) => {
        const from = at.get(e.from)
        const to = at.get(e.to)
        if (!from || !to) return null
        if (e.label === "") return null
        const { mid } = curve(from, to)
        return (
          <span
            key={e.id}
            title={e.title}
            className="absolute max-w-[112px] -translate-x-1/2 -translate-y-1/2 truncate rounded-full border border-border bg-card px-1.5 py-px text-[10px] leading-4 text-muted-foreground"
            style={{ left: mid.x, top: mid.y }}
          >
            {e.label}
          </span>
        )
      })}
      <div role="list" aria-label="Lineage graph">
        {nodes.map((n) => (
          <div
            key={n.id}
            role="listitem"
            className={cn(
              "absolute flex flex-col justify-center gap-0.5 rounded-xl border bg-card px-3 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]",
              n.centre
                ? "border-primary bg-primary/10 ring-2 ring-primary/30"
                : n.recorded
                  ? "border-border"
                  : "border-dashed border-border"
            )}
            style={{ left: n.x, top: n.y, width: NODE_W, height: NODE_H }}
          >
            {n.group ? (
              <GroupToggle node={n} group={n.group} onToggle={onToggle} />
            ) : (
              <>
                <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                  {n.caption}
                </span>
                <NodeName node={n} />
              </>
            )}
          </div>
        ))}
      </div>
    </div>
  )
})

/** Room between the box's edge and the drawing, at any zoom. */
const BOX_PAD = 24
/** The tallest the box grows; a taller drawing opens fitted inside it. */
const BOX_CAP = 560
/** How far a pointer moves before a press becomes a drag (and stops being a click). */
const DRAG_SLOP = 4

/**
 * A box the drawing pans and zooms in (`BI-71` decision 3): drag (mouse or
 * one finger) moves it; Ctrl/⌘ + wheel, a trackpad pinch and a two-finger
 * pinch zoom around the pointer; `+`, `−` and Fit sit in the corner, and
 * `+`/`-`/`0` do the same while the box has focus. A plain wheel is left
 * to the page, so the box never traps its scroll.
 *
 * The view is `null` while the drawing is fitted, so it stays fitted as the
 * box resizes and is fitted again when the drawing changes size (a group
 * opened or closed). A button, a key or a fit eases over 200 ms; a drag,
 * pinch or wheel follows the hand with no easing.
 */
function PanZoomBox({ content, children }: { content: Size; children: React.ReactNode }) {
  // `full`: the overlay is mounted; `shown`: it is faded in. Closing fades
  // out first, then unmounts. The map inside is a fresh one each way, so it
  // opens fitted on open and on close.
  const [full, setFull] = React.useState(false)
  const [shown, setShown] = React.useState(false)
  const [held, setHeld] = React.useState(0)
  const slotRef = React.useRef<HTMLDivElement>(null)
  const buttonRef = React.useRef<HTMLButtonElement>(null)
  const overlayRef = React.useRef<HTMLDivElement>(null)
  const [toggled, setToggled] = React.useState(false)
  const timer = React.useRef<ReturnType<typeof setTimeout> | null>(null)
  React.useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current)
  }, [])

  const reduced = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false
  const open = () => {
    setToggled(true)
    setHeld(slotRef.current?.offsetHeight ?? 0)
    setFull(true)
  }
  const close = React.useCallback(() => {
    setToggled(true)
    setShown(false)
    if (reduced()) setFull(false)
    else timer.current = setTimeout(() => setFull(false), OVERLAY_MS)
  }, [])
  // Fade in on the frame after it mounts.
  React.useEffect(() => {
    if (!full) return
    const id = requestAnimationFrame(() => setShown(true))
    return () => cancelAnimationFrame(id)
  }, [full])
  // Page scroll is locked while the overlay is open; Esc or F closes it.
  React.useEffect(() => {
    if (!full) return
    const before = document.body.style.overflow
    document.body.style.overflow = "hidden"
    const onKey = (e: KeyboardEvent) => {
      // PR #642 review SHOULD-FIX 1: the overlay is modal, so Tab loops
      // inside it instead of reaching the page behind. The app's Dialog
      // primitive is a centred popup with its own Esc and focus return, which
      // would change what the owner passed in QA, so only the loop is kept here.
      if (e.key === "Tab" && !e.ctrlKey && !e.metaKey && !e.altKey) {
        const items = Array.from(
          overlayRef.current?.querySelectorAll<HTMLElement>(
            'a[href], button:not([disabled]), [tabindex]:not([tabindex="-1"])'
          ) ?? []
        )
        if (items.length === 0) return
        const first = items[0]
        const last = items[items.length - 1]
        const at = document.activeElement
        const inside = overlayRef.current?.contains(at) ?? false
        if (e.shiftKey && (!inside || at === first)) {
          e.preventDefault()
          last.focus()
        } else if (!e.shiftKey && (!inside || at === last)) {
          e.preventDefault()
          first.focus()
        }
        return
      }
      if (e.ctrlKey || e.metaKey || e.altKey) return
      if (e.key === "Escape" || e.key === "f" || e.key === "F") {
        e.preventDefault()
        close()
      }
    }
    document.addEventListener("keydown", onKey)
    return () => {
      document.body.style.overflow = before
      document.removeEventListener("keydown", onKey)
    }
  }, [full, close])

  const box = (
    <PanZoomInner
      key={full ? "full" : "inline"}
      content={content}
      full={full}
      onToggleFull={full ? close : open}
      buttonRef={buttonRef}
      focusButton={toggled}
    >
      {children}
    </PanZoomInner>
  )
  return (
    <>
      <div ref={slotRef} style={full ? { height: held } : undefined}>
        {!full && box}
      </div>
      {full &&
        createPortal(
          <div
            ref={overlayRef}
            role="dialog"
            aria-modal="true"
            aria-label="Lineage map, full screen"
            className={cn(
              "fixed inset-0 z-50 bg-background transition-opacity duration-200 ease-ui motion-reduce:transition-none",
              shown ? "opacity-100" : "opacity-0"
            )}
          >
            {box}
          </div>,
          document.body
        )}
    </>
  )
}

/** How long the full-screen overlay eases in and out. */
const OVERLAY_MS = 200

function PanZoomInner({
  content,
  children,
  full,
  onToggleFull,
  buttonRef,
  focusButton,
}: {
  content: Size
  children: React.ReactNode
  full: boolean
  onToggleFull: () => void
  buttonRef: React.RefObject<HTMLButtonElement | null>
  focusButton: boolean
}) {
  const boxRef = React.useRef<HTMLDivElement>(null)
  const [width, setWidth] = React.useState(0)
  const [fullHeight, setFullHeight] = React.useState(0)
  React.useEffect(() => {
    if (focusButton) buttonRef.current?.focus()
  }, [focusButton, buttonRef])
  const [chosen, setChosen] = React.useState<View | null>(null)
  const [eased, setEased] = React.useState(false)
  const [dragging, setDragging] = React.useState(false)

  React.useLayoutEffect(() => {
    if (!boxRef.current) return
    const r = boxRef.current.getBoundingClientRect()
    setWidth(r.width)
    setFullHeight(r.height)
  }, [])
  React.useEffect(() => {
    const el = boxRef.current
    if (!el) return
    const ro = new ResizeObserver((e) => {
      setWidth(e[0].contentRect.width)
      setFullHeight(e[0].contentRect.height)
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  // A drawing of a new size (a group opened or closed) is fitted again.
  const sizeKey = `${content.width}x${content.height}`
  const [fittedFor, setFittedFor] = React.useState(sizeKey)
  if (fittedFor !== sizeKey) {
    setFittedFor(sizeKey)
    setChosen(null)
    setEased(true)
  }

  const box = { width, height: full ? fullHeight : boxHeight(content, width, BOX_PAD, BOX_CAP) }
  const measured = width > 0 && box.height > 0
  const min = fitScale(content, box, BOX_PAD)
  const view: View = !measured
    ? { scale: 1, x: BOX_PAD, y: BOX_PAD }
    : chosen
      ? clampView(chosen, content, box, BOX_PAD)
      : fitView(content, box, BOX_PAD)

  // The handlers below run outside render; they read the latest of these,
  // written after each render commits.
  const live = React.useRef({ view, content, box })
  React.useLayoutEffect(() => {
    live.current = { view, content, box }
  })

  const move = React.useCallback((next: View, ease: boolean) => {
    // Several wheel or pinch events can arrive before a render commits; each
    // builds on the last one, not on the view last drawn.
    live.current = { ...live.current, view: next }
    setEased(ease)
    setChosen(next)
  }, [])
  const zoomBy = React.useCallback(
    (factor: number, point?: { x: number; y: number }, ease = false) => {
      const { view: v, content: c, box: b } = live.current
      move(zoomAt(v, factor, point ?? { x: b.width / 2, y: b.height / 2 }, c, b, BOX_PAD), ease)
    },
    [move]
  )
  const fit = () => {
    setEased(true)
    setChosen(null)
  }

  // Ctrl/⌘ + wheel (a trackpad pinch arrives as one too) and Safari's
  // gesture events. Not React handlers: both must be non-passive to stop
  // the page zooming instead.
  React.useEffect(() => {
    const el = boxRef.current
    if (!el) return
    const local = (e: { clientX: number; clientY: number }) => {
      const r = el.getBoundingClientRect()
      return { x: e.clientX - r.left, y: e.clientY - r.top }
    }
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return
      e.preventDefault()
      const lines = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1
      zoomBy(Math.exp(-e.deltaY * lines * 0.01), local(e))
    }
    type Gesture = Event & { scale: number; clientX: number; clientY: number }
    let start: View | null = null
    const onGestureStart = (e: Event) => {
      e.preventDefault()
      start = live.current.view
    }
    const onGestureChange = (e: Event) => {
      e.preventDefault()
      const g = e as Gesture
      if (!start) return
      const { content: c, box: b } = live.current
      move(zoomAt(start, g.scale, local(g), c, b, BOX_PAD), false)
    }
    el.addEventListener("wheel", onWheel, { passive: false })
    el.addEventListener("gesturestart", onGestureStart)
    el.addEventListener("gesturechange", onGestureChange)
    return () => {
      el.removeEventListener("wheel", onWheel)
      el.removeEventListener("gesturestart", onGestureStart)
      el.removeEventListener("gesturechange", onGestureChange)
    }
  }, [move, zoomBy])

  // Pointers: one drags, two pinch. A press becomes a drag only after
  // DRAG_SLOP pixels, so a click on a node still opens it; a drag that
  // ends over a node does not.
  const pointers = React.useRef(new Map<number, { x: number; y: number }>())
  const gesture = React.useRef<
    | { kind: "drag"; from: { x: number; y: number }; view: View; moved: boolean }
    | { kind: "pinch"; distance: number; mid: { x: number; y: number }; view: View }
    | null
  >(null)
  const swallowClick = React.useRef(false)

  const at = (e: React.PointerEvent) => {
    const r = e.currentTarget.getBoundingClientRect()
    return { x: e.clientX - r.left, y: e.clientY - r.top }
  }
  const pair = () => {
    const [a, b] = [...pointers.current.values()]
    return { distance: Math.hypot(a.x - b.x, a.y - b.y) || 1, mid: { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 } }
  }
  const startDrag = (from: { x: number; y: number }, moved: boolean) => {
    gesture.current = { kind: "drag", from, view: live.current.view, moved }
  }

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.pointerType === "mouse" && e.button !== 0) return
    swallowClick.current = false
    pointers.current.set(e.pointerId, at(e))
    if (pointers.current.size === 1) startDrag(at(e), false)
    else if (pointers.current.size === 2) {
      gesture.current = { kind: "pinch", ...pair(), view: live.current.view }
      swallowClick.current = true
    }
  }
  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!pointers.current.has(e.pointerId)) return
    pointers.current.set(e.pointerId, at(e))
    const g = gesture.current
    if (!g) return
    const { content: c, box: b } = live.current
    if (g.kind === "pinch" && pointers.current.size >= 2) {
      const { distance, mid } = pair()
      const zoomed = zoomAt(g.view, distance / g.distance, g.mid, c, b, BOX_PAD)
      move(clampView({ ...zoomed, x: zoomed.x + mid.x - g.mid.x, y: zoomed.y + mid.y - g.mid.y }, c, b, BOX_PAD), false)
      return
    }
    if (g.kind !== "drag") return
    const p = at(e)
    const dx = p.x - g.from.x
    const dy = p.y - g.from.y
    if (!g.moved) {
      if (Math.hypot(dx, dy) < DRAG_SLOP) return
      g.moved = true
      swallowClick.current = true
      setDragging(true)
      e.currentTarget.setPointerCapture?.(e.pointerId)
    }
    move(clampView({ ...g.view, x: g.view.x + dx, y: g.view.y + dy }, c, b, BOX_PAD), false)
  }
  const onPointerEnd = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!pointers.current.delete(e.pointerId)) return
    const [left] = [...pointers.current.values()]
    // One finger lifted from a pinch: the other carries on as a drag.
    if (left) startDrag(left, true)
    else {
      gesture.current = null
      setDragging(false)
    }
  }

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.ctrlKey || e.metaKey || e.altKey) return
    if (e.key === "+" || e.key === "=") zoomBy(ZOOM_STEP, undefined, true)
    else if (e.key === "-" || e.key === "_") zoomBy(1 / ZOOM_STEP, undefined, true)
    else if (e.key === "0") fit()
    else if ((e.key === "f" || e.key === "F") && !full) onToggleFull()
    else return
    e.preventDefault()
  }

  const fitted = chosen === null
  return (
    <div
      ref={boxRef}
      role="region"
      aria-label="Lineage map"
      aria-keyshortcuts="+ - 0 F"
      tabIndex={0}
      data-zoom={view.scale.toFixed(3)}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerEnd}
      onPointerCancel={onPointerEnd}
      onClickCapture={(e) => {
        if (!swallowClick.current) return
        swallowClick.current = false
        e.preventDefault()
        e.stopPropagation()
      }}
      onKeyDown={onKeyDown}
      className={cn(
        "relative touch-none overflow-hidden bg-muted/20 outline-none select-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
        full ? "bg-background" : "rounded-lg border border-border",
        dragging ? "cursor-grabbing" : "cursor-grab"
      )}
      style={{ height: full ? "100%" : box.height }}
    >
      <div
        data-slot="map-drawing"
        className={cn("absolute top-0 left-0 origin-top-left", eased && "transition-transform duration-200 ease-ui")}
        style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})` }}
      >
        {children}
      </div>
      <div
        className={cn(
          "absolute right-2 flex items-center rounded-lg bg-background p-0.5 shadow-[0_1px_2px_rgb(0_0_0/0.06)] ring-1 ring-border",
          full ? "top-[max(0.5rem,env(safe-area-inset-top))] right-[max(0.5rem,env(safe-area-inset-right))]" : "top-2"
        )}
        onPointerDown={(e) => e.stopPropagation()}
      >
        <MapButton label="Zoom in" keys="+" disabled={!measured || view.scale >= ZOOM_MAX} onClick={() => zoomBy(ZOOM_STEP, undefined, true)}>
          <Plus />
        </MapButton>
        <MapButton label="Zoom out" keys="−" disabled={!measured || view.scale <= min + 1e-6} onClick={() => zoomBy(1 / ZOOM_STEP, undefined, true)}>
          <Minus />
        </MapButton>
        <MapButton label="Fit" keys="0" disabled={!measured || fitted} onClick={fit}>
          <Scan />
        </MapButton>
        <MapButton buttonRef={buttonRef} label={full ? "Exit full screen" : "Full screen"} keys="F" disabled={false} onClick={onToggleFull}>
          {full ? <Minimize2 /> : <Maximize2 />}
        </MapButton>
      </div>
    </div>
  )
}

/** One of the map's corner buttons: an icon, its name and shortcut in the tooltip. */
function MapButton({
  label,
  keys,
  disabled,
  onClick,
  buttonRef,
  children,
}: {
  label: string
  keys: string
  disabled: boolean
  onClick: () => void
  buttonRef?: React.Ref<HTMLButtonElement>
  children: React.ReactNode
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button ref={buttonRef} variant="ghost" size="icon" aria-label={label} disabled={disabled} onClick={onClick}>
            {children}
          </Button>
        }
      />
      <TooltipContent>
        {label} <span className="text-background/60">{keys}</span>
      </TooltipContent>
    </Tooltip>
  )
}

/** A node's name: a link to its page when it has one. The full name is in the tooltip. */
function NodeName({ node: n }: { node: MapNode }) {
  return n.href ? (
    <Link
      href={n.href}
      draggable={false}
      className="truncate text-sm font-medium leading-5 text-foreground hover:text-primary"
      title={n.label}
    >
      {shortLabel(n)}
    </Link>
  ) : (
    <span className="truncate text-sm font-medium leading-5" title={n.label}>
      {shortLabel(n)}
    </span>
  )
}

/**
 * A dashboard's charts as one node: the whole node is the button that
 * opens them to its right, and closes them again. The chevron turns to say
 * which way it goes.
 */
function GroupToggle({
  node: n,
  group,
  onToggle,
}: {
  node: MapNode
  group: NonNullable<MapNode["group"]>
  onToggle: (key: string) => void
}) {
  const Icon = group.key === OTHER ? ChartColumn : BarChart3
  return (
    <button
      type="button"
      aria-expanded={group.open}
      aria-label={n.label}
      title={n.label}
      onClick={() => onToggle(group.key)}
      className="-mx-3 flex h-full min-w-0 items-center gap-2 rounded-[inherit] px-3 text-left outline-none transition-colors duration-150 ease-ui hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span aria-hidden className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="inline-flex items-center gap-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
          <Icon className="size-3" />
          {n.caption}
        </span>
        {/* The count never truncates: it is what the group is for. */}
        <span className="flex min-w-0 text-sm font-medium leading-5">
          <span className="truncate">{group.title}</span>
          <span className="shrink-0 whitespace-pre text-muted-foreground">{` · ${chartsWord(group.count)}`}</span>
        </span>
      </span>
      <ChevronRight
        aria-hidden
        className={cn(
          "size-4 shrink-0 text-muted-foreground transition-transform duration-200 ease-ui",
          group.open && "rotate-180"
        )}
      />
    </button>
  )
}
