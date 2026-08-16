"use client"

import * as React from "react"
import Link from "next/link"
import { PlusIcon, RotateCcwIcon } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { PageHeader } from "@/components/patterns/page-header"
import { MetricCard, MetricGrid } from "@/components/patterns/metric-card"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  ErrorState,
  LoadingSkeleton,
  MetricSkeleton,
} from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill, StatusBadge, TierBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatBytes, formatPercent, formatRelativeTime } from "@/lib/format"
import {
  DATA_ARCH_COMPONENT_LABEL,
  DATA_LAYER_ARCH,
  DATA_LAYER_LABEL,
  DATA_LAYER_LABEL_ID,
  STORAGE_TIER_LABEL,
  type DataLayer,
  type StorageTier,
} from "@/lib/status"
import { storageService } from "@/services"
import type { LifecyclePolicy, TieringOp } from "@/services/contracts/storage"

const TIERS: StorageTier[] = ["hot", "warm", "cold", "ai"]
const DATA_LAYERS = Object.keys(DATA_LAYER_LABEL) as DataLayer[]

const policyCols: ColumnDef<LifecyclePolicy>[] = [
  { key: "name", header: "Policy", render: (r) => r.name },
  { key: "scope", header: "Scope", render: (r) => r.scope },
  {
    key: "rules",
    header: "Hot → Warm → Cold",
    render: (r) => `${r.hotDays}d → ${r.warmDays}d → ${r.coldAfterDays}d+`,
  },
  {
    key: "status",
    header: "Status",
    render: (r) => <StatusBadge status={r.status} />,
  },
  { key: "savings", header: "Savings", render: (r) => r.estimatedSavings },
  {
    key: "applied",
    header: "Last applied",
    render: (r) => (
      <span className="text-muted-foreground">
        {formatRelativeTime(r.lastAppliedAt)}
      </span>
    ),
  },
]

function opCols(): ColumnDef<TieringOp>[] {
  return [
    {
      key: "asset",
      header: "Asset",
      render: (r) =>
        r.assetId ? (
          <Link
            href={`/data/assets/${r.assetId}`}
            className="font-medium hover:underline"
          >
            {r.asset}
          </Link>
        ) : (
          r.asset
        ),
    },
    {
      key: "move",
      header: "Move",
      render: (r) => (
        <span className="inline-flex items-center gap-1">
          <TierBadge tier={r.from} />
          <span className="text-muted-foreground">→</span>
          <TierBadge tier={r.to} />
        </span>
      ),
    },
    {
      key: "status",
      header: "Status",
      render: (r) => <StatusBadge status={r.status} />,
    },
    { key: "at", header: "When", render: (r) => formatRelativeTime(r.at) },
    { key: "detail", header: "Detail", render: (r) => r.detail },
  ]
}

