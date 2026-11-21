"use client"

import * as React from "react"
import Link from "next/link"
import { ArrowRight, ChevronRight } from "lucide-react"

import { Skeleton } from "@/components/ui/skeleton"
import { KIND_LABEL, nodeHref, nodeKind, type LineageNode } from "@/features/governance/lineage-graph"
import { cn } from "@/lib/utils"
import type { AssetDetail } from "@/services/contracts/assets"
import { layerDot } from "./catalog-layers"
import { assetDependents, lineageKey, lineageSides, type AssetLineageState } from "./asset-lineage"

/**
 * The lineage around an asset at a glance (`UX-8`): the source-to-chart
 * strip of mockup C above the Lineage tab's map, and the same items as a
 * short list on the Overview. Both read the one graph the page loads
 * (`useAssetLineage`), so they never disagree with the map or the tab's
 * count, and neither says anything while the graph loads or when the
 * reader may not see it: the map below (or the Lineage tab) says why.
 */

/** One thing on either side of the asset, as the strip and the list draw it. */
type Item = { id: string; label: string; caption: string; href: string | null; dot?: string }

/** How many items each side shows before "+ n more". */
const SIDE_MAX = 3

function nodeItem(node: LineageNode): Item {
  const kind = nodeKind(node)
  return {
    id: node.id,
    label: node.label,
    caption: KIND_LABEL[kind] ?? kind,
    href: nodeHref(node),
    dot: layerDot(kind),
  }
}

/**
 * What feeds the asset, nearest first, and what reads it: the graph's
 * downstream nodes, then the catalog's own dependents (saved queries,
 * dashboards) the graph does not hold, without repeats.
 */
export function lineageItems(a: AssetDetail, state: AssetLineageState): { upstream: Item[]; downstream: Item[] } | null {
  if (state.status !== "success" || state.data === null || !state.data.supported) return null
  const graph = state.data
  const { upstream, downstream } = lineageSides(graph)
  const down = downstream.map(nodeItem)
  const seen = new Set(down.map((d) => d.id))
  for (const d of assetDependents(a, graph)) {
    if (seen.has(d.id)) continue
    seen.add(d.id)
    down.push({ id: d.id, label: d.name, caption: KIND_LABEL[d.kind] ?? d.kind, href: d.href })
  }
  return { upstream: upstream.map(nodeItem), downstream: down }
}

function Chip({ item, here = false }: { item: Item; here?: boolean }) {
  const body = (
    <>
      <span className="flex min-w-0 items-center gap-1.5 text-[13px] font-medium text-foreground">
        {item.dot ? <span aria-hidden className={cn("size-2 shrink-0 rounded-full", item.dot)} /> : null}
        <span className="truncate">{item.label}</span>
      </span>
      <span className="truncate text-xs text-muted-foreground">{item.caption}</span>
    </>
  )
  const box = cn(
    "flex min-w-0 flex-col gap-0.5 rounded-lg px-3 py-2",
    here ? "bg-primary/10 ring-1 ring-primary/30" : "bg-background ring-1 ring-border dark:bg-card"
  )
  return item.href && !here ? (
    <Link
      href={item.href}
      title={item.label}
      className={cn(
        box,
        "outline-none transition-[box-shadow,background-color] duration-200 ease-ui hover:ring-foreground/20 focus-visible:ring-3 focus-visible:ring-ring/50"
      )}
    >
      {body}
    </Link>
  ) : (
    <div className={box} title={item.label}>
      {body}
    </div>
  )
}

function More({ n, onOpen }: { n: number; onOpen?: () => void }) {
  if (n <= 0) return null
  return onOpen ? (
    <button
      type="button"
      onClick={onOpen}
      className="rounded-md px-2 py-1 text-left text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50"
    >
      {`+ ${n} more`}
    </button>
  ) : (
    <span className="px-2 text-xs text-muted-foreground">{`+ ${n} more`}</span>
  )
}

function Arrow() {
  return <ArrowRight aria-hidden className="size-4 shrink-0 text-muted-foreground/60" />
}

/**
 * The strip: the nearest sources on the left (farthest first, so it reads
 * left to right as the data flows), this table, and what reads it stacked
 * on the right. Nothing when no lineage is recorded either side.
 */
