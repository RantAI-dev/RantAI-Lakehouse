import * as React from "react";

/**
 * The three drawn dashboard tiles of S5 (the dot map and the calendar are
 * `dashboard-tiles-static.tsx`, WEB-5 review fix R6), as SVG with made-up,
 * deterministic numbers; the section says "Illustrations, not customer
 * data." Colours stay in the page palette: lake blue and foam on glass.
 *
 * WEB-5 owner QA O10: they move like a live dashboard, all in CSS (the
 * classes below are styled in app/globals.css, "the living tiles"), started
 * and paused by `live-tile.tsx`. Server components now: nothing here runs
 * in the browser. Every animated path has `pathLength="1"`, so one dash
 * rule draws any of them.
 */

/** The words a tile draws itself: its accessible name, and the KPI line. */
export type TileText = { aria: string; delta?: string };

const LAKE = "#44A6FA";
const FOAM = "#E8F4FF";

const svgProps = {
  viewBox: "0 0 360 190",
  className: "h-auto w-full overflow-visible",
  role: "img" as const,
};

/** The line's points, for the hover labels: [x, y, value]. */
const LINE_POINTS = [
  [95, 100, "12.1k"],
  [185, 95, "13.4k"],
  [285, 62, "17.8k"],
  [350, 24, "21.3k"],
] as const;

export function LineChartTile({ tile }: { tile: TileText }) {
  const a = "M10 150 C45 140 60 90 95 100 S150 140 185 95 S250 40 285 62 S335 30 350 24";
  const b = "M10 160 C50 150 70 135 105 138 S160 150 195 122 S255 100 290 106 S335 84 350 80";
  return (
    <svg {...svgProps} aria-label={tile.aria}>
      <defs>
        <linearGradient id="lk-area" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={LAKE} stopOpacity="0.35" />
          <stop offset="100%" stopColor={LAKE} stopOpacity="0" />
        </linearGradient>
      </defs>
      {[40, 80, 120, 160].map((y) => (
        <line key={y} x1="10" x2="350" y1={y} y2={y} stroke={FOAM} strokeOpacity="0.08" />
      ))}
      <path className="lk-fade" style={{ animationDelay: "0.5s" }} d={`${a} L350 180 L10 180 Z`} fill="url(#lk-area)" />
      <path className="lk-draw" pathLength={1} d={a} fill="none" stroke={LAKE} strokeWidth="2.5" strokeLinecap="round" />
      <path className="lk-fade" style={{ animationDelay: "0.3s" }} d={b} fill="none" stroke={FOAM} strokeOpacity="0.55" strokeWidth="1.5" strokeLinecap="round" strokeDasharray="1 5" />
      {/* The newest point: it lands when the line arrives, breathes, and now and then nudges up as if data came in. */}
      <g className="lk-nudge">
        <circle className="lk-breathe" cx="350" cy="24" r="9" fill={LAKE} fillOpacity="0.25" />
        <circle className="lk-land" cx="350" cy="24" r="4" fill={LAKE} />
      </g>
      {/* Hover or focus: the value at each point. Pointer-sized zones over the chart, one label each. */}
      {LINE_POINTS.map(([x, y, v], i) => (
        <g key={x} className="lk-point" data-last={i === LINE_POINTS.length - 1 ? "" : undefined}>
          <rect x={x - 45} y="0" width="90" height="190" fill="transparent" />
          <g className="lk-point-label">
            <circle cx={x} cy={y} r="3.5" fill={FOAM} />
            <rect x={x - 26} y={y - 34} width="52" height="22" rx="11" fill="#020514" fillOpacity="0.9" stroke={FOAM} strokeOpacity="0.25" />
            <text x={x} y={y - 19} textAnchor="middle" fill={FOAM} fontSize="12" fontWeight="500" fontFamily="var(--font-sans), sans-serif" style={{ fontVariantNumeric: "tabular-nums" }}>
              {v}
            </text>
          </g>
        </g>
      ))}
    </svg>
  );
}

