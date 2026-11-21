import assert from "node:assert/strict"
import test from "node:test"

import type { AlertRunResult } from "@/services/contracts/alerts"
import { formatRunResult } from "./run-result"

const base: AlertRunResult = { id: "al_1", name: "Watch", type: "alert", fired: false }

test("a rule that could not be evaluated reads Failed with its reason (BI-67)", () => {
  assert.equal(formatRunResult({ ...base, failed: "the metric's source is not available." }), "Failed: the metric's source is not available.")
})

test("a deliberate skip still reads skipped", () => {
  assert.equal(formatRunResult({ ...base, skipped: "not due" }), "skipped: not due")
})

test("an evaluated rule is unchanged", () => {
  assert.equal(formatRunResult({ ...base, value: 5 }), "ok (value 5, not breached)")
})
