"use client"

import * as React from "react"
import { useParams } from "next/navigation"
import { PageHeader } from "@/components/patterns/page-header"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatBytes } from "@/lib/format"
import {
  validateMaintenancePolicyForm,
  type MaintenancePolicyFormValues,
} from "@/lib/lakehouse-maintenance-form"
import {
  maintenanceSummary,
  snapshotRelativeTime,
  snapshotsNewestFirst,
} from "@/lib/lakehouse-view"
import { fmtMeasured } from "@/lib/measured"
import { lakehouseService } from "@/services"
import type { LakehouseMaintenance } from "@/services/contracts/lakehouse"

/** Text-input value ("" or a digit string) parsed to the form's `number | null`. */
function parseOptionalInt(raw: string): number | null {
  if (raw.trim() === "") return null
  const n = Number(raw)
  return Number.isFinite(n) ? Math.trunc(n) : null
}

function formValuesFromPolicy(m: LakehouseMaintenance): MaintenancePolicyFormValues {
  return {
    snapshotsToKeep: m.snapshotsToKeep,
    orphanAgeHours: m.orphanAgeHours,
    compactSmallFiles: m.compactSmallFiles,
    schedule: m.schedule,
  }
}

/**
 * Read-only Iceberg table detail (schema, partition spec, a newest-first
 * snapshot timeline, stats) plus a maintenance-policy write form (WS2 §4
 * Task B4). `orphanAgeHours` is stored by this form but not applied by
 * this build — `remove_orphan_files` runs without an age argument on this
 * `ClickHouse` — so the field is labelled honestly rather than implying it
 * takes effect.
 */
