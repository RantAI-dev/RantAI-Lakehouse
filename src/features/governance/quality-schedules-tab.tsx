"use client"

import * as React from "react"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { governanceService } from "@/services"
import type { QualitySchedule } from "@/services/contracts/governance"
import { intervalLabel, ScheduleControls, scheduleSentence } from "./quality-schedule"
import { TableKeyLink } from "./table-key-link"

/**
 * Data Quality's Schedules tab (`WEB-3` S3-D5): every table that has a
 * schedule of its own, from `GET /api/governance/quality/schedules`, with
 * the same control the asset page uses to change one. The endpoint lists
 * only tables with a saved schedule (`routes/quality_schedules.rs`); every
 * other table runs on the default, which the tab says in one line rather
 * than listing tables it was not told about. Changing needs
 * `governance:write`; a reader sees the list without the button.
 */
export function QualitySchedulesTab({ canWrite }: { canWrite: boolean }) {
  const state = useService((signal) => governanceService.listQualitySchedules(signal), [])
  const [editing, setEditing] = React.useState<QualitySchedule | null>(null)

  if (state.status === "loading") return <LoadingSkeleton />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={state.reload} />
  const { schedules, defaultIntervalMinutes } = state.data

  const columns: ColumnDef<QualitySchedule>[] = [
    {
      key: "table",
      header: "Table",
      render: (s) => <TableKeyLink tableKey={s.asset} tab="quality" />,
      sortValue: (s) => s.asset,
    },
    {
      key: "runs",
      header: "Checks run",
      render: (s) => scheduleSentence(s, defaultIntervalMinutes),
    },
    {
      key: "changed",
      header: "Changed",
      render: (s) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(s.updatedAt)}
          {s.updatedBy ? ` by ${s.updatedBy}` : ""}
        </span>
      ),
      sortValue: (s) => s.updatedAt,
    },
    ...(canWrite
      ? [
          {
            key: "actions",
            header: "",
            className: "text-right",
            render: (s: QualitySchedule) => (
              <Button size="sm" variant="outline" onClick={() => setEditing(s)} aria-label={`Edit the schedule of ${s.asset}`}>
                Edit
              </Button>
            ),
          },
        ]
      : []),
  ]

  return (
    <div className="flex flex-col gap-3">
      <p className="text-sm text-muted-foreground">
        A table not listed here runs its checks every {intervalLabel(defaultIntervalMinutes)} (the
        default). A table&apos;s schedule is also on its page, under Quality.
      </p>
      <DataTable
        columns={columns}
        rows={schedules}
        rowKey={(s) => s.asset}
        emptyState={
          <EmptyState
            title="No table has a schedule of its own"
            description={`Every table runs its checks every ${intervalLabel(defaultIntervalMinutes)}. Set another schedule from a table's Quality tab.`}
            className="py-4"
          />
        }
      />
      {editing ? (
        <Dialog open onOpenChange={(open) => (open ? undefined : setEditing(null))}>
          <DialogContent className="sm:max-w-lg">
            <DialogHeader>
              <DialogTitle>Change the schedule</DialogTitle>
              <DialogDescription>
                When the checks of <span className="font-mono">{editing.asset}</span> run.
              </DialogDescription>
            </DialogHeader>
            <ScheduleControls
              asset={editing.asset}
              current={editing}
              defaultInterval={defaultIntervalMinutes}
              onSaved={() => {
                setEditing(null)
                state.reload()
              }}
            />
          </DialogContent>
        </Dialog>
      ) : null}
    </div>
  )
}
