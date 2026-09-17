"use client"

import { SectionCard } from "@/components/patterns/section-card"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useService } from "@/hooks/use-service"
import { governanceService, lakehouseService } from "@/services"
import type { DatasetSla } from "@/services/contracts/governance"
import type { LakehouseSnapshot, LakehouseTableDetail } from "@/services/contracts/lakehouse"

/**
 * WS2's `GET /api/lakehouse/tables/{ns}/{table}` returns `TableDetail`,
 * which carries no top-level `lastUpdatedMs` field — only the
 * in-process-only `TableSummary` does (WS5 plan review U1, re-verified
 * against `rust/crates/lakehouse-iceberg/src/rest.rs`). This derives the
 * same value from `TableDetail.snapshots` client-side; it is documented
 * as a derivation, never presented as if it were a wire field. `null`
 * means the table has no snapshots yet — not "just created" or "fresh".
 */
export function deriveLastUpdatedMs(detail: {
  snapshots: Array<Pick<LakehouseSnapshot, "timestampMs">>
}): number | null {
  if (detail.snapshots.length === 0) return null
  return Math.max(...detail.snapshots.map((s) => s.timestampMs))
}

/**
 * Three honest states, never collapsed to a boolean (WS5 item E2): "ok"
 * (updated within the expected interval), "late" (older than the
 * expected interval), and "unknown" (no snapshot timestamp could be
 * determined at all — distinct from "ok", never rendered as fresh).
 */
export function freshnessStatus(
  nowMs: number,
  lastUpdatedMs: number | null,
  expectedIntervalMinutes: number
): "ok" | "late" | "unknown" {
  if (lastUpdatedMs === null) return "unknown"
  return nowMs - lastUpdatedMs <= expectedIntervalMinutes * 60_000 ? "ok" : "late"
}

/**
 * Wraps `freshnessStatus` with a default `nowMs`, mirroring
 * `format.ts`'s `formatRelativeTime(iso, now = Date.now())` pattern — the
 * impure `Date.now()` call lives in a plain function's default parameter,
 * never directly inside a component's render body (the
 * `react-hooks/purity` lint rule this repo enforces).
 */
function currentFreshnessStatus(
  lastUpdatedMs: number | null,
  expectedIntervalMinutes: number,
  nowMs = Date.now()
): "ok" | "late" | "unknown" {
  return freshnessStatus(nowMs, lastUpdatedMs, expectedIntervalMinutes)
}

const STATUS_LABEL: Record<"ok" | "late" | "unknown", string> = {
  ok: "On time",
  late: "Late",
  unknown: "Not measurable",
}

const STATUS_TONE: Record<"ok" | "late" | "unknown", string> = {
  ok: "text-emerald-600 dark:text-emerald-400",
  late: "text-destructive",
  unknown: "text-muted-foreground",
}

function FreshnessRow({ sla }: { sla: DatasetSla }) {
  const [namespace, table] = sla.tableName.split(".")
  // Until the table/namespace resolves, or on any fetch failure, this
  // degrades to `actual: null` / status "unknown" — never a fabricated
  // snapshot time standing in for a real one.
  const detail = useService<LakehouseTableDetail | null>(
    async (signal) => {
      try {
        return await lakehouseService.getTableDetail(namespace, table, signal)
      } catch {
        return null
      }
    },
    [namespace, table]
  )
  const lastUpdatedMs = detail.data ? deriveLastUpdatedMs(detail.data) : null
  const status = currentFreshnessStatus(lastUpdatedMs, sla.expectedIntervalMinutes)
  return (
    <tr className="border-t border-border">
      <td className="px-3 py-2 font-medium">{sla.tableName}</td>
      <td className="px-3 py-2 text-muted-foreground">{sla.expectedIntervalMinutes}m</td>
      <td className="px-3 py-2 text-muted-foreground">
        {lastUpdatedMs === null ? "—" : new Date(lastUpdatedMs).toLocaleString()}
      </td>
      <td className={`px-3 py-2 font-medium ${STATUS_TONE[status]}`}>{STATUS_LABEL[status]}</td>
    </tr>
  )
}

/**
 * Overview "Freshness" strip (grand plan §7). Every table with a
 * `dataset_sla` row is rendered as one of three distinct states — "on
 * time", "late", or "not measurable" — never a two-state on/off signal:
 * a table's snapshot time is either inside the expected interval, past
 * it, or could not be determined at all (WS5 plan review U12/U15). A
 * table that has no `dataset_sla` row is not shown here at all — it has
 * no expectation to compare against, which is different from "on time".
 *
 * A `permission_denied` service error already renders as `ErrorState`'s
 * "You don't have access" (`page-states.tsx`'s `PermissionState`), so no
 * new permission-branching code is needed here (U12).
 */
export function FreshnessStrip() {
  const slas = useService((signal) => governanceService.listDatasetSla(signal), [])
  if (slas.status === "loading") return <LoadingSkeleton rows={3} />
  if (slas.status === "error") return <ErrorState error={slas.error} onRetry={slas.reload} />
  if (slas.data.length === 0) {
    return (
      <EmptyState
        title="No freshness SLAs configured"
        description="Add one from Governance to see dataset freshness here."
      />
    )
  }
  return (
    <SectionCard title="Freshness" description="Expected vs. actual update time per table">
      <div className="overflow-hidden rounded-lg border border-border">
        <table className="w-full text-sm">
          <thead className="bg-muted/40 text-left text-xs text-muted-foreground">
            <tr>
              <th className="px-3 py-2 font-medium">Table</th>
              <th className="px-3 py-2 font-medium">Expected</th>
              <th className="px-3 py-2 font-medium">Last updated</th>
              <th className="px-3 py-2 font-medium">Status</th>
            </tr>
          </thead>
          <tbody>
            {slas.data.map((sla) => (
              <FreshnessRow key={sla.tableName} sla={sla} />
            ))}
          </tbody>
        </table>
      </div>
    </SectionCard>
  )
}
