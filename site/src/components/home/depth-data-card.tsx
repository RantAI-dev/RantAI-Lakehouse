"use client";

import * as React from "react";
import { Check, CornerDownRight, FileSpreadsheet, RefreshCw, Users, X } from "lucide-react";
import type { HomeCopy } from "@/lib/copy";
import { DEPTH_LAYERS, type DepthId } from "@/lib/home-content";
import { scrambleFrame } from "@/lib/scramble";
import {
  DEPTH_TABLES,
  GOLD_ROWS,
  GOLD_TOTAL,
  RAW_RECORDS,
  SILVER_CHECKS,
  SILVER_RECORDS,
  SOURCE_FILES,
  type SourceId,
} from "./depth-records";

/**
 * WEB-5 O21: the data card of the depths section. One small table that the
 * visitor watches change as the dive passes each depth (owner, 2026-11-07:
 * "ini 3 layer ini tu apa?"): the same five rows arrive raw in Bronze,
 * are cleaned and checked in Silver, and fold into revenue by city in Gold.
 *
 * It is one DOM for all three depths, so the change reads as cause and
 * effect rather than as three pictures:
 * - the rows are absolutely placed in slots (`--slot`), and CSS moves them
 *   when the depth changes: the duplicate slides out and the rows under it
 *   close the gap; in Gold each row travels to its city's group and fades
 *   as the group's bar grows out of it;
 * - a city or date that Silver rewrites re-spells itself (`scrambleFrame`)
 *   and flashes once;
 * - the Gold revenues and the total count up.
 *
 * WEB-5 D5: React renders only when the depth changes (twice over the whole
 * section, from `depths-stage.tsx`), never per scroll event; the re-spelling
 * and the count-up write `textContent` from one animation frame loop that
 * ends on the value React rendered. `animate={false}` (the reduced-motion
 * list, and the no-JS page) is the same card, still, at its depth.
 *
 * The card is decorative (`aria-hidden`); a visually hidden sentence per
 * depth says what it shows. The records are fictional (`depth-records.ts`);
 * O32 (owner, 2026-11-07) dropped the visible "Illustration" pill, as
 * visitors can tell they are not real data.
 */

type DepthsCopy = HomeCopy["depths"];
type CardCopy = DepthsCopy["card"];
type Stage = 0 | 1 | 2;

const ID_OF: readonly DepthId[] = ["bronze", "silver", "gold"];
const SOURCE_ICON: Record<SourceId, typeof FileSpreadsheet> = { orders: FileSpreadsheet, crm: Users };

const SILVER_BY_ID = new Map(SILVER_RECORDS.map((r) => [r.id, r]));
const GOLD_INDEX = new Map(GOLD_ROWS.map((g, i) => [g.city, i]));
const GOLD_MAX = GOLD_ROWS[0]?.revenue ?? 1;

/** Where each row sits, per depth: Bronze in arrival order, Silver closing the duplicate's gap, Gold on its city's group. */
function slotOf(id: string, stage: Stage): number {
  const index = RAW_RECORDS.findIndex((r) => r.id === id);
  if (stage === 0) return index;
  const silver = SILVER_BY_ID.get(id);
  if (stage === 2) return GOLD_INDEX.get(silver?.city ?? "") ?? 0;
  if (silver?.duplicate) return index;
  return SILVER_RECORDS.slice(0, index).filter((r) => !r.duplicate).length;
}

const SCRAMBLE_MS = 520;
const COUNT_MS = 900;
const easeOut = (t: number) => 1 - (1 - t) ** 3;

