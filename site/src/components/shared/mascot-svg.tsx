import * as React from "react";

/**
 * The RantAI Lakehouse mark as inline SVG.
 *
 * WEB-5 owner QA finding O2: the geometry is the official mark's, copied
 * from rantai-page `public/logo/RantAI Lakehouse Dark.svg` and
 * `Light.svg` (viewBox 196 x 167). It replaces a hand trace from a PNG
 * that forced the mark into a 512 square, with a tall bullet dome and an
 * oval eye, so the mark looked squashed. The official one is wider than
 * tall (1.17): the dome is a circle of r 98 centred at (98, 98) and cut
 * flat at y = 124, the eye a round hole of r 19.5 at (131.5, 84.5), and
 * the bowl of water runs from y 138 to 166.5. Never fit it into a square:
 * size it by one side and let the other follow (`h-6 w-auto`), as
 * rantai-page's `ProductLogo` does.
 *
 * Used where a canvas would be heavier than the picture it draws: the top
 * bar, the hero fallback, the horizon on the dashboards section, the final
 * call and the footer. The 3D creature (`scene/mascot.ts`) is built on the
 * same round dome (WEB-5 O3 gives it two eyes; the mark keeps one, since
 * the mark is the creature seen from the side).
 *
 * WEB-5 review fix R5: a server module. The mark and the logo are static
 * SVG, so they are not hydrated; the final call's moving creature is
 * `mascot-rising.tsx`, the one client piece.
 */

export const MARK_WIDTH = 196;
export const MARK_HEIGHT = 167;
export const MARK_VIEWBOX = `0 0 ${MARK_WIDTH} ${MARK_HEIGHT}`;

/** The dome alone: the circle (98, 98) r 98, cut flat at y = 124. */
export const DOME =
  "M192.514 124C194.786 115.72 196 107.002 196 98C196 43.8761 152.124 0 98 0C43.8761 0 0 43.8761 0 98C0 107.002 1.21368 115.72 3.4863 124H192.514Z";
/** The official dome path with its eye cut out (`fill-rule="evenodd"`), so the background shows through. */
export const DOME_WITH_EYE = `${DOME}M151 84.5C151 95.2695 142.27 104 131.5 104C120.73 104 112 95.2695 112 84.5C112 73.7305 120.73 65 131.5 65C142.27 65 151 73.7305 151 84.5Z`;
/**
 * The dome's curved edge without its base line, for the rim light. WEB-1
 * review round 1 SHOULD-FIX 3: a stroke of the closed dome put half its
 * width outside the edge and drew a stray line along the base, so the rim
 * is this open arc, clipped to the dome.
 */
export const DOME_ARC =
  "M3.4863 124C1.21368 115.72 0 107.002 0 98C0 43.8761 43.8761 0 98 0C152.124 0 196 43.8761 196 98C196 107.002 194.786 115.72 192.514 124";
/** The bowl of lake water below the horizon gap. */
export const BOWL =
  "M48.9602 166.5C35.5025 166.5 22.0694 161.401 14.9187 150C12.5163 146.17 10.3714 142.162 8.5083 138H187.492C185.629 142.162 183.484 146.17 181.081 150C173.931 161.401 160.498 166.5 147.04 166.5H48.9602Z";
export const EYE = { cx: 131.5, cy: 84.5, r: 19.5 } as const;
/** The dome's flat base (the waterline) and the top of the bowl, in mark units. */
export const DOME_BASE_Y = 124;
export const BOWL_TOP_Y = 138;

type MarkProps = { className?: string; title?: string };

/**
 * The mark for dark backgrounds, in the official Dark colours: a white
 * dome with the eye cut out, and the lake-blue bowl.
 */
export function MascotMark({ className, title }: MarkProps) {
  return (
    <svg
      viewBox={MARK_VIEWBOX}
      width={MARK_WIDTH}
      height={MARK_HEIGHT}
      className={className}
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      focusable="false"
    >
      <path d={BOWL} fill="#44A6FA" fillRule="evenodd" clipRule="evenodd" />
      <path d={DOME_WITH_EYE} fill="white" fillRule="evenodd" clipRule="evenodd" />
    </svg>
  );
}

type LogoProps = {
  className?: string;
};

/**
 * The mark as a lit character on a dark lake, on the official shapes: a
 * navy dome with a lake-blue rim light on its upper right, so the dark body
 * still reads against the night, and a glowing white eye.
 */
export function MascotLogo({ className }: LogoProps) {
  const uid = React.useId().replace(/:/g, "");
  const body = `lk-body-${uid}`;
  const rim = `lk-rim-${uid}`;
  const water = `lk-water-${uid}`;
  const glow = `lk-glow-${uid}`;
  const clip = `lk-clip-${uid}`;
  return (
    <svg
      viewBox={MARK_VIEWBOX}
      width={MARK_WIDTH}
      height={MARK_HEIGHT}
      className={className}
      aria-hidden="true"
      focusable="false"
    >
      <defs>
        <clipPath id={clip}>
          <path d={DOME} />
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
      <path d={DOME} fill={`url(#${body})`} />
      {/* 3.5 units: the old rim's 9 of 512, at this mark's scale. */}
      <path d={DOME_ARC} fill="none" stroke={`url(#${rim})`} strokeWidth="3.5" clipPath={`url(#${clip})`} />
      <g>
        <circle cx={EYE.cx} cy={EYE.cy} r={EYE.r * 2.1} fill={`url(#${glow})`} />
        <circle {...EYE} fill="#ffffff" />
      </g>
      <path d={BOWL} fill={`url(#${water})`} />
    </svg>
  );
}
