"use client";

import * as React from "react";
import { useTheme } from "next-themes";
import Image from "next/image";
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
 * Halaman PUBLIK read-only sebuah dashboard (dibuka lewat token share, tanpa
 * login, tanpa chrome konsol). Cocok dikirim ke atasan / pihak luar: mereka
 * cukup buka link, lihat KPI & chart, ganti tema terang/gelap. Tidak bisa
 * mengedit apa pun.
 */
export function PublicDashboard({ token }: { token: string }) {
  const { resolvedTheme, setTheme } = useTheme();
  const [mounted, setMounted] = React.useState(false);
  React.useEffect(() => setMounted(true), []);
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
      } catch {
        if (alive) setState("error");
      }
    })();
    return () => { alive = false; };
  }, [token]);

  if (state === "notfound") {
    return (
      <Centered>
        <p className="text-lg font-semibold">Dashboard not available</p>
        <p className="mt-1 text-sm text-muted-foreground">This link has been revoked or never existed.</p>
      </Centered>
    );
  }
  if (state === "error") {
    return (
      <Centered>
        <p className="text-lg font-semibold">Could not load dashboard</p>
        <p className="mt-1 text-sm text-muted-foreground">Please try again later.</p>
      </Centered>
    );
  }

  const items: GridItem[] = (data?.charts ?? []).map((spec) => ({
    id: spec.id,
    title: spec.title,
    badge: <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">{spec.kind}</span>,
    body: <TileBody spec={spec} cell={data?.results[spec.id]} dark={dark} loading={state === "loading"} year="all" />,
  }));

  return (
    <div className="min-h-screen bg-muted/25">
      {/* Header ringkas — brand + judul + toggle tema */}
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
        <button
          type="button"
          suppressHydrationWarning
          onClick={() => setTheme(dark ? "light" : "dark")}
          className="rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          {mounted ? (dark ? "Light" : "Dark") : "Theme"}
        </button>
      </header>

      <main className="mx-auto w-full max-w-7xl px-4 py-5 sm:px-6">
        {state === "loading" ? (
          <div className="grid gap-3 sm:grid-cols-2">
            {[0, 1, 2, 3].map((i) => <div key={i} className="h-40 animate-pulse rounded-xl bg-muted/50" />)}
          </div>
        ) : items.length === 0 ? (
          <Centered><p className="text-sm text-muted-foreground">This dashboard has no charts yet.</p></Centered>
        ) : (
          <DashboardGrid items={items} layout={data?.layout ?? {}} editable={false} onLayoutChange={() => {}} />
        )}
      </main>

      <footer className="border-t border-border px-4 py-4 text-center text-[11px] text-muted-foreground sm:px-6">
        Powered by Rantai Lake — Enterprise Lakehouse Console
      </footer>
    </div>
  );
}

function Centered({ children }: { children: React.ReactNode }) {
  return <div className="grid min-h-[60vh] place-content-center px-4 text-center">{children}</div>;
}
