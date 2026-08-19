"use client";

import * as React from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { useTheme } from "next-themes";
import { RefreshCw, Sparkles, Download, Pencil, Eye, Copy, Trash2, MoreHorizontal } from "lucide-react";
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
  const notifyChange = () => { try { window.dispatchEvent(new Event("dashboards:changed")); } catch { /* ignore */ } };

  const load = React.useCallback(async () => {
    setLoading(true); setError(null);
    try {
      const q = new URLSearchParams({ board });
      if (year !== "all") q.set("year", year);
      if (!adoptingRef.current && filtersRef.current.length) q.set("filters", JSON.stringify(filtersRef.current));
      const res = await fetch(`/api/dashboard?${q.toString()}`, { cache: "no-store" });
      const json = (await res.json()) as Payload;
      if (!res.ok) throw new Error((json as { error?: string }).error ?? "Gagal memuat dashboard");
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
  React.useEffect(() => { if (isDefault) setEdit(false); }, [isDefault]);

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

  const boards = data?.boards ?? [{ id: "default", name: "Utama" }];
  const dashName = boards.find((b) => b.id === board)?.name ?? "Dashboards";
  const kpis = data?.kpis ?? [];
  const charts = data?.charts ?? [];

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
    <div className="flex flex-col gap-4">
      <PageHeader
        title={dashName}
        description={isDefault ? "Dashboard bawaan (contoh). Buat dashboard sendiri dari sidebar untuk mengatur tata letak." : "Kanvas dashboard — seret & ubah ukuran tile di mode Edit. Tersimpan otomatis."}
        actions={
          <>
            <Select value={year} onValueChange={(v) => setYear(v ?? "all")}>
              <SelectTrigger className="h-7 w-[120px] text-xs"><SelectValue /></SelectTrigger>
              <SelectContent>{YEARS.map((y) => <SelectItem key={y} value={y}>{y === "all" ? "Semua tahun" : `Tahun ${y}`}</SelectItem>)}</SelectContent>
            </Select>
            {!isDefault ? (
              <Button variant={edit ? "default" : "outline"} size="sm" onClick={() => setEdit((e) => !e)}>
                {edit ? <Eye className="size-4" /> : <Pencil className="size-4" />}{edit ? "Selesai" : "Edit"}
              </Button>
            ) : null}
            <ChartBuilder board={board} boards={boards} onSaved={load} />
            <div className="relative">
              <Button variant="outline" size="sm" onClick={() => setMenuOpen((o) => !o)} aria-label="Menu"><MoreHorizontal className="size-4" /></Button>
              {menuOpen ? (
                <>
                  <div className="fixed inset-0 z-10" onClick={() => setMenuOpen(false)} />
                  <div className="absolute right-0 z-20 mt-1 w-44 rounded-lg border border-border bg-card p-1 shadow-xl">
                    {!isDefault ? (
                      <button onClick={() => { setNewName(dashName); setRenameOpen(true); setMenuOpen(false); }} className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm hover:bg-muted"><Pencil className="size-4" /> Ganti nama</button>
                    ) : null}
                    <a href="/api/dashboard/export" download className="flex items-center gap-2 rounded-md px-2.5 py-1.5 text-sm hover:bg-muted"><Download className="size-4" /> Ekspor YAML</a>
                    <button onClick={() => void duplicateDashboard()} className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm hover:bg-muted"><Copy className="size-4" /> Duplikat</button>
                    {!isDefault ? (
                      <button onClick={() => void deleteDashboard()} className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm text-destructive hover:bg-destructive/10"><Trash2 className="size-4" /> Hapus dashboard</button>
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
      {data?.storeError ? <div className="rounded-lg border border-amber-500/40 bg-amber-500/5 px-4 py-2 text-xs text-amber-600 dark:text-amber-400">Chart tersimpan tak bisa dimuat: {data.storeError}</div> : null}

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
          Dashboard ini masih kosong. Klik <span className="font-medium text-foreground">Chart baru</span> atau minta AI Copilot: <span className="font-medium text-foreground">“bikin chart …”</span>.
        </div>
      ) : (
        <DashboardGrid items={items} layout={layout} editable={edit && !isDefault} onLayoutChange={persistLayout} />
      )}

      {editing ? (
        <ChartBuilder hideTrigger open={!!editing} onOpenChange={(o) => { if (!o) setEditing(null); }}
          editId={editing.id} initial={editing.def} board={board} boards={boards}
          onSaved={() => { setEditing(null); void load(); }} />
      ) : null}

      <Dialog open={renameOpen} onOpenChange={setRenameOpen}>
        <DialogContent className="sm:max-w-sm">
          <DialogHeader><DialogTitle>Ganti nama dashboard</DialogTitle></DialogHeader>
          <Input autoFocus value={newName} onChange={(e) => setNewName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") void saveRename(); }} placeholder="Nama dashboard" />
          <DialogFooter>
            <DialogClose render={<Button variant="ghost" size="sm" />}>Batal</DialogClose>
            <Button size="sm" onClick={() => void saveRename()}>Simpan</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
