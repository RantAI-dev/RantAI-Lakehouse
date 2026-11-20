#!/usr/bin/env node
/**
 * Draws the lake depth contours behind the sign-in page's preview (`WEB-15`
 * T1, owner 2026-11-14: the grey panel was "kosong banget"). Writes
 * `public/login-contours-0.svg`, `-1.svg` and `-2.svg`: the same lake, its
 * depth lines drawn at three water levels a third of a step apart. The page
 * shows them as CSS masks (so the colour comes from the theme) and, with
 * motion allowed, lets each one spread out from the deep point and fade in
 * turn, which reads as ripples rolling outward (T6, `login.css`).
 *
 * Deterministic: a fixed seed, no dependencies. Run it again only to change
 * the drawing: `node scripts/login-contours.mjs`.
 *
 * How it is drawn: a depth field (three soft basins, one large behind the
 * dashboard card, plus a low, seeded swell so no line is a clean ellipse) is
 * sampled on an 8 px grid; marching squares finds each depth level, the
 * segments are joined into lines, and each line is smoothed into cubic
 * curves. The drawing is 2000 x 1704 at 1:1 px, so the lines stay 1 px at
 * every panel size; the card's top-left corner sits at (ANCHOR_X, ANCHOR_Y).
 */
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const W = 2000;
/** Both sizes are whole grid steps (`STEP`). */
const H = 1704;
/** Where the card's top-left corner lands in the drawing (the page positions the masks to match). */
const ANCHOR_X = 400;
const ANCHOR_Y = 320;
const STEP = 8;
const LEVELS = 17;
const PHASES = 3;

/** mulberry32: a small seeded generator, so the lake is the same on every run. */
function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const rand = rng(20261114);
const waves = Array.from({ length: 6 }, (_, i) => {
  const angle = rand() * Math.PI * 2;
  const k = (0.0035 + rand() * 0.004) * (i < 3 ? 1 : 1.9);
  return { kx: Math.cos(angle) * k, ky: Math.sin(angle) * k, phase: rand() * Math.PI * 2, amp: i < 3 ? 0.07 : 0.025 };
});

/** Basins in drawing coordinates: centre, radii and depth. */
const basins = [
  { x: ANCHOR_X + 430, y: ANCHOR_Y + 420, rx: 560, ry: 470, d: 1.0 },
  { x: ANCHOR_X + 40, y: ANCHOR_Y + 790, rx: 260, ry: 210, d: 0.55 },
  { x: ANCHOR_X + 760, y: ANCHOR_Y - 60, rx: 300, ry: 200, d: 0.45 },
];

function depth(x, y) {
  let v = 0;
  for (const b of basins) {
    const dx = (x - b.x) / b.rx;
    const dy = (y - b.y) / b.ry;
    v += b.d * Math.exp(-(dx * dx + dy * dy));
  }
  for (const w of waves) v += w.amp * Math.sin(w.kx * x + w.ky * y + w.phase);
  return v;
}

const cols = W / STEP + 1;
const rows = H / STEP + 1;
const field = new Float64Array(cols * rows);
let min = Infinity;
let max = -Infinity;
for (let j = 0; j < rows; j++) {
  for (let i = 0; i < cols; i++) {
    const v = depth(i * STEP, j * STEP);
    field[j * cols + i] = v;
    if (v < min) min = v;
    if (v > max) max = v;
  }
}

/** Marching squares for one level: segments as pairs of points. */
function segments(level) {
  const out = [];
  const at = (i, j) => field[j * cols + i];
  const lerp = (a, b, va, vb) => a + ((level - va) / (vb - va)) * (b - a);
  for (let j = 0; j < rows - 1; j++) {
    for (let i = 0; i < cols - 1; i++) {
      const v0 = at(i, j), v1 = at(i + 1, j), v2 = at(i + 1, j + 1), v3 = at(i, j + 1);
      const c = (v0 > level ? 1 : 0) | (v1 > level ? 2 : 0) | (v2 > level ? 4 : 0) | (v3 > level ? 8 : 0);
      if (c === 0 || c === 15) continue;
      const x = i * STEP, y = j * STEP;
      const top = () => [lerp(x, x + STEP, v0, v1), y];
      const right = () => [x + STEP, lerp(y, y + STEP, v1, v2)];
      const bottom = () => [lerp(x, x + STEP, v3, v2), y + STEP];
      const left = () => [x, lerp(y, y + STEP, v0, v3)];
      const centre = (v0 + v1 + v2 + v3) / 4 > level;
      switch (c) {
        case 1: case 14: out.push([left(), top()]); break;
        case 2: case 13: out.push([top(), right()]); break;
        case 3: case 12: out.push([left(), right()]); break;
        case 4: case 11: out.push([right(), bottom()]); break;
        case 6: case 9: out.push([top(), bottom()]); break;
        case 7: case 8: out.push([left(), bottom()]); break;
        case 5: if (centre) { out.push([left(), top()], [right(), bottom()]); } else { out.push([left(), bottom()], [top(), right()]); } break;
        case 10: if (centre) { out.push([top(), right()], [left(), bottom()]); } else { out.push([left(), top()], [right(), bottom()]); } break;
      }
    }
  }
  return out;
}

