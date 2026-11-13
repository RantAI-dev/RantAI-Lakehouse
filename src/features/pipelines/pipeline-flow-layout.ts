/**
 * The pipeline flowchart's graph and layout, kept free of React so it is
 * tested on its own.
 *
 * A Dagster job is drawn as what its ops read, then the ops in dependency
 * order, then what they write. The reads and writes are what each op
 * declares in its own code (`op_metadata.source_metadata`), so the chart
 * labels them "declared". An authored pipeline is drawn as its stored
 * definition (BLD-1 part 1 D17): its inputs, its steps, its output table.
 * The layout is columns left to right; nodes that share a column are
 * stacked, and an edge inside one column runs top to bottom.
 */
import { sqlStepReads } from "@/lib/pipeline-editor"
import type { EntityStatus } from "@/lib/status"
import type { AuthoredDefinition, PipelineOpEdge, PipelineOpNode } from "@/services/contracts/pipelines"
import { topoSortOps } from "./topo-sort-ops"

export type FlowKind = "source" | "op" | "transform" | "step" | "sink"

export type FlowNodeSpec = {
  id: string
  column: number
  kind: FlowKind
  label: string
  sublabel?: string
  status?: EntityStatus
  /** For an op, its name; for a transform, an input or a step, its index. Used to open the side panel. */
  ref?: string
  /** For a pipeline step: whether it is SQL or the form (BLD-1 part 1 D1). */
  stepKind?: "sql" | "transform"
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
  source: { w: 208, h: 58 },
  sink: { w: 208, h: 58 },
  op: { w: 224, h: 86 },
  transform: { w: 216, h: 58 },
  step: { w: 216, h: 58 },
}

/** Sized so a two-op job (read, op, op, write) fits the content width of a 1440 px screen. */
const COL_GAP = 56
const ROW_GAP = 16
const PAD = 12

/** Places `nodes` in their columns, each column centred on the tallest one. */
export function layoutFlow(nodes: readonly FlowNodeSpec[], edges: readonly FlowEdgeSpec[]): FlowLayout {
  const columns = [...new Set(nodes.map((n) => n.column))].sort((a, b) => a - b)
  // Within a column, ops and transforms first, then data, so a job's
  // spine of ops lines up and its data hangs below.
  const rank = (n: FlowNodeSpec) => (n.kind === "op" || n.kind === "transform" || n.kind === "step" ? 0 : 1)
  const byColumn = columns.map((c) =>
    nodes.filter((n) => n.column === c).sort((a, b) => rank(a) - rank(b))
  )
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
 * A Dagster job: ops by dependency depth, each op's declared reads in the
 * column just before it and its declared writes in the column just after,
 * so no edge has to cross another op to reach its data. A phrase two ops
 * share is one node, placed where it first appears, with two edges.
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
  // Column 0 is kept for reads only when an entry op declares some.
  const opBase = sorted.some(({ node, upstream }) => upstream.length === 0 && (node.reads ?? []).length > 0) ? 1 : 0

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
    const column = opBase + (depth.get(node.name) ?? 0)
    nodes.push({
      id,
      column,
      kind: "op",
      label: node.name,
      sublabel: node.sourceRef ?? undefined,
      status: stepStatus(node.name),
      ref: node.name,
    })
    for (const phrase of node.reads ?? []) edges.push({ from: dataNode("source", phrase, Math.max(0, column - 1)), to: id })
    for (const phrase of node.writes ?? []) edges.push({ from: id, to: dataNode("sink", phrase, column + 1) })
  }
  for (const e of opEdges) edges.push({ from: `op:${e.from}`, to: `op:${e.to}` })
  return { nodes, edges }
}

/**
 * An authored pipeline (BLD-1 part 1 D17): its inputs in the first column,
 * each step one column right of the right-most thing it reads, and the
 * output table right of the last step, which is what it writes.
 *
 * An edge says what a step reads: a form step's `reads`, and a SQL step's
 * names after `FROM` and `JOIN` (`sqlStepReads`, a drawing aid that leaves
 * out what it cannot place rather than guess). With no step the output is a
 * copy of the one input, and the chart says so (BLD-10 D3).
 */
export function authoredFlow(
  def: AuthoredDefinition,
  describe: (raw: string) => { verb: string; detail: string } | null
): { nodes: FlowNodeSpec[]; edges: FlowEdgeSpec[] } {
  const nodes: FlowNodeSpec[] = []
  const edges: FlowEdgeSpec[] = []
  /** Each name's node and column, as the definition reaches it. */
  const placed = new Map<string, { id: string; column: number }>()
  def.inputs.forEach((input, i) => {
    const id = `input:${i}`
    nodes.push({
      id,
      column: 0,
      kind: "source",
      label: `${input.zone}.${input.table}`,
      sublabel: input.name !== input.table ? `read as ${input.name}` : undefined,
      ref: String(i),
    })
    placed.set(input.name, { id, column: 0 })
  })
  def.steps.forEach((step, i) => {
    const id = `step:${i}`
    const known = [...placed.keys()]
    const reads =
      step.kind === "transform" ? (known.includes(step.reads) ? [step.reads] : []) : sqlStepReads(step.sql, known)
    const column = 1 + Math.max(0, ...reads.map((name) => placed.get(name)?.column ?? 0))
    const operations = step.kind === "transform" ? step.transforms.map((raw) => describe(raw)?.verb ?? "transform") : []
    nodes.push({
      id,
      column,
      kind: "step",
      label: step.name,
      sublabel:
        step.kind === "sql"
          ? "SQL"
          : operations.length > 0
            ? operations.join(", ")
            : "no operation",
      ref: String(i),
      stepKind: step.kind,
    })
    for (const name of reads) {
      const from = placed.get(name)
      if (from) edges.push({ from: from.id, to: id })
    }
    placed.set(step.name, { id, column })
  })
  // The output is what the last step returns; with no step, the one input.
  const last = def.steps.length > 0 ? nodes[nodes.length - 1] : null
  const writer = last ?? (def.inputs.length === 1 ? nodes[0] : null)
  const target: FlowNodeSpec = {
    id: "target",
    column: (writer?.column ?? 0) + 1,
    kind: "sink",
    label: `${def.targetZone}.${def.targetTable}`,
    // BLD-10 D3: a pipeline with no step copies its input as is. The chart
    // says so, rather than leaving a reader to guess what an empty middle
    // means.
    sublabel: last === null ? "plain copy of the source" : undefined,
  }
  nodes.push(target)
  if (writer) edges.push({ from: writer.id, to: target.id })
  return { nodes, edges }
}
