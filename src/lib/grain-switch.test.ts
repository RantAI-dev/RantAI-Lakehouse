import assert from "node:assert/strict"
import test from "node:test"

import {
  differsFromSaved,
  grainQuery,
  readGrainParam,
  savedGrainBody,
  shownGrain,
  withGrainParam,
} from "./grain-switch"

test("the address carries a truncation, own, or nothing, and anything else is nothing", () => {
  assert.equal(readGrainParam("month"), "month")
  assert.equal(readGrainParam("own"), "own")
  assert.equal(readGrainParam(null), null)
  for (const bad of ["day_of_week", "fortnight", "", "Month", "month;--"]) {
    assert.equal(readGrainParam(bad), null, bad)
  }
})

test("the control shows the address's choice, else the saved grain, else each chart's own", () => {
  assert.equal(shownGrain("quarter", "year"), "quarter")
  assert.equal(shownGrain(null, "year"), "year")
  assert.equal(shownGrain(null, undefined), "")
  assert.equal(shownGrain(null, ""), "")
  assert.equal(shownGrain("own", "year"), "", "own sets the saved default aside")
  assert.equal(shownGrain(null, "day_of_week"), "", "a saved part is not a switch")
})

test("a choice survives a round trip through the address beside the filters", () => {
  const search = "f=abc&x=1"
  const written = withGrainParam(search, "month")
  assert.equal(new URLSearchParams(written).get("grain"), "month")
  assert.equal(new URLSearchParams(written).get("f"), "abc")
  assert.equal(readGrainParam(new URLSearchParams(written).get("grain")), "month")
  const own = withGrainParam(written, "own")
  assert.equal(new URLSearchParams(own).get("grain"), "own")
  const cleared = withGrainParam(own, null)
  assert.equal(new URLSearchParams(cleared).get("grain"), null)
  assert.equal(new URLSearchParams(cleared).get("f"), "abc")
})

test("the API is asked only when the address says something", () => {
  assert.equal(grainQuery(null), null)
  assert.equal(grainQuery("week"), "week")
  assert.equal(grainQuery("own"), "own")
})

test("Save as default is offered only when the shown grain differs from the saved one", () => {
  assert.equal(differsFromSaved("month", "month"), false)
  assert.equal(differsFromSaved("month", undefined), true)
  assert.equal(differsFromSaved("", "month"), true)
  assert.equal(differsFromSaved("", ""), false)
  assert.equal(savedGrainBody(""), "", "an empty body value clears the saved grain")
  assert.equal(savedGrainBody("year"), "year")
})
