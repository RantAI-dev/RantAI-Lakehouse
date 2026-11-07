/**
 * Shared numbers for the 3D scene. One place so the mascot, the water
 * shader's contact shading and the camera path agree on the same geometry.
 */

/**
 * WEB-5 owner QA finding O2: the dome follows the official mark
 * (rantai-page `public/logo/RantAI Lakehouse Dark.svg`, `components/shared/
 * mascot-svg.tsx`): a circle of r 98 centred at (98, 98), cut flat at
 * y = 124, so a sphere cap that is a little wider above the waterline than
 * at it, and 124 / 98 of its radius tall. It replaces a traced superellipse
 * 1.49 half-widths tall, which made the 3D creature a tall bullet.
 */
const LOGO_R = 98;
/** How far below the sphere's centre the mark cuts the dome flat (124 − 98). */
const LOGO_CUT = 124 - 98;

/**
 * The dome's half-width at the waterline (world units), kept from before so
 * the water's contact shading, the foam collar and the splash, which all
 * sit at the waterline, keep their size.
 */
export const MASCOT_R = 1.35;
/** The dome's sphere: radius, and its centre's height above the waterline. */
export const DOME_SPHERE = (() => {
  const r = (MASCOT_R * LOGO_R) / Math.sqrt(LOGO_R * LOGO_R - LOGO_CUT * LOGO_CUT);
  return { r, cy: (r * LOGO_CUT) / LOGO_R } as const;
})();
/** Dome height above the waterline: 124 / 98 of the sphere's radius, as in the mark. */
export const MASCOT_H = DOME_SPHERE.cy + DOME_SPHERE.r;

/**
 * WEB-5 owner decision O8 (2026-11-07): the creature is a whale. Above the
 * water you see only its head, which is the mark's dome, so the logo and
 * the mascot stay one character; below it, the whole whale.
 *
 * The head is the dome stretched front to back by `HEAD_LONG` (seen from
 * the hero camera, which looks along its length, the silhouette is still
 * the mark's).
 */
export const HEAD_LONG = 1.2;

/**
 * The eyes (O3: two, mirrored about the front; O8: a whale's).
 * - Low on the sides of the head, near the waterline, as on a real whale:
 *   `rho` 0.26 of the dome's height (the mark's own eye sits at 0.32),
 *   `phi` 0.52 rad (30°) either side of the front, so from the hero camera
 *   both still face it enough to read as round.
 * - Small: 0.74 of the mark's eye. Each is a bead, not a flat disc, so it
 *   stays round seen at an angle.
 * - `socket` is how far each eye can move in its socket towards the
 *   pointer (radians round, and fraction of the height up and down); the
 *   head itself turns only a little (`HEAD_TURN`), so both eyes stay on
 *   the face. Before O8 the eyes slid up to 0.6 rad and the dome turned
 *   0.15, which put one eye mid-face and the other on the silhouette.
 */
export const EYES_3D = {
  r: 0.74 * (19.5 / LOGO_R) * DOME_SPHERE.r,
  rho: 0.26,
  phi: 0.52,
  socket: { phi: 0.13, rho: 0.07 },
} as const;
/** How far the head turns towards the pointer: radians round (yaw) and up or down (pitch). */
export const HEAD_TURN = { yaw: 0.06, pitch: 0.03 } as const;

/** Direction the moonlight comes from (towards the moon), unit length. */
export const MOON_DIR: readonly [number, number, number] = (() => {
  const v = [0.06, 0.24, -0.97] as const;
  const l = Math.hypot(...v);
  return [v[0] / l, v[1] / l, v[2] / l] as const;
})();

/** Number of simultaneous water ripples the shader supports. */
export const RIPPLES = 6;

/** Fixed clock for the reduced-motion still frame, picked for a pleasing ripple phase. */
export const STILL_TIME = 7.3;

export const CAMERA_FOV = 42;

/** World y of the three slabs' centres (Bronze, Silver, Gold). */
export const SLAB_Y = [-12, -18.5, -25] as const;