export function LineageStrip({ asset: a, state }: { asset: AssetDetail; state: AssetLineageState }) {
  const items = lineageItems(a, state)
  if (items === null || (items.upstream.length === 0 && items.downstream.length === 0)) return null
  const up = items.upstream.slice(0, SIDE_MAX).reverse()
  const down = items.downstream.slice(0, SIDE_MAX)
  const self: Item = { id: "self", label: "This table", caption: lineageKey(a), href: null, dot: layerDot(a.layer) }
  return (
    <section aria-label="Lineage at a glance" className="overflow-x-auto rounded-xl bg-muted/40 p-3 dark:bg-card/60">
      <div className="flex w-max min-w-full items-center gap-2">
        {items.upstream.length > SIDE_MAX ? <More n={items.upstream.length - SIDE_MAX} /> : null}
        {up.map((item) => (
          <React.Fragment key={item.id}>
            <div className="grid w-44 shrink-0">
              <Chip item={item} />
            </div>
            <Arrow />
          </React.Fragment>
        ))}
        <div className="grid w-44 shrink-0">
          <Chip item={self} here />
        </div>
        {down.length > 0 ? (
          <>
            <Arrow />
            <div className="grid w-48 shrink-0 gap-1.5">
              {down.map((item) => (
                <Chip key={item.id} item={item} />
              ))}
              <More n={items.downstream.length - SIDE_MAX} />
            </div>
          </>
        ) : null}
      </div>
    </section>
  )
}

function ShortRow({ item }: { item: Item }) {
  const label = (
    <span className="flex min-w-0 items-center gap-2">
      {item.dot ? <span aria-hidden className={cn("size-2 shrink-0 rounded-full", item.dot)} /> : null}
      <span className="truncate">{item.label}</span>
    </span>
  )
  return (
    <li className="flex items-center gap-2 py-1 text-sm">
      {item.href ? (
        <Link href={item.href} title={item.label} className="min-w-0 flex-1 hover:text-primary">
          {label}
        </Link>
      ) : (
        <span title={item.label} className="min-w-0 flex-1">
          {label}
        </span>
      )}
      <span className="shrink-0 text-xs text-muted-foreground">{item.caption}</span>
    </li>
  )
}

/**
 * The Overview's lineage: what it comes from and what reads it, three each,
 * with the way to the Lineage tab. One quiet line when there is nothing to
 * list or the reader may not see it.
 */
export function LineageShortList({
  asset: a,
  state,
  onOpen,
}: {
  asset: AssetDetail
  state: AssetLineageState
  onOpen: () => void
}) {
  if (state.status === "loading") {
    return (
      <div role="status" aria-label="Loading lineage" className="flex flex-col gap-2 py-1">
        <Skeleton className="h-4 w-40" />
        <Skeleton className="h-4 w-32" />
      </div>
    )
  }
  const items = lineageItems(a, state)
  if (items === null) {
    const line =
      state.status === "success" && state.data === null
        ? "Lineage needs the lineage:read permission."
        : state.status === "success" && state.data !== null && !state.data.supported
          ? "Lineage is not available here."
          : "Lineage could not be read."
    return <p className="text-sm text-muted-foreground">{line}</p>
  }
  if (items.upstream.length === 0 && items.downstream.length === 0) {
    return <p className="text-sm text-muted-foreground">No lineage recorded.</p>
  }
  const more = Math.max(0, items.upstream.length - SIDE_MAX) + Math.max(0, items.downstream.length - SIDE_MAX)
  return (
    <div className="flex flex-col gap-2">
      {items.upstream.length > 0 ? (
        <div>
          <div className="text-xs text-muted-foreground">Comes from</div>
          <ul>
            {items.upstream.slice(0, SIDE_MAX).map((item) => (
              <ShortRow key={item.id} item={item} />
            ))}
          </ul>
        </div>
      ) : null}
      {items.downstream.length > 0 ? (
        <div>
          <div className="text-xs text-muted-foreground">Read by</div>
          <ul>
            {items.downstream.slice(0, SIDE_MAX).map((item) => (
              <ShortRow key={item.id} item={item} />
            ))}
          </ul>
        </div>
      ) : null}
      <button
        type="button"
        onClick={onOpen}
        className="inline-flex items-center gap-1 self-start rounded-md text-xs font-medium text-primary outline-none hover:underline focus-visible:ring-3 focus-visible:ring-ring/50"
      >
        {more > 0 ? `${more} more in Lineage` : "Open Lineage"}
        <ChevronRight className="size-3.5" aria-hidden />
      </button>
    </div>
  )
}
