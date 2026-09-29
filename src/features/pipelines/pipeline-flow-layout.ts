/**
 * The pipeline flowchart's graph and layout, kept free of React so it is
 * tested on its own.
 *
 * A Dagster job is drawn as what its ops read, then the ops in dependency
 * order, then what they write. The reads and writes are what each op
 * declares in its own code (`op_metadata.source_metadata`), so the chart
 * labels them "declared". An authored pipeline is drawn as its stored
 * definition: source table, the transforms in order, target table. The
 * layout is columns left to right; nodes that share a column are stacked,
 * and an edge inside one column (a transform chain) runs top to bottom.
 */
import type { EntityStatus } from "@/lib/status"
import type { AuthoredDefinition, PipelineOpEdge, PipelineOpNode } from "@/services/contracts/pipelines"
import { topoSortOps } from "./topo-sort-ops"

export type FlowKind = "source" | "op" | "transform" | "sink"

export type FlowNodeSpec = {
  id: string
  column: number
  kind: FlowKind
  label: string
  sublabel?: string
  status?: EntityStatus
  /** For an op, its name; for a transform, its index. Used to open the side panel. */
  ref?: string
}

export type FlowEdgeSpec = { from: string; to: string }

export type PositionedNode = FlowNodeSpec & { x: number; y: number; w: number; h: number }

export type FlowLayout = {
  nodes: PositionedNode[]
  edges: { from: PositionedNode; to: PositionedNode }[]
  width: number
  height: number
}

const SIZE: Record<FlowKind, { w: number; h: number }> = {
  source: { w: 236, h: 58 },
  sink: { w: 236, h: 58 },
  op: { w: 252, h: 86 },
  transform: { w: 228, h: 58 },
}

const COL_GAP = 76
const ROW_GAP = 16
const PAD = 12

/** Places `nodes` in their columns, each column centred on the tallest one. */
export function layoutFlow(nodes: readonly FlowNodeSpec[], edges: readonly FlowEdgeSpec[]): FlowLayout {
  const columns = [...new Set(nodes.map((n) => n.column))].sort((a, b) => a - b)
  const byColumn = columns.map((c) => nodes.filter((n) => n.column === c))
  const colWidth = byColumn.map((col) => Math.max(...col.map((n) => SIZE[n.kind].w)))
  const colHeight = byColumn.map(
    (col) => col.reduce((sum, n) => sum + SIZE[n.kind].h, 0) + ROW_GAP * (col.length - 1)
  )
  const height = Math.max(0, ...colHeight) + PAD * 2

  const placed: PositionedNode[] = []
  let x = PAD
  byColumn.forEach((col, i) => {
    let y = PAD + (height - PAD * 2 - colHeight[i]) / 2
    for (const n of col) {
      const { w, h } = SIZE[n.kind]
      placed.push({ ...n, x: x + (colWidth[i] - w) / 2, y, w, h })
      y += h + ROW_GAP
    }
    x += colWidth[i] + COL_GAP
  })
  const width = x - COL_GAP + PAD

  const byId = new Map(placed.map((n) => [n.id, n]))
  const positionedEdges = edges.flatMap((e) => {
    const from = byId.get(e.from)
    const to = byId.get(e.to)
    return from && to ? [{ from, to }] : []
  })
  return { nodes: placed, edges: positionedEdges, width: Math.max(width, 0), height }
}

/** SVG path for one edge: a horizontal S-curve between columns, a straight drop inside one. */
export function edgePath(from: PositionedNode, to: PositionedNode): string {
  if (from.column === to.column) {
    const x = from.x + from.w / 2
    return `M ${x} ${from.y + from.h} L ${x} ${to.y}`
  }
  const x1 = from.x + from.w
  const y1 = from.y + from.h / 2
  const x2 = to.x
  const y2 = to.y + to.h / 2
  const mid = (x1 + x2) / 2
  return `M ${x1} ${y1} C ${mid} ${y1}, ${mid} ${y2}, ${x2} ${y2}`
}

/**
 * A Dagster job: declared reads, then ops by dependency depth, then
 * declared writes. A phrase two ops share is one node with two edges.
 */
export function dagsterFlow(
  ops: readonly PipelineOpNode[],
  opEdges: readonly PipelineOpEdge[],
  stepStatus: (op: string) => EntityStatus | undefined
): { nodes: FlowNodeSpec[]; edges: FlowEdgeSpec[] } {
  const sorted = topoSortOps([...ops], [...opEdges])
  const depth = new Map<string, number>()
  for (const { node, upstream } of sorted) {
    depth.set(node.name, upstream.length === 0 ? 0 : Math.max(...upstream.map((u) => (depth.get(u) ?? 0) + 1)))
  }
  const hasReads = ops.some((op) => (op.reads ?? []).length > 0)
  const opBase = hasReads ? 1 : 0
  const maxDepth = Math.max(0, ...depth.values())

  const nodes: FlowNodeSpec[] = []
  const edges: FlowEdgeSpec[] = []
  const seen = new Set<string>()
  const dataNode = (kind: "source" | "sink", phrase: string, column: number) => {
    const id = `${kind}:${phrase}`
    if (!seen.has(id)) {
      seen.add(id)
      nodes.push({ id, column, kind, label: phrase })
    }
    return id
  }

  for (const { node } of sorted) {
    const id = `op:${node.name}`
    nodes.push({
      id,
      column: opBase + (depth.get(node.name) ?? 0),
      kind: "op",
      label: node.name,
      sublabel: node.sourceRef ?? undefined,
      status: stepStatus(node.name),
      ref: node.name,
    })
    for (const phrase of node.reads ?? []) edges.push({ from: dataNode("source", phrase, 0), to: id })
    for (const phrase of node.writes ?? []) {
      edges.push({ from: id, to: dataNode("sink", phrase, opBase + maxDepth + 1) })
    }
  }
  for (const e of opEdges) edges.push({ from: `op:${e.from}`, to: `op:${e.to}` })
  return { nodes, edges }
}

/** An authored pipeline: source table, each transform in order (one column, top to bottom), target table. */
export function authoredFlow(
  def: AuthoredDefinition,
  describe: (raw: string) => { verb: string; detail: string } | null
): { nodes: FlowNodeSpec[]; edges: FlowEdgeSpec[] } {
  const source: FlowNodeSpec = {
    id: "source",
    column: 0,
    kind: "source",
    label: `${def.sourceZone}.${def.sourceTable}`,
    sublabel: def.incrementalColumn ? `incremental on ${def.incrementalColumn}` : "full read",
  }
  const transforms: FlowNodeSpec[] = def.transforms.map((raw, i) => {
    const t = describe(raw)
    return {
      id: `t${i}`,
      column: 1,
      kind: "transform",
      label: t ? t.verb : "transform",
      sublabel: t ? t.detail : raw,
      ref: String(i),
    }
  })
  const target: FlowNodeSpec = {
    id: "target",
    column: transforms.length > 0 ? 2 : 1,
    kind: "sink",
    label: `${def.targetZone}.${def.targetTable}`,
  }
  const chain = [source, ...transforms, target]
  const edges = chain.slice(1).map((n, i) => ({ from: chain[i].id, to: n.id }))
  return { nodes: chain, edges }
}
