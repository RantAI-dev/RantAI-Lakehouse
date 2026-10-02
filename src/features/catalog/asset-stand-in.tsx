import { assetStandInTable } from "@/lib/asset-query"
import type { AssetDetail } from "@/services/contracts/assets"

/**
 * Says so when the rows on this tab are not the asset's own: a Bronze
 * dataset whose Bronze table cannot be read is shown through its Silver
 * model, which is a different table — deduplicated, and with only the
 * columns it conforms. Renders nothing when the page reads the asset
 * itself.
 */
export function StandInNotice({ asset }: { asset: Pick<AssetDetail, "queryTarget" | "tableKey"> }) {
  const table = assetStandInTable(asset)
  if (!table) return null
  return (
    <p
      role="note"
      className="rounded-lg border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground"
    >
      Read from <span className="font-mono">{table}</span>, this dataset&apos;s Silver model: its
      own Bronze table cannot be read here. A model can hold fewer columns and rows than the table
      it is built from.
    </p>
  )
}
