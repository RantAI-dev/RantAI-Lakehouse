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
   * The camera path's height. Since WEB-5 O5 it is the camera's own height
   * (there is no hop across the surface any more); kept as its own field
   * because the underwater blend is defined on the path.
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
 * became water in a few dozen pixels of scroll. Then (O5) "patah": the
 * waterline was a flat bar over the picture and the camera hopped across a
 * band at the surface. Now:
 * - the hero is `DIVE.heroVh` viewports tall while the live lake runs (the
 *   DOM hero keeps 150 vh, see `app/globals.css`);
 * - the camera eases down to the waterline, then crosses it slowly in one
 *   continuous move: from one lens height above the water (`LENS.half`) to
 *   one below, over 0.8 vh, so the waterline sweeps the whole screen over
 *   about 0.8 vh of scroll, centred where the depth gauge leaves 0 m;
 * - after the waterline the camera keeps sinking into the sources section
 *   (the storyboard's −5 m), reaching y -2.4 at 2.7 vh;
 * - the underwater blend (the creature's gaze, the light) follows the
 *   camera from y 0.9 down to -1.8, over about two viewports of scroll.
 * All in viewport multiples, so the pace is the same on every screen.
 */
export const DIVE = {
  /** Hero section height in viewports, live-lake mode. */
  heroVh: 2.2,
  /** Scroll positions of the camera keys, in viewports. */
  approach: 0.6,
  waterlineAbove: 0.95,
  waterlineBelow: 1.75,
  under: 2.7,
  /** Camera y at each key (the water plane is y 0). */
  yApproach: 0.5,
  yAbove: 0.26,
  yBelow: -0.26,
  yUnder: -2.4,
  /** Camera x at the `under` key: to the whale's right (O8). */
  xUnder: 1.2,
  /** The underwater blend runs from this camera height down to the next. */
  blendTop: 0.9,
  blendBottom: -1.8,
} as const;

/**
 * WEB-5 O5, the lens. The screen is treated as a lens `2 * half` world units
 * tall, centred on the camera: the water stands on it at the lake's height,
 * plus a little wave (at most `wave`). The engine and the lens shader both
 * use these numbers, so the camera's path and the sweep of the waterline
 * cannot disagree.
 */
export const LENS = { half: 0.26, wave: 0.053 } as const;

/**
 * Where the waterline crosses the screen for a camera at height `y`, as a
 * screen position (-1 bottom, +1 top), ignoring the waves. Above +1 the
 * whole screen is under water; below -1 none of it is.
 */
export function lensLine(y: number): number {
  return -y / LENS.half;
}

/** Which pictures a camera at height `y` needs: the one above the water, the one below, or both. */
export function lensCoverage(y: number): { above: boolean; below: boolean } {
  // A small margin for the blur and the bend around the line.
  const reach = LENS.half * 1.06 + LENS.wave;
  return { above: y > -reach, below: y < reach };
}

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
    // WEB-5 O8: under the surface the camera drifts a little to the right
    // and looks across to the left, along the whale's body (whale.ts).
    { at: DIVE.under * vh, pos: [DIVE.xUnder, DIVE.yUnder, 8.4], look: [-2.4, -1.7, -1.4] },
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
  // One continuous camera (O5): the near-plane clipping at the surface is
  // handled by the engine, which moves the water plane, never the camera.
  const position: Vec3 = [channel((k) => k.pos[0]), pathY, channel((k) => k.pos[2])];
  const look: Vec3 = [channel((k) => k.look[0]), channel((k) => k.look[1]), channel((k) => k.look[2])];

  const span = Math.max(1, depthsHeight - viewportH);
  const depthProgress = clamp01((scrollY - depthsTop) / span);
  return {
    position,
    look,
    pathY,
    // Fully "above" well before the surface and fully "under" well after
    // it, so what follows it changes over the whole slow crossing (S1).
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
