/**
 * Landing palette, sampled from `public/logo-light.png` (plan §2.3).
 *
 * The CSS in `landing.css` declares the same values as custom properties for
 * class names; this module is for the places that need a number or a
 * THREE.Color (the scene). Change both together. No other hues are used on
 * the page: the three accents belong to the three depths only.
 */
export const PALETTE = {
  abyss: "#020514",
  navy: "#050A30",
  lake: "#44A6FA",
  foam: "#E8F4FF",
  bronze: "#C8834A",
  silver: "#B9C6D3",
  gold: "#F2C14E",
} as const;
