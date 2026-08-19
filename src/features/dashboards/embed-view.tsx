"use client";

import * as React from "react";
import { useTheme } from "next-themes";
import { DashboardGrid, type GridItem } from "./dashboard-grid";
import { TileBody } from "./tile-body";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";
import type { LayoutMap } from "@/services/clients/bi-store";

type Cell = { columns: string[]; rows: Record<string, unknown>[] } | { error: string };
type Payload = {
  board: { id: string; name: string };
  layout: LayoutMap;
  charts: (ChartRenderSpec & { text?: string; caption?: string })[];
  results: Record<string, Cell>;
};

/**
 * View EMBED — dirancang untuk di dalam <iframe> di situs/app lain (ala
 * embed publik Metabase). Tanpa header/footer/chrome. Dua mode:
 *  - seluruh dashboard (grid) — default
 *  - satu chart saja bila `chartId` diberikan (?chart=<id>) — mengisi iframe.
 * Read-only, data dari mart Gold. Latar transparan agar menyatu dgn host.
 */
export function EmbedView({ token, chartId }: { token: string; chartId?: string }) {
  const { resolvedTheme } = useTheme();
  const dark = resolvedTheme === "dark";
  const [data, setData] = React.useState<Payload | null>(null);
  const [state, setState] = React.useState<"loading" | "ok" | "notfound" | "error">("loading");

  React.useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const res = await fetch(`/api/public/dashboard/${encodeURIComponent(token)}`, { cache: "no-store" });
        if (!alive) return;
        if (res.status === 404) { setState("notfound"); return; }
        if (!res.ok) { setState("error"); return; }
        setData(await res.json());
        setState("ok");
      } catch { if (alive) setState("error"); }
    })();
    return () => { alive = false; };
  }, [token]);

  if (state === "notfound" || state === "error") {
    return <div className="grid h-screen place-content-center px-4 text-center text-sm text-muted-foreground">Dashboard not available.</div>;
  }

  const charts = data?.charts ?? [];

  // ── Mode: satu chart mengisi iframe ────────────────────────────────────────
  if (chartId) {
    const spec = charts.find((c) => c.id === chartId);
    if (state === "ok" && !spec) {
      return <div className="grid h-screen place-content-center text-sm text-muted-foreground">Chart not found.</div>;
    }
    return (
      <div className="flex h-screen flex-col overflow-hidden bg-transparent p-2">
        <div className="flex h-full flex-col overflow-hidden rounded-xl border border-border bg-card">
          <div className="flex items-center gap-2 border-b border-border px-3 py-1.5">
            <p className="min-w-0 flex-1 truncate text-sm font-semibold">{spec?.title ?? "…"}</p>
            <a href={`/public/dashboard/${token}`} target="_blank" rel="noopener noreferrer"
               className="text-[10px] font-medium text-muted-foreground hover:text-foreground">Rantai Lake ↗</a>
          </div>
          <div className="min-h-0 flex-1 p-2">
            {spec ? <TileBody spec={spec} cell={data?.results[spec.id]} dark={dark} loading={state === "loading"} year="all" /> : null}
          </div>
        </div>
      </div>
    );
  }

  // ── Mode: seluruh dashboard ────────────────────────────────────────────────
  const items: GridItem[] = charts.map((spec) => ({
    id: spec.id,
    title: spec.title,
    badge: <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">{spec.kind}</span>,
    body: <TileBody spec={spec} cell={data?.results[spec.id]} dark={dark} loading={state === "loading"} year="all" />,
  }));

  return (
    <div className="min-h-screen bg-transparent p-2">
      {state === "loading" ? (
        <div className="grid gap-3 sm:grid-cols-2">
          {[0, 1, 2, 3].map((i) => <div key={i} className="h-40 animate-pulse rounded-xl bg-muted/50" />)}
        </div>
      ) : items.length === 0 ? (
        <div className="grid h-40 place-content-center text-sm text-muted-foreground">No charts.</div>
      ) : (
        <DashboardGrid items={items} layout={data?.layout ?? {}} editable={false} onLayoutChange={() => {}} />
      )}
    </div>
  );
}
