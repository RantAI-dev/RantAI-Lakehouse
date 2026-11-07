"use client";

import * as React from "react";
import { m, type MotionValue } from "framer-motion";
import { DOME, DOME_ARC, DOME_BASE_Y, EYE, MARK_WIDTH } from "./mascot-svg";

/**
 * The final call's creature: the dome half sunk in a night lake, its eye
 * turned up to the headline and following the pointer a little.
 *
 * The stage is 760 x 560 units; the waterline is at y = 372, so the water is
 * the bottom 33.6% of the stage. The dome bobs in HTML while the water, which
 * is full-bleed (as wide as the screen, not as the stage), sits in front of
 * its foot and its waves slide across. The dome's foot is below the
 * waterline (`DOME_Y`), so the bob never shows it.
 *
 * WEB-5 O2: drawn from the official mark (`mascot-svg.tsx`), scaled so its
 * dome is 500 units wide, as the old trace's was. The eye is placed in the
 * stage's own units, outside that scale, so the pointer offsets
 * (`final-mascot.tsx`) keep their size. One eye: this is the mark, drawn
 * large (O3 gives two eyes to the 3D creature only).
 */
const SCALE = 500 / MARK_WIDTH;
const DOME_X = (760 - 500) / 2;
/**
 * The dome's base 34 units under the waterline at y 372, so its eye stands
 * about as high above the water as the old, taller trace's did. The bob
 * lifts it at most 12 units (2.2 % of 560) and tilts its edge about 3, so
 * the foot still never shows.
 */
const DOME_Y = 372 + 34 - DOME_BASE_Y * SCALE;
const EYE_STAGE = { cx: DOME_X + EYE.cx * SCALE, cy: DOME_Y + EYE.cy * SCALE, r: EYE.r * SCALE };
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
                <path d={DOME} />
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
            <g transform={`translate(${DOME_X} ${DOME_Y}) scale(${SCALE})`}>
              <path d={DOME} fill={`url(#${body})`} />
              {/* 9 stage units wide, as before. */}
              <path d={DOME_ARC} fill="none" stroke={`url(#${rim})`} strokeWidth={9 / SCALE} clipPath={`url(#${clip})`} />
            </g>
            <m.g style={{ x: eyeX, y: eyeY }}>
              <circle cx={EYE_STAGE.cx} cy={EYE_STAGE.cy} r={EYE_STAGE.r * 2.3} fill={`url(#${glow})`} />
              <circle {...EYE_STAGE} fill="#ffffff" />
            </m.g>
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
