import * as React from "react";

/**
 * The RantAI Lakehouse creature as inline SVG.
 *
 * Geometry is traced from `public/logo-light.png` (512 x 512): a navy dome
 * with one white eye on its right side, a gap for the horizon, and the blue
 * bowl of water below. The same shapes are the 3D mascot's profile
 * (`scene/mascot.ts`), so the WebGL scene, the no-WebGL fallback and the
 * footer all show one character. Used where a canvas would be heavier than
 * the picture it draws: the top bar, the hero fallback, the horizon on the
 * dashboards section, the final call and the footer.
 *
 * WEB-5 review fix R5: a server module. The mark and the logo are static
 * SVG, so they are not hydrated; the final call's moving creature is
 * `mascot-rising.tsx`, the one client piece.
 */

/** Dome silhouette; flat base at y = 382, apex at y = 0. */
export const DOME_PATH = "M6 382 C6 145 101 0 256 0 C411 0 506 145 506 382 Z";
/**
 * The dome's curved edge without its base line. WEB-1 review round 1
 * SHOULD-FIX 3: the rim light used to be the whole dome path stroked 5 units
 * wide, so half of it fell outside the dome (a bright arc beside the edge on
 * the right) and a stray line ran along the base. It is now this open arc,
 * clipped to the dome so it only ever lights the inside of the edge.
 */
export const DOME_ARC = "M6 382 C6 145 101 0 256 0 C411 0 506 145 506 382";
/** Bowl of lake water below the horizon gap. */
export const BOWL_PATH =
  "M22 424 H490 C470 482 438 512 378 512 H134 C74 512 42 482 22 424 Z";
export const EYE = { cx: 343, cy: 260, rx: 51, ry: 60 } as const;

type MarkProps = { className?: string; title?: string };

/**
 * The mark for dark backgrounds: foam dome, abyss eye, lake bowl. The navy
 * dome of the colour logo would vanish on the abyss, so this swaps it for
 * foam, as `public/logo-dark.png` does.
 */
export function MascotMark({ className, title }: MarkProps) {
  return (
    <svg
      viewBox="0 0 512 512"
      className={className}
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      focusable="false"
    >
      <path d={DOME_PATH} fill="#E8F4FF" />
      <ellipse {...EYE} fill="#020514" />
      <path d={BOWL_PATH} fill="#44A6FA" />
    </svg>
  );
}

type LogoProps = {
  className?: string;
};

/**
 * The logo as a lit character on a dark lake: navy dome with a lake-blue rim
 * light on its upper right, so the dark body still reads against the night.
 */
export function MascotLogo({ className }: LogoProps) {
  const uid = React.useId().replace(/:/g, "");
  const body = `lk-body-${uid}`;
  const rim = `lk-rim-${uid}`;
  const water = `lk-water-${uid}`;
  const glow = `lk-glow-${uid}`;
  const clip = `lk-clip-${uid}`;
  return (
    <svg viewBox="0 0 512 512" className={className} aria-hidden="true" focusable="false">
      <defs>
        <clipPath id={clip}>
          <path d={DOME_PATH} />
        </clipPath>
        <radialGradient id={body} cx="34%" cy="22%" r="85%">
          <stop offset="0%" stopColor="#16237a" />
          <stop offset="45%" stopColor="#0a1250" />
          <stop offset="100%" stopColor="#050a30" />
        </radialGradient>
        <linearGradient id={rim} x1="0" y1="0" x2="1" y2="0.2">
          <stop offset="0%" stopColor="#44a6fa" stopOpacity="0" />
          <stop offset="62%" stopColor="#44a6fa" stopOpacity="0.15" />
          <stop offset="100%" stopColor="#8fcbff" stopOpacity="0.95" />
        </linearGradient>
        <linearGradient id={water} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#6cbcff" />
          <stop offset="100%" stopColor="#2f86e0" />
        </linearGradient>
        <radialGradient id={glow} cx="50%" cy="50%" r="50%">
          <stop offset="0%" stopColor="#ffffff" stopOpacity="0.55" />
          <stop offset="100%" stopColor="#ffffff" stopOpacity="0" />
        </radialGradient>
      </defs>
      <path d={DOME_PATH} fill={`url(#${body})`} />
      <path d={DOME_ARC} fill="none" stroke={`url(#${rim})`} strokeWidth="9" clipPath={`url(#${clip})`} />
      <g>
        <ellipse cx={EYE.cx} cy={EYE.cy} rx={EYE.rx * 2.1} ry={EYE.ry * 2.1} fill={`url(#${glow})`} />
        <ellipse {...EYE} fill="#ffffff" />
      </g>
      <path d={BOWL_PATH} fill={`url(#${water})`} />
    </svg>
  );
}

