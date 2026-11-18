"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { MetricCard } from "@/components/patterns/metric-card"
import { SectionCard } from "@/components/patterns/section-card"
import { SelectField } from "@/components/patterns/select-field"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { saveBlob } from "@/lib/browser-file"
import { formatDateTime, formatDuration, formatNumber } from "@/lib/format"
import { withNotify } from "@/lib/notify"
import { usageService } from "@/services"
import type {
  UsageQuery,
  UsageRate,
  UsageReport,
  UsageRow,
  UsageStorageBucket,
  UsageStorageReport,
  UsageStorageRow,
} from "@/services"

/**
 * Usage (`OPS-8` part B): the runs of a period summed per day, person or
 * engine, with the totals, a CSV file of the same sums and the rates behind
 * the estimate in money.
 *
 * Every figure is a sum of recorded runs. The only money is the estimate,
 * and its column appears only once an administrator has entered a rate, so
 * the page never shows a price nobody set. The tenant group is offered only
 * to a caller holding `*:*`, the one caller the API lets read every tenant,
 * and the add-rate form only to a caller holding `settings:write`.
 *
 * The size of every layer (`OPS-8` part C) is the platform operator's: a
 * layer holds the tables of every tenant, and the API refuses the read to
 * anybody else. A tenant administrator sees a line saying so, and no
 * section.
 */

const DAY_MS = 24 * 60 * 60 * 1000
const DEFAULT_DAYS = 30
const MAX_DAYS = 366
const DAY_TEXT = /^\d{4}-\d{2}-\d{2}$/

const GROUPS = [
  { value: "day", label: "Day" },
  { value: "person", label: "Person" },
  { value: "engine", label: "Engine" },
] as const

const TENANT_GROUP = { value: "tenant", label: "Tenant" } as const

const RATE_KINDS = [
  { value: "scan_tb", label: "Data scanned, per TB" },
  { value: "engine_hour", label: "Engine time, per hour" },
  { value: "storage_gb_month", label: "Storage, per GB per month" },
] as const

function dayOf(instant: number): string {
  return new Date(instant).toISOString().slice(0, 10)
}

/** `YYYY-MM-DD` as the instant its UTC day starts, or `null` when it is not a day. */
function dayStart(text: string): number | null {
  if (!DAY_TEXT.test(text)) return null
  const at = Date.parse(`${text}T00:00:00Z`)
  return Number.isNaN(at) ? null : at
}

/** What is wrong with a period, in the words the API uses, or `null` when it can be asked for. */
function periodProblem(from: string, to: string): string | null {
  const start = dayStart(from)
  const end = dayStart(to)
  if (start === null || end === null) return "Pick a first and a last day."
  if (start > end) return "The first day must not be after the last day."
  if ((end - start) / DAY_MS + 1 > MAX_DAYS) return "A period is at most 366 days."
  return null
}

/**
 * Bytes in decimal units (KB is 1,000 bytes, TB is 10^12), not the 1,024 steps
 * of `formatBytes`. The scan rate is priced per 10^12 bytes and the storage
 * rate per 10^9, so a reader who multiplies the size shown by the rate gets
 * the estimate.
 */
function formatDecimalBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—"
  const units = ["B", "KB", "MB", "GB", "TB", "PB"]
  let value = bytes
  let unit = 0
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000
    unit += 1
  }
  const text = value >= 100 || unit === 0 ? String(Math.round(value)) : value.toFixed(1).replace(/\.0$/, "")
  return `${text} ${units[unit]}`
}

/**
 * A decimal string as money, worked on its digits and never through a float:
 * at least two places, at most `maxDecimals` (rounded half up on the digits),
 * and trailing zeros beyond the second dropped. A rate is shown with up to
 * six places, the six it is stored with, so 0.000012 is not shown as 0.00; an
 * estimate with up to four.
 *
 * PR review fix (SHOULD-FIX): this went through `Number`, which showed a rate
 * of 0.000012 as 0.00 and lost the low digits of an amount of 16 digits.
 */
function formatMoney(amount: string | null, maxDecimals = 4): string {
  if (amount === null) return "No rate"
  const parts = /^(\d+)(?:\.(\d+))?$/.exec(amount)
  if (!parts) return "No rate"
  let whole = parts[1]
  let fraction = parts[2] ?? ""
  if (fraction.length > maxDecimals) {
    const rounded =
      BigInt(whole + fraction.slice(0, maxDecimals)) + BigInt(fraction.charCodeAt(maxDecimals) >= 53 ? 1 : 0)
    const digits = rounded.toString().padStart(maxDecimals + 1, "0")
    whole = digits.slice(0, digits.length - maxDecimals)
    fraction = digits.slice(digits.length - maxDecimals)
  }
  fraction = fraction.replace(/0+$/, "").padEnd(2, "0")
  return `${whole.replace(/\B(?=(\d{3})+(?!\d))/g, ",")}.${fraction}`
}

