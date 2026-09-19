"use client";

import * as React from "react";
import { Filter, Table2 } from "lucide-react";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { apiFetch } from "@/services/http";
import { RowsTable, type Rows } from "./tile-dialogs";

export type DrillTarget = { name: string; column: string; mart: string; x: number; y: number };
export type RecordsState = Rows & { value: string; loading: boolean };

/** Raw Gold rows behind one clicked value (up to 100). */
export async function fetchRecords(mart: string, column: string, value: string): Promise<Rows> {
  try {
    const q = new URLSearchParams({ mart, column, value, limit: "100" });
    const res = await apiFetch(`/api/dashboard/records?${q.toString()}`, { cache: "no-store" });
    const json = await res.json();
    if (!res.ok) throw new Error(json?.error ?? "Failed to load records");
    return { columns: json.columns ?? [], rows: json.rows ?? [] };
  } catch {
    return { columns: [], rows: [] };
  }
}

/** The menu at the cursor after clicking a data point: filter by it, or see its rows. */
export function DrillMenu({
  drill, onClose, onFilter, onRecords,
}: {
  readonly drill: DrillTarget;
  readonly onClose: () => void;
  /** Absent for built-in tiles: dashboard filters don't apply to them. */
  readonly onFilter?: () => void;
  readonly onRecords: () => void;
}) {
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const w = typeof window !== "undefined" ? window.innerWidth : 1200;
  const h = typeof window !== "undefined" ? window.innerHeight : 800;
  return (
    <>
      <div className="fixed inset-0 z-40" onClick={onClose} />
      <div
        role="menu"
        className="fixed z-50 w-60 rounded-lg border border-border bg-card p-1 shadow-xl"
        style={{ left: Math.min(drill.x, w - 250), top: Math.min(drill.y, h - 130) }}
      >
        <p className="truncate px-2 py-1 text-[11px] text-muted-foreground">
          {drill.column}: <span className="font-medium text-foreground">{drill.name}</span>
        </p>
        {onFilter ? (
          <button role="menuitem" onClick={onFilter} className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm hover:bg-muted">
            <Filter className="size-4" /> Filter dashboard by this
          </button>
        ) : null}
        <button role="menuitem" onClick={onRecords} className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm hover:bg-muted">
          <Table2 className="size-4" /> View records
        </button>
      </div>
    </>
  );
}

export function RecordsDialog({
  records, onClose,
}: {
  readonly records: RecordsState | null;
  readonly onClose: () => void;
}) {
  return (
    <Dialog open={!!records} onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent className="max-h-[85vh] overflow-hidden sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2"><Table2 className="size-4" /> Records · {records?.value}</DialogTitle>
        </DialogHeader>
        {records?.loading ? (
          <div className="h-40 animate-pulse rounded bg-muted/40" />
        ) : records && records.rows.length ? (
          <>
            <RowsTable data={records} />
            <p className="text-[11px] text-muted-foreground">Showing up to 100 rows.</p>
          </>
        ) : (
          <p className="py-8 text-center text-sm text-muted-foreground">No records found.</p>
        )}
      </DialogContent>
    </Dialog>
  );
}
