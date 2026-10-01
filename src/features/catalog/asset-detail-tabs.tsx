"use client"

import * as React from "react"
import Link from "next/link"
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
import { EmptyState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { assetQueryStudioHref } from "@/lib/asset-query"
import { cn } from "@/lib/utils"
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
import { AssetSchema } from "./asset-schema"
import { useIcebergTable } from "./asset-storage"

type CountTone = "danger" | "warning"

/**
 * How much is behind a tab, so the empty ones can be skipped without a
 * click. A zero is dimmed; failing quality checks turn the count red.
 */
function TabCount({ count, tone }: { count: number; tone?: CountTone }) {
  return (
    <span
      className={cn(
        "min-w-4 rounded-full px-1 text-center text-[11px] leading-4 font-medium tabular-nums",
        tone === "danger"
          ? "bg-destructive/10 text-destructive"
          : tone === "warning"
            ? "bg-amber-500/15 text-amber-700 dark:text-amber-400"
            : count === 0
              ? "text-muted-foreground/50"
              : "bg-muted text-muted-foreground group-data-active/tab:bg-primary/10 group-data-active/tab:text-primary"
      )}
    >
      {count}
    </span>
  )
}

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
): { count: number; tone?: CountTone } | null {
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
 * The app navbar's height (`h-16` in `app-navbar.tsx`): the tab strip
 * sticks right under it.
 */
const STICKY_TOP_PX = 64

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
  const stripRef = React.useRef<HTMLDivElement>(null)
  const iceberg = useIcebergTable(a)
  const lineage = useAssetLineage(a)

  // On a narrow screen the strip scrolls sideways; keep the open tab in
  // it, so a link to `?tab=activity` does not open a tab you cannot see.
  // Sideways only: the page itself never moves for this.
  React.useEffect(() => {
    const strip = stripRef.current
    const active = strip?.querySelector<HTMLElement>("[role=tab][data-active]")
    if (!strip || !active) return
    const s = strip.getBoundingClientRect()
    const t = active.getBoundingClientRect()
    if (t.left < s.left) strip.scrollLeft += t.left - s.left - 16
    else if (t.right > s.right) strip.scrollLeft += t.right - s.right + 16
  }, [tab])
  const sampleColumns = Object.keys(a.sample[0] ?? {})

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
      // Scrolled past the strip, a new tab would open mid-page; start it
      // at the top instead, just under the stuck strip.
      const top = rootRef.current?.getBoundingClientRect().top
      if (top !== undefined && top < STICKY_TOP_PX && window.scrollY > 0) {
        window.scrollTo({ top: window.scrollY + top - STICKY_TOP_PX })
      }
    },
    [pathname, searchParams]
  )

  return (
    <div ref={rootRef}>
      <Tabs value={tab} onValueChange={(v) => selectTab(v as AssetTab)} className="gap-3">
        {/* Sticky under the navbar, bled to the edges of `<main>`'s padding
            (`app-frame.tsx`) so content scrolling underneath never shows
            beside it. Two layers repeat the page's own background: the
            inset's `bg-muted/25` over the body's `bg-background`. */}
        <div className="sticky top-16 z-10 -mx-4 bg-background sm:-mx-5 lg:-mx-6">
          <div
            ref={stripRef}
            className="overflow-x-auto border-b border-border bg-muted/25 px-4 [scrollbar-width:none] sm:px-5 lg:px-6"
          >
            <TabsList
              variant="line"
              className="w-max justify-start gap-0.5 p-0 group-data-horizontal/tabs:h-10"
            >
              {TABS.map(({ value, label, icon: Icon }) => {
                const count = tabCount(a, lineage, value)
                return (
                  <TabsTrigger
                    key={value}
                    value={value}
                    className="group/tab h-full flex-none px-2.5 after:bg-primary group-data-horizontal/tabs:after:bottom-0"
                  >
                    <Icon
                      className="size-3.5 text-muted-foreground group-data-active/tab:text-primary"
                      aria-hidden
                    />
                    {label}
                    {count ? <TabCount count={count.count} tone={count.tone} /> : null}
                  </TabsTrigger>
                )
              })}
            </TabsList>
          </div>
        </div>

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
          <SectionCard
            size="sm"
            title="Sample rows"
            description={
              a.sample.length > 0
                ? `The first ${a.sample.length} row${a.sample.length === 1 ? "" : "s"}, as you would read them: masking and row filters applied.`
                : "Masking and row filters applied."
            }
            action={
              <Button size="sm" variant="outline" render={<Link href={assetQueryStudioHref(a)} />}>
                Open in Query Studio
              </Button>
            }
          >
            {a.sampleRestricted ? (
              <EmptyState
                title="Sample rows need query access"
                description="Rows are data, so they require the query:read permission. Use Request access above to ask for it."
                className="py-4"
              />
            ) : a.sample.length === 0 ? (
              <EmptyState title="No sample rows available" className="py-4" />
            ) : (
              <div className="overflow-hidden rounded-lg border border-border">
                <Table>
                  <TableHeader>
                    <TableRow className="hover:bg-transparent">
                      {sampleColumns.map((col) => (
                        <TableHead key={col} className="font-mono text-xs font-medium">
                          {col}
                        </TableHead>
                      ))}
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {a.sample.map((row, i) => (
                      <TableRow key={i}>
                        {sampleColumns.map((col) => (
                          <TableCell key={col} className="py-1.5 font-mono text-xs">
                            {row[col] ?? "—"}
                          </TableCell>
                        ))}
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
          </SectionCard>
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
          <AssetActivity asset={a} iceberg={iceberg} />
        </TabsContent>
      </Tabs>
    </div>
  )
}
