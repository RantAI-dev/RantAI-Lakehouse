"use client"

import * as React from "react"
import Link from "next/link"
import { usePathname } from "next/navigation"
import {
  Activity,
  ArrowRight,
  BadgeCheck,
  ChevronLeft,
  Copy,
  FileUp,
  MoreHorizontal,
  Pencil,
  ShieldCheck,
  SquareTerminal,
  Table2,
  Trash2,
  Users,
  type LucideIcon,
} from "lucide-react"

import {
  ClassificationBadge,
  CertificationBadge,
  HealthBadge,
  TierBadge,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { useAuth } from "@/features/auth/auth-provider"
import { assetHasReadableTable, assetQueryStudioHref } from "@/lib/asset-query"
import { copyTextWithToast } from "@/lib/copy-text"
import { formatNumber, formatRelativeTime } from "@/lib/format"
import { DATA_LAYER_LABEL } from "@/lib/status"
import { cn } from "@/lib/utils"
import { ASSET_TYPE_LABEL, type AssetDetail } from "@/services/contracts/assets"
import { EditDetailsDialog } from "./asset-about"
import { classificationTitle, healthTitle, layerTitle, tierTitle } from "./asset-badges"
import { AssetConnectDialog } from "./asset-connect-dialog"
import { CertificationDialog, DeprecationNotice } from "./asset-certification"
import { canDropTable, useDropTable } from "./asset-drop"
import { lineageKey } from "./asset-lineage"
import type { MeaningState } from "./asset-meaning"
import { MeaningBadge } from "./asset-meaning-view"
import { qualitySummary, type AssetTab } from "./asset-overview"
import { icebergTableOf, type IcebergTableState } from "./asset-storage"
import { layerDot } from "./catalog-layers"
import { OpenFormatCard } from "./open-format-card"

/**
 * The asset page's header (`UX-8`, mockup A): the name and its certified
 * mark; the table's address (copy), layer, tier, classification, type,
 * health and meaning on one line; the description; and on the right the ⋯
 * with every action on the table and the primary **Query**.
 *
 * The ⋯ holds what was spread over the header and the tabs: Certification
 * (was a header button), Edit details (the About card's Edit), Publish in
 * open format (a Gold card on the Overview), the two copies, and Drop table
 * (the Snapshots card's button). Each keeps the permission it had.
 */
export function AssetHeader({
  asset: a,
  iceberg,
  meanings,
  missingPermissions,
  onRequestAccess,
  onChanged,
}: {
  asset: AssetDetail
  iceberg: IcebergTableState
  meanings: MeaningState
  /** The permissions the Request access dialog would offer; none hides it. */
  missingPermissions: readonly string[]
  onRequestAccess: () => void
  /** Reloads the asset after an action changed it. */
  onChanged: () => void
}) {
  const { hasPermission } = useAuth()
  const [dialog, setDialog] = React.useState<"certification" | "edit" | "publish" | null>(null)
  // P2 `reczz28d1eZjrBAZ` (merged from main): the connect dialog is a separate
  // header action, a compact trigger rather than inline connection details.
  const [connectOpen, setConnectOpen] = React.useState(false)
  const table = icebergTableOf(iceberg)
  // The primary opens Query Studio only on a table the console can read and
  // for a reader who may run a query; otherwise the page has no primary and
  // the Overview says why (owner, 2026-11-09).
  const canQuery =
    hasPermission("query:read") && assetHasReadableTable(a, table, iceberg.status === "loading")
  const drop = useDropTable(a, {
    enabled: canDropTable(a, iceberg, hasPermission),
    onDropped: iceberg.reload,
  })
  const address = lineageKey(a)
  const dot = layerDot(a.layer)
  const certify = hasPermission("governance:write")
  const edit = hasPermission("catalog:write")
  // DATA-1/DATA-65: Gold marts and Silver tables publish in open format; the
  // card itself says who may switch it. UX-8 moved publishing here from the
  // Overview, so the Silver affordance DATA-65 added there lives on this menu.
  const publish = a.layer === "gold" || a.layer === "silver"

  return (
    <header className="flex flex-col gap-1.5">
      {/* The way back to the list, as the header had it: the navbar's
          breadcrumb does not name the Catalog on this route. */}
      <Link
        href="/catalog"
        className="inline-flex items-center gap-1 self-start rounded-sm text-xs text-muted-foreground outline-none transition-colors duration-150 ease-ui hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50"
      >
        <ChevronLeft className="size-3.5" aria-hidden />
        Catalog
      </Link>
      <div className="flex items-start gap-2">
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1">
          <h1 className="min-w-0 text-xl leading-8 font-semibold tracking-tight break-words">{a.name}</h1>
          <CertificationBadge certification={a.certification} />
        </div>
        <DropdownMenu>
          <Tooltip>
            <TooltipTrigger
              render={
                <DropdownMenuTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label="More actions for this asset"
                    className="text-muted-foreground data-[state=open]:bg-muted"
                  >
                    <MoreHorizontal />
                  </Button>
                </DropdownMenuTrigger>
              }
            />
            <TooltipContent>More</TooltipContent>
          </Tooltip>
          <DropdownMenuContent align="end" className="w-56">
            {certify ? (
              <DropdownMenuItem onSelect={() => setDialog("certification")}>
                <BadgeCheck className="text-muted-foreground" />
                Certification…
              </DropdownMenuItem>
            ) : null}
            {edit ? (
              <DropdownMenuItem onSelect={() => setDialog("edit")}>
                <Pencil className="text-muted-foreground" />
                Edit details…
              </DropdownMenuItem>
            ) : null}
            {publish ? (
              <DropdownMenuItem onSelect={() => setDialog("publish")}>
                <FileUp className="text-muted-foreground" />
                Publish in open format…
              </DropdownMenuItem>
            ) : null}
            {certify || edit || publish ? <DropdownMenuSeparator /> : null}
            <DropdownMenuItem onSelect={() => void copyTextWithToast(address, "Table name copied")}>
              <Table2 className="text-muted-foreground" />
              Copy table name
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => void copyTextWithToast(a.id, "Asset ID copied")}>
              <Copy className="text-muted-foreground" />
              Copy asset ID
            </DropdownMenuItem>
            {drop.droppable ? (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem variant="destructive" onSelect={drop.open}>
                  <Trash2 />
                  Drop table…
                </DropdownMenuItem>
              </>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>
        <Button variant="outline" onClick={() => setConnectOpen(true)}>
          Connect
        </Button>
        {missingPermissions.length > 0 ? (
          // Where access is missing, once: the Sample and Lineage tabs'
          // empty states point back up here.
          <Button variant="outline" onClick={onRequestAccess}>
            Request access
          </Button>
        ) : null}
        {canQuery ? (
          <Button
            className="max-sm:w-8 max-sm:px-0"
            render={<Link href={assetQueryStudioHref(a, table)} />}
          >
            <SquareTerminal data-icon="inline-start" aria-hidden />
            <span className="max-sm:sr-only">Query</span>
          </Button>
        ) : null}
      </div>

      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px] text-muted-foreground">
        <span className="inline-flex min-w-0 items-center gap-0.5">
          {/* The address is code (plan decision 3): the one monospace run in the header. */}
          <span className="truncate font-mono text-xs text-foreground/80" title={address}>
            {address}
          </span>
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  variant="ghost"
                  size="icon-xs"
                  aria-label="Copy table name"
                  className="text-muted-foreground"
                  onClick={() => void copyTextWithToast(address, "Table name copied")}
                >
                  <Copy />
                </Button>
              }
            />
            <TooltipContent>Copy table name</TooltipContent>
          </Tooltip>
        </span>
        <span aria-hidden>·</span>
        <span className="inline-flex items-center gap-1.5 text-foreground/80" title={layerTitle(a)}>
          {dot ? <span aria-hidden className={cn("size-2 rounded-full", dot)} /> : null}
          {DATA_LAYER_LABEL[a.layer] ?? a.layer}
        </span>
        <TierBadge tier={a.tier} title={tierTitle(a)} />
        <ClassificationBadge classification={a.classification} title={classificationTitle(a)} />
        <span aria-hidden>·</span>
        <span>{ASSET_TYPE_LABEL[a.type] ?? a.type}</span>
        <HealthBadge health={a.health} title={healthTitle(a)} />
        <MeaningBadge state={meanings} columns={a.schema.map((c) => c.name)} />
      </div>

      {a.description ? (
        <p className="line-clamp-2 max-w-3xl text-sm leading-6 text-foreground/80" title={a.description}>
          {a.description}
        </p>
      ) : null}
      <DeprecationNotice asset={a} replacementHidden={a.shownWithTableGrant === true} />

      {dialog === "certification" ? (
        <CertificationDialog
          asset={a}
          onClose={() => setDialog(null)}
          onSaved={() => {
            setDialog(null)
            onChanged()
          }}
        />
      ) : null}
      {dialog === "edit" ? (
        <EditDetailsDialog
          asset={a}
          open
          onClose={() => setDialog(null)}
          onSaved={() => {
            setDialog(null)
            onChanged()
          }}
        />
      ) : null}
      <Dialog open={dialog === "publish"} onOpenChange={(next) => (next ? undefined : setDialog(null))}>
        <DialogContent className="sm:max-w-2xl">
          <DialogTitle className="sr-only">Publish in open format</DialogTitle>
          <div className="max-h-[75vh] overflow-y-auto">
            <OpenFormatCard assetId={a.id} />
          </div>
        </DialogContent>
      </Dialog>
      {/* Mounted only while open: its snippets need a `zone.table` id, which a header on an asset without one has no use for. */}
      {connectOpen ? <AssetConnectDialog asset={a} open onOpenChange={setConnectOpen} /> : null}
      {drop.dialog}
    </header>
  )
}

