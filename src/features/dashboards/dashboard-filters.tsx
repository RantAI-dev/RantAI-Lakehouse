"use client";

import * as React from "react";
import { Filter, Plus, X, Check, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import type { FilterDef } from "@/services/clients/bi-store";

/**
 * Bar filter dashboard (lintas-tile) ala Tableau/Metabase. Pilih kolom dimensi
 * → nilai (multi-select) → menyaring SEMUA tile yang punya kolom itu. Filter
 * aktif tampil sebagai chip. Berlaku live & (di dashboard user) tersimpan.
 */
export function DashboardFilters({
  columns, filters, onChange,
}: {
  columns: string[];
  filters: FilterDef[];
  onChange: (next: FilterDef[]) => void;
}) {
  const [editing, setEditing] = React.useState<string | null>(null); // kolom yang lagi dibuka
  const [valuesList, setValuesList] = React.useState<string[]>([]);
  const [picked, setPicked] = React.useState<Set<string>>(new Set());
  const [loading, setLoading] = React.useState(false);
  const ref = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    if (!editing) return;
    const onDoc = (e: MouseEvent) => { if (ref.current && !ref.current.contains(e.target as Node)) setEditing(null); };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [editing]);

  const openColumn = async (col: string) => {
    setEditing(col);
    setPicked(new Set(filters.find((f) => f.column === col)?.values ?? []));
    setLoading(true);
    try {
      const j = await fetch(`/api/dashboard/values?column=${encodeURIComponent(col)}`).then((r) => r.json());
      setValuesList(j.values ?? []);
    } catch { setValuesList([]); } finally { setLoading(false); }
  };

  const apply = (col: string) => {
    const values = [...picked];
    const rest = filters.filter((f) => f.column !== col);
    onChange(values.length ? [...rest, { column: col, values }] : rest);
    setEditing(null);
  };
  const removeFilter = (col: string) => onChange(filters.filter((f) => f.column !== col));
  const available = columns.filter((c) => !filters.some((f) => f.column === c));

  return (
    <div className="flex flex-wrap items-center gap-1.5" ref={ref}>
      <span className="flex items-center gap-1 text-xs text-muted-foreground"><Filter className="size-3.5" /> Filter:</span>

      {/* Chip filter aktif */}
      {filters.map((f) => (
        <div key={f.column} className="relative">
          <button
            onClick={() => (editing === f.column ? setEditing(null) : void openColumn(f.column))}
            className="inline-flex items-center gap-1 rounded-full border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary"
          >
            <span className="font-medium">{f.column}</span>
            <span className="max-w-[120px] truncate opacity-80">= {f.values.join(", ")}</span>
            <ChevronDown className="size-3" />
          </button>
          <span onClick={() => removeFilter(f.column)} role="button" aria-label="Hapus filter" className="absolute -right-1 -top-1 grid size-4 cursor-pointer place-items-center rounded-full border bg-background text-muted-foreground hover:text-destructive">
            <X className="size-2.5" />
          </span>
          {editing === f.column ? <ValuePanel col={f.column} loading={loading} valuesList={valuesList} picked={picked} setPicked={setPicked} onApply={() => apply(f.column)} /> : null}
        </div>
      ))}

      {/* Tambah filter */}
      {available.length ? (
        <div className="relative">
          <button
            onClick={() => setEditing(editing === "__add__" ? null : "__add__")}
            className="inline-flex items-center gap-1 rounded-full border border-dashed px-2.5 py-1 text-xs text-muted-foreground hover:bg-muted"
          >
            <Plus className="size-3" /> Filter
          </button>
          {editing === "__add__" ? (
            <div className="absolute left-0 top-full z-20 mt-1 w-44 rounded-lg border border-border bg-card p-1 shadow-xl">
              <p className="px-2 py-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Kolom</p>
              {available.map((c) => (
                <button key={c} onClick={() => void openColumn(c)} className="block w-full rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted">{c}</button>
              ))}
            </div>
          ) : null}
          {editing && editing !== "__add__" && !filters.some((f) => f.column === editing) ? (
            <ValuePanel col={editing} loading={loading} valuesList={valuesList} picked={picked} setPicked={setPicked} onApply={() => apply(editing)} />
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function ValuePanel({
  col, loading, valuesList, picked, setPicked, onApply,
}: {
  col: string; loading: boolean; valuesList: string[]; picked: Set<string>;
  setPicked: (s: Set<string>) => void; onApply: () => void;
}) {
  const toggle = (v: string) => {
    const n = new Set(picked);
    if (n.has(v)) n.delete(v); else n.add(v);
    setPicked(n);
  };
  return (
    <div className="absolute left-0 top-full z-20 mt-1 w-56 rounded-lg border border-border bg-card p-1 shadow-xl">
      <p className="px-2 py-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">{col}</p>
      <div className="max-h-56 overflow-y-auto">
        {loading ? <p className="px-2 py-2 text-xs text-muted-foreground">Memuat…</p> :
          valuesList.length === 0 ? <p className="px-2 py-2 text-xs text-muted-foreground">Tak ada nilai.</p> :
          valuesList.map((v) => {
            const on = picked.has(v);
            return (
              <button key={v} onClick={() => toggle(v)} className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted">
                <span className={cn("grid size-4 shrink-0 place-items-center rounded border", on ? "border-primary bg-primary text-primary-foreground" : "border-border")}>{on ? <Check className="size-3" /> : null}</span>
                <span className="truncate">{v}</span>
              </button>
            );
          })}
      </div>
      <div className="flex justify-end gap-1 border-t border-border p-1">
        <button onClick={onApply} className="rounded-md bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground hover:bg-primary/85">Terapkan</button>
      </div>
    </div>
  );
}
