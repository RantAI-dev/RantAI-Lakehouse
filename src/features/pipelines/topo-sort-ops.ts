import type { PipelineOpEdge, PipelineOpNode } from "@/services/contracts/pipelines"

export type SortedOp = { node: PipelineOpNode; upstream: string[] }

/**
 * Kahn's algorithm: order `ops` so every node appears after all of its
 * upstream dependencies (`edges[].from` runs before `edges[].to`). A
 * cycle — which a real Dagster job graph can never contain, since Dagster
 * itself refuses to load one — falls back to declaration order with a
 * `console.warn` rather than throwing: a hypothetical malformed response
 * must degrade to "an unordered but still-real list of ops", never crash
 * the page (WS4 item F3).
 */
export function topoSortOps(ops: PipelineOpNode[], edges: PipelineOpEdge[]): SortedOp[] {
  const upstreamOf = new Map<string, string[]>()
  const inDegree = new Map<string, number>()
  for (const op of ops) {
    upstreamOf.set(op.name, [])
    inDegree.set(op.name, 0)
  }
  const outgoing = new Map<string, string[]>()
  for (const edge of edges) {
    if (!upstreamOf.has(edge.to) || !upstreamOf.has(edge.from)) continue
    upstreamOf.get(edge.to)!.push(edge.from)
    inDegree.set(edge.to, (inDegree.get(edge.to) ?? 0) + 1)
    if (!outgoing.has(edge.from)) outgoing.set(edge.from, [])
    outgoing.get(edge.from)!.push(edge.to)
  }

  const byName = new Map(ops.map((op) => [op.name, op]))
  const remaining = new Map(inDegree)
  const queue = ops.filter((op) => (inDegree.get(op.name) ?? 0) === 0).map((op) => op.name)
  const order: string[] = []

  while (queue.length > 0) {
    const name = queue.shift()!
    order.push(name)
    for (const next of outgoing.get(name) ?? []) {
      const left = (remaining.get(next) ?? 0) - 1
      remaining.set(next, left)
      if (left === 0) queue.push(next)
    }
  }

  if (order.length !== ops.length) {
    console.warn("topoSortOps: cycle detected in pipeline op graph; falling back to declaration order")
    return ops.map((op) => ({ node: op, upstream: upstreamOf.get(op.name) ?? [] }))
  }

  return order.map((name) => ({ node: byName.get(name)!, upstream: upstreamOf.get(name) ?? [] }))
}
