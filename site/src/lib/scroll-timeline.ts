/**
 * Maps the page's scroll position to the 3D scene's state. Pure and
 * DOM-free so it can be tested: the engine feeds it `scrollY` and a measured
 * layout every frame and applies what comes back.
 *
 * The story is one continuous camera path (plan §1): hover above the lake,
 * dive through the surface (S1), sink past the falling sources (S2), then
 * settle beside each of the three slabs in turn (S3). The path is a
 * monotone cubic (PCHIP) through hand-placed keys, one per scroll anchor.
 * Monotone, so the camera never overshoots a key and bobs back up; cubic, so
 * it never stops dead at a key the way a chain of eased lerps would.
 */

import { SLAB_Y } from "@/components/effects/scene/scene-config";

export type Vec3 = readonly [number, number, number];

export type SceneLayout = {
  /** Height of `100vh` in px (the large viewport on phones). */
  vh: number;
  /** `window.innerHeight`, which is what `useScroll` measures against. */
  viewportH: number;
  /** Document-space top and height of the `#depths` section. */
  depthsTop: number;
  depthsHeight: number;
};

export type SceneState = {
  position: Vec3;
  look: Vec3;
  /**
   * The camera path's height before the anti-clipping hop (`avoidSurface`):
   * continuous through the surface, so the waterline overlay and the
   * underwater blend follow it without a jump (D14 S1).
   */
  pathY: number;
  /** 0 above the surface, 1 well below it. */
  underwater: number;
  /** 0 at the hero, 1 once the camera has dived through (hero scroll over one `vh`). */
  dive: number;
  /** 0..1 across the pinned depths section. */
  depthProgress: number;
  /** Continuous slab focus: 0 Bronze, 1 Silver, 2 Gold. */
  focus: number;
  activeLayer: 0 | 1 | 2;
  /** Slabs fade in while the camera sinks towards them. */
  slabsReveal: number;
  /** Weights for the view-offset anchors: the hero layout and the depths layout. */
  heroWeight: number;
  depthsWeight: number;
};

export const clamp01 = (x: number): number => Math.min(1, Math.max(0, x));

export function smoothstep(edge0: number, edge1: number, x: number): number {
  const t = clamp01((x - edge0) / (edge1 - edge0));
  return t * t * (3 - 2 * t);
}

/**
 * Monotone cubic interpolation (Fritsch-Carlson / PCHIP) of `ys` over `xs`.
 * `xs` must be strictly increasing. Outside the range it holds the end value.
 */
export function pchip(xs: readonly number[], ys: readonly number[], x: number): number {
  const n = xs.length;
  if (n === 0) return 0;
  if (n === 1 || x <= xs[0]) return ys[0];
  if (x >= xs[n - 1]) return ys[n - 1];
  const h: number[] = [];
  const d: number[] = [];
  for (let k = 0; k < n - 1; k++) {
    h.push(xs[k + 1] - xs[k]);
    d.push((ys[k + 1] - ys[k]) / h[k]);
  }
  const m: number[] = new Array<number>(n).fill(0);
  m[0] = d[0];
  m[n - 1] = d[n - 2];
  for (let k = 1; k < n - 1; k++) {
    if (d[k - 1] * d[k] <= 0) {
      m[k] = 0;
    } else {
      const w1 = 2 * h[k] + h[k - 1];
      const w2 = h[k] + 2 * h[k - 1];
      m[k] = (w1 + w2) / (w1 / d[k - 1] + w2 / d[k]);
    }
  }
  let i = 0;
  while (i < n - 2 && x > xs[i + 1]) i++;
  const t = (x - xs[i]) / h[i];
  const t2 = t * t;
  const t3 = t2 * t;
  return (
    (2 * t3 - 3 * t2 + 1) * ys[i] +
    (t3 - 2 * t2 + t) * h[i] * m[i] +
    (-2 * t3 + 3 * t2) * ys[i + 1] +
    (t3 - t2) * h[i] * m[i + 1]
  );
}

/** Which depth is lit when the pinned section is `progress` of the way through. */
export function activeLayerIndex(progress: number): 0 | 1 | 2 {
  const p = clamp01(progress);
  if (p < 1 / 3) return 0;
  if (p < 2 / 3) return 1;
  return 2;
}

/** Continuous slab focus: each slab is centred on the middle of its third. */
export function focusFromProgress(progress: number): number {
  return Math.min(2, Math.max(0, clamp01(progress) * 3 - 0.5));
}

/**
 * WEB-5 D14 S1, the dive. The owner found the crossing "too sudden": the
 * camera went from 1.15 to 0.05 in 0.42 vh and under by 1.0 vh, so the sky
 * became water in a few dozen pixels of scroll. Now:
 * - the hero is `DIVE.heroVh` viewports tall while the live lake runs (the
 *   DOM hero keeps 150 vh, see `app/globals.css`);
 * - the camera eases down to the waterline, then slows across it: two keys
 *   0.5 vh apart, 0.16 above and 0.16 below the surface, so it crosses at
 *   about 0.6 units per viewport where it used to cross at about 2.6;
 * - after the waterline the camera keeps sinking into the sources section
 *   (the storyboard's −5 m), reaching y -2.4 at 2.7 vh;
 * - the underwater blend (fog, colour, light) follows the camera from y 0.9
 *   down to -1.8, over about two viewports of scroll rather than about 0.3.
 * All in viewport multiples, so the pace is the same on every screen.
 */
