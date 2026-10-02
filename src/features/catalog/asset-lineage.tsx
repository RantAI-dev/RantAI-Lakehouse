"use client"

import Link from "next/link"
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useAuth } from "@/features/auth/auth-provider"
import { KIND_LABEL, focusIds, type LineageNode } from "@/features/governance/lineage-graph"
import { useService, type ServiceState } from "@/hooks/use-service"
import { assetQueryStudioHref } from "@/lib/asset-query"
import { isIcebergCandidate } from "@/lib/lakehouse-view"
import { governanceService } from "@/services"
import type { AssetDetail } from "@/services/contracts/assets"
import type { LineageGraph } from "@/services/contracts/governance"
import { LineageMindmap, edgePipeline, mindmapModel } from "./asset-lineage-map"

/**
 * What the lineage graph calls this asset: its table key
 * (`bronze.orders`), not a Bronze asset's registry slug. An API build
 * older than `tableKey` gets the same key worked out here.
 */
export function lineageKey(a: AssetDetail): string {
  if (a.tableKey) return a.tableKey
  return isIcebergCandidate(a) && a.tableName ? `bronze.${a.tableName}` : a.id
}

/**
 * The recorded lineage around the asset, loaded once for the tab, its
 * count and the overview. `data` is `null` when the caller lacks
 * `lineage:read`: nothing is asked for, and nothing is shown as empty.
 */
export type AssetLineageState = ServiceState<LineageGraph | null> & { reload: () => void }

export function useAssetLineage(a: AssetDetail): AssetLineageState {
  const { hasPermission } = useAuth()
  const allowed = hasPermission("lineage:read")
  const key = lineageKey(a)
  return useService(
    (s) => (allowed ? governanceService.getLineage(key, s) : Promise.resolve(null)),
    [allowed, key]
  )
}

type Related = { id: string; name: string; kind: string; href: string | null; detail?: string }

/**
 * Everything recorded on each side of the asset, nearest first: what it is
 * built from, and what is built from it. The graph the API returns is
 * already cut down to those two sides, so a node is upstream exactly when
 * the asset can be reached from it.
 */
export function lineageSides(graph: LineageGraph | null): {
  upstream: LineageNode[]
  downstream: LineageNode[]
} {
  if (!graph) return { upstream: [], downstream: [] }
  const focus = focusIds(graph)
  const byId = new Map(graph.nodes.map((n) => [n.id, n]))
  const walk = (forward: boolean) => {
    const seen = new Set(focus)
    const queue = [...focus]
    const found: LineageNode[] = []
    while (queue.length > 0) {
      const id = queue.shift()!
      for (const edge of graph.edges) {
        const next = forward ? (edge.from === id ? edge.to : null) : edge.to === id ? edge.from : null
        if (next === null || seen.has(next)) continue
        seen.add(next)
        queue.push(next)
        const node = byId.get(next)
        if (node) found.push(node)
      }
    }
    return found
  }
  return { upstream: walk(false), downstream: walk(true) }
}

/**
 * What reads this asset: the pipelines the graph records reading it, then
 * whatever the catalog itself lists (`dependents`), without repeats.
 */
export function assetDependents(a: AssetDetail, graph: LineageGraph | null): Related[] {
  const focus = graph ? focusIds(graph) : new Set<string>()
  const out = new Map<string, Related>()
  for (const edge of graph?.edges ?? []) {
    const pipeline = edge.kind === "pipeline" && focus.has(edge.from) ? edgePipeline(edge.evidence) : null
    if (pipeline) {
      out.set(pipeline.id, {
        id: pipeline.id,
        name: pipeline.name,
        kind: "pipeline",
        href: `/pipelines/${encodeURIComponent(pipeline.id)}`,
      })
    }
  }
  for (const d of a.dependents) {
    if (!out.has(d.id)) {
      // The built-in dashboard has no board row, so its id ("default") is
      // the only name the API has for it.
      const name = d.kind === "dashboard" && d.name === "default" ? "Default dashboard" : d.name
      out.set(d.id, { id: d.id, name, kind: d.kind, href: dependentHref(d.id, d.kind), detail: d.detail })
    }
  }
  return [...out.values()]
}

