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
 * WEB-5 O11 (owner, 2026-11-07, replacing O8's realistic whale): the
 * creature is a simple, chubby toy whale. Its body is one round shape: the
 * mark's dome is its top, so above the water the silhouette is exactly the
 * logo's, and below the waterline it rounds off into a soft egg, a little
 * squashed, so the whole body is wider than tall. `BODY_LONG` stretches it
 * front to back, like a bean (the hero camera looks along it, so the dome
 * still reads as the mark).
 */
export const BODY_LONG = 1.22;
/** Below the waterline the round body is squashed this much at its belly (1 = a sphere). */
export const BELLY_SQUASH = 0.8;

/**
 * The eyes (O3: two, mirrored about the front; the mark keeps one).
 * - O11 placed them on the front-sides of the dome, `phi` 0.44 rad either
 *   side of the front and `rho` 0.36 of the dome's height, so from the hero
 *   camera both read as a face; `r` 0.2, about a seventh of the body's
 *   width, so they read at phone size.
 * - O29 (owner): plain glowing white beads in the logo's style, no pupil,
 *   no highlight, no blush. They follow the pointer by a small shift in
 *   their sockets (`socket`: radians round, fraction of the height), and the
 *   head turns only a little (`HEAD_TURN`).
 */
export const EYES_3D = { r: 0.2, rho: 0.36, phi: 0.44, socket: { phi: 0.08, rho: 0.04 } } as const;
/**
 * How the head turns (radians).
 * WEB-5 O51 (owner: "buat pausnya lebih mereng ke arah kanan aja, jadi biar
 * tetap keliatan kalo ini matanya satu, cuma kalo user ngearahin kursornya
 * terlalu kiri/atau kanan tetap keliatan kalo dia tu matanya tetap dua"):
 * - `rest`: at rest it is turned to the right, as the mark is seen from the
 *   side, so one eye shows on the right of the dome and the other is just
 *   behind its silhouette from the hero camera (its eyes sit 0.44 either
 *   side of the front, 0.14 wide; the silhouette from 7 units is at about
 *   1.38; at 1.15 a sliver of the far eye still showed, so 1.3).
 * - `left` / `right`: with a mouse near the screen's left or right edge it
 *   turns to look at it, far enough that both eyes show.
 * - `pitch`: a small nod with the pointer's height.
 */
export const HEAD_TURN = { rest: 1.3, left: -0.3, right: 0.3, pitch: 0.03 } as const;

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
