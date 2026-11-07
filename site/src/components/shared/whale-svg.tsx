import * as React from "react";
import { WhaleEye } from "./whale-motion";

/**
 * WEB-5 O24/O33: the whale in 2D, the same character as the 3D mascot
 * (O11's round body, O29's face and skin, `components/effects/scene/
 * mascot.ts`): one chubby round body in the mark's deep navy, lit like the
 * logo's dome (the same gradient, a lake-blue rim), two plain glowing white
 * eyes, a foam-white belly with soft lake-blue grooves, small flippers and
 * a little tail fluke curling up behind. It is drawn whole, as it is seen
 * deep under water at the final call.
 *
 * This is the mascot, not the logo: the logo, the icons and the OG images
 * keep the official one-eyed mark (`mascot-svg.tsx`).
 *
 * Drawn in its own 600 x 480 units. A server component: everything is
 * static SVG and CSS, and only the eyes are a client leaf (`WhaleEye`,
 * which shifts them a little towards the final stage's gaze). It blinks
 * and its fins and tail move by CSS (scoped keyframes below), all off under
 * reduced motion. `idPrefix` keeps its gradient ids unique if it is ever
 * drawn twice on a page.
 */

export const WHALE_VIEW = { w: 600, h: 480 } as const;

/*
 * Proportions from the 3D whale (O11), at 137 units per world unit, so the
 * two read as one character: the body is 2.8 wide (the mark's sphere,
 * r 1.4) and widest 0.37 above its waterline (y 314 here), 1.77 tall above
 * it and squashed to 0.82 below; the eyes sit 0.42 of the half-width either
 * side of the middle, 0.64 above the waterline, 0.143 of the body's width;
 * the belly line is highest at the front (0.12 under the waterline) and
 * falls to the sides; the grooves are 0.27 rad apart and fan in towards
 * the bottom.
 */
const BODY_PATH = "M108 266 A192 184 0 0 1 492 266 A192 156 0 0 1 108 266 Z";
const EYES = [
  { cx: 219, cy: 230 },
  { cx: 381, cy: 230 },
] as const;
const EYE_R = 27;
/** The belly line: a quadratic arc from the left side over the front to the right side. */
const BELLY = { x0: 96, x1: 504, ySide: 362, yCtrl: 300 } as const;
const bellyY = (x: number) => {
  const t = (x - BELLY.x0) / (BELLY.x1 - BELLY.x0);
  return (1 - t) * (1 - t) * BELLY.ySide + 2 * t * (1 - t) * BELLY.yCtrl + t * t * BELLY.ySide;
};
const GROOVES = [-3, -2, -1, 0, 1, 2, 3].map((k) => {
  const xTop = 300 + Math.sin(k * 0.27) * 192;
  const top = bellyY(xTop) + 8;
  const xBottom = 300 + (xTop - 300) * 0.55;
  const xMid = (xTop + xBottom) / 2 + (xTop - 300) * 0.08;
  return `M${xTop.toFixed(1)} ${top.toFixed(1)} Q${xMid.toFixed(1)} ${(top + 44).toFixed(1)} ${xBottom.toFixed(1)} 418`;
});

/** The scoped motion: blink, paddle, wag, spout. Unique names, so nothing else on the page can collide. */
const MOTION_CSS = `
@media (prefers-reduced-motion: no-preference) {
  .lk-w-lid { transform-box: fill-box; transform-origin: 50% 50%; animation: lk-w-blink 5.6s ease-in-out infinite; }
  .lk-w-lid-b { animation-delay: 0.04s; }
  .lk-w-fin-l { transform-box: fill-box; transform-origin: 90% 20%; animation: lk-w-paddle 3.2s ease-in-out infinite; }
  .lk-w-fin-r { transform-box: fill-box; transform-origin: 10% 20%; animation: lk-w-paddle-r 3.2s ease-in-out infinite; }
  .lk-w-tail { transform-box: fill-box; transform-origin: 0% 100%; animation: lk-w-wag 2.8s ease-in-out infinite; }
}
@keyframes lk-w-blink { 0%, 94%, 100% { transform: scaleY(1); } 96.5% { transform: scaleY(0.1); } }
@keyframes lk-w-paddle { 0%, 100% { transform: rotate(-4deg); } 50% { transform: rotate(9deg); } }
@keyframes lk-w-paddle-r { 0%, 100% { transform: rotate(4deg); } 50% { transform: rotate(-9deg); } }
@keyframes lk-w-wag { 0%, 100% { transform: rotate(-5deg); } 50% { transform: rotate(6deg); } }
`;

