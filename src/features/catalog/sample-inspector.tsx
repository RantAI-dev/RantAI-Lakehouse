"use client"

/**
 * The Sample tab's inspector: the panel beside the grid (under it on a
 * narrower screen) that shows what was picked.
 *
 * - **A cell**: its row and column, its whole value (wrapped, `NULL` and an
 *   empty text explained in a sentence) and a Copy button; Copy is
 *   offered for a value only: `NULL` and an empty text have nothing to copy.
 * - **A column**, picked by its header or as the column of a picked cell:
 *   the same opened column the Schema tab shows (`ColumnDetail`), from the
 *   table's profile, so the two tabs state the same facts. The statistics
 *   are the table's, not computed from the rows shown: 25 rows are not a
 *   column.
 *
 * The profile is read from the data, so it is requested by `useProfile` the
 * first time this component is mounted, and the tab mounts it only once
 * something has been picked: opening the tab asks for no profile. It stays
 * mounted after that (rendering nothing while nothing is picked) so closing
 * and picking again does not read the table a second time.
 */
import * as React from "react"
import { Copy, X } from "lucide-react"
import { Button } from "@/components/ui/button"
import { sortKeyColumns, valueShares } from "@/lib/column-profile"
import { cellKind } from "@/lib/sample-grid"
import { notifyError, notifySuccess } from "@/lib/notify"
import type { AssetDetail } from "@/services/contracts/assets"
import { schemaRows, useProfile } from "./asset-column-data"
import { ColumnDetail, EMPTY_VALUE, ProfileNote, type SchemaRow } from "./asset-columns"
import { icebergTableOf, type IcebergTableState } from "./asset-storage"
import { NULL_LABEL, type Picked } from "./sample-grid"

const NOT_IN_SCHEMA = "Not in the schema"

async function copyValue(value: string) {
  try {
    await navigator.clipboard.writeText(value)
    notifySuccess("Copied value")
  } catch (err) {
    // Usually a denied clipboard permission or a non-secure origin.
    notifyError("Failed to copy", err)
  }
}

/** The picked cell's value, in words where it is not a value. */
function CellValue({ value }: { value: string | null | undefined }) {
  const kind = cellKind(value)
  if (kind === "null") {
    return (
      <p className="text-xs">
        <span className="font-mono italic text-muted-foreground">{NULL_LABEL}</span>
        {": the table holds no value in this cell. That is not the same as an empty text."}
      </p>
    )
  }
  if (kind === "empty") {
    return (
      <p className="text-xs">
        <span className="font-mono italic text-muted-foreground">{EMPTY_VALUE}</span>
        {": the cell holds an empty text, with no characters in it. That is not the same as NULL."}
      </p>
    )
  }
  return (
    <pre className="max-h-60 overflow-auto rounded-md border border-border bg-muted/40 p-2 font-mono text-xs break-all whitespace-pre-wrap">
      {value}
    </pre>
  )
}

export function SampleInspector({
  asset: a,
  iceberg,
  picked,
  rowNumber,
  value,
  onClose,
}: {
  asset: AssetDetail
  iceberg: IcebergTableState
  picked: Picked | null
  /** For a picked cell: its place among the rows as shown, from 1. */
  rowNumber: number
  /** For a picked cell: its value. */
  value: string | null | undefined
  onClose: () => void
}) {
  const profile = useProfile(a.id)
  const ref = React.useRef<HTMLElement>(null)
  const open = picked !== null
  // Under the grid, below `xl`, the panel can open out of sight; bring it in, no further than needed.
  React.useEffect(() => {
    if (open) ref.current?.scrollIntoView({ block: "nearest" })
  }, [open])

  if (picked === null) return null

  const name = picked.column
  const { rows } = schemaRows(a.schema, icebergTableOf(iceberg), a.type !== "iceberg-table")
  // A column the schema does not list still opens, on the facts that need no schema.
  const row: SchemaRow = rows.find((r) => r.column.name === name) ?? {
    column: { name, dataType: NOT_IN_SCHEMA },
    nullable: null,
    partition: null,
  }
  const stat = profile.kind === "ready" ? profile.byName.get(name) : undefined
  const shares = profile.kind === "ready" ? valueShares(stat, profile.profile.rowsProfiled) : null
  const sortKey = sortKeyColumns(
    a.storage?.sortingKey,
    rows.map((r) => r.column.name)
  ).has(name)

  return (
    <aside
      ref={ref}
      aria-label="Inspector"
      data-slot="sample-inspector"
      className="flex min-w-0 flex-col gap-3 rounded-lg border border-border bg-card p-3 xl:max-h-[70vh] xl:w-88 xl:shrink-0 xl:overflow-y-auto"
    >
      <div className="flex items-start justify-between gap-2">
        <h3 className="text-sm font-semibold">{picked.kind === "cell" ? "Cell" : "Column"}</h3>
        <Button type="button" size="icon-sm" variant="ghost" aria-label="Close inspector" onClick={onClose}>
          <X aria-hidden />
        </Button>
      </div>

      {picked.kind === "cell" ? (
        <section aria-label="Cell" className="flex flex-col gap-2">
          <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
            <dt className="text-muted-foreground">Row</dt>
            <dd className="tabular-nums">{rowNumber}</dd>
            <dt className="text-muted-foreground">Column</dt>
            <dd className="font-mono break-all">{name}</dd>
          </dl>
          <p className="text-xs font-medium text-muted-foreground">Value</p>
          <CellValue value={value} />
          {cellKind(value) !== "value" ? null : (
            <div>
              <Button type="button" size="sm" variant="outline" onClick={() => void copyValue(value ?? "")}>
                <Copy aria-hidden />
                Copy value
              </Button>
            </div>
          )}
        </section>
      ) : null}

      <section
        aria-label={`Column ${name}`}
        className={picked.kind === "cell" ? "flex flex-col gap-2 border-t border-border pt-3" : "flex flex-col gap-2"}
      >
        {picked.kind === "cell" ? (
          <p className="text-xs font-medium text-muted-foreground">About the column</p>
        ) : (
          <p className="font-mono text-sm font-medium break-all">{name}</p>
        )}
        {/* The panel is 22rem wide beside the grid, too narrow for the facts' viewport-sized columns. */}
        <div className="xl:[&_dl]:grid-cols-1">
          <ColumnDetail row={row} state={profile} column={stat} shares={shares} sortKey={sortKey} />
        </div>
        <p className="text-xs text-muted-foreground">
          <ProfileNote state={profile} />
        </p>
      </section>
    </aside>
  )
}
