/**
 * The parameter dialog's draft (`BI-11`): what is typed, and the definition
 * it becomes. The server checks everything again; this only keeps the form
 * from sending what it can see is incomplete, and turns the lines of a list
 * into entries.
 */
import type { ParamInput } from "@/services/contracts/params"
import type { ParamDef, ParamListEntry, ParamType } from "./params"

export type ParamDraft = {
  name: string
  label: string
  type: ParamType
  default: string
  min: string
  max: string
  maxLength: string
  /** A list's choices, one per line: `value` or `value | label`. */
  values: string
}

export const emptyParamDraft = (): ParamDraft => ({
  name: "", label: "", type: "number", default: "", min: "", max: "", maxLength: "", values: "",
})

export const PARAM_TYPE_LABELS: Record<ParamType, string> = {
  number: "Number", text: "Text", date: "Date", list: "Choice from a list",
}

const NAME = /^[A-Za-z_][A-Za-z0-9_]{0,63}$/

export function draftFromDef(def: ParamDef): ParamDraft {
  return {
    name: def.name,
    label: def.label === def.name ? "" : def.label,
    type: (def.type as ParamType),
    default: String(def.default ?? ""),
    min: def.min === undefined ? "" : String(def.min),
    max: def.max === undefined ? "" : String(def.max),
    maxLength: def.maxLength === undefined ? "" : String(def.maxLength),
    values: (def.values ?? []).map((v) => (v.label && v.label !== String(v.value) ? `${v.value} | ${v.label}` : String(v.value))).join("\n"),
  }
}

/** The lines of a list as entries; numbers when every value is one. */
export function entriesFromLines(text: string): ParamListEntry[] {
  const rows = text.split("\n").map((l) => l.trim()).filter(Boolean).map((l) => {
    const at = l.indexOf("|")
    const value = (at < 0 ? l : l.slice(0, at)).trim()
    const label = at < 0 ? "" : l.slice(at + 1).trim()
    return { value, label }
  })
  const numeric = rows.length > 0 && rows.every((r) => r.value !== "" && Number.isFinite(Number(r.value)))
  return rows.map((r) => ({ value: numeric ? Number(r.value) : r.value, label: r.label || r.value }))
}

/** What is still needed before the draft can be saved. */
export function paramProblems(d: ParamDraft, editing: boolean): string[] {
  const out: string[] = []
  if (!editing && !NAME.test(d.name.trim())) out.push("a name of letters, digits and _")
  if (d.type === "number") {
    if (d.default.trim() === "" || !Number.isFinite(Number(d.default))) out.push("a default number")
    for (const [word, v] of [["minimum", d.min], ["maximum", d.max]] as const) if (v.trim() !== "" && !Number.isFinite(Number(v))) out.push(`a ${word} that is a number`)
  } else if (d.type === "date") {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(d.default.trim())) out.push("a default date (YYYY-MM-DD)")
  } else if (d.type === "list") {
    const entries = entriesFromLines(d.values)
    if (!entries.length) out.push("at least one choice")
    else if (!entries.some((e) => String(e.value) === d.default.trim())) out.push("a default that is one of the choices")
  }
  return out
}

/** The definition to send. Empty optional fields are left out. */
export function inputFromDraft(d: ParamDraft): ParamInput {
  const out: ParamInput = {
    name: d.name.trim(),
    type: d.type,
    default: d.type === "number" ? Number(d.default) : d.type === "list" ? entriesFromLines(d.values).find((e) => String(e.value) === d.default.trim())?.value ?? d.default : d.default,
  }
  if (d.label.trim()) out.label = d.label.trim()
  if (d.type === "number") {
    if (d.min.trim() !== "") out.min = Number(d.min)
    if (d.max.trim() !== "") out.max = Number(d.max)
  } else if (d.type === "date") {
    if (d.min.trim() !== "") out.min = d.min.trim()
    if (d.max.trim() !== "") out.max = d.max.trim()
  } else if (d.type === "text") {
    if (d.maxLength.trim() !== "") out.maxLength = Number(d.maxLength)
  } else {
    out.values = entriesFromLines(d.values)
  }
  return out
}
