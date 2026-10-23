"use client";

import type { ReportingContext } from "@/lib/time-grain";
import * as React from "react";
import { useTheme } from "next-themes";
import { cn } from "@/lib/utils";
import { dashboardLook } from "@/lib/dashboard-look";
import { DashboardGrid, type GridItem } from "./dashboard-grid";
import { ReportingProvider } from "./reporting-context";
import { TileBody } from "./tile-body";
import { OwnFiltersMarker, ownFilters } from "./filters/own-filters-marker";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";
import { selectedTab } from "@/lib/dashboard-tabs";
import type { BoardTab, LayoutMap, PhoneTile } from "@/services";
import { TabBar } from "./tab-bar";
import type { TileFailure } from "@/services/contracts/dashboards";
import { EMBED_UNAVAILABLE, embedRefusalMessage } from "./embed-refusal";
import { PasswordGate, openPublic } from "./public-link-access";

/** BI-19 part A: the same words as the public page, for a link that does not work. */
const LINK_GONE = "This link does not work";

type Cell = { columns: string[]; rows: Record<string, unknown>[] } | TileFailure;
type Payload = {
  board: { id: string; name: string; theme?: string; font?: string };
  layout: LayoutMap;
  /** BI-18 part C: only when the board has them. */
  tabs?: BoardTab[];
  phoneLayout?: PhoneTile[];
  charts: (ChartRenderSpec & { text?: string; caption?: string })[];
  results: Record<string, Cell>;
  /** The zone and first weekday the buckets were cut with (BI-9), and the formatting settings (BI-17). */
  reporting?: ReportingContext;
};

/**
 * The EMBED view — designed to sit inside an <iframe> on another site/app
 * (Metabase-public-embed style). No header/footer/chrome. Two modes:
 *  - the whole dashboard (grid) — default
 *  - a single chart when `chartId` is given (?chart=<id>) — fills the iframe.
 * Read-only, data from Gold marts. Transparent background so it blends into the host.
 */
