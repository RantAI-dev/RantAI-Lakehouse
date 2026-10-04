import { strict as assert } from "node:assert"
import { test } from "node:test"
import { clampZoom, readView, stepZoom, withView, ZOOM_MAX, ZOOM_MIN } from "./geo-view"

test("zoom stays between 1x and 20x", () => {
  assert.equal(clampZoom(0.2), ZOOM_MIN)
  assert.equal(clampZoom(50), ZOOM_MAX)
  assert.equal(clampZoom(Number.NaN), ZOOM_MIN)
  assert.equal(stepZoom(1, -1), ZOOM_MIN)
  assert.equal(stepZoom(20, 1), ZOOM_MAX)
  assert.equal(stepZoom(2, 1), 3)
  assert.equal(stepZoom(3, -1), 2)
})

test("the view is read from the geo component, else from the first series", () => {
  assert.deepEqual(readView({ geo: [{ zoom: 4, center: [110, -2] }], series: [{ zoom: 9 }] }), {
    zoom: 4,
    center: [110, -2],
  })
  assert.deepEqual(readView({ series: [{ type: "map", zoom: 2.5 }] }), { zoom: 2.5, center: undefined })
  assert.deepEqual(readView({ series: [{ type: "map" }] }), { zoom: 1, center: undefined })
  assert.equal(readView({}), null)
  assert.equal(readView(null), null)
})

test("a view is written back onto the geo component or the map series", () => {
  const view = { zoom: 3, center: [100, 1] as [number, number] }
  assert.deepEqual(withView({ geo: { map: "m" }, series: [{ type: "scatter" }] }, view), {
    geo: { map: "m", zoom: 3, center: [100, 1] },
    series: [{ type: "scatter" }],
  })
  assert.deepEqual(withView({ series: [{ type: "map" }, { type: "x" }] }, view), {
    series: [{ type: "map", zoom: 3, center: [100, 1] }, { type: "x" }],
  })
})

test("no view, or the untouched one, leaves the option as it is", () => {
  const option = { geo: { map: "m" } }
  assert.equal(withView(option, null), option)
  assert.equal(withView(option, { zoom: 1 }), option)
})
