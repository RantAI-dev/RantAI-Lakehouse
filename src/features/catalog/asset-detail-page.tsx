"use client"

import Link from "next/link"
import { useParams } from "next/navigation"
import { EntityHeader } from "@/components/patterns/page-header"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { MetadataList } from "@/components/patterns/metadata-list"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import {
  ClassificationBadge,
  HealthBadge,
  TierBadge,
} from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatBytes, formatCompactNumber, formatRelativeTime } from "@/lib/format"
import { DATA_LAYER_LABEL, ENGINE_CATEGORY_LABEL } from "@/lib/status"
import { assetService } from "@/services"
import { ASSET_TYPE_LABEL } from "@/services/contracts/assets"
import { AssetDetailTabs } from "./asset-detail-tabs"

/** Asset detail with schema, freshness, lineage hops, policies, and snapshots. */
export function AssetDetailPage() {
  const params = useParams<{ assetId: string }>()
  const state = useService(
    (s) => assetService.getAsset(params.assetId, s),
    [params.assetId]
  )

  if (state.status === "loading") return <LoadingSkeleton rows={8} />
  if (state.status === "error")
    return <ErrorState error={state.error} onRetry={state.reload} />
  const a = state.data

  return (
    <div className="flex flex-col gap-3">
      <EntityHeader
        className="pb-3"
        eyebrow={<Link href="/data" className="hover:underline">Data Explorer</Link>}
        title={a.name}
        titleAccessory={
          <>
            <TierBadge tier={a.tier} />
            <ClassificationBadge classification={a.classification} />
            <HealthBadge health={a.health} />
          </>
        }
        description={a.description}
      />
      <MetadataList
        density="compact"
        columns={3}
        items={[
          { label: "Namespace", value: <span className="font-mono text-xs">{a.namespace}</span> },
          { label: "Type", value: ASSET_TYPE_LABEL[a.type] },
          { label: "Layer", value: DATA_LAYER_LABEL[a.layer] },
          { label: "Format", value: a.format },
          { label: "Engine", value: ENGINE_CATEGORY_LABEL[a.engine] },
          { label: "Rows", value: formatCompactNumber(a.rows) },
          { label: "Size", value: formatBytes(a.sizeBytes) },
          { label: "Owner", value: a.owner },
          { label: "Residency", value: a.residency },
          { label: "Lifecycle", value: a.lifecyclePolicy },
          {
            label: "Freshness",
            value: <FreshnessIndicator lagSeconds={a.freshnessLagSeconds} />,
          },
          { label: "Updated", value: formatRelativeTime(a.lastUpdated) },
        ]}
      />
      <AssetDetailTabs asset={a} />
    </div>
  )
}
