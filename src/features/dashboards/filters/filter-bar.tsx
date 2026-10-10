"use client";

import * as React from "react";
import { Calendar, ChevronDown, Hash, Plus, RotateCcw, Type, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import { filterLabel, isActiveFilter } from "@/lib/dashboard-filter-state";
import type { FilterDef, FilterField } from "@/services/clients/bi-store";
import { DateEditor, NumberEditor, TextEditor, blankFilter, type EditorProps } from "./filter-editors";

function KindIcon({ kind, className }: { kind: string; className?: string }) {
  const Icon = kind === "number" ? Hash : kind === "date" || kind === "datetime" ? Calendar : Type;
  return <Icon className={className} aria-hidden />;
}

function Editor({ kind, ...props }: EditorProps & { kind: string }) {
  if (kind === "number") return <NumberEditor {...props} />;
  if (kind === "date" || kind === "datetime") return <DateEditor {...props} />;
  return <TextEditor {...props} />;
}

/**
 * Cross-tile dashboard filters: one chip per active filter, an "Add filter"
 * menu over every column the board's tiles read, and Save as default / Reset
 * for the temporary state (BI-18). Changing a filter here only reports it
 * upward; the page decides what to do with it.
 */
export function FilterBar({
  board, fields, filters, onChange, dirty, canSaveDefault, saving, onSaveDefault, onReset,
}: {
  board: string;
  fields: FilterField[];
  filters: FilterDef[];
  onChange: (next: FilterDef[]) => void;
  /** The state differs from the saved default. */
  dirty: boolean;
  /** Caller has `dashboard:write` and the board is a user dashboard. */
  canSaveDefault: boolean;
  saving: boolean;
  onSaveDefault: () => void;
  onReset: () => void;
}) {
  const [addOpen, setAddOpen] = React.useState(false);
  const [picked, setPicked] = React.useState<FilterField | null>(null);
  const [query, setQuery] = React.useState("");
  const kindOf = (column: string) => fields.find((f) => f.column === column)?.kind ?? "text";
  const active = filters.filter(isActiveFilter);
  const used = new Set(filters.map((f) => f.column));
  const offered = fields.filter((f) => !used.has(f.column) && f.column.toLowerCase().includes(query.trim().toLowerCase()));
  const others = (column: string) => filters.filter((f) => f.column !== column && isActiveFilter(f));

  const replace = (old: FilterDef, next: FilterDef) => onChange(filters.map((f) => (f === old ? next : f)));
  const closeAdd = () => { setAddOpen(false); setPicked(null); setQuery(""); };

  return (
    <div className="flex flex-wrap items-center gap-2">
      {active.map((f) => (
        <FilterChip
          key={`${f.column}:${f.op ?? "in"}`}
          filter={f}
          kind={kindOf(f.column)}
          board={board}
          others={others(f.column)}
          onApply={(next) => replace(f, next)}
          onRemove={() => onChange(filters.filter((x) => x !== f))}
        />
      ))}

      <Popover open={addOpen} onOpenChange={(o) => { if (o) setAddOpen(true); else closeAdd(); }}>
        <PopoverTrigger asChild>
          <Button variant="outline" size="sm" disabled={fields.length === 0}>
            <Plus className="size-3.5" /> Add filter
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-72">
          {picked ? (
            <>
              <p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                <KindIcon kind={picked.kind} className="size-3.5" /> {picked.column}
              </p>
              <Editor
                kind={picked.kind}
                board={board}
                column={picked.column}
                others={others(picked.column)}
                initial={blankFilter(picked.column, picked.kind)}
                onApply={(next) => { onChange([...filters, next]); closeAdd(); }}
              />
            </>
          ) : (
            <>
              <Input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search columns" aria-label="Search columns" />
              <div className="max-h-64 overflow-y-auto">
                {offered.length === 0 ? <p className="px-2 py-2 text-xs text-muted-foreground">No more columns to filter on.</p> : null}
                {offered.map((f) => (
                  <button
                    key={f.column}
                    type="button"
                    onClick={() => setPicked(f)}
                    className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted"
                  >
                    <KindIcon kind={f.kind} className="size-3.5 shrink-0 text-muted-foreground" />
                    <span className="truncate">{f.column}</span>
                    <span className="ml-auto shrink-0 text-[11px] text-muted-foreground">{f.tiles} tile{f.tiles === 1 ? "" : "s"}</span>
                  </button>
                ))}
              </div>
            </>
          )}
        </PopoverContent>
      </Popover>

      {dirty ? (
        <span className="ml-auto flex items-center gap-1.5">
          <Button variant="ghost" size="sm" onClick={onReset}>
            <RotateCcw className="size-3.5" /> Reset
          </Button>
          {canSaveDefault ? (
            <Button size="sm" disabled={saving} onClick={onSaveDefault}>
              {saving ? "Saving…" : "Save as default"}
            </Button>
          ) : null}
        </span>
      ) : null}
    </div>
  );
}

function FilterChip({
  filter, kind, board, others, onApply, onRemove,
}: {
  filter: FilterDef;
  kind: string;
  board: string;
  others: FilterDef[];
  onApply: (next: FilterDef) => void;
  onRemove: () => void;
}) {
  const [open, setOpen] = React.useState(false);
  const label = filterLabel(filter);
  return (
    <div className="inline-flex h-7 items-center rounded-md border border-border bg-background text-foreground">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button variant="ghost" size="sm" className={cn("h-7 max-w-64 gap-1 rounded-r-none border-0 px-2 font-normal shadow-none")} title={label}>
            <KindIcon kind={kind} className="size-3.5 shrink-0 opacity-70" />
            <span className="truncate">{label}</span>
            <ChevronDown className="size-3.5 shrink-0 opacity-60" />
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-72">
          {open ? (
            <Editor
              kind={kind}
              board={board}
              column={filter.column}
              others={others}
              initial={filter}
              onApply={(next) => { onApply(next); setOpen(false); }}
            />
          ) : null}
        </PopoverContent>
      </Popover>
      <Button variant="ghost" size="icon-sm" className="size-7 rounded-l-none" aria-label={`Remove ${filter.column} filter`} onClick={onRemove}>
        <X className="size-3.5" />
      </Button>
    </div>
  );
}
