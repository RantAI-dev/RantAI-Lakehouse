import * as React from "react";

/**
 * By day only (`html[data-tod="day"]`, app/globals.css; at night it is not
 * rendered by CSS, and the page keeps its dark feather into the night sea).
 * The way up from the FAQ's deep water to the final call's day sky, about 1.15
 * screens long, made of soft layers so no part of it reads as a band:
 * - the water lightens by eased steps, from the abyss to a sunlit teal;
 * - sun shafts come down from above and strengthen as you rise;
 * - marine snow thins out and is gone before the light is strong;
 * - last, the underside of the surface: bright, with a wavy, rippling lower
 *   edge, blurred so it never has a line. It ends on the colour the final
 *   call's sky starts from (`#c9e6f2`), which the final's top haze takes over.
 * Pure CSS and one inline SVG: no canvas, so it works on the DOM path too.
 * Reduced motion keeps every layer, still.
 */
const SHAFTS = [
  { l: 4, w: 9, d: 11, o: 0.8 },
  { l: 17, w: 6, d: 9, o: 0.6 },
  { l: 30, w: 12, d: 13, o: 0.9 },
  { l: 47, w: 7, d: 10, o: 0.65 },
  { l: 60, w: 11, d: 12, o: 0.85 },
  { l: 76, w: 6, d: 9.5, o: 0.6 },
  { l: 88, w: 10, d: 11.5, o: 0.75 },
];

const WAVE_A =
  "M0 150 C120 112 240 188 360 150 S600 112 720 150 S960 188 1080 150 S1320 112 1440 150 S1560 188 1680 150 V400 H0Z";
const WAVE_B =
  "M0 176 C90 150 210 206 330 176 S570 146 690 176 S930 206 1050 176 S1290 146 1410 176 S1530 206 1680 176 V400 H0Z";

export function SurfaceRise() {
  return (
    <div id="surface" aria-hidden="true" className="lk-surf pointer-events-none relative z-[1] overflow-hidden">
      <div className="lk-surf-water absolute inset-0" />
      <div className="lk-surf-snow absolute inset-0" />
      <div className="lk-surf-shafts absolute inset-0">
        {SHAFTS.map((s) => (
          <span
            key={s.l}
            className="lk-shaft absolute"
            style={{ left: `${s.l}%`, width: `${s.w}vw`, opacity: s.o, animationDuration: `${s.d}s` }}
          />
        ))}
      </div>
      <svg
        viewBox="0 0 1440 400"
        preserveAspectRatio="none"
        className="absolute inset-x-[-8%] bottom-0 h-[40%] w-[116%]"
      >
        <defs>
          <linearGradient id="lk-surf-g" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0.2" stopColor="#dff3fa" stopOpacity="0" />
            <stop offset="0.42" stopColor="#d6eef7" stopOpacity="0.55" />
            <stop offset="0.7" stopColor="#c9e6f2" stopOpacity="1" />
          </linearGradient>
          <linearGradient id="lk-surf-g2" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0.3" stopColor="#ffffff" stopOpacity="0" />
            <stop offset="0.55" stopColor="#ffffff" stopOpacity="0.5" />
            <stop offset="0.8" stopColor="#ffffff" stopOpacity="0.0" />
          </linearGradient>
          <filter id="lk-surf-b" x="-5%" y="-30%" width="110%" height="160%">
            <feGaussianBlur stdDeviation="7" />
          </filter>
          <filter id="lk-surf-b2" x="-5%" y="-30%" width="110%" height="160%">
            <feGaussianBlur stdDeviation="3" />
          </filter>
        </defs>
        <g className="lk-wave lk-wave-a" filter="url(#lk-surf-b)">
          <path d={WAVE_B} fill="url(#lk-surf-g2)" />
        </g>
        <g className="lk-wave lk-wave-b" filter="url(#lk-surf-b)">
          <path d={WAVE_A} fill="url(#lk-surf-g)" />
        </g>
        <g className="lk-wave lk-wave-a" filter="url(#lk-surf-b2)">
          <path d={WAVE_A.replace(" V400 H0Z", "")} fill="none" stroke="#fff" strokeOpacity="0.5" strokeWidth="3" />
          <path d={WAVE_B.replace(" V400 H0Z", "")} fill="none" stroke="#fff" strokeOpacity="0.32" strokeWidth="2" />
        </g>
      </svg>
    </div>
  );
}
