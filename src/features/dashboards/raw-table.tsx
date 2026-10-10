"use client";

import * as React from "react";
import { ArrowDown, ArrowUp, ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ErrorWithReference } from "@/components/error-reference";
import { useService } from "@/hooks/use-service";
import type { ColumnSetting, TableDefFields } from "@/lib/table-types";
import { cn } from "@/lib/utils";
import type { FilterDef } from "@/services/clients/bi-store";
import { SkippedFiltersMarker } from "./filters/skipped-marker";
import { cellClass, CellValue, columnStyle } from "./table-cell";
import type { Rows } from "./tile-dialogs";
import { fetchRecordsPage, hasNextPage, pageRange, splitReference, type RecordsPage, type RecordsRequest } from "./records";

type Sort = { column: string; dir: "asc" | "desc" };

/** Where a raw table reads its rows from, and what it lists. */
export type RawTableSpec = { mart: string; sqlSource?: string; title: string; def?: TableDefFields };

/** The sort the viewer chose, else the saved one. */
function effectiveSort(chosen: Sort | null, def: TableDefFields | undefined): Sort | null {
  if (chosen) return chosen;
  return def?.sortColumn ? { column: def.sortColumn, dir: def.sortDir === "desc" ? "desc" : "asc" } : null;
}

/**
 * A table of raw rows (`BI-16` part A). The dashboard carries the first page
 * and the total; later pages and another sort come from the records route
 * (`paging` says the filters to send with them). An embed or public link gives
 * no `paging`: it shows the first page and says how many rows there are.
 */
export function RawTable({
  spec, cell, paging,
}: {
  readonly spec: RawTableSpec;
  readonly cell: Rows;
  readonly paging?: { readonly filters: FilterDef[] };
}) {
  const def = spec.def;
  const settings: Record<string, ColumnSetting> = def?.columnSettings ?? {};
  const listed = def?.columns?.length ? def.columns : cell.columns;
  const shown = listed.filter((c) => !settings[c]?.hidden);
  const [offset, setOffset] = React.useState(0);
  const [chosen, setChosen] = React.useState<Sort | null>(null);
  const sort = effectiveSort(chosen, def);
  const canPage = !!paging && typeof cell.total === "number";
  // The payload's page is the first page in the saved order; anything else is fetched.
  const needsFetch = canPage && (offset > 0 || chosen !== null);
  const request: RecordsRequest = {
    title: spec.title, mart: spec.mart, sqlSource: spec.sqlSource, filters: paging?.filters ?? [],
    columns: shown, sortColumn: sort?.column, sortDir: sort?.dir,
  };
  const requestKey = JSON.stringify(request);
  const state = useService<RecordsPage | null>(
    (signal) => (needsFetch ? fetchRecordsPage(request, offset, signal) : Promise.resolve(null)),
    // `cell` changes whenever the dashboard reloads, which refreshes a page the viewer is on.
    [needsFetch, requestKey, offset, cell],
  );

  const fetched = needsFetch ? state.data : null;
  const rows = needsFetch ? fetched?.rows ?? [] : cell.rows;
  const total = fetched?.total ?? cell.total;
  const page: Pick<RecordsPage, "rows" | "total" | "offset"> | null =
    typeof total === "number" ? { rows, total, offset: needsFetch ? fetched?.offset ?? offset : 0 } : null;
  const failure = needsFetch && state.status === "error" ? splitReference(state.error.message) : null;
  const waiting = needsFetch && state.status === "loading";
  const limit = fetched?.limit ?? cell.limit ?? 50;

  function sortBy(column: string) {
    const current = effectiveSort(chosen, def);
    setOffset(0);
    setChosen({ column, dir: current?.column === column && current.dir === "asc" ? "desc" : "asc" });
  }

  return (
    <div className="flex h-full flex-col">
      <div className={cn("min-h-0 flex-1 overflow-auto", waiting && "opacity-60")}>
        <table className="w-full border-collapse text-xs">
          <thead className="sticky top-0 bg-card">
            <tr className="border-b border-border">
              {shown.map((c) => {
                const label = settings[c]?.label ?? c;
                const active = sort?.column === c;
                return (
                  <th key={c} style={columnStyle(settings[c])} className="px-2 py-1.5 text-left font-medium text-muted-foreground" aria-sort={active ? (sort?.dir === "desc" ? "descending" : "ascending") : undefined}>
                    {canPage ? (
                      <button type="button" onClick={() => sortBy(c)} className="inline-flex items-center gap-1 hover:text-foreground">
                        {label}
                        {active ? (sort?.dir === "desc" ? <ArrowDown className="size-3" aria-hidden /> : <ArrowUp className="size-3" aria-hidden />) : null}
                      </button>
                    ) : label}
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {failure ? (
              <tr><td colSpan={shown.length || 1} role="alert" className="px-2 py-6 text-center text-destructive"><ErrorWithReference message={failure.message} errorId={failure.errorId} /></td></tr>
            ) : rows.length === 0 && !waiting ? (
              <tr><td colSpan={shown.length || 1} className="px-2 py-6 text-center text-muted-foreground">No rows.</td></tr>
            ) : rows.map((r, i) => (
              <tr key={i} className="border-b border-border/40 last:border-0">
                {shown.map((c) => (
                  <td key={c} style={columnStyle(settings[c])} className={cellClass(settings[c])}><CellValue value={r[c]} setting={settings[c]} /></td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {page ? (
        <div className="flex shrink-0 items-center justify-between gap-2 border-t border-border/60 pt-1.5">
          <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
            {canPage ? pageRange(page) : `First ${page.rows.length.toLocaleString("en-US")} of ${page.total.toLocaleString("en-US")} rows`}
            {cell.filtersSkipped?.length ? <SkippedFiltersMarker skipped={cell.filtersSkipped} /> : null}
          </p>
          {canPage ? (
            <div className="flex gap-1">
              <Button size="xs" variant="outline" aria-label="Previous page" disabled={page.offset === 0 || waiting} onClick={() => setOffset(Math.max(0, page.offset - limit))}>
                <ChevronLeft className="size-3.5" />
              </Button>
              <Button size="xs" variant="outline" aria-label="Next page" disabled={!hasNextPage(page) || waiting} onClick={() => setOffset(page.offset + limit)}>
                <ChevronRight className="size-3.5" />
              </Button>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
