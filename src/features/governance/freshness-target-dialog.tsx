"use client"

import * as React from "react"
import { SelectField } from "@/components/patterns/select-field"
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
import { useServiceAction } from "@/hooks/use-service"
import { notifyError, notifySuccess } from "@/lib/notify"
import { governanceService } from "@/services"
import type { DatasetSla } from "@/services/contracts/governance"
import { TablePicker, useTableCatalog } from "./table-picker"

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

/**
 * Set, change or remove one table's freshness SLA (`PUT` /
 * `DELETE /api/governance/sla`). It was the asset page's own dialog; `WEB-3`
 * S3-D5 reuses it on Data Quality's Freshness targets tab, so it takes the
 * SLA's table key rather than an asset. With no `table` it asks for one
 * (adding a target from the tab); the server checks it is
 * `<namespace>.<table>`.
 */
export function FreshnessTargetDialog({
  table,
  label,
  currentMinutes,
  onClose,
  onSaved,
}: {
  /** The SLA's table key (`silver.orders`), or absent to ask for one. */
  table?: string
  /** What the description calls the table: its name on the asset page. */
  label?: string
  /** The SLA in force, or `null` when the table has none (nothing to remove). */
  currentMinutes: number | null
  onClose: () => void
  onSaved: () => void
}) {
  // Start from the SLA in force; without one the form starts from a day.
  const current = currentMinutes ? inLargestUnit(currentMinutes) : { amount: "1", unit: "days" as Unit }
  const [tableText, setTableText] = React.useState("")
  const [amount, setAmount] = React.useState(current.amount)
  const [unit, setUnit] = React.useState<Unit>(current.unit)
  const save = useServiceAction((signal, input: DatasetSla) =>
    governanceService.putDatasetSla(input, signal)
  )
  const remove = useServiceAction((signal, name: string) =>
    governanceService.deleteDatasetSla(name, signal)
  )
  // WEB-8 D8: a new target's table is picked from the catalog.
  const catalog = useTableCatalog()
  const minutes = slaMinutes(amount, unit)
  const tableName = table ?? tableText.trim()
  const hasSla = currentMinutes !== null && table !== undefined

  async function removeTarget() {
    // `run` resolves to `null` only on failure.
    if ((await remove.run(tableName)) === null) {
      notifyError("Failed to remove the freshness target", remove.error)
      return
    }
    notifySuccess("Freshness target removed")
    onSaved()
  }

  async function submit() {
    if (minutes === null || tableName === "") return
    const saved = await save.run({ tableName, expectedIntervalMinutes: minutes })
    if (saved === null) return
    notifySuccess("Freshness target saved")
    onSaved()
  }

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{table === undefined ? "Add freshness target" : "Set freshness target"}</DialogTitle>
          <DialogDescription>
            How often {label ?? table ?? "the table"} must receive new data. Past that, it reads as
            late and its health as degraded.
          </DialogDescription>
        </DialogHeader>
        {table === undefined ? (
          <TablePicker
            id="sla-table"
            label="Table"
            value={tableText}
            onChange={setTableText}
            catalog={catalog}
            description={
              <>
                The table as <span className="font-mono">namespace.table</span>, the key its freshness
                is checked under.
              </>
            }
          />
        ) : null}
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
            <SelectField
              aria-label="Unit"
              value={unit}
              options={(Object.keys(UNIT_MINUTES) as Unit[]).map((u) => ({
                value: u,
                label: u,
              }))}
              onValueChange={(next) => setUnit(next as Unit)}
            />
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
              title="The table goes back to having no target of its own"
            >
              {remove.status === "pending" ? "Removing…" : "Remove target"}
            </Button>
          ) : null}
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            onClick={() => void submit()}
            disabled={
              minutes === null || tableName === "" || save.status === "pending" || remove.status === "pending"
            }
          >
            {save.status === "pending" ? "Saving…" : "Save target"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
