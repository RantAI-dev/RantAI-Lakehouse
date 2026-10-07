import type { ToolStep } from "./tool-step"

/** What the model asked the user through the `ask_user` tool. */
export type Ask = { term: string; question: string; options: string[] }

const MIN_OPTIONS = 2
const MAX_OPTIONS = 4

function nonEmpty(v: unknown): v is string {
  return typeof v === "string" && v.trim() !== ""
}

/**
 * The question and options of a successful `ask_user` step, or `null` for
 * any other step. The result comes from the model's tool call, so its shape
 * is checked here instead of trusted: a malformed ask renders as nothing
 * rather than as broken buttons.
 */
export function askFromStep(step: ToolStep): Ask | null {
  if (step.tool !== "ask_user" || !step.ok) return null
  const r = step.result
  if (!r || typeof r !== "object") return null
  const { asked, term, question, options } = r as Record<string, unknown>
  if (asked !== true || !nonEmpty(term) || !nonEmpty(question)) return null
  if (!Array.isArray(options) || options.length < MIN_OPTIONS || options.length > MAX_OPTIONS) return null
  if (!options.every(nonEmpty)) return null
  return { term, question, options }
}