/**
 * Tables the catalog relates to this one that the graph does not record —
 * today, the Silver model built from a Bronze dataset of the same name.
 */
export function relatedTables(a: AssetDetail, graph: LineageGraph | null): Related[] {
  const { upstream, downstream } = relatedSides(a, graph)
  return [...upstream, ...downstream]
}

/** [`relatedTables`], kept apart by the side of the asset each is on. */
function relatedSides(
  a: AssetDetail,
  graph: LineageGraph | null
): { upstream: Related[]; downstream: Related[] } {
  // A graph id carries its kind (`table:silver.orders`); the catalog's own
  // ids do not, so a node is matched by what follows the prefix as well.
  const inGraph = new Set(
    (graph?.nodes ?? []).flatMap((n) => [n.id, n.id.slice(n.id.indexOf(":") + 1), n.label])
  )
  const outside = (tables: AssetDetail["upstream"]): Related[] =>
    tables
      .filter((t) => !inGraph.has(t.id))
      .map((t) => ({ id: t.id, name: t.name, kind: "table", href: `/data/assets/${encodeURIComponent(t.id)}` }))
  return { upstream: outside(a.upstream), downstream: outside(a.downstream) }
}

/**
 * The asset's own node, for a map whose graph does not hold it: what the
 * graph would call it, and the layer it is in.
 */
function selfNode(a: AssetDetail): { label: string; kind: string } {
  const kind = a.layer === "silver" ? "silver" : a.layer === "gold" ? "gold" : "bronze"
  return { label: lineageKey(a), kind }
}

/** How many things the Lineage tab has to show, for its count. */
export function lineageCount(a: AssetDetail, state: AssetLineageState): number {
  const graph = state.status === "success" ? state.data : null
  const { upstream, downstream } = lineageSides(graph)
  const dependents = assetDependents(a, graph)
  // A reading pipeline is already counted as downstream.
  const counted = new Set([...upstream, ...downstream].map((n) => n.id))
  return (
    upstream.length +
    downstream.length +
    dependents.filter((d) => !counted.has(d.id)).length +
    relatedTables(a, graph).length
  )
}

function dependentHref(id: string, kind: string) {
  const k = kind.toLowerCase()
  const ref = encodeURIComponent(id)
  if (k.includes("pipeline")) return `/pipelines/${ref}`
  if (k.includes("agent")) return `/agents/employees/${ref}`
  if (k.includes("dashboard")) return `/dashboards?board=${ref}`
  if (k.includes("query")) return `/query-studio?saved=${ref}`
  return `/data/assets/${ref}`
}

function RelatedList({ items }: { items: Related[] }) {
  return (
    <ul className="divide-y divide-border text-sm">
      {items.map((d) => (
        <li key={d.id} className="flex items-center gap-2 py-1.5">
          {d.href ? (
            <Link href={d.href} className="truncate font-mono text-primary hover:underline">
              {d.name}
            </Link>
          ) : (
            <span className="truncate font-mono">{d.name}</span>
          )}
          {d.detail ? <span className="shrink-0 text-xs text-muted-foreground">{d.detail}</span> : null}
          <Pill tone="neutral" className="ml-auto shrink-0">
            {KIND_LABEL[d.kind] ?? d.kind}
          </Pill>
        </li>
      ))}
    </ul>
  )
}

