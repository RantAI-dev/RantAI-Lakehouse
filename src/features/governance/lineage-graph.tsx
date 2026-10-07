"use client"

import * as React from "react"
import Link from "next/link"
import { ChevronRight, ExternalLink } from "lucide-react"
import { cn } from "@/lib/utils"
import type { LineageGraph } from "@/services/contracts/governance"

export type LineageNode = LineageGraph["nodes"][number]

/** What a node is, as its small caption. */
export const KIND_LABEL: Record<string, string> = {
  connector: "Connector",
  source: "Publisher",
  dataset: "Dataset",
  bronze: "Bronze",
  silver: "Silver",
  gold: "Gold",
  iceberg: "Iceberg",
  pipeline: "Pipeline",
}

/** An id's kind prefix and the rest: `table:silver.orders` is `["table", "silver.orders"]`. */
function idParts(id: string): [string, string] {
  const at = id.indexOf(":")
  return at < 0 ? [id, ""] : [id.slice(0, at), id.slice(at + 1)]
}

/** The nodes the graph was asked about: the API marks them `kind: "focus"`. */
export function focusIds(graph: LineageGraph): Set<string> {
  return new Set(graph.nodes.filter((n) => n.kind === "focus").map((n) => n.id))
}

/**
 * What a node is. A focus node's own kind is replaced by `"focus"`, so it
 * is read back from the id, whose prefix the API sets per kind
 * (`routes/lineage.rs`): `connector:`, `bronze:`, `dataset:`, `source:`,
 * `iceberg:`, and `table:<database>.<table>`.
 */
export function nodeKind(node: LineageNode): string {
  if (node.kind !== "focus") return node.kind
  const [prefix, rest] = idParts(node.id)
  if (prefix !== "table") return prefix
  if (rest.startsWith("silver.")) return "silver"
  if (rest.startsWith("serving.")) return "gold"
  return "table"
}

/**
 * The page a node opens, when its id names one. A Bronze table has none
 * here: its catalog page is addressed by a registry slug the graph does
 * not carry.
 */
export function nodeHref(node: LineageNode): string | null {
  const [prefix, rest] = idParts(node.id)
  if (!rest) return null
  const ref = encodeURIComponent(rest)
  switch (prefix) {
    case "connector":
      return `/connectors/${ref}`
    case "dataset":
      return `/data/assets/${ref}`
    case "table":
      return rest.startsWith("silver.") || rest.startsWith("serving.") ? `/data/assets/${ref}` : null
    default:
      return null
  }
}

/**
 * Each node's column: its longest path from a node with nothing upstream,
 * so every arrow points right. A node on a cycle goes one column past the
 * rest.
 */
export function nodeDepths(graph: LineageGraph): Map<string, number> {
  const ids = new Set(graph.nodes.map((n) => n.id))
  const edges = graph.edges.filter((e) => ids.has(e.from) && ids.has(e.to))
  const waiting = new Map<string, number>([...ids].map((id) => [id, 0]))
  for (const e of edges) waiting.set(e.to, (waiting.get(e.to) ?? 0) + 1)
  const depth = new Map<string, number>()
  const queue = [...ids].filter((id) => waiting.get(id) === 0)
  for (const id of queue) depth.set(id, 0)
  while (queue.length > 0) {
    const id = queue.shift()!
    const here = depth.get(id) ?? 0
    for (const e of edges) {
      if (e.from !== id) continue
      depth.set(e.to, Math.max(depth.get(e.to) ?? 0, here + 1))
      const left = (waiting.get(e.to) ?? 1) - 1
      waiting.set(e.to, left)
      if (left === 0) queue.push(e.to)
    }
  }
  const past = depth.size === 0 ? 0 : Math.max(...depth.values()) + 1
  for (const id of ids) if (!depth.has(id)) depth.set(id, past)
  return depth
}

/**
 * The graph as columns, left to right ([`nodeDepths`]). Branches stack
 * within a column.
 *
 * With `onTrace` (the Lineage page), a node's name re-focuses the graph on
 * it and a corner icon opens its page. Without (an asset's Lineage tab),
 * the name itself opens the page.
 */
export function LineageColumns({
  graph,
  onTrace,
}: {
  readonly graph: LineageGraph
  readonly onTrace?: (id: string) => void
}) {
  const focus = focusIds(graph)
  const depths = nodeDepths(graph)
  const columns: LineageNode[][] = []
  for (const node of graph.nodes) {
    const depth = depths.get(node.id) ?? 0
    const column = columns[depth] ?? []
    column.push(node)
    columns[depth] = column
  }
  const filled = columns.filter((c) => c && c.length > 0)
  return (
    <div
      className="flex items-stretch gap-2 overflow-x-auto rounded-lg border border-border bg-muted/20 p-4"
      role="list"
      aria-label="Lineage graph"
    >
      {filled.map((column, i) => (
        <React.Fragment key={column[0].id}>
          <div className="flex shrink-0 flex-col justify-center gap-2">
            {column.map((node) => {
              const href = nodeHref(node)
              const kind = nodeKind(node)
              return (
                <div
                  key={node.id}
                  role="listitem"
                  className={cn(
                    "flex w-52 flex-col gap-1 rounded-lg border border-border bg-card px-3 py-2.5 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]",
                    focus.has(node.id) && "border-primary ring-1 ring-primary/30"
                  )}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                      {KIND_LABEL[kind] ?? kind}
                    </span>
                    {href && onTrace ? (
                      <Link
                        href={href}
                        className="text-muted-foreground hover:text-foreground"
                        aria-label={`Open ${node.label}`}
                      >
                        <ExternalLink className="size-3.5" />
                      </Link>
                    ) : null}
                  </div>
                  {onTrace ? (
                    <button
                      type="button"
                      onClick={() => onTrace(node.id)}
                      className="truncate text-left text-sm font-medium leading-5 text-foreground hover:text-primary"
                      title={`Trace lineage from ${node.label}`}
                    >
                      {node.label}
                    </button>
                  ) : href && !focus.has(node.id) ? (
                    <Link
                      href={href}
                      className="truncate text-sm font-medium leading-5 text-foreground hover:text-primary"
                      title={node.label}
                    >
                      {node.label}
                    </Link>
                  ) : (
                    <span className="truncate text-sm font-medium leading-5" title={node.label}>
                      {node.label}
                    </span>
                  )}
                </div>
              )
            })}
          </div>
          {i < filled.length - 1 ? (
            <ChevronRight className="size-4 shrink-0 self-center text-muted-foreground" aria-hidden />
          ) : null}
        </React.Fragment>
      ))}
    </div>
  )
}
