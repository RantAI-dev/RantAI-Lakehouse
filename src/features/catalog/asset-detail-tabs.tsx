"use client"

import * as React from "react"
import { usePathname, useSearchParams } from "next/navigation"
import {
  Activity,
  BadgeCheck,
  Columns3,
  GitFork,
  LayoutGrid,
  Rows3,
  ShieldCheck,
  type LucideIcon,
} from "lucide-react"
import {
  keepStripInView,
  LineTabsList,
  type LineTab,
  type LineTabCount,
} from "@/components/patterns/line-tabs"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import type { AssetDetail } from "@/services/contracts/assets"
import { AssetAccess } from "./asset-access"
import { AssetActivity } from "./asset-activity"
import {
  AssetLineage,
  lineageCount,
  useAssetLineage,
  type AssetLineageState,
} from "./asset-lineage"
import { AssetOverview, type AssetTab } from "./asset-overview"
import { AssetQuality } from "./asset-quality"
import { AssetSample } from "./asset-sample"
import { AssetSchema } from "./asset-schema"
import { useIcebergTable } from "./asset-storage"

/** The seven tabs, in order. `?tab=` names any but the first. */
const TABS: { value: AssetTab; label: string; icon: LucideIcon }[] = [
  { value: "overview", label: "Overview", icon: LayoutGrid },
  { value: "schema", label: "Schema", icon: Columns3 },
  { value: "sample", label: "Sample", icon: Rows3 },
  { value: "quality", label: "Quality", icon: BadgeCheck },
  { value: "access", label: "Access", icon: ShieldCheck },
  { value: "lineage", label: "Lineage", icon: GitFork },
  { value: "activity", label: "Activity", icon: Activity },
]

function parseTab(raw: string | null): AssetTab {
  return TABS.find((t) => t.value === raw)?.value ?? "overview"
}

/**
 * The count a tab carries, or `null` for one with nothing to count: the
 * overview summarizes, a sample is always a handful of rows, and activity
 * mixes snapshots, changes and usage into no single number.
 */
function tabCount(
  a: AssetDetail,
  lineage: AssetLineageState,
  tab: AssetTab
): LineTabCount | null {
  switch (tab) {
    case "schema":
      return { count: a.schema.length }
    case "quality": {
      const failed = a.qualityChecks.some((q) => q.status === "failed")
      const warning = a.qualityChecks.some((q) => q.status === "warning")
      return {
        count: a.qualityChecks.length,
        tone: failed ? "danger" : warning ? "warning" : undefined,
      }
    }
    case "access":
      return { count: a.policySummary.length }
    case "lineage":
      // No number until the graph is in: a count that jumps reads as a change.
      return lineage.status === "loading" ? null : { count: lineageCount(a, lineage) }
    default:
      return null
  }
}

/**
 * Tab strip for the asset detail page. The open tab lives in `?tab=`, so a
 * refresh or a shared link lands on it.
 */
export function AssetDetailTabs({
  asset: a,
  onAssetChanged,
}: {
  asset: AssetDetail
  /** Reloads the asset after something on a tab changed it. */
  onAssetChanged: () => void
}) {
  const pathname = usePathname()
  const searchParams = useSearchParams()
  const urlTab = parseTab(searchParams.get("tab"))
  const [tab, setTab] = React.useState<AssetTab>(urlTab)
  // A link to another `?tab=` of this same page changes only the URL: the
  // page stays mounted, so the URL's tab is taken over here, during render.
  const [seenUrlTab, setSeenUrlTab] = React.useState(urlTab)
  if (urlTab !== seenUrlTab) {
    setSeenUrlTab(urlTab)
    setTab(urlTab)
  }
  const rootRef = React.useRef<HTMLDivElement>(null)
  const iceberg = useIcebergTable(a)
  const lineage = useAssetLineage(a)

  const selectTab = React.useCallback(
    (next: AssetTab) => {
      setTab(next)
      const params = new URLSearchParams(searchParams.toString())
      if (next === "overview") params.delete("tab")
      else params.set("tab", next)
      const query = params.toString()
      // The History API rather than the router: Next keeps
      // `useSearchParams` in step with it, and a tab switch needs no
      // server round trip.
      window.history.replaceState(null, "", query ? `${pathname}?${query}` : pathname)
      keepStripInView(rootRef.current)
    },
    [pathname, searchParams]
  )

  const stripTabs: LineTab[] = TABS.map((t) => ({ ...t, count: tabCount(a, lineage, t.value) }))

  return (
    <div ref={rootRef}>
      <Tabs value={tab} onValueChange={(v) => selectTab(v as AssetTab)} className="gap-3">
        <LineTabsList tabs={stripTabs} active={tab} />

        <TabsContent value="overview">
          <AssetOverview
            asset={a}
            iceberg={iceberg}
            lineage={lineage}
            onNavigate={selectTab}
            onAssetChanged={onAssetChanged}
          />
        </TabsContent>

        <TabsContent value="schema">
          <AssetSchema asset={a} iceberg={iceberg} />
        </TabsContent>

        <TabsContent value="sample">
          <AssetSample asset={a} iceberg={iceberg} />
        </TabsContent>

        <TabsContent value="quality">
          <AssetQuality asset={a} onChanged={onAssetChanged} />
        </TabsContent>

        <TabsContent value="access">
          <AssetAccess asset={a} onChanged={onAssetChanged} />
        </TabsContent>

        <TabsContent value="lineage">
          <AssetLineage asset={a} state={lineage} />
        </TabsContent>

        <TabsContent value="activity">
          <AssetActivity asset={a} iceberg={iceberg} onChanged={onAssetChanged} />
        </TabsContent>
      </Tabs>
    </div>
  )
}
