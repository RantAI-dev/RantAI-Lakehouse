import * as React from "react";
import { DEPTH_LAYERS } from "@/lib/home-content";

/**
 * The column-lineage picture's markup (WEB-5 T3). It has no state and no
 * hooks, so the server renders it as the page's still picture
 * (`lineage-trace-host.tsx`) and the live version (`lineage-trace.tsx`,
 * loaded after the first paint) renders the very same elements and only adds
 * the loop and the pointer. Both start on the first path.
 */

const BLUE = "#44A6FA";
const INK = "#E8F4FF";
const LIT = "#d6ecff";

export const TABLES = [
  { name: "orders", cols: ["order_id", "city_id", "amount", "created_at"] },
  { name: "orders_clean", cols: ["order_id", "order_date", "city", "amount"] },
  { name: "revenue_by_city", cols: ["city", "day", "revenue", "orders"] },
] as const;
/** Each path is one column's row in Bronze, Silver and Gold; the first is the copy's `amount → revenue`. */
export const PATHS = [
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



export type LineageHandlers = {
  /** The pointer left the picture. */
  onLeave?: (e: React.PointerEvent) => void;
  /** A column was entered by the pointer (`enter`) or clicked (`click`). */
  onCell?: (kind: "enter" | "click", table: number, row: number, e: React.PointerEvent | React.MouseEvent) => void;
};

export function LineageView({
  path,
  rootRef,
  onLeave,
  onCell,
}: Readonly<{ path: number; rootRef?: React.Ref<HTMLDivElement> } & LineageHandlers>) {
  const on = PATHS[path];
  return (
    <div ref={rootRef} aria-hidden="true" className="@container my-1" onPointerLeave={onLeave}>
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
                    onPointerEnter={onCell && ((e) => onCell("enter", ti, ci, e))}
                    onClick={onCell && ((e) => onCell("click", ti, ci, e))}
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
