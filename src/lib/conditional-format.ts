import { asNumber } from "./cell-format"
import { interpolate, luminance } from "./chart-palettes"
import type { CondRule } from "./chart-style"

/**
 * Colour a table cell or a KPI value by its value (`BI-17` part A, feature
 * item 6). The first rule of the column that holds wins. A rule colours the
 * cell's background (the text turns black or white, whichever reads better) or
 * the text itself. Colour is never the only cue where it carries meaning:
 * the number is still written.
 */
export type CellStyle = { background?: string; color?: string }

/** The shown minimum and maximum of each column a `range` rule reads. */
export type Domains = Record<string, { min: number; max: number }>

/** The text colour that reads on `background`: black on a light one, white on a dark one. */
export function readableOn(background: string): string {
  return luminance(background) > 0.179 ? "#000000" : "#ffffff"
}

/** The numbers each `range` rule's column holds in `rows`, as `{min, max}`. */
export function domainsOf(rules: readonly CondRule[] | undefined, rows: readonly Record<string, unknown>[]): Domains {
  const out: Domains = {}
  for (const rule of rules ?? []) {
    if (rule.rule.type !== "range" || out[rule.column]) continue
    let min = Infinity
    let max = -Infinity
    for (const row of rows) {
      const n = asNumber(row[rule.column])
      if (n === null) continue
      min = Math.min(min, n)
      max = Math.max(max, n)
    }
    if (min <= max) out[rule.column] = { min, max }
  }
  return out
}

function matches(rule: CondRule, value: unknown): boolean {
  const test = rule.rule
  switch (test.type) {
    case "compare": {
      const n = asNumber(value)
      if (n === null) return false
      switch (test.op) {
        case "lt": return n < test.value
        case "le": return n <= test.value
        case "gt": return n > test.value
        case "ge": return n >= test.value
        case "eq": return n === test.value
        case "ne": return n !== test.value
      }
      return false
    }
    case "between": {
      const n = asNumber(value)
      return n !== null && n >= test.from && n <= test.to
    }
    case "contains":
      return String(value ?? "").toLowerCase().includes(test.text.toLowerCase())
    case "range":
      return false
  }
}

function paint(rule: CondRule, colour: string): CellStyle {
  return (rule.target ?? "cell") === "text"
    ? { color: colour }
    : { background: colour, color: readableOn(colour) }
}

/**
 * The style of one value. `column` is the cell's column; a KPI passes `null`
 * because it has one value and a rule of the KPI names that value whatever it
 * is called. `domains` is needed only by `range` rules.
 */
export function conditionalStyle(
  rules: readonly CondRule[] | undefined,
  column: string | null,
  value: unknown,
  domains: Domains = {},
): CellStyle | undefined {
  for (const rule of rules ?? []) {
    if (column !== null && rule.column !== column) continue
    if (rule.rule.type === "range") {
      const n = asNumber(value)
      const d = domains[rule.column]
      if (n === null || !d) continue
      const t = d.max === d.min ? 0.5 : (n - d.min) / (d.max - d.min)
      return paint(rule, interpolate(rule.rule.colors, t))
    }
    if (rule.color && matches(rule, value)) return paint(rule, rule.color)
  }
  return undefined
}
