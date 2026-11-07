import * as React from "react";
import { SwimLayer } from "./whale-motion";
import { WHALE_VIEW, WHALE_WATERLINE, WhaleSvg } from "./whale-svg";

/**
 * The final call: a night sea, like the hero, so the page closes where it
 * began.
 *
 * WEB-5 O42 (owner, on O33's whale in the abyss: "ini apaan serem banget";
 * the owner chose a night sea like the hero): the whale's head floats at
 * the waterline (O29: navy, two plain glowing white eyes; `WhaleSvg`'s
 * surface mode, so no belly, flipper or tail shows), bobs, blinks and spouts
 * every few seconds. The water is the hero's palette:
 * - a far band of water behind the whale, then two wave bands in front of
 *   it, drifting at different speeds and in opposite directions;
 * - the moon's glitter path under the section's moon (13 % from the left):
 *   small dashes that twinkle;
 * - the whale's broken reflection, shimmering;
 * - a gentle ring at the waterline, and a splash ring as it settles
 *   (`data-arrived`, set by `final-mascot.tsx`).
 * The whale is small (a third of the stage), so the scene reads as calm.
 *
 * A server component: SVG and CSS only. The loops use the page's keyframes
 * (`lk-wave`, `lk-twinkle`, `lk-ring`), which app/globals.css stops under
 * reduced motion; the splash and the whale's own keyframes are gated the
 * same way. The stage is 760 x 560 units with the waterline at y 372
 * (66.4 %); the water is full-bleed (as wide as the screen).
 */

/** The whale's box in the stage, so its own waterline lands on the stage's (y 372). */
const SCALE = 0.62;
const WHALE_BOX = {
  left: (380 - 300 * SCALE) / 760,
  top: (372 - WHALE_WATERLINE * SCALE) / 560,
  width: (WHALE_VIEW.w * SCALE) / 760,
  height: (WHALE_VIEW.h * SCALE) / 560,
};
const pct = (x: number) => `${(x * 100).toFixed(3)}%`;

/** A wave line that tiles seamlessly across 1520 units: `count` full periods, crest at y 22 ± amp. */
function waveLine(count: number, amp: number, phase = 0): string {
  const period = 1520 / count;
  let d = `M0 22`;
  for (let i = 0; i < count; i++) {
    const x0 = i * period;
    const a = amp * (1 + 0.25 * Math.sin(i * 1.7 + phase));
    d += ` Q${(x0 + period / 4).toFixed(1)} ${(22 - a).toFixed(1)} ${(x0 + period / 2).toFixed(1)} 22 T${(x0 + period).toFixed(1)} 22`;
  }
  return d;
}
// Periods that divide the 1520 tile, so each band loops seamlessly.
const WAVE_BACK = `${waveLine(10, 5, 0.4)} V40 H0 Z`;
const WAVE_MID = `${waveLine(8, 8, 1.1)} V40 H0 Z`;
const WAVE_FRONT = `${waveLine(5, 6, 2.3)} V40 H0 Z`;

/** The glitter path: deterministic dashes, wider and further apart towards the viewer. */
const GLITTER = Array.from({ length: 18 }, (_, i) => {
  const d = i / 17;
  return {
    top: 4 + d * 82,
    left: (Math.sin(i * 2.4) * (2 + d * 9)).toFixed(2),
    width: Math.round(6 + d * 22 + ((i * 7) % 5)),
    delay: ((i * 0.37) % 3.2).toFixed(2),
    dur: (2.2 + ((i * 5) % 7) * 0.25).toFixed(2),
  };
});
/** The whale's reflection: a few soft, broken strokes under it. */
const REFLECTION = [
  { top: 6, w: 34, x: 0 },
  { top: 14, w: 26, x: -5 },
  { top: 23, w: 18, x: 4 },
  { top: 33, w: 12, x: -3 },
];

const SCENE_CSS = `
@media (prefers-reduced-motion: no-preference) {
  [data-arrived="on"] .lk-r-splash { animation: lk-r-splash 1.6s cubic-bezier(0.2, 0.6, 0.3, 1) both; }
  [data-arrived="on"] .lk-r-splash-b { animation-delay: 0.25s; }
}
.lk-r-splash { opacity: 0; transform: translate(-50%, -50%) scale(0.6); }
@keyframes lk-r-splash {
  0% { opacity: 0.8; transform: translate(-50%, -50%) scale(0.6); }
  100% { opacity: 0; transform: translate(-50%, -50%) scale(1.7); }
}
`;

