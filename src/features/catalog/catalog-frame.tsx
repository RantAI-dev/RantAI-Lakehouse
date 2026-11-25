"use client"

import * as React from "react"
import Link from "next/link"
import { Plus } from "lucide-react"

import { useDataTableSearchValue } from "@/components/data-table/data-table-search"
import { Button } from "@/components/ui/button"
import { SearchField, SegmentPill } from "@/features/dashboards/library-parts"
import { useSlidingIndicator } from "@/hooks/use-sliding-indicator"
import type { DataLayer } from "@/lib/status"
import { cn } from "@/lib/utils"
import { LAYER_CHOICES, activeLayerChoice } from "./catalog-layers"

/**
 * The pieces of the Catalog's page frame (`UX-8`, mockup A; `DESIGN.md` §3:
 * title left, then the icon search, ⋯ and the primary on one 32 px row,
 * then one filter row). They are the same controls the Dashboards page
 * draws (`SearchField`, the sliding segment pill), so the two pages read as
 * one app.
 */

/**
 * The page's search: the icon-only search of the page frame, writing the
 * table's own `search` param (the same state the toolbar's search box held,
 * through `useDataTableSearchValue`). Must sit under the table's
 * `DataTableQueryKeysProvider`.
 */
export function CatalogSearch({ className }: { className?: string }) {
  const { value, update } = useDataTableSearchValue()
  return (
    <SearchField
      value={value}
      onChange={update}
      placeholder="Name, column, tag, description or owner"
      inputLabel="Search by name, column, tag, description or owner"
      buttonLabel="Search the catalog"
      className={className}
      expandedClassName="w-[min(18rem,calc(100vw-12rem))] shrink-0"
    />
  )
}

/**
 * All · Bronze · Silver · Gold as one segmented control (mockup A). Each is a
 * real button with `aria-pressed`; the raised pill slides to the choice.
 */
export function LayerSwitch({
  values,
  onPick,
  className,
}: {
  /** The `layer` filter's values. */
  values: readonly string[]
  onPick: (layers: DataLayer[]) => void
  className?: string
}) {
  const active = activeLayerChoice(values)
  const { listRef, indicatorRef } = useSlidingIndicator<HTMLDivElement>(active ?? "none", { box: true })
  return (
    <div
      ref={listRef}
      role="group"
      aria-label="Layer"
      className={cn("relative isolate inline-flex h-8 shrink-0 rounded-lg bg-muted p-0.5 dark:bg-input/30", className)}
    >
      <SegmentPill indicatorRef={indicatorRef} />
      {LAYER_CHOICES.map((choice) => {
        const on = choice.id === active
        return (
          <button
            key={choice.id}
            type="button"
            aria-pressed={on}
            onClick={() => onPick(choice.layers)}
            data-indicator-item
            data-indicator-active={on || undefined}
            className={cn(
              "inline-flex h-7 flex-1 items-center justify-center gap-1.5 rounded-md px-2.5 text-[13px] font-medium whitespace-nowrap",
              "outline-none transition-[color,scale] duration-200 ease-ui focus-visible:ring-3 focus-visible:ring-ring/50 active:scale-[0.97]",
              on ? "text-foreground" : "text-muted-foreground hover:text-foreground"
            )}
          >
            {choice.dot ? <span aria-hidden className={cn("size-2 rounded-full", choice.dot)} /> : null}
            {choice.label}
          </button>
        )
      })}
    </div>
  )
}

/**
 * The page's primary: Add data, which opens Sources (`/connectors`), the
 * place a new table comes from (owner, 2026-11-21). The caller shows it only
 * to a holder of `connector:manage`, the permission the Sources page needs;
 * without it the page has no primary. Icon-only on a phone, so the row fits.
 */
export function AddDataButton() {
  // One button, not a phone copy and a laptop copy: on a phone the label is
  // read out but not drawn, and the button closes to the 32 px square.
  return (
    <Button className="max-sm:w-8 max-sm:px-0" render={<Link href="/connectors" />}>
      <Plus data-icon="inline-start" aria-hidden />
      <span className="max-sm:sr-only">Add data</span>
    </Button>
  )
}
