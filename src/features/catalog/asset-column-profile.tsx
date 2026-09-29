"use client"

import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useAuth } from "@/features/auth/auth-provider"
import { useService } from "@/hooks/use-service"
import { formatCompactNumber, formatNumber, formatPercent } from "@/lib/format"
import { assetService } from "@/services"
import type { AssetColumn, ColumnProfile } from "@/services/contracts/assets"

/**
 * Null share as a thin meter plus its number. The number carries the
 * meaning; the bar only lets the eye find the gappy columns in a long list.
 */
function NullMeter({ fraction }: { fraction: number }) {
  return (
    <span className="flex items-center gap-2">
      <span className="h-1.5 w-12 overflow-hidden rounded-full bg-muted" aria-hidden>
        <span
          className="block h-full rounded-full bg-foreground/40"
          style={{ width: `${Math.min(fraction, 1) * 100}%` }}
        />
      </span>
      <span className="tabular-nums">{formatPercent(fraction)}</span>
    </span>
  )
}

function ProfileRow({ c, column }: { c: ColumnProfile; column?: AssetColumn }) {
  const name = (
    <TableCell className="py-1.5">
      <span className="font-mono text-xs font-medium">{c.name}</span>
      {column?.masked ? (
        <Pill tone="warning" className="ml-1.5">
          masked
        </Pill>
      ) : null}
      <div className="text-xs text-muted-foreground">{c.dataType}</div>
    </TableCell>
  )
  if (!c.profiled) {
    return (
      <TableRow>
        {name}
        <TableCell colSpan={4} className="py-1.5 text-xs text-muted-foreground">
          Not profiled (type not supported)
        </TableCell>
      </TableRow>
    )
  }
  const range =
    c.min != null && c.max != null ? (c.min === c.max ? c.min : `${c.min} – ${c.max}`) : "—"
  return (
    <TableRow>
      {name}
      <TableCell className="py-1.5 text-xs">
        {c.nullFraction == null ? "—" : <NullMeter fraction={c.nullFraction} />}
      </TableCell>
      <TableCell className="py-1.5 text-xs tabular-nums" title="Approximate">
        {c.distinctCount == null ? "—" : `≈ ${formatCompactNumber(c.distinctCount)}`}
      </TableCell>
      <TableCell className="max-w-56 truncate py-1.5 font-mono text-xs" title={range}>
        {range}
      </TableCell>
      <TableCell className="py-1.5">
        {c.topValues && c.topValues.length > 0 ? (
          <span className="flex flex-wrap gap-1">
            {c.topValues.map((t) => (
              <Pill
                key={t.value}
                tone="neutral"
                className="max-w-40 font-mono"
                title={`${t.value} · ${formatNumber(t.count)} rows`}
              >
                <span className="truncate">{t.value === "" ? "(empty)" : t.value}</span>
                <span className="text-muted-foreground/70 tabular-nums">
                  {formatCompactNumber(t.count)}
                </span>
              </Pill>
            ))}
          </span>
        ) : (
          <span className="text-xs text-muted-foreground">Mostly unique</span>
        )}
      </TableCell>
    </TableRow>
  )
}

/** Loads and renders the profile; mounted only for callers who may run it. */
function ProfileBody({ assetId, schema }: { assetId: string; schema: AssetColumn[] }) {
  const state = useService(
    (s) =>
      assetService.getAssetProfile
        ? assetService.getAssetProfile(assetId, s)
        : Promise.reject(new Error("This deployment cannot profile assets.")),
    [assetId]
  )

  if (state.status === "loading") return <EmptyState title="Profiling columns…" className="py-4" />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={state.reload} />
  const p = state.data
  if (!p.supported) return <EmptyState title="Profile not available" description={p.reason} className="py-4" />

  const byName = new Map(schema.map((c) => [c.name, c]))
  return (
    <div className="flex flex-col gap-2">
      <p className="text-xs text-muted-foreground">
        {formatNumber(p.rowsProfiled)} rows of <span className="font-mono">{p.source}</span>
        {p.sourceKind === "iceberg" ? " (Iceberg)" : ""}
        {p.sampled ? ` (first ${formatNumber(p.rowLimit)} only)` : ""}
        {p.columnsCapped ? " · some columns left out" : ""}. Masking and row filters apply.
      </p>
      <div className="overflow-x-auto rounded-lg border border-border">
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="text-xs">Column</TableHead>
              <TableHead className="text-xs">Nulls</TableHead>
              <TableHead className="text-xs">Distinct</TableHead>
              <TableHead className="text-xs">Range</TableHead>
              <TableHead className="text-xs">Top values</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {p.columns.map((c) => (
              <ProfileRow key={c.name} c={c} column={byName.get(c.name)} />
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  )
}

/**
 * Per-column stats for an asset. Needs `query:read`: the stats are read
 * from the data, so the API gates them like running a query.
 */
export function AssetColumnProfile({
  assetId,
  schema,
}: {
  assetId: string
  schema: AssetColumn[]
}) {
  const { hasPermission } = useAuth()
  return (
    <SectionCard
      size="sm"
      title="Column profile"
      description="Completeness, cardinality, range, and most frequent values."
    >
      {hasPermission("query:read") ? (
        <ProfileBody assetId={assetId} schema={schema} />
      ) : (
        <EmptyState
          title="Profiling needs query access"
          description="Column stats are read from the data itself, so they require the query:read permission."
          className="py-4"
        />
      )}
    </SectionCard>
  )
}