/** One trust fact: what it is, its value, and the tab that tells the rest. */
type Fact = { key: string; label: string; value: string; icon: LucideIcon; tone: "good" | "warn" | "bad" | "quiet"; tab: AssetTab }

const TONE: Record<Fact["tone"], string> = {
  good: "text-emerald-600 dark:text-emerald-400",
  warn: "text-amber-600 dark:text-amber-400",
  bad: "text-destructive",
  quiet: "text-muted-foreground",
}

/** The four facts of the strip, from the asset alone. A dash is a fact nothing measures yet. */
export function trustFacts(a: AssetDetail, canRead: boolean): Fact[] {
  const late =
    a.freshnessLagSeconds !== null &&
    a.freshnessTargetSeconds !== null &&
    a.freshnessTargetSeconds !== undefined &&
    a.freshnessLagSeconds > a.freshnessTargetSeconds
  const q = qualitySummary(a.qualityChecks)
  const masked = a.schema.filter((c) => c.masked).length
  return [
    {
      key: "updated",
      label: "Updated",
      value:
        a.lastUpdated === null
          ? "—"
          : `${formatRelativeTime(a.lastUpdated)}${late ? " · late" : ""}`,
      icon: Activity,
      tone: a.lastUpdated === null ? "quiet" : late ? "warn" : "good",
      tab: "activity",
    },
    {
      key: "quality",
      label: "Quality",
      value:
        a.qualityChecks.length === 0
          ? "No checks"
          : q.evaluated === 0
            ? `${a.qualityChecks.length} rule${a.qualityChecks.length === 1 ? "" : "s"}, not run yet`
            : `${q.passed} of ${q.evaluated} passed${q.failed > 0 ? ` · ${q.failed} failed` : q.warning > 0 ? ` · ${q.warning} warning${q.warning === 1 ? "" : "s"}` : ""}`,
      icon: BadgeCheck,
      tone: q.failed > 0 ? "bad" : q.warning > 0 ? "warn" : q.evaluated > 0 ? "good" : "quiet",
      tab: "quality",
    },
    {
      key: "used",
      label: "Used",
      value:
        a.usage === null
          ? "—"
          : `${formatNumber(a.usage.queries)} quer${a.usage.queries === 1 ? "y" : "ies"} in ${a.usage.days} days`,
      icon: Users,
      tone: a.usage === null ? "quiet" : "good",
      tab: "activity",
    },
    {
      key: "access",
      label: "Access",
      value: canRead
        ? masked > 0
          ? `You can read · ${masked} masked`
          : "You can read · nothing masked"
        : "You can see it, not read its rows",
      icon: ShieldCheck,
      tone: canRead ? (masked > 0 ? "warn" : "good") : "quiet",
      tab: "access",
    },
  ]
}