export function StoragePage() {
  const overview = useService((s) => storageService.getOverview(s), [])
  const policies = useService((s) => storageService.listPolicies(s), [])
  const ops = useService((s) => storageService.listOperations(s), [])
  const [createOpen, setCreateOpen] = React.useState(false)
  const [restoreOpen, setRestoreOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [scope, setScope] = React.useState("")
  const [hotDays, setHotDays] = React.useState("")
  const [warmDays, setWarmDays] = React.useState("")
  const [coldAfterDays, setColdAfterDays] = React.useState("")
  const create = useServiceAction(
    (signal, input: Parameters<typeof storageService.createLifecyclePolicy>[0]) =>
      storageService.createLifecyclePolicy(input, signal)
  )
  const restore = useServiceAction(
    (signal, input: Parameters<typeof storageService.restoreAsset>[0]) =>
      storageService.restoreAsset(input, signal)
  )

  function resetForm() {
    setName("")
    setScope("")
    setHotDays("")
    setWarmDays("")
    setColdAfterDays("")
  }

  async function handleCreate() {
    const result = await create.run({
      name: name.trim(),
      scope: scope.trim(),
      hotDays: Number(hotDays),
      warmDays: Number(warmDays),
      coldAfterDays: Number(coldAfterDays),
    })
    if (result) {
      setCreateOpen(false)
      resetForm()
      policies.reload()
    }
  }

  const canSubmit = Boolean(
    name.trim() &&
      scope.trim() &&
      hotDays.trim() &&
      warmDays.trim() &&
      coldAfterDays.trim() &&
      !Number.isNaN(Number(hotDays)) &&
      !Number.isNaN(Number(warmDays)) &&
      !Number.isNaN(Number(coldAfterDays))
  )

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Storage Lifecycle"
        description="Physical Hot, Warm, Cold, and AI tiers control where bytes live — separate from logical Raw, Bronze, Silver, and Gold modeling layers in the catalog."
        actions={
          <>
            <Button size="sm" variant="outline" onClick={() => setRestoreOpen(true)}>
              <RotateCcwIcon data-icon="inline-start" />
              Restore to Hot
            </Button>
            <Button size="sm" onClick={() => setCreateOpen(true)}>
              <PlusIcon data-icon="inline-start" />
              Create Lifecycle Policy
            </Button>
          </>
        }
      />
      {overview.status === "loading" ? <MetricSkeleton /> : null}
      {overview.status === "error" ? (
        <ErrorState error={overview.error} onRetry={overview.reload} />
      ) : null}
      {overview.status === "success" ? (
        <>
          <MetricGrid>
            {TIERS.map((t) => (
              <MetricCard
                key={t}
                label={STORAGE_TIER_LABEL[t]}
                value={formatBytes(overview.data.byTier[t].bytes)}
                hint={`${overview.data.byTier[t].assets} assets · ${formatPercent(overview.data.byTier[t].growth7d)} 7d growth`}
              />
            ))}
          </MetricGrid>
          <MetricGrid className="lg:grid-cols-3">
            <MetricCard
              label="Savings vs all-hot"
              value={formatPercent(overview.data.savingsVsAllHot)}
            />
            <MetricCard
              label="Failed tiering ops"
              value={overview.data.failedTieringOps}
              trendTone="negative"
            />
            <MetricCard label="Pending restores" value={overview.data.pendingRestores} />
          </MetricGrid>
          <SectionCard
            title="Physical storage tiers"
            description="Lifecycle path for where data is stored. Logical modeling layers (Raw → Gold) are a separate catalog dimension."
          >
            <div className="flex flex-wrap items-center gap-2">
              <TierBadge tier="hot" />
              <span className="text-muted-foreground" aria-hidden>
                →
              </span>
              <TierBadge tier="warm" />
              <span className="text-muted-foreground" aria-hidden>
                →
              </span>
              <TierBadge tier="cold" />
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <span className="text-muted-foreground" aria-hidden>
                →
              </span>
              <TierBadge tier="ai" />
              <span className="text-xs text-muted-foreground">
                derivative datasets rebuilt from lineage
              </span>
            </div>
            <div className="mt-4 space-y-2">
              <p className="text-xs text-muted-foreground">
                Three orthogonal axes: <strong>maturity</strong> (Bronze→Gold /
                Mentah→Siap-Pakai), <strong>access temperature</strong> (Hot / Warm /
                Cold), and <strong>architecture component</strong> (Data Lake →
                Warehouse → Mart). A Gold dataset may still span Hot, Warm, and Cold.
              </p>
              <div className="flex flex-wrap items-center gap-2">
                {DATA_LAYERS.map((layer) => (
                  <Pill key={layer} tone="neutral">
                    {DATA_LAYER_LABEL[layer]} · {DATA_LAYER_LABEL_ID[layer]}
                    <span className="text-muted-foreground/70">
                      {" "}
                      — {DATA_ARCH_COMPONENT_LABEL[DATA_LAYER_ARCH[layer]]}
                    </span>
                  </Pill>
                ))}
              </div>
            </div>
          </SectionCard>
        </>
      ) : null}

      <SectionCard title="Lifecycle policies">
        {policies.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
        {policies.status === "error" ? (
          <ErrorState error={policies.error} onRetry={policies.reload} />
        ) : null}
        {policies.status === "success" ? (
          <DataTable columns={policyCols} rows={policies.data} rowKey={(r) => r.id} />
        ) : null}
      </SectionCard>

      <SectionCard title="Recent tiering operations">
        {ops.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
        {ops.status === "error" ? (
          <ErrorState error={ops.error} onRetry={ops.reload} />
        ) : null}
        {ops.status === "success" ? (
          <DataTable columns={opCols()} rows={ops.data} rowKey={(r) => r.id} />
        ) : null}
      </SectionCard>

      <ConfirmActionDialog
        open={restoreOpen}
        onOpenChange={setRestoreOpen}
        title="Restore to Hot"
        description="Rehydrate lake.sales.orders_history from Cold into Hot for interactive query access."
        impact="Restore jobs compete with tiering bandwidth. Large partitions may take several minutes."
        confirmLabel="Start restore"
        confirming={restore.status === "pending"}
        onConfirm={async () => {
          const result = await restore.run({
            assetId: "ice-orders-history",
            assetName: "lake.sales.orders_history",
            from: "cold",
            to: "hot",
          })
          if (result) {
            setRestoreOpen(false)
            ops.reload()
            overview.reload()
          }
        }}
      />

      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="Create Lifecycle Policy"
        description="Define hot, warm, and cold tiering thresholds for a scope."
        canSubmit={canSubmit}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
      >
        <div className="space-y-1.5">
          <Label htmlFor="lp-name">Name</Label>
          <Input id="lp-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="lp-scope">Scope</Label>
          <Input
            id="lp-scope"
            value={scope}
            onChange={(e) => setScope(e.target.value)}
            placeholder="lakehouse/analytics/*"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="lp-hot">Hot days</Label>
          <Input
            id="lp-hot"
            type="number"
            min={0}
            value={hotDays}
            onChange={(e) => setHotDays(e.target.value)}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="lp-warm">Warm days</Label>
          <Input
            id="lp-warm"
            type="number"
            min={0}
            value={warmDays}
            onChange={(e) => setWarmDays(e.target.value)}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="lp-cold">Cold after days</Label>
          <Input
            id="lp-cold"
            type="number"
            min={0}
            value={coldAfterDays}
            onChange={(e) => setColdAfterDays(e.target.value)}
          />
        </div>
      </CreateSheet>
    </div>
  )
}
