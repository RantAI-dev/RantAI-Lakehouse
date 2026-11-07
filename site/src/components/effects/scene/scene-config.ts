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
 * WEB-5 owner decision O3: the 3D creature has two eyes, mirrored about its
 * front, so it reads as a face when it looks at the camera. The mark keeps
 * one eye, since the mark is the creature seen from the side.
 * - Size and height start from the mark's (r 19.5 of 98; the centre 39.5
 *   above the base, 0.32 of the dome's height). Seen from the hero camera,
 *   which looks down a little, two eyes that size sat low and read as
 *   goggles, so each is 0.82 of the mark's eye and sits at 0.42 of the
 *   height (checked in the live hero at 1440 and 390, SwiftShader).
 * - `phi` is each eye's angle from the front, around the vertical axis.
 *   Taken literally, the mark's side view puts the eye about 70° round,
 *   where from the front it shows edge-on at the silhouette, which does not
 *   read as a face. 0.42 rad (24°) each way spaces them like a face at
 *   phone size and in the 1440 hero; the single eye they replace sat at
 *   0.35 rad.
 */
export const EYES_3D = { r: 0.82 * (19.5 / LOGO_R) * DOME_SPHERE.r, rho: 0.42, phi: 0.42 } as const;

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
