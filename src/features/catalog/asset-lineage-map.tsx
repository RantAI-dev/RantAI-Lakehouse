"use client"

import Link from "next/link"
import { KIND_LABEL, focusIds, nodeHref, nodeKind } from "@/features/governance/lineage-graph"
import { cn } from "@/lib/utils"
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
 */
export function LineageMindmap({ model }: { model: { nodes: MapNode[]; edges: MapEdge[] } }) {
  const { nodes, width, height } = mindmapLayout(model)
  const at = new Map(nodes.map((n) => [n.id, n]))
  const centre = nodes.find((n) => n.centre)
  const dashed = model.edges.some((e) => !e.recorded)

  return (
    <div className="flex flex-col gap-2">
      <div className="overflow-x-auto rounded-lg border border-border bg-muted/20 p-6">
        <div className="relative mx-auto" style={{ width, height }}>
          <svg
            className="pointer-events-none absolute inset-0 overflow-visible"
            width={width}
            height={height}
            aria-hidden
          >
            {model.edges.map((e) => {
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
          {model.edges.map((e) => {
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
                <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                  {n.caption}
                </span>
                {n.href ? (
                  <Link
                    href={n.href}
                    className="truncate text-sm font-medium leading-5 text-foreground hover:text-primary"
                    title={n.label}
                  >
                    {shortLabel(n)}
                  </Link>
                ) : (
                  <span className="truncate text-sm font-medium leading-5" title={n.label}>
                    {shortLabel(n)}
                  </span>
                )}
              </div>
            ))}
          </div>
        </div>
      </div>
      <p className="text-xs text-muted-foreground">
        This asset is in the middle: what it is built from is to its left, what is built from it to
        its right.
        {dashed ? " Dashed branches are not recorded lineage: they are what reads it, and tables the catalog ties to it." : ""}
      </p>
    </div>
  )
}