export function EmbedView({ token, jwt, chartId }: { token?: string; jwt?: string; chartId?: string }) {
  const { resolvedTheme } = useTheme();
  const appDark = resolvedTheme === "dark";
  const [data, setData] = React.useState<Payload | null>(null);
  // BI-17 part A (D13): the dashboard's own theme and font apply in an embed too.
  const look = dashboardLook(data?.board, appDark, data?.reporting?.fonts);
  const dark = look.dark;
  const [tabId, setTabId] = React.useState<string | null>(null);
  const [state, setState] = React.useState<"loading" | "ok" | "notfound" | "password" | "error">("loading");
  const [attempt, setAttempt] = React.useState(0);
  // SEC-12: what a refused signed token reads. The public link keeps the
  // plain line.
  const [refusal, setRefusal] = React.useState(EMBED_UNAVAILABLE);

  // Data source: a public token (GET, read-only) OR a signed JWT embed (POST,
  // server-side locked filters). Public link: /embed/dashboard/<token>.
  const linkBase = token ? `/public/dashboard/${token}` : "";

  React.useEffect(() => {
    let alive = true;
    (async () => {
      try {
        if (!jwt) {
          // BI-19 part A: a public link may need a password (a ticket from the
          // unlock route). Deliberately NOT `apiFetch`, as below.
          const result = await openPublic<Payload>(token ?? "");
          if (!alive) return;
          if (result.kind === "gone") { setRefusal(LINK_GONE); setState("notfound"); return; }
          if (result.kind === "password") { setState("password"); return; }
          if (result.kind === "error") { setState("error"); return; }
          setData(result.data);
          setState("ok");
          return;
        }
        // Deliberately NOT `apiFetch`: embed views are unauthenticated
        // by design (a signed JWT, never a session), so a
        // 401/403/404 is "not found" for the viewer, not a login prompt.
        const res = await fetch(`/api/embed/data`, {
          method: "POST", headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ jwt }), cache: "no-store",
        });
        if (!alive) return;
        if (res.status === 404 || res.status === 403 || res.status === 401) {
          setRefusal(embedRefusalMessage(res.status, await res.json().catch(() => null)));
          setState("notfound");
          return;
        }
        if (res.status === 503) {
          setRefusal(embedRefusalMessage(res.status, await res.json().catch(() => null)));
          setState("error");
          return;
        }
        if (!res.ok) { setState("error"); return; }
        setData(await res.json());
        setState("ok");
      } catch { if (alive) setState("error"); }
    })();
    return () => { alive = false; };
  }, [token, jwt, attempt]);

  if (state === "password") {
    return <PasswordGate token={token ?? ""} onOpen={() => { setState("loading"); setAttempt((n) => n + 1); }} />;
  }
  if (state === "notfound" || state === "error") {
    return <div className="grid h-screen place-content-center px-4 text-center text-sm text-muted-foreground">{refusal}</div>;
  }

  const charts = data?.charts ?? [];
  const tab = selectedTab(data?.tabs ?? [], tabId);

  // ── Mode: a single chart fills the iframe ──────────────────────────────────
  if (chartId) {
    const spec = charts.find((c) => c.id === chartId);
    if (state === "ok" && !spec) {
      return <div className="grid h-screen place-content-center text-sm text-muted-foreground">Chart not found.</div>;
    }
    return (
      <ReportingProvider reporting={data?.reporting}>
      <div className={cn("flex h-screen flex-col overflow-hidden bg-transparent p-2", look.className)} style={look.style}>
        <div className="flex h-full flex-col overflow-hidden rounded-xl border border-border bg-card">
          <div className="flex items-center gap-2 border-b border-border px-3 py-1.5">
            <p className="min-w-0 flex-1 truncate text-sm font-semibold">{spec?.title ?? "…"}</p>
            {linkBase ? (
              <a href={linkBase} target="_blank" rel="noopener noreferrer"
                 className="text-[10px] font-medium text-muted-foreground hover:text-foreground">Rantai Lake ↗</a>
            ) : (
              <span className="text-[10px] font-medium text-muted-foreground">Rantai Lake</span>
            )}
          </div>
          <div className="min-h-0 flex-1 p-2">
            {spec ? <TileBody spec={spec} cell={data?.results[spec.id]} dark={dark} loading={state === "loading"} /> : null}
          </div>
        </div>
      </div>
      </ReportingProvider>
    );
  }

  // ── Mode: the whole dashboard ──────────────────────────────────────────────
  const items: GridItem[] = charts.map((spec) => ({
    id: spec.id,
    kind: spec.kind,
    title: spec.title,
    badge: <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">{spec.kind}</span>,
    // BI-6: the chart's own filters, listed (the viewer cannot change them).
    hint: ownFilters(spec) ? <OwnFiltersMarker filters={ownFilters(spec) ?? []} /> : null,
    body: <TileBody spec={spec} cell={data?.results[spec.id]} dark={dark} loading={state === "loading"} />,
  }));

  return (
    <ReportingProvider reporting={data?.reporting}>
    <div className={cn("min-h-screen bg-transparent p-2", look.className)} style={look.style}>
      {state === "loading" ? (
        <div className="grid gap-3 sm:grid-cols-2">
          {[0, 1, 2, 3].map((i) => <div key={i} className="h-40 animate-pulse rounded-xl bg-muted/50" />)}
        </div>
      ) : items.length === 0 ? (
        <div className="grid h-40 place-content-center text-sm text-muted-foreground">No charts.</div>
      ) : (
        <>
          <TabBar tabs={data?.tabs ?? []} selected={tab} onSelect={setTabId} />
          <DashboardGrid items={items} layout={data?.layout ?? {}} editable={false} onLayoutChange={() => {}}
            tabs={data?.tabs} tab={tab} phone={data?.phoneLayout} foldKey={data?.board.id} />
        </>
      )}
    </div>
    </ReportingProvider>
  );
}