/**
 * One strip of four trust facts under the header (`UX-8`, mockup A):
 * Updated, Quality, Used and Access, each a link to the tab with the rest.
 * It replaces the facts strip and the Overview's four tiles. What nothing
 * measures yet is a dash, and that is said once, under the strip.
 */
export function TrustStrip({ asset: a }: { asset: AssetDetail }) {
  const pathname = usePathname()
  const { hasPermission } = useAuth()
  const facts = trustFacts(a, hasPermission("query:read"))
  const unmeasured = facts.filter((f) => f.value === "—").map((f) => f.label)
  return (
    <div className="flex flex-col gap-1.5">
      <div className="grid grid-cols-2 gap-px overflow-hidden rounded-xl bg-border/60 lg:grid-cols-4">
        {facts.map((f) => (
          <Link
            key={f.key}
            href={`${pathname}?tab=${f.tab}`}
            scroll={false}
            className="group/fact flex min-w-0 flex-col gap-0.5 bg-background px-3.5 py-2.5 outline-none transition-colors duration-200 ease-ui hover:bg-muted/50 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:ring-inset"
          >
            <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <f.icon className={cn("size-3.5", TONE[f.tone])} aria-hidden />
              {f.label}
              <ArrowRight
                aria-hidden
                className="ml-auto size-3.5 -translate-x-1 opacity-0 transition-[opacity,translate] duration-200 ease-ui group-hover/fact:translate-x-0 group-hover/fact:opacity-100"
              />
            </span>
            <span className="truncate text-sm font-medium tabular-nums" title={f.value}>
              {f.value}
            </span>
          </Link>
        ))}
      </div>
      {unmeasured.length > 0 ? (
        <p className="text-xs text-muted-foreground">
          {`A dash is not measured yet: ${unmeasured.join(" and ")} ${unmeasured.length === 1 ? "appears" : "appear"} once a load reports when it wrote and Query Studio or AI Copilot read the table.`}
        </p>
      ) : null}
    </div>
  )
}