export function LakehouseTableDetailPage() {
  const params = useParams<{ namespace: string; table: string }>()
  const detailState = useService(
    (s) => lakehouseService.getTableDetail(params.namespace, params.table, s),
    [params.namespace, params.table]
  )
  const maintenanceState = useService(
    (s) => lakehouseService.getMaintenance(params.namespace, params.table, s),
    [params.namespace, params.table]
  )
  const savePolicy = useServiceAction((signal, values: MaintenancePolicyFormValues) =>
    lakehouseService.setMaintenancePolicy(
      params.namespace,
      params.table,
      values,
      signal
    )
  )

  const [form, setForm] = React.useState<MaintenancePolicyFormValues | null>(null)
  React.useEffect(() => {
    if (maintenanceState.status === "success") {
      setForm(formValuesFromPolicy(maintenanceState.data))
    }
  }, [maintenanceState.status, maintenanceState.data])

  const formErrors = form === null ? {} : validateMaintenancePolicyForm(form)
  const hasErrors = Object.keys(formErrors).length > 0

  async function handleSave() {
    if (form === null || hasErrors) return
    const result = await savePolicy.run(form)
    if (result !== null) maintenanceState.reload()
  }

  if (detailState.status === "loading") return <LoadingSkeleton rows={8} />
  if (detailState.status === "error") {
    if (detailState.error.code === "not_found") {
      return (
        <EmptyState
          title="Table not found"
          description={`${params.namespace}.${params.table} does not exist in this warehouse.`}
        />
      )
    }
    return <ErrorState error={detailState.error} onRetry={detailState.reload} />
  }
  const detail = detailState.data
  const snapshots = snapshotsNewestFirst(detail.snapshots)

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title={`${params.namespace}.${params.table}`}
        description="Iceberg schema, partitioning, snapshots, and maintenance policy."
      />

      <SectionCard size="sm" title="Schema">
        {detail.schema.length === 0 ? (
          <EmptyState title="No schema fields" className="py-4" />
        ) : (
          <div className="overflow-hidden rounded-lg border border-border">
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead>ID</TableHead>
                  <TableHead>Name</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead>Required</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {detail.schema.map((f) => (
                  <TableRow key={f.id}>
                    <TableCell className="font-mono text-xs">{f.id}</TableCell>
                    <TableCell className="font-mono text-xs">{f.name}</TableCell>
                    <TableCell className="font-mono text-xs">{f.type}</TableCell>
                    <TableCell>{f.required ? "Yes" : "No"}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </SectionCard>

      <SectionCard size="sm" title="Partition spec">
        {detail.partitionSpec.length === 0 ? (
          <EmptyState title="This table is unpartitioned" className="py-4" />
        ) : (
          <ul className="divide-y divide-border text-sm">
            {detail.partitionSpec.map((p) => (
              <li
                key={`${p.sourceId}-${p.name}`}
                className="flex flex-wrap items-baseline gap-2 py-1.5"
              >
                <span className="font-mono font-medium">{p.name}</span>
                <span className="text-muted-foreground">{p.transform}</span>
                <span className="ml-auto text-xs text-muted-foreground">
                  source #{p.sourceId}
                </span>
              </li>
            ))}
          </ul>
        )}
      </SectionCard>

      <SectionCard size="sm" title="Snapshots" description="Newest first.">
        {snapshots.length === 0 ? (
          <EmptyState title="No snapshots for this table" className="py-4" />
        ) : (
          <ul className="divide-y divide-border text-sm">
            {snapshots.map((snap) => (
              <li key={snap.id} className="flex justify-between gap-2 py-1.5">
                <span>{snap.operation}</span>
                <span className="text-muted-foreground">
                  {snapshotRelativeTime(snap.timestampMs)} ·{" "}
                  {fmtMeasured(snap.summary.totalRecords)} records
                </span>
              </li>
            ))}
          </ul>
        )}
      </SectionCard>

      <SectionCard size="sm" title="Stats">
        <dl className="grid grid-cols-2 gap-3 text-sm sm:grid-cols-3">
          <div>
            <dt className="text-xs text-muted-foreground">Files</dt>
            <dd>{fmtMeasured(detail.stats.fileCount)}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Small files</dt>
            <dd>{fmtMeasured(detail.stats.smallFileCount)}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Records</dt>
            <dd>{fmtMeasured(detail.stats.recordCount)}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Size</dt>
            <dd>{fmtMeasured(detail.stats.totalBytes, formatBytes)}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Snapshots</dt>
            <dd>{detail.stats.snapshotCount}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Metadata log entries</dt>
            <dd>{detail.stats.metadataLogCount}</dd>
          </div>
        </dl>
      </SectionCard>

      <SectionCard
        size="sm"
        title="Maintenance policy"
        description="Snapshot retention, orphan-file age, and compaction for the maintenance job."
      >
        {maintenanceState.status === "loading" ? <LoadingSkeleton rows={2} /> : null}
        {maintenanceState.status === "error" ? (
          <ErrorState error={maintenanceState.error} onRetry={maintenanceState.reload} />
        ) : null}
        {maintenanceState.status === "success" && form !== null ? (
          <div className="flex flex-col gap-4 text-sm">
            <p className="text-muted-foreground">{maintenanceSummary(maintenanceState.data)}</p>

            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div className="grid gap-1.5">
                <Label htmlFor="snapshots-to-keep">Snapshots to keep</Label>
                <Input
                  id="snapshots-to-keep"
                  type="number"
                  min={1}
                  placeholder="Default"
                  value={form.snapshotsToKeep ?? ""}
                  onChange={(e) =>
                    setForm({ ...form, snapshotsToKeep: parseOptionalInt(e.target.value) })
                  }
                />
                {formErrors.snapshotsToKeep ? (
                  <p className="text-xs text-destructive">{formErrors.snapshotsToKeep}</p>
                ) : null}
              </div>

              <div className="grid gap-1.5">
                <Label htmlFor="orphan-age-hours">
                  Orphan-file age (hours) — stored; not applied by this build
                </Label>
                <Input
                  id="orphan-age-hours"
                  type="number"
                  min={1}
                  placeholder="Default"
                  value={form.orphanAgeHours ?? ""}
                  onChange={(e) =>
                    setForm({ ...form, orphanAgeHours: parseOptionalInt(e.target.value) })
                  }
                />
                {formErrors.orphanAgeHours ? (
                  <p className="text-xs text-destructive">{formErrors.orphanAgeHours}</p>
                ) : null}
              </div>

              <div className="grid gap-1.5">
                <Label>Compact small files</Label>
                <Select
                  value={form.compactSmallFiles ? "on" : "off"}
                  onValueChange={(v) => setForm({ ...form, compactSmallFiles: v === "on" })}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="off">Off</SelectItem>
                    <SelectItem value="on">On</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div className="grid gap-1.5">
                <Label>Schedule</Label>
                <Select
                  value={form.schedule ?? "none"}
                  onValueChange={(v) => setForm({ ...form, schedule: v === "none" ? null : v })}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="none">None</SelectItem>
                    <SelectItem value="daily">Daily</SelectItem>
                    <SelectItem value="weekly">Weekly</SelectItem>
                  </SelectContent>
                </Select>
                {formErrors.schedule ? (
                  <p className="text-xs text-destructive">{formErrors.schedule}</p>
                ) : null}
              </div>
            </div>

            {savePolicy.status === "error" ? (
              <ErrorState error={savePolicy.error} onRetry={handleSave} />
            ) : null}

            <div>
              <Button
                onClick={handleSave}
                disabled={hasErrors || savePolicy.status === "pending"}
              >
                {savePolicy.status === "pending" ? "Saving…" : "Save policy"}
              </Button>
            </div>

            <p className="text-muted-foreground">
              {maintenanceState.data.lastRun === null
                ? "No maintenance run recorded"
                : `Last run ${maintenanceState.data.lastRun.runAt} — applied ${maintenanceState.data.lastRun.applied.deletedDataFiles} data file(s), ${maintenanceState.data.lastRun.applied.deletedManifestFiles} manifest file(s) deleted`}
            </p>

            {maintenanceState.data.lastVerbRuns.length > 0 ? (
              <div className="overflow-hidden rounded-lg border border-border">
                <Table>
                  <TableHeader>
                    <TableRow className="hover:bg-transparent">
                      <TableHead>Verb</TableHead>
                      <TableHead>Engine</TableHead>
                      <TableHead>Outcome</TableHead>
                      <TableHead>Detail</TableHead>
                      <TableHead>Ran at</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {maintenanceState.data.lastVerbRuns.map((run) => (
                      <TableRow key={run.verb}>
                        <TableCell className="font-mono text-xs">{run.verb}</TableCell>
                        <TableCell className="font-mono text-xs">{run.engine}</TableCell>
                        <TableCell>{run.outcome}</TableCell>
                        <TableCell className="text-muted-foreground">
                          {run.detail === "" ? "—" : run.detail}
                        </TableCell>
                        <TableCell className="text-muted-foreground">{run.runAt}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            ) : null}
          </div>
        ) : null}
      </SectionCard>
    </div>
  )
}
