"use client"

import * as React from "react"
import Link from "next/link"
import { Timer } from "lucide-react"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useAuth } from "@/features/auth/auth-provider"
import { useServiceAction } from "@/hooks/use-service"
import { assetSnapshotQueryHref } from "@/lib/asset-query"
import { snapshotRetentionText } from "@/lib/snapshot-picker"
import {
  formatCompactNumber,
  formatDateTime,
  formatLagSeconds,
  formatNumber,
  formatRelativeTime,
} from "@/lib/format"
import {
  isIcebergCandidate,
  msToIso,
  snapshotRelativeTime,
  snapshotsNewestFirst,
} from "@/lib/lakehouse-view"
import { fmtMeasured, type Measured } from "@/lib/measured"
import { notifyError, notifySuccess } from "@/lib/notify"
import { schemaHistory } from "@/lib/schema-history"
import { governanceService } from "@/services"
import type { AssetDetail } from "@/services/contracts/assets"
import type { DatasetSla } from "@/services/contracts/governance"
import type { LakehouseSchemaVersion } from "@/services/contracts/lakehouse"
import { lineageKey } from "./asset-lineage"
import type { IcebergTableState } from "./asset-storage"
import { ALL, CountToggle } from "./count-toggle"

/** How many snapshots, or changes, are listed before the reader asks for more. */
const PAGE_SIZES = [10, 25, 50] as const

/** The sizes that would cut a list of `total` short, then all of it. */
function sizesFor(total: number): number[] {
  return [...PAGE_SIZES.filter((n) => n < total), ALL]
}

function QuietEmpty({ title, description }: { title: string; description?: string }) {
  return <EmptyState title={title} description={description} className="py-4" />
}

/** `+1,200` for rows a snapshot added; a dash when it added none or did not say. */
function signed(value: Measured, sign: "+" | "−") {
  return value === null || value === 0 ? "—" : `${sign}${formatNumber(value)}`
}

/**
 * The asset's Iceberg snapshots, newest first — one per load — from the
 * table the whole page shares (`useIcebergTable`). Each can be queried as
 * the table stood right after it. A `not_found` means the registry's
 * `tableName` is not a real Bronze table: a quiet empty state, not an
 * error.
 */
function IcebergSnapshots({
  asset: a,
  state,
  limit,
}: {
  asset: AssetDetail
  state: IcebergTableState
  /** How many of the newest snapshots to list. */
  limit: number
}) {
  if (state.status === "loading") return <QuietEmpty title="Loading snapshots…" />
  if (state.status === "error") {
    if (state.error.code === "not_found") {
      return <QuietEmpty title="No Iceberg table for this asset" />
    }
    return <ErrorState error={state.error} onRetry={state.reload} />
  }

  const snapshots = snapshotsNewestFirst(state.data?.snapshots ?? []).slice(0, limit)
  if (snapshots.length === 0) return <QuietEmpty title="No snapshots for this asset" />

  return (
    <div className="overflow-hidden rounded-lg border border-border">
      <Table>
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            <TableHead className="text-xs">Committed</TableHead>
            <TableHead className="text-xs">Operation</TableHead>
            <TableHead className="text-right text-xs">Added</TableHead>
            <TableHead className="text-right text-xs">Deleted</TableHead>
            <TableHead className="text-right text-xs">Rows after</TableHead>
            <TableHead className="text-right text-xs">Data files</TableHead>
            <TableHead className="text-xs">Snapshot</TableHead>
            <TableHead className="text-xs" aria-label="Actions" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {snapshots.map((s, i) => {
            const href = assetSnapshotQueryHref(a, s.id)
            return (
              <TableRow key={s.id}>
                <TableCell className="py-1.5 text-xs" title={formatDateTime(msToIso(s.timestampMs))}>
                  {snapshotRelativeTime(s.timestampMs)}
                  {i === 0 ? (
                    <Pill tone="neutral" className="ml-1.5">
                      current
                    </Pill>
                  ) : null}
                </TableCell>
                <TableCell className="py-1.5 text-xs">{s.operation}</TableCell>
                <TableCell className="py-1.5 text-right text-xs tabular-nums">
                  {signed(s.summary.addedRecords, "+")}
                </TableCell>
                <TableCell className="py-1.5 text-right text-xs tabular-nums">
                  {signed(s.summary.deletedRecords, "−")}
                </TableCell>
                <TableCell className="py-1.5 text-right text-xs tabular-nums">
                  {fmtMeasured(s.summary.totalRecords, formatNumber)}
                </TableCell>
                <TableCell className="py-1.5 text-right text-xs tabular-nums">
                  {fmtMeasured(s.summary.totalDataFiles, formatNumber)}
                </TableCell>
                <TableCell className="py-1.5 font-mono text-xs text-muted-foreground">{s.id}</TableCell>
                <TableCell className="py-1 text-right">
                  {href ? (
                    <Button size="sm" variant="ghost" render={<Link href={href} />}>
                      Query this version
                    </Button>
                  ) : null}
                </TableCell>
              </TableRow>
            )
          })}
        </TableBody>
      </Table>
    </div>
  )
}

