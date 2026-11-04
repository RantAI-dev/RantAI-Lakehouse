"use client";

import * as React from "react";
import { motion, type MotionValue } from "motion/react";

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
  /** Eye offset in SVG units, driven by the pointer where the page wants it. */
  eyeX?: MotionValue<number>;
  eyeY?: MotionValue<number>;
};

/**
 * The logo as a lit character on a dark lake: navy dome with a lake-blue rim
 * light on its upper right, so the dark body still reads against the night.
 */
export function MascotLogo({ className, eyeX, eyeY }: LogoProps) {
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
      <motion.g style={{ x: eyeX, y: eyeY }}>
        <ellipse cx={EYE.cx} cy={EYE.cy} rx={EYE.rx * 2.1} ry={EYE.ry * 2.1} fill={`url(#${glow})`} />
        <ellipse {...EYE} fill="#ffffff" />
      </motion.g>
      <path d={BOWL_PATH} fill={`url(#${water})`} />
    </svg>
  );
}

/**
 * The final call's creature: the dome half sunk in a night lake, its eye
 * turned up to the headline and following the pointer a little.
 *
 * The stage is 760 x 560 units; the waterline is at y = 372, so the water is
 * the bottom 33.6% of the stage. The dome bobs in HTML while the water, which
 * is full-bleed (as wide as the screen, not as the stage), sits in front of
 * its foot and its waves slide across. The dome's foot is 70 units below the
 * waterline, so the bob never shows it.
 */
export function MascotRising({
  className,
  eyeX,
  eyeY,
}: {
  className?: string;
  eyeX: MotionValue<number>;
  eyeY: MotionValue<number>;
}) {
  const uid = React.useId().replace(/:/g, "");
  const body = `lk-rbody-${uid}`;
  const rim = `lk-rrim-${uid}`;
  const glow = `lk-rglow-${uid}`;
  const refl = `lk-rrefl-${uid}`;
  const clip = `lk-rclip-${uid}`;
  const dx = (760 - 512) / 2;
  const dy = 372 - 382 + 70;
  return (
    <div className={className}>
      <div className="relative aspect-[760/560] w-full">
        {/* Soft ring ripples from the waterline. */}
        <div aria-hidden="true" className="absolute top-[66.4%] left-1/2 w-[72%]">
          <span className="lk-ring absolute top-0 left-0 block aspect-[5/1] w-full rounded-[50%] border border-[#44A6FA]/60" />
          <span
            className="lk-ring absolute top-0 left-0 block aspect-[5/1] w-full rounded-[50%] border border-[#44A6FA]/40"
            style={{ animationDelay: "2s" }}
          />
        </div>
        <div className="lk-bob absolute inset-0">
          <svg viewBox="0 0 760 560" className="absolute inset-0 size-full" aria-hidden="true" focusable="false">
            <defs>
              <clipPath id={clip}>
                <path d={DOME_PATH} />
              </clipPath>
              <radialGradient id={body} cx="34%" cy="22%" r="85%">
                <stop offset="0%" stopColor="#1a2a8c" />
                <stop offset="45%" stopColor="#0a1250" />
                <stop offset="100%" stopColor="#050a30" />
              </radialGradient>
              <linearGradient id={rim} x1="0" y1="0" x2="1" y2="0.3">
                <stop offset="0%" stopColor="#44a6fa" stopOpacity="0" />
                <stop offset="60%" stopColor="#44a6fa" stopOpacity="0.2" />
                <stop offset="100%" stopColor="#9fd0ff" stopOpacity="1" />
              </linearGradient>
              <radialGradient id={glow} cx="50%" cy="50%" r="50%">
                <stop offset="0%" stopColor="#ffffff" stopOpacity="0.6" />
                <stop offset="100%" stopColor="#ffffff" stopOpacity="0" />
              </radialGradient>
            </defs>
            <g transform={`translate(${dx} ${dy})`}>
              <path d={DOME_PATH} fill={`url(#${body})`} />
              <path d={DOME_ARC} fill="none" stroke={`url(#${rim})`} strokeWidth="9" clipPath={`url(#${clip})`} />
              <motion.g style={{ x: eyeX, y: eyeY }}>
                <ellipse cx={EYE.cx} cy={EYE.cy} rx={EYE.rx * 2.3} ry={EYE.ry * 2.3} fill={`url(#${glow})`} />
                <ellipse {...EYE} fill="#ffffff" />
              </motion.g>
            </g>
          </svg>
        </div>

        {/* The water: full-bleed, in front of the dome's foot. */}
        <div
          aria-hidden="true"
          className="absolute bottom-0 left-1/2 h-[33.6%] w-screen -translate-x-1/2 bg-[linear-gradient(180deg,#0b2a66_0%,#06143f_55%,#030b2a_100%)]"
        >
          <div className="absolute -top-[14%] left-0 h-[22%] w-full overflow-hidden">
            <svg viewBox="0 0 1520 40" preserveAspectRatio="none" className="lk-wave-slow absolute top-0 left-0 h-full w-[200%]">
              <path d={WAVE_SLOW} fill="#0b2a66" />
              <path d={WAVE_SLOW_LINE} fill="none" stroke="#44a6fa" strokeOpacity="0.6" strokeWidth="1.5" />
            </svg>
            <svg viewBox="0 0 1520 40" preserveAspectRatio="none" className="lk-wave-fast absolute top-[30%] left-0 h-full w-[200%]">
              <path d={WAVE_FAST} fill="#071a4d" fillOpacity="0.92" />
              <path d={WAVE_FAST_LINE} fill="none" stroke="#9fd0ff" strokeOpacity="0.4" strokeWidth="1" />
            </svg>
          </div>
          <svg viewBox="0 0 760 188" preserveAspectRatio="none" className="absolute top-0 left-1/2 h-full w-[min(100%,760px)] -translate-x-1/2" focusable="false">
            <defs>
              <linearGradient id={refl} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor="#44a6fa" stopOpacity="0.3" />
                <stop offset="100%" stopColor="#44a6fa" stopOpacity="0" />
              </linearGradient>
            </defs>
            <ellipse cx="380" cy="34" rx="190" ry="30" fill={`url(#${refl})`} />
          </svg>
        </div>
      </div>
    </div>
  );
}

/** A wave line that tiles seamlessly across 1520 units: `count` full periods. */
function waveLine(count: number, amp: number): string {
  const period = 1520 / count;
  let d = `M0 22`;
  for (let i = 0; i < count; i++) {
    const x0 = i * period;
    d += ` Q${x0 + period / 4} ${22 - amp} ${x0 + period / 2} 22 T${x0 + period} 22`;
  }
  return d;
}
const WAVE_SLOW_LINE = waveLine(8, 14);
const WAVE_SLOW = `${WAVE_SLOW_LINE} V40 H0 Z`;
const WAVE_FAST_LINE = waveLine(16, 8);
const WAVE_FAST = `${WAVE_FAST_LINE} V40 H0 Z`;
