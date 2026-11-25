"use client"

import type { ColumnDef } from "@tanstack/react-table"
import {
  BadgeCheck,
  Boxes,
  CalendarClock,
  Database,
  HardDrive,
  Layers,
  Tag,
  Tags,
  Text,
  User,
} from "lucide-react"

import { Copyable } from "@/components/copyable"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { AssetRowActions, assetAddress } from "./data-explorer-actions"
import { layerDot } from "./catalog-layers"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { HelpHint } from "@/components/patterns/help-hint"
import { CertificationBadge, Pill, TierBadge } from "@/components/patterns/status-badge"
import { formatBytes } from "@/lib/format"
import { governedPairLabels } from "./governed-tags"
import { matchedOnLabel } from "@/lib/palette-search"
import {
  DATA_LAYER_LABEL,
  STORAGE_TIER_LABEL,
  type DataLayer,
  type StorageTier,
} from "@/lib/status"
import {
  ASSET_TYPE_LABEL,
  type Asset,
  type AssetType,
} from "@/services/contracts/assets"

/**
 * Column definitions for the Data Explorer's advanced table.
 *
 * `meta` drives the toolbar, not just presentation: `label` names the
 * column in the filter/sort menus, `variant` decides which filter control
 * appears, `options` populate the faceted pickers, and `enableGrouping`
 * opts a column into "Group by". A column without `meta` still renders but
 * is invisible to the toolbar.
 *
 * Every `id` here must also exist in the backend's `FILTERABLE_FIELDS`
 * allowlist (`routes/catalog_query.rs`) — the server answers anything else
 * with a 400 rather than ignoring it.
 */

const layerOptions = Object.entries(DATA_LAYER_LABEL).map(([value, label]) => ({
  label,
  value,
}))

const tierOptions = Object.entries(STORAGE_TIER_LABEL).map(([value, label]) => ({
  label,
  value,
}))

// DATA-12: the filter offers the two marks; an unmarked table has none to pick.
const certificationOptions = [
  { value: "certified", label: "Certified" },
  { value: "deprecated", label: "Deprecated" },
]

const typeOptions = Object.entries(ASSET_TYPE_LABEL).map(([value, label]) => ({
  label,
  value,
}))

/**
 * What a dash in the Freshness column means, said once on its header
 * (`UX-8`, plan decision 5) instead of "Not measured" on every row.
 */
export const FRESHNESS_NOT_MEASURED =
  "A dash means nothing measures this asset's freshness yet: no load has reported when it last wrote. Give its pipeline a schedule or a freshness target and the age appears here."

/** The reason line under a name; nothing when the server sent none. */
export function MatchedOnLine({ matchedOn }: { matchedOn: Asset["matchedOn"] }) {
  const reason = matchedOnLabel(matchedOn)
  if (!reason) return null
  return <span className="truncate text-xs text-muted-foreground">{reason}</span>
}

/**
 * Built as a function rather than a constant because the `actions` column
 * needs to navigate, and the router only exists inside the component.
 * Callers should memoise the result — a fresh array on every render would
 * make TanStack Table rebuild its column model each time.
 */
