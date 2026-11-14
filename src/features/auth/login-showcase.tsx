"use client";

/**
 * The sign-in page's living preview of the console (`WEB-15`, mockup A): a
 * small dashboard drawn with the console's real `TileBody` (ECharts bars
 * grow, a line draws, the KPI counts up), which rests and draws itself again
 * every 9 s while it is on screen and the tab is visible. Its charts answer
 * the pointer like the real ones. The numbers are fixed sample values, made
 * up for this page and labelled "Sample data" beside it: no API call, no
 * customer data (feature page decision 2).
 *
 * Loaded by `login-page.tsx` with `next/dynamic` and `ssr: false`, after the
 * form, so ECharts never delays typing (feature page decision 3). With
 * `prefers-reduced-motion` the dashboard draws once, still, and stays.
 */
import * as React from "react";
import { useTheme } from "next-themes";
import { Code2, Database, LayoutDashboard, MessageSquare, Workflow } from "lucide-react";
import Image from "next/image";
import { RenderModeProvider, type RenderMode } from "@/features/dashboards/render-mode";
import type { Rows } from "@/features/dashboards/tile-dialogs";
import { TileBody } from "@/features/dashboards/tile-body";
// WEB-15 review BLOCKER 1: from the shared hooks, never from `features/landing`,
// which the console image does not contain (`.dockerignore`, WEB-2).
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";
import { cn } from "@/lib/utils";

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

function spec(id: string, title: string, kind: ChartRenderSpec["kind"], x: string, y: string, def?: unknown): ChartRenderSpec {
  return { id, title, kind, mart: "mart_sample", source: "ui", x, y, def };
}

/** Where the count-up starts, as a share of the final value. */
const COUNT_FROM = 0.55;

/** The KPI's value at a point `t` (0 to 1) of its count-up. */
function kpiValue(total: number, t: number): number {
  return Math.round(total * (COUNT_FROM + (1 - COUNT_FROM) * t));
}

/** The KPI's cell at a point of its count-up: both years scale together, so the change keeps its sign and percent. */
function kpiCell(t: number): Rows {
  return {
    columns: ["year", "v"],
    rows: [
      { year: "2023-01-01", v: kpiValue(VISITORS_2023, t) },
      { year: "2024-01-01", v: kpiValue(VISITORS_2024, t) },
    ],
    grain: "year",
  };
}

const KPI_SPEC = spec("visitors", "Foreign visitors, 2024", "kpi", "year", "v", { compare: { kind: "previous", dateColumn: "year" } });
// The console's blue chart palette (mockup A's final version).
const BARS: Tile = {
  spec: spec("monthly", "Visitors per month", "bar", "Month", "Visitors", { colors: { palette: "sequential-blue" } }),
  cell: { columns: ["Month", "Visitors"], rows: MONTHS.map((m, i) => ({ Month: m, Visitors: MONTHLY[i] })) },
};
const LINE: Tile = {
  spec: spec("gold", "Rows landed in Gold", "line", "Day", "Rows", { colors: { palette: "sequential-blue" } }),
  cell: { columns: ["Day", "Rows"], rows: GOLD_ROWS.map((v, i) => ({ Day: DAYS[i], Rows: v })) },
};

/* ------------------------------------------------------------------ */
/* The loop                                                            */
/* ------------------------------------------------------------------ */

/** How long one drawing stays before the preview draws itself again (spec: 9 s). */
export const LOOP_MS = 9_000;
const COUNT_MS = 1_300;
/** The dim before a redraw; the root's opacity transition is 200 ms. */
const FADE_MS = 240;

function subscribeVisibility(onChange: () => void): () => void {
  document.addEventListener("visibilitychange", onChange);
  return () => document.removeEventListener("visibilitychange", onChange);
}

/** Whether the tab is the one in front (`document.visibilityState`). */
function usePageVisible(): boolean {
  return React.useSyncExternalStore(
    subscribeVisibility,
    () => document.visibilityState === "visible",
    () => true,
  );
}

