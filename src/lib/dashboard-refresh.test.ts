import { strict as assert } from "node:assert"
import { test } from "node:test"
import { REFRESH_OPTIONS, canSaveRefresh, effectiveRefresh, listedInterval, refreshLabel } from "./dashboard-refresh"

test("the intervals are the server's list, off first", () => {
  assert.deepEqual(REFRESH_OPTIONS.map((o) => o.seconds), [0, 60, 300, 600, 900, 1800, 3600])
})

test("a viewer's choice for the session wins over the saved default, and 0 is a choice", () => {
  assert.equal(effectiveRefresh(300, null), 300)
  assert.equal(effectiveRefresh(300, 60), 60)
  assert.equal(effectiveRefresh(300, 0), 0)
  assert.equal(effectiveRefresh(undefined, null), 0)
})

test("a value that is not a listed interval is off, never a timer of an odd length", () => {
  for (const bad of [30, 59, -1, 1.5, Number.NaN, "300", null, undefined]) {
    assert.equal(listedInterval(bad), 0, String(bad))
  }
  assert.equal(effectiveRefresh(45, null), 0)
  assert.equal(effectiveRefresh(300, 45), 300)
})

test("only someone who may write, with a choice that differs from the saved one, can save it", () => {
  assert.equal(canSaveRefresh({ mayWrite: true, saved: 300, effective: 60 }), true)
  assert.equal(canSaveRefresh({ mayWrite: true, saved: 300, effective: 300 }), false)
  assert.equal(canSaveRefresh({ mayWrite: false, saved: 300, effective: 60 }), false)
  assert.equal(canSaveRefresh({ mayWrite: true, saved: undefined, effective: 0 }), false)
  assert.equal(canSaveRefresh({ mayWrite: true, saved: undefined, effective: 60 }), true)
})

test("labels read as the menu does, and an unknown number as manual", () => {
  assert.equal(refreshLabel(60), "Every 1m")
  assert.equal(refreshLabel(3600), "Every 60m")
  assert.equal(refreshLabel(7), "Manual")
})
