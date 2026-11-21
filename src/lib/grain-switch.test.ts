import assert from "node:assert/strict"
import test from "node:test"

import {
  differsFromSaved,
  grainNotAppliedSentence,
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

// QA LH-BI-9-05 (BI-50): the four reasons the server sends, one general
// sentence for a reason this console does not know yet, and nothing without
// a reason.
test("each reason the server gives has its sentence, an unknown reason the general one, and no reason none", () => {
  assert.equal(grainNotAppliedSentence("kind"), "Group dates by does not apply to this type of chart.")
  assert.equal(
    grainNotAppliedSentence("part"),
    "Group dates by does not apply: this chart groups by a part of the date, such as the day of the week.",
  )
  assert.equal(grainNotAppliedSentence("not_a_date"), "Group dates by does not apply: this chart's dimension is not a date.")
  assert.equal(
    grainNotAppliedSentence("no_grain"),
    "Group dates by does not apply: this chart has no date grouping of its own to change.",
  )
  assert.equal(grainNotAppliedSentence("something_new"), "Group dates by does not apply to this chart.")
  assert.equal(grainNotAppliedSentence(undefined), null)
  assert.equal(grainNotAppliedSentence(""), null)
})
