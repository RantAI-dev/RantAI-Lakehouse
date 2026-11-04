import * as React from "react";
import type { TileText } from "./dashboard-tiles";

/**
 * The two dashboard tiles that do not animate, the dot map and the calendar
 * heatmap: hundreds of SVG shapes each, so they are server components and
 * are never hydrated (WEB-5 review fix R5). Same drawing as before; the
 * animated tiles stay in `dashboard-tiles.tsx`.
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

const svgProps = {
  viewBox: "0 0 360 190",
  className: "h-auto w-full",
  role: "img" as const,
};

/** A dot-matrix map: a landmass-like blob of dots whose size and brightness stand for a value per region. */
export function DotMapTile({ tile }: { tile: TileText }) {
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
    <svg {...svgProps} aria-label={tile.aria}>
      {dots.map((d, i) => (
        <circle key={i} cx={d.x} cy={d.y} r={1.6 + d.v * 2.2} fill={d.v > 0.7 ? FOAM : LAKE} fillOpacity={0.25 + d.v * 0.7} />
      ))}
    </svg>
  );
}

export function CalendarTile({ tile }: { tile: TileText }) {
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
    <svg {...svgProps} aria-label={tile.aria}>
      {cells}
    </svg>
  );
}
