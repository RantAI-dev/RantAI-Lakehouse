"use client";

import * as React from "react";
import { afterPaint } from "@/lib/after-paint";

/**
 * Task C (owner, 2026-11-07: "tetap tambahin background air air gitu, biar ga kosong kosong
 * banget"): the life of the water behind the content sections, on the one fixed layer of
 * `depth-backdrop.tsx`, never per section and never above anything (the layer is first in the
 * page, so every section paints over it).
 *
 * - Marine snow in two layers (small and slow, larger and quicker), a few bubbles rising at the
 *   screen's edges, and a few very faint light shafts that fade out with depth (`--lk-shaft`).
 * - Every few tens of seconds, far behind, a school of fish or the whale crosses at very low
 *   contrast: a still silhouette that moves, not an animation of its own parts.
 * - CSS transforms and opacity only; fixed positions and timings; fewer particles under 768 px.
 *   The backdrop writes `--lk-life` (0 outside the water sections, so the edges of the zone are
 *   a fade, never a cut) and `data-run`, which pauses every animation off screen and while the
 *   tab is hidden. Under reduced motion nothing moves: a still frame of snow and shafts.
 * - Mounted once the page is usable (`afterPaint`). No dependency, no canvas.
 */
const make = (n: number) =>
  Array.from({ length: n }, (_, i) => {
    const near = i % 3 === 0;
    return {
      left: (3 + ((i * 37) % 94)).toFixed(1),
      top: ((i * 29) % 100).toFixed(1),
      size: near ? 2.5 : 1.5,
      alpha: near ? 0.32 : 0.2,
      dur: near ? 18 + ((i * 7) % 6) : 30 + ((i * 11) % 10),
      delay: -((i * 3.7) % 30),
    };
  });
const BUBBLES = Array.from({ length: 12 }, (_, i) => ({
  left: i % 2 === 0 ? 2 + ((i * 7) % 11) : 87 + ((i * 5) % 11),
  size: 3 + ((i * 3) % 6),
  dur: 14 + ((i * 7) % 8),
  delay: -((i * 13) % 19),
}));
const SHAFTS = [
  { left: 8, w: 15, rot: 14, a: 0.05 },
  { left: 34, w: 10, rot: 12, a: 0.04 },
  { left: 62, w: 17, rot: 15, a: 0.045 },
  { left: 84, w: 9, rot: 13, a: 0.035 },
];
// A small school: a loose arrowhead of tiny fish.
const FISH = [
  [0, 0], [-14, -7], [-14, 7], [-28, -14], [-28, 0], [-28, 14], [-42, -7], [-42, 7],
] as const;

const CSS = `
.lk-life { opacity: var(--lk-life); }
.lk-life-snow { animation: lk-life-snow linear infinite; }
.lk-life-bubble { animation: lk-life-bubble linear infinite; }
.lk-life-shafts { opacity: var(--lk-shaft, 0); mask-image: linear-gradient(180deg, #000 0%, transparent 85%); }
.lk-life-shaft { position: absolute; top: -10%; height: 90%; transform-origin: top center;
  background: linear-gradient(180deg, rgba(150,200,255,1), rgba(150,200,255,0)); animation: lk-life-sway ease-in-out infinite alternate; }
.lk-life-far { position: absolute; opacity: 0; animation: lk-life-pass linear infinite; will-change: transform; }
.lk-water[data-run="0"] .lk-life * { animation-play-state: paused; }
@keyframes lk-life-snow {
  0% { transform: translate3d(0, -6vh, 0); }
  100% { transform: translate3d(1.5vw, 106vh, 0); }
}
@keyframes lk-life-bubble {
  0% { transform: translate3d(0, 8vh, 0); opacity: 0; }
  12% { opacity: 0.7; }
  50% { transform: translate3d(5px, -48vh, 0); }
  86% { opacity: 0.45; }
  100% { transform: translate3d(-4px, -104vh, 0); opacity: 0; }
}
@keyframes lk-life-sway {
  from { transform: rotate(calc(var(--r) - 2deg)); }
  to { transform: rotate(calc(var(--r) + 2deg)); }
}
/* Crossing in the first third of a long loop, then nothing for a while. */
@keyframes lk-life-pass {
  0% { transform: translate3d(-30vw, 0, 0); opacity: 0; }
  4% { opacity: 1; }
  30% { opacity: 1; }
  34% { transform: translate3d(125vw, -3vh, 0); opacity: 0; }
  100% { transform: translate3d(125vw, -3vh, 0); opacity: 0; }
}
@media (prefers-reduced-motion: reduce) {
  .lk-life *, .lk-life-shaft { animation: none !important; }
  .lk-life-bubble, .lk-life-far { display: none; }
  .lk-life-snow { transform: translate3d(0, 40vh, 0); }
}
`;

