import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  activeLayerIndex,
  avoidSurface,
  focusFromProgress,
  pchip,
  sceneStateAt,
  smoothstep,
  type SceneLayout,
} from "./scroll-timeline";

// A 900 px tall viewport laid out the way the page is: hero 150 vh, sources
// 230 vh, then the depths section (300 vh) whose pinned range is 200 vh.
const VH = 900;
const layout: SceneLayout = {
  vh: VH,
  viewportH: VH,
  depthsTop: VH * 3.8,
  depthsHeight: VH * 3,
};

test("pchip passes through every knot", () => {
  const xs = [0, 1, 3, 6];
  const ys = [2, -1, 4, 4];
  xs.forEach((x, i) => assert.ok(Math.abs(pchip(xs, ys, x) - ys[i]) < 1e-9, `knot ${x}`));
});

test("pchip never overshoots the values it is given", () => {
  const xs = [0, 1, 2, 3];
  const ys = [0, 0, 10, 10];
  for (let x = 0; x <= 3; x += 0.05) {
    const v = pchip(xs, ys, x);
    assert.ok(v >= -1e-9 && v <= 10 + 1e-9, `value ${v} at ${x} left the data range`);
  }
});

test("pchip holds the end values outside its range", () => {
  assert.equal(pchip([0, 1], [5, 9], -4), 5);
  assert.equal(pchip([0, 1], [5, 9], 40), 9);
});

test("smoothstep is 0 below, 1 above, and 0.5 at the middle", () => {
  assert.equal(smoothstep(0, 1, -1), 0);
  assert.equal(smoothstep(0, 1, 2), 1);
  assert.equal(smoothstep(0, 1, 0.5), 0.5);
});

test("the camera starts above the lake and ends below the gold slab's top", () => {
  const top = sceneStateAt(0, layout);
  assert.ok(top.position[1] > 1, "starts above the surface");
  assert.equal(top.underwater < 0.01, true);
  const end = sceneStateAt(layout.depthsTop + layout.depthsHeight, layout);
  assert.ok(end.position[1] < -15, "ends deep");
  assert.equal(end.underwater > 0.99, true);
});

test("the camera only ever sinks as the page scrolls down", () => {
  let prev = Infinity;
  for (let y = 0; y <= layout.depthsTop + layout.depthsHeight; y += 20) {
    const s = sceneStateAt(y, layout);
    assert.ok(s.position[1] <= prev + 1e-9, `rose at scrollY ${y}`);
    prev = s.position[1];
  }
});

test("the camera crosses the surface during the first viewport of scroll", () => {
  const before = sceneStateAt(0.1 * VH, layout);
  const after = sceneStateAt(VH, layout);
  assert.ok(before.position[1] > 0, "still above at 0.1 vh");
  assert.ok(after.position[1] < 0, "under at 1 vh");
});

test("depth progress is 0 before the section and 1 after its pinned range", () => {
  assert.equal(sceneStateAt(0, layout).depthProgress, 0);
  assert.equal(sceneStateAt(layout.depthsTop + layout.depthsHeight, layout).depthProgress, 1);
  const mid = sceneStateAt(layout.depthsTop + (layout.depthsHeight - VH) / 2, layout);
  assert.ok(Math.abs(mid.depthProgress - 0.5) < 1e-9);
});

test("each layer becomes active in turn: bronze, silver, gold", () => {
  assert.equal(activeLayerIndex(0), 0);
  assert.equal(activeLayerIndex(0.32), 0);
  assert.equal(activeLayerIndex(0.34), 1);
  assert.equal(activeLayerIndex(0.66), 1);
  assert.equal(activeLayerIndex(0.67), 2);
  assert.equal(activeLayerIndex(1), 2);
});

test("slab focus is centred on each third of the pinned range", () => {
  assert.ok(Math.abs(focusFromProgress(1 / 6) - 0) < 1e-9);
  assert.ok(Math.abs(focusFromProgress(1 / 2) - 1) < 1e-9);
  assert.ok(Math.abs(focusFromProgress(5 / 6) - 2) < 1e-9);
  assert.equal(focusFromProgress(0), 0);
  assert.equal(focusFromProgress(1), 2);
});

test("a layout that is not measured yet still yields a finite state", () => {
  const s = sceneStateAt(120, { vh: 0, viewportH: 0, depthsTop: 0, depthsHeight: 0 });
  for (const v of [...s.position, ...s.look, s.underwater, s.focus]) {
    assert.ok(Number.isFinite(v));
  }
});

test("the camera never rests on the water plane", () => {
  for (let y = -1; y <= 1; y += 0.01) {
    assert.ok(Math.abs(avoidSurface(y)) >= 0.13, `y ${y} maps to ${avoidSurface(y)}`);
  }
  assert.equal(avoidSurface(5), 5);
  assert.equal(avoidSurface(-5), -5);
});
