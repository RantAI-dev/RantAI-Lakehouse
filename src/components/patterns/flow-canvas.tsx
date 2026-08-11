"use client"

import { ChevronRight } from "lucide-react"
import { cn } from "@/lib/utils"
import type { EntityStatus } from "@/lib/status"
import { StatusBadge } from "./status-badge"

export type FlowNode = {
  id: string
  label: string
  sublabel?: string
  status?: EntityStatus
  kind?: string
}

/**
 * Left-to-right flow renderer for pipelines, routing, and workflows.
 * Nodes render in sequence with arrow connectors; failed or degraded nodes
 * carry their status badge (never color alone). For long lifecycles or
 * accessibility-first inspection, pair with a table/list alternative.
 */
export function FlowCanvas({
  nodes,
  selectedId,
  onSelect,
  className,
}: {
  nodes: FlowNode[]
  selectedId?: string
  onSelect?: (node: FlowNode) => void
  className?: string
}) {
  return (
    <div
      className={cn(
        "flex flex-wrap items-stretch gap-2 rounded-lg border border-border bg-muted/20 p-4",
        className
      )}
      role="list"
      aria-label="Flow steps"
    >
      {nodes.map((node, i) => (
        <div key={node.id} className="flex items-center gap-2" role="listitem">
          <button
            type="button"
            onClick={onSelect ? () => onSelect(node) : undefined}
            className={cn(
              "flex min-w-36 flex-col items-start gap-1 rounded-lg border border-border bg-card px-3 py-2.5 text-left shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)] transition-colors",
              onSelect && "cursor-pointer hover:border-primary/40",
              !onSelect && "cursor-default",
              selectedId === node.id && "border-primary ring-1 ring-primary/30"
            )}
          >
            {node.kind ? (
              <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                {node.kind}
              </span>
            ) : null}
            <span className="text-sm font-medium leading-5 text-foreground">
              {node.label}
            </span>
            {node.sublabel ? (
              <span className="max-w-44 truncate text-xs text-muted-foreground">
                {node.sublabel}
              </span>
            ) : null}
            {node.status ? <StatusBadge status={node.status} /> : null}
          </button>
          {i < nodes.length - 1 ? (
            <ChevronRight
              className="size-4 shrink-0 text-muted-foreground"
              aria-hidden
            />
          ) : null}
        </div>
      ))}
    </div>
  )
}
