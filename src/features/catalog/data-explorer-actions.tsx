"use client"

import * as React from "react"
import { Boxes, Copy, ExternalLink, MoreHorizontal } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { notifyError, notifySuccess } from "@/lib/notify"
import type { Asset } from "@/services/contracts/assets"

/**
 * What a row lets you do, defined once.
 *
 * The Data Explorer offers these in two places — the ⋮ button and the
 * right-click menu — which are built from different primitives and so have
 * to be *rendered* separately. Describing the actions as data means only
 * the rendering differs: adding an action here makes it appear in both,
 * instead of being written twice and drifting apart the first time someone
 * updates one of them.
 *
 * Everything here is read-only. The catalog API exposes no mutations
 * (`GET /api/catalog*` only), so there is deliberately no Edit or Delete;
 * `catalog:write` exists as a permission but no route consumes it yet.
 */
export interface AssetAction {
  id: string
  label: string
  icon: React.ComponentType<{ className?: string }>
  onSelect: () => void
  /** Renders a divider above this item. */
  separatorBefore?: boolean
}

async function copyToClipboard(value: string, label: string) {
  try {
    await navigator.clipboard.writeText(value)
    notifySuccess(`${label} copied`)
  } catch (err) {
    // Usually a denied clipboard permission or a non-secure origin.
    notifyError(`Failed to copy ${label.toLowerCase()}`, err)
  }
}

export function getAssetActions(
  asset: Asset,
  { onOpen }: { onOpen: (asset: Asset) => void }
): AssetAction[] {
  return [
    {
      id: "open",
      label: "Open asset",
      icon: ExternalLink,
      onSelect: () => onOpen(asset),
    },
    {
      id: "copy-id",
      label: "Copy asset ID",
      icon: Copy,
      separatorBefore: true,
      onSelect: () => void copyToClipboard(asset.id, "Asset ID"),
    },
    {
      id: "copy-namespace",
      label: "Copy namespace",
      icon: Boxes,
      onSelect: () => void copyToClipboard(asset.namespace, "Namespace"),
    },
  ]
}

/**
 * The ⋮ button in the pinned `actions` column.
 *
 * The context menu on the row is faster once you know it is there, but it
 * is also invisible — nothing on screen suggests a right-click does
 * anything. This is the discoverable half of the same menu.
 */
export function AssetRowActions({
  asset,
  onOpen,
}: {
  asset: Asset
  onOpen: (asset: Asset) => void
}) {
  const actions = getAssetActions(asset, { onOpen })

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`Actions for ${asset.name}`}
          className="text-muted-foreground data-[state=open]:bg-muted"
          // The whole row carries an `onRowClick` that opens the asset, so
          // without this the click that opens this menu also navigates
          // away from the page — the menu would flash and vanish.
          onClick={(event) => event.stopPropagation()}
        >
          <MoreHorizontal />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        // Same reason as the trigger: selecting an item must not bubble up
        // into the row's click handler.
        onClick={(event) => event.stopPropagation()}
      >
        {actions.map((action) => (
          <React.Fragment key={action.id}>
            {action.separatorBefore ? <DropdownMenuSeparator /> : null}
            <DropdownMenuItem onSelect={action.onSelect}>
              <action.icon />
              {action.label}
            </DropdownMenuItem>
          </React.Fragment>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