export function SankeyTile({ tile }: { tile: TileText }) {
  const left = [
    { y: 28, h: 50 },
    { y: 92, h: 38 },
    { y: 142, h: 30 },
  ];
  const right = [
    { y: 24, h: 62 },
    { y: 104, h: 34 },
    { y: 152, h: 22 },
  ];
  const flows = [
    { from: 0, to: 0, w: 40 },
    { from: 0, to: 1, w: 10 },
    { from: 1, to: 0, w: 22 },
    { from: 1, to: 1, w: 16 },
    { from: 2, to: 1, w: 8 },
    { from: 2, to: 2, w: 22 },
  ];
  const usedL = [0, 0, 0];
  const usedR = [0, 0, 0];
  const paths = flows.map((f) => {
    const y0 = left[f.from].y + usedL[f.from] + f.w / 2;
    const y1 = right[f.to].y + usedR[f.to] + f.w / 2;
    usedL[f.from] += f.w * 0.9;
    usedR[f.to] += f.w * 0.9;
    return { d: `M44 ${y0} C170 ${y0} 190 ${y1} 316 ${y1}`, w: f.w * 0.9 };
  });
  return (
    <svg {...svgProps} aria-label={tile.aria}>
      {paths.map((p, i) => (
        <path
          key={i}
          className="lk-draw"
          style={{ animationDelay: `${i * 0.08}s` }}
          pathLength={1}
          d={p.d}
          fill="none"
          stroke={i % 2 ? FOAM : LAKE}
          strokeOpacity={i % 2 ? 0.3 : 0.5}
          strokeWidth={p.w}
        />
      ))}
      {/* Idle: slow light travelling along the bands, left to right. */}
      {paths.map((p, i) => (
        <path
          key={`f${i}`}
          className="lk-flow"
          style={{ animationDelay: `${-i * 1.3}s` }}
          pathLength={1}
          d={p.d}
          fill="none"
          stroke={FOAM}
          strokeOpacity="0.3"
          strokeWidth="2"
          strokeLinecap="round"
        />
      ))}
      {left.map((n, i) => (
        <rect key={`l${i}`} x="36" y={n.y} width="8" height={n.h} rx="3" fill={FOAM} fillOpacity="0.85" />
      ))}
      {right.map((n, i) => (
        <rect key={`r${i}`} className="lk-fade" style={{ animationDelay: "0.7s" }} x="316" y={n.y} width="8" height={n.h} rx="3" fill={FOAM} fillOpacity="0.85" />
      ))}
    </svg>
  );
}

/** One rolling digit: a column of 0 to 9, shown at `d`; it rolls up from 0 on entrance. */
function Digit({ d, idle }: { d: number; idle?: boolean }) {
  return (
    <span className="lk-digit inline-block h-[1em] overflow-hidden align-top">
      <span className={idle ? "lk-roll lk-tick block" : "lk-roll block"} style={{ "--lk-d": d } as React.CSSProperties}>
        {Array.from({ length: 10 }, (_, i) => (
          <span key={i} className="block h-[1em] leading-none">
            {i}
          </span>
        ))}
      </span>
    </span>
  );
}

export function KpiTile({ tile }: { tile: TileText }) {
  const pts = [22, 30, 26, 38, 34, 46, 42, 58, 52, 66, 62, 78];
  const d = pts.map((v, i) => `${i === 0 ? "M" : "L"}${20 + i * 28} ${150 - v}`).join(" ");
  return (
    <div className="relative [container-type:inline-size]">
      <svg {...svgProps} aria-label={tile.aria}>
        <g className="lk-fade" style={{ animationDelay: "0.9s" }}>
          {/* The trend arrow appears once the number has rolled up. */}
          <path d="M22 81 L29 74 M24 74 H29 V79" fill="none" stroke={LAKE} strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
          <text x="36" y="86" fill={LAKE} fontSize="14" fontFamily="var(--font-sans), sans-serif">
            {tile.delta}
          </text>
        </g>
        <path className="lk-draw" style={{ animationDelay: "0.2s" }} pathLength={1} d={d} fill="none" stroke={LAKE} strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
        <circle className="lk-land" style={{ animationDelay: "1.2s" }} cx={20 + 11 * 28} cy={150 - 78} r="4" fill={LAKE} />
      </svg>
      {/* The number, as HTML over the drawing so its digits can roll (the old SVG text, same place and size: 54 of 360). */}
      <p
        aria-hidden="true"
        className="pointer-events-none absolute top-[9.5%] left-[5.5%] flex text-[15cqw] leading-none font-medium tracking-[-0.04em] text-[#E8F4FF] tabular-nums"
      >
        <Digit d={2} />
        <Digit d={4} />
        <span>.</span>
        <Digit d={6} idle />
        <span>k</span>
      </p>
    </div>
  );
}
