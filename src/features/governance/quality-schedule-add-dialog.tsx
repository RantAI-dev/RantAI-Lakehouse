"use client"

import * as React from "react"
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
import { ScheduleControls } from "./quality-schedule"
import { TablePicker, useTableCatalog } from "./table-picker"

/**
 * Add a schedule for a table from Data Quality's Schedules tab (`DATA-46`).
 * `WEB-8` D8's catalog picker supplies the options; a table the tab already
 * lists is left out, because changing or resetting that schedule is the row's
 * Edit/Reset action, not a second "add". When the catalog cannot be read the
 * picker falls back to a typed `namespace.table` with its own note, so a
 * tenant/catalog refusal is named rather than shown as an empty catalog.
 */
export function AddScheduleDialog({
  scheduledTables,
  defaultInterval,
  onClose,
  onSaved,
}: {
  /** Table keys the tab already lists; excluded from the picker. */
  scheduledTables: string[]
  defaultInterval: number
  onClose: () => void
  onSaved: () => void
}) {
  const catalog = useTableCatalog()
  const [table, setTable] = React.useState("")

  // Do not offer a table twice: its schedule is on a row of the list already.
  // Case is folded because the API stores table keys lowercased.
  const choices = React.useMemo(() => {
    if (catalog.choices === null) return null
    const scheduled = new Set(scheduledTables.map((key) => key.toLowerCase()))
    return catalog.choices.filter((key) => !scheduled.has(key.toLowerCase()))
  }, [catalog.choices, scheduledTables])
  const pickerCatalog = React.useMemo(() => ({ ...catalog, choices }), [catalog, choices])
  // An empty picker has two honest, different reasons: the catalog has no
  // tables, or every one of its tables already has a schedule to edit.
  const description =
    catalog.choices !== null && catalog.choices.length === 0
      ? "The catalog lists no tables to schedule."
      : choices !== null && choices.length === 0
        ? "Every catalog table already has a schedule — change or reset one from the list instead."
        : catalog.choices === null && catalog.note === null
          ? "Loading the catalog's tables; you can also type namespace.table."
          : "Only tables without a saved schedule are offered."

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Add a schedule</DialogTitle>
          <DialogDescription>
            Pick a table from the catalog and choose when its checks run. A table that already has a
            schedule is changed from its row in the list.
          </DialogDescription>
        </DialogHeader>
        <TablePicker
          id="schedule-table"
          label="Table"
          value={table}
          onChange={setTable}
          catalog={pickerCatalog}
          description={description}
        />
        {table !== "" ? (
          // Keyed on the table so a different pick starts from the default
          // schedule rather than the previous table's choice.
          <ScheduleControls
            key={table}
            asset={table}
            current={undefined}
            defaultInterval={defaultInterval}
            onSaved={onSaved}
          />
        ) : null}
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
