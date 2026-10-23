"use client";

import * as React from "react";
import { cn } from "@/lib/utils";
import { dashboardLook } from "@/lib/dashboard-look";
import { normalizeFilters } from "@/lib/dashboard-filter-state";
import { filterLine, issuedLine, paramLines } from "@/lib/render-summary";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";
import type { ReportingContext } from "@/lib/time-grain";
import type { ParamControl } from "@/lib/params";
import type { BoardTab, LayoutMap, PhoneTile } from "@/services";
import type { FilterDef } from "@/services/clients/bi-store";
import type { TileFailure } from "@/services/contracts/dashboards";
import { DashboardGrid, type GridItem } from "./dashboard-grid";
import { RenderModeProvider, type RenderMode } from "./render-mode";
import { ReportingProvider } from "./reporting-context";
import { TileBody } from "./tile-body";

type Cell = { columns: string[]; rows: Record<string, unknown>[] } | TileFailure;

/** What `POST /api/render/data` answers: the dashboard payload and the facts for the first page. */
export type RenderPayload = {
  board: string;
  layout: LayoutMap;
  tabs?: BoardTab[];
  phoneLayout?: PhoneTile[];
  filters?: FilterDef[];
  params?: ParamControl[];
  boards?: { id: string; name: string }[];
  charts: (ChartRenderSpec & { text?: string; caption?: string })[];
  results: Record<string, Cell>;
  reporting?: ReportingContext;
  theme?: string;
  font?: string;
  render?: { requestedBy?: string; issuedAt?: number; paper?: string; orientation?: string };
};

/**
 * How long the view waits for charts that never report a finished draw before
 * it says it is ready anyway. Below the API's 60-second deadline, so a chart
 * that stalls costs a blank tile in the PDF, not the whole PDF.
 */
export const READY_SAFETY_MS = 20_000;

/** What replaces an image or web-page card (feature decision 3): the renderer opens the console and nothing else. */
export const NOT_IN_PDF = "Shown in the console only";

/**
 * How a page that could not load its data fails the PDF (`BI-1` review fix
 * (SHOULD-FIX) R3). Gotenberg is asked for `failOnConsoleExceptions`, so an
 * uncaught exception in this page fails the conversion and the route answers
 * "The PDF could not be made." instead of sending a PDF of an error sentence.
 * An object, so a test can replace `raise` (a real throw would fail the test run).
 */
export const renderFailure = {
  raise(message: string): void {
    window.setTimeout(() => { throw new Error(message); }, 0);
  },
};

/** Set on `window` when every tile has drawn or shown its error; Gotenberg waits for it. */
function markReady() {
  (window as unknown as { __lakehouseRenderReady?: boolean }).__lakehouseRenderReady = true;
  document.documentElement.dataset.renderReady = "true";
}

/**
 * The PDF's print view (`BI-1`, plan D4), opened by Gotenberg at
 * `/render/dashboard/<token>`. It has no session: the token in the address is
 * posted to `POST /api/render/data`, which answers the dashboard as the person
 * who asked. Every tab is drawn, one after the other, each after the first
 * starting on a new page with its title; sections are unfolded (nothing here
 * folds); image and web cards are a placeholder; charts draw without animation.
 * `window.__lakehouseRenderReady` is set once every chart has finished and the
 * browser has painted, or after {@link READY_SAFETY_MS}. A page that cannot load
 * its data shows the sentence and raises an uncaught exception, which fails the
 * conversion (R3); it is marked ready so Gotenberg checks at once.
 */
