"use client";

import * as React from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { useTheme } from "next-themes";
import { RefreshCw, Sparkles, Download, Pencil, Eye, Copy, Trash2, MoreHorizontal, Maximize2, Minimize2, Plus, Move, Share2, Link2, Check, Globe, Code2 } from "lucide-react";
import { PageHeader } from "@/components/patterns/page-header";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogClose,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import type { ChartRenderSpec, ChartSource } from "@/lib/dashboard-specs";
import type { LayoutMap, FilterDef } from "@/services/clients/bi-store";
import { fmtInt } from "./chart-option";
import { ChartBuilder, type ChartDef } from "./chart-builder";
import { DashboardGrid, type GridItem } from "./dashboard-grid";
import { TileBody } from "./tile-body";
import { DashboardFilters } from "./dashboard-filters";
import { useCopilot } from "@/features/copilot/use-copilot";

type Cell = { columns: string[]; rows: Record<string, unknown>[] } | { error: string };
type KpiMeta = { id: string; title: string; caption?: string; format: string };
type BoardOpt = { id: string; name: string };
type ChartCard = ChartRenderSpec & { board?: string; def?: ChartDef };
type Payload = {
  board: string; years: number[]; layout: LayoutMap; filters: FilterDef[]; filterColumns: string[];
  boards: BoardOpt[]; kpis: KpiMeta[];
  charts: ChartCard[]; results: Record<string, Cell>; storeError?: string | null;
};

function hasRows(c: Cell | undefined): c is { columns: string[]; rows: Record<string, unknown>[] } {
  return !!c && "rows" in c;
}
const SOURCE_BADGE: Record<ChartSource, { label: string; cls: string } | null> = {
  builtin: null,
  ai: { label: "AI", cls: "bg-violet-500/15 text-violet-600 dark:text-violet-400" },
  ui: { label: "Manual", cls: "bg-sky-500/15 text-sky-600 dark:text-sky-400" },
};
const YEARS = ["all", "2014", "2015", "2016", "2017", "2018", "2019", "2020", "2021", "2022", "2023", "2024", "2025", "2026"];

/**
 * Dashboard ala Tableau/Metabase — kanvas tile drag/resize, multi-dashboard
 * (dipilih via ?board=, dikelola dari sidebar). Mode Edit menata tata letak
 * (disimpan ke lakehouse); mode Lihat presentasi bersih. Kartu manual & AI.
 */
