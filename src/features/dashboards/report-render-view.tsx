"use client";

import * as React from "react";
import { cn } from "@/lib/utils";
import { dashboardLook } from "@/lib/dashboard-look";
import { issuedLine, paperContentWidth } from "@/lib/render-summary";
import type { ReportingContext } from "@/lib/time-grain";
import type { Report, ReportChartEntry } from "@/services/contracts/dashboard-reports";
import { READY_SAFETY_MS, markReady, renderFailure } from "./render-view";
import { RenderModeProvider, type RenderMode } from "./render-mode";
import { ReportBlocks } from "./report-blocks";
import { ReportingProvider } from "./reporting-context";

/** What `POST /api/render/data` answers for a report token (`BI-18` part D, T2). */
export type ReportRenderPayload = {
  report: Report;
  charts: Record<string, ReportChartEntry>;
  reporting?: ReportingContext | null;
  render?: { requestedBy?: string; issuedAt?: number; paper?: string; orientation?: string };
};

/**
 * The report's print view (`BI-18` part D, T2, decision R3), opened by Gotenberg
 * at `/render/report/<token>`. The same path as a dashboard's
 * (`render-view.tsx`): no session, the token is posted to
 * `POST /api/render/data`, which answers the report with every chart block read
 * as the person who asked; the blocks are drawn in order by `ReportBlocks`, the
 * reader page's own component, so the PDF shows what the page shows. Printed
 * light, at the paper's width; ready once every chart has finished drawing (or
 * after `READY_SAFETY_MS`), and a page that cannot load raises so the
 * conversion fails rather than printing an error sentence (`BI-1` R3).
 */
export function ReportRenderView({ token }: { token: string }) {
  const [data, setData] = React.useState<ReportRenderPayload | null>(null);
  const [state, setState] = React.useState<"loading" | "ok" | "error">("loading");
  const [message, setMessage] = React.useState("");
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
          setMessage(typeof body?.error === "string" ? body.error : "This report could not be loaded.");
          setState("error");
          renderFailure.raise("render data refused");
          return;
        }
        setData((await res.json()) as ReportRenderPayload);
        setState("ok");
      } catch {
        if (alive) { setMessage("This report could not be loaded."); setState("error"); renderFailure.raise("render data failed"); }
      }
    })();
    return () => { alive = false; };
  }, [token]);

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

  const look = dashboardLook({ theme: "light" }, false, data.reporting?.fonts);
  const locale = data.reporting?.numberLocale ?? "en-US";
  const issued = issuedLine(data.render?.issuedAt, data.reporting?.timeZone ?? "UTC", locale);
  const width = paperContentWidth(data.render?.paper, data.render?.orientation);

  return (
    <ReportingProvider reporting={data.reporting ?? undefined}>
      <RenderModeProvider value={mode}>
        <div
          className={cn("mx-auto min-h-screen bg-background p-2 text-foreground", look.className)}
          style={{ ...look.style, width }}
          data-render-view
        >
          <header className="mb-4 border-b pb-3">
            <h1 className="text-xl font-semibold">{data.report.title}</h1>
            <p className="mt-2 text-xs text-muted-foreground">
              Downloaded by {data.render?.requestedBy ?? "—"}{issued ? `, ${issued}` : ""}
            </p>
          </header>
          {data.report.blocks.length === 0 ? (
            <p className="text-sm text-muted-foreground">This report is empty.</p>
          ) : (
            <ReportBlocks blocks={data.report.blocks} charts={data.charts} dark={false} print />
          )}
        </div>
      </RenderModeProvider>
    </ReportingProvider>
  );
}
