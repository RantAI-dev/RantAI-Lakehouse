"use client";

import * as React from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { bucketLabel, isGrain, isTruncation } from "@/lib/time-grain";
import { buildPivot, canCollapse, collapsibleGroups, columnLabel, groupId, visibleRows, type PivotRow } from "@/lib/pivot";
import type { ColumnSetting, TableDefFields } from "@/lib/table-types";
import { cn } from "@/lib/utils";
import { cellClass, CellValue } from "./table-cell";
import type { Rows } from "./tile-dialogs";

/** A value's header: its label setting, else `sum of visitors`. */
function valueLabel(def: TableDefFields, i: number): string {
  const v = def.values?.[i];
  if (!v) return "";
  return def.columnSettings?.[v.column]?.label ?? `${v.aggregate} of ${v.column}`;
}

/**
 * A pivot table (`BI-16` part A): sticky headers, the database's totals and
 * subtotals in place, and groups that fold. A folded group is view state, not
 * saved. Only a pivot with `all` totals can fold, because the folded group
 * stays in view as its subtotal. A result cut at the cell cap says so.
 */
export function PivotTable({ def, cell }: { readonly def: TableDefFields; readonly cell: Rows }) {
  const rowFields = def.rows ?? [];
  const colFields = def.columns ?? [];
  const valueCount = def.values?.length ?? 0;
  // BI-16A review fix (SHOULD-FIX) R3: the grained field's buckets read as BI-9
  // labels ("Q3 2025"). The tile does not carry which field the server grained
  // (the first date field of rows then columns), so it is the first whose body
  // keys all have a date's shape; only a truncation has one.
  const grain = isGrain(cell.grain) && isTruncation(cell.grain) ? cell.grain : null;
  const grainField = grain ? [...rowFields, ...colFields].find((f) => cell.rows.every((r) => r[f] == null || /^\d{4}-\d{2}-\d{2}/.test(String(r[f])) )) : undefined;
  const model = React.useMemo(
    () => buildPivot({ rows: cell.rows, rowFields, colFields, valueCount, labelFor: grain && grainField ? (f, key) => (f === grainField ? bucketLabel(grain, key) : null) : undefined }),
    // The field lists are read from the definition; the rows are what change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [cell.rows, rowFields.join("\u0001"), colFields.join("\u0001"), valueCount, grain, grainField],
  );
  const [collapsed, setCollapsed] = React.useState<ReadonlySet<string>>(new Set());
  const foldable = canCollapse(def.totals) ? collapsibleGroups(model) : new Set<string>();
  const rows = visibleRows(model, collapsed);
  if (model.rows.length === 0) {
    return <p className="grid h-full place-items-center text-xs text-muted-foreground">No data.</p>;
  }
  const settingOf = (i: number): ColumnSetting | undefined => {
    const column = def.values?.[i]?.column;
    return column ? def.columnSettings?.[column] : undefined;
  };
  const toggle = (id: string) => setCollapsed((prev) => {
    const next = new Set(prev);
    if (!next.delete(id)) next.add(id);
    return next;
  });
  const headRows = valueCount > 1 ? 2 : 1;
  // BI-16A review fix (BLOCKER) R1: every sticky cell is opaque (`bg-card`) and
  // stacked: header row z-20, the first row-field column z-10, their corner z-30,
  // so a label never draws over a value. Rows are a fixed `h-7` so the second
  // header row can stick at `top-7`.
  const stickyHead = "sticky h-7 bg-card px-2 py-0 text-left font-medium text-muted-foreground";

  return (
    <div className="flex h-full flex-col">
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-max min-w-full border-collapse text-xs">
          <thead>
            <tr className="border-b border-border">
              {rowFields.map((f, fi) => (
                <th key={f} rowSpan={headRows} className={cn(stickyHead, "top-0 min-w-28 align-bottom", fi === 0 ? "left-0 z-30 border-r border-border/60" : "z-20")}>{f}</th>
              ))}
              {model.columns.map((c, ci) => (
                <th key={ci} colSpan={Math.max(1, valueCount)} className={cn(stickyHead, "top-0 z-20 min-w-20 text-right", c.kind === "total" && "text-foreground")}>
                  {colFields.length === 0 && valueCount === 1 ? valueLabel(def, 0) : columnLabel(c)}
                </th>
              ))}
            </tr>
            {headRows === 2 ? (
              <tr className="border-b border-border">
                {model.columns.flatMap((c, ci) => Array.from({ length: valueCount }, (_, vi) => (
                  <th key={`${ci}-${vi}`} className={cn(stickyHead, "top-7 z-20 min-w-20 text-right font-normal")}>{valueLabel(def, vi)}</th>
                )))}
              </tr>
            ) : null}
          </thead>
          <tbody>
            {rows.map((row, ri) => (
              <tr key={groupId(row.path) + row.kind} className={cn("border-b border-border/40 last:border-0", row.kind !== "leaf" && "bg-muted font-medium")}>
                <RowHead row={row} rowFields={rowFields} previous={rows[ri - 1]} foldable={foldable} collapsed={collapsed} onToggle={toggle} />
                {row.cells.flatMap((values, ci) => values.map((n, vi) => (
                  <td key={`${ci}-${vi}`} className={cellClass(settingOf(vi), "text-right")}>
                    {n === null ? "" : <CellValue value={n} setting={settingOf(vi)} />}
                  </td>
                )))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {cell.truncated ? (
        <p className="shrink-0 border-t border-border/60 pt-1.5 text-[11px] text-muted-foreground">
          Cut off: only the first 10,000 cells are shown. Narrow the fields or add a filter.
        </p>
      ) : null}
    </div>
  );
}

/** The row's field cells: a group's label once, and a total labelled where its fields end. */
function RowHead({
  row, rowFields, previous, foldable, collapsed, onToggle,
}: {
  readonly row: PivotRow;
  readonly rowFields: readonly string[];
  readonly previous: PivotRow | undefined;
  readonly foldable: ReadonlySet<string>;
  readonly collapsed: ReadonlySet<string>;
  readonly onToggle: (id: string) => void;
}) {
  // BI-16A review fix (BLOCKER) R1: only the first row-field column sticks (several
  // sticking at `left-0` would draw over each other); it is opaque, as wide as
  // its label needs up to a limit, and above the values.
  const base = cn("px-2 py-1 text-left min-w-28 max-w-56 truncate", row.kind === "leaf" ? "bg-card" : "bg-muted");
  const stuck = "sticky left-0 z-10 border-r border-border/60";
  const cls = (i: number) => cn(base, i === 0 && stuck);
  if (row.kind === "grand") {
    return <th colSpan={Math.max(1, rowFields.length)} className={cls(0)}>Total</th>;
  }
  const depth = row.path.length;
  const id = groupId(row.path);
  return (
    <>
      {rowFields.map((f, i) => {
        if (i < depth) {
          // An outer label is written once per group, not on every row.
          const repeats = previous && previous.path.length > i && previous.path.slice(0, i + 1).join("\u0001") === row.path.slice(0, i + 1).join("\u0001");
          return <th key={f} title={row.labels[i]} className={cn(cls(i), "font-normal", row.kind === "subtotal" && "font-medium")}>{repeats ? "" : row.labels[i]}</th>;
        }
        if (i === depth && row.kind === "subtotal") {
          const folded = collapsed.has(id);
          return (
            <th key={f} colSpan={rowFields.length - depth} className={cls(i)}>
              {foldable.has(id) ? (
                <button type="button" aria-expanded={!folded} aria-label={`${folded ? "Expand" : "Collapse"} ${row.labels.join(" · ")}`} onClick={() => onToggle(id)} className="inline-flex items-center gap-1 text-muted-foreground hover:text-foreground">
                  {folded ? <ChevronRight className="size-3.5" aria-hidden /> : <ChevronDown className="size-3.5" aria-hidden />}
                  total
                </button>
              ) : "total"}
            </th>
          );
        }
        return row.kind === "subtotal" ? null : <th key={f} className={cls(i)} />;
      })}
    </>
  );
}
