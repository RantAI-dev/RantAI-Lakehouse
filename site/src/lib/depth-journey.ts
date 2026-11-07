/**
 * WEB-5 D14, the journey down (O33: one descent, no way back up), as pure
 * functions of the scroll position, so they are tested without a browser. The page measures each
 * chapter's top (`JOURNEY` ids) and hands them in; these turn a scroll
 * position into the depth the gauge shows (S3) and the colour of the water
 * behind the second half of the page (S2).
 */

export type Anchor = { at: number; depth: number };

export type Water = { top: string; bottom: string };

/**
 * The water's colour by depth, top and bottom of the screen, deepest
 * first. WEB-5 O33: it only darkens with depth, and the page only goes
 * deeper, so the background never lightens again.
 *
 * WEB-5 O43 (owner: "balikin lah kalo gitu permainan gradient dan partikel
 * partikel airnya"): the O33 ramp is back, with its gradient from the top
 * of the screen to the bottom (O39 had made every stop solid).
 * - −100 m (the assistant) is about the colour the 3D water shows where the
 *   scene zone ends (the lake engine's deep fog through its tone mapping,
 *   lake-engine.ts); the backdrop takes over from that exact colour there
 *   (lib/scene-colour.ts) and eases into this ramp. The scene zone's own
 *   no-WebGL gradient ends on it too (scene-zone.tsx).
 * - −400 m (the bottom, reached as the final call comes on screen, O42) is
 *   the page's own background, #020514, top and bottom alike, which is
 *   also where the final's night sky starts.
 * Shallower than −100 m the backdrop is covered (the canvas or the scene
 * zone's gradient), so the ramp holds its −100 m colour there.
 */
/**
 * Task A (owner, 2026-11-07, "jangan biru semuanya"): below the scene zone the page is the
 * neutral near-black, oklch(0.148 0.004 228.8) = rantai.dev's dark background. The ramp is flat
 * on purpose; the hand-over from the 3D water (depth-backdrop.tsx) blends into it over a
 * viewport and a half, and the final call's night sea is feathered in by its own top band.
 */
export const NEUTRAL = "#090b0c";

export const WATER_BY_DEPTH: readonly { depth: number; top: string; bottom: string }[] = [
  { depth: -400, top: NEUTRAL, bottom: NEUTRAL },
  { depth: -100, top: NEUTRAL, bottom: NEUTRAL },
];

const hex = (c: string): [number, number, number] => [
  parseInt(c.slice(1, 3), 16),
  parseInt(c.slice(3, 5), 16),
  parseInt(c.slice(5, 7), 16),
];
const toHex = (rgb: number[]): string =>
  `#${rgb.map((v) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, "0")).join("")}`;

/** Linear blend of two `#rrggbb` colours. */
export function mixColour(a: string, b: string, t: number): string {
  const ca = hex(a);
  const cb = hex(b);
  return toHex(ca.map((v, i) => v + (cb[i] - v) * t));
}

/** Depth at a scroll position: linear between the measured anchors, held outside them. */
export function depthAt(scrollY: number, anchors: readonly Anchor[]): number {
  if (anchors.length === 0) return 0;
  if (scrollY <= anchors[0].at) return anchors[0].depth;
  for (let i = 1; i < anchors.length; i++) {
    const a = anchors[i - 1];
    const b = anchors[i];
    if (scrollY <= b.at) {
      const span = b.at - a.at;
      return span <= 0 ? b.depth : a.depth + ((b.depth - a.depth) * (scrollY - a.at)) / span;
    }
  }
  return anchors[anchors.length - 1].depth;
}

/** The water's colours at a depth: darker the deeper, held past the ramp's ends. */
export function waterAt(depth: number): Water {
  const stops = WATER_BY_DEPTH;
  if (depth <= stops[0].depth) return { top: stops[0].top, bottom: stops[0].bottom };
  for (let i = 1; i < stops.length; i++) {
    const a = stops[i - 1];
    const b = stops[i];
    if (depth <= b.depth) {
      const t = (depth - a.depth) / (b.depth - a.depth);
      return { top: mixColour(a.top, b.top, t), bottom: mixColour(a.bottom, b.bottom, t) };
    }
  }
  const last = stops[stops.length - 1];
  return { top: last.top, bottom: last.bottom };
}

/** The anchor a stepped reading (reduced motion) shows: the last one already reached. */
export function steppedDepthAt(scrollY: number, anchors: readonly Anchor[]): number {
  let depth = anchors[0]?.depth ?? 0;
  for (const a of anchors) if (scrollY >= a.at) depth = a.depth;
  return depth;
}

/** The pinned depths section, in scroll positions: where its pin starts and how long it lasts. */
export type DepthsRange = { top: number; span: number };

/** The three slabs' depths, in the order the pin shows them (Bronze, Silver, Gold). */
export const SLAB_DEPTHS = [-10, -40, -90] as const;

