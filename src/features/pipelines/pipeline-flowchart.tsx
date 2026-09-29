"use client"

import * as React from "react"
import { useReducedMotion } from "motion/react"
import {
  BoxIcon,
  CopyXIcon,
  DatabaseIcon,
  FilterIcon,
  GlobeIcon,
  HardDriveIcon,
  LayersIcon,
  ListChecksIcon,
  PencilLineIcon,
  ReplaceIcon,
  SlidersHorizontalIcon,
} from "lucide-react"
import { formatDuration } from "@/lib/format"
import { cn } from "@/lib/utils"
import { edgePath, layoutFlow, type FlowEdgeSpec, type FlowNodeSpec, type PositionedNode } from "./pipeline-flow-layout"
import { runFill, statusLabel } from "./run-history-strip"

/** The kind of store a declared phrase names, read from its first word. */
function DataIcon({ label }: { label: string }) {
  const cls = "size-4"
  if (/^Iceberg/i.test(label)) return <LayersIcon className={cls} aria-hidden />
  if (/^(ClickHouse|Postgres)/i.test(label)) return <DatabaseIcon className={cls} aria-hidden />
  if (/^(GET|POST|PUT|DELETE) /.test(label)) return <GlobeIcon className={cls} aria-hidden />
  if (/^RustFS/i.test(label)) return <HardDriveIcon className={cls} aria-hidden />
  if (/^Op config/i.test(label)) return <SlidersHorizontalIcon className={cls} aria-hidden />
  return <BoxIcon className={cls} aria-hidden />
}

/** The icon of one of the five transform verbs. */
function VerbIcon({ verb }: { verb: string }) {
  const cls = "size-4"
  switch (verb) {
    case "dedupe":
      return <CopyXIcon className={cls} aria-hidden />
    case "filter":
      return <FilterIcon className={cls} aria-hidden />
    case "rename":
      return <PencilLineIcon className={cls} aria-hidden />
    case "cast":
      return <ReplaceIcon className={cls} aria-hidden />
    case "select":
      return <ListChecksIcon className={cls} aria-hidden />
    default:
      return <BoxIcon className={cls} aria-hidden />
  }
}

function NodeCard({
  node,
  selected,
  onSelect,
  durationMs,
  dataCaption,
}: {
  node: PositionedNode
  selected: boolean
  onSelect?: (node: PositionedNode) => void
  durationMs?: number | null
  dataCaption: { source: string; sink: string }
}) {
  const style = { left: node.x, top: node.y, width: node.w, height: node.h }
  const base =
    "absolute flex items-center gap-2.5 rounded-xl border text-left transition-[box-shadow,border-color,background-color] outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
  const ring = selected
    ? "border-primary shadow-[0_0_0_3px_color-mix(in_oklab,var(--primary)_22%,transparent),0_10px_30px_-12px_var(--primary)]"
    : "hover:border-primary/40"

  if (node.kind === "source" || node.kind === "sink") {
    return (
      <button
        type="button"
        style={style}
        onClick={onSelect ? () => onSelect(node) : undefined}
        title={node.label}
        className={cn(base, "border-dashed border-border bg-background/90 px-3 backdrop-blur-sm", ring, !onSelect && "cursor-default")}
      >
        <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
          <DataIcon label={node.label} />
        </span>
        <span className="min-w-0">
          <span className="block font-mono text-[9px] uppercase tracking-[0.16em] text-muted-foreground">
            {node.kind === "source" ? dataCaption.source : dataCaption.sink}
          </span>
          <span className="line-clamp-2 text-xs font-medium leading-4">{node.label}</span>
          {node.sublabel ? <span className="block truncate text-[10px] text-muted-foreground">{node.sublabel}</span> : null}
        </span>
      </button>
    )
  }

  if (node.kind === "transform") {
    return (
      <button
        type="button"
        style={style}
        onClick={onSelect ? () => onSelect(node) : undefined}
        className={cn(base, "border-border bg-card px-3 shadow-xs", ring)}
      >
        <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-primary/10 text-primary">
          <VerbIcon verb={node.label} />
        </span>
        <span className="min-w-0">
          <span className="block text-sm font-medium capitalize leading-5">{node.label}</span>
          <span className="block truncate text-[11px] text-muted-foreground">{node.sublabel}</span>
        </span>
      </button>
    )
  }

  return (
    <button
      type="button"
      style={style}
      onClick={onSelect ? () => onSelect(node) : undefined}
      className={cn(base, "overflow-hidden border-border bg-card py-2.5 pl-4 pr-3 shadow-sm", ring)}
    >
      <span
        aria-hidden
        className={cn("absolute inset-y-0 left-0 w-1", node.status ? runFill(node.status) : "bg-border", node.status === "running" && "animate-pulse")}
      />
      <span className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="font-mono text-[9px] uppercase tracking-[0.16em] text-muted-foreground">op</span>
        <span className="truncate font-mono text-sm font-semibold leading-5">{node.label}</span>
        <span className="flex items-center gap-2 text-[11px] text-muted-foreground">
          {node.status ? (
            <span className="inline-flex items-center gap-1">
              <span className={cn("size-1.5 rounded-full", runFill(node.status))} aria-hidden />
              {statusLabel(node.status)}
            </span>
          ) : (
            <span>no run selected</span>
          )}
          {durationMs != null ? <span className="font-mono tabular-nums">{formatDuration(durationMs)}</span> : null}
        </span>
      </span>
    </button>
  )
}