export function DashboardPage() {
  const router = useRouter();
  const params = useSearchParams();
  const board = params.get("board") || "default";
  const isDefault = board === "default";

  const { resolvedTheme } = useTheme();
  const dark = resolvedTheme === "dark";
  const [data, setData] = React.useState<Payload | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [year, setYear] = React.useState("all");
  const [edit, setEdit] = React.useState(false);
  const [layout, setLayout] = React.useState<LayoutMap>({});
  const [filters, setFilters] = React.useState<FilterDef[]>([]);
  const filtersRef = React.useRef<FilterDef[]>([]);
  const adoptingRef = React.useRef(true);
  const [editing, setEditing] = React.useState<{ id: string; def: ChartDef } | null>(null);
  const [menuOpen, setMenuOpen] = React.useState(false);
  const [renameOpen, setRenameOpen] = React.useState(false);
  const [newName, setNewName] = React.useState("");
  const [fullscreen, setFullscreen] = React.useState(false);
  const [autoSec, setAutoSec] = React.useState("0");
  const [shareOpen, setShareOpen] = React.useState(false);
  const [shareToken, setShareToken] = React.useState("");
  const [shareBusy, setShareBusy] = React.useState(false);
  const [copied, setCopied] = React.useState<string | false>(false);
  const notifyChange = () => { try { window.dispatchEvent(new Event("dashboards:changed")); } catch { /* ignore */ } };

  const load = React.useCallback(async () => {
    setLoading(true); setError(null);
    try {
      const q = new URLSearchParams({ board });
      if (year !== "all") q.set("year", year);
      if (!adoptingRef.current && filtersRef.current.length) q.set("filters", JSON.stringify(filtersRef.current));
      const res = await fetch(`/api/dashboard?${q.toString()}`, { cache: "no-store" });
      const json = (await res.json()) as Payload;
      if (!res.ok) throw new Error((json as { error?: string }).error ?? "Failed to load dashboard");
      setData(json);
      setLayout(json.layout ?? {});
      if (adoptingRef.current) {
        filtersRef.current = json.filters ?? [];
        setFilters(json.filters ?? []);
        adoptingRef.current = false;
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [board, year]);

  // Ganti dashboard → adopsi ulang filter tersimpan board itu.
  React.useEffect(() => { adoptingRef.current = true; filtersRef.current = []; setFilters([]); }, [board]);
  React.useEffect(() => { void load(); }, [load]);

  // Auto-refresh berkala (presentasi).
  React.useEffect(() => {
    const s = Number(autoSec);
    if (!s) return;
    const t = setInterval(() => void load(), s * 1000);
    return () => clearInterval(t);
  }, [autoSec, load]);
  // Esc keluar fullscreen.
  React.useEffect(() => {
    if (!fullscreen) return;
    const h = (e: KeyboardEvent) => { if (e.key === "Escape") setFullscreen(false); };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [fullscreen]);

  const applyFilters = React.useCallback((next: FilterDef[]) => {
    filtersRef.current = next;
    setFilters(next);
    if (!isDefault) {
      void fetch("/api/dashboard/boards", {
        method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ id: board, filters: next }),
      });
    }
    void load();
  }, [board, isDefault, load]);
  // Start in VIEW mode; user clicks "Edit layout" to arrange. Reset on board switch.
  React.useEffect(() => { setEdit(false); }, [board]);
  // Buka /dashboards (demo) → langsung ke dashboard user terbaru bila ada.
  React.useEffect(() => {
    if (data && isDefault && data.boards.length > 1) {
      router.replace(`/dashboards?board=${data.boards[data.boards.length - 1].id}`);
    }
  }, [data, isDefault, router]);

  // Simpan layout (debounced) untuk dashboard user.
  const saveTimer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const persistLayout = React.useCallback((next: LayoutMap) => {
    setLayout(next);
    if (isDefault) return;
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      void fetch("/api/dashboard/boards", {
        method: "PUT", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ id: board, layout: next }),
      });
    }, 600);
  }, [board, isDefault]);

  async function remove(id: string) {
    await fetch(`/api/dashboard/specs?id=${encodeURIComponent(id)}`, { method: "DELETE" });
    void load();
  }
  async function duplicateDashboard() {
    setMenuOpen(false);
    const res = await fetch("/api/dashboard/boards", {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ duplicate: board }),
    });
    const json = await res.json();
    notifyChange();
    if (json?.board?.id) router.push(`/dashboards?board=${json.board.id}`);
  }
  async function deleteDashboard() {
    setMenuOpen(false);
    if (isDefault) return;
    await fetch(`/api/dashboard/boards?id=${encodeURIComponent(board)}`, { method: "DELETE" });
    notifyChange();
    router.push("/dashboards");
  }
  async function newDashboard() {
    const res = await fetch("/api/dashboard/boards", {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ name: "New dashboard" }),
    });
    const json = await res.json();
    notifyChange();
    if (json?.board?.id) router.push(`/dashboards?board=${json.board.id}`);
  }
  // ── Share (public read-only link) ─────────────────────────────────────────
  async function openShare() {
    setMenuOpen(false);
    if (isDefault) return;
    setCopied(false);
    try {
      const res = await fetch("/api/dashboard/boards", { cache: "no-store" });
      const json = await res.json();
      const b = (json?.boards ?? []).find((x: { id: string; publicToken?: string }) => x.id === board);
      setShareToken(b?.publicToken ?? "");
    } catch { setShareToken(""); }
    setShareOpen(true);
  }
  async function setPublic(enable: boolean) {
    setShareBusy(true);
    try {
      const res = await fetch("/api/dashboard/boards", {
        method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ id: board, public: enable }),
      });
      const json = await res.json();
      setShareToken(typeof json?.publicToken === "string" ? json.publicToken : "");
      setCopied(false);
    } finally { setShareBusy(false); }
  }
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  const shareUrl = shareToken ? `${origin}/public/dashboard/${shareToken}` : "";
  const embedDashUrl = shareToken ? `${origin}/embed/dashboard/${shareToken}` : "";
  const embedIframe = shareToken
    ? `<iframe src="${embedDashUrl}" width="100%" height="600" frameborder="0" style="border:1px solid #e5e7eb;border-radius:12px" title="Rantai Lake dashboard"></iframe>`
    : "";
  async function copyText(text: string, key: string) {
    if (!text) return;
    try { await navigator.clipboard.writeText(text); setCopied(key); setTimeout(() => setCopied(false), 1800); } catch { /* ignore */ }
  }

  async function saveRename() {
    const name = newName.trim();
    if (!name || isDefault) return;
    await fetch("/api/dashboard/boards", {
      method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ id: board, name }),
    });
    setRenameOpen(false);
    notifyChange();
    void load();
  }

  const boards = data?.boards ?? [{ id: "default", name: "Main" }];
  const dashName = boards.find((b) => b.id === board)?.name ?? "Dashboards";
  const kpis = data?.kpis ?? [];
  const charts = data?.charts ?? [];

  // Publish page context to the Copilot so it's aware of THIS dashboard.
  const { setPageContext } = useCopilot();
  React.useEffect(() => {
    if (isDefault) { setPageContext(null); return; }
    const tiles = charts.map((c) => `"${c.title}" (${c.kind}${c.source !== "builtin" ? `, id ${c.id}` : ""})`).join("; ");
    setPageContext({
      key: "dashboard-view",
      title: `Working on "${dashName}"`,
      hint: "Create a chart, edit a tile, or ask about this dashboard.",
      suggest: {
        ask: ["Explain the charts on this dashboard", "Which category leads here?"],
        build: ["Add a KPI of total visitors", "Add a table of top countries", "Add a pie of visitors by region"],
      },
      system:
        `The user is viewing the dashboard "${dashName}" (board id: ${board}). ` +
        `Tiles: ${tiles || "none yet"}. ` +
        `When creating a chart use board="${board}". To change a tile, use update_chart with its id. ` +
        `You can also explain what the charts show.`,
    });
    return () => setPageContext(null);
  }, [board, isDefault, dashName, charts, setPageContext]);

  // Bangun tile untuk grid.
  const items: GridItem[] = charts.map((spec) => {
    const cell = data?.results[spec.id];
    const badge = SOURCE_BADGE[spec.source];
    return {
      id: spec.id,
      title: spec.title,
      subtitle: spec.subtitle,
      badge: (
        <div className="flex items-center gap-1.5">
          {badge ? (
            <span className={cn("inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium", badge.cls)}>
              {spec.source === "ai" ? <Sparkles className="size-2.5" /> : null}{badge.label}
            </span>
          ) : null}
          <span className="hidden rounded-full border px-2 py-0.5 font-mono text-[10px] text-muted-foreground sm:inline">{spec.mart}</span>
        </div>
      ),
      onEdit: spec.source !== "builtin" && spec.def ? () => setEditing({ id: spec.id, def: spec.def as ChartDef }) : undefined,
      onRemove: spec.source !== "builtin" ? () => void remove(spec.id) : undefined,
      body: <TileBody spec={spec} cell={cell} dark={dark} loading={loading} year={year} />,
    };
  });

  return (
    <div className={cn("flex flex-col gap-4", fullscreen && "fixed inset-0 z-40 overflow-auto bg-background p-4 sm:p-6")}>
      <PageHeader
        title={dashName}
        description={isDefault ? "Built-in dashboard (demo). Create your own dashboard from the sidebar to arrange the layout." : "Dashboard canvas — drag & resize tiles in Edit mode. Saved automatically."}
        actions={
          <>
            <Select value={year} onValueChange={(v) => setYear(v ?? "all")}>
              <SelectTrigger className="h-7 w-[120px] text-xs"><SelectValue /></SelectTrigger>
              <SelectContent>{YEARS.map((y) => <SelectItem key={y} value={y}>{y === "all" ? "All years" : `Year ${y}`}</SelectItem>)}</SelectContent>
            </Select>
            <Select value={autoSec} onValueChange={(v) => setAutoSec(v ?? "0")}>
              <SelectTrigger className="h-7 w-[110px] text-xs"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="0">Manual</SelectItem>
                <SelectItem value="30">Every 30s</SelectItem>
                <SelectItem value="60">Every 1m</SelectItem>
                <SelectItem value="300">Every 5m</SelectItem>
              </SelectContent>
            </Select>
            <Button variant="outline" size="sm" onClick={() => setFullscreen((f) => !f)} aria-label="Fullscreen">
              {fullscreen ? <Minimize2 className="size-4" /> : <Maximize2 className="size-4" />}
            </Button>
            {isDefault ? (
              <Button size="sm" onClick={() => void newDashboard()}><Plus className="size-4" /> New dashboard</Button>
            ) : (
              <Button variant={edit ? "default" : "outline"} size="sm" onClick={() => setEdit((e) => !e)}>
                {edit ? <Eye className="size-4" /> : <Pencil className="size-4" />}{edit ? "Done" : "Edit layout"}
              </Button>
            )}
            <ChartBuilder board={board} boards={boards} onSaved={load} />
            <div className="relative">
              <Button variant="outline" size="sm" onClick={() => setMenuOpen((o) => !o)} aria-label="Menu"><MoreHorizontal className="size-4" /></Button>
              {menuOpen ? (
                <>
                  <div className="fixed inset-0 z-10" onClick={() => setMenuOpen(false)} />
                  <div className="absolute right-0 z-20 mt-1 w-44 rounded-lg border border-border bg-card p-1 shadow-xl">
                    {!isDefault ? (
                      <button onClick={() => { setNewName(dashName); setRenameOpen(true); setMenuOpen(false); }} className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm hover:bg-muted"><Pencil className="size-4" /> Rename</button>
                    ) : null}
                    {!isDefault ? (
                      <button onClick={() => void openShare()} className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm hover:bg-muted"><Share2 className="size-4" /> Share…</button>
                    ) : null}
                    <a href="/api/dashboard/export" download className="flex items-center gap-2 rounded-md px-2.5 py-1.5 text-sm hover:bg-muted"><Download className="size-4" /> Export YAML</a>
                    <button onClick={() => void duplicateDashboard()} className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm hover:bg-muted"><Copy className="size-4" /> Duplicate</button>
                    {!isDefault ? (
                      <button onClick={() => void deleteDashboard()} className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm text-destructive hover:bg-destructive/10"><Trash2 className="size-4" /> Delete dashboard</button>
                    ) : null}
                  </div>
                </>
              ) : null}
            </div>
            <Button variant="outline" size="sm" onClick={() => void load()} disabled={loading}><RefreshCw className={cn("size-4", loading && "animate-spin")} /></Button>
          </>
        }
      />

      {error ? <div className="rounded-lg border border-destructive/40 bg-destructive/5 px-4 py-3 text-sm text-destructive">{error}</div> : null}
      {data?.storeError ? <div className="rounded-lg border border-amber-500/40 bg-amber-500/5 px-4 py-2 text-xs text-amber-600 dark:text-amber-400">Could not load saved charts: {data.storeError}</div> : null}

      {data?.filterColumns?.length ? (
        <div className="rounded-lg border border-border bg-card/50 px-3 py-2">
          <DashboardFilters columns={data.filterColumns} filters={filters} onChange={applyFilters} />
        </div>
      ) : null}

      {/* KPI row (dashboard bawaan) */}
      {kpis.length ? (
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
          {kpis.map((k) => {
            const cell = data?.results[k.id];
            const v = hasRows(cell) ? Number(cell.rows[0]?.v ?? 0) : null;
            return (
              <div key={k.id} className="rounded-xl border bg-card p-4 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
                <p className="text-xs font-medium text-muted-foreground">{k.title}</p>
                {loading && v === null ? <div className="mt-2 h-7 w-24 animate-pulse rounded bg-muted" /> : <p className="mt-1 text-2xl font-semibold tabular-nums text-foreground">{v === null ? "—" : fmtInt(v)}</p>}
                {k.caption ? <p className="mt-1 text-[11px] text-muted-foreground">{k.caption}</p> : null}
              </div>
            );
          })}
        </div>
      ) : null}

      {/* Kanvas */}
      {!loading && charts.length === 0 ? (
        <div className="rounded-lg border border-dashed py-16 text-center text-sm text-muted-foreground">
          This dashboard is empty. Click <span className="font-medium text-foreground">New chart</span> or ask AI Copilot: <span className="font-medium text-foreground">“create a chart …”</span>.
        </div>
      ) : (
        <>
          {edit && !isDefault ? (
            <div className="flex items-center gap-2 rounded-lg border border-dashed border-primary/30 bg-primary/5 px-3 py-1.5 text-xs text-muted-foreground">
              <Move className="size-3.5 text-primary" />
              Edit mode: <span className="font-medium text-foreground">drag the header</span> to move, <span className="font-medium text-foreground">drag the bottom-right corner</span> to resize. Use ✏️/🗑️ per tile to edit/delete. Saved automatically.
            </div>
          ) : null}
          <DashboardGrid items={items} layout={layout} editable={edit && !isDefault} onLayoutChange={persistLayout} />
        </>
      )}

      {editing ? (
        <ChartBuilder hideTrigger open={!!editing} onOpenChange={(o) => { if (!o) setEditing(null); }}
          editId={editing.id} initial={editing.def} board={board} boards={boards}
          onSaved={() => { setEditing(null); void load(); }} />
      ) : null}

      <Dialog open={shareOpen} onOpenChange={setShareOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader><DialogTitle className="flex items-center gap-2"><Share2 className="size-4" /> Share “{dashName}”</DialogTitle></DialogHeader>
          <div className="space-y-3">
            <div className="flex items-start gap-3 rounded-lg border border-border bg-muted/30 p-3">
              <Globe className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium">Public link</p>
                <p className="text-xs text-muted-foreground">Anyone with the link can view this dashboard read-only — no sign-in. Charts stay live; they cannot edit anything.</p>
              </div>
              <button
                type="button" role="switch" aria-checked={!!shareToken} disabled={shareBusy}
                onClick={() => void setPublic(!shareToken)}
                className={cn("relative mt-0.5 h-5 w-9 shrink-0 rounded-full transition-colors disabled:opacity-50", shareToken ? "bg-primary" : "bg-muted-foreground/30")}
              >
                <span className={cn("absolute top-0.5 size-4 rounded-full bg-white transition-all", shareToken ? "left-[18px]" : "left-0.5")} />
              </button>
            </div>

            {shareToken ? (
              <div className="space-y-3">
                {/* Public link */}
                <div className="space-y-1.5">
                  <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Public link</p>
                  <div className="flex items-center gap-2">
                    <div className="flex min-w-0 flex-1 items-center gap-2 rounded-md border border-border bg-background px-2.5 py-2">
                      <Link2 className="size-3.5 shrink-0 text-muted-foreground" />
                      <span className="truncate text-xs text-foreground">{shareUrl}</span>
                    </div>
                    <Button size="sm" variant="outline" onClick={() => void copyText(shareUrl, "link")}>
                      {copied === "link" ? <Check className="size-4 text-emerald-500" /> : <Copy className="size-4" />}{copied === "link" ? "Copied" : "Copy"}
                    </Button>
                  </div>
                  <a href={shareUrl} target="_blank" rel="noopener noreferrer" className="inline-block text-xs font-medium text-primary hover:underline">Open preview ↗</a>
                </div>

                {/* Embed (iframe) — ala Metabase */}
                <div className="space-y-1.5">
                  <p className="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground"><Code2 className="size-3.5" /> Embed in a website</p>
                  <div className="rounded-md border border-border bg-muted/30 p-2">
                    <code className="block max-h-20 overflow-auto whitespace-pre-wrap break-all font-mono text-[11px] leading-relaxed text-foreground">{embedIframe}</code>
                  </div>
                  <div className="flex items-center gap-2">
                    <Button size="sm" variant="outline" onClick={() => void copyText(embedIframe, "embed")}>
                      {copied === "embed" ? <Check className="size-4 text-emerald-500" /> : <Copy className="size-4" />}{copied === "embed" ? "Copied" : "Copy iframe"}
                    </Button>
                    <a href={embedDashUrl} target="_blank" rel="noopener noreferrer" className="text-xs font-medium text-primary hover:underline">Preview embed ↗</a>
                  </div>
                  <p className="text-[11px] text-muted-foreground">One chart only? Append <code className="rounded bg-muted px-1 font-mono">?chart=&lt;id&gt;</code> to the embed URL.</p>
                </div>

                <div className="flex items-center justify-between gap-2 border-t border-border pt-2">
                  <button onClick={() => void setPublic(false)} disabled={shareBusy} className="text-xs text-destructive hover:underline disabled:opacity-50">Revoke link</button>
                </div>
                <p className="rounded-md bg-amber-500/5 px-2.5 py-1.5 text-[11px] text-amber-600 dark:text-amber-400">
                  Works wherever the server is reachable. For the public internet, expose the server (e.g. Cloudflare Tunnel / reverse proxy).
                </p>
              </div>
            ) : (
              <p className="text-xs text-muted-foreground">Turn on the switch to generate a shareable link.</p>
            )}
          </div>
          <DialogFooter>
            <DialogClose render={<Button variant="ghost" size="sm" />}>Close</DialogClose>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={renameOpen} onOpenChange={setRenameOpen}>
        <DialogContent className="sm:max-w-sm">
          <DialogHeader><DialogTitle>Rename dashboard</DialogTitle></DialogHeader>
          <Input autoFocus value={newName} onChange={(e) => setNewName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") void saveRename(); }} placeholder="Dashboard name" />
          <DialogFooter>
            <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
            <Button size="sm" onClick={() => void saveRename()}>Simpan</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