/**
 * The depth the gauge reads (WEB-5 D14 S3). Inside the pinned depths section
 * it follows the slab in focus, so it reads −10, −40 and −90 m where those
 * slabs are lit, the same readings as their labels; elsewhere it is the
 * chapter anchors' depth. `stepped` (reduced motion) holds each value
 * instead of blending.
 */
export function gaugeDepthAt(
  scrollY: number,
  anchors: readonly Anchor[],
  depths: DepthsRange | null,
  stepped = false,
): number {
  if (depths && scrollY >= depths.top && scrollY <= depths.top + depths.span) {
    const progress = depths.span > 0 ? (scrollY - depths.top) / depths.span : 0;
    // The same focus curve as the slabs: each lit in the middle of its third.
    const focus = Math.min(2, Math.max(0, progress * 3 - 0.5));
    if (stepped) return SLAB_DEPTHS[Math.min(2, Math.floor(progress * 3))];
    const i = Math.min(1, Math.floor(focus));
    const t = focus - i;
    return SLAB_DEPTHS[i] + (SLAB_DEPTHS[i + 1] - SLAB_DEPTHS[i]) * t;
  }
  return (stepped ? steppedDepthAt : depthAt)(scrollY, anchors);
}

/** The bottom of the journey, where the gauge's scale ends (O33). */
export const FLOOR_DEPTH = 400;

/**
 * Where a depth sits on the gauge's scale, 0 (surface) to 1 (the bottom).
 * O33: on a square-root scale, so the first metres (0, −5, −10) and the
 * last hundreds (−300, −400) both have room on one bar.
 */
export function gaugeFraction(depth: number): number {
  return Math.sqrt(Math.min(1, Math.max(0, -depth / FLOOR_DEPTH)));
}

/** "−40 m": a U+2212 minus, whole metres; the surface reads "0 m". */
export function formatDepth(depth: number): string {
  const m = Math.round(Math.abs(depth));
  return m === 0 ? "0 m" : `\u2212${m} m`;
}

/** A section's box in page coordinates (its top plus the scroll position, and its height). */
export type SectionBox = { top: number; height: number };

/** What the page's sections turn into: the depth anchors and the depths pin. */
export type Journey = { anchors: Anchor[]; depths: DepthsRange | null };

/**
 * WEB-5 review fix R9 (SHOULD-FIX): the one place that turns the chapters'
 * boxes into the journey. The gauge and the water used to build their own
 * anchors, and the copies had already drifted apart; both now go through
 * this (via `measureJourney`, lib/measure-journey.ts), so they agree on
 * every depth by construction.
 *
 * - Each chapter's depth applies when its top reaches the middle of the
 *   screen. A chapter without a box (not on the page) is skipped.
 * - `holdSurfaceThroughHero`: also hold the hero's depth until the hero's
 *   pin ends, about where the live camera goes under (S1). The gauge needs
 *   it, because its reading is on screen over the hero and must not leave
 *   0 m while the visitor is still above the water. The water does not:
 *   behind the hero and the scene zone it is covered by the canvas or the
 *   zone's own gradient, and only its colour where that cover ends shows.
 * - The depths pin starts on Bronze (−10 m) and ends on Gold (−90 m).
 * - O33: there is no climb and no surface at the end any more; the last
 *   chapter (the final call) is the deepest point, reached as its top comes
 *   onto the screen (O42), where the gauge then fades out.
 */
export function journeyFromBoxes(
  chapters: readonly { id: string; depth: number }[],
  boxes: Readonly<Record<string, SectionBox | undefined>>,
  viewportH: number,
  opts: { holdSurfaceThroughHero: boolean },
): Journey {
  const half = viewportH / 2;
  const anchors: Anchor[] = [];
  for (const c of chapters) {
    const box = boxes[c.id];
    if (!box) continue;
    // O42: the bottom is reached as the final call comes onto the screen
    // (its top at the screen's bottom), before its night sea is in view.
    anchors.push({ at: c.id === "final" ? box.top - viewportH : box.top - half, depth: c.depth });
    if (c.id === "hero" && opts.holdSurfaceThroughHero) {
      anchors.push({ at: box.top + box.height - viewportH, depth: c.depth });
    }
  }
  let depths: DepthsRange | null = null;
  const d = boxes.depths;
  if (d) {
    depths = { top: d.top, span: Math.max(1, d.height - viewportH) };
    // O33: hold Bronze until the pin starts, where the gauge switches to the
    // slabs' own readings; it used to sink on towards Gold first and then
    // jump back up to −10 m there.
    anchors.push({ at: depths.top, depth: SLAB_DEPTHS[0] });
    anchors.push({ at: depths.top + depths.span, depth: SLAB_DEPTHS[2] });
  }
  anchors.sort((a, b) => a.at - b.at);
  return { anchors, depths };
}
