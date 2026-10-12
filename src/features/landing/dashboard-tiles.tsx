"use client";

import * as React from "react";
import { motion } from "motion/react";
import { useReducedMotionSafe } from "./use-reduced-motion";

/**
 * The five illustrative dashboard tiles of S5, drawn as SVG (no screenshots,
 * plan §4 S5). Every number is deterministic and made up: the section carries
 * the caption "Illustrations, not customer data." Colours stay within the
 * page palette: lake blue and foam on glass.
 */

const LAKE = "#44A6FA";
const FOAM = "#E8F4FF";

/**
 * Stable pseudo-random in 0..1 from two integers. Integer maths only: the
 * server renders with one JavaScript engine and the browser hydrates with
 * another, and `Math.sin` is not guaranteed to agree to the last digit.
 */
function hash(a: number, b: number): number {
  let h = Math.imul(a | 0, 374761393) ^ Math.imul(b | 0, 668265263);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  h ^= h >>> 16;
  return (h >>> 0) / 4294967296;
}

function useDraw() {
  const reduced = useReducedMotionSafe();
  return {
    initial: { pathLength: reduced ? 1 : 0, opacity: reduced ? 1 : 0 },
    whileInView: { pathLength: 1, opacity: 1 },
    viewport: { once: true, amount: 0.6 },
    transition: { duration: reduced ? 0 : 1.4, ease: [0.16, 1, 0.3, 1] as const },
  };
}

const svgProps = {
  viewBox: "0 0 360 190",
  className: "h-auto w-full",
  role: "img" as const,
};

export function LineChartTile() {
  const draw = useDraw();
  const a = "M10 150 C45 140 60 90 95 100 S150 140 185 95 S250 40 285 62 S335 30 350 24";
  const b = "M10 160 C50 150 70 135 105 138 S160 150 195 122 S255 100 290 106 S335 84 350 80";
  return (
    <svg {...svgProps} aria-label="Illustrative line chart">
      <defs>
        <linearGradient id="lk-area" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={LAKE} stopOpacity="0.35" />
          <stop offset="100%" stopColor={LAKE} stopOpacity="0" />
        </linearGradient>
      </defs>
      {[40, 80, 120, 160].map((y) => (
        <line key={y} x1="10" x2="350" y1={y} y2={y} stroke={FOAM} strokeOpacity="0.08" />
      ))}
      <motion.path d={`${a} L350 180 L10 180 Z`} fill="url(#lk-area)" initial={{ opacity: 0 }} whileInView={{ opacity: 1 }} viewport={{ once: true, amount: 0.6 }} transition={{ duration: 1.2, delay: 0.5 }} />
      <motion.path d={a} fill="none" stroke={LAKE} strokeWidth="2.5" strokeLinecap="round" {...draw} />
      <motion.path d={b} fill="none" stroke={FOAM} strokeOpacity="0.55" strokeWidth="1.5" strokeLinecap="round" strokeDasharray="1 5" {...draw} />
      <circle cx="350" cy="24" r="4" fill={LAKE} />
      <circle cx="350" cy="24" r="9" fill={LAKE} fillOpacity="0.25" />
    </svg>
  );
}

/** A dot-matrix map: a landmass-like blob of dots whose size and brightness stand for a value per region. */
export function DotMapTile() {
  const dots: { x: number; y: number; v: number }[] = [];
  const cols = 34;
  const rows = 15;
  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      const nx = (c / (cols - 1)) * 2 - 1;
      const ny = (r / (rows - 1)) * 2 - 1;
      // Two overlapping lobes, a gap between them, and a ragged edge.
      const lobe = Math.min(
        (nx + 0.42) ** 2 / 0.32 + (ny + 0.05) ** 2 / 0.62,
        (nx - 0.4) ** 2 / 0.3 + (ny - 0.12) ** 2 / 0.5,
      );
      const edge = (hash(c, r) - 0.5) * 0.35;
      if (lobe + edge < 1) {
        const region = Math.floor(((nx + 1) / 2) * 4) + Math.floor(((ny + 1) / 2) * 3) * 4;
        dots.push({ x: 14 + c * 10.1, y: 16 + r * 11.5, v: 0.25 + hash(region, 3) * 0.75 });
      }
    }
  }
  return (
    <svg {...svgProps} aria-label="Illustrative map of dots">
      {dots.map((d, i) => (
        <circle key={i} cx={d.x} cy={d.y} r={1.6 + d.v * 2.2} fill={d.v > 0.7 ? FOAM : LAKE} fillOpacity={0.25 + d.v * 0.7} />
      ))}
    </svg>
  );
}

export function SankeyTile() {
  const draw = useDraw();
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
  return (
    <svg {...svgProps} aria-label="Illustrative Sankey diagram">
      {flows.map((f, i) => {
        const y0 = left[f.from].y + usedL[f.from] + f.w / 2;
        const y1 = right[f.to].y + usedR[f.to] + f.w / 2;
        usedL[f.from] += f.w * 0.9;
        usedR[f.to] += f.w * 0.9;
        return (
          <motion.path
            key={i}
            d={`M44 ${y0} C170 ${y0} 190 ${y1} 316 ${y1}`}
            fill="none"
            stroke={i % 2 ? FOAM : LAKE}
            strokeOpacity={i % 2 ? 0.3 : 0.5}
            strokeWidth={f.w * 0.9}
            {...draw}
          />
        );
      })}
      {left.map((n, i) => (
        <rect key={`l${i}`} x="36" y={n.y} width="8" height={n.h} rx="3" fill={FOAM} fillOpacity="0.85" />
      ))}
      {right.map((n, i) => (
        <rect key={`r${i}`} x="316" y={n.y} width="8" height={n.h} rx="3" fill={FOAM} fillOpacity="0.85" />
      ))}
    </svg>
  );
}

export function KpiTile() {
  const draw = useDraw();
  const pts = [22, 30, 26, 38, 34, 46, 42, 58, 52, 66, 62, 78];
  const d = pts.map((v, i) => `${i === 0 ? "M" : "L"}${20 + i * 28} ${150 - v}`).join(" ");
  return (
    <svg {...svgProps} aria-label="Illustrative KPI tile">
      <text x="20" y="62" fill={FOAM} fontSize="54" fontWeight="500" letterSpacing="-2" fontFamily="var(--font-sans), sans-serif">
        24.6k
      </text>
      <text x="22" y="86" fill={LAKE} fontSize="14" fontFamily="var(--font-mono), monospace">
        +8.4% vs last week
      </text>
      <motion.path d={d} fill="none" stroke={LAKE} strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" {...draw} />
      <circle cx={20 + 11 * 28} cy={150 - 78} r="4" fill={LAKE} />
    </svg>
  );
}

export function CalendarTile() {
  const cols = 20;
  const cells: React.ReactNode[] = [];
  for (let c = 0; c < cols; c++) {
    for (let r = 0; r < 7; r++) {
      const v = hash(c + 1, r + 11);
      // Weekends are quieter, and a few busy weeks stand out.
      const quiet = r >= 5 ? 0.45 : 1;
      const level = Math.min(1, v * quiet * (0.55 + 0.6 * hash(c, 2)));
      cells.push(
        <rect
          key={`${c}-${r}`}
          x={12 + c * 17}
          y={14 + r * 23}
          width="13"
          height="17"
          rx="3.5"
          fill={level > 0.55 ? FOAM : LAKE}
          fillOpacity={0.12 + level * 0.8}
        />,
      );
    }
  }
  return (
    <svg {...svgProps} aria-label="Illustrative calendar heatmap">
      {cells}
    </svg>
  );
}
