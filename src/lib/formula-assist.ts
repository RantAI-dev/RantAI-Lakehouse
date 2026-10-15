import type { FormulaFunction, FormulaProblem } from "@/services/contracts/calc-fields"

/**
 * Pure helpers behind the formula box (BI-8): what to suggest at the caret,
 * how to insert it, which function the caret is inside (for the help line)
 * and how to split the text around a mistake the server positioned. The
 * server stays the only judge of a formula; nothing here parses it.
 */

export type Suggestion = {
  kind: "column" | "field" | "function"
  /** What the list shows. */
  label: string
  /** What replaces the typed prefix. */
  insert: string
  /** The signature of a function, or the formula of a field. */
  detail?: string
}

export type Names = {
  columns: string[]
  /** Calculated fields of the source, with their formulas for the detail line. */
  fields: { name: string; formula: string }[]
}

type State = "code" | "bracket" | "string"

/** The lexical state at `caret` and where the open bracket or quote began. */
function stateAt(text: string, caret: number): { state: State; start: number } {
  let state: State = "code"
  let quote = ""
  let start = 0
  for (let i = 0; i < caret && i < text.length; i++) {
    const c = text[i]
    if (state === "string") {
      if (c === quote) state = "code"
    } else if (state === "bracket") {
      if (c === "]") state = "code"
    } else if (c === "'" || c === '"') {
      state = "string"
      quote = c
      start = i
    } else if (c === "[") {
      state = "bracket"
      start = i
    }
  }
  return { state, start }
}

const isIdent = (s: string) => /^[A-Za-z_][A-Za-z0-9_]*$/.test(s)
const MAX_ITEMS = 8

function rank<T>(items: T[], name: (t: T) => string, prefix: string): T[] {
  const p = prefix.toLowerCase()
  const starts = items.filter((t) => name(t).toLowerCase().startsWith(p))
  const has = items.filter((t) => !name(t).toLowerCase().startsWith(p) && name(t).toLowerCase().includes(p))
  return [...starts, ...has]
}

/**
 * Suggestions for the word at `caret`: inside `[` the columns and fields,
 * after a letter the functions and the columns that fit; none inside text.
 * `from`..`to` is the span they replace.
 */
export function suggest(
  text: string,
  caret: number,
  names: Names,
  functions: FormulaFunction[]
): { from: number; to: number; items: Suggestion[] } {
  const { state, start } = stateAt(text, caret)
  const none = { from: caret, to: caret, items: [] as Suggestion[] }
  if (state === "string") return none
  const all = [
    ...names.columns.map((c) => ({ name: c, field: false, formula: "" })),
    ...names.fields.map((f) => ({ name: f.name, field: true, formula: f.formula })),
  ]
  if (state === "bracket") {
    const prefix = text.slice(start + 1, caret)
    const closes = text[caret] === "]"
    const items = rank(all, (a) => a.name, prefix)
      .slice(0, MAX_ITEMS)
      .map<Suggestion>((a) => ({
        kind: a.field ? "field" : "column",
        label: a.name,
        insert: closes ? a.name : `${a.name}]`,
        detail: a.field ? a.formula : undefined,
      }))
    return { from: start + 1, to: closes ? caret : caret, items }
  }
  let from = caret
  while (from > 0 && /[A-Za-z0-9_]/.test(text[from - 1])) from--
  const prefix = text.slice(from, caret)
  if (prefix === "" || /^[0-9]/.test(prefix)) return none
  const fns = rank(functions, (f) => f.name, prefix)
    .filter((f) => f.name.toLowerCase().startsWith(prefix.toLowerCase()))
    .map<Suggestion>((f) => ({ kind: "function", label: f.name, insert: `${f.name}(`, detail: f.signature }))
  const cols = rank(all, (a) => a.name, prefix)
    .filter((a) => a.name.toLowerCase().startsWith(prefix.toLowerCase()))
    .map<Suggestion>((a) => ({
      kind: a.field ? "field" : "column",
      label: a.name,
      insert: isIdent(a.name) ? a.name : `[${a.name}]`,
      detail: a.field ? a.formula : undefined,
    }))
  return { from, to: caret, items: [...fns, ...cols].slice(0, MAX_ITEMS) }
}

/** `text` with `from`..`to` replaced by the suggestion, and where the caret goes. */
export function applySuggestion(
  text: string,
  range: { from: number; to: number },
  s: Suggestion
): { text: string; caret: number } {
  const next = text.slice(0, range.from) + s.insert + text.slice(range.to)
  return { text: next, caret: range.from + s.insert.length }
}

/** The function whose parentheses hold the caret, and which argument the caret is in. */
export function callAtCaret(
  text: string,
  caret: number,
  functions: FormulaFunction[]
): { fn: FormulaFunction; argIndex: number } | null {
  const stack: { name: string; arg: number }[] = []
  let state: State = "code"
  let quote = ""
  for (let i = 0; i < caret && i < text.length; i++) {
    const c = text[i]
    if (state === "string") {
      if (c === quote) state = "code"
    } else if (state === "bracket") {
      if (c === "]") state = "code"
    } else if (c === "'" || c === '"') {
      state = "string"
      quote = c
    } else if (c === "[") state = "bracket"
    else if (c === "(") {
      const m = /([A-Za-z_][A-Za-z0-9_]*)\s*$/.exec(text.slice(0, i))
      stack.push({ name: m ? m[1] : "", arg: 0 })
    } else if (c === ")") stack.pop()
    else if (c === "," && stack.length) stack[stack.length - 1].arg++
  }
  for (let i = stack.length - 1; i >= 0; i--) {
    const fn = functions.find((f) => f.name.toLowerCase() === stack[i].name.toLowerCase())
    if (fn) return { fn, argIndex: stack[i].arg }
  }
  return null
}

/** The text before, inside and after the span a server problem names (positions count characters). */
export function splitAtProblem(
  text: string,
  problem: Pick<FormulaProblem, "position" | "length">
): { before: string; hit: string; after: string } {
  const chars = Array.from(text)
  const start = Math.min(Math.max(problem.position, 0), chars.length)
  const end = Math.min(start + Math.max(problem.length, 1), chars.length)
  return {
    before: chars.slice(0, start).join(""),
    hit: chars.slice(start, end).join(""),
    after: chars.slice(end).join(""),
  }
}

/** UTF-16 caret index to a character count, to compare with a server position. */
export function charIndex(text: string, utf16Index: number): number {
  return Array.from(text.slice(0, utf16Index)).length
}

/** What a checked formula is, in the words the box shows under it. */
export function levelLabel(level: string, type: string): string {
  const what = level === "aggregate" ? "Aggregate" : level === "table" ? "Table calculation (over the chart's result)" : "One value per row"
  return `${what}, ${type}.`
}