export function MascotRising({ className }: { className?: string }) {
  return (
    <div className={className}>
      <style dangerouslySetInnerHTML={{ __html: SCENE_CSS }} />
      <div className="relative aspect-[760/560] w-full">
        {/* The far water, behind the whale: a band that drifts slowly. */}
        <div aria-hidden="true" className="absolute top-[64.6%] left-1/2 h-[10%] w-screen -translate-x-1/2 overflow-hidden">
          <svg viewBox="0 0 1520 40" preserveAspectRatio="none" className="lk-wave-slow absolute top-0 left-0 h-full w-[200%]" style={{ animationDuration: "34s" }}>
            <defs>
              <linearGradient id="lk-final-wave-back" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor="#123478" />
                <stop offset="70%" stopColor="#0b2a66" stopOpacity="0.8" />
                <stop offset="100%" stopColor="#0b2a66" stopOpacity="0" />
              </linearGradient>
            </defs>
            <path d={WAVE_BACK} fill="url(#lk-final-wave-back)" />
          </svg>
        </div>

        {/* The whale, which the stage raises to the surface as the section comes in. */}
        <SwimLayer className="absolute inset-0">
          <div
            className="lk-bob absolute"
            style={{ left: pct(WHALE_BOX.left), top: pct(WHALE_BOX.top), width: pct(WHALE_BOX.width), height: pct(WHALE_BOX.height) }}
          >
            <WhaleSvg surface className="size-full drop-shadow-[0_0_40px_rgba(68,166,250,0.22)]" />
          </div>
        </SwimLayer>

        {/* The near water: full-bleed, in front of the whale below the waterline. On phones it also
            covers the section's bottom padding, so the section ends on the water's last colour. */}
        <div aria-hidden="true" className="absolute bottom-0 left-1/2 h-[33.6%] w-screen -translate-x-1/2 max-md:-bottom-6 max-md:h-[calc(33.6%+1.5rem)]">
          {/* The body of the water fades in under the wave crests, so its top is never a straight edge,
              and ends on the footer's own colour, so the section's bottom is not one either. */}
          <div className="absolute inset-0 bg-[linear-gradient(180deg,rgba(11,42,102,0)_0%,rgba(11,42,102,0.96)_11%,rgba(6,20,64,0.98)_45%,#020514_100%)]" />
          {/* Two wave bands at its top edge, drifting at different speeds and in opposite directions.
              Each fades out by the band's bottom, so nothing is cut by a straight line. */}
          <div className="absolute -top-[9%] left-0 h-[22%] w-full overflow-hidden">
            <svg viewBox="0 0 1520 40" preserveAspectRatio="none" className="lk-wave-slow absolute top-0 left-0 h-full w-[200%]" style={{ animationDuration: "22s" }}>
              <defs>
                <linearGradient id="lk-final-wave-mid" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="#24519f" stopOpacity="0.55" />
                  <stop offset="35%" stopColor="#0f2f74" stopOpacity="0.95" />
                  <stop offset="100%" stopColor="#0b2a66" stopOpacity="0" />
                </linearGradient>
              </defs>
              <path d={WAVE_MID} fill="url(#lk-final-wave-mid)" />
            </svg>
            <svg
              viewBox="0 0 1520 40"
              preserveAspectRatio="none"
              className="lk-wave-fast absolute top-[34%] left-0 h-[66%] w-[200%]"
              style={{ animationDuration: "15s", animationDirection: "reverse" }}
            >
              <defs>
                <linearGradient id="lk-final-wave-front" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="#3a6cc0" stopOpacity="0.32" />
                  <stop offset="30%" stopColor="#0c2864" stopOpacity="0.96" />
                  <stop offset="100%" stopColor="#0b2a66" stopOpacity="0" />
                </linearGradient>
              </defs>
              <path d={WAVE_FRONT} fill="url(#lk-final-wave-front)" />
            </svg>
          </div>

          {/* The whale's reflection, broken by the waves. */}
          <div className="absolute top-[3%] left-1/2 h-full w-[min(32%,250px)] -translate-x-1/2">
            {REFLECTION.map((r, i) => (
              <span
                key={i}
                className="lk-twinkle absolute h-[3px] -translate-x-1/2 rounded-full bg-[linear-gradient(90deg,transparent,rgba(122,170,240,0.5),transparent)]"
                style={{ top: `${r.top}%`, left: `${50 + r.x}%`, width: `${r.w}%`, animationDelay: `${i * 0.6}s`, animationDuration: "4.2s" }}
              />
            ))}
          </div>

          {/* The moon's glitter path, under the section's moon. */}
          <div className="absolute top-0 left-[13%] h-full">
            {GLITTER.map((g, i) => (
              <span
                key={i}
                className="lk-twinkle absolute h-[2px] -translate-x-1/2 rounded-full bg-[linear-gradient(90deg,transparent,rgba(232,244,255,0.85),transparent)]"
                style={{ top: `${g.top}%`, left: `${g.left}vw`, width: g.width, animationDelay: `${g.delay}s`, animationDuration: `${g.dur}s` }}
              />
            ))}
          </div>
        </div>

        {/* Rings at the waterline: one slow, gentle ring always, and a splash when it settles. */}
        <div aria-hidden="true" className="pointer-events-none absolute top-[66.4%] left-1/2 h-0 w-[44%] -translate-x-1/2">
          {/* Centred in its base style too (the keyframes centre it while they run); still frames have no ring. */}
          <span
            className="lk-ring absolute top-0 left-1/2 block aspect-[5/1] w-full rounded-[50%] border border-[#7fb8f5]/30 motion-reduce:hidden"
            style={{ transform: "translate(-50%, -50%)" }}
          />
          <span className="lk-r-splash absolute top-0 left-1/2 block aspect-[5/1] w-[82%] rounded-[50%] border-2 border-[#cfe6ff]/60" />
          <span className="lk-r-splash lk-r-splash-b absolute top-0 left-1/2 block aspect-[5/1] w-[82%] rounded-[50%] border border-[#9fd0ff]/45" />
        </div>
      </div>
    </div>
  );
}
