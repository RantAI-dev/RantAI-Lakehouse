"use client"

import * as React from "react"
import { EllipsisVerticalIcon } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { useServiceAction } from "@/hooks/use-service"
import { connectorService } from "@/services"
import type { IngestTableAction } from "@/services/contracts/connectors"

/**
 * What each action of a run of ONE table says before it starts (`SRC-10`
 * decision 5). The two that remove rows say so, and that they cannot be
 * brought back from here (Iceberg keeps the old snapshot until it expires, but
 * nothing in the console restores it, and saying more would promise it).
 */
export const TABLE_ACTIONS: Record<
  IngestTableAction,
  { menu: string; title: (table: string) => string; impact: (target: string) => string; confirm: string; destructive: boolean }
> = {
  refresh_keep: {
    menu: "Refresh, keeping rows",
    title: (table) => `Refresh ${table}, keeping rows`,
    impact: (target) =>
      `Loads the whole table again now, by its load mode. Rows already in bronze.${target} are kept: a table that adds every row gets a second full copy, and a table that keeps one row per key ends with the source's current rows.`,
    confirm: "Refresh",
    destructive: false,
  },
  refresh_remove: {
    menu: "Refresh, removing rows",
    title: (table) => `Refresh ${table}, removing rows`,
    impact: (target) =>
      `Removes every row in bronze.${target}, then loads the whole table again. The removed rows cannot be brought back from here.`,
    confirm: "Remove rows and refresh",
    destructive: true,
  },
  clear: {
    menu: "Clear",
    title: (table) => `Clear ${table}`,
    impact: (target) =>
      `Removes every row in bronze.${target} and loads nothing. The removed rows cannot be brought back from here, and the next run starts from the beginning.`,
    confirm: "Clear table",
    destructive: true,
  },
}

const ORDER: IngestTableAction[] = ["refresh_keep", "refresh_remove", "clear"]

/** The words for an action a run list shows; an action a newer API adds is shown as it is. */
export function tableActionLabel(action: string): string {
  return TABLE_ACTIONS[action as IngestTableAction]?.menu ?? action
}

/**
 * Refresh or clear one saved table, each behind a confirmation. The menu is
 * replaced by a disabled button whose title is the reason while the table
 * cannot be run (a run is going, the connector is paused, a schema change waits).
 * The API's own refusal (a run started elsewhere in the meantime, a table that
 * began to wait) is shown in the dialog exactly as it answered.
 */
export function TableActionsMenu({
  connectorId,
  table,
  target,
  disabledReason,
  onStarted,
}: {
  connectorId: string
  /** The source object's name, as in the ingest spec. */
  table: string
  /** The Bronze table it lands in. */
  target: string
  /** Why no action can start now, or `null`. */
  disabledReason: string | null
  /** Called with the new run's id. */
  onStarted: (runId: string) => void
}) {
  const [asked, setAsked] = React.useState<IngestTableAction | null>(null)
  const start = useServiceAction((signal, action: IngestTableAction) =>
    connectorService.runIngest(connectorId, signal, { object: table, action })
  )
  const label = `Refresh or clear ${table}`
  const triggerClass =
    "inline-flex size-6 items-center justify-center rounded text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50"

  function close() {
    setAsked(null)
    start.reset()
  }

  async function confirm() {
    if (!asked) return
    const result = await start.run(asked)
    if (result?.runId) {
      onStarted(result.runId)
      close()
    }
  }

  if (disabledReason !== null) {
    return (
      <button type="button" disabled aria-label={label} title={disabledReason} className={triggerClass}>
        <EllipsisVerticalIcon className="size-3.5" />
      </button>
    )
  }

  const info = asked ? TABLE_ACTIONS[asked] : null
  const refusal = start.error?.message ?? (start.data?.supported === false ? start.data.reason : null)
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger aria-label={label} className={triggerClass}>
          <EllipsisVerticalIcon className="size-3.5" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-52">
          {ORDER.map((action) => (
            <DropdownMenuItem key={action} onClick={() => setAsked(action)}>
              {TABLE_ACTIONS[action].menu}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
      {asked && info ? (
        <ConfirmActionDialog
          open
          onOpenChange={(open) => {
            if (!open) close()
          }}
          title={info.title(table)}
          description={info.impact(target)}
          confirmLabel={info.confirm}
          confirming={start.status === "pending"}
          destructive={info.destructive}
          onConfirm={() => void confirm()}
        >
          {refusal ? <p className="text-sm text-destructive">{refusal}</p> : null}
        </ConfirmActionDialog>
      ) : null}
    </>
  )
}