/**
 * Plays the preview's loop: `run` changes to remount the charts (ECharts
 * animates a chart's first draw), `t` runs the KPI's count-up from 0 to 1,
 * and `fading` dims the window just before it starts over. The redraw runs
 * only while the preview is on screen (IntersectionObserver) and the tab is
 * visible; with reduced motion nothing is scheduled and the KPI shows its
 * final value from the first frame.
 */
function usePreviewLoop(ref: React.RefObject<HTMLElement | null>, reduced: boolean) {
  const [run, setRun] = React.useState(0);
  const [t, setT] = React.useState(0);
  const [fading, setFading] = React.useState(false);
  const [onScreen, setOnScreen] = React.useState(false);
  const pageVisible = usePageVisible();

  React.useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const io = new IntersectionObserver(([e]) => setOnScreen(e.isIntersecting), { threshold: 0.2 });
    io.observe(el);
    return () => io.disconnect();
  }, [ref]);

  // The count-up of each run: an ease-out cubic, so it lands softly.
  React.useEffect(() => {
    if (reduced) return;
    let frame = 0;
    const start = performance.now() + 150;
    const tick = (now: number) => {
      const p = Math.min(1, Math.max(0, (now - start) / COUNT_MS));
      setT(1 - Math.pow(1 - p, 3));
      if (p < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [run, reduced]);

  // The loop itself.
  React.useEffect(() => {
    if (reduced || !onScreen || !pageVisible) return;
    let fadeTimer = 0;
    const id = window.setInterval(() => {
      setFading(true);
      fadeTimer = window.setTimeout(() => {
        setFading(false);
        setT(0);
        setRun((r) => r + 1);
      }, FADE_MS);
    }, LOOP_MS);
    return () => {
      window.clearInterval(id);
      window.clearTimeout(fadeTimer);
      setFading(false);
    };
  }, [reduced, onScreen, pageVisible]);

  return { run, t: reduced ? 1 : t, fading };
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
    <span className="relative mb-2 inline-block size-6 shrink-0">
      <Image src="/logo-light.png" alt="" fill sizes="24px" className="object-contain dark:hidden" />
      <Image src="/logo-dark.png" alt="" fill sizes="24px" className="hidden object-contain dark:block" />
    </span>
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
  const { run, t, fading } = usePreviewLoop(ref, reduced);
  const bars = <TileBody key={`bars-${run}-${dark}`} spec={BARS.spec} cell={BARS.cell} dark={dark} loading={false} hideLegend />;

  const body = compact ? (
    <div className="flex h-full flex-col p-3">
      <div className="flex items-baseline justify-between gap-3">
        <p className="truncate text-xs font-medium text-muted-foreground">Foreign visitors, 2024</p>
        <p className="text-lg font-semibold tabular-nums" data-slot="showcase-kpi">
          {kpiValue(VISITORS_2024, t).toLocaleString("en-US")}
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
            <TileBody spec={KPI_SPEC} cell={kpiCell(t)} dark={dark} loading={false} />
          </PreviewTile>
          <PreviewTile title="Rows landed in Gold, this week" className="col-span-3">
            <TileBody key={`line-${run}-${dark}`} spec={LINE.spec} cell={LINE.cell} dark={dark} loading={false} hideLegend />
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
        "h-full overflow-hidden rounded-2xl bg-background shadow-2xl shadow-black/10 ring-1 ring-foreground/6 dark:shadow-black/40",
        // Entrance once: the window settles in as it arrives (200 ms, ease-out),
        // and the same opacity dims it before each redraw. Nothing moves
        // under reduced motion.
        "transition-[opacity,transform] duration-200 ease-[cubic-bezier(0.2,0,0,1)] starting:translate-y-1.5 starting:opacity-0 motion-reduce:transition-none",
        fading ? "opacity-0" : "opacity-100",
      )}
    >
      <RenderModeProvider value={reduced ? STILL : null}>{body}</RenderModeProvider>
    </div>
  );
}
