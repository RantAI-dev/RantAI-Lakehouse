"use client"

import { cn } from "@/lib/utils"
import type { AssetDetail } from "@/services/contracts/assets"

/**
 * What a page opened with a `query:read` grant says where the related
 * panels would be (`GOV-11`, `DEC-20`; review round 1 R2). The API admits
 * such a person to that table's own facts and withholds what names another
 * table or another person, so the panels are empty because they were not
 * sent — saying so is the difference between "nothing exists" and "not
 * shown to you".
 *
 * The flag is the server's: a person may hold `catalog:read` and still be
 * admitted by a grant, and the server decides from how it admitted the
 * caller. The console never re-derives it from `hasPermission`.
 */
export const TABLE_GRANT_NOTE =
  "Shown with a table grant: related tables and usage are hidden"

/** The sentence, in the panels that would otherwise look empty; nothing for a `catalog:read` reader. */
export function TableGrantNote({
  asset,
  className,
}: {
  asset: AssetDetail
  className?: string
}) {
  if (asset.shownWithTableGrant !== true) return null
  return (
    <p
      className={cn(
        "rounded-md border border-dashed border-border px-2 py-1.5 text-xs text-muted-foreground",
        className
      )}
    >
      {TABLE_GRANT_NOTE}
    </p>
  )
}