/**
 * The page while the asset loads, shaped like it (`DESIGN.md` §3, one
 * loading state): header, strip, tabs, then the readme and its side.
 */
export function AssetPageSkeleton() {
  return (
    <div role="status" aria-label="Loading the asset" className="flex flex-col gap-4">
      <div className="flex items-start gap-2">
        <div className="flex flex-1 flex-col gap-2">
          <Skeleton className="h-7 w-72" />
          <Skeleton className="h-4 w-96 max-w-full" />
          <Skeleton className="h-4 w-[28rem] max-w-full" />
        </div>
        <Skeleton className="size-8 rounded-lg" />
        <Skeleton className="h-8 w-20 rounded-lg" />
      </div>
      <Skeleton className="h-[60px] w-full rounded-xl" />
      <div className="flex gap-5 border-b border-border pb-2">
        {[16, 18, 14, 14, 14, 14, 16].map((w, i) => (
          <Skeleton key={i} className="h-4" style={{ width: `${w * 4}px` }} />
        ))}
      </div>
      <div className="grid gap-8 lg:grid-cols-[minmax(0,1fr)_300px]">
        <div className="flex flex-col gap-2">
          {Array.from({ length: 6 }, (_, i) => (
            <Skeleton key={i} className="h-11 w-full" />
          ))}
        </div>
        <div className="flex flex-col gap-2">
          {Array.from({ length: 8 }, (_, i) => (
            <Skeleton key={i} className="h-4 w-full" />
          ))}
        </div>
      </div>
    </div>
  )
}
