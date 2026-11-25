"use client";

/**
 * The sign-in page's living preview of the console (`WEB-15`, mockup A): a
 * small dashboard whose charts are the console's real `TileBody` (ECharts
 * bars grow, a line draws) beside a KPI that counts up; it rests and draws
 * lives on every 9 s while it is on screen and the tab is visible: the bars
 * ease to new heights, the line moves, the KPI counts to its new total, and
 * the window itself never leaves. Its charts answer
 * the pointer like the real ones. The numbers are fixed sample values, made
 * up for this page and described as sample data by the panel label: no API call, no
 * customer data (feature page decision 2).
 *
 * Loaded by `login-page.tsx` with `next/dynamic` and `ssr: false`, after the
 * form, so ECharts never delays typing (feature page decision 3). With
 * `prefers-reduced-motion` the dashboard draws once, still, and stays.
 */
import * as React from "react";
import { useTheme } from "next-themes";
import { ArrowUp, Code2, Database, LayoutDashboard, MessageSquare, Workflow } from "lucide-react";
import Image from "next/image";
import { RenderModeProvider, type RenderMode } from "@/features/dashboards/render-mode";
import type { Rows } from "@/features/dashboards/tile-dialogs";
import { TileBody } from "@/features/dashboards/tile-body";
// WEB-15 review BLOCKER 1: the hook is shared, in `src/hooks`.
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";
import { signedPercent, sparklinePoints } from "@/lib/kpi-compare";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { useOnScreen, usePageVisible } from "./use-in-view";

/* ------------------------------------------------------------------ */
/* Sample data (labelled as such on the page)                          */
/* ------------------------------------------------------------------ */

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHLY = [98_412, 91_207, 104_885, 112_430, 121_964, 138_210, 162_845, 171_390, 146_022, 135_118, 128_904, 161_137];
/** The KPI's total for 2024: the sum of the bars, so the two tiles agree. */
export const VISITORS_2024 = MONTHLY.reduce((s, v) => s + v, 0);
const VISITORS_2023 = 1_318_240;
const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const GOLD_ROWS = [468, 494, 503, 497, 521, 538, 571].map((k) => k * 1_000);

type Tile = { spec: ChartRenderSpec; cell: Rows };

/**
 * One live tick's sample values: step 0 is the base data above (what a still
 * dashboard shows); later steps move each value by up to 7 % on a smooth,
 * deterministic wave, so the dashboard changes the way a live one does
 * without a random number or a blank frame.
 */
function wave(step: number, i: number, base: number): number {
  return step === 0 ? base : Math.round(base * (1 + 0.07 * Math.sin(step * 1.7 + i * 1.1)));
}
const monthlyAt = (step: number) => MONTHLY.map((v, i) => wave(step, i, v));
const goldAt = (step: number) => GOLD_ROWS.map((v, i) => wave(step, i + 3, v));
const sumOf = (xs: number[]) => xs.reduce((a, b) => a + b, 0);

function spec(id: string, title: string, kind: ChartRenderSpec["kind"], x: string, y: string, def?: unknown): ChartRenderSpec {
  return { id, title, kind, mart: "mart_sample", source: "ui", x, y, def };
}

/** Where the count-up starts, as a share of the final value. */
const COUNT_FROM = 0.55;

/** The change against 2023 at a total: the KPI's tooltip and its one-line delta. */
const deltaOf = (total: number) => total - VISITORS_2023;
const percentOf = (total: number) => (deltaOf(total) / VISITORS_2023) * 100;

/**
 * Day and month names read flat (WEB-15 T2, owner's screenshot): the
 * console tilts category labels past six, but here twelve short months and
 * seven days fit the tiles' width level. The chart style's own axis setting
 * (`style-option.ts`) keeps every label and its overlap check, which hides a
 * label rather than letting two collide on a narrow window.
 */
const FLAT_X = { x: { rotate: 0 as const } };
// The console's blue chart palette (mockup A's final version).
const BARS: Tile = {
  spec: spec("monthly", "Visitors per month", "bar", "Month", "Visitors", { colors: { palette: "sequential-blue" }, axes: FLAT_X }),
  cell: { columns: ["Month", "Visitors"], rows: MONTHS.map((m, i) => ({ Month: m, Visitors: MONTHLY[i] })) },
};
const barsCell = (step: number): Rows => ({
  columns: ["Month", "Visitors"],
  rows: MONTHS.map((m, i) => ({ Month: m, Visitors: monthlyAt(step)[i] })),
});
const lineCell = (step: number): Rows => ({
  columns: ["Day", "Rows"],
  rows: DAYS.map((d, i) => ({ Day: d, Rows: goldAt(step)[i] })),
});
const LINE: Tile = {
  spec: spec("gold", "Rows landed in Gold", "line", "Day", "Rows", { colors: { palette: "sequential-blue" }, axes: FLAT_X }),
  cell: { columns: ["Day", "Rows"], rows: GOLD_ROWS.map((v, i) => ({ Day: DAYS[i], Rows: v })) },
};