/**
 * The pipeline drawn as a flowchart: nodes placed by `layoutFlow`, edges as
 * SVG curves with arrowheads. Edges touching the selected node are drawn in
 * the brand colour, and edges into or out of a running op move, so a live
 * run reads at a glance. Motion stops under `prefers-reduced-motion`.
 */
export function PipelineFlowchart({
  nodes,
  edges,
  selectedId,
  onSelect,
  durations,
  dataCaption = { source: "reads", sink: "writes" },
}: {
  nodes: FlowNodeSpec[]
  edges: FlowEdgeSpec[]
  selectedId: string | null
  onSelect?: (node: PositionedNode) => void
  /** Wall time per node id, from the selected run's steps. */
  durations?: Record<string, number | null>
  dataCaption?: { source: string; sink: string }
}) {
  const reduce = useReducedMotion() ?? false
  const layout = React.useMemo(() => layoutFlow(nodes, edges), [nodes, edges])
  const markerId = React.useId().replace(/:/g, "")

  return (
    <div
      className="relative overflow-x-auto rounded-xl border border-border bg-card [background-image:radial-gradient(var(--border)_1px,transparent_1px)] [background-size:18px_18px]"
      role="group"
      aria-label="Pipeline flowchart"
    >
      <div className="relative mx-auto" style={{ width: layout.width, height: Math.max(layout.height, 120) }}>
        <svg width={layout.width} height={layout.height} className="absolute inset-0 overflow-visible" aria-hidden>
          <defs>
            <marker id={`${markerId}-arrow`} viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
              <path d="M 0 0 L 10 5 L 0 10 z" className="fill-muted-foreground/60" />
            </marker>
            <marker id={`${markerId}-arrow-hot`} viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
              <path d="M 0 0 L 10 5 L 0 10 z" className="fill-primary" />
            </marker>
          </defs>
          {layout.edges.map(({ from, to }) => {
            const hot = selectedId !== null && (from.id === selectedId || to.id === selectedId)
            const live = !reduce && (from.status === "running" || to.status === "running")
            return (
              <path
                key={`${from.id}->${to.id}`}
                d={edgePath(from, to)}
                fill="none"
                strokeWidth={hot ? 2 : 1.5}
                markerEnd={`url(#${markerId}-arrow${hot ? "-hot" : ""})`}
                className={cn(
                  hot ? "stroke-primary" : "stroke-muted-foreground/40",
                  live && "[stroke-dasharray:6_6] motion-safe:animate-[flow-dash_0.8s_linear_infinite]"
                )}
              />
            )
          })}
        </svg>
        {layout.nodes.map((node) => (
          <NodeCard
            key={node.id}
            node={node}
            selected={node.id === selectedId}
            onSelect={onSelect}
            durationMs={durations?.[node.id]}
            dataCaption={dataCaption}
          />
        ))}
      </div>
    </div>
  )
}
