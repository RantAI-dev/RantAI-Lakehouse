"use client"

import * as React from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"

/**
 * One text field per column of the result, so a reader can narrow the grid
 * on each column on its own.
 *
 * The fields filter the rows the section holds, so the grid, the statistics
 * and the copy all read the same rows. The state lives with the section that
 * owns the grid and never reaches the URL: the shared table keeps its own
 * filter state in the query string, and two filter systems writing the same
 * place would fight over it.
 */
export function QueryColumnFilters({
  columns,
  filters,
  onFiltersChange,
}: {
  /** The result's columns, in the result's order. */
  readonly columns: readonly string[]
  /** The text typed per column; a column that is not here has no filter. */
  readonly filters: Readonly<Record<string, string>>
  /** Called with the whole set of filters, so the caller owns the state. */
  readonly onFiltersChange: (filters: Record<string, string>) => void
}) {
  const id = React.useId()
  const hasFilters = Object.values(filters).some((text) => text !== "")

  return (
    <div className="flex flex-wrap items-end gap-2">
      {columns.map((column, index) => {
        const fieldId = `${id}-${index}`
        return (
          <div key={column} className="flex min-w-32 flex-col gap-1">
            <label htmlFor={fieldId} className="text-xs text-muted-foreground">
              Filter {column}
            </label>
            <Input
              id={fieldId}
              value={filters[column] ?? ""}
              placeholder="contains"
              onChange={(event) =>
                onFiltersChange({ ...filters, [column]: event.target.value })
              }
            />
          </div>
        )
      })}
      {hasFilters ? (
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={() => onFiltersChange({})}
        >
          Clear filters
        </Button>
      ) : null}
    </div>
  )
}
