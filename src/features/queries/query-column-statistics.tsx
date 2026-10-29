"use client"

import * as React from "react"

import { formatNumber } from "@/lib/format"
import {
  columnStatistics,
  histogram,
  type QueryRow,
} from "./query-grid-tools"

/**
 * The statistics of one column of the rows the grid shows: how many cells
 * hold nothing, how many distinct values there are, the smallest and the
 * largest value, and how the values are spread out.
 *
 * The rows are the ones the grid shows, so the numbers describe what the
 * reader is looking at and not the whole result.
 */
export function QueryColumnStatistics({
  columns,
  rows,
}: {
  /** The result's columns, in the result's order. */
  readonly columns: readonly string[]
  /** The rows the grid shows right now. */
  readonly rows: readonly QueryRow[]
}) {
  const id = React.useId()
  const [chosen, setChosen] = React.useState<string | null>(null)
  // The first column is the default. A column that is not in the result any
  // more — a new run — falls back to it as well.
  const column =
    chosen !== null && columns.includes(chosen) ? chosen : (columns[0] ?? "")

  if (columns.length === 0) return null

  const stats = columnStatistics(rows, column)
  const shape = histogram(rows, column)
  const maxCount =
    shape.kind === "bins"
      ? Math.max(0, ...shape.bins.map((bin) => bin.count))
      : Math.max(0, ...shape.values.map((entry) => entry.count))
  // The bar is a share of the fullest bar, so the shape of the distribution
  // reads even when the counts are small.
  const width = (count: number) =>
    maxCount > 0 ? (count / maxCount) * 100 : 0

  return (
    <section
      aria-label="Column statistics"
      className="space-y-3 rounded-md border p-3"
    >
      <div className="flex flex-wrap items-center gap-2">
        <label htmlFor={id} className="text-xs text-muted-foreground">
          Column
        </label>
        <select
          id={id}
          value={column}
          onChange={(event) => setChosen(event.target.value)}
          className="h-8 rounded-lg border border-input bg-transparent px-2.5 text-sm"
        >
          {columns.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
      </div>

      <p className="text-xs text-muted-foreground">
        Statistics for {column} of the {formatNumber(rows.length)} rows shown.
      </p>

      <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-xs sm:grid-cols-4">
        <div>
          <dt className="text-muted-foreground">Empty cells</dt>
          <dd className="font-mono tabular-nums">{formatNumber(stats.empty)}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Distinct values</dt>
          <dd className="font-mono tabular-nums">{formatNumber(stats.distinct)}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Minimum</dt>
          <dd className="font-mono tabular-nums">{stats.min ?? "—"}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Maximum</dt>
          <dd className="font-mono tabular-nums">{stats.max ?? "—"}</dd>
        </div>
      </dl>

      {shape.kind === "bins" ? (
        <ul className="space-y-1">
          {shape.bins.map((bin) => (
            <li key={`${bin.lower}-${bin.upper}`} className="flex items-center gap-2 text-xs">
              <span className="w-24 shrink-0 font-mono tabular-nums text-muted-foreground">
                {edge(bin.lower)}–{edge(bin.upper)}
              </span>
              <span className="h-2 flex-1 rounded bg-muted">
                <span
                  className="block h-2 rounded bg-primary"
                  style={{ width: `${width(bin.count)}%` }}
                />
              </span>
              <span className="w-10 shrink-0 text-right font-mono tabular-nums">
                {formatNumber(bin.count)}
              </span>
            </li>
          ))}
        </ul>
      ) : (
        <ul className="space-y-1">
          {shape.values.map((entry) => (
            <li key={entry.value} className="flex items-center gap-2 text-xs">
              <span className="w-24 shrink-0 truncate font-mono text-muted-foreground">
                {entry.value}
              </span>
              <span className="h-2 flex-1 rounded bg-muted">
                <span
                  className="block h-2 rounded bg-primary"
                  style={{ width: `${width(entry.count)}%` }}
                />
              </span>
              <span className="w-10 shrink-0 text-right font-mono tabular-nums">
                {formatNumber(entry.count)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

/**
 * A bin's edge, rounded to two decimals so the label stays readable.
 *
 * The edges are computed from the smallest and the largest value, so they
 * carry more digits than a reader can use.
 */
function edge(value: number): string {
  return String(Number(value.toFixed(2)))
}