/* ------------------------------------------------------------------ */
/* The loop                                                            */
/* ------------------------------------------------------------------ */

/** How long one reading stays before the preview moves to the next (spec: 9 s). */
export const LOOP_MS = 9_000;
/** The first count-up, then the shorter, eased move to each new total. */
const COUNT_MS = 1_300;
const MOVE_MS = 600;

/**
 * Plays the preview's life: `step` advances every `LOOP_MS` and the data
 * follows it (the charts animate their own change; nothing remounts and
 * nothing dims). The tick runs only while the preview is on screen
 * (IntersectionObserver) and the tab is visible; with reduced motion nothing
 * is scheduled and step 0, the base data, stays.
 */
function usePreviewLoop(ref: React.RefObject<HTMLElement | null>, reduced: boolean) {
  const [step, setStep] = React.useState(0);
  const onScreen = useOnScreen(ref, 0.2);
  const pageVisible = usePageVisible();

  React.useEffect(() => {
    if (reduced || !onScreen || !pageVisible) return;
    const id = window.setInterval(() => setStep((n) => n + 1), LOOP_MS);
    return () => window.clearInterval(id);
  }, [reduced, onScreen, pageVisible]);

  return reduced ? 0 : step;
}

/**
 * The KPI's shown number: it counts up from 55 % on the first look, then
 * eases (cubic ease-out, never linear) to each new total, starting from
 * wherever it is, so it never jumps and never blanks.
 */
