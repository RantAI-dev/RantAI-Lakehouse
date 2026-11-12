"use client"

import * as React from "react"

import {
  Faceted,
  FacetedBadgeList,
  FacetedContent,
  FacetedEmpty,
  FacetedGroup,
  FacetedInput,
  FacetedItem,
  FacetedList,
  FacetedTrigger,
} from "@/components/ui/faceted"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService } from "@/hooks/use-service"
import { tableKeyOf } from "@/lib/table-key"
import { assetService } from "@/services"
import type { AssetDetail } from "@/services/contracts/assets"

/**
 * The table picker the governance forms share (`WEB-8` D8, spec
 * `web-8.md`): a combobox over the catalog's assets, showing each one as the
 * row's table key (`bronze.<table>`, `silver.<table>`, `serving.<table>`) —
 * the name a quality rule, a freshness target and an alert rule all send —
 * with the columns of the picked table read from its own detail.
 *
 * It never invents a table: the options are `assetService.listAssets`, read
 * through `tableKeyOf`. A row's `name` is its display name and a Bronze
 * row's `namespace` is the registry's, so `namespace.name` named no table
 * (QA `LH-DATA-14-01` F2, `DATA-54`). When that read fails the field goes
 * back to the free text it was, with the sentence saying so, rather than an
 * empty list that would read as "this deployment has no tables". A table the
 * record already names that the catalog does not list is offered as its own
 * marked entry, so opening the picker cannot drop it.
 */

export type TableCatalog = {
  /** Every table the catalog lists, sorted; `null` while it is loading or when it could not be read. */
  choices: string[] | null
  /** Why there is no list (a failed read); `null` while loading or once it is ready. */
  note: string | null
  /** The catalog id behind a table key, for the detail read that answers its columns. */
  idOf: (table: string) => string | null
}

/** The catalog's tables as picker options, loaded through `useService`. */
export function useTableCatalog(): TableCatalog {
  const state = useService((signal) => assetService.listAssets({}, signal), [])
  const assets = state.status === "success" ? state.data : null
  // QA `LH-DATA-14-01` F2 (`DATA-54`): the option is the row's table key. A
  // row's `name` is its display name and a Bronze row's `namespace` is the
  // registry's, so the pair named no table. A row the registry gives no table
  // name has no key and is not offered.
  const choices = React.useMemo(
    () =>
      assets === null
        ? null
        : Array.from(
            new Set(assets.map(tableKeyOf).filter((key): key is string => key !== null))
          ).sort(),
    [assets]
  )
  // Review round 1 R11: the fallback names the shape the server accepts. A
  // Bronze row's namespace is the registry's, so `namespace.table` named no
  // table and is answered 400 (QA `LH-DATA-14-01` F2, `DATA-54`).
  const note =
    state.status === "error"
      ? "Could not read the catalog's tables; type the table as silver.<table>, serving.<table> or bronze.<table>."
      : null
  const idOf = React.useCallback(
    (table: string) => assets?.find((asset) => tableKeyOf(asset) === table)?.id ?? null,
    [assets]
  )
  return { choices, note, idOf }
}

/**
 * One table's columns, from the asset's own detail (`AssetDetail.schema`), or
 * `null` when the table has no catalog id or its detail could not be read —
 * the callers then leave their column field as free text.
 */
export function useTableColumns(assetId: string | null): string[] | null {
  const state = useService(
    (signal): Promise<AssetDetail | null> =>
      assetId === null ? Promise.resolve(null) : assetService.getAsset(assetId, signal),
    [assetId]
  )
  return React.useMemo(() => {
    if (state.status !== "success" || state.data === null) return null
    const names = state.data.schema.map((column) => column.name)
    return names.length > 0 ? names : null
  }, [state.status, state.data])
}

/**
 * The combobox itself. `allowEveryTable` adds the `*` entry a quality alert
 * accepts ("every table"); `description` is the sentence the form already
 * carried under the field.
 */
export function TablePicker({
  id,
  label,
  value,
  onChange,
  catalog,
  allowEveryTable = false,
  description,
}: {
  id?: string
  label: string
  value: string
  onChange: (next: string) => void
  catalog: TableCatalog
  allowEveryTable?: boolean
  description?: React.ReactNode
}) {
  const options = React.useMemo(() => {
    const tables = catalog.choices ?? []
    return [
      ...(allowEveryTable ? [{ value: "*", label: "Every table" }] : []),
      ...tables.map((table) => ({ value: table, label: table })),
      // A table the form already holds but the catalog does not list: shown,
      // selectable back, and never silently replaced.
      ...(value !== "" && value !== "*" && !tables.includes(value)
        ? [{ value, label: `${value} (not in the catalog)` }]
        : []),
    ]
  }, [allowEveryTable, catalog.choices, value])

  return (
    <div className="grid gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      {catalog.choices === null ? (
        <>
          <Input
            id={id}
            aria-label={label}
            value={value}
            onChange={(e) => onChange(e.target.value)}
            placeholder="namespace.table"
          />
          {catalog.note ? <p className="text-xs text-muted-foreground">{catalog.note}</p> : null}
        </>
      ) : (
        <Faceted value={value === "" ? undefined : value} onValueChange={(next) => onChange(next ?? "")}>
          <FacetedTrigger asChild>
            <Button
              id={id}
              type="button"
              variant="outline"
              size="sm"
              className="w-full justify-between font-normal"
              aria-label={label}
            >
              <FacetedBadgeList
                options={options}
                placeholder="Pick a table"
                badgeClassName="font-mono"
              />
            </Button>
          </FacetedTrigger>
          <FacetedContent className="w-64">
            <FacetedInput placeholder="Search tables" aria-label={`Search ${label}`} />
            <FacetedList>
              <FacetedEmpty>No table matches.</FacetedEmpty>
              <FacetedGroup>
                {options.map((option) => (
                  <FacetedItem key={option.value} value={option.value}>
                    <span className="font-mono text-xs">{option.label}</span>
                  </FacetedItem>
                ))}
              </FacetedGroup>
            </FacetedList>
          </FacetedContent>
        </Faceted>
      )}
      {description ? <div className="text-xs text-muted-foreground">{description}</div> : null}
    </div>
  )
}