/** The recorded graph around the asset, or why there is none to draw. */
function GraphCard({ a, state }: { a: AssetDetail; state: AssetLineageState }) {
  if (state.status === "loading") {
    return (
      <SectionCard size="sm" title="Lineage">
        <div className="flex gap-3 py-1" role="status" aria-label="Loading lineage">
          <Skeleton className="h-16 w-52" />
          <Skeleton className="h-16 w-52" />
          <Skeleton className="h-16 w-52" />
        </div>
      </SectionCard>
    )
  }
  if (state.status === "error") {
    return (
      <SectionCard size="sm" title="Lineage">
        <ErrorState error={state.error} onRetry={state.reload} />
      </SectionCard>
    )
  }
  const graph = state.data
  if (graph === null) {
    return (
      <SectionCard size="sm" title="Lineage">
        <EmptyState
          title="Lineage needs lineage access"
          description="Seeing where data comes from requires the lineage:read permission. Use Request access above to ask for it."
          className="py-4"
        />
      </SectionCard>
    )
  }
  if (!graph.supported) {
    return (
      <SectionCard size="sm" title="Lineage">
        <EmptyState title="Lineage not available" description={graph.reason} className="py-4" />
      </SectionCard>
    )
  }
  // The map branches into more than the recorded graph: what reads the
  // asset and what the catalog relates to it. The pipelines the graph
  // records are its edges already, so only the catalog's own dependents
  // are added.
  const related = relatedSides(a, graph)
  const model = mindmapModel({
    graph,
    self: selfNode(a),
    dependents: assetDependents(a, null),
    relatedUpstream: related.upstream,
    relatedDownstream: related.downstream,
  })
  const unrecorded = graph.nodes.length === 0
  if (unrecorded && model.nodes.length === 1) {
    return (
      <SectionCard size="sm" title="Lineage">
        <EmptyState
          title={`No lineage recorded for ${lineageKey(a)}`}
          description={graph.note}
          className="py-4"
        />
      </SectionCard>
    )
  }
  const { upstream, downstream } = lineageSides(graph)
  return (
    <SectionCard
      size="sm"
      title="Lineage"
      description={
        unrecorded
          ? `No lineage is recorded for ${lineageKey(a)}. What reads it and what the catalog ties to it is drawn around it.`
          : `${upstream.length} upstream · ${downstream.length} downstream of this asset, by recorded lineage.`
      }
    >
      <LineageMindmap model={model} />
      {/* What the graph cannot contain, so a missing step is not read as "there is none". */}
      {[graph.note, ...(graph.coverage ?? [])].filter(Boolean).map((caveat) => (
        <p key={caveat} className="mt-2 text-xs text-muted-foreground">
          {caveat}
        </p>
      ))}
    </SectionCard>
  )
}

/**
 * The Lineage tab: the recorded graph around the asset, the columns its
 * pipelines map, what reads it, and any table the catalog relates to it
 * outside the graph.
 */
export function AssetLineage({ asset: a, state }: { asset: AssetDetail; state: AssetLineageState }) {
  const graph = state.status === "success" ? state.data : null
  const dependents = assetDependents(a, graph)
  const related = relatedTables(a, graph)
  const mappings = graph?.columnMappings ?? []

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="outline"
          render={<Link href={`/lineage?focus=${encodeURIComponent(lineageKey(a))}`} />}
        >
          Open lineage graph
        </Button>
        <Button size="sm" variant="ghost" render={<Link href={assetQueryStudioHref(a)} />}>
          Query this asset
        </Button>
      </div>

      <GraphCard a={a} state={state} />

      {mappings.length > 0 ? (
        <SectionCard
          size="sm"
          title="Column mappings"
          description="How the pipelines in this graph carry columns from source to target."
        >
          <ul className="divide-y divide-border text-sm">
            {mappings.map((m) => (
              <li key={`${m.source}-${m.target}`} className="flex flex-wrap items-baseline gap-x-2 py-1.5">
                <span className="font-mono text-xs">{m.source}</span>
                <span className="text-muted-foreground">→</span>
                <span className="font-mono text-xs">{m.target}</span>
                <span className="ml-auto text-xs text-muted-foreground">{m.transform}</span>
              </li>
            ))}
          </ul>
        </SectionCard>
      ) : null}

      <div className="grid gap-2 lg:grid-cols-2">
        <SectionCard
          size="sm"
          title="Dependents"
          description="Pipelines, saved queries and dashboards that read this asset."
        >
          {dependents.length === 0 ? (
            <EmptyState title="No dependents recorded" className="py-4" />
          ) : (
            <RelatedList items={dependents} />
          )}
        </SectionCard>
        <SectionCard
          size="sm"
          title="Related tables"
          description="Tables the catalog ties to this one outside the recorded graph."
        >
          {related.length === 0 ? (
            <EmptyState title="No related tables" className="py-4" />
          ) : (
            <RelatedList items={related} />
          )}
        </SectionCard>
      </div>
    </div>
  )
}
