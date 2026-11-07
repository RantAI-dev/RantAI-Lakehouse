import * as React from "react";
import { SwimLayer } from "./whale-motion";
import { WHALE_VIEW, WhaleSvg } from "./whale-svg";

/**
 * The final call's creature, at the bottom of the D14 journey.
 *
 * WEB-5 O33 (owner: "makin bawah makin gelap aja"): the page is one descent,
 * and the final call is its deepest point, not a return to the surface. The
 * whale (O29's character, `whale-svg.tsx`) swims down to greet the visitor
 * there, as the guide, and settles below the call to action. This replaces
 * O24's surfacing over the waves and the D14 S5 rising mark.
 * - A soft bioluminescent glow around the whale and a faint shaft of light
 *   from far above keep the deep readable and warm; both are local, so the
 *   water around them stays as dark as the page's own end colour.
 * - Sparse marine snow drifts down past it.
 * - The whale sits in its own band below the headline and the buttons
 *   (`final-mascot.tsx` places the stage there), and swims in from the
 *   right inside that band (`SwimLayer`), so it never passes over the text.
 *
 * A server component: the art is static SVG and CSS. The glow's breathing
 * and the snow are scoped keyframes, off under reduced motion, which leaves
 * the still final frame.
 */

const SCALE = 0.92;
const WHALE_BOX = {
  left: (380 - (WHALE_VIEW.w * SCALE) / 2) / 760,
  top: 0.02,
  width: (WHALE_VIEW.w * SCALE) / 760,
  height: (WHALE_VIEW.h * SCALE) / 560,
};
const pct = (x: number) => `${(x * 100).toFixed(3)}%`;

/** Marine snow: deterministic specks (the server and the browser must agree). */
const SNOW = Array.from({ length: 16 }, (_, i) => ({
  left: (7 + ((i * 37) % 86)).toFixed(1),
  top: ((i * 23) % 80).toFixed(1),
  size: i % 5 === 0 ? 3 : 2,
  dur: (14 + ((i * 7) % 9)).toFixed(1),
  delay: (-((i * 5.3) % 18)).toFixed(1),
}));

const SCENE_CSS = `
@media (prefers-reduced-motion: no-preference) {
  .lk-f-glow { animation: lk-f-breathe 6s ease-in-out infinite; }
  .lk-f-snow { animation: lk-f-snow linear infinite; }
  [data-arrived="on"] .lk-f-hello { animation: lk-f-hello 1.4s cubic-bezier(0.3, 0.7, 0.4, 1) both; }
}
.lk-f-hello { opacity: 0; }
@keyframes lk-f-breathe { 0%, 100% { opacity: 0.85; } 50% { opacity: 1; } }
@keyframes lk-f-snow {
  0% { transform: translate3d(0, -20px, 0); opacity: 0; }
  12% { opacity: 0.55; }
  88% { opacity: 0.55; }
  100% { transform: translate3d(10px, 120px, 0); opacity: 0; }
}
@keyframes lk-f-hello { 0% { opacity: 0; } 35% { opacity: 1; } 100% { opacity: 0; } }
`;

export function MascotRising({ className }: { className?: string }) {
  return (
    <div className={className}>
      <style dangerouslySetInnerHTML={{ __html: SCENE_CSS }} />
      <div className="relative aspect-[760/560] w-full">
        {/* A faint shaft of light from far above, narrowing as it falls. */}
        <div
          aria-hidden="true"
          // Faded on all four sides (a vertical fade, masked across), so it has no edge.
          className="pointer-events-none absolute -top-[40%] left-1/2 h-[120%] w-[56%] -translate-x-1/2 bg-[linear-gradient(180deg,transparent_0%,rgba(68,166,250,0.09)_35%,rgba(68,166,250,0.04)_70%,transparent_100%)] [mask-image:linear-gradient(90deg,transparent_0%,#000_50%,transparent_100%)]"
        />
        {/* Marine snow. */}
        <div aria-hidden="true" className="pointer-events-none absolute inset-0">
          {SNOW.map((s, i) => (
            <span
              key={i}
              className="lk-f-snow absolute rounded-full bg-[#cfe6ff]/50"
              style={{ left: `${s.left}%`, top: `${s.top}%`, width: s.size, height: s.size, animationDuration: `${s.dur}s`, animationDelay: `${s.delay}s` }}
            />
          ))}
        </div>

        <SwimLayer className="absolute inset-0">
          {/* The whale's own soft light: bioluminescence around it, breathing. */}
          <div
            aria-hidden="true"
            className="lk-f-glow pointer-events-none absolute top-[6%] left-1/2 aspect-square w-[78%] -translate-x-1/2 rounded-full bg-[radial-gradient(circle,rgba(68,166,250,0.2)_0%,rgba(68,166,250,0.08)_38%,transparent_68%)]"
          />
          {/* A brief brighter ring as it arrives: its hello. */}
          <div
            aria-hidden="true"
            className="lk-f-hello pointer-events-none absolute top-[10%] left-1/2 aspect-square w-[64%] -translate-x-1/2 rounded-full bg-[radial-gradient(circle,transparent_52%,rgba(159,208,255,0.22)_62%,transparent_72%)]"
          />
          <div
            className="lk-bob absolute"
            style={{ left: pct(WHALE_BOX.left), top: pct(WHALE_BOX.top), width: pct(WHALE_BOX.width), height: pct(WHALE_BOX.height) }}
          >
            <WhaleSvg className="size-full drop-shadow-[0_0_40px_rgba(68,166,250,0.28)]" />
          </div>
        </SwimLayer>
      </div>
    </div>
  );
}
