/**
 * Shared numbers for the 3D scene. One place so the mascot, the water
 * shader's contact shading and the camera path agree on the same geometry.
 */

/** Dome half-width (world units). The logo's dome is 512 wide, 382 tall. */
export const MASCOT_R = 1.35;
/** Dome height: 382 / 256 of the half-width, as in `public/logo-light.png`. */
export const MASCOT_H = MASCOT_R * (382 / 256);
/** Superellipse exponent of the dome profile; > 2 squares the shoulders like the logo. */
export const DOME_EXPONENT = 2.8;

/** Eye centre in the logo, as fractions: x of half-width from centre, y of dome height above the base. */
export const EYE_LOGO = { x: 0.34, y: (382 - 260) / 382, rx: 51 / 256, ry: 60 / 256 } as const;

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
