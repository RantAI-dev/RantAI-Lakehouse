"use client"

import Link from "next/link"
import { RotateCcw, X } from "lucide-react"

import { MetadataList } from "@/components/patterns/metadata-list"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import type { ServiceState } from "@/hooks/use-service"
import { cn } from "@/lib/utils"
import type { CatalogNamespace } from "@/services/contracts/assets"

/**
 * One labelled row of quick filters above the Catalog's table.
 *
 * The label column has a fixed width so the Namespace and Layer rows line
 * up, and the chips scroll sideways inside their row on a phone instead of
 * wrapping into a wall of buttons or widening the page.
 */
export function QuickFilterRow({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="flex min-w-0 items-center gap-2">
      <span className="w-20 shrink-0 text-xs text-muted-foreground">{label}</span>
      <div className="-my-1 flex min-w-0 flex-1 items-center gap-1 overflow-x-auto py-1">
        {children}
      </div>
    </div>
  )
}

/**
 * The namespace row (`WEB-3` S2-D1): what the old Catalog page's namespace
 * table and its drawer of five did, as a filter on the one asset list.
 *
 * Each chip is a link that sets the table's `namespace` filter to the
 * namespace **id** (DATA-23: the assets carry the id, and a display name
 * that differs from it, "Silver Layer" vs `silver`, matched nothing). The
 * namespaces load on their own: if they fail, the row says so with a Retry
 * and the asset list below still works (S2-D4).
 */
export function NamespaceFilterRow({
  state,
  activeId,
  hrefFor,
}: {
  state: ServiceState<CatalogNamespace[]> & { reload: () => void }
  /** The namespace the table is filtered to, or `null` for all of them. */
  activeId: string | null
  /** The Catalog address with the namespace filter set, or cleared for `null`. */
  hrefFor: (namespaceId: string | null) => string
}) {
  if (state.status === "loading") {
    return (
      <QuickFilterRow label="Namespace">
        <span role="status" aria-label="Loading namespaces" className="flex gap-1">
          {[0, 1, 2].map((i) => (
            <Skeleton key={i} className="h-7 w-20" />
          ))}
        </span>
      </QuickFilterRow>
    )
  }
  if (state.status === "error") {
    return (
      <QuickFilterRow label="Namespace">
        <span className="text-xs text-muted-foreground">Could not load namespaces.</span>
        <Button size="sm" variant="ghost" onClick={state.reload}>
          <RotateCcw className="size-3.5" aria-hidden />
          Retry
        </Button>
      </QuickFilterRow>
    )
  }
  // No namespaces is not worth a row: the table's own empty state says
  // there is nothing to list.
  if (state.data.length === 0) return null

  return (
    <QuickFilterRow label="Namespace">
      <Button
        size="sm"
        variant={activeId === null ? "secondary" : "ghost"}
        render={<Link href={hrefFor(null)} />}
      >
        All
      </Button>
      {state.data.map((ns) => {
        const active = ns.id === activeId
        return (
          <Button
            key={ns.id}
            size="sm"
            variant={active ? "secondary" : "ghost"}
            aria-pressed={active}
            title={ns.id === ns.name ? undefined : ns.id}
            className={cn("shrink-0", active && "font-semibold")}
            render={<Link href={hrefFor(ns.id)} />}
          >
            {ns.name}
            <span className="tabular-nums text-muted-foreground">{ns.assetCount}</span>
          </Button>
        )
      })}
    </QuickFilterRow>
  )
}

/**
 * What the old drawer said about a namespace, shown above the list while
 * the list is filtered to it: its description and the ownership, residency
 * and source-engine metadata the asset rows do not carry.
 */
export function NamespaceSummary({
  namespace,
  clearHref,
}: {
  namespace: CatalogNamespace
  clearHref: string
}) {
  return (
    <div className="flex flex-col gap-3 rounded-lg border border-border bg-muted/20 p-3 sm:flex-row sm:items-start sm:justify-between">
      <div className="flex min-w-0 flex-col gap-2">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-2">
          <p className="text-sm font-semibold text-foreground">{namespace.name}</p>
          {namespace.name !== namespace.id ? (
            <span className="font-mono text-xs text-muted-foreground">{namespace.id}</span>
          ) : null}
        </div>
        {namespace.description ? (
          <p className="text-sm text-muted-foreground">{namespace.description}</p>
        ) : null}
        <MetadataList
          density="compact"
          columns={4}
          items={[
            { label: "Assets", value: namespace.assetCount },
            { label: "Owner", value: namespace.owner || "—" },
            { label: "Residency", value: namespace.residency || "—" },
            { label: "Source engine", value: namespace.sourceEngine || "—" },
          ]}
        />
      </div>
      <Button
        size="sm"
        variant="ghost"
        className="self-start"
        render={<Link href={clearHref} />}
      >
        <X className="size-3.5" aria-hidden />
        All namespaces
      </Button>
    </div>
  )
}