/** Joins segments that share an end into lines (closed or open at the drawing's edge). */
function joinSegments(segs) {
  const key = (p) => `${Math.round(p[0] * 100)},${Math.round(p[1] * 100)}`;
  const ends = new Map();
  segs.forEach((s, n) => {
    for (const p of s) {
      const k = key(p);
      if (!ends.has(k)) ends.set(k, []);
      ends.get(k).push(n);
    }
  });
  const used = new Uint8Array(segs.length);
  const lines = [];
  for (let n = 0; n < segs.length; n++) {
    if (used[n]) continue;
    used[n] = 1;
    const line = [segs[n][0], segs[n][1]];
    for (const forward of [true, false]) {
      for (;;) {
        const tip = forward ? line[line.length - 1] : line[0];
        const next = (ends.get(key(tip)) ?? []).find((m) => !used[m]);
        if (next === undefined) break;
        used[next] = 1;
        const [a, b] = segs[next];
        const far = key(a) === key(tip) ? b : a;
        if (forward) line.push(far); else line.unshift(far);
      }
    }
    lines.push(line);
  }
  return lines;
}

/** Every third grid crossing (about 24 px apart), then a Catmull-Rom curve through them as cubic Béziers, in whole pixels and relative steps to keep the files small. */
function path(line) {
  const closed = Math.hypot(line[0][0] - line.at(-1)[0], line[0][1] - line.at(-1)[1]) < 0.01;
  let pts = line.filter((_, i) => i % 3 === 0);
  if (closed) pts = pts.filter((p, i) => i === 0 || Math.hypot(p[0] - pts[0][0], p[1] - pts[0][1]) > 0.01);
  else if (pts.at(-1) !== line.at(-1)) pts.push(line.at(-1));
  if (pts.length < 3) return "";
  const n = pts.length;
  const get = (i) => (closed ? pts[(i + n) % n] : pts[Math.max(0, Math.min(n - 1, i))]);
  const r = Math.round;
  let at = [r(pts[0][0]), r(pts[0][1])];
  let d = `M${at[0]} ${at[1]}`;
  const last = closed ? n : n - 1;
  for (let i = 0; i < last; i++) {
    const p0 = get(i - 1), p1 = get(i), p2 = get(i + 1), p3 = get(i + 2);
    const c1 = [r(p1[0] + (p2[0] - p0[0]) / 6), r(p1[1] + (p2[1] - p0[1]) / 6)];
    const c2 = [r(p2[0] - (p3[0] - p1[0]) / 6), r(p2[1] - (p3[1] - p1[1]) / 6)];
    const to = [r(p2[0]), r(p2[1])];
    d += `c${nums([c1[0] - at[0], c1[1] - at[1], c2[0] - at[0], c2[1] - at[1], to[0] - at[0], to[1] - at[1]])}`;
    at = to;
  }
  return closed ? `${d}Z` : d;
}

/** Numbers for path data: a minus sign already separates a number from the one before it. */
function nums(values) {
  return values.map((v, i) => (i > 0 && v >= 0 ? ` ${v}` : String(v))).join("");
}

/** Lines too short to read as a contour (a fleck at a saddle) are left out. */
function length(line) {
  let s = 0;
  for (let i = 1; i < line.length; i++) s += Math.hypot(line[i][0] - line[i - 1][0], line[i][1] - line[i - 1][1]);
  return s;
}

const outDir = join(dirname(fileURLToPath(import.meta.url)), "..", "public");
const lo = Math.max(min, 0.04);
const span = (max - lo) / (LEVELS + 1);
for (let p = 0; p < PHASES; p++) {
  const ds = [];
  for (let l = 0; l < LEVELS; l++) {
    const level = lo + (l + p / PHASES) * span;
    for (const line of joinSegments(segments(level))) {
      if (length(line) < 60) continue;
      const d = path(line);
      if (d) ds.push(d);
    }
  }
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">` +
    `<!-- Generated by scripts/login-contours.mjs (WEB-15). Lake depth lines, phase ${p} of ${PHASES}. -->` +
    `<path d="${ds.join("")}" fill="none" stroke="#000" stroke-width="1" stroke-linejoin="round"/></svg>\n`;
  writeFileSync(join(outDir, `login-contours-${p}.svg`), svg);
  console.log(`login-contours-${p}.svg: ${ds.length} lines, ${(svg.length / 1024).toFixed(1)} KiB`);
}
