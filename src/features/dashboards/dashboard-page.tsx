"use client";

import * as React from "react";
import { useTheme } from "next-themes";
import { RefreshCw, Trash2, Sparkles, Pencil, Plus, Download } from "lucide-react";
import { PageHeader } from "@/components/patterns/page-header";
import { SectionCard } from "@/components/patterns/section-card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogClose, DialogTrigger,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import type { ChartRenderSpec, ChartSource } from "@/lib/dashboard-specs";
import { EChart } from "./echart";
import { buildOption, fmtInt } from "./chart-option";
import { ChartBuilder, type ChartDef } from "./chart-builder";

type Cell = { columns: string[]; rows: Record<string, unknown>[] } | { error: string };
type KpiMeta = { id: string; title: string; caption?: string; format: string };
type BoardOpt = { id: string; name: string };
type ChartCard = ChartRenderSpec & { board?: string; def?: ChartDef };
type Payload = {
  board: string; years: number[]; boards: BoardOpt[]; kpis: KpiMeta[];
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
 * Dashboard visual lakehouse — "BI lakehouse". Board bernama (tabs), filter
 * tahun, builder manual + edit, dan sinkron dengan chart yang dibuat lewat chat
 * (AI). Semua kartu digerakkan semantic layer di console.bi_chart.
 */
export function DashboardPage() {
  const { resolvedTheme } = useTheme();
  const dark = resolvedTheme === "dark";
  const [data, setData] = React.useState<Payload | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [board, setBoard] = React.useState("default");
  const [year, setYear] = React.useState("all");
  const [editing, setEditing] = React.useState<{ id: string; def: ChartDef } | null>(null);
  const [newBoardName, setNewBoardName] = React.useState("");
  const [boardDialogOpen, setBoardDialogOpen] = React.useState(false);

  const load = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const q = new URLSearchParams({ board });
      if (year !== "all") q.set("year", year);
      const res = await fetch(`/api/dashboard?${q.toString()}`, { cache: "no-store" });
      const json = (await res.json()) as Payload;
      if (!res.ok) throw new Error((json as { error?: string }).error ?? "Gagal memuat dashboard");
      setData(json);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [board, year]);

  React.useEffect(() => { void load(); }, [load]);

  async function remove(id: string) {
    await fetch(`/api/dashboard/specs?id=${encodeURIComponent(id)}`, { method: "DELETE" });
    void load();
  }
  async function createBoard() {
    const name = newBoardName.trim();
    if (!name) return;
    const res = await fetch("/api/dashboard/boards", {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ name }),
    });
    const json = await res.json();
    setNewBoardName("");
    setBoardDialogOpen(false);
    if (json?.board?.id) setBoard(json.board.id);
    else void load();
  }
  async function removeBoard() {
    if (board === "default") return;
    await fetch(`/api/dashboard/boards?id=${encodeURIComponent(board)}`, { method: "DELETE" });
    setBoard("default");
  }

  const boards = data?.boards ?? [{ id: "default", name: "Utama" }];
  const kpis = data?.kpis ?? [];
  const charts = data?.charts ?? [];

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Dashboards"
        description="Board visual dari mart Gold (serving.*). Bikin manual di sini atau lewat AI Copilot (chat). Grafik oleh Apache ECharts."
        actions={
          <>
            <Select value={year} onValueChange={(v) => setYear(v ?? "all")}>
              <SelectTrigger className="h-7 w-[130px] text-xs"><SelectValue /></SelectTrigger>
              <SelectContent>
                {YEARS.map((y) => <SelectItem key={y} value={y}>{y === "all" ? "Semua tahun" : `Tahun ${y}`}</SelectItem>)}
              </SelectContent>
            </Select>
            <Button variant="outline" size="sm" render={<a href="/api/dashboard/export" download />}>
              <Download className="size-4" /> Ekspor YAML
            </Button>
            <ChartBuilder board={board} boards={boards} onSaved={load} />
            <Button variant="outline" size="sm" onClick={() => void load()} disabled={loading}>
              <RefreshCw className={cn("size-4", loading && "animate-spin")} /> Segarkan
            </Button>
          </>
        }
      />

      {/* Board tabs */}
      <div className="flex flex-wrap items-center gap-1.5 border-b border-border pb-2">
        {boards.map((b) => (
          <button
            key={b.id}
            type="button"
            onClick={() => setBoard(b.id)}
            className={cn(
              "rounded-md px-3 py-1 text-sm transition-colors",
              board === b.id ? "bg-muted font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
            )}
          >
            {b.name}
          </button>
        ))}
        <Dialog open={boardDialogOpen} onOpenChange={setBoardDialogOpen}>
          <DialogTrigger render={<Button variant="ghost" size="xs" />}>
            <Plus className="size-3.5" /> Board
          </DialogTrigger>
          <DialogContent className="sm:max-w-sm">
            <DialogHeader><DialogTitle>Board baru</DialogTitle></DialogHeader>
            <Input
              autoFocus value={newBoardName} onChange={(e) => setNewBoardName(e.target.value)}
              placeholder="mis. Wisman, Ekonomi Kreatif…"
              onKeyDown={(e) => { if (e.key === "Enter") void createBoard(); }}
            />
            <DialogFooter>
              <DialogClose render={<Button variant="ghost" size="sm" />}>Batal</DialogClose>
              <Button size="sm" onClick={() => void createBoard()}>Buat</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
        {board !== "default" ? (
          <button
            type="button" onClick={() => void removeBoard()}
            className="ml-auto rounded-md px-2 py-1 text-xs text-muted-foreground hover:text-destructive"
          >
            <Trash2 className="mr-1 inline size-3.5" /> Hapus board
          </button>
        ) : null}
      </div>

      {error ? (
        <div className="rounded-lg border border-destructive/40 bg-destructive/5 px-4 py-3 text-sm text-destructive">{error}</div>
      ) : null}
      {data?.storeError ? (
        <div className="rounded-lg border border-amber-500/40 bg-amber-500/5 px-4 py-2 text-xs text-amber-600 dark:text-amber-400">
          Chart tersimpan tak bisa dimuat: {data.storeError}
        </div>
      ) : null}

      {/* KPI row (hanya board Utama) */}
      {kpis.length ? (
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
          {kpis.map((k) => {
            const cell = data?.results[k.id];
            const v = hasRows(cell) ? Number(cell.rows[0]?.v ?? 0) : null;
            return (
              <div key={k.id} className="rounded-xl border bg-card p-4 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
                <p className="text-xs font-medium text-muted-foreground">{k.title}</p>
                {loading && v === null ? (
                  <div className="mt-2 h-7 w-24 animate-pulse rounded bg-muted" />
                ) : (
                  <p className="mt-1 text-2xl font-semibold tabular-nums text-foreground">{v === null ? "—" : fmtInt(v)}</p>
                )}
                {k.caption ? <p className="mt-1 text-[11px] text-muted-foreground">{k.caption}</p> : null}
              </div>
            );
          })}
        </div>
      ) : null}

      {/* Charts grid */}
      {!loading && charts.length === 0 ? (
        <div className="rounded-lg border border-dashed py-16 text-center text-sm text-muted-foreground">
          Board ini masih kosong. Klik <span className="font-medium text-foreground">Chart baru</span> atau minta AI Copilot:
          <span className="font-medium text-foreground"> “bikin chart …”</span>.
        </div>
      ) : null}
      <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        {charts.map((spec) => {
          const cell = data?.results[spec.id];
          const full = spec.span === 2;
          const badge = SOURCE_BADGE[spec.source];
          return (
            <SectionCard
              key={spec.id}
              title={spec.title}
              description={spec.subtitle}
              className={cn(full && "lg:col-span-2")}
              contentClassName="pt-1"
              action={
                <div className="flex items-center gap-1.5">
                  {badge ? (
                    <span className={cn("inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium", badge.cls)}>
                      {spec.source === "ai" ? <Sparkles className="size-2.5" /> : null}{badge.label}
                    </span>
                  ) : null}
                  <span className="rounded-full border px-2 py-0.5 font-mono text-[10px] text-muted-foreground">{spec.mart}</span>
                  {spec.source !== "builtin" ? (
                    <>
                      <button
                        type="button" aria-label="Ubah chart"
                        onClick={() => spec.def && setEditing({ id: spec.id, def: spec.def })}
                        className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
                      >
                        <Pencil className="size-3.5" />
                      </button>
                      <button
                        type="button" aria-label="Hapus chart" onClick={() => void remove(spec.id)}
                        className="rounded p-1 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                      >
                        <Trash2 className="size-3.5" />
                      </button>
                    </>
                  ) : null}
                </div>
              }
            >
              {loading && !cell ? (
                <div className="h-[280px] animate-pulse rounded-lg bg-muted/50" />
              ) : cell && "error" in cell ? (
                <p className="py-8 text-center text-sm text-destructive">{cell.error}</p>
              ) : hasRows(cell) && cell.rows.length ? (
                <EChart option={buildOption(spec, cell.rows, dark)} height={full ? 320 : 280} />
              ) : (
                <p className="py-8 text-center text-sm text-muted-foreground">Tak ada data{year !== "all" ? ` untuk tahun ${year}` : ""}.</p>
              )}
            </SectionCard>
          );
        })}
      </div>

      {/* Dialog edit (dikendalikan) */}
      {editing ? (
        <ChartBuilder
          hideTrigger
          open={!!editing}
          onOpenChange={(o) => { if (!o) setEditing(null); }}
          editId={editing.id}
          initial={editing.def}
          board={board}
          boards={boards}
          onSaved={() => { setEditing(null); void load(); }}
        />
      ) : null}
    </div>
  );
}
