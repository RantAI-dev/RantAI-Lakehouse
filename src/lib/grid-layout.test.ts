import { strict as assert } from "node:assert"
import { test } from "node:test"
import { settleLayout } from "./grid-layout"

test("settleLayout mendorong tile yang tertimpa ke bawah tile yang digeser", () => {
  const out = settleLayout(
    {
      a: { x: 0, y: 0, w: 6, h: 6 },
      // `b` baru saja ditarik ke atas `a`.
      b: { x: 3, y: 2, w: 6, h: 6 },
    },
    "b"
  )
  assert.deepEqual(out.b, { x: 3, y: 2, w: 6, h: 6 })
  assert.deepEqual(out.a, { x: 0, y: 8, w: 6, h: 6 })
})

test("settleLayout merambatkan dorongan ke tile di bawahnya", () => {
  const out = settleLayout(
    {
      a: { x: 0, y: 0, w: 6, h: 4 },
      b: { x: 0, y: 4, w: 6, h: 4 },
      big: { x: 0, y: 0, w: 12, h: 3 },
    },
    "big"
  )
  assert.deepEqual(out.a, { x: 0, y: 3, w: 6, h: 4 })
  assert.deepEqual(out.b, { x: 0, y: 7, w: 6, h: 4 })
})

test("settleLayout tidak menarik tile ke atas atau mengubah layout yang sudah rapi", () => {
  const tidy = {
    a: { x: 0, y: 0, w: 6, h: 4 },
    b: { x: 6, y: 10, w: 6, h: 4 },
  }
  assert.deepEqual(settleLayout(tidy), tidy)
})

test("settleLayout tanpa anchor merapikan layout tersimpan yang bertumpuk", () => {
  const out = settleLayout({
    a: { x: 0, y: 0, w: 6, h: 6 },
    b: { x: 0, y: 3, w: 6, h: 6 },
  })
  assert.deepEqual(out.a, { x: 0, y: 0, w: 6, h: 6 })
  assert.deepEqual(out.b, { x: 0, y: 6, w: 6, h: 6 })
})
