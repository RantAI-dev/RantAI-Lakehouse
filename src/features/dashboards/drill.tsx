"use client";

import * as React from "react";
import { ChevronLeft, ChevronRight, Filter, Table2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ErrorWithReference } from "@/components/error-reference";
import { RowsTable } from "@/components/patterns/rows-table";
import { useService } from "@/hooks/use-service";
import { SkippedFiltersMarker } from "./filters/skipped-marker";
import { fetchRecordsPage, hasNextPage, pageRange, splitReference, type RecordsRequest } from "./records";

export type DrillTarget = {
  name: string; column: string; mart: string; sqlSource?: string; x: number; y: number;
  /** A grouped chart's bucket as the person read it ("Mar 2026"); `name` then holds the stored bucket (BI-9). */
  label?: string;
  /** The grain of `name`, when it is a bucket (BI-9). */
  grain?: string;
};

/** The menu at the cursor after clicking a data point: filter by it, or see its rows. */
export function DrillMenu({
  drill, onClose, onFilter, onRecords,
}: {
  readonly drill: DrillTarget;
  readonly onClose: () => void;
  /** Absent for built-in tiles: dashboard filters don't apply to them. */
  readonly onFilter?: () => void;
  /** Absent when the rows cannot be listed. */
  readonly onRecords?: () => void;
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
          {drill.column}: <span className="font-medium text-foreground">{drill.label ?? drill.name}</span>
        </p>
        {onFilter ? (
          <button role="menuitem" onClick={onFilter} className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm hover:bg-muted">
            <Filter className="size-4" /> Filter dashboard by this
          </button>
        ) : null}
        {onRecords ? (
          <button role="menuitem" onClick={onRecords} className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm hover:bg-muted">
            <Table2 className="size-4" /> View records
          </button>
        ) : null}
      </div>
    </>
  );
}

/**
 * The rows behind a clicked value or a whole tile, a page at a time. It
 * loads its own pages and starts on the first page each time it opens.
 */
export function RecordsDialog({
  request, onClose,
}: {
  readonly request: RecordsRequest | null;
  readonly onClose: () => void;
}) {
  return (
    <Dialog open={!!request} onOpenChange={(o) => { if (!o) onClose(); }}>
      {request ? <RecordsContent request={request} /> : null}
    </Dialog>
  );
}

function RecordsContent({ request }: { readonly request: RecordsRequest }) {
  const [offset, setOffset] = React.useState(0);
  const state = useService((signal) => fetchRecordsPage(request, offset, signal), [request, offset]);
  const page = state.data;
  const failure = state.status === "error" ? splitReference(state.error.message) : null;
  return (
    <DialogContent className="max-h-[85vh] overflow-hidden sm:max-w-3xl">
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2"><Table2 className="size-4" /> Records · {request.title}</DialogTitle>
      </DialogHeader>
      {state.status === "loading" ? (
        <div className="h-40 animate-pulse rounded bg-muted/40" />
      ) : failure ? (
        <p role="alert" className="py-8 text-center text-sm text-destructive">
          <ErrorWithReference message={failure.message} errorId={failure.errorId} />
        </p>
      ) : page && page.rows.length ? (
        <RowsTable columns={page.columns} rows={page.rows} />
      ) : (
        <p className="py-8 text-center text-sm text-muted-foreground">No records found.</p>
      )}
      {page ? (
        <div className="flex items-center justify-between gap-2">
          <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
            {pageRange(page)}
            {page.filtersSkipped.length ? <SkippedFiltersMarker skipped={page.filtersSkipped} /> : null}
          </p>
          <div className="flex gap-1.5">
            <Button size="sm" variant="outline" disabled={page.offset === 0} onClick={() => setOffset(Math.max(0, page.offset - page.limit))}>
              <ChevronLeft className="size-4" /> Previous
            </Button>
            <Button size="sm" variant="outline" disabled={!hasNextPage(page)} onClick={() => setOffset(page.offset + page.limit)}>
              Next <ChevronRight className="size-4" />
            </Button>
          </div>
        </div>
      ) : null}
    </DialogContent>
  );
}
