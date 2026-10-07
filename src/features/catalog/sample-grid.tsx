"use client"

/**
 * The Sample tab's grid: the asset's first rows as a table that reads like
 * data. Plan: `docs/superpowers/plans/2026-10-05-sample-tab-data-preview.md`.
 *
 * Type, glyph and alignment come from the asset's schema by column name; a
 * column the schema does not list gets no glyph, no type and left alignment,
 * never a guess from its values. A cell is its value as stored: `NULL` and an
 * empty text are written out in a quiet italic, a long value is cut at a
 * fixed width and its whole is the cell's title and the inspector's body.
 *
 * # Why a bare `<table>` and not the shared `Table`
 *
 * The shared `Table` wraps its table in an `overflow-x-auto` div with no
 * height, so a sticky header inside it has no scroller to stick to: the
 * header would stay with the page, not with the grid. The grid scrolls in
 * its own frame in both directions, so the table sits directly in that
 * frame.
 *
 * # Sticky parts
 *
 * The header row sticks to the top of the frame, the row-number gutter to its
 * left edge at every width, and the first data column beside the gutter when
 * the frame is at least `@lg` (32rem) wide. That is the *frame's* width, not
 * the viewport's: the inspector takes 22rem of it from `xl` up, and a sticky
 * column of up to 12rem in a frame of 20rem would leave no room to scroll
 * the others. Sticky cells need an opaque background or the rows scrolling
 * under them show through, so every cell paints one from theme tokens
 * (`--card`, `--muted`; both opaque in light and dark) and the tints are
 * `color-mix` over them, not an alpha. The table uses `border-separate`
 * because in a collapsed table the borders of a sticky cell stay behind
 * when it sticks.
 *
 * # One tab stop
 *
 * The body is a roving-tabindex grid: one cell has `tabIndex=0`, the rest
 * `-1`, and the arrow keys, Home and End move focus among them. A grid of
 * 1,800 buttons in the tab order would make the page unusable from the
 * keyboard. The header's controls are roving the same way along the header
 * row (the active column's name and sort button are the two tab stops).
 */
import * as React from "react"
import { ArrowDown, ArrowUp, ArrowUpDown } from "lucide-react"
import { typeFamily } from "@/lib/column-type"
import { cellKind, type SortState } from "@/lib/sample-grid"
import { cn } from "@/lib/utils"
import type { AssetColumn } from "@/services/contracts/assets"
import { EMPTY_VALUE, FAMILY } from "./asset-columns"

/** One row of the sample: a string as stored, `null` for a `NULL`. */
export type SampleRow = Record<string, string | null>

/**
 * What the inspector shows: a cell (the row is its place in the sample as it
 * came back, so it follows the row when the order changes) or a whole column.
 */
export type Picked =
  | { kind: "cell"; at: number; column: string }
  | { kind: "column"; column: string }

export const NULL_LABEL = "NULL"

/** A value this long may be cut by the column's width, so it carries its whole as a title. */
const TITLE_ABOVE = 24

/** The gutter is `w-12` (3rem); the first data column sticks at this offset. */
const FIRST_COLUMN_LEFT = "@lg:left-12"

const quiet = "italic text-muted-foreground"

function Cell({
  value,
  at,
  col,
  numeric,
  first,
  active,
  picked,
  tinted,
  onFocus,
  onPick,
}: {
  value: string | null | undefined
  at: number
  col: number
  numeric: boolean
  first: boolean
  active: boolean
  picked: boolean
  tinted: boolean
  onFocus: () => void
  onPick: () => void
}) {
  const kind = cellKind(value)
  return (
    <td
      role="gridcell"
      aria-selected={picked}
      tabIndex={active ? 0 : -1}
      data-at={at}
      data-col={col}
      onFocus={onFocus}
      onClick={onPick}
      className={cn(
        "cursor-default border-b border-border px-2 py-1.5 align-middle outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-chart-3",
        picked
          ? "bg-[color-mix(in_oklab,var(--chart-3)_16%,var(--card))] ring-2 ring-inset ring-chart-3"
          : tinted
            ? "bg-[color-mix(in_oklab,var(--chart-3)_7%,var(--card))]"
            : "bg-card group-hover/row:bg-muted",
        first && cn("@lg:sticky @lg:z-10 @lg:border-r", FIRST_COLUMN_LEFT),
        numeric && "text-right"
      )}
    >
      <div
        title={
          kind === "null"
            ? "No value (NULL)"
            : kind === "empty"
              ? "Empty text"
              : (value ?? "").length > TITLE_ABOVE
                ? (value ?? "")
                : undefined
        }
        className={cn(
          "max-w-64 truncate font-mono text-xs",
          first && "@lg:max-w-48",
          numeric && "tabular-nums"
        )}
      >
        {kind === "null" ? (
          <span className={quiet}>{NULL_LABEL}</span>
        ) : kind === "empty" ? (
          <span className={quiet}>{EMPTY_VALUE}</span>
        ) : (
          value
        )}
      </div>
    </td>
  )
}