export function RenderView({ token }: { token: string }) {
  const [data, setData] = React.useState<RenderPayload | null>(null);
  const [state, setState] = React.useState<"loading" | "ok" | "error">("loading");
  const [message, setMessage] = React.useState("");
  // Charts mounted and finished. The ref is what the effect reads when a frame
  // arrives; the state only wakes the effect when a count changes.
  const live = React.useRef({ mounted: 0, finished: 0 });
  const [counts, setCounts] = React.useState({ mounted: 0, finished: 0 });
  const mode = React.useMemo<RenderMode>(() => ({
    chartMounted: () => { live.current.mounted += 1; setCounts({ ...live.current }); },
    chartFinished: () => { live.current.finished += 1; setCounts({ ...live.current }); },
  }), []);

  React.useEffect(() => {
    let alive = true;
    (async () => {
      try {
        // Deliberately NOT `apiFetch`: the renderer has no session, the token is the credential.
        const res = await fetch("/api/render/data", {
          method: "POST", headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ token }), cache: "no-store",
        });
        if (!alive) return;
        if (!res.ok) {
          const body = (await res.json().catch(() => null)) as { error?: unknown } | null;
          setMessage(typeof body?.error === "string" ? body.error : "This dashboard could not be loaded.");
          setState("error");
          renderFailure.raise("render data refused");
          return;
        }
        setData((await res.json()) as RenderPayload);
        setState("ok");
      } catch {
        if (alive) { setMessage("This dashboard could not be loaded."); setState("error"); renderFailure.raise("render data failed"); }
      }
    })();
    return () => { alive = false; };
  }, [token]);

  // Ready: the data is in, every chart that mounted has finished, and the
  // browser has had two frames to paint. A page that failed to load is ready
  // too: it prints its sentence. The counts are read again when a frame
  // arrives, not taken from the render that scheduled it: a chart mounts in
  // the same commit that scheduled this effect.
  React.useEffect(() => {
    if (state === "loading") return;
    const settled = () => live.current.mounted <= live.current.finished;
    if (!settled()) {
      const timer = window.setTimeout(markReady, READY_SAFETY_MS);
      return () => window.clearTimeout(timer);
    }
    let second = 0;
    const first = requestAnimationFrame(() => {
      second = requestAnimationFrame(() => { if (settled()) markReady(); });
    });
    return () => { cancelAnimationFrame(first); cancelAnimationFrame(second); };
  }, [state, counts]);

  if (state === "error") {
    return <div role="alert" className="grid h-screen place-content-center px-4 text-center text-sm">{message}</div>;
  }
  if (!data) return <div className="p-4 text-sm text-muted-foreground">Preparing the PDF…</div>;

  const look = dashboardLook(data, false, data.reporting?.fonts);
  const locale = data.reporting?.numberLocale ?? "en-US";
  const name = data.boards?.find((b) => b.id === data.board)?.name ?? (data.board === "default" ? "Main" : data.board);
  const filters = normalizeFilters(data.filters ?? []).map(filterLine);
  const params = paramLines(data.params);
  const issued = issuedLine(data.render?.issuedAt, data.reporting?.timeZone ?? "UTC", locale);
  const items: GridItem[] = data.charts.map((spec) => ({
    id: spec.id,
    kind: spec.kind,
    title: spec.title,
    subtitle: spec.subtitle,
    body: spec.kind === "image" || spec.kind === "web"
      ? <p role="status" className="grid h-full place-items-center px-3 text-center text-xs text-muted-foreground">{NOT_IN_PDF}</p>
      : <TileBody spec={spec} cell={data.results[spec.id]} dark={look.dark} loading={false} />,
  }));
  const tabs = data.tabs ?? [];

  return (
    <ReportingProvider reporting={data.reporting}>
      <RenderModeProvider value={mode}>
        <div className={cn("min-h-screen bg-background p-2 text-foreground", look.className)} style={look.style} data-render-view>
          <header className="mb-4 border-b pb-3">
            <h1 className="text-xl font-semibold">{name}</h1>
            <dl className="mt-2 grid gap-x-4 gap-y-0.5 text-xs text-muted-foreground sm:grid-cols-[max-content_1fr]">
              <dt>Filters applied</dt>
              <dd data-testid="render-filters">{filters.length ? filters.join(" · ") : "None"}</dd>
              {params.length ? (<><dt>Parameters</dt><dd data-testid="render-params">{params.join(" · ")}</dd></>) : null}
              <dt>Downloaded by</dt>
              <dd>{data.render?.requestedBy ?? "—"}{issued ? `, ${issued}` : ""}</dd>
            </dl>
          </header>
          {items.length === 0 ? (
            <p className="text-sm text-muted-foreground">No charts.</p>
          ) : tabs.length === 0 ? (
            <DashboardGrid items={items} layout={data.layout} editable={false} onLayoutChange={() => {}} phone={data.phoneLayout} />
          ) : (
            tabs.map((tab, i) => (
              <section key={tab.id} className={cn("mb-6", i > 0 && "render-tab-break")} data-render-tab={tab.id}>
                <h2 className="mb-2 text-base font-semibold">{tab.title}</h2>
                <DashboardGrid items={items} layout={data.layout} editable={false} onLayoutChange={() => {}} tabs={tabs} tab={tab.id} phone={data.phoneLayout} />
              </section>
            ))
          )}
        </div>
      </RenderModeProvider>
    </ReportingProvider>
  );
}
