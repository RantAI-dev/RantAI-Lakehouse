"use client";

import * as React from "react";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";
import { DEPTH_LAYERS } from "@/lib/home-content";

/**
 * The governance bento's lineage picture, "column lineage" (owner pick,
 * 2026-11-24, WEB-5; the three pills before it read "terlalu simple").
 *
 * Three small tables, Bronze → Silver → Gold, with their columns. One
 * column's path is traced across them and a light runs along it, then the
 * next path takes over. Pointing at any column traces that column instead.
 * The first path is `amount → revenue`, the one the tile's line describes.
 *
 * Table and column names are code (monospace) and illustrative; what the
 * product records is in `docs/core/features/column-lineage.md`. Needs no
 * copy: table, column and layer names are code or product names. Motion is
 * CSS (`.lk-lt-pulse` in app/globals.css) or a short timer, and runs only
 * while the tile is on screen. Reduced motion and the server markup show
 * the first path as a still, complete picture. `aria-hidden`: the tile's
 * title and line say it.
 */

const BLUE = "#44A6FA";
const INK = "#E8F4FF";
const LIT = "#d6ecff";

/** On screen, by the same rule as `LiveTile` (45 % of the tile), so timers stop when it scrolls away. */
function useOnScreen(ref: React.RefObject<HTMLElement | null>): boolean {
  const [on, setOn] = React.useState(false);
  React.useEffect(() => {
    const el = ref.current?.closest(".lk-tile") ?? ref.current;
    if (!el) return;
    const io = new IntersectionObserver(([e]) => setOn(e.isIntersecting), { threshold: 0.45 });
    io.observe(el);
    return () => io.disconnect();
  }, [ref]);
  return on;
}

/** A mouse holds a choice while it is over the picture; a tap picks and lets the loop go on. */
const isMouse = (e: React.PointerEvent) => e.pointerType === "mouse";


const TABLES = [
  { name: "orders", cols: ["order_id", "city_id", "amount", "created_at"] },
  { name: "orders_clean", cols: ["order_id", "order_date", "city", "amount"] },
  { name: "revenue_by_city", cols: ["city", "day", "revenue", "orders"] },
] as const;
/** Each path is one column's row in Bronze, Silver and Gold; the first is the copy's `amount → revenue`. */
const PATHS = [
  [2, 3, 2],
  [1, 2, 0],
  [3, 1, 1],
  [0, 0, 3],
] as const;
/** Fixed geometry, so the connectors meet the rows exactly: padding, header, row (px). */
const PAD = 7;
const HEAD = 22;
const ROW = 17;
const HEIGHT = PAD * 2 + HEAD + ROW * 4;
const rowY = (r: number) => PAD + HEAD + ROW * r + ROW / 2;


export function ColumnLineage() {
  const ref = React.useRef<HTMLDivElement>(null);
  const onScreen = useOnScreen(ref);
  const reduced = useReducedMotionSafe();
  const [path, setPath] = React.useState(0);
  const [held, setHeld] = React.useState(false);

  React.useEffect(() => {
    if (!onScreen || reduced || held) return;
    const t = setInterval(() => setPath((p) => (p + 1) % PATHS.length), 4400);
    return () => clearInterval(t);
  }, [onScreen, reduced, held]);

  const trace = (table: number, row: number) => {
    const p = PATHS.findIndex((x) => x[table] === row);
    if (p >= 0) setPath(p);
  };
  const on = PATHS[path];

  return (
    <div ref={ref} aria-hidden="true" className="@container my-1" onPointerLeave={(e) => isMouse(e) && setHeld(false)}>
      <div className="grid grid-cols-[minmax(0,1fr)_16px_minmax(0,1fr)_16px_minmax(0,1fr)] @min-[520px]:grid-cols-[minmax(0,1fr)_44px_minmax(0,1fr)_44px_minmax(0,1fr)]" style={{ height: HEIGHT }}>
        {TABLES.map((t, ti) => (
          <React.Fragment key={t.name}>
            {ti > 0 && (
              <div className="relative">
                <TraceEdges w={16} gap={ti - 1} path={path} className="@min-[520px]:hidden" />
                <TraceEdges w={44} gap={ti - 1} path={path} className="hidden @min-[520px]:block" />
              </div>
            )}
            <div className="lk-gv-node rounded-2xl bg-[var(--lk-n-raised)] px-1.5" style={{ paddingBlock: PAD, animationDelay: `${ti * 0.15}s` }}>
              <div className="flex items-center gap-1.5 px-1.5" style={{ height: HEAD }}>
                <span className="size-1.5 shrink-0 rounded-full" style={{ background: DEPTH_LAYERS[ti].accent }} />
                {/* Narrow: the layer alone; wide: the table (code) and its layer. */}
                <span className="hidden min-w-0 truncate font-mono text-[11.5px] text-[#E8F4FF] @min-[520px]:block">{t.name}</span>
                <span className="text-[11px] text-white/45 @min-[520px]:ml-auto">{DEPTH_LAYERS[ti].label}</span>
              </div>
              {t.cols.map((c, ci) => {
                const lit = on[ti] === ci;
                return (
                  <div
                    key={c}
                    onPointerEnter={(e) => {
                      if (!isMouse(e)) return;
                      trace(ti, ci);
                      setHeld(true);
                    }}
                    onClick={() => trace(ti, ci)}
                    className={`flex items-center rounded-md px-1.5 font-mono text-[11px] transition-colors duration-200 ease-out motion-reduce:transition-none ${
                      lit ? "bg-[#44A6FA]/15 text-[#d6ecff]" : "text-white/50 hover:text-white/75"
                    }`}
                    style={{ height: ROW }}
                  >
                    <span className="truncate">{c}</span>
                  </div>
                );
              })}
            </div>
          </React.Fragment>
        ))}
      </div>
    </div>
  );
}

/** The connectors between two tables: every mapping faint, the traced one blue, with a light along it. */
function TraceEdges({ w, gap, path, className }: Readonly<{ w: number; gap: number; path: number; className: string }>) {
  const d = (k: number) => {
    const y1 = rowY(PATHS[k][gap]);
    const y2 = rowY(PATHS[k][gap + 1]);
    return `M0 ${y1} C${w / 2} ${y1} ${w / 2} ${y2} ${w} ${y2}`;
  };
  return (
    <svg viewBox={`0 0 ${w} ${HEIGHT}`} width={w} height={HEIGHT} className={`absolute inset-y-0 left-0 overflow-visible ${className}`}>
      {PATHS.map((_, k) => (
        <path key={k} className="lk-draw" style={{ animationDelay: `${0.3 + gap * 0.2}s` }} pathLength={1} d={d(k)} fill="none" stroke={INK} strokeOpacity="0.14" strokeWidth="1.25" />
      ))}
      <g className="lk-fade" style={{ animationDelay: "0.7s" }}>
        {PATHS.map((_, k) => (
          <path
            key={k}
            d={d(k)}
            fill="none"
            stroke={BLUE}
            strokeWidth="1.75"
            strokeLinecap="round"
            className="transition-opacity duration-200 ease-out motion-reduce:transition-none"
            style={{ opacity: k === path ? 1 : 0 }}
          />
        ))}
      </g>
      {/* Keyed on the path, so a new trace starts its light from Bronze. */}
      <path key={path} className="lk-lt-pulse" style={{ animationDelay: `${gap * 1.1}s` }} pathLength={1} d={d(path)} fill="none" stroke={LIT} strokeWidth="2.5" strokeLinecap="round" />
    </svg>
  );
}
