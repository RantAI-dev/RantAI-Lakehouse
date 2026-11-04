/**
 * WEB-5 D14, the journey down and back up, as pure functions of the scroll
 * position, so they are tested without a browser. The page measures each
 * chapter's top (`JOURNEY` ids) and hands them in; these turn a scroll
 * position into the depth the gauge shows (S3) and the colour of the water
 * behind the second half of the page (S2).
 */

export type Anchor = { at: number; depth: number };

export type Water = { top: string; bottom: string };

/**
 * The water's colour by depth, top and bottom of the screen: from the
 * abyss at the deepest point, lighter on the way up, to the night sky over
 * the lake at the surface. The surface stop is the final scene's old
 * background (#020514 sky over #0b2a66 water), so the return lands on the
 * picture the page always ended on.
 */
export const WATER_BY_DEPTH: readonly { depth: number; top: string; bottom: string }[] = [
  { depth: -100, top: "#020514", bottom: "#040a2a" },
  { depth: -75, top: "#050a30", bottom: "#0b1a4a" },
  { depth: -50, top: "#0b1a4a", bottom: "#10286a" },
  { depth: -15, top: "#0d2462", bottom: "#163f8c" },
  { depth: -3, top: "#10306f", bottom: "#1c4c9c" },
  { depth: 0, top: "#020514", bottom: "#0b2a66" },
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

/**
 * The water's colours at a depth. Past the deepest point the depth goes
 * back up, but the colour depends only on how deep it is, so the ascent
 * lightens as the descent darkened.
 */
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

/** "−40 m": a U+2212 minus, whole metres; the surface reads "0 m". */
export function formatDepth(depth: number): string {
  const m = Math.round(Math.abs(depth));
  return m === 0 ? "0 m" : `\u2212${m} m`;
}

/**
 * WEB-5 D14 S5, the ascent: how much of the way-up look to show, 0..1. It is
 * 1 from where the climb starts (`from`, the dashboards) to just before the
 * surface (`to`, halfway into the final), with `fade` of scroll to come in
 * and to go out, and 0 elsewhere (the dive, the deepest point, the night
 * sky). Bubbles and caustics are scaled by it.
 */
export function ascentAt(scrollY: number, from: number, to: number, fade: number): number {
  if (!(to > from) || fade <= 0) return 0;
  const rise = Math.min(1, Math.max(0, (scrollY - from) / fade));
  const fall = Math.min(1, Math.max(0, (to - scrollY) / fade));
  return Math.min(rise, fall);
}

/** Caustic light by depth: none at the deepest point, strongest just under the surface. */
export function causticAt(depth: number): number {
  return Math.min(1, Math.max(0, (depth + 100) / 97));
}


/** A section's box in page coordinates (its top plus the scroll position, and its height). */
export type SectionBox = { top: number; height: number };

/** What the page's sections turn into: the depth anchors, the depths pin, and the climb (S5). */
export type Journey = { anchors: Anchor[]; depths: DepthsRange | null; climb: { from: number; to: number } };

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
 * - The depths pin ends on Gold (−90 m), so that is the depth after it.
 * - The surface (0 m) is reached `surfaceAtFinal` of the way into the final.
 * - The climb runs from a screen before the dashboards to the surface.
 */
export function journeyFromBoxes(
  chapters: readonly { id: string; depth: number }[],
  boxes: Readonly<Record<string, SectionBox | undefined>>,
  viewportH: number,
  opts: { holdSurfaceThroughHero: boolean; surfaceAtFinal: number },
): Journey {
  const half = viewportH / 2;
  const anchors: Anchor[] = [];
  for (const c of chapters) {
    const box = boxes[c.id];
    if (!box) continue;
    anchors.push({ at: box.top - half, depth: c.depth });
    if (c.id === "hero" && opts.holdSurfaceThroughHero) {
      anchors.push({ at: box.top + box.height - viewportH, depth: c.depth });
    }
  }
  let depths: DepthsRange | null = null;
  const d = boxes.depths;
  if (d) {
    depths = { top: d.top, span: Math.max(1, d.height - viewportH) };
    anchors.push({ at: depths.top + depths.span, depth: SLAB_DEPTHS[2] });
  }
  const final = boxes.final;
  if (final) anchors.push({ at: final.top + final.height * opts.surfaceAtFinal - half, depth: 0 });
  anchors.sort((a, b) => a.at - b.at);
  const dash = boxes.dashboards;
  const surface = anchors[anchors.length - 1];
  return {
    anchors,
    depths,
    climb: { from: dash ? dash.top - viewportH : 0, to: surface ? surface.at : 0 },
  };
}
