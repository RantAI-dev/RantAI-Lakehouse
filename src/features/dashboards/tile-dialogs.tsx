"use client";

import { Download, Table2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { downloadCsv, toCsv } from "@/lib/csv";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";
import { RowsTable } from "@/components/patterns/rows-table";
import { TileBody } from "./tile-body";

export type Rows = { columns: string[]; rows: Record<string, unknown>[] };
export type Cell = Rows | { error: string };

export function hasRows(c: Cell | undefined): c is Rows {
  return !!c && "rows" in c;
}

/** A tile's result as a CSV download, named after the chart. */
export function downloadRowsCsv(title: string, data: Rows): void {
  const rows = data.rows.map((r) =>
    Object.fromEntries(data.columns.map((c) => [c, r[c] == null ? "" : String(r[c])]))
  );
  const name = title.replace(/[^\w\s-]/g, "").trim().replace(/\s+/g, "-").toLowerCase() || "chart";
  downloadCsv(`${name}.csv`, toCsv(data.columns, rows));
}

/** The numbers behind one chart, with a CSV download. */
export function TileDataDialog({
  title, cell, onClose,
}: {
  readonly title: string;
  readonly cell: Cell | undefined;
  readonly onClose: () => void;
}) {
  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent className="max-h-[85vh] overflow-hidden sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2"><Table2 className="size-4" /> {title}</DialogTitle>
        </DialogHeader>
        {hasRows(cell) && cell.rows.length ? (
          <>
            <RowsTable columns={cell.columns} rows={cell.rows} />
            <div className="flex items-center justify-between">
              <p className="text-[11px] text-muted-foreground">{cell.rows.length} rows</p>
              <Button size="sm" variant="outline" onClick={() => downloadRowsCsv(title, cell)}>
                <Download className="size-4" /> Download CSV
              </Button>
            </div>
          </>
        ) : (
          <p className="py-8 text-center text-sm text-muted-foreground">
            {cell && "error" in cell ? `This chart failed to load: ${cell.error}` : "No data."}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}

/** One chart at a readable size. */
export function TileExpandDialog({
  spec, cell, dark, year, onClose,
}: {
  readonly spec: ChartRenderSpec & { text?: string; caption?: string };
  readonly cell: Cell | undefined;
  readonly dark: boolean;
  readonly year: string;
  readonly onClose: () => void;
}) {
  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent className="sm:max-w-5xl">
        <DialogHeader>
          <DialogTitle>{spec.title}</DialogTitle>
          {spec.subtitle ? <p className="text-sm text-muted-foreground">{spec.subtitle}</p> : null}
        </DialogHeader>
        <div className="h-[65vh]">
          <TileBody spec={spec} cell={cell} dark={dark} loading={false} year={year} />
        </div>
      </DialogContent>
    </Dialog>
  );
}