export function getDataExplorerColumns({
  onOpen,
}: {
  onOpen: (asset: Asset) => void
}): ColumnDef<Asset>[] {
  return [
    {
      id: "name",
      accessorKey: "name",
      header: ({ column }) => <DataTableColumnHeader column={column} label="Name" />,
      // UX-8 (mockup A): the name, and the table's address under it — the
      // name a person types in SQL, which also says the namespace, so the
      // Namespace column is hidden by default (it stays defined: its filter,
      // sort and grouping live on it). The address is code, so it is the one
      // monospace line in the row (plan decision 3).
      cell: ({ row }) => (
        <div className="flex min-w-0 flex-col py-0.5">
          {/* Capped and truncated: asset names run long ("Kunjungan Daya
              Tarik Wisata"), and left to itself this column took ~270px —
              enough to push the last columns off a narrow screen on its own.
              The full name is still available on hover and on the detail
              page. */}
          <div className="flex items-center gap-1.5">
            <Copyable value={row.original.name} className="max-w-[12rem] xl:max-w-[22rem]">
              <span
                className="truncate font-medium tracking-tight text-foreground"
                title={row.original.name}
              >
                {row.original.name}
              </span>
            </Copyable>
            {/* DATA-12: the Tags column is hidden on most screens, so the
                mark rides the name instead of a column of its own. */}
            <CertificationBadge certification={row.original.certification} />
          </div>
          <span
            className="max-w-[12rem] truncate font-mono text-xs text-muted-foreground xl:max-w-[22rem]"
            title={assetAddress(row.original)}
          >
            {assetAddress(row.original)}
          </span>
          {/* Why a search returned this row (DATA-11); only a search with a
              term sends it, and a match on the name needs no reason. */}
          <MatchedOnLine matchedOn={row.original.matchedOn} />
        </div>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Name",
        placeholder: "Search name…",
        variant: "text",
        icon: Text,
      },
    },
    {
      id: "namespace",
      accessorKey: "namespace",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Namespace" />
      ),
      // Hidden by default (UX-8: the address under the name says it); the
      // column stays for its filter, sort and grouping, and the Columns menu
      // brings it back.
      cell: ({ row }) => (
        <Copyable value={row.original.namespace}>
          <span className="font-mono text-xs">{row.original.namespace}</span>
        </Copyable>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Namespace",
        placeholder: "Search namespace…",
        variant: "text",
        icon: Boxes,
        enableGrouping: true,
      },
    },
    {
      id: "owner",
      accessorKey: "owner",
      header: ({ column }) => <DataTableColumnHeader column={column} label="Owner" />,
      cell: ({ row }) => (
        // Capped and truncated like the Name cell, with the full value as
        // its title: an owner is free text and can be an address (DATA-11
        // review SHOULD-FIX 7).
        <span
          className="block max-w-[10rem] truncate text-sm"
          title={row.original.owner || undefined}
        >
          {row.original.owner || "—"}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Owner",
        placeholder: "Search owner…",
        variant: "text",
        icon: User,
      },
    },
    {
      // Not sortable: the server refuses `tags` as a sort field (a list has
      // no order to sort by), so the header must not offer it. The filter
      // sends `iLike`, which matches when any tag contains the text. DATA-12
      // part 2: the filter reads governed pairs too (as `key: value`, the
      // server's `tags` array filter), so the column shows both kinds.
      id: "tags",
      accessorFn: (asset) => [...governedPairLabels(asset.governedTags), ...(asset.tags ?? [])],
      header: ({ column }) => <DataTableColumnHeader column={column} label="Tags" />,
      cell: ({ row }) => {
        const tags = row.original.tags ?? []
        const governed = governedPairLabels(row.original.governedTags)
        if (tags.length === 0 && governed.length === 0) {
          return <span className="text-muted-foreground">—</span>
        }
        return (
          // Capped so a long list wraps inside the column instead of widening
          // the table (DATA-11 review SHOULD-FIX 7).
          <span className="flex max-w-[14rem] flex-wrap gap-1">
            {governed.map((pair) => (
              <Pill key={`governed:${pair}`} tone="info" title="Governed tag">
                {pair}
              </Pill>
            ))}
            {tags.map((tag) => (
              <Pill key={tag} tone="neutral">
                {tag}
              </Pill>
            ))}
          </span>
        )
      },
      enableSorting: false,
      enableColumnFilter: true,
      meta: {
        label: "Tags",
        placeholder: "Search tags…",
        variant: "text",
        icon: Tags,
      },
    },
    {
      id: "type",
      accessorKey: "type",
      header: ({ column }) => <DataTableColumnHeader column={column} label="Type" />,
      cell: ({ row }) => (
        <span className="text-sm">
          {ASSET_TYPE_LABEL[row.original.type as AssetType] ?? row.original.type}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Type",
        variant: "multiSelect",
        options: typeOptions,
        icon: Database,
        enableGrouping: true,
      },
    },
    {
      id: "layer",
      accessorKey: "layer",
      header: ({ column }) => <DataTableColumnHeader column={column} label="Layer" />,
      cell: ({ row }) => {
        const dot = layerDot(row.original.layer)
        return (
          <span className="inline-flex items-center gap-1.5 whitespace-nowrap text-sm">
            {dot ? <span aria-hidden className={`size-2 shrink-0 rounded-full ${dot}`} /> : null}
            {DATA_LAYER_LABEL[row.original.layer as DataLayer] ?? row.original.layer}
          </span>
        )
      },
      enableColumnFilter: true,
      meta: {
        label: "Layer",
        variant: "multiSelect",
        options: layerOptions,
        icon: Layers,
        enableGrouping: true,
      },
    },
    {
      id: "tier",
      accessorKey: "tier",
      header: ({ column }) => <DataTableColumnHeader column={column} label="Tier" />,
      cell: ({ row }) => <TierBadge tier={row.original.tier as StorageTier} />,
      enableColumnFilter: true,
      meta: {
        label: "Tier",
        variant: "multiSelect",
        options: tierOptions,
        icon: Tag,
        enableGrouping: true,
      },
    },
    {
      id: "freshnessLagSeconds",
      accessorKey: "freshnessLagSeconds",
      header: ({ column }) => (
        <span className="inline-flex items-center gap-0.5">
          <DataTableColumnHeader column={column} label="Freshness" />
          {/* Outside the header's own button, and kept from reaching it:
              the header opens its menu on pointer down and starts a column
              drag, neither of which a look at the hint should do. */}
          <span
            className="inline-flex"
            onPointerDown={(event) => event.stopPropagation()}
            onClick={(event) => event.stopPropagation()}
            onKeyDown={(event) => event.stopPropagation()}
          >
            <HelpHint>{FRESHNESS_NOT_MEASURED}</HelpHint>
          </span>
        </span>
      ),
      cell: ({ row }) =>
        row.original.freshnessLagSeconds === null ? (
          // UX-8: a quiet dash, explained once on the header.
          <span className="text-muted-foreground/70">
            —<span className="sr-only">Not measured</span>
          </span>
        ) : (
          <FreshnessIndicator
            lagSeconds={row.original.freshnessLagSeconds}
            targetSeconds={row.original.freshnessTargetSeconds ?? null}
            targetSource={row.original.freshnessTargetSource}
          />
        ),
      enableColumnFilter: true,
      meta: {
        label: "Freshness",
        // Filtered as a number (lag in seconds) even though it reads as a
        // relative time — that is what the API stores and compares.
        variant: "number",
        unit: "s",
        icon: CalendarClock,
      },
    },
    {
      id: "sizeBytes",
      accessorKey: "sizeBytes",
      header: ({ column }) => <DataTableColumnHeader column={column} label="Size" />,
      cell: ({ row }) => (
        <span className="tabular-nums text-muted-foreground">
          {formatBytes(row.original.sizeBytes)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Size",
        variant: "number",
        icon: HardDrive,
      },
    },
    {
      // Hidden by default (see `columnVisibility` on the page): the mark is
      // shown beside the name, and this column exists to filter on it
      // (`DATA-12`; `certification` is filterable, sortable and groupable on
      // the server).
      id: "certification",
      accessorKey: "certification",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Certification" />
      ),
      cell: ({ row }) => <CertificationBadge certification={row.original.certification} />,
      enableColumnFilter: true,
      meta: {
        label: "Certification",
        variant: "multiSelect",
        options: certificationOptions,
        icon: BadgeCheck,
        enableGrouping: true,
      },
    },
    {
      // Hidden by default (see `columnVisibility` on the page) but defined so
      // the ID stays filterable, sortable, and copyable when someone needs it.
      id: "id",
      accessorKey: "id",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Asset ID" />
      ),
      cell: ({ row }) => (
        <Copyable value={row.original.id}>
          <span className="font-mono text-xs">{row.original.id}</span>
        </Copyable>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Asset ID",
        placeholder: "Search asset ID…",
        variant: "text",
        icon: Text,
      },
    },
    {
      // Pinned right by the page, so it stays reachable while the rest of
      // the table scrolls sideways.
      //
      // No `accessorKey` on purpose: it carries no data, so it is not
      // sortable, filterable, or groupable, and the settings menu skips it
      // (that list only includes columns with an accessor). A user cannot
      // hide the only visible affordance for these actions.
      id: "actions",
      // Empty visually, but screen readers still announce a column here.
      header: () => <span className="sr-only">Actions</span>,
      cell: ({ row }) => <AssetRowActions asset={row.original} onOpen={onOpen} />,
      enableSorting: false,
      enableHiding: false,
      size: 48,
    },
  ]
}
