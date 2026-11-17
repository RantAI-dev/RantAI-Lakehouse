"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { governanceService } from "@/services"
import type { DatasetSla } from "@/services/contracts/governance"
import { FreshnessTargetDialog } from "./freshness-target-dialog"
import { intervalLabel } from "./quality-schedule"
import { TableKeyLink } from "./table-key-link"

/**
 * Data Quality's Freshness targets tab (`WEB-3` S3-D5): every freshness SLA
 * (`GET /api/governance/sla`), with Add, Edit and Remove through the same
 * dialog the asset page uses. A table past its target reads as late and its
 * health as degraded. Changing needs `governance:write`, as the API demands.
 */
export function FreshnessTargetsTab({ canWrite }: { canWrite: boolean }) {
  const state = useService((signal) => governanceService.listDatasetSla(signal), [])
  // `"new"` is the Add dialog; a target is its Edit dialog.
  const [editing, setEditing] = React.useState<DatasetSla | "new" | null>(null)
  const [removing, setRemoving] = React.useState<DatasetSla | null>(null)
  const remove = useServiceAction(
    withNotify(
      { success: "Freshness target removed", error: "Failed to remove the freshness target" },
      (signal, table: string) => governanceService.deleteDatasetSla(table, signal)
    )
  )

  async function confirmRemove() {
    if (!removing) return
    // `run` resolves to `null` only on failure, which `withNotify` reports.
    if ((await remove.run(removing.tableName)) === null) return
    setRemoving(null)
    state.reload()
  }

  const columns: ColumnDef<DatasetSla>[] = [
    {
      key: "table",
      header: "Table",
      render: (t) => <TableKeyLink tableKey={t.tableName} tab="activity" />,
      sortValue: (t) => t.tableName,
    },
    {
      key: "target",
      header: "New data at least every",
      render: (t) => intervalLabel(t.expectedIntervalMinutes),
      sortValue: (t) => t.expectedIntervalMinutes,
    },
    {
      key: "owner",
      header: "Owner",
      render: (t) => t.owner ?? <span className="text-muted-foreground">not set</span>,
    },
    ...(canWrite
      ? [
          {
            key: "actions",
            header: "",
            className: "text-right",
            render: (t: DatasetSla) => (
              <div className="flex justify-end gap-1">
                <Button size="sm" variant="outline" onClick={() => setEditing(t)} aria-label={`Edit the target of ${t.tableName}`}>
                  Edit
                </Button>
                <Button size="sm" variant="ghost" onClick={() => setRemoving(t)} aria-label={`Remove the target of ${t.tableName}`}>
                  Remove
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
          How often each table must receive new data. Past that, it reads as late and its health as
          degraded.
        </p>
        {canWrite ? (
          <Button size="sm" onClick={() => setEditing("new")}>
            <PlusIcon data-icon="inline-start" />
            Add target
          </Button>
        ) : null}
      </div>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" ? (
        <DataTable
          columns={columns}
          rows={state.data}
          rowKey={(t) => t.tableName}
          emptyState={
            <EmptyState
              title="No freshness target"
              description="Without a target, a table's age is shown without calling it on time or late."
              className="py-4"
            />
          }
        />
      ) : null}
      {editing ? (
        <FreshnessTargetDialog
          table={editing === "new" ? undefined : editing.tableName}
          currentMinutes={editing === "new" ? null : editing.expectedIntervalMinutes}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null)
            state.reload()
          }}
        />
      ) : null}
      <ConfirmActionDialog
        open={removing !== null}
        onOpenChange={(open) => (open ? undefined : setRemoving(null))}
        title="Remove this freshness target?"
        description={`${removing?.tableName ?? "The table"} goes back to having no target of its own.`}
        confirmLabel="Remove"
        destructive
        confirming={remove.status === "pending"}
        onConfirm={() => void confirmRemove()}
      />
    </div>
  )
}
