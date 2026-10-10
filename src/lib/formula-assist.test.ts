import assert from "node:assert/strict"
import test from "node:test"
import { applySuggestion, callAtCaret, charIndex, splitAtProblem, suggest } from "./formula-assist"
import type { FormulaFunction } from "@/services/contracts/calc-fields"

const fn = (name: string, signature: string): FormulaFunction => ({
  name, category: "x", signature, help: "h.", example: "", minArgs: 0, maxArgs: null, aggregate: false,
})
const FUNCTIONS = [fn("Sum", "Sum(x)"), fn("SumIf", "SumIf(x, test)"), fn("Round", "Round(x, digits)")]
const NAMES = { columns: ["revenue", "cost", "Net Sales"], fields: [{ name: "profit", formula: "[revenue] - [cost]" }] }

test("a bare prefix offers the functions first, then the columns and fields that start with it", () => {
  const got = suggest("Su", 2, NAMES, FUNCTIONS)
  assert.deepEqual(got.items.map((i) => i.label), ["Sum", "SumIf"])
  assert.equal(got.items[0].insert, "Sum(")
  const p = suggest("pr", 2, NAMES, FUNCTIONS)
  assert.deepEqual(p.items.map((i) => [i.label, i.kind, i.detail]), [["profit", "field", "[revenue] - [cost]"]])
})

test("inside a bracket the columns and fields that contain the typed text are offered, closing the bracket", () => {
  const got = suggest("[ne", 3, NAMES, FUNCTIONS)
  assert.deepEqual(got.items.map((i) => i.insert), ["Net Sales]"])
  assert.deepEqual([got.from, got.to], [1, 3])
  const closed = suggest("[re]", 3, NAMES, FUNCTIONS)
  assert.equal(closed.items[0].insert, "revenue", "an existing ] is not doubled")
})

test("a column whose name is not a plain word is inserted in brackets", () => {
  const got = suggest("Ne", 2, NAMES, FUNCTIONS)
  assert.equal(got.items[0].insert, "[Net Sales]")
})

test("nothing is suggested inside text, after a digit, or with nothing typed", () => {
  assert.equal(suggest("'Su", 3, NAMES, FUNCTIONS).items.length, 0)
  assert.equal(suggest("2Su", 3, NAMES, FUNCTIONS).items.length, 0)
  assert.equal(suggest("1 + ", 4, NAMES, FUNCTIONS).items.length, 0)
})

test("applying a suggestion replaces the typed prefix and moves the caret after it", () => {
  const range = suggest("1 + Su", 6, NAMES, FUNCTIONS)
  const out = applySuggestion("1 + Su", range, range.items[0])
  assert.deepEqual(out, { text: "1 + Sum(", caret: 8 })
})

test("the help line follows the function the caret is in and the argument it is on", () => {
  const at = (t: string, c = t.length) => callAtCaret(t, c, FUNCTIONS)
  assert.equal(at("Round(")?.fn.name, "Round")
  assert.equal(at("Round([a], ")?.argIndex, 1)
  assert.equal(at("Round(Sum(x), ")?.fn.name, "Round", "inner call closed")
  assert.equal(at("Round(Sum(")?.fn.name, "Sum")
  assert.equal(at("Round('a,b', ")?.argIndex, 1, "a comma in text is not an argument")
  assert.equal(at("1 + 2"), null)
  assert.equal(at("(1 + ")?.fn, undefined)
})

test("the text splits around the server's span by characters, not UTF-16 units", () => {
  assert.deepEqual(splitAtProblem("ab + cd", { position: 5, length: 2 }), { before: "ab + ", hit: "cd", after: "" })
  assert.deepEqual(splitAtProblem("\u{1F600} + x", { position: 4, length: 1 }), { before: "\u{1F600} + ", hit: "x", after: "" })
  assert.deepEqual(splitAtProblem("ab", { position: 9, length: 3 }), { before: "ab", hit: "", after: "" })
  assert.equal(charIndex("\u{1F600}ab", 2), 1)
})
