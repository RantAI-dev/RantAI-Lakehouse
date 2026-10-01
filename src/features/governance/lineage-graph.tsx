"use client"

import * as React from "react"
import Link from "next/link"
import { ChevronRight, ExternalLink } from "lucide-react"
import { StatusBadge } from "@/components/patterns/status-badge"
import { ENTITY_STATUS_LABEL, type EntityStatus } from "@/lib/status"
import { cn } from "@/lib/utils"
import type { LineageGraph, LineageNode } from "@/services/contracts/governance"

/** What a node is, as its small caption. */
export const KIND_LABEL: Record<string, string> = {
  source: "Source",
  bronze: "Bronze",
  silver: "Silver",
  serving: "Serving (gold)",
  pipeline: "Pipeline",
}

/** The page a node opens, when it has one. */
export function nodeHref(node: LineageNode): string | null {
  if (!node.ref) return null
  const ref = encodeURIComponent(node.ref)
  switch (node.kind) {
    case "pipeline":
      return `/pipelines/${ref}`
    case "source":
      return `/connectors/${ref}/edit`
    case "bronze":
    case "silver":
    case "serving":
      return `/data/assets/${ref}`
    default:
      return null
  }
}

function isEntityStatus(value: string): value is EntityStatus {
  return value in ENTITY_STATUS_LABEL
}

/**
 * The graph as columns, left to right: a node's column is its longest path
 * from something with nothing upstream (`depth`, from the API), so every
 * arrow points right. Branches stack within a column.
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
  const focusIds = new Set(graph.focusIds ?? [])
  const columns: LineageNode[][] = []
  for (const node of graph.nodes) {
    const depth = node.depth ?? 0
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
              return (
                <div
                  key={node.id}
                  role="listitem"
                  className={cn(
                    "flex w-52 flex-col gap-1 rounded-lg border border-border bg-card px-3 py-2.5 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]",
                    focusIds.has(node.id) && "border-primary ring-1 ring-primary/30"
                  )}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                      {KIND_LABEL[node.kind] ?? node.kind}
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
                  ) : href && !focusIds.has(node.id) ? (
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
                  {node.kind === "pipeline" && node.sublabel && isEntityStatus(node.sublabel) ? (
                    <StatusBadge status={node.sublabel} className="self-start" />
                  ) : node.sublabel ? (
                    <span className="truncate text-xs text-muted-foreground">{node.sublabel}</span>
                  ) : null}
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
