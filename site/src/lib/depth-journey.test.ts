import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  ascentAt,
  causticAt,
  depthAt,
  formatDepth,
  gaugeDepthAt,
  journeyFromBoxes,
  mixColour,
  steppedDepthAt,
  WATER_BY_DEPTH,
  waterAt,
} from "./depth-journey";

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

test("inside the depths section the gauge reads the lit slab's depth, as its label does", () => {
  const depths = { top: 10000, span: 600 };
  assert.equal(gaugeDepthAt(10000 + 100, anchors, depths), -10); // bronze lit (1/6)
  assert.equal(gaugeDepthAt(10000 + 300, anchors, depths), -40); // silver lit (1/2)
  assert.equal(gaugeDepthAt(10000 + 500, anchors, depths), -90); // gold lit (5/6)
  assert.equal(gaugeDepthAt(10000 + 250, anchors, depths, true), -40);
  assert.equal(gaugeDepthAt(500, anchors, depths), -50); // outside: the chapter anchors
});

test("formatDepth writes whole metres with a real minus sign", () => {
  assert.equal(formatDepth(-40.4), "\u221240 m");
  assert.equal(formatDepth(-0.2), "0 m");
});

test("the ascent look is on only between the climb's start and the surface, fading in and out", () => {
  assert.equal(ascentAt(900, 1000, 5000, 200), 0);
  assert.equal(ascentAt(1100, 1000, 5000, 200), 0.5);
  assert.equal(ascentAt(3000, 1000, 5000, 200), 1);
  assert.equal(ascentAt(4900, 1000, 5000, 200), 0.5);
  assert.equal(ascentAt(5100, 1000, 5000, 200), 0);
});

test("caustic light grows as the water gets shallower", () => {
  assert.equal(causticAt(-100), 0);
  assert.ok(causticAt(-50) < causticAt(-15));
  assert.equal(causticAt(-3), 1);
});

// WEB-5 review fix R9: the pure part of the one measurement both the gauge
// and the water use. A 1000 px screen and boxes laid out like the page.
const chapters = [
  { id: "hero", depth: 0 },
  { id: "sources", depth: -5 },
  { id: "depths", depth: -10 },
  { id: "assistant", depth: -100 },
  { id: "dashboards", depth: -100 },
  { id: "final", depth: -3 },
];
const boxes = {
  hero: { top: 0, height: 2200 },
  sources: { top: 2200, height: 1000 },
  depths: { top: 3200, height: 4000 },
  assistant: { top: 7200, height: 1000 },
  dashboards: { top: 8200, height: 1000 },
  final: { top: 9200, height: 800 },
};
const vh = 1000;

test("journeyFromBoxes puts each chapter's depth where its top reaches the middle of the screen, plus the Gold pin end and the surface", () => {
  const j = journeyFromBoxes(chapters, boxes, vh, { holdSurfaceThroughHero: false, surfaceAtFinal: 0.5 });
  assert.deepEqual(j.anchors, [
    { at: -500, depth: 0 },
    { at: 1700, depth: -5 },
    { at: 2700, depth: -10 },
    { at: 6200, depth: -90 },
    { at: 6700, depth: -100 },
    { at: 7700, depth: -100 },
    { at: 8700, depth: -3 },
    { at: 9100, depth: 0 },
  ]);
  assert.deepEqual(j.depths, { top: 3200, span: 3000 });
  assert.deepEqual(j.climb, { from: 7200, to: 9100 });
});

test("holding the surface through the hero adds one anchor at the end of the hero's pin and changes nothing else", () => {
  const free = journeyFromBoxes(chapters, boxes, vh, { holdSurfaceThroughHero: false, surfaceAtFinal: 0.5 });
  const held = journeyFromBoxes(chapters, boxes, vh, { holdSurfaceThroughHero: true, surfaceAtFinal: 0.5 });
  assert.deepEqual(held.anchors, [...free.anchors.slice(0, 1), { at: 1200, depth: 0 }, ...free.anchors.slice(1)]);
  assert.deepEqual(held.depths, free.depths);
  assert.deepEqual(held.climb, free.climb);
  // Held: still 0 m just before the pin ends; free: already on its way down.
  assert.equal(depthAt(1199, held.anchors), 0);
  assert.ok(depthAt(1199, free.anchors) < 0);
});

test("journeyFromBoxes skips sections that are not on the page and keeps the anchors in scroll order", () => {
  const j = journeyFromBoxes(chapters, { hero: boxes.hero, final: boxes.final }, vh, {
    holdSurfaceThroughHero: true,
    surfaceAtFinal: 0.5,
  });
  assert.equal(j.depths, null);
  assert.deepEqual(j.climb, { from: 0, to: 9100 });
  assert.deepEqual(
    j.anchors.map((a) => a.depth),
    [0, 0, -3, 0],
  );
  const empty = journeyFromBoxes(chapters, {}, vh, { holdSurfaceThroughHero: false, surfaceAtFinal: 0.5 });
  assert.deepEqual(empty, { anchors: [], depths: null, climb: { from: 0, to: 0 } });
});

test("a depths section no taller than the screen still gets a one-pixel pin", () => {
  const j = journeyFromBoxes(chapters, { depths: { top: 100, height: 800 } }, vh, {
    holdSurfaceThroughHero: false,
    surfaceAtFinal: 0.5,
  });
  assert.deepEqual(j.depths, { top: 100, span: 1 });
});
