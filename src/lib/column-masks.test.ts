import { strict as assert } from "node:assert"
import { test } from "node:test"
import { columnMasks, sharedMaskSentence } from "./column-masks"

const policy = (over: Record<string, unknown>) => ({
  id: "p1",
  name: "mask_email",
  status: "ready",
  roles: ["Analyst"],
  mask: ["email"],
  rowFilter: null,
  ...over,
})

test("columnMasks marks a policy that masks only this column as removable here", () => {
  const masks = columnMasks([policy({})], "email")
  assert.equal(masks.length, 1)
  assert.equal(masks[0].kind, "own")
})

test("columnMasks marks a policy that also masks other columns, or filters rows, as shared", () => {
  const [wide] = columnMasks([policy({ mask: ["email", "phone", "address"] })], "email")
  assert.equal(wide.kind, "shared")
  assert.ok(wide.kind === "shared")
  assert.deepEqual(wide.otherColumns, ["phone", "address"])
  assert.equal(wide.filtersRows, false)

  const [filtered] = columnMasks([policy({ rowFilter: "region = 'ID'" })], "email")
  assert.equal(filtered.kind, "shared")
})

test("columnMasks leaves out drafts and policies that do not mask this column", () => {
  assert.deepEqual(columnMasks([policy({ status: "draft" }), policy({ mask: ["phone"] }), policy({ mask: undefined })], "email"), [])
})

test("sharedMaskSentence says what else the policy does", () => {
  const [a] = columnMasks([policy({ name: "pii", mask: ["email", "phone", "address"] })], "email")
  assert.ok(a.kind === "shared")
  assert.equal(sharedMaskSentence(a), "Part of pii, which also masks phone and address.")
  const [b] = columnMasks([policy({ name: "pii", mask: ["email", "phone"], rowFilter: "x = 1" })], "email")
  assert.ok(b.kind === "shared")
  assert.equal(sharedMaskSentence(b), "Part of pii, which also masks phone and filters rows.")
  const [c] = columnMasks([policy({ name: "pii", rowFilter: "x = 1" })], "email")
  assert.ok(c.kind === "shared")
  assert.equal(sharedMaskSentence(c), "Part of pii, which also filters rows.")
})