const FIELD_LABEL = "flex flex-col gap-1 text-xs text-muted-foreground"

export function UsagePage() {
  const { hasPermission, status: authStatus } = useAuth()
  const canReadAllTenants = hasPermission("*:*")
  const canWriteRates = hasPermission("settings:write")

  const [period, setPeriod] = React.useState(() => {
    const today = Date.now()
    return { from: dayOf(today - (DEFAULT_DAYS - 1) * DAY_MS), to: dayOf(today) }
  })
  const [group, setGroup] = React.useState<string>("day")
  const problem = periodProblem(period.from, period.to)

  const query: UsageQuery = {
    from: period.from,
    to: period.to,
    group,
    // A person's own tenant is the default; the tenant group compares them all.
    ...(group === "tenant" ? { tenant: "all" as const } : {}),
  }
  const usage = useService<UsageReport | null>(
    (s) => (problem ? Promise.resolve(null) : usageService.getUsage(query, s)),
    [period.from, period.to, group, problem]
  )
  const rates = useService((s) => usageService.listRates(s), [])
  // The storage of every layer is the platform operator's: the API refuses
  // the read to anybody else, so the page does not ask for it. `null` is the
  // answer for a caller that may not have it.
  const storage = useService<UsageStorageReport | null>(
    (s) =>
      canReadAllTenants && !problem
        ? usageService.getStorage({ from: period.from, to: period.to }, s)
        : Promise.resolve(null),
    [period.from, period.to, problem, canReadAllTenants]
  )

  const download = useServiceAction((signal, q: UsageQuery) => usageService.exportUsageCsv(q, signal))
  async function downloadCsv() {
    const file = await download.run(query)
    if (file) saveBlob(file.blob, file.fileName)
  }

  const groups = canReadAllTenants ? [...GROUPS, TENANT_GROUP] : GROUPS
  const groupLabel = [...GROUPS, TENANT_GROUP].find((g) => g.value === group)?.label ?? "Group"

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Usage"
        description="What was run in a period, summed per day, person or engine. Every figure is added up from the queries the console recorded."
        actions={
          <Button
            variant="outline"
            size="sm"
            disabled={problem !== null || download.status === "pending"}
            onClick={() => void downloadCsv()}
          >
            Download CSV
          </Button>
        }
      />
      {download.error ? (
        <p role="alert" className="text-sm text-destructive">
          {download.error.message}
        </p>
      ) : null}

      <div className="flex flex-col gap-3 rounded-md border bg-card px-3 py-3 sm:flex-row sm:items-end">
        <label className={FIELD_LABEL}>
          From
          <Input
            type="date"
            value={period.from}
            onChange={(e) => setPeriod((p) => ({ ...p, from: e.target.value }))}
            className="h-8 text-xs"
          />
        </label>
        <label className={FIELD_LABEL}>
          To
          <Input
            type="date"
            value={period.to}
            onChange={(e) => setPeriod((p) => ({ ...p, to: e.target.value }))}
            className="h-8 text-xs"
          />
        </label>
        <div role="group" aria-label="Group by" className="flex items-center gap-1.5">
          {groups.map((g) => (
            <Button
              key={g.value}
              type="button"
              size="sm"
              variant={group === g.value ? "default" : "outline"}
              aria-pressed={group === g.value}
              onClick={() => setGroup(g.value)}
            >
              {g.label}
            </Button>
          ))}
        </div>
        <p className="text-xs text-muted-foreground sm:ml-auto">
          Days are UTC days, and the last day is included. A period is at most 366 days.
        </p>
      </div>

      {problem ? (
        <p role="alert" className="text-sm text-destructive">
          {problem}
        </p>
      ) : null}
      {!problem && usage.status === "loading" ? <LoadingSkeleton /> : null}
      {!problem && usage.status === "error" ? (
        <ErrorState error={usage.error} onRetry={usage.reload} />
      ) : null}
      {!problem && usage.status === "success" && usage.data ? (
        <UsageReportView report={usage.data} groupLabel={groupLabel} />
      ) : null}

      {canReadAllTenants ? (
        <StoragePanel state={storage} />
      ) : authStatus === "authenticated" ? (
        <p className="text-xs text-muted-foreground">
          Storage is shown to the platform operator only.
        </p>
      ) : null}

      <RatesPanel
        state={rates}
        canWrite={canWriteRates}
        onAdded={() => {
          rates.reload()
          usage.reload()
        }}
      />
    </div>
  )
}

