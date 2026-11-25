import * as React from "react";
import type { HomeCopy } from "@/lib/copy";

/**
 * The three drawn dashboard tiles of S5 (the dot map and the calendar are
 * `dashboard-tiles-static.tsx`, WEB-5 review fix R6), as SVG with made-up,
 * deterministic numbers; each tile's accessible name says it is
 * illustrative (O32 removed the visible caption). Colours stay in the page
 * palette: lake blue and foam on glass.
 *
 * WEB-5 owner QA O10: they move like a live dashboard, all in CSS ("the
 * living tiles" in app/globals.css), started and paused by `live-tile.tsx`.
 * Every animated path has `pathLength="1"`, so one dash rule draws any of
 * them.
 *
 * WEB-5 owner QA O18: each one answers the pointer, in CSS only (".lk-hov"
 * rules in app/globals.css), so nothing here runs in the browser:
 * - line chart: hover zones split the width at the midpoints between
 *   points, so the crosshair snaps to the nearest point, which grows, with
 *   its value and month;
 * - sankey: a band or a node lights its flows end to end and dims the
 *   rest, with "Source → Target: n";
 * - KPI: hover zones over the sparkline scrub it, and the rolling digits
 *   follow the hovered day (rules in a small server-rendered <style>).
 * Tooltips are SVG or HTML in the site's glass; they still appear under
 * reduced motion, without the easing.
 */

type Tiles = HomeCopy["dashboards"]["tiles"];

/** The words a tile draws itself: its accessible name, and the KPI line. */
export type TileText = { aria: string; delta?: string };

const LAKE = "#44A6FA";
const FOAM = "#E8F4FF";
const GLASS = "#020514";

const svgProps = {
  viewBox: "0 0 360 190",
  className: "h-auto w-full overflow-visible",
  role: "img" as const,
};

/** The line's points on the curve (the path's segment ends), with their illustrative values. */
const LINE_POINTS = [
  { x: 10, y: 150, v: "8.2k" },
  { x: 95, y: 100, v: "12.1k" },
  { x: 185, y: 95, v: "13.4k" },
  { x: 285, y: 62, v: "17.8k" },
  { x: 350, y: 24, v: "21.3k" },
] as const;

/** A two-line glass tooltip in SVG, kept inside the tile: it flips to the left near the right edge. */
function SvgTip({ x, y, top, bottom, width = 68 }: { x: number; y: number; top: string; bottom: string; width?: number }) {
  const left = x + width + 10 > 360 ? x - width - 10 : x + 10;
  const ty = Math.max(4, Math.min(190 - 40, y - 46));
  return (
    <g className="lk-hov-tip">
      <rect x={left} y={ty} width={width} height="36" rx="10" fill={GLASS} fillOpacity="0.92" stroke={FOAM} strokeOpacity="0.22" />
      <text x={left + 10} y={ty + 16} fill={FOAM} fontSize="12" fontWeight="600" fontFamily="var(--font-sans), sans-serif" style={{ fontVariantNumeric: "tabular-nums" }}>
        {top}
      </text>
      <text x={left + 10} y={ty + 29} fill={FOAM} fillOpacity="0.65" fontSize="10" fontFamily="var(--font-sans), sans-serif">
        {bottom}
      </text>
    </g>
  );
}

export function LineChartTile({ tile }: { tile: Tiles["line"] }) {
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
      {/* O18: one zone per point, split at the midpoints; hovering a zone snaps the crosshair to its point. */}
      {LINE_POINTS.map((p, i) => {
        const x0 = i === 0 ? 0 : (LINE_POINTS[i - 1].x + p.x) / 2;
        const x1 = i === LINE_POINTS.length - 1 ? 360 : (p.x + LINE_POINTS[i + 1].x) / 2;
        return (
          <g key={p.x} className="lk-hov-zone" data-last={i === LINE_POINTS.length - 1 ? "" : undefined}>
            <rect x={x0} y="0" width={x1 - x0} height="190" fill="transparent" />
            <g className="lk-hov-show">
              <line x1={p.x} x2={p.x} y1="8" y2="182" stroke={FOAM} strokeOpacity="0.35" strokeWidth="1" />
              <circle className="lk-hov-grow" cx={p.x} cy={p.y} r="5.5" fill={FOAM} stroke={LAKE} strokeWidth="2" />
              <SvgTip x={p.x} y={p.y} top={p.v} bottom={tile.points[i]} />
            </g>
          </g>
        );
      })}
    </svg>
  );
}

/** Audit #23: the nodes sit inward, so each side has room for its labels. */
const SK_DESKTOP = { l: 76, r: 276, font: 11, bend: 110 };
/**
 * WEB-5 M10 (phone pass): a phone's tile draws the drawing at ~300 px, so 11-unit labels came out
 * at 8-10 px. Its own geometry: the nodes further in, so labels of 15 units (12 px or more at
 * 360 px) fit on both sides, "Media sosial" and "Dokumentasi" included.
 */
const SK_PHONE = { l: 104, r: 256, font: 15, bend: 72 };

