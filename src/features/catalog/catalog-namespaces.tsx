"use client"

import * as React from "react"
import { ChevronDown, FolderOpen, RotateCcw } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { Skeleton } from "@/components/ui/skeleton"
import type { ServiceState } from "@/hooks/use-service"
import { cn } from "@/lib/utils"
import type { CatalogNamespace } from "@/services/contracts/assets"

/**
 * The Catalog's namespace filter (`UX-8`, mockup A): one searchable picker
 * in the filter row, where `WEB-3` S2-D1 had a row of one chip per
 * namespace. A row of chips grows with the catalog and scrolled sideways on
 * a laptop; the picker stays one 32 px control however many there are.
 *
 * Picking a namespace sets the table's `namespace` filter to the namespace
 * **id** (DATA-23: the assets carry the id, and a display name that differs
 * from it, "Silver Layer" vs `silver`, matched nothing). It is the same
 * `filters` entry the toolbar's Filter menu writes, so the two cannot
 * disagree (plan decision: the quick controls are the menu's own state).
 *
 * The namespaces load on their own: while they load the trigger is a
 * placeholder, if they fail the picker says so with a Retry, and the asset
 * list below works either way (S2-D4).
 */
export function NamespacePicker({
  state,
  activeId,
  onPick,
}: {
  state: ServiceState<CatalogNamespace[]> & { reload: () => void }
  /** The namespace the table is filtered to, or `null` for all of them. */
  activeId: string | null
  /** Sets the namespace filter, or clears it for `null`. */
  onPick: (namespaceId: string | null) => void
}) {
  const [open, setOpen] = React.useState(false)

  if (state.status === "loading") {
    return (
      <span role="status" aria-label="Loading namespaces">
        <Skeleton className="h-8 w-32 rounded-lg" />
      </span>
    )
  }
  // No namespaces is not worth a control: the table's own empty state says
  // there is nothing to list.
  if (state.status === "success" && state.data.length === 0) return null

  const namespaces = state.status === "success" ? state.data : []
  const active = namespaces.find((ns) => ns.id === activeId) ?? null
  const total = namespaces.reduce((sum, ns) => sum + ns.assetCount, 0)

  const pick = (id: string | null) => {
    setOpen(false)
    onPick(id)
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          className={cn("max-w-56 font-normal", activeId !== null && "border-border bg-muted/50 font-medium")}
        >
          <FolderOpen className="text-muted-foreground" aria-hidden />
          {/* A namespace the list does not name (an old link) still shows, by its id. */}
          <span className="truncate">{active ? active.name : (activeId ?? "Namespace")}</span>
          <ChevronDown className="text-muted-foreground" aria-hidden />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 p-0">
        {state.status === "error" ? (
          <div className="flex flex-wrap items-center gap-2 p-3">
            <span className="text-xs text-muted-foreground">Could not load namespaces.</span>
            <Button size="sm" variant="ghost" onClick={state.reload}>
              <RotateCcw className="size-3.5" aria-hidden />
              Retry
            </Button>
          </div>
        ) : (
          <Command>
            <CommandInput placeholder="Find a namespace" aria-label="Find a namespace" />
            <CommandList>
              <CommandEmpty>No namespace matches.</CommandEmpty>
              <CommandGroup>
                <CommandItem
                  value="__all__ all namespaces"
                  data-checked={activeId === null}
                  onSelect={() => pick(null)}
                >
                  <span className="flex-1 truncate">All namespaces</span>
                  <span className="text-xs text-muted-foreground tabular-nums">{total.toLocaleString("en-US")}</span>
                </CommandItem>
                {namespaces.map((ns) => (
                  <CommandItem
                    key={ns.id}
                    // Searched by both the name the list shows and the id the filter sends.
                    value={`${ns.id} ${ns.name}`}
                    data-checked={ns.id === activeId}
                    onSelect={() => pick(ns.id)}
                  >
                    <span className="flex min-w-0 flex-1 flex-col">
                      <span className="truncate">{ns.name}</span>
                      {ns.id !== ns.name ? (
                        <span className="truncate font-mono text-[11px] text-muted-foreground">{ns.id}</span>
                      ) : null}
                    </span>
                    <span className="text-xs text-muted-foreground tabular-nums">
                      {ns.assetCount.toLocaleString("en-US")}
                    </span>
                  </CommandItem>
                ))}
              </CommandGroup>
            </CommandList>
          </Command>
        )}
      </PopoverContent>
    </Popover>
  )
}

/**
 * What the old drawer said about a namespace, as one line under the filter
 * row while the list is filtered to it (`UX-8`; it was a card with an
 * "Assets" count, a second "All namespaces" button and four labelled facts).
 * The count is the footer's "Showing X of Y" and the way back is the
 * picker's "All namespaces", so the line carries only what the rows do not:
 * the description, the owner, the residency and the source engine. A fact
 * that is not recorded is left out rather than shown as a dash.
 */
export function NamespaceLine({ namespace }: { namespace: CatalogNamespace }) {
  const facts = [
    namespace.description,
    namespace.owner ? `Owned by ${namespace.owner}` : "",
    namespace.residency,
    namespace.sourceEngine,
  ].filter((fact) => fact.trim() !== "")
  const whole = [namespace.name, ...facts].join(" · ")
  return (
    <p className="truncate text-xs leading-5 text-muted-foreground" title={whole}>
      <span className="font-medium text-foreground">{namespace.name}</span>
      {facts.map((fact) => (
        <React.Fragment key={fact}>
          <span aria-hidden> · </span>
          <span>{fact}</span>
        </React.Fragment>
      ))}
    </p>
  )
}
