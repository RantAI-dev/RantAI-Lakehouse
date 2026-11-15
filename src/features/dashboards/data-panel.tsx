"use client";

/**
 * `BI-58` T2: the columns of the chart's source in the builder's data panel,
 * grouped as Dates, Text and Numbers, then one heading per joined table
 * (`BI-7`). A small chip names the column's use ("X axis", "Value",
 * "Split"). One click on a column that is not in use puts it where it fits
 * (`lib/column-click`), and any other click opens a small menu of the uses
 * the chart can take now. The panel holds no state of the chart: the builder
 * says what each column offers and applies the use it is given.
 */
import * as React from "react";
import { Calendar, Hash, Type, type LucideIcon } from "lucide-react";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { autoUse, menuUses, type ColumnGroup, type ColumnUse } from "@/lib/column-click";
import { cn } from "@/lib/utils";

export type PanelColumn = {
  name: string;
  group: ColumnGroup;
  /** The joined table's alias the column comes from, or null for the source's own. */
  table: string | null;
  /** The chip: "X axis", "Value", "Split" (a Gantt chart says "Task" and "Group"), or null. */
  role: string | null;
  offers: Readonly<Record<ColumnUse, boolean>>;
};

const GROUPS: readonly { group: ColumnGroup; title: string; icon: LucideIcon }[] = [
  { group: "date", title: "Dates", icon: Calendar },
  { group: "text", title: "Text", icon: Type },
  { group: "number", title: "Numbers", icon: Hash },
];
const ICON: Record<ColumnGroup, LucideIcon> = { date: Calendar, text: Type, number: Hash };
const ORDER: Record<ColumnGroup, number> = { date: 0, text: 1, number: 2 };

export function ColumnList({ columns, fieldLabel, textOf, xEmpty, valueEmpty, onUse }: {
  readonly columns: readonly PanelColumn[];
  /** The column as every picker shows it (its own name, a joined table or "fx" as a tag, a metric's label). */
  readonly fieldLabel: (name: string) => React.ReactNode;
  /** The column's readable text, for its accessible name. */
  readonly textOf: (name: string) => string;
  readonly xEmpty: boolean;
  readonly valueEmpty: boolean;
  readonly onUse: (name: string, use: ColumnUse) => void;
}) {
  const own = columns.filter((c) => !c.table);
  const tables = [...new Set(columns.flatMap((c) => (c.table ? [c.table] : [])))];
  const sections = [
    ...GROUPS.map((g) => ({ key: g.group, title: g.title, items: own.filter((c) => c.group === g.group) })),
    ...tables.map((t) => ({
      key: `table:${t}`, title: t,
      items: columns.filter((c) => c.table === t).sort((a, b) => ORDER[a.group] - ORDER[b.group]),
    })),
  ].filter((s) => s.items.length);
  if (!sections.length) return null;
  return (
    <div className="grid gap-4" data-slot="column-list">
      {sections.map((s) => (
        <div key={s.key} role="group" aria-label={s.title} className="grid min-w-0 gap-0.5">
          <p className="truncate px-2 pb-1 text-xs font-medium text-muted-foreground" title={s.title}>{s.title}</p>
          {s.items.map((c) => (
            <ColumnRow
              key={c.name} column={c} label={fieldLabel(c.name)} text={textOf(c.name)}
              click={{ group: c.group, offers: c.offers, xEmpty, valueEmpty, inUse: c.role !== null }} onUse={onUse}
            />
          ))}
        </div>
      ))}
    </div>
  );
}

function ColumnRow({ column, label, text, click, onUse }: {
  readonly column: PanelColumn;
  readonly label: React.ReactNode;
  readonly text: string;
  readonly click: Parameters<typeof autoUse>[0];
  readonly onUse: (name: string, use: ColumnUse) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const uses = menuUses(click);
  const auto = autoUse(click);
  const Icon = ICON[column.group];
  return (
    <DropdownMenu
      open={open}
      onOpenChange={(next) => {
        if (!next) { setOpen(false); return; }
        // One click on an unused column puts it where it fits; the menu opens only otherwise.
        if (auto) { onUse(column.name, auto); return; }
        if (uses.length) setOpen(true);
      }}
    >
      <DropdownMenuTrigger
        disabled={!auto && !uses.length}
        aria-label={column.role ? `${text}, ${column.role}` : text}
        className={cn(
          "group/col flex w-full min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none",
          "transition-[background-color,color] duration-200 ease-[cubic-bezier(0.2,0,0,1)] hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50 data-popup-open:bg-muted",
          "disabled:pointer-events-none disabled:opacity-50",
        )}
      >
        <Icon aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 font-medium">{label}</span>
        {column.role ? (
          <span className="shrink-0 rounded bg-primary/10 px-1.5 py-0.5 text-[11px] font-medium text-primary dark:text-sky-300">{column.role}</span>
        ) : (
          <span aria-hidden className="shrink-0 text-xs text-muted-foreground opacity-0 transition-opacity duration-200 group-hover/col:opacity-100 group-focus-visible/col:opacity-100">
            {auto ? "Add" : "Use"}
          </span>
        )}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-48">
        {uses.map((u) => (
          <DropdownMenuItem key={u.use} onClick={() => onUse(column.name, u.use)}>{u.label}</DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
