import { describe, expect, it } from "bun:test"

import { askFromStep } from "./ask-options"
import type { ToolStep } from "./tool-step"

function askStep(result: unknown, overrides: Partial<ToolStep> = {}): ToolStep {
  return { tool: "ask_user", args: {}, ok: true, result, ...overrides }
}

const VALID = {
  asked: true,
  term: "active customer",
  question: "What counts as an active customer?",
  options: ["Ordered in the last 30 days", "Ordered in the last 90 days"],
}

describe("askFromStep", () => {
  it("returns the term, the question and the options of a successful ask_user step", () => {
    expect(askFromStep(askStep(VALID))).toEqual({
      term: "active customer",
      question: "What counts as an active customer?",
      options: ["Ordered in the last 30 days", "Ordered in the last 90 days"],
    })
  })

  it("returns null for a step of another tool", () => {
    expect(askFromStep(askStep(VALID, { tool: "run_sql" }))).toBeNull()
  })

  it("returns null when the step did not succeed", () => {
    expect(askFromStep(askStep(VALID, { ok: false }))).toBeNull()
    expect(askFromStep(askStep({ error: "ask-back is off" }))).toBeNull()
  })

  it("returns null when the result does not say it asked", () => {
    expect(askFromStep(askStep({ term: VALID.term, question: VALID.question, options: VALID.options }))).toBeNull()
    expect(askFromStep(askStep({ ...VALID, asked: "yes" }))).toBeNull()
  })

  it("returns null with fewer than two options", () => {
    expect(askFromStep(askStep({ ...VALID, options: ["Only one"] }))).toBeNull()
  })

  it("returns null with more than four options", () => {
    expect(askFromStep(askStep({ ...VALID, options: ["a", "b", "c", "d", "e"] }))).toBeNull()
  })

  it("returns null when an option is not a non-empty string", () => {
    expect(askFromStep(askStep({ ...VALID, options: ["a", 2] }))).toBeNull()
    expect(askFromStep(askStep({ ...VALID, options: ["a", "  "] }))).toBeNull()
  })

  it("returns null when the term or the question is missing or empty", () => {
    expect(askFromStep(askStep({ ...VALID, term: "" }))).toBeNull()
    expect(askFromStep(askStep({ ...VALID, question: 3 }))).toBeNull()
  })
})
