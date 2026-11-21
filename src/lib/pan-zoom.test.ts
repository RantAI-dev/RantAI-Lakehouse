import { strict as assert } from "node:assert"
import { test } from "node:test"
import { boxHeight, clampView, fitScale, fitView, zoomAt, ZOOM_FLOOR, ZOOM_MAX } from "./pan-zoom"

const PAD = 24
const BOX = { width: 848, height: 448 }

test("the fit shows the whole drawing, never enlarges a small one, and stops at the floor", () => {
  // 1600 × 400 in an 800 × 400 drawable area: the width decides, 0.5.
  assert.equal(fitScale({ width: 1600, height: 400 }, BOX, PAD), 0.5)
  // 800 × 1600: the height decides, 0.25.
  assert.equal(fitScale({ width: 800, height: 1600 }, BOX, PAD), 0.25)
  assert.equal(fitScale({ width: 200, height: 100 }, BOX, PAD), 1)
  assert.equal(fitScale({ width: 200, height: 100_000 }, BOX, PAD), ZOOM_FLOOR)
})

test("the fitted drawing is centred in the box", () => {
  const v = fitView({ width: 1600, height: 400 }, BOX, PAD)
  assert.deepEqual(v, { scale: 0.5, x: 24, y: (448 - 200) / 2 })
})

test("zoom stays between the fit and 2", () => {
  const content = { width: 1600, height: 400 }
  assert.equal(clampView({ scale: 0.1, x: 0, y: 0 }, content, BOX, PAD).scale, 0.5)
  assert.equal(clampView({ scale: 9, x: 0, y: 0 }, content, BOX, PAD).scale, ZOOM_MAX)
  // A drawing smaller than the box cannot be zoomed out below 1.
  assert.equal(clampView({ scale: 0.5, x: 0, y: 0 }, { width: 100, height: 100 }, BOX, PAD).scale, 1)
})

test("a drawing larger than the box moves only until its edge meets the padding", () => {
  const content = { width: 1600, height: 400 }
  // At zoom 1 it is 1600 wide in an 848 box: x runs from 848 - 24 - 1600 to 24.
  assert.equal(clampView({ scale: 1, x: 500, y: 0 }, content, BOX, PAD).x, 24)
  assert.equal(clampView({ scale: 1, x: -5000, y: 0 }, content, BOX, PAD).x, 848 - 24 - 1600)
  // 400 tall fits in 448 - 48: it stays centred however it is dragged.
  assert.equal(clampView({ scale: 1, x: 0, y: -300 }, content, BOX, PAD).y, 24)
})

test("zooming around a point keeps that point of the drawing under it", () => {
  const content = { width: 1600, height: 1600 }
  const start = { scale: 1, x: -400, y: -400 }
  const point = { x: 300, y: 200 }
  const v = zoomAt(start, 1.5, point, content, BOX, PAD)
  assert.equal(v.scale, 1.5)
  // The drawing point under (300, 200) was (700, 600); it is still there.
  assert.equal(v.x + 1.5 * 700, 300)
  assert.equal(v.y + 1.5 * 600, 200)
})

test("the box is as tall as the drawing at the width's fit, up to the cap", () => {
  assert.equal(boxHeight({ width: 400, height: 200 }, 848, PAD, 560), 248)
  // A phone: 1600 wide in a 358 box is drawn at 0.2, so 400 tall becomes 80.
  assert.equal(boxHeight({ width: 1600, height: 400 }, 358, PAD, 560), 80 + 48)
  assert.equal(boxHeight({ width: 400, height: 5000 }, 848, PAD, 560), 560)
})