/** What pressing the sort control of a column will do, said in words. */
function sortLabel(name: string, sort: SortState): string {
  if (sort === null || sort.column !== name) return `Sort ${name} ascending`
  return sort.direction === "asc" ? `Sort ${name} descending` : `Stop sorting by ${name}`
}

function HeaderCell({
  name,
  col,
  column,
  first,
  tabStop,
  tinted,
  sort,
  onSort,
  onPickColumn,
  onFocus,
}: {
  name: string
  col: number
  /** The schema's entry for the column; `undefined` when the schema does not list it. */
  column: AssetColumn | undefined
  first: boolean
  /** The header's roving tab stop: this column's two controls are tabbable. */
  tabStop: boolean
  tinted: boolean
  sort: SortState
  onSort: () => void
  onPickColumn: () => void
  onFocus: () => void
}) {
  const family = column ? typeFamily(column.dataType) : undefined
  const meta = family ? FAMILY[family] : undefined
  const Glyph = meta?.icon
  const numeric = family === "number"
  const sorted = sort !== null && sort.column === name ? sort : null
  const SortIcon = sorted ? (sorted.direction === "asc" ? ArrowUp : ArrowDown) : ArrowUpDown
  const label = sortLabel(name, sort)
  return (
    <th
      scope="col"
      aria-sort={sorted ? (sorted.direction === "asc" ? "ascending" : "descending") : undefined}
      onFocus={onFocus}
      className={cn(
        "sticky top-0 h-11 border-b border-border px-2 text-left align-middle font-normal",
        tinted ? "bg-[color-mix(in_oklab,var(--chart-3)_12%,var(--muted))]" : "bg-muted",
        first ? cn("z-30 @lg:border-r", FIRST_COLUMN_LEFT) : "z-20"
      )}
    >
      <div className={cn("flex max-w-64 items-center gap-1.5", first && "@lg:max-w-48", numeric && "justify-end")}>
        {Glyph && meta ? (
          <span
            role="img"
            aria-label={meta.label}
            title={meta.label}
            className="grid size-5 shrink-0 place-items-center rounded-md bg-background text-muted-foreground"
          >
            <Glyph className="size-3" aria-hidden />
          </span>
        ) : null}
        <div className="min-w-0">
          <button
            type="button"
            data-head="name"
            data-col={col}
            tabIndex={tabStop ? 0 : -1}
            title={name}
            onClick={onPickColumn}
            className="block max-w-full truncate rounded-sm text-left font-mono text-xs font-medium outline-none focus-visible:ring-2 focus-visible:ring-chart-3"
          >
            {name}
          </button>
          {column ? (
            <div
              title={column.dataType}
              className="truncate font-mono text-[11px] leading-4 font-normal text-muted-foreground"
            >
              {column.dataType}
            </div>
          ) : null}
        </div>
        <button
          type="button"
          data-head="sort"
          data-col={col}
          tabIndex={tabStop ? 0 : -1}
          aria-label={label}
          title={`${label}. Only the rows shown are sorted, not the table.`}
          onClick={onSort}
          className={cn(
            "grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground outline-none hover:bg-background hover:text-foreground focus-visible:ring-2 focus-visible:ring-chart-3",
            sorted ? "text-foreground" : "opacity-60"
          )}
        >
          <SortIcon className="size-3.5" aria-hidden />
        </button>
      </div>
    </th>
  )
}

/**
 * The grid. `order` is the rows' places in `rows`, in the order to show
 * them, so a sort changes `order` and not the rows; a row keeps its `at`
 * (its place in `rows`) and its picked state when the sort moves it.
 *
 * Picking: a press on a cell picks it; a press on a column's name picks the
 * column. Enter and Space pick the cell that has focus. While something is
 * picked the picked cell follows focus, so the arrow keys walk the
 * inspector through the sample; with nothing picked, moving focus does not
 * open it, so passing through the grid with the keyboard changes nothing
 * on the page.
 */
