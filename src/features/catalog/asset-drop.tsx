"use client"

import * as React from "react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { Button } from "@/components/ui/button"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatDate, formatNumber } from "@/lib/format"
import { notifySuccess } from "@/lib/notify"
import { assetService } from "@/services"
import type { AssetDetail, AssetDropStatus } from "@/services/contracts/assets"

/**
 * Drop table on the table's page (`DATA-55`, QA `LH-DATA-15-06`, `TC-1731`,
 * `DEC-21`): a table a file upload made can be dropped from the Activity
 * tab's Snapshots card, with the 7-day restore offered on the same card once
 * it is dropped.
 *
 * Nothing renders unless the server says the table may be dropped: a loading
 * read, an error, `supported: false` (outside the tenant that owns the shared
 * catalog) and `droppable: false` all render nothing, so no control is offered
 * for a table the drop route would refuse. The permission check that hides the
 * control is in `AssetActivity`.
 */

/** Whether the server's answer is the one that offers the button. */
function droppableOrNull(status: AssetDropStatus | null): { table: string; keptDays: number } | null {
  if (status === null || status.supported !== undefined || !status.droppable) return null
  return { table: status.table, keptDays: status.keptDays }
}

export function DropTableAction({ asset, onDropped }: { asset: AssetDetail; onDropped: () => void }) {
  const status = useService(
    (s) => assetService.getDropStatus(asset.id, undefined, s),
    [asset.id]
  )
  const [open, setOpen] = React.useState(false)
  const ready = droppableOrNull(status.status === "success" ? status.data : null)
  if (ready === null) return null

  return (
    <>
      <Button size="sm" variant="outline" onClick={() => setOpen(true)}>
        Drop table
      </Button>
      {open ? (
        <DropTableDialog
          asset={asset}
          table={ready.table}
          keptDays={ready.keptDays}
          onClose={() => setOpen(false)}
          onDropped={onDropped}
        />
      ) : null}
    </>
  )
}

/**
 * The confirmation (`DATA-55` D15): it names the table and its row count — the
 * count is a scan of the whole table, so it is asked for here, when the dialog
 * opens, and nowhere else — says how long the table can be restored and what
 * will fail until it is, and confirms nothing until that count has answered.
 * A refusal keeps the dialog open with the server's own sentence.
 */
function DropTableDialog({
  asset,
  table,
  keptDays,
  onClose,
  onDropped,
}: {
  asset: AssetDetail
  table: string
  keptDays: number
  onClose: () => void
  onDropped: () => void
}) {
  const count = useService(
    (s) => assetService.getDropStatus(asset.id, { withRowCount: true }, s),
    [asset.id]
  )
  const drop = useServiceAction((signal) => assetService.dropTable(asset.id, signal))

  const counted = count.status === "success" ? count.data : null
  const canConfirm = counted !== null && counted.supported === undefined && counted.droppable

  const description = (() => {
    if (count.status === "loading") return `Counting the rows of ${table}…`
    if (count.status === "error") return count.error.message
    // Loading and error are answered above, so this branch is unreachable; it
    // keeps the sentence shape for the type checker rather than showing blank.
    if (counted === null) return `Counting the rows of ${table}…`
    if (counted.supported !== undefined || !counted.droppable) return counted.reason
    const rows = counted.rowCount
    if (rows === null || rows === undefined) return `The rows of ${table} could not be counted.`
    return `${table} holds ${formatNumber(rows)} row${rows === 1 ? "" : "s"}.`
  })()

  const dependents = asset.dependents.length
  const readers =
    dependents === 1
      ? "1 saved query or dashboard that you can open reads it."
      : dependents > 1
        ? `${formatNumber(dependents)} saved queries and dashboards that you can open read it.`
        : null
  const impact = `It can be restored from this page for ${keptDays} days. After that it is gone for good. Nothing else is deleted: pipelines, quality rules, saved queries and dashboards that read this table will fail until it is restored.${
    readers ? ` ${readers}` : ""
  }`

  async function confirm() {
    const result = await drop.run()
    if (result === null) return
    notifySuccess("Table dropped", `It can be restored from this page until ${formatDate(result.expiresAt)}.`)
    onClose()
    onDropped()
  }

  return (
    <ConfirmActionDialog
      open
      onOpenChange={(next) => {
        if (!next) onClose()
      }}
      title={`Drop ${table}?`}
      description={description}
      impact={impact}
      confirmLabel="Drop table"
      destructive
      confirming={drop.status === "pending"}
      confirmDisabled={!canConfirm}
      onConfirm={async () => {
        await confirm()
      }}
    >
      {drop.error !== null ? (
        <p role="alert" className="text-sm text-destructive">
          {drop.error.message}
        </p>
      ) : null}
    </ConfirmActionDialog>
  )
}
