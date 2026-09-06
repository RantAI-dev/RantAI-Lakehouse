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

export const dataExplorerColumns: ColumnDef<Asset>[] = [
  {
    id: "name",
    accessorKey: "name",
    header: ({ column }) => <DataTableColumnHeader column={column} label="Name" />,
    cell: ({ row }) => (
      <div className="min-w-0 py-0.5">
        <Copyable value={row.original.name}>
          <span className="font-medium tracking-tight text-foreground">
            {row.original.name}
          </span>
        </Copyable>
        {/* The namespace is what people paste into SQL, so it gets its own
            copy affordance rather than being decoration. */}
        <Copyable
          value={row.original.namespace}
          className="mt-0.5 text-xs text-muted-foreground"
        >
          <span className="font-mono">{row.original.namespace}</span>
        </Copyable>
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
]
