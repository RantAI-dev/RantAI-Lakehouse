"use client";

import { Download, Table2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { downloadCsv, toCsv } from "@/lib/csv";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";
import { RowsTable } from "@/components/patterns/rows-table";
import type { FilterSkip } from "@/services/clients/bi-store";
import { visibleColumns } from "@/lib/chart-shape";
import { TileBody } from "./tile-body";
import type { TileFailure } from "@/services/contracts/dashboards";
import type { BinMeta } from "@/lib/chart-bins";
import type { LayerCell } from "./combined-option";

export type Rows = {
  columns: string[];
  rows: Record<string, unknown>[];
  /** Active filters this tile's data could not honour (BI-18). */
  filtersSkipped?: FilterSkip[];
  /** The grain the rows were bucketed with, when the chart has one (BI-9). */
  grain?: string;
  /** The kind of the grouped dimension (`date` or `datetime`), so the dashboard's switch knows whether hour and minute fit (BI-9). */
  grainColumn?: string;
  /** The bins of a binned chart, by column: the width and the data's range, read over the same rows as the bins (BI-10). */
  bins?: Record<string, BinMeta>;
  /** A dashboard grain switch this chart could not take; it kept its own (BI-9). */
  grainSkipped?: string;
  /** More buckets existed than the chart's limit; the latest are shown (BI-9). A pivot: more cells than it may return, so the first are shown (BI-16). */
  truncated?: boolean;
  /** A raw table's first page: the rows its pages walk through, the page size and the offset. Absent when the count could not be read (BI-16). */
  total?: number;
  limit?: number;
  offset?: number;
  /** Lines on the value axis, at the values this read gave their parameters (BI-11). */
  referenceLines?: { value: number; label?: string }[];
  /** The chart's other questions, each answered on its own (BI-13). */
  layers?: LayerCell[];
};
export type Cell = Rows | TileFailure;

export function hasRows(c: Cell | undefined): c is Rows {
  return !!c && "rows" in c;
}

/** A tile's result as a CSV download, named after the chart. */
export function downloadRowsCsv(title: string, data: Rows): void {
  // BI-6 R1: the server's internal columns (`__other`, ...) are not in the file.
  const columns = visibleColumns(data.columns);
  const rows = data.rows.map((r) =>
    Object.fromEntries(columns.map((c) => [c, r[c] == null ? "" : String(r[c])]))
  );
  const name = title.replace(/[^\w\s-]/g, "").trim().replace(/\s+/g, "-").toLowerCase() || "chart";
  downloadCsv(`${name}.csv`, toCsv(columns, rows));
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
            <RowsTable columns={visibleColumns(cell.columns)} rows={cell.rows} />
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
  spec, cell, dark, onClose,
}: {
  readonly spec: ChartRenderSpec & { text?: string; caption?: string };
  readonly cell: Cell | undefined;
  readonly dark: boolean;
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
          <TileBody spec={spec} cell={cell} dark={dark} loading={false} />
        </div>
      </DialogContent>
    </Dialog>
  );
}