export function SankeyTile({ tile }: { tile: Tiles["sankey"] }) {
  return (
    <>
      <SankeyDrawing tile={tile} geo={SK_DESKTOP} className="max-lg:hidden" interactive />
      <SankeyDrawing tile={tile} geo={SK_PHONE} className="lg:hidden" />
    </>
  );
}

function SankeyDrawing({
  tile,
  geo,
  className,
  interactive = false,
}: {
  tile: Tiles["sankey"];
  geo: typeof SK_DESKTOP;
  className: string;
  /**
   * The pointer's part (O18): the hover rules (global, so written once), the node hit areas and
   * the tooltips. Only the desktop drawing has it: a phone has no pointer to follow, and the
   * page's HTML stays smaller (the phone drawing is server-rendered twice, as HTML and as data).
   */
  interactive?: boolean;
}) {
  const { l: SK_L, r: SK_R, font, bend } = geo;
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
    return { ...f, y0, y1, d: `M${SK_L + 8} ${y0} C${SK_L + bend} ${y0} ${SK_R - bend} ${y1} ${SK_R} ${y1}`, sw: f.w * 0.9 };
  });
  /** Illustrative counts: a band's width in tenths of thousands. */
  const count = (w: number) => `${(w / 10).toFixed(1)}k`;
  const totalL = [0, 1, 2].map((n) => flows.filter((f) => f.from === n).reduce((s, f) => s + f.w, 0));
  const totalR = [0, 1, 2].map((n) => flows.filter((f) => f.to === n).reduce((s, f) => s + f.w, 0));
  // O18: a hovered band lights its tooltip; a hovered node lights its flows and its own tooltip.
  const rules = [
    ...paths.map((_, i) => `.lk-sankey:has(.lk-flowgrp[data-i="${i}"]:hover) .lk-flowtip[data-i="${i}"]{opacity:1}`),
    ...[0, 1, 2].map(
      (n) =>
        `.lk-sankey:has(.lk-node[data-l="${n}"]:hover) .lk-flowgrp:not([data-from="${n}"]){opacity:.18}` +
        `.lk-sankey:has(.lk-node[data-l="${n}"]:hover) .lk-nodetip[data-l="${n}"]{opacity:1}` +
        `.lk-sankey:has(.lk-node[data-r="${n}"]:hover) .lk-flowgrp:not([data-to="${n}"]){opacity:.18}` +
        `.lk-sankey:has(.lk-node[data-r="${n}"]:hover) .lk-nodetip[data-r="${n}"]{opacity:1}`,
    ),
  ].join("");
  return (
    <svg {...svgProps} aria-label={tile.aria} className={`${svgProps.className} lk-sankey ${className}`}>
      {interactive && <style>{rules}</style>}
      {paths.map((p, i) => (
        <g key={i} className="lk-flowgrp" data-i={i} data-from={p.from} data-to={p.to}>
          <path
            className="lk-draw lk-band"
            style={{ animationDelay: `${i * 0.08}s` }}
            pathLength={1}
            d={p.d}
            fill="none"
            stroke={i % 2 ? FOAM : LAKE}
            strokeOpacity={i % 2 ? 0.3 : 0.5}
            strokeWidth={p.sw}
          />
        </g>
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
          pointerEvents="none"
        />
      ))}
      {interactive &&
        left.map((n, i) => (
          <rect key={`l${i}`} className="lk-node" data-l={i} x={SK_L - 4} y={n.y} width="16" height={n.h} rx="3" fill="transparent" />
        ))}
      {interactive &&
        right.map((n, i) => (
          <rect key={`r${i}`} className="lk-node" data-r={i} x={SK_R - 4} y={n.y} width="16" height={n.h} rx="3" fill="transparent" />
        ))}
      {left.map((n, i) => (
        <rect key={`lv${i}`} x={SK_L} y={n.y} width="8" height={n.h} rx="3" fill={FOAM} fillOpacity="0.85" pointerEvents="none" />
      ))}
      {right.map((n, i) => (
        <rect key={`rv${i}`} className="lk-fade" style={{ animationDelay: "0.7s" }} x={SK_R} y={n.y} width="8" height={n.h} rx="3" fill={FOAM} fillOpacity="0.85" pointerEvents="none" />
      ))}
      {/* Audit #23: every node is named, sources on the left, pages on the right. */}
      {left.map((n, i) => (
        <text key={`ln${i}`} x={SK_L - 8} y={n.y + n.h / 2} textAnchor="end" dominantBaseline="central" fontSize={font} fill={FOAM} fillOpacity="0.72" pointerEvents="none">
          {tile.from[i]}
        </text>
      ))}
      {right.map((n, i) => (
        <text key={`rn${i}`} className="lk-fade" style={{ animationDelay: "0.7s" }} x={SK_R + 16} y={n.y + n.h / 2} dominantBaseline="central" fontSize={font} fill={FOAM} fillOpacity="0.72" pointerEvents="none">
          {tile.to[i]}
        </text>
      ))}
      {/* Tooltips last, so they sit above every band; shown by the hovered band or node. */}
      {interactive && paths.map((p, i) => (
        <g key={`t${i}`} className="lk-flowtip" data-i={i}>
          <SvgTip x={(SK_L + SK_R) / 2} y={(p.y0 + p.y1) / 2} top={`${tile.from[p.from]} → ${tile.to[p.to]}`} bottom={count(p.w)} width={128} />
        </g>
      ))}
      {interactive && left.map((n, i) => (
        <g key={`tl${i}`} className="lk-nodetip" data-l={i}>
          <SvgTip x={SK_L + 16} y={n.y + n.h / 2 + 20} top={tile.from[i]} bottom={`${count(totalL[i])} ${tile.total}`} width={96} />
        </g>
      ))}
      {interactive && right.map((n, i) => (
        <g key={`tr${i}`} className="lk-nodetip" data-r={i}>
          <SvgTip x={SK_R - 8} y={n.y + n.h / 2 + 20} top={tile.to[i]} bottom={`${count(totalR[i])} ${tile.total}`} width={96} />
        </g>
      ))}
    </svg>
  );
}