function useCountedTotal(target: number, reduced: boolean): number {
  const [shown, setShown] = React.useState(() => (reduced ? target : Math.round(target * COUNT_FROM)));
  const shownRef = React.useRef(shown);
  const first = React.useRef(true);
  React.useEffect(() => {
    if (reduced) {
      shownRef.current = target;
      setShown(target);
      return;
    }
    const from = shownRef.current;
    const wasFirst = first.current;
    first.current = false;
    const duration = wasFirst ? COUNT_MS : MOVE_MS;
    const start = performance.now() + (wasFirst ? 150 : 0);
    let frame = 0;
    const tick = (now: number) => {
      const p = Math.min(1, Math.max(0, (now - start) / duration));
      const v = Math.round(from + (target - from) * (1 - Math.pow(1 - p, 3)));
      shownRef.current = v;
      setShown(v);
      if (p < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [target, reduced]);
  return reduced ? target : shown;
}

/**
 * Under reduced motion the charts draw without animation: the PDF print
 * view's switch (`render-mode.tsx`), with nobody to report to.
 */
const STILL: RenderMode = { chartMounted: () => {}, chartFinished: () => {} };

/* ------------------------------------------------------------------ */
/* The preview                                                         */
/* ------------------------------------------------------------------ */

function PreviewTile({ title, className, children }: { title: string; className?: string; children: React.ReactNode }) {
  return (
    <div className={cn("flex min-h-0 flex-col rounded-xl bg-card p-3 shadow-xs ring-1 ring-foreground/5", className)}>
      <p className="truncate text-xs font-medium text-muted-foreground">{title}</p>
      <div className="min-h-0 flex-1">{children}</div>
    </div>
  );
}

/** The official mark at its own proportions (DESIGN.md §5), small, in the drawn sidebar. */
function SidebarLogo() {
  return (
    <span className="relative mb-2 inline-block aspect-[196/167] h-6 shrink-0">
      <Image src="/logo-light.svg" alt="" fill unoptimized sizes="24px" className="object-contain dark:hidden" />
      <Image src="/logo-dark.svg" alt="" fill unoptimized sizes="24px" className="hidden object-contain dark:block" />
    </span>
  );
}

/**
 * The KPI tile: the number, and its change against 2023 on one line.
 *
 * Drawn here rather than by the console's `KpiCard` (WEB-15 T2, owner's
 * screenshot): that card repeats the period under the number ("2024"), which
 * this tile's title already names, and its "+254,284 (+19.3%) vs 2023" wraps
 * to two lines at the tile's width. The change is the same reading with the
 * same helper (`signedPercent`); the line keeps the percent, and the
 * amount moves to the tooltip. The trend under it is the bars' own months.
 */
function ShowcaseKpi({ value, total }: { value: number; total: number }) {
  return (
    <div className="grid h-full place-content-center justify-items-center px-2 text-center">
      <p className="text-4xl font-semibold tracking-tight tabular-nums text-foreground" data-slot="showcase-kpi">
        {value.toLocaleString("en-US")}
      </p>
      <Tooltip>
        <TooltipTrigger
          render={
            <p className="mt-1.5 inline-flex items-center gap-1 whitespace-nowrap text-sm font-medium tabular-nums text-emerald-600 dark:text-emerald-400" />
          }
        >
          <ArrowUp className="size-4" aria-hidden />
          {signedPercent(percentOf(total))}
          <span className="font-normal text-muted-foreground">vs 2023</span>
        </TooltipTrigger>
        <TooltipContent>{deltaOf(total).toLocaleString("en-US")} more visitors than in 2023</TooltipContent>
      </Tooltip>
      <svg viewBox="0 0 120 28" aria-hidden className="mt-2.5 h-7 w-32 text-emerald-600 dark:text-emerald-400">
        <polyline
          points={sparklinePoints(MONTHLY, 120, 28)}
          pathLength={1}
          className="login-draw"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinejoin="round"
          strokeLinecap="round"
        />
      </svg>
    </div>
  );
}

const PLACES = [LayoutDashboard, Database, Code2, Workflow, MessageSquare];

/**
 * The preview window. `compact` is the phone's: one tile, the bars, with the
 * KPI's count-up in its header, so the form stays on the first screen.
 */
export function LoginShowcase({ compact = false }: { compact?: boolean }) {
  const ref = React.useRef<HTMLDivElement>(null);
  const reduced = useReducedMotionSafe();
  const { resolvedTheme } = useTheme();
  const dark = resolvedTheme === "dark";
  const step = usePreviewLoop(ref, reduced);
  const total = sumOf(monthlyAt(step));
  const value = useCountedTotal(total, reduced);
  const bars = <TileBody key={`bars-${dark}`} spec={BARS.spec} cell={barsCell(step)} dark={dark} loading={false} hideLegend live />;

  const body = compact ? (
    <div className="flex h-full flex-col p-3">
      <div className="flex items-baseline justify-between gap-3">
        <p className="truncate text-xs font-medium text-muted-foreground">Foreign visitors, 2024</p>
        <p className="text-lg font-semibold tabular-nums" data-slot="showcase-kpi">
          {value.toLocaleString("en-US")}
        </p>
      </div>
      <div className="min-h-0 flex-1">{bars}</div>
    </div>
  ) : (
    <div className="flex h-full min-h-0">
      {/* The console's sidebar, drawn small: the product's places, Dashboards open. */}
      <nav aria-hidden className="flex w-12 shrink-0 flex-col items-center gap-2 border-r border-foreground/5 py-3">
        <SidebarLogo />
        {PLACES.map((Icon, i) => (
          <span
            key={i}
            className={cn(
              "grid size-8 place-items-center rounded-lg text-muted-foreground",
              i === 0 && "bg-primary/12 text-primary dark:bg-sky-400/12 dark:text-sky-300",
            )}
          >
            <Icon className="size-4" />
          </span>
        ))}
      </nav>
      <div className="flex min-w-0 flex-1 flex-col gap-3 p-4">
        <div className="flex items-center gap-2">
          <p className="mr-2 text-sm font-semibold">Tourism overview</p>
          <span className="rounded-full bg-muted px-2.5 py-1 text-[11px] text-muted-foreground">2024</span>
          <span className="rounded-full bg-muted px-2.5 py-1 text-[11px] text-muted-foreground">All regions</span>
        </div>
        <div className="grid min-h-0 flex-1 grid-cols-5 grid-rows-[0.9fr_1.25fr] gap-3">
          <PreviewTile title="Foreign visitors, 2024" className="col-span-2">
            <ShowcaseKpi value={value} total={total} />
          </PreviewTile>
          <PreviewTile title="Rows landed in Gold, this week" className="col-span-3">
            <TileBody key={`line-${dark}`} spec={LINE.spec} cell={lineCell(step)} dark={dark} loading={false} hideLegend live />
          </PreviewTile>
          <PreviewTile title="Visitors per month, 2024" className="col-span-5">
            {bars}
          </PreviewTile>
        </div>
      </div>
    </div>
  );

  return (
    <div
      ref={ref}
      data-motion={reduced ? "still" : "live"}
      className={cn(
        "h-full overflow-hidden rounded-2xl bg-background ring-1",
        // On the navy lake panel (T7) the window lifts with one wide, soft,
        // navy-black shadow and a hairline of light at its edge, never a hard
        // black rim; the phone's tile sits on the white page as before.
        compact
          ? "shadow-2xl shadow-black/10 ring-foreground/6 dark:shadow-black/40"
          : "shadow-[0_28px_70px_-24px_rgb(1_3_18/0.7)] ring-white/10",
        // Entrance once: the window rises 12 px onto the lake and lands
        // (360 ms, a strong ease-out; T9, the first look's card beat, longer
        // than a control's 240 ms because it travels further). After that
        // the window never fades or moves again (owner, 2026-11-14: "dashboardnya
        // jangan hilang"). Nothing moves under reduced motion.
        "transition-[opacity,translate] duration-[360ms] ease-[cubic-bezier(0.23,1,0.32,1)] starting:translate-y-3 starting:opacity-0 motion-reduce:transition-none",
      )}
    >
      <RenderModeProvider value={reduced ? STILL : null}>{body}</RenderModeProvider>
    </div>
  );
}