const selectClassName = "h-8 rounded-lg border border-input bg-transparent px-2.5 text-sm"

const UNIT_MINUTES = { minutes: 1, hours: 60, days: 1440 } as const
type Unit = keyof typeof UNIT_MINUTES

/**
 * How often the table is expected to refresh, as the whole number of
 * minutes the SLA stores — or `null` while the form does not say one.
 */
export function slaMinutes(amount: string, unit: Unit): number | null {
  const n = Number(amount)
  if (amount.trim() === "" || !Number.isInteger(n) || n <= 0) return null
  return n * UNIT_MINUTES[unit]
}

/** The largest unit a number of minutes is a whole count of. */
function inLargestUnit(minutes: number): { amount: string; unit: Unit } {
  for (const unit of ["days", "hours"] as const) {
    if (minutes % UNIT_MINUTES[unit] === 0) return { amount: String(minutes / UNIT_MINUTES[unit]), unit }
  }
  return { amount: String(minutes), unit: "minutes" }
}

function SetTargetDialog({
  asset: a,
  onClose,
  onSaved,
}: {
  asset: AssetDetail
  onClose: () => void
  onSaved: () => void
}) {
  // Start from the SLA in force; a target taken from the registry's
  // frequency is not an SLA, so the form then starts from a day.
  const current =
    a.freshnessTargetSource === "sla" && a.freshnessTargetSeconds
      ? inLargestUnit(Math.round(a.freshnessTargetSeconds / 60))
      : { amount: "1", unit: "days" as Unit }
  const [amount, setAmount] = React.useState(current.amount)
  const [unit, setUnit] = React.useState<Unit>(current.unit)
  const save = useServiceAction((signal, input: DatasetSla) =>
    governanceService.putDatasetSla(input, signal)
  )
  const remove = useServiceAction((signal, table: string) =>
    governanceService.deleteDatasetSla(table, signal)
  )
  const minutes = slaMinutes(amount, unit)
  // Only an SLA can be removed: a target the registry's refresh frequency
  // gives is not something this dialog set.
  const hasSla = a.freshnessTargetSource === "sla"

  async function removeTarget() {
    // `run` resolves to `null` only on failure.
    if ((await remove.run(lineageKey(a))) === null) {
      notifyError("Failed to remove the freshness target", remove.error)
      return
    }
    notifySuccess("Freshness target removed")
    onSaved()
  }

  async function submit() {
    if (minutes === null) return
    const saved = await save.run({ tableName: lineageKey(a), expectedIntervalMinutes: minutes })
    if (saved === null) return
    notifySuccess("Freshness target saved")
    onSaved()
  }

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Set freshness target</DialogTitle>
          <DialogDescription>
            How often {a.name} must receive new data. Past that, it reads as late and its
            health as degraded.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-1.5">
          <Label htmlFor="sla-amount">New data at least every</Label>
          <div className="flex gap-2">
            <Input
              id="sla-amount"
              inputMode="numeric"
              className="w-24"
              value={amount}
              onChange={(e) => setAmount(e.target.value)}
              aria-invalid={minutes === null}
            />
            <select
              aria-label="Unit"
              className={selectClassName}
              value={unit}
              onChange={(e) => setUnit(e.target.value as Unit)}
            >
              {(Object.keys(UNIT_MINUTES) as Unit[]).map((u) => (
                <option key={u} value={u}>
                  {u}
                </option>
              ))}
            </select>
          </div>
          {save.error ? <p className="text-sm text-destructive">{save.error.message}</p> : null}
        </div>
        <DialogFooter>
          {hasSla ? (
            <Button
              size="sm"
              variant="ghost"
              className="mr-auto text-destructive"
              onClick={() => void removeTarget()}
              disabled={remove.status === "pending" || save.status === "pending"}
              title="The asset goes back to having no target of its own"
            >
              {remove.status === "pending" ? "Removing…" : "Remove target"}
            </Button>
          ) : null}
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            onClick={() => void submit()}
            disabled={minutes === null || save.status === "pending" || remove.status === "pending"}
          >
            {save.status === "pending" ? "Saving…" : "Save target"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/**
 * How fresh the asset is against how fresh it is meant to be, and the way
 * to say how fresh that is. Without a target an age is only an age — which
 * is every Silver and serving table until someone sets one here.
 */
function FreshnessCard({ asset: a, onChanged }: { asset: AssetDetail; onChanged: () => void }) {
  const { hasPermission } = useAuth()
  const [setting, setSetting] = React.useState(false)
  const target = a.freshnessTargetSeconds ?? null
  // An SLA names a table as `<namespace>.<table>`.
  const canTarget = lineageKey(a).includes(".")

  return (
    <SectionCard
      size="sm"
      title="Freshness"
      description={
        target === null
          ? "No target is set, so the age is shown without calling it on time or late."
          : `Expected within ${formatLagSeconds(target)}, ${
              a.freshnessTargetSource === "sla"
                ? "by its freshness SLA"
                : "from the refresh frequency its registry states"
            }.`
      }
      action={
        hasPermission("governance:write") && canTarget ? (
          <Button size="sm" variant="outline" onClick={() => setSetting(true)}>
            <Timer />
            {a.freshnessTargetSource === "sla" ? "Change target" : "Set target"}
          </Button>
        ) : undefined
      }
    >
      <FreshnessIndicator
        lagSeconds={a.freshnessLagSeconds}
        targetSeconds={target}
        targetSource={a.freshnessTargetSource}
        className="text-sm"
      />
      {setting ? (
        <SetTargetDialog
          asset={a}
          onClose={() => setSetting(false)}
          onSaved={() => {
            setSetting(false)
            onChanged()
          }}
        />
      ) : null}
    </SectionCard>
  )
}

type Change = AssetDetail["changeHistory"][number]

/**
 * One history of what changed about the asset, newest first: what people
 * did to it (the audit trail) and what its table's schema became — an
 * Iceberg table's own schema versions, or for a Silver or Gold table the
 * versions the console recorded (`recorded`, `AssetDetail.schemaVersions`).
 * A schema version has no author on record, so it is named by its version;
 * an Iceberg version with no time on record cannot be placed among the
 * others and is left to the Schema tab. A recorded version is dated when
 * the console saw the table change, which is the only time there is.
 */
export function changeTimeline(
  history: Change[],
  versions: LakehouseSchemaVersion[],
  recorded: AssetDetail["schemaVersions"] = []
): Change[] {
  const schema = schemaHistory(versions).flatMap((v) =>
    v.sinceMs === null
      ? []
      : [
          {
            id: `schema-${v.schemaId}`,
            at: msToIso(v.sinceMs),
            actor: `Schema v${v.schemaId}`,
            summary: v.changes.join(" · "),
          },
        ]
  )
  const seen = recorded.map((v) => ({
    id: `schema-recorded-${v.version}`,
    at: v.at,
    actor: `Schema v${v.version}`,
    summary: v.change,
  }))
  return [...history, ...schema, ...seen].sort((a, b) => b.at.localeCompare(a.at))
}

/**
 * The Activity tab: what happened to the asset over time — how fresh it
 * is, its loads (snapshots), the changes to it and its schema, and who
 * queries it.
 */
export function AssetActivity({
  asset: a,
  iceberg,
  onChanged,
}: {
  asset: AssetDetail
  iceberg: IcebergTableState
  /** Reloads the asset after its freshness target was set. */
  onChanged: () => void
}) {
  const changes = changeTimeline(
    a.changeHistory,
    iceberg.status === "success" ? (iceberg.data?.schemaVersions ?? []) : [],
    a.schemaVersions
  )
  const [snapshotLimit, setSnapshotLimit] = React.useState<number>(PAGE_SIZES[0])
  const [changeLimit, setChangeLimit] = React.useState<number>(PAGE_SIZES[0])
  const snapshotCount = iceberg.status === "success" ? (iceberg.data?.snapshots.length ?? 0) : 0
  // DATA-16 D2: how far back versions go is what the list shows, never a promise.
  const retention =
    iceberg.status === "success" && iceberg.data && iceberg.data.snapshots.length > 0
      ? snapshotRetentionText(iceberg.data.snapshots)
      : null
  // Reads by dashboards are not in the query history the usage is counted from.
  const dashboards = a.dependents.filter((d) => d.kind.toLowerCase().includes("dashboard")).length
  const uncounted =
    dashboards > 0
      ? `Not counted: reads by the ${dashboards} dashboard${dashboards === 1 ? "" : "s"} that use${
          dashboards === 1 ? "s" : ""
        } this asset.`
      : null

  return (
    <div className="flex flex-col gap-2">
      <FreshnessCard asset={a} onChanged={onChanged} />
      <SectionCard
        size="sm"
        title="Snapshots"
        description={`One per load of an Iceberg table, newest first. Any of them can be queried as the table stood then.${
          retention ? ` ${retention}.` : ""
        }${snapshotCount > snapshotLimit ? ` Showing the newest ${snapshotLimit} of ${snapshotCount}.` : ""}`}
        action={
          snapshotCount > PAGE_SIZES[0] ? (
            <CountToggle
              label="Show"
              ariaLabel="Snapshots to show"
              options={sizesFor(snapshotCount)}
              value={snapshotLimit}
              onChange={setSnapshotLimit}
            />
          ) : undefined
        }
      >
        {isIcebergCandidate(a) && a.tableName ? (
          <IcebergSnapshots asset={a} state={iceberg} limit={snapshotLimit} />
        ) : a.snapshots.length === 0 ? (
          <QuietEmpty
            title="Only Iceberg tables keep snapshots"
            description="A ClickHouse table is rewritten in place, so there are no earlier versions to go back to."
          />
        ) : (
          <ul className="divide-y divide-border text-sm">
            {a.snapshots.map((s) => (
              <li key={s.id} className="flex justify-between gap-2 py-1.5">
                <span>{s.operation}</span>
                <span className="text-muted-foreground">
                  {formatRelativeTime(s.committedAt)} · {formatCompactNumber(s.records)} records
                </span>
              </li>
            ))}
          </ul>
        )}
      </SectionCard>
      <div className="grid gap-2 lg:grid-cols-2">
        <SectionCard
          size="sm"
          title="Change history"
          description={`Edits to its details, changes to its schema, and the rules, classifications, policies and targets added for it.${
            changes.length > changeLimit ? ` Showing the newest ${changeLimit} of ${changes.length}.` : ""
          }`}
          action={
            changes.length > PAGE_SIZES[0] ? (
              <CountToggle
                label="Show"
                ariaLabel="Changes to show"
                options={sizesFor(changes.length)}
                value={changeLimit}
                onChange={setChangeLimit}
              />
            ) : undefined
          }
        >
          {changes.length === 0 ? (
            <QuietEmpty title="No changes recorded" />
          ) : (
            <ul className="divide-y divide-border text-sm">
              {changes.slice(0, changeLimit).map((c) => (
                <li key={c.id} className="flex flex-wrap items-baseline gap-2 py-1.5">
                  <span className="font-medium">{c.actor}</span>
                  <span className="text-muted-foreground">{c.summary}</span>
                  <span className="ml-auto text-xs text-muted-foreground">
                    {formatRelativeTime(c.at)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </SectionCard>
        <SectionCard
          size="sm"
          title="Usage (7d)"
          description="Queries run in Query Studio or by the copilot that read this asset, by anyone. Only your own query text is shown."
        >
          {a.usage === null ? (
            <QuietEmpty title="Usage not measured" description="The query history could not be read." />
          ) : a.usage.queries7d === 0 ? (
            <QuietEmpty
              title="No Query Studio or copilot query in the last 7 days"
              description={uncounted ?? undefined}
            />
          ) : (
            <div className="flex flex-col gap-3">
              <dl className="grid grid-cols-3 gap-2 text-sm">
                {(
                  [
                    ["Queries", formatNumber(a.usage.queries7d)],
                    ["People", formatNumber(a.usage.users7d)],
                    ["Avg latency", `${formatNumber(a.usage.avgLatencyMs)} ms`],
                  ] as const
                ).map(([label, value]) => (
                  <div key={label}>
                    <dt className="text-xs text-muted-foreground">{label}</dt>
                    <dd className="text-lg font-semibold tabular-nums">{value}</dd>
                  </div>
                ))}
              </dl>
              <div>
                {uncounted ? <p className="mb-2 text-xs text-muted-foreground">{uncounted}</p> : null}
                <div className="mb-1 text-xs font-medium text-muted-foreground">Your recent queries</div>
                {a.recentQueries.length === 0 ? (
                  <p className="text-sm text-muted-foreground">None of them were yours.</p>
                ) : (
                  <ul className="divide-y divide-border text-sm">
                    {a.recentQueries.map((q) => (
                      <li key={q.id} className="flex items-center gap-2 py-1.5">
                        <span className="min-w-0 flex-1 truncate font-mono text-xs" title={q.sql}>
                          {q.sql}
                        </span>
                        {q.status && q.status !== "completed" ? <Pill tone="warning">{q.status}</Pill> : null}
                        <span className="shrink-0 text-xs text-muted-foreground">
                          {formatRelativeTime(q.at)}
                        </span>
                        {q.auditEventId ? (
                          <Link
                            href={`/audit?event=${encodeURIComponent(q.auditEventId)}`}
                            className="shrink-0 text-xs text-primary hover:underline"
                          >
                            Audit
                          </Link>
                        ) : null}
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            </div>
          )}
        </SectionCard>
      </div>
    </div>
  )
}