export function WaterLife() {
  const [n, setN] = React.useState<{ snow: number; bubbles: number } | null>(null);
  React.useEffect(
    () => afterPaint(() => setN(window.innerWidth < 768 ? { snow: 14, bubbles: 5 } : { snow: 30, bubbles: 12 })),
    [],
  );
  if (!n) return null;
  const snow = make(n.snow);
  return (
    <div className="lk-life absolute inset-0">
      <style dangerouslySetInnerHTML={{ __html: CSS }} />
      <div className="lk-life-shafts absolute inset-0">
        {SHAFTS.map((s, i) => (
          <span
            key={i}
            className="lk-life-shaft"
            style={
              {
                left: `${s.left}%`,
                width: `${s.w}vw`,
                opacity: s.a * 3,
                "--r": `${s.rot}deg`,
                transform: `rotate(${s.rot}deg)`,
                animationDuration: `${26 + i * 7}s`,
                animationDelay: `${-i * 5}s`,
                filter: "blur(18px)",
              } as React.CSSProperties
            }
          />
        ))}
      </div>
      {snow.map((p, i) => (
        <span
          key={`s${i}`}
          className="lk-life-snow absolute -top-[6vh] rounded-full bg-[#cfe6ff]"
          style={{ left: `${p.left}%`, width: p.size, height: p.size, opacity: p.alpha, animationDuration: `${p.dur}s`, animationDelay: `${p.delay}s` }}
        />
      ))}
      {BUBBLES.slice(0, n.bubbles).map((b, i) => (
        <span
          key={`b${i}`}
          className="lk-life-bubble absolute bottom-0 rounded-full border border-[#E8F4FF]/35 bg-[#E8F4FF]/5"
          style={{ left: `${b.left}%`, width: b.size, height: b.size, animationDuration: `${b.dur}s`, animationDelay: `${b.delay}s` }}
        />
      ))}
      {/* Far behind, very low contrast: a school of fish, and later the whale. */}
      <svg className="lk-life-far" style={{ top: "34%", left: 0, width: 60, height: 40, animationDuration: "52s", animationDelay: "-6s", color: "#8fb8e8" }} viewBox="-50 -20 60 40" aria-hidden="true">
        <g fillOpacity="0.1" fill="currentColor">
          {FISH.map(([x, y], i) => (
            <path key={i} d={`M${x} ${y} l-6 -2.5 l0 5 z M${x} ${y} m0 -1.6 a3 1.6 0 1 0 0.01 0`} />
          ))}
        </g>
      </svg>
      <svg className="lk-life-far" style={{ top: "62%", left: 0, width: 220, height: 90, animationDuration: "78s", animationDelay: "-40s", color: "#8fb8e8" }} viewBox="0 0 220 90" aria-hidden="true">
        <path fill="currentColor" fillOpacity="0.07" d="M8 52 C30 24 90 14 140 30 C168 38 184 48 214 38 C206 56 196 66 174 66 C150 80 100 84 58 76 C30 72 14 64 8 52 Z" />
      </svg>
    </div>
  );
}