export function SampleGrid({
  columns,
  rows,
  order,
  schema,
  sort,
  onSort,
  picked,
  onPick,
}: {
  columns: string[]
  rows: SampleRow[]
  order: number[]
  schema: ReadonlyMap<string, AssetColumn>
  sort: SortState
  onSort: (column: string) => void
  picked: Picked | null
  onPick: (next: Picked) => void
}) {
  const frameRef = React.useRef<HTMLDivElement>(null)
  // `null` until the reader moves: the first cell is the tab stop, and it stays the
  // top-left one when a sort changes which row that is.
  const [active, setActive] = React.useState<{ at: number; col: number } | null>(null)
  const [headCol, setHeadCol] = React.useState(0)
  const last = columns.length - 1
  const activeAt = active !== null && order.includes(active.at) ? active.at : order[0]
  const activeCol = Math.min(active?.col ?? 0, last)
  const pickedColumn = picked?.column ?? null
  const pickedCell = picked?.kind === "cell" ? picked : null

  const focusCell = (at: number, col: number) =>
    frameRef.current?.querySelector<HTMLElement>(`td[data-at="${at}"][data-col="${col}"]`)?.focus()

  const moveTo = (at: number, col: number) => {
    setActive({ at, col })
    if (picked !== null) onPick({ kind: "cell", at, column: columns[col] })
    focusCell(at, col)
  }

  const onBodyKeyDown = (e: React.KeyboardEvent<HTMLTableSectionElement>) => {
    if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return
    const cell = e.target instanceof Element ? e.target.closest<HTMLElement>("td[data-at]") : null
    if (!cell) return
    const at = Number(cell.dataset.at)
    const col = Number(cell.dataset.col)
    const pos = order.indexOf(at)
    let nextPos = pos
    let nextCol = col
    switch (e.key) {
      case "ArrowDown":
        nextPos = Math.min(pos + 1, order.length - 1)
        break
      case "ArrowUp":
        nextPos = Math.max(pos - 1, 0)
        break
      case "ArrowRight":
        nextCol = Math.min(col + 1, last)
        break
      case "ArrowLeft":
        nextCol = Math.max(col - 1, 0)
        break
      case "Home":
        nextCol = 0
        break
      case "End":
        nextCol = last
        break
      case "Enter":
      case " ":
        e.preventDefault()
        onPick({ kind: "cell", at, column: columns[col] })
        return
      default:
        return
    }
    // The arrow keys would otherwise scroll the frame under the focus.
    e.preventDefault()
    moveTo(order[nextPos], nextCol)
  }

  const onHeadKeyDown = (e: React.KeyboardEvent<HTMLTableSectionElement>) => {
    if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return
    const control = e.target instanceof Element ? e.target.closest<HTMLElement>("[data-head]") : null
    if (!control) return
    const col = Number(control.dataset.col)
    const targets: Record<string, number | undefined> = {
      ArrowRight: Math.min(col + 1, last),
      ArrowLeft: Math.max(col - 1, 0),
      Home: 0,
      End: last,
    }
    const next = targets[e.key]
    if (next === undefined) return
    e.preventDefault()
    setHeadCol(next)
    frameRef.current?.querySelector<HTMLElement>(`[data-head="${control.dataset.head}"][data-col="${next}"]`)?.focus()
  }

  return (
    // `isolate`: the sticky cells' z-indexes stay inside the frame, not over the page's own sticky tab bar.
    <div ref={frameRef} className="@container isolate min-w-0 overflow-hidden rounded-lg border border-border">
      <div className="max-h-[70vh] scroll-pt-11 scroll-pl-14 overflow-auto @lg:scroll-pl-60">
        <table role="grid" aria-label="Sample rows" className="w-max min-w-full border-separate border-spacing-0 text-xs">
          <thead onKeyDown={onHeadKeyDown}>
            <tr>
              <th
                scope="col"
                className="sticky top-0 left-0 z-30 h-11 w-12 min-w-12 max-w-12 border-r border-b border-border bg-muted px-2 text-right align-middle text-[11px] font-medium text-muted-foreground"
              >
                <span className="sr-only">Row number</span>
                <span aria-hidden>#</span>
              </th>
              {columns.map((name, c) => (
                <HeaderCell
                  key={name}
                  name={name}
                  col={c}
                  column={schema.get(name)}
                  first={c === 0}
                  tabStop={c === Math.min(headCol, last)}
                  tinted={pickedColumn === name}
                  sort={sort}
                  onSort={() => onSort(name)}
                  onPickColumn={() => onPick({ kind: "column", column: name })}
                  onFocus={() => setHeadCol(c)}
                />
              ))}
            </tr>
          </thead>
          <tbody onKeyDown={onBodyKeyDown} className="[&>tr:last-child>*]:border-b-0">
            {order.map((at, pos) => {
              const row = rows[at]
              return (
                <tr key={at} className="group/row">
                  <th
                    scope="row"
                    className="sticky left-0 z-10 w-12 min-w-12 max-w-12 border-r border-b border-border bg-card px-2 py-1.5 text-right align-middle text-[11px] font-normal text-muted-foreground tabular-nums group-hover/row:bg-muted"
                  >
                    {pos + 1}
                  </th>
                  {columns.map((name, c) => {
                    const column = schema.get(name)
                    return (
                      <Cell
                        key={name}
                        value={row[name]}
                        at={at}
                        col={c}
                        numeric={column !== undefined && typeFamily(column.dataType) === "number"}
                        first={c === 0}
                        active={at === activeAt && c === activeCol}
                        picked={pickedCell !== null && pickedCell.at === at && pickedCell.column === name}
                        tinted={pickedColumn === name}
                        onFocus={() => setActive({ at, col: c })}
                        onPick={() => onPick({ kind: "cell", at, column: name })}
                      />
                    )
                  })}
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    </div>
  )
}
