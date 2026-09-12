"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { FilterToolbar, SearchField } from "@/components/patterns/filter-toolbar"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Pill } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { governanceService } from "@/services"
import type { MaintenanceRun } from "@/services/contracts/governance"

const columns: ColumnDef<MaintenanceRun>[] = [
  { key: "table", header: "Bronze table", render: (r) => r.tableName },
  { key: "runAt", header: "Run", render: (r) => formatRelativeTime(r.runAt) },
  {
    key: "dry",
    header: "Dry run (would delete)",
    render: (r) => (
      <span className="font-mono text-xs">
        {r.dryRun.deletedDataFiles} data / {r.dryRun.deletedManifestFiles} manifest
      </span>
    ),
  },
  {
    key: "applied",
    header: "Applied",
    render: (r) => (
      <span className="font-mono text-xs">
        {r.applied.deletedDataFiles} data / {r.applied.deletedManifestFiles} manifest
      </span>
    ),
  },
  {
    key: "skipped",
    header: "Skipped verbs",
    render: (r) =>
      r.skippedVerbs ? (
        <Pill tone="warning" title={r.skippedVerbs}>
          {r.skippedVerbs}
        </Pill>
      ) : (
        <span className="text-xs text-muted-foreground">none</span>
      ),
  },
]

/**
 * Bronze Iceberg maintenance runs (P4/P6). Reads `GET
 * /api/governance/maintenance`, which surfaces `lake.bronze_meta.
 * maintenance_run` — written by `dagster/dispar_orchestrate/
 * maintenance.py`'s `bronze_maintenance_job`.
 *
 * Only `expire_snapshots` runs in-engine on this ClickHouse version:
 * `remove_orphan_files` does not exist for Iceberg tables and `OPTIMIZE`
 * fails at runtime with an HTTP 403 against a catalog-registered table
 * (measured in `docs/plans/G3-RESULT.md`). Small-file compaction for Bronze
 * runs out-of-band via the Trino-as-cron escape hatch (ADR 0009) — this
 * page does not surface Trino's run history because it has no `bronze_
 * meta.*` record of its own; only the in-engine `expire_snapshots` chain
 * (dry-run + applied) is tracked here, honestly.
 */
export function MaintenancePage() {
  const state = useService((s) => governanceService.listMaintenanceRuns(s), [])
  const [search, setSearch] = React.useState("")

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q) return state.data ?? []
    return (state.data ?? []).filter((r) => r.tableName.toLowerCase().includes(q))
  }, [state.data, search])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Bronze Maintenance"
        description="expire_snapshots dry-run and applied metrics per Bronze Iceberg table. remove_orphan_files and in-engine OPTIMIZE do not work on this ClickHouse version — see the Trino-as-cron escape hatch (ADR 0009) for small-file compaction."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search Bronze table..."
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <DataTable columns={columns} rows={filtered} rowKey={(r) => `${r.tableName}-${r.runAt}`} />
      ) : null}
    </div>
  )
}