export const DIVE = {
  /** Hero section height in viewports, live-lake mode. */
  heroVh: 2.2,
  /** Scroll positions of the camera keys, in viewports. */
  approach: 0.6,
  waterlineAbove: 1.1,
  waterlineBelow: 1.6,
  under: 2.7,
  /** Camera y at each key (the water plane is y 0). */
  yApproach: 0.45,
  yAbove: 0.16,
  yBelow: -0.16,
  yUnder: -2.4,
  /** The underwater blend runs from this camera height down to the next. */
  blendTop: 0.9,
  blendBottom: -1.8,
} as const;

/** Camera height above the slab it is looking at, and its distance from it. */
const SLAB_CAM_DY = 5;
const SLAB_CAM_Z = 9.2;

type Key = { at: number; pos: Vec3; look: Vec3 };

function keysFor(layout: SceneLayout): Key[] {
  const { vh, depthsTop, depthsHeight, viewportH } = layout;
  const span = Math.max(1, depthsHeight - viewportH);
  const slabKey = (i: number, at: number): Key => ({
    at,
    pos: [0, SLAB_Y[i] + SLAB_CAM_DY, SLAB_CAM_Z],
    look: [0, SLAB_Y[i] - 0.3, 0],
  });
  const raw: Key[] = [
    { at: 0, pos: [0, 1.15, 7.2], look: [0, 0.75, 0] },
    { at: DIVE.approach * vh, pos: [0, DIVE.yApproach, 6.9], look: [0, 0.6, 0] },
    // The waterline: a slow, half-above, half-below stretch (D14 S1).
    { at: DIVE.waterlineAbove * vh, pos: [0, DIVE.yAbove, 6.7], look: [0, 0.4, 0] },
    { at: DIVE.waterlineBelow * vh, pos: [0, DIVE.yBelow, 6.6], look: [0, 0.1, 0] },
    { at: DIVE.under * vh, pos: [0, DIVE.yUnder, 6.4], look: [0, -0.6, 0] },
    { at: depthsTop, pos: [0, SLAB_Y[0] + 6.4, 10.2], look: [0, SLAB_Y[0] + 1, 0] },
    slabKey(0, depthsTop + span / 6),
    slabKey(1, depthsTop + span / 2),
    slabKey(2, depthsTop + (span * 5) / 6),
    slabKey(2, depthsTop + span),
  ];
  // Strictly increasing, whatever the measured layout is.
  for (let i = 1; i < raw.length; i++) {
    if (raw[i].at <= raw[i - 1].at) raw[i] = { ...raw[i], at: raw[i - 1].at + 1 };
  }
  return raw;
}

/**
 * Keeps the camera off the water plane itself. Within the camera's near
 * distance of the surface the plane is clipped away and the view opens onto
 * the submerged half of the scene, so the camera hops across a thin band
 * instead of lingering in it. The hop happens at the moment the picture
 * changes from sky to ceiling anyway.
 *
 * WEB-5 D14 S1: the band is 0.07 (was 0.14), so the hop is 0.14 units, still
 * above the camera's 0.05 near plane but half as far as before; with the
 * slow waterline stretch it is one small step inside a long crossing.
 */
export function avoidSurface(y: number): number {
  const band = 0.07;
  const k = 1 - smoothstep(0, 0.6, Math.abs(y));
  return y + (y >= 0 ? band : -band) * k;
}

export function sceneStateAt(scrollY: number, layout: SceneLayout): SceneState {
  const { vh, depthsTop, depthsHeight, viewportH } = layout;
  const keys = keysFor(layout);
  const xs = keys.map((k) => k.at);
  const channel = (pick: (k: Key) => number) =>
    pchip(
      xs,
      keys.map(pick),
      scrollY,
    );
  const pathY = channel((k) => k.pos[1]);
  const position: Vec3 = [channel((k) => k.pos[0]), avoidSurface(pathY), channel((k) => k.pos[2])];
  const look: Vec3 = [channel((k) => k.look[0]), channel((k) => k.look[1]), channel((k) => k.look[2])];

  const span = Math.max(1, depthsHeight - viewportH);
  const depthProgress = clamp01((scrollY - depthsTop) / span);
  return {
    position,
    look,
    pathY,
    // Fully "above" well before the surface and fully "under" well after
    // it, so fog, colour and light change over the whole slow crossing (S1).
    // From the path's height, not the hopped one, so the blend never jumps
    // with the anti-clipping hop (S1).
    underwater: 1 - smoothstep(DIVE.blendBottom, DIVE.blendTop, pathY),
    dive: clamp01(scrollY / Math.max(1, DIVE.waterlineBelow * vh)),
    depthProgress,
    focus: focusFromProgress(depthProgress),
    activeLayer: activeLayerIndex(depthProgress),
    slabsReveal: smoothstep(vh * DIVE.under, depthsTop, scrollY),
    heroWeight: 1 - smoothstep(0.4 * vh, DIVE.waterlineBelow * vh, scrollY),
    depthsWeight: smoothstep(depthsTop - 0.9 * vh, depthsTop - 0.1 * vh, scrollY),
  };
}