export function WhaleSvg({ className, idPrefix = "final" }: { className?: string; idPrefix?: string }) {
  const id = (n: string) => `lk-w-${n}-${idPrefix}`;
  return (
    <svg viewBox={`0 0 ${WHALE_VIEW.w} ${WHALE_VIEW.h}`} className={className} aria-hidden="true" focusable="false">
      {/* Raw text: React would escape the quotes in the selectors, and a style element's text is not decoded. */}
      <style dangerouslySetInnerHTML={{ __html: MOTION_CSS }} />
      <defs>
        {/* The mark's lit dome (MascotLogo): navy, lit from the upper left. */}
        <radialGradient id={id("body")} cx="34%" cy="24%" r="82%">
          <stop offset="0%" stopColor="#16237a" />
          <stop offset="45%" stopColor="#0a1250" />
          <stop offset="100%" stopColor="#050a30" />
        </radialGradient>
        <linearGradient id={id("rim")} x1="0" y1="0" x2="1" y2="0.3">
          <stop offset="0%" stopColor="#44a6fa" stopOpacity="0" />
          <stop offset="60%" stopColor="#44a6fa" stopOpacity="0.25" />
          <stop offset="100%" stopColor="#9fd0ff" stopOpacity="0.95" />
        </linearGradient>
        <radialGradient id={id("eye")} cx="50%" cy="50%" r="50%">
          <stop offset="0%" stopColor="#ffffff" stopOpacity="0.6" />
          <stop offset="100%" stopColor="#ffffff" stopOpacity="0" />
        </radialGradient>
        <linearGradient id={id("belly")} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#ffffff" />
          <stop offset="100%" stopColor="#d3e7fb" />
        </linearGradient>
        <radialGradient id={id("sheen")} cx="50%" cy="50%" r="50%">
          <stop offset="0%" stopColor="#9fd0ff" stopOpacity="0.16" />
          <stop offset="100%" stopColor="#9fd0ff" stopOpacity="0" />
        </radialGradient>
        <linearGradient id={id("fin")} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#131f6e" />
          <stop offset="100%" stopColor="#070d3a" />
        </linearGradient>
        <clipPath id={id("clip")}>
          <path d={BODY_PATH} />
        </clipPath>
      </defs>

      {/* The tail, as on the 3D whale: a short stalk from behind, dipping and
          curling up to a small two-lobed fluke whose tips turn up. */}
      <g className="lk-w-tail">
        <path d="M424 330 C 480 344, 520 318, 528 250" fill="none" stroke={`url(#${id("fin")})`} strokeWidth="30" strokeLinecap="round" />
        <path
          d="M528 258 C 512 230, 486 214, 458 218 C 480 196, 514 202, 530 226 C 538 196, 566 180, 594 188 C 574 204, 552 228, 540 258 Z"
          fill={`url(#${id("fin")})`}
        />
      </g>

      {/* The body: one round, chubby shape, with a soft plastic sheen. */}
      <path d={BODY_PATH} fill={`url(#${id("body")})`} />
      <g clipPath={`url(#${id("clip")})`}>
        {/* The belly, rising a little towards the middle, with its grooves. */}
        <path d={`M${BELLY.x0} ${BELLY.ySide} Q 300 ${BELLY.yCtrl} ${BELLY.x1} ${BELLY.ySide} L ${BELLY.x1} 440 L ${BELLY.x0} 440 Z`} fill={`url(#${id("belly")})`} />
        {GROOVES.map((d, i) => (
          <path key={i} d={d} fill="none" stroke="#7fbaf5" strokeOpacity="0.75" strokeWidth="4" strokeLinecap="round" />
        ))}
        {/* A soft shade along the bottom, so the round body has weight. */}
        <ellipse cx="300" cy="420" rx="192" ry="60" fill="#0b2a66" fillOpacity="0.16" />
      </g>
      <ellipse cx="222" cy="152" rx="96" ry="50" fill={`url(#${id("sheen")})`} transform="rotate(-20 222 152)" />
      {/* The lake-blue rim along the upper right, as the moonlight draws it on the 3D dome. */}
      <path
        d="M300 84 A192 184 0 0 1 490 246"
        fill="none"
        stroke={`url(#${id("rim")})`}
        strokeWidth="9"
        strokeLinecap="round"
        clipPath={`url(#${id("clip")})`}
      />

      {/* Flippers, low on the sides, paddling. */}
      <path className="lk-w-fin-l" d="M124 300 C 92 310, 66 334, 60 362 C 88 358, 120 342, 142 318 Z" fill={`url(#${id("fin")})`} />
      <path className="lk-w-fin-r" d="M476 300 C 508 310, 534 334, 540 362 C 512 358, 480 342, 458 318 Z" fill={`url(#${id("fin")})`} />

      {/* Eyes (O29): two plain glowing whites that blink and shift a little towards the pointer. */}
      {EYES.map((e, i) => (
        <g key={i} className={i === 0 ? "lk-w-lid" : "lk-w-lid lk-w-lid-b"}>
          <WhaleEye cx={e.cx} cy={e.cy} r={EYE_R} glow={`url(#${id("eye")})`} />
        </g>
      ))}

    </svg>
  );
}
