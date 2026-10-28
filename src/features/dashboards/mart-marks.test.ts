import assert from "node:assert/strict"
import { test } from "bun:test"
import { selectedMart, type Mart } from "./mart-marks"

const MARTS: Mart[] = [
  { name: "orders", rows: 10, certification: "deprecated", certificationNote: "Superseded" },
  { name: "stock", rows: 3 },
]

test("selectedMart finds the mart a mart choice names", () => {
  assert.equal(selectedMart(MARTS, { kind: "mart", name: "orders" })?.certification, "deprecated")
  assert.equal(selectedMart(MARTS, { kind: "mart", name: "stock" })?.certification, undefined)
})

test("selectedMart is undefined for a SQL source, no choice, or a mart not in the list", () => {
  assert.equal(selectedMart(MARTS, { kind: "sql", id: "s1" }), undefined)
  assert.equal(selectedMart(MARTS, null), undefined)
  assert.equal(selectedMart(MARTS, { kind: "mart", name: "gone" }), undefined)
})
