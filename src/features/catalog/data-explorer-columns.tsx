"use client"

import type { ColumnDef } from "@tanstack/react-table"
import {
  Boxes,
  CalendarClock,
  Database,
  HardDrive,
  Layers,
  Tag,
  Text,
} from "lucide-react"

import { Copyable } from "@/components/copyable"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { AssetRowActions } from "./data-explorer-actions"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { TierBadge } from "@/components/patterns/status-badge"
import { formatBytes } from "@/lib/format"
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

const typeOptions = Object.entries(ASSET_TYPE_LABEL).map(([value, label]) => ({
  label,
  value,
}))

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
      // Just the name. The namespace used to repeat here as a subtitle, but
      // it already has its own sortable, filterable, copyable column — the
      // second copy only widened the row and gave the same value two places
      // to be copied from.
      cell: ({ row }) => (
        // Capped and truncated: asset names run long ("Kunjungan Daya
        // Tarik Wisata"), and left to itself this column took ~270px —
        // enough to push the last columns off a narrow screen on its own.
        // The full name is still available on hover and on the detail page.
        <Copyable value={row.original.name} className="max-w-[12rem] xl:max-w-[22rem]">
          <span
            className="truncate font-medium tracking-tight text-foreground"
            title={row.original.name}
          >
            {row.original.name}
          </span>
        </Copyable>
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
      cell: ({ row }) => (
        <span className="text-sm">
          {DATA_LAYER_LABEL[row.original.layer as DataLayer] ?? row.original.layer}
        </span>
      ),
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
        <DataTableColumnHeader column={column} label="Freshness" />
      ),
      cell: ({ row }) => (
        <FreshnessIndicator lagSeconds={row.original.freshnessLagSeconds} />
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
