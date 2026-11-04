import { strict as assert } from "node:assert";
import { test } from "node:test";
import { depthAt, mixColour, steppedDepthAt, WATER_BY_DEPTH, waterAt } from "./depth-journey";

const anchors = [
  { at: 0, depth: 0 },
  { at: 1000, depth: -100 },
  { at: 2000, depth: -50 },
  { at: 3000, depth: 0 },
];

test("depthAt is linear between anchors and holds the ends", () => {
  assert.equal(depthAt(-50, anchors), 0);
  assert.equal(depthAt(500, anchors), -50);
  assert.equal(depthAt(1500, anchors), -75);
  assert.equal(depthAt(9999, anchors), 0);
});

test("the journey goes down to the deepest point and back up to the surface", () => {
  const ys = Array.from({ length: 31 }, (_, i) => depthAt(i * 100, anchors));
  const deepest = Math.min(...ys);
  assert.equal(deepest, -100);
  assert.equal(ys[ys.length - 1], 0);
});

test("waterAt is darkest at the deepest point and lands on the final's old night sky at the surface", () => {
  assert.deepEqual(waterAt(-120), { top: "#020514", bottom: "#040a2a" });
  assert.deepEqual(waterAt(0), { top: "#020514", bottom: "#0b2a66" });
});

test("the water lightens on the way up until just below the surface", () => {
  const lum = (c: string) => parseInt(c.slice(1, 3), 16) + parseInt(c.slice(3, 5), 16) + parseInt(c.slice(5, 7), 16);
  let prev = -1;
  for (let d = -100; d <= -3; d += 1) {
    const l = lum(waterAt(d).bottom);
    assert.ok(l >= prev, `darker at ${d} m`);
    prev = l;
  }
});

// Below -3 m the water changes slowly with depth. The last 3 m are the
// surfacing (light water to the night sky), which the page spreads over half
// the final section's height, so it is checked per scroll, not per metre.
test("waterAt is continuous below the surface: one metre never changes a channel by more than 5", () => {
  for (let d = -100; d < -3; d += 1) {
    const a = waterAt(d).top;
    const b = waterAt(d + 1).top;
    for (let i = 1; i < 7; i += 2) {
      assert.ok(Math.abs(parseInt(a.slice(i, i + 2), 16) - parseInt(b.slice(i, i + 2), 16)) <= 5, `${a} -> ${b} at ${d} m`);
    }
  }
  assert.equal(WATER_BY_DEPTH[0].depth, -100);
});

test("mixColour blends, and the stepped reading holds the last anchor reached", () => {
  assert.equal(mixColour("#000000", "#ffffff", 0.5), "#808080");
  assert.equal(steppedDepthAt(1500, anchors), -100);
  assert.equal(steppedDepthAt(2999, anchors), -50);
});
