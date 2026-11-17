import type { PivotTotals } from "./table-types"

/**
 * Lay out a pivot table (`BI-16` part A) from the long-format rows the server
 * returns (`lakehouse_bi::tables::pivot_sql`): the key fields, one `__v<i>`
 * per value, and one `__g<j>` flag per key field (rows first), `1` when the
 * field is rolled up in that row, so the row is a total. A flag, never an
 * empty value, marks a total: a real group can be empty or null.
 *
 * Every number is the database's: this module places cells and never adds
 * them up, so an average of averages cannot happen here.
 */

export type PivotInput = {
  rows: readonly Record<string, unknown>[]
  rowFields: readonly string[]
  colFields: readonly string[]
  valueCount: number
  /** A display label for a key of one field, or `null` for the default (BI-9 bucket labels). */
  labelFor?: (field: string, key: string) => string | null
}

export type PivotColumn = {
  /** The column keys this header stands for; empty for the grand total or for a pivot without column fields. */
  path: string[]
  labels: string[]
  kind: "leaf" | "total"
}

export type PivotRow = {
  path: string[]
  labels: string[]
  /** `leaf`: a group of every row field. `subtotal`: of the outer fields only. `grand`: of everything. */
  kind: "leaf" | "subtotal" | "grand"
  /** `cells[column][value]`; `null` where the database returned no cell. */
  cells: (number | null)[][]
}

export type PivotModel = {
  rowFields: string[]
  colFields: string[]
  valueCount: number
  columns: PivotColumn[]
  rows: PivotRow[]
}

const NULL_KEY = "\u0000null"
const EMPTY_LABEL = "(empty)"

function keyOf(value: unknown): string {
  return value === null || value === undefined ? NULL_KEY : String(value)
}

function labelOf(key: string): string {
  return key === NULL_KEY || key === "" ? EMPTY_LABEL : key
}

function compareKeys(a: string, b: string): number {
  if (a === b) return 0
  if (a === NULL_KEY) return 1
  if (b === NULL_KEY) return -1
  const na = Number(a)
  const nb = Number(b)
  if (a.trim() !== "" && b.trim() !== "" && Number.isFinite(na) && Number.isFinite(nb)) return na - nb
  return a.localeCompare(b, undefined, { numeric: true })
}

function num(v: unknown): number | null {
  const n = typeof v === "number" ? v : typeof v === "string" && v.trim() !== "" ? Number(v) : Number.NaN
  return Number.isFinite(n) ? n : null
}

type Trie = { key: string; children: Map<string, Trie>; present: boolean }

function trieOf(paths: readonly string[][]): Trie {
  const root: Trie = { key: "", children: new Map(), present: false }
  for (const path of paths) {
    let node = root
    for (const key of path) {
      let next = node.children.get(key)
      if (!next) {
        next = { key, children: new Map(), present: false }
        node.children.set(key, next)
      }
      node = next
    }
    node.present = true
  }
  return root
}

/** Paths in display order: children first, then the group's own total, the grand total last. */
function ordered(root: Trie): string[][] {
  const out: string[][] = []
  const walk = (node: Trie, path: string[]) => {
    for (const child of [...node.children.values()].sort((a, b) => compareKeys(a.key, b.key))) {
      walk(child, [...path, child.key])
    }
    if (path.length > 0 && node.present) out.push(path)
  }
  walk(root, [])
  if (root.present) out.push([])
  return out
}

const idOf = (path: readonly string[]) => JSON.stringify(path)

export function buildPivot(input: PivotInput): PivotModel {
  const nR = input.rowFields.length
  const nC = input.colFields.length
  const keyFields = [...input.rowFields, ...input.colFields]
  const cellsByKey = new Map<string, (number | null)[]>()
  const rowPaths: string[][] = []
  const colPaths: string[][] = []
  for (const row of input.rows) {
    const rolled = keyFields.map((_, j) => Number(row[`__g${j}`]) === 1)
    // Totals are prefixes: a field is rolled up only if every later field of its side is. Anything else is not a shape the server makes.
    const rowDepth = rolled.slice(0, nR).filter((r) => !r).length
    const colDepth = rolled.slice(nR).filter((r) => !r).length
    if (rolled.slice(0, nR).some((r, i) => r && i < rowDepth) || rolled.slice(nR).some((r, i) => r && i < colDepth)) continue
    const rowPath = input.rowFields.slice(0, rowDepth).map((f) => keyOf(row[f]))
    const colPath = input.colFields.slice(0, colDepth).map((f) => keyOf(row[f]))
    rowPaths.push(rowPath)
    colPaths.push(colPath)
    cellsByKey.set(`${idOf(rowPath)}|${idOf(colPath)}`, Array.from({ length: input.valueCount }, (_, i) => num(row[`__v${i}`])))
  }

  const fieldLabel = (field: string, key: string) => input.labelFor?.(field, key) ?? labelOf(key)
  const colOrder = nC === 0 ? [[]] : ordered(trieOf(colPaths))
  const columns: PivotColumn[] = colOrder.map((path) => ({
    path,
    labels: path.map((k, i) => fieldLabel(input.colFields[i], k)),
    kind: path.length === nC ? "leaf" : "total",
  }))
  const rows: PivotRow[] = ordered(trieOf(rowPaths)).map((path) => ({
    path,
    labels: path.map((k, i) => fieldLabel(input.rowFields[i], k)),
    kind: path.length === nR ? "leaf" : path.length === 0 ? "grand" : "subtotal",
    cells: colOrder.map((colPath) => cellsByKey.get(`${idOf(path)}|${idOf(colPath)}`) ?? Array.from({ length: input.valueCount }, () => null)),
  }))
  return { rowFields: [...input.rowFields], colFields: [...input.colFields], valueCount: input.valueCount, columns, rows }
}

/** The text of a column header: `2026-01 · Retail`, `Total`, `Retail total`. */
export function columnLabel(column: PivotColumn): string {
  if (column.kind === "leaf") return column.labels.join(" · ") || ""
  return column.labels.length === 0 ? "Total" : `${column.labels.join(" · ")} total`
}

/** The text of a row's total: `Total`, `Jawa Barat total`. */
export function rowLabel(row: PivotRow): string {
  if (row.kind === "grand") return "Total"
  return row.kind === "subtotal" ? `${row.labels.join(" · ")} total` : row.labels.join(" · ")
}

/** Ids of the groups that can fold: those that have a subtotal row to stay in view. */
export function collapsibleGroups(model: PivotModel): Set<string> {
  return new Set(model.rows.filter((r) => r.kind === "subtotal").map((r) => idOf(r.path)))
}

export function groupId(path: readonly string[]): string {
  return idOf(path)
}

/** The rows to draw when the groups in `collapsed` are folded: a folded group keeps its subtotal row and hides what is inside it. */
export function visibleRows(model: PivotModel, collapsed: ReadonlySet<string>): PivotRow[] {
  if (collapsed.size === 0) return model.rows
  return model.rows.filter((row) => {
    for (let k = 1; k < row.path.length; k += 1) {
      if (collapsed.has(idOf(row.path.slice(0, k)))) return false
    }
    return true
  })
}

/** Whether the totals asked for can fold groups at all (only `all` has subtotals). */
export function canCollapse(totals: PivotTotals | undefined): boolean {
  return totals === "all"
}