function UsageReportView({
  report,
  groupLabel,
}: {
  readonly report: UsageReport
  readonly groupLabel: string
}) {
  const { totals, currency } = report
  const priced = currency !== null
  return (
    <>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <MetricCard
          label="Queries"
          value={formatNumber(totals.queries)}
          hint={`${formatNumber(totals.completed)} completed, ${formatNumber(totals.failed)} failed, ${formatNumber(totals.blocked)} blocked, ${formatNumber(totals.cancelled)} cancelled`}
        />
        <MetricCard label="Data scanned" value={formatDecimalBytes(totals.scannedBytes)} />
        <MetricCard
          label="Engine time (wall clock)"
          value={formatDuration(totals.engineMs)}
          hint="Time on the clock, not CPU time"
        />
        {priced ? (
          <MetricCard
            label={`Estimate (${currency})`}
            value={formatMoney(totals.estimate)}
            hint="Worked out from the rates below. Not an invoice."
          />
        ) : null}
      </div>
      {report.rows.length === 0 ? (
        <EmptyState
          title="No queries in this period"
          description="Nothing was run in these days, or nothing was recorded for your tenant. Try a longer period."
        />
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{groupLabel}</TableHead>
              <TableHead className="text-right">Queries</TableHead>
              <TableHead className="text-right">Completed</TableHead>
              <TableHead className="text-right">Failed</TableHead>
              <TableHead className="text-right">Blocked</TableHead>
              <TableHead className="text-right">Cancelled</TableHead>
              <TableHead className="text-right">Data scanned</TableHead>
              <TableHead className="text-right">Engine time (wall clock)</TableHead>
              {priced ? <TableHead className="text-right">{`Estimate (${currency})`}</TableHead> : null}
            </TableRow>
          </TableHeader>
          <TableBody>
            {report.rows.map((row) => (
              <UsageRowView key={row.key} row={row} priced={priced} />
            ))}
          </TableBody>
        </Table>
      )}
      <p className="text-xs text-muted-foreground">
        Data scanned uses decimal units: 1 TB is 10^12 bytes, the unit the scan rate is priced in.
      </p>
    </>
  )
}

function UsageRowView({ row, priced }: { readonly row: UsageRow; readonly priced: boolean }) {
  return (
    <TableRow>
      <TableCell>{row.label}</TableCell>
      <TableCell className="text-right tabular-nums">{formatNumber(row.queries)}</TableCell>
      <TableCell className="text-right tabular-nums">{formatNumber(row.completed)}</TableCell>
      <TableCell className="text-right tabular-nums">{formatNumber(row.failed)}</TableCell>
      <TableCell className="text-right tabular-nums">{formatNumber(row.blocked)}</TableCell>
      <TableCell className="text-right tabular-nums">{formatNumber(row.cancelled)}</TableCell>
      <TableCell className="text-right tabular-nums">{formatDecimalBytes(row.scannedBytes)}</TableCell>
      <TableCell className="text-right tabular-nums">{formatDuration(row.engineMs)}</TableCell>
      {priced ? (
        <TableCell className="text-right tabular-nums">{formatMoney(row.estimate)}</TableCell>
      ) : null}
    </TableRow>
  )
}

/**
 * The newest measurement of every layer: the server answers one row per
 * layer per day, and the page shows the latest day of each layer. A layer
 * measured on no day at all is not in the list.
 */
function newestPerLayer(rows: UsageStorageRow[]): UsageStorageRow[] {
  const newest = new Map<string, UsageStorageRow>()
  for (const row of rows) {
    const seen = newest.get(row.layer)
    if (seen === undefined || row.day > seen.day) newest.set(row.layer, row)
  }
  return [...newest.values()].sort((a, b) => a.layer.localeCompare(b.layer))
}

/**
 * The layers whose bytes are not in the warehouse bucket: a ClickHouse layer
 * is measured on the ClickHouse server's own disk, so it carries one number
 * instead of two. The rows themselves say which layers those are.
 */
function layersWithoutStoredBytes(rows: UsageStorageRow[]): string[] {
  return rows.filter((row) => row.storedBytes === null).map((row) => row.layer)
}

/**
 * The line that names the layers that carry one number, and says why: their
 * bytes sit on the ClickHouse server's own disk, not in the warehouse bucket.
 */
