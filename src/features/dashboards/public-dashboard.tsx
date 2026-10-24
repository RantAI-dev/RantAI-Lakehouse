"use client";

import type { ReportingContext } from "@/lib/time-grain";
import * as React from "react";
import { useTheme } from "next-themes";
import Image from "next/image";
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
import { LinkGone, PasswordGate, openPublic } from "./public-link-access";

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
 * The PUBLIC read-only page for a dashboard (opened via a share token, no
 * login, no console chrome). Fit for sending to a manager / outside party:
 * they just open the link, see the KPIs & charts, and toggle light/dark
 * theme. They cannot edit anything.
 */
export function PublicDashboard({ token }: { token: string }) {
  const { resolvedTheme, setTheme } = useTheme();
  const [mounted, setMounted] = React.useState(false);
  React.useEffect(() => setMounted(true), []);
  const appDark = resolvedTheme === "dark";
  const [data, setData] = React.useState<Payload | null>(null);
  // BI-17 part A (D13): the dashboard's own theme and font apply on a public link too.
  const look = dashboardLook(data?.board, appDark, data?.reporting?.fonts);
  const dark = look.dark;
  const fixedTheme = look.className !== "";
  const [tabId, setTabId] = React.useState<string | null>(null);
  const [state, setState] = React.useState<"loading" | "ok" | "notfound" | "password" | "error">("loading");
  // Bumped when the password page succeeds, so the dashboard is asked for again.
  const [attempt, setAttempt] = React.useState(0);

  React.useEffect(() => {
    let alive = true;
    (async () => {
      try {
        // `openPublic` is deliberately NOT `apiFetch`: this is the
        // unauthenticated public share view, and `apiFetch`'s 401 -> /login
        // redirect would be wrong for a viewer who has no account.
        const result = await openPublic<Payload>(token);
        if (!alive) return;
        if (result.kind === "gone") { setState("notfound"); return; }
        if (result.kind === "password") { setState("password"); return; }
        if (result.kind === "error") { setState("error"); return; }
        setData(result.data);
        setState("ok");
      } catch {
        if (alive) setState("error");
      }
    })();
    return () => { alive = false; };
  }, [token, attempt]);

  if (state === "notfound") return <LinkGone />;
  if (state === "password") {
    return <PasswordGate token={token} onOpen={() => { setState("loading"); setAttempt((n) => n + 1); }} />;
  }
  if (state === "error") {
    return (
      <Centered>
        <p className="text-lg font-semibold">Could not load dashboard</p>
        <p className="mt-1 text-sm text-muted-foreground">Please try again later.</p>
      </Centered>
    );
  }

  const tab = selectedTab(data?.tabs ?? [], tabId);
  const items: GridItem[] = (data?.charts ?? []).map((spec) => ({
    kind: spec.kind,
    id: spec.id,
    title: spec.title,
    badge: <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">{spec.kind}</span>,
    // BI-6: the chart's own filters, listed (the viewer cannot change them).
    hint: ownFilters(spec) ? <OwnFiltersMarker filters={ownFilters(spec) ?? []} /> : null,
    body: <TileBody spec={spec} cell={data?.results[spec.id]} dark={dark} loading={state === "loading"} />,
  }));

  return (
    <ReportingProvider reporting={data?.reporting}>
    <div className={cn("min-h-screen bg-muted/25", look.className)} style={look.style}>
      {/* Compact header — brand + title + theme toggle */}
      <header className="sticky top-0 z-10 flex items-center gap-3 border-b border-border bg-card/80 px-4 py-3 backdrop-blur-md sm:px-6">
        <span className="relative block h-6 w-[116px] shrink-0">
          <Image src="/logo-light.png" alt="Rantai Lake" fill sizes="116px" className="object-contain object-left dark:hidden" priority />
          <Image src="/logo-dark.png" alt="" fill sizes="116px" className="hidden object-contain object-left dark:block" priority />
        </span>
        <span className="hidden h-5 w-px bg-border sm:block" />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold leading-tight">{data?.board.name ?? "Dashboard"}</p>
          <p className="text-[11px] text-muted-foreground">Shared dashboard · read-only</p>
        </div>
        {/* An editor who fixed the theme has chosen how this dashboard reads; the viewer's toggle would undo it. */}
        {fixedTheme ? null : <button
          type="button"
          suppressHydrationWarning
          onClick={() => setTheme(appDark ? "light" : "dark")}
          className="rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          {mounted ? (appDark ? "Light" : "Dark") : "Theme"}
        </button>}
      </header>

      <main className="mx-auto w-full max-w-7xl px-4 py-5 sm:px-6">
        {state === "loading" ? (
          <div className="grid gap-3 sm:grid-cols-2">
            {[0, 1, 2, 3].map((i) => <div key={i} className="h-40 animate-pulse rounded-xl bg-muted/50" />)}
          </div>
        ) : items.length === 0 ? (
          <Centered><p className="text-sm text-muted-foreground">This dashboard has no charts yet.</p></Centered>
        ) : (
          <>
          <TabBar tabs={data?.tabs ?? []} selected={tab} onSelect={setTabId} />
          <DashboardGrid items={items} layout={data?.layout ?? {}} editable={false} onLayoutChange={() => {}}
            tabs={data?.tabs} tab={tab} phone={data?.phoneLayout} foldKey={data?.board.id} />
        </>
        )}
      </main>

      <footer className="border-t border-border px-4 py-4 text-center text-[11px] text-muted-foreground sm:px-6">
        Powered by Rantai Lake — Enterprise Lakehouse Console
      </footer>
    </div>
    </ReportingProvider>
  );
}

function Centered({ children }: { children: React.ReactNode }) {
  return <div className="grid min-h-[60vh] place-content-center px-4 text-center">{children}</div>;
}
