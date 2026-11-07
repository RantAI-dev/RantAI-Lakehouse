import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  depthAt,
  FLOOR_DEPTH,
  formatDepth,
  gaugeDepthAt,
  gaugeFraction,
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

// WEB-5 O33: one descent. The reading and the water's colour only ever go
// one way down the page.
test("the water only darkens with depth, down to the page's own background at the bottom", () => {
  // Relative luminance (Rec. 709 weights, on the sRGB values: monotone is all this needs).
  const lum = (c: string) =>
    0.2126 * parseInt(c.slice(1, 3), 16) + 0.7152 * parseInt(c.slice(3, 5), 16) + 0.0722 * parseInt(c.slice(5, 7), 16);
  let prevTop = Infinity;
  let prevBottom = Infinity;
  for (let d = -100; d >= -FLOOR_DEPTH; d -= 1) {
    const w = waterAt(d);
    assert.ok(lum(w.top) <= prevTop && lum(w.bottom) <= prevBottom, `lighter at ${d} m`);
    prevTop = lum(w.top);
    prevBottom = lum(w.bottom);
  }
  assert.deepEqual(waterAt(-FLOOR_DEPTH), { top: "#020514", bottom: "#020514" });
  // O43: a gradient on screen (lighter at the top) until the bottom, where it is the page's own colour.
  assert.notEqual(waterAt(-150).top, waterAt(-150).bottom);
  assert.deepEqual(waterAt(-1000), waterAt(-FLOOR_DEPTH));
  assert.deepEqual(waterAt(-20), waterAt(-100));
  assert.equal(WATER_BY_DEPTH[0].depth, -FLOOR_DEPTH);
});

test("waterAt is continuous: one metre never changes a channel by more than 2", () => {
  for (let d = -FLOOR_DEPTH; d < -100; d += 1) {
    for (const k of ["top", "bottom"] as const) {
      const a = waterAt(d)[k];
      const b = waterAt(d + 1)[k];
      for (let i = 1; i < 7; i += 2) {
        assert.ok(Math.abs(parseInt(a.slice(i, i + 2), 16) - parseInt(b.slice(i, i + 2), 16)) <= 2, `${a} -> ${b} at ${d} m`);
      }
    }
  }
});

test("the gauge's scale is a square root of the depth: the surface at the top, the bottom at the end", () => {
  assert.equal(gaugeFraction(0), 0);
  assert.equal(gaugeFraction(-FLOOR_DEPTH), 1);
  assert.equal(gaugeFraction(-100), 0.5);
  assert.equal(gaugeFraction(-9999), 1);
  assert.ok(gaugeFraction(-10) > 0.15, "the first metres have room");
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

// WEB-5 review fix R9: the pure part of the one measurement both the gauge
// and the water use. A 1000 px screen and boxes laid out like the page,
// with O33's one-descent depths.
const chapters = [
  { id: "hero", depth: 0 },
  { id: "sources", depth: -5 },
  { id: "depths", depth: -10 },
  { id: "assistant", depth: -100 },
  { id: "dashboards", depth: -150 },
  { id: "final", depth: -400 },
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

test("journeyFromBoxes puts each chapter's depth where its top reaches the middle of the screen, plus the Gold pin end", () => {
  const j = journeyFromBoxes(chapters, boxes, vh, { holdSurfaceThroughHero: false });
  assert.deepEqual(j.anchors, [
    { at: -500, depth: 0 },
    { at: 1700, depth: -5 },
    { at: 2700, depth: -10 },
    { at: 3200, depth: -10 },
    { at: 6200, depth: -90 },
    { at: 6700, depth: -100 },
    { at: 7700, depth: -150 },
    { at: 8200, depth: -400 },
  ]);
  assert.deepEqual(j.depths, { top: 3200, span: 3000 });
});

test("O33: the reading only ever goes down the page, and ends at the bottom", () => {
  const j = journeyFromBoxes(chapters, boxes, vh, { holdSurfaceThroughHero: true });
  let prev = Infinity;
  for (let y = -1000; y <= 11000; y += 10) {
    const d = gaugeDepthAt(y, j.anchors, j.depths);
    assert.ok(d <= prev + 1e-9, `rose at ${y}`);
    prev = d;
  }
  assert.equal(prev, -400);
});

test("holding the surface through the hero adds one anchor at the end of the hero's pin and changes nothing else", () => {
  const free = journeyFromBoxes(chapters, boxes, vh, { holdSurfaceThroughHero: false });
  const held = journeyFromBoxes(chapters, boxes, vh, { holdSurfaceThroughHero: true });
  assert.deepEqual(held.anchors, [...free.anchors.slice(0, 1), { at: 1200, depth: 0 }, ...free.anchors.slice(1)]);
  assert.deepEqual(held.depths, free.depths);
  // Held: still 0 m just before the pin ends; free: already on its way down.
  assert.equal(depthAt(1199, held.anchors), 0);
  assert.ok(depthAt(1199, free.anchors) < 0);
});

test("journeyFromBoxes skips sections that are not on the page and keeps the anchors in scroll order", () => {
  const j = journeyFromBoxes(chapters, { hero: boxes.hero, final: boxes.final }, vh, { holdSurfaceThroughHero: true });
  assert.equal(j.depths, null);
  assert.deepEqual(
    j.anchors.map((a) => a.depth),
    [0, 0, -400],
  );
  const empty = journeyFromBoxes(chapters, {}, vh, { holdSurfaceThroughHero: false });
  assert.deepEqual(empty, { anchors: [], depths: null });
});

test("a depths section no taller than the screen still gets a one-pixel pin", () => {
  const j = journeyFromBoxes(chapters, { depths: { top: 100, height: 800 } }, vh, { holdSurfaceThroughHero: false });
  assert.deepEqual(j.depths, { top: 100, span: 1 });
  assert.deepEqual(j.anchors, [
    { at: -400, depth: -10 },
    { at: 100, depth: -10 },
    { at: 101, depth: -90 },
  ]);
});