export function DepthDataCard({
  stage,
  copy,
  animate = true,
  className,
}: {
  stage: Stage;
  copy: CardCopy;
  animate?: boolean;
  className?: string;
}) {
  const id = ID_OF[stage];
  const accent = DEPTH_LAYERS[stage].accent;
  const fmt = React.useMemo(() => new Intl.NumberFormat(copy.numberLocale), [copy.numberLocale]);
  const root = React.useRef<HTMLDivElement>(null);
  const prev = React.useRef<Stage>(stage);
  const kept = SILVER_RECORDS.filter((r) => !r.duplicate).length;

  // The re-spelling and the count-up: one frame loop per change of depth,
  // over the cells that carry `data-from` (the text before the change).
  React.useLayoutEffect(() => {
    const from = prev.current;
    prev.current = stage;
    const el = root.current;
    if (!el || from === stage || !animate) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const words = [...el.querySelectorAll<HTMLElement>("[data-word]")].map((node) => ({
      node,
      from: (from === 0 ? node.dataset.raw : node.dataset.clean) ?? "",
      to: node.textContent ?? "",
    }));
    const counters = stage === 2 ? [...el.querySelectorAll<HTMLElement>("[data-count]")] : [];
    const targets = counters.map((n) => Number(n.dataset.count));
    let raf = 0;
    const t0 = performance.now();
    const tick = (now: number) => {
      const t = now - t0;
      const ps = Math.min(1, t / SCRAMBLE_MS);
      for (const w of words) if (w.from !== w.to) w.node.textContent = scrambleFrame(w.from, w.to, ps, Math.random);
      const pc = easeOut(Math.min(1, Math.max(0, t - 260) / COUNT_MS));
      counters.forEach((n, i) => (n.textContent = fmt.format(Math.round(targets[i] * pc))));
      if (ps < 1 || pc < 1) raf = requestAnimationFrame(tick);
    };
    tick(t0);
    return () => {
      cancelAnimationFrame(raf);
      for (const w of words) w.node.textContent = w.to;
      counters.forEach((n, i) => (n.textContent = fmt.format(targets[i])));
    };
  }, [stage, animate, fmt]);

  const rowCount = stage === 0 ? RAW_RECORDS.length : stage === 1 ? kept : GOLD_ROWS.length;

  return (
    // `data-depths-card`: the 3D scene measures this box to keep its slabs
    // 48 px clear of the card (WEB-5 O34, lake-engine.ts).
    <div className={className} data-depths-card>
      <p className="sr-only">{copy.sr[id]}</p>
      <div
        ref={root}
        aria-hidden="true"
        data-stage={id}
        className={`lk-depth-card${animate ? "" : " lk-depth-still"}`}
        style={{ "--lk-depth-accent": accent } as React.CSSProperties}
      >
        {/* Header: the table the rows live in, one per depth, and how many rows it holds. */}
        <div className="flex h-10 items-center gap-2 border-b border-[#E8F4FF]/10 px-3.5">
          <span className="lk-depth-dot size-1.5 shrink-0 rounded-full" />
          <span className="relative h-5 min-w-0 flex-1">
            {(["bronze", "silver", "gold"] as const).map((d) => (
              <span key={d} className="lk-depth-swap absolute inset-0 flex items-center" data-for={d}>
                <code className="truncate !text-[12px] !leading-5 text-[#E8F4FF]/90">{DEPTH_TABLES[d]}</code>
              </span>
            ))}
          </span>
          <span className="lk-depth-count shrink-0 text-[11px] text-[#E8F4FF]/60 tabular-nums">
            {rowCount} {stage === 2 ? copy.cities : copy.rows}
          </span>
        </div>

        {/* Body: the rows, and over them the Gold groups. Fixed height, so a change of depth moves nothing around the card. */}
        <div className="lk-depth-body relative">
          <div className="lk-depth-grid lk-depth-head" data-cols>
            <span className="lk-depth-src" />
            <span className="lk-depth-ord">
              {copy.columns.order}
              <CheckMark ok />
            </span>
            <span>
              {copy.columns.city}
              <CheckMark ok />
            </span>
            <span>{copy.columns.date}</span>
            <span className="text-right">
              {copy.columns.amount}
              <CheckMark ok={false} />
            </span>
          </div>

          {RAW_RECORDS.map((raw) => {
            const clean = SILVER_BY_ID.get(raw.id);
            const dup = clean?.duplicate ?? false;
            const city = stage === 0 ? raw.city : (clean?.city ?? raw.city);
            const date = stage === 0 ? raw.date : (clean?.date ?? raw.date);
            const Icon = SOURCE_ICON[raw.source];
            return (
              <div
                key={raw.id}
                className="lk-depth-row lk-depth-grid"
                data-dup={dup || undefined}
                style={{ "--slot": slotOf(raw.id, stage), "--i": RAW_RECORDS.indexOf(raw) } as React.CSSProperties}
              >
                {/* O31: the source as its glyph only; its name is in the Bronze footer and on hover. */}
                <span className="lk-depth-src relative flex items-center text-[#E8F4FF]/60">
                  <Icon className="size-3.5 shrink-0" strokeWidth={2} />
                  <span className="lk-depth-tip">{SOURCE_FILES[raw.source]}</span>
                </span>
                <span className="lk-depth-ord text-[#E8F4FF]/70 tabular-nums">
                  {raw.order}
                </span>
                <Word raw={raw.city} clean={clean?.city ?? raw.city} value={city} twice={dup} />
                <Word raw={raw.date} clean={clean?.date ?? raw.date} value={date} />
                <span className="text-right tabular-nums">
                  {raw.amount === null ? (
                    <span className="lk-depth-empty inline-flex items-center gap-1 rounded px-1.5 text-[10px] leading-4">
                      <X className="lk-depth-empty-x size-2.5" strokeWidth={3} />
                      {copy.empty}
                    </span>
                  ) : (
                    fmt.format(raw.amount)
                  )}
                </span>
              </div>
            );
          })}

          {/* Gold: the groups the rows fold into, with the revenue as a bar. */}
          <div className="lk-depth-gold absolute inset-0">
            <div className="lk-depth-grid-gold lk-depth-head">
              <span>{copy.columns.city}</span>
              <span className="text-right">{copy.columns.orders}</span>
              <span className="text-right">{copy.columns.revenue}</span>
            </div>
            {GOLD_ROWS.map((g, i) => (
              <div key={g.city} className="lk-depth-group lk-depth-grid-gold" style={{ "--slot": i } as React.CSSProperties}>
                <span className="font-medium text-[#E8F4FF]">{g.city}</span>
                <span className="text-right text-[#E8F4FF]/70 tabular-nums">{g.orders}</span>
                <span className="flex min-w-0 flex-col items-end gap-1">
                  <span className="font-medium text-[#E8F4FF] tabular-nums" data-count={g.revenue}>
                    {fmt.format(g.revenue)}
                  </span>
                  <span className="lk-depth-track relative h-[5px] w-full overflow-hidden rounded-full">
                    <span
                      className="lk-depth-bar absolute inset-y-0 left-0 rounded-full"
                      style={{ width: `${Math.round((g.revenue / GOLD_MAX) * 100)}%`, "--i": i } as React.CSSProperties}
                    />
                  </span>
                </span>
              </div>
            ))}
            <div className="lk-depth-total absolute inset-x-3.5 bottom-2.5 flex items-end justify-between gap-3">
              <span className="text-[11px] text-[#E8F4FF]/65">{copy.total}</span>
              <span className="lk-depth-kpi text-[22px] leading-7 font-semibold tabular-nums">
                <span className="mr-1 text-[13px] font-medium text-[#E8F4FF]/60">Rp</span>
                <span data-count={GOLD_TOTAL}>{fmt.format(GOLD_TOTAL)}</span>
              </span>
            </div>
          </div>
        </div>

        {/* Footer: what this depth promises, one line each. */}
        <div className="lk-depth-foot relative border-t border-[#E8F4FF]/10">
          {/* Bronze: the two sources the rows came from, as the row glyphs show them. */}
          <div className="lk-depth-swap absolute inset-0 flex items-center gap-1.5 px-3" data-for="bronze">
            {(["orders", "crm"] as const).map((src) => (
              <SourceChip key={src} source={src} />
            ))}
          </div>
          <div className="lk-depth-swap lk-depth-checks absolute inset-0 flex items-center gap-1.5 px-3" data-for="silver">
            <Badge ok>{copy.checks.duplicates}</Badge>
            <Badge ok>{copy.checks.city}</Badge>
            <Badge ok={SILVER_CHECKS.emptyAmounts === 0}>{copy.checks.amount}</Badge>
          </div>
          <div className="lk-depth-swap absolute inset-0 flex items-center justify-between gap-2 px-3" data-for="gold">
            <span className="lk-depth-lag inline-flex shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] leading-4 whitespace-nowrap">
              <RefreshCw className="lk-depth-spin size-3" strokeWidth={2.25} />
              {copy.lag}
            </span>
            <span className="flex min-w-0 items-center gap-1 text-[11px] text-[#E8F4FF]/70">
              <CornerDownRight className="lk-depth-hint size-3.5 shrink-0" strokeWidth={2} />
              <span className="lk-depth-feeds line-clamp-2">{copy.feeds}</span>
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}

/** A cell whose text Silver may rewrite; it keeps both spellings for the re-spelling. */
function Word({ raw, clean, value, twice }: { raw: string; clean: string; value: string; twice?: boolean }) {
  const fixed = raw !== clean;
  // A trailing space is invisible; Bronze shows where it is.
  const trailing = /\s$/.test(raw);
  return (
    <span className="lk-depth-word relative min-w-0 truncate" data-fix={fixed || undefined}>
      <span data-word data-raw={raw} data-clean={clean} className="whitespace-pre">
        {value}
      </span>
      {trailing && <span className="lk-depth-space">␣</span>}
      {/* The second copy of an order, marked where every width shows it. */}
      {twice && <span className="lk-depth-twice ml-1.5 rounded px-1 text-[10px] leading-4">×2</span>}
    </span>
  );
}

function SourceChip({ source }: { source: SourceId }) {
  const Icon = SOURCE_ICON[source];
  return (
    <span className="inline-flex min-w-0 items-center gap-1 rounded-md bg-[#E8F4FF]/[0.06] px-1.5 py-0.5 ring-1 ring-[#E8F4FF]/10">
      <Icon className="size-3 shrink-0 text-[#E8F4FF]/60" strokeWidth={2} />
      <code className="truncate !text-[11px] !leading-4 text-[#E8F4FF]/85">{SOURCE_FILES[source]}</code>
    </span>
  );
}

/** The mark a quality check puts on its column in Silver. */
function CheckMark({ ok }: { ok: boolean }) {
  const Icon = ok ? Check : X;
  return (
    <span className="lk-depth-mark ml-1 inline-grid size-3.5 place-items-center rounded-full align-[-2px]" data-ok={ok || undefined}>
      <Icon className="size-2.5" strokeWidth={3} />
    </span>
  );
}

function Badge({ ok, children }: { ok: boolean; children: React.ReactNode }) {
  const Icon = ok ? Check : X;
  return (
    <span className="lk-depth-badge inline-flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-[11px] leading-4" data-ok={ok || undefined}>
      <Icon className="size-3" strokeWidth={2.75} />
      {children}
    </span>
  );
}
