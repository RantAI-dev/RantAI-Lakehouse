import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  activeLayerIndex,
  focusFromProgress,
  lensCoverage,
  lensLine,
  LENS,
  pchip,
  sceneStateAt,
  smoothstep,
  DIVE,
  type SceneLayout,
} from "./scroll-timeline";

// A 900 px tall viewport laid out the way the page is with the live lake:
// hero 220 vh (WEB-5 D14 S1), sources 230 vh, then the depths section
// (300 vh) whose pinned range is 200 vh.
const VH = 900;
const layout: SceneLayout = {
  vh: VH,
  viewportH: VH,
  depthsTop: VH * (DIVE.heroVh + 2.3),
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

// WEB-5 D14 S1 replaces "crosses during the first viewport": the owner asked
// for a slower dive, so the crossing now sits in a half-viewport waterline
// stretch inside the 220 vh hero, and these tests guard that it stays slow.
test("the camera crosses the surface inside the waterline stretch", () => {
  const before = sceneStateAt(DIVE.waterlineAbove * VH, layout);
  const after = sceneStateAt(DIVE.waterlineBelow * VH, layout);
  assert.ok(before.position[1] > 0, "still above where the stretch starts");
  assert.ok(after.position[1] < 0, "under where it ends");
  assert.ok(sceneStateAt(0.6 * VH, layout).position[1] > 0, "above at 0.6 vh");
});

test("the change from sky to underwater spreads over more than one viewport of scroll", () => {
  let first = -1;
  let last = -1;
  for (let y = 0; y <= 3 * VH; y += 1) {
    const u = sceneStateAt(y, layout).underwater;
    if (first < 0 && u > 0.01) first = y;
    if (last < 0 && u > 0.99) last = y;
  }
  assert.ok(first >= 0 && last > first, `blend from ${first} to ${last}`);
  assert.ok(last - first >= 1.0 * VH, `blend spans ${(last - first) / VH} vh`);
});

test("no small scroll step changes the picture much: at most 2 % underwater per 1 % of a viewport", () => {
  const step = 0.01 * VH;
  let prev = sceneStateAt(0, layout).underwater;
  for (let y = step; y <= 3 * VH; y += step) {
    const u = sceneStateAt(y, layout).underwater;
    assert.ok(Math.abs(u - prev) <= 0.02, `jump of ${u - prev} at ${y / VH} vh`);
    prev = u;
  }
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

// WEB-5 O5 replaces "the camera never rests on the water plane": the
// owner found the old hop across the surface "patah". The camera now moves
// in one continuous path through the surface, and the engine keeps the
// water plane off its near plane instead.
test("the camera moves continuously through the surface, with no hop", () => {
  const step = 0.01 * VH;
  let prev = sceneStateAt(0, layout).position[1];
  for (let y = step; y <= DIVE.under * VH; y += step) {
    const now = sceneStateAt(y, layout).position[1];
    assert.ok(Math.abs(now - prev) <= 0.035, `moved ${now - prev} at ${y / VH} vh`);
    prev = now;
  }
  let crossing = -1;
  for (let y = 0; y <= DIVE.under * VH && crossing < 0; y += 1) if (sceneStateAt(y, layout).position[1] < 0) crossing = y;
  assert.ok(crossing > DIVE.waterlineAbove * VH && crossing < DIVE.waterlineBelow * VH, `crosses at ${crossing / VH} vh`);
  const s = sceneStateAt(crossing, layout);
  assert.equal(s.position[1], s.pathY);
});

test("the waterline sweeps up the whole screen over a long stretch of scroll, never back", () => {
  let enters = -1;
  let leaves = -1;
  let prev = -Infinity;
  for (let y = 0; y <= DIVE.under * VH; y += 1) {
    const line = lensLine(sceneStateAt(y, layout).position[1]);
    assert.ok(line >= prev - 1e-9, `the line moved down at ${y / VH} vh`);
    // While it is on screen: at most 4 % of the screen's height per 1 % of a
    // viewport of scroll.
    if (prev > -Infinity && Math.abs(line) <= 1.1) assert.ok((line - prev) / 2 <= 0.04 / (0.01 * VH), `the line jumped at ${y / VH} vh`);
    if (enters < 0 && line > -1) enters = y;
    if (leaves < 0 && line > 1) leaves = y;
    prev = line;
  }
  assert.ok(enters > 0 && leaves > enters, `from ${enters} to ${leaves}`);
  assert.ok((leaves - enters) / VH >= 0.7, `sweeps in ${(leaves - enters) / VH} vh`);
});

test("both pictures are drawn only while the waterline can be on screen", () => {
  assert.deepEqual(lensCoverage(1), { above: true, below: false });
  assert.deepEqual(lensCoverage(0), { above: true, below: true });
  assert.deepEqual(lensCoverage(-1), { above: false, below: true });
  // Wherever the line (with its waves) can touch the screen, both are there.
  for (let y = -0.5; y <= 0.5; y += 0.005) {
    const c = lensCoverage(y);
    if (Math.abs(y) <= LENS.half + LENS.wave) assert.ok(c.above && c.below, `y ${y}`);
  }
});