function oneNumberLine(layers: string[]): string {
  if (layers.length === 1) {
    return `${layers[0]} is a ClickHouse layer: its bytes sit on the ClickHouse server's own disk, not in the warehouse bucket, so it has one number.`
  }
  return `${layers.join(" and ")} are ClickHouse layers: their bytes sit on the ClickHouse server's own disk, not in the warehouse bucket, so they have one number.`
}

/**
 * The rest of the bucket: the bytes no current table accounts for, with the
 * two measurement times it was worked out from. `null` when the two
 * measurements disagree — the page says "not comparable" rather than a
 * negative number.
 */
function bucketRestLine(bucket: UsageStorageBucket, storageMeasuredAt: string | null): string {
  const amount =
    bucket.unaccountedBytes === null ? "not comparable" : formatDecimalBytes(bucket.unaccountedBytes)
  const storageWhen = storageMeasuredAt ? formatDateTime(storageMeasuredAt) : "—"
  return `In the bucket but in no current table: ${amount} (bucket measured ${formatDateTime(bucket.measuredAt)}; storage measured ${storageWhen}).`
}

/**
 * The size of every layer, measured once a day by the storage job.
 *
 * A layer whose tables could not be measured never reads as a layer of zero
 * bytes: the size is the bytes of the tables that were measured, and the
 * count says how many are missing from it.
 *
 * Two numbers per Iceberg layer: the data that is current, and what the
 * object store holds for it. The bucket holds more than the current tables
 * — history and orphan files sit in it too — so the rest of the bucket is
 * one line of its own.
 */
function StoragePanel({
  state,
}: {
  readonly state: ReturnType<typeof useService<UsageStorageReport | null>>
}) {
  const report = state.status === "success" ? state.data : null
  const rows = report ? newestPerLayer(report.rows) : []
  const currency = report?.currency ?? null
  const priced = currency !== null
  const skipped = rows.filter((row) => row.tablesSkipped > 0)
  const oneNumber = layersWithoutStoredBytes(rows)
  const measuredAt = report?.measuredAt ?? null
  const bucket = report?.bucket ?? null
  return (
    <SectionCard
      title="Storage per layer"
      description="The size of every layer, measured once a day. A layer holds the tables of every tenant, so this is the platform operator's."
    >
      <div className="flex flex-col gap-3">
        {state.status === "loading" ? <LoadingSkeleton rows={2} /> : null}
        {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
        {/* A report that says nothing was measured is an empty state, not an
            outage. `report` is `null` while the read has not answered yet, so
            the two are not the same thing. */}
        {state.status === "success" && report !== null && measuredAt === null ? (
          <EmptyState title="Not measured yet" description="The daily job runs at 01:00 UTC." />
        ) : null}
        {rows.length > 0 ? (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Layer</TableHead>
                <TableHead>Measured on</TableHead>
                <TableHead className="text-right">Tables</TableHead>
                <TableHead className="text-right">Current data</TableHead>
                <TableHead className="text-right">Stored on disk</TableHead>
                {priced ? <TableHead className="text-right">{`Estimate (${currency})`}</TableHead> : null}
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((row) => (
                <StorageRowView key={row.layer} row={row} priced={priced} />
              ))}
            </TableBody>
          </Table>
        ) : null}
        {oneNumber.length > 0 ? (
          <p className="text-xs text-muted-foreground">{oneNumberLine(oneNumber)}</p>
        ) : null}
        {bucket ? (
          <p className="text-xs text-muted-foreground">{bucketRestLine(bucket, measuredAt)}</p>
        ) : null}
        {skipped.length > 0 ? (
          <p className="text-xs text-muted-foreground">
            {skipped
              .map(
                (row) =>
                  `${row.layer}: ${row.tablesSkipped} of its tables could not be measured, so the size above is not the whole layer${
                    row.skippedReason ? ` (${row.skippedReason})` : ""
                  }.`
              )
              .join(" ")}
          </p>
        ) : null}
        <p className="text-xs text-muted-foreground">
          Sizes use decimal units: 1 GB is 10^9 bytes, the unit the storage rate is priced in.
        </p>
      </div>
    </SectionCard>
  )
}

function StorageRowView({ row, priced }: { readonly row: UsageStorageRow; readonly priced: boolean }) {
  return (
    <TableRow>
      <TableCell>{row.layer}</TableCell>
      <TableCell>{row.day}</TableCell>
      <TableCell className="text-right tabular-nums">
        {formatNumber(row.tableCount)}
        {row.tablesSkipped > 0 ? (
          <span className="block text-xs text-muted-foreground">
            {`${formatNumber(row.tablesSkipped)} not measured`}
          </span>
        ) : null}
      </TableCell>
      <TableCell className="text-right tabular-nums">{formatDecimalBytes(row.currentBytes)}</TableCell>
      {/* A layer outside the warehouse bucket has no disk number of its own. */}
      <TableCell className="text-right tabular-nums">
        {row.storedBytes === null ? "—" : formatDecimalBytes(row.storedBytes)}
      </TableCell>
      {priced ? (
        <TableCell className="text-right tabular-nums">{formatMoney(row.estimate)}</TableCell>
      ) : null}
    </TableRow>
  )
}