/** One rolling digit: a column of 0 to 9, shown at `d`; it rolls up from 0 on entrance. `slot` lets a hover pick another digit. */
function Digit({ d, slot, idle }: { d: number; slot: "a" | "b" | "c"; idle?: boolean }) {
  return (
    <span className="lk-digit inline-block h-[1em] overflow-hidden align-top">
      <span className={idle ? "lk-roll lk-tick block" : "lk-roll block"} style={{ "--lk-d": `var(--lk-k${slot}, ${d})` } as React.CSSProperties}>
        {Array.from({ length: 10 }, (_, i) => (
          <span key={i} className="block h-[1em] leading-none">
            {i}
          </span>
        ))}
      </span>
    </span>
  );
}

const KPI_PTS = [22, 30, 26, 38, 34, 46, 42, 58, 52, 66, 62, 78];
/** Each day's value, from 12.0k to the week's 24.6k, as three digits (tens, ones, tenths). */
const KPI_DIGITS = KPI_PTS.map((v) => {
  const tenths = Math.round((12 + ((v - 22) / (78 - 22)) * 12.6) * 10);
  return [Math.floor(tenths / 100), Math.floor(tenths / 10) % 10, tenths % 10] as const;
});
/** O18: hovering day `i` sets the digits the number rolls to (`--lk-ka/kb/kc`). */
const KPI_RULES = KPI_DIGITS.map(
  ([a, b, c], i) => `.lk-kpi:has(.lk-kz[data-i="${i}"]:hover){--lk-ka:${a};--lk-kb:${b};--lk-kc:${c}}`,
).join("");

export function KpiTile({ tile }: { tile: Tiles["kpi"] }) {
  const x = (i: number) => 20 + i * 28;
  const y = (v: number) => 150 - v;
  const d = KPI_PTS.map((v, i) => `${i === 0 ? "M" : "L"}${x(i)} ${y(v)}`).join(" ");
  return (
    <div className="lk-kpi relative [container-type:inline-size]">
      <style>{KPI_RULES}</style>
      <svg {...svgProps} aria-label={tile.aria}>
        <g className="lk-fade" style={{ animationDelay: "0.9s" }}>
          {/* The trend arrow appears once the number has rolled up. */}
          <path d="M22 81 L29 74 M24 74 H29 V79" fill="none" stroke={LAKE} strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
          <text x="36" y="86" fill={LAKE} fontSize="14" fontFamily="var(--font-sans), sans-serif">
            {tile.delta}
          </text>
        </g>
        <path className="lk-draw" style={{ animationDelay: "0.2s" }} pathLength={1} d={d} fill="none" stroke={LAKE} strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
        <circle className="lk-land" style={{ animationDelay: "1.2s" }} cx={x(11)} cy={y(78)} r="4" fill={LAKE} />
        {/* O18: scrub zones, one per day, split at the midpoints. */}
        {KPI_PTS.map((v, i) => (
          <g key={i} className="lk-hov-zone lk-kz" data-i={i}>
            <rect x={i === 0 ? 0 : x(i) - 14} y="60" width={i === 0 ? x(0) + 14 : 28} height="130" fill="transparent" />
            <g className="lk-hov-show">
              <line x1={x(i)} x2={x(i)} y1="64" y2="182" stroke={FOAM} strokeOpacity="0.35" />
              <circle className="lk-hov-grow" cx={x(i)} cy={y(v)} r="5" fill={FOAM} stroke={LAKE} strokeWidth="2" />
            </g>
          </g>
        ))}
      </svg>
      {/* The number, as HTML over the drawing so its digits can roll (the old SVG text, same place and size: 54 of 360). */}
      <p
        aria-hidden="true"
        className="pointer-events-none absolute top-[9.5%] left-[5.5%] flex text-[15cqw] leading-none font-medium tracking-[-0.04em] text-[#E8F4FF] tabular-nums"
      >
        <Digit d={2} slot="a" />
        <Digit d={4} slot="b" />
        <span>.</span>
        <Digit d={6} slot="c" idle />
        <span>k</span>
      </p>
    </div>
  );
}
