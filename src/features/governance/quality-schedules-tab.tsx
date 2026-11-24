"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
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
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { notifySuccess } from "@/lib/notify"
import { governanceService } from "@/services"
import type { QualitySchedule } from "@/services/contracts/governance"
import { AddScheduleDialog } from "./quality-schedule-add-dialog"
import { intervalLabel, ScheduleControls, scheduleSentence } from "./quality-schedule"
import { TableKeyLink } from "./table-key-link"

/**
 * Data Quality's Schedules tab (`WEB-3` S3-D5): every table that has a
 * schedule of its own, from `GET /api/governance/quality/schedules`, with
 * the same control the asset page uses to change one. The endpoint lists
 * only tables with a saved schedule (`routes/quality_schedules.rs`); every
 * other table runs on the default, which the tab says in one line rather
 * than listing tables it was not told about. `DATA-46` adds a schedule for a
 * table that has none through a catalog table picker; changing or resetting
 * needs `governance:write`, so a reader sees the list without the buttons.
 *
 * **Reset** (S3a-D3) deletes the table's own schedule, so the table goes
 * back to the default and leaves this list.
 */
export function QualitySchedulesTab({ canWrite }: { canWrite: boolean }) {
  const state = useService((signal) => governanceService.listQualitySchedules(signal), [])
  const [adding, setAdding] = React.useState(false)
  const [editing, setEditing] = React.useState<QualitySchedule | null>(null)
  const [resetting, setResetting] = React.useState<QualitySchedule | null>(null)
  const reset = useServiceAction((signal, asset: string) =>
    governanceService.resetQualitySchedule(asset, signal).then(() => asset)
  )

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
              <div className="flex justify-end gap-2">
                <Button size="sm" variant="outline" onClick={() => setEditing(s)} aria-label={`Edit the schedule of ${s.asset}`}>
                  Edit
                </Button>
                <Button size="sm" variant="ghost" onClick={() => setResetting(s)} aria-label={`Reset the schedule of ${s.asset}`}>
                  Reset
                </Button>
              </div>
            ),
          },
        ]
      : []),
  ]

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-sm text-muted-foreground">
          A table not listed here runs its checks every {intervalLabel(defaultIntervalMinutes)} (the
          default). Its schedule is also on the table&apos;s page, under Quality.
        </p>
        {canWrite ? (
          <Button size="sm" onClick={() => setAdding(true)}>
            <PlusIcon data-icon="inline-start" />
            Add schedule
          </Button>
        ) : null}
      </div>
      <DataTable
        columns={columns}
        rows={schedules}
        rowKey={(s) => s.asset}
        emptyState={
          <EmptyState
            title="No table has a schedule of its own"
            description={
              canWrite
                ? `Every table runs its checks every ${intervalLabel(defaultIntervalMinutes)}. Use Add schedule to set one for a table.`
                : `Every table runs its checks every ${intervalLabel(defaultIntervalMinutes)}.`
            }
            className="py-4"
          />
        }
      />
      {adding ? (
        <AddScheduleDialog
          scheduledTables={schedules.map((s) => s.asset)}
          defaultInterval={defaultIntervalMinutes}
          onClose={() => setAdding(false)}
          onSaved={() => {
            setAdding(false)
            state.reload()
          }}
        />
      ) : null}
      {resetting ? (
        <ConfirmActionDialog
          open
          onOpenChange={(open) => {
            if (open) return
            reset.reset()
            setResetting(null)
          }}
          title="Reset the schedule"
          description={`Put ${resetting.asset} back on the default schedule?`}
          impact={`Its checks then run every ${intervalLabel(defaultIntervalMinutes)}, like every table not listed here.`}
          confirmLabel="Reset"
          confirming={reset.status === "pending"}
          onConfirm={() => {
            void (async () => {
              const done = await reset.run(resetting.asset)
              // A failure stays in the dialog: its sentence is read from
              // `reset.error` on the next render, not from this closure.
              if (done === null) return
              notifySuccess(`${done} is back on the default schedule`)
              setResetting(null)
              state.reload()
            })()
          }}
        >
          {reset.status === "error" ? (
            <p role="alert" className="text-sm text-destructive">
              {reset.error.message}
            </p>
          ) : null}
        </ConfirmActionDialog>
      ) : null}
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