function RatesPanel({
  state,
  canWrite,
  onAdded,
}: {
  readonly state: ReturnType<typeof useService<{ currency: string | null; rates: UsageRate[] }>>
  readonly canWrite: boolean
  readonly onAdded: () => void
}) {
  return (
    <SectionCard
      title="Rates"
      description="A rate turns the sums above into an estimate in money. It applies from its start day until a newer rate of the same kind replaces it. A rate is never changed or removed: a new price is a new rate."
    >
      <div className="flex flex-col gap-4">
        {state.status === "loading" ? <LoadingSkeleton rows={2} /> : null}
        {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
        {state.status === "success" && state.data.rates.length === 0 ? (
          <EmptyState
            title="No rates yet"
            description="Until a rate is entered, this page shows no money."
          />
        ) : null}
        {state.status === "success" && state.data.rates.length > 0 ? (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Rate</TableHead>
                <TableHead className="text-right">Amount</TableHead>
                <TableHead>Starts on</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {state.data.rates.map((rate) => (
                <TableRow key={rate.id}>
                  <TableCell>{RATE_KINDS.find((k) => k.value === rate.kind)?.label ?? rate.kind}</TableCell>
                  <TableCell className="text-right tabular-nums">{`${formatMoney(rate.amount, 6)} ${rate.currency}`}</TableCell>
                  <TableCell>{rate.effectiveFrom}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : null}
        {canWrite && state.status !== "loading" ? (
          <AddRateForm
            currency={state.status === "success" ? state.data.currency : null}
            onAdded={onAdded}
          />
        ) : null}
      </div>
    </SectionCard>
  )
}

function AddRateForm({
  currency,
  onAdded,
}: {
  readonly currency: string | null
  readonly onAdded: () => void
}) {
  const [kind, setKind] = React.useState<string>(RATE_KINDS[0].value)
  const [amount, setAmount] = React.useState("")
  const [typedCurrency, setTypedCurrency] = React.useState("")
  const [startsOn, setStartsOn] = React.useState(() => dayOf(Date.now()))

  const add = useServiceAction(
    withNotify(
      { success: "Rate added", error: "Could not add the rate" },
      (signal, rate: { kind: string; currency: string; amount: string; effectiveFrom: string }) =>
        usageService.addRate(rate, signal)
    )
  )

  async function submit() {
    const stored = await add.run({
      kind,
      currency: currency ?? typedCurrency,
      amount: amount.trim(),
      effectiveFrom: startsOn,
    })
    if (stored === null) return // the form shows the API's sentence
    setAmount("")
    onAdded()
  }

  return (
    <form
      className="flex flex-col gap-3 border-t pt-4"
      onSubmit={(e) => {
        e.preventDefault()
        void submit()
      }}
    >
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <label className={FIELD_LABEL}>
          Rate
          <SelectField
            aria-label="Rate"
            value={kind}
            onValueChange={setKind}
            options={RATE_KINDS}
            size="sm"
            className="w-full text-xs"
          />
        </label>
        <label className={FIELD_LABEL}>
          Amount
          <Input
            inputMode="decimal"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            placeholder="5.00"
            className="h-8 text-xs"
          />
        </label>
        <label className={FIELD_LABEL}>
          Currency
          <Input
            value={currency ?? typedCurrency}
            onChange={(e) => setTypedCurrency(e.target.value.toUpperCase())}
            // All rates share one currency: once one is entered, it is fixed.
            disabled={currency !== null}
            maxLength={3}
            placeholder="USD"
            className="h-8 text-xs"
          />
        </label>
        <label className={FIELD_LABEL}>
          Starts on
          <Input
            type="date"
            value={startsOn}
            onChange={(e) => setStartsOn(e.target.value)}
            className="h-8 text-xs"
          />
        </label>
      </div>
      {add.error ? (
        <p role="alert" className="text-sm text-destructive">
          {add.error.message}
        </p>
      ) : null}
      <div>
        <Button type="submit" size="sm" disabled={add.status === "pending"}>
          Add rate
        </Button>
      </div>
    </form>
  )
}
