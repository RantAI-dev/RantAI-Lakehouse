"use client";

import * as React from "react";
import { Calendar, CircleHelp, Hash, Plus, RotateCcw, Type, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import { filterLabel, filtersEqual, isActiveFilter, isRequiredColumn } from "@/lib/dashboard-filter-state";
import { withTies } from "@/lib/dashboard-tabs";
import type { BoardTab } from "@/services";
import type { FilterDef, FilterField } from "@/services/clients/bi-store";
import { DateEditor, NumberEditor, TextEditor, blankFilter, type EditorProps } from "./filter-editors";
import { ChipTrigger, chipShell } from "./chip";
import { EASE, FOCUS } from "../dashboard-header";

function KindIcon({ kind, className }: { kind: string; className?: string }) {
  const Icon = kind === "number" ? Hash : kind === "date" || kind === "datetime" ? Calendar : Type;
  return <Icon className={className} aria-hidden />;
}

const REQUIRED_HELP = "Viewers can change this filter's value but cannot remove it. Takes effect after Save as default.";

/** The "Required" row of an editor: a switch, and the one-line help in a tooltip rather than a paragraph. */
function RequiredRow({ column, checked, onChange }: { column: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <div className="mt-2 flex items-center justify-between gap-2 border-t border-border pt-2 text-xs text-muted-foreground">
      <span className="flex items-center gap-1">
        Required
        <Tooltip>
          <TooltipTrigger
            render={<button type="button" aria-label={REQUIRED_HELP} className="rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring" />}
          >
            <CircleHelp className="size-3.5" aria-hidden />
          </TooltipTrigger>
          <TooltipContent>{REQUIRED_HELP}</TooltipContent>
        </Tooltip>
      </span>
      <Switch size="sm" checked={checked} onCheckedChange={onChange} aria-label={`Require the ${column} filter`} />
    </div>
  );
}

/**
 * "Applies to": every tab, or the tabs ticked (BI-18 part C). A filter tied
 * to tabs is not given to the tiles of the others, and those tiles do not
 * report it as skipped. Takes effect with "Save as default" like `required`.
 */
function AppliesToRow({ tabs, value, onChange }: { tabs: BoardTab[]; value: string[] | undefined; onChange: (chosen: string[]) => void }) {
  const chosen = value ?? [];
  const toggle = (id: string) => onChange(chosen.includes(id) ? chosen.filter((c) => c !== id) : [...chosen, id]);
  return (
    <div className="mt-2 flex flex-wrap items-center gap-1 border-t border-border pt-2 text-xs text-muted-foreground">
      <span className="mr-1">Applies to</span>
      <button type="button" aria-pressed={chosen.length === 0}
              className={cn("rounded-full border px-2 py-0.5", chosen.length === 0 ? "border-primary bg-primary/10 text-foreground" : "border-border")}
              onClick={() => onChange([])}>All tabs</button>
      {tabs.map((t) => (
        <button key={t.id} type="button" aria-pressed={chosen.includes(t.id)}
                className={cn("max-w-32 truncate rounded-full border px-2 py-0.5", chosen.includes(t.id) ? "border-primary bg-primary/10 text-foreground" : "border-border")}
                onClick={() => toggle(t.id)}>{t.title}</button>
      ))}
    </div>
  );
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
 *
 * `BI-61` T3: the chips are borderless and tinted when they differ from the
 * saved default; "+ Filter" is offered where the page says (`canAdd`: in
 * edit mode). The pieces are laid out by the row they sit in (`FilterRow`).
 */
export function FilterBar({
  board, fields, filters, savedFilters, onChange, dirty, canSaveDefault, saving, onSaveDefault, onReset, tabs, canAdd = true,
}: {
  /** Whether "+ Filter" is offered (`BI-61` T3). Absent: offered. */
  canAdd?: boolean;
  board: string;
  /** The dashboard's tabs; with more than one, an editor can tie a filter to some of them. */
  tabs?: BoardTab[];
  fields: FilterField[];
  filters: FilterDef[];
  /** The board's saved default; its `required` flags decide which chips cannot be removed. */
  savedFilters: FilterDef[];
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
  // Required for a filter being added; it only becomes the default's with "Save as default".
  const [newRequired, setNewRequired] = React.useState(false);
  const [newTies, setNewTies] = React.useState<string[]>([]);
  const tieable = canSaveDefault && (tabs?.length ?? 0) > 1;
  const kindOf = (column: string) => fields.find((f) => f.column === column)?.kind ?? "text";
  const active = filters.filter(isActiveFilter);
  // Only filters that restrict something take their column out of the list.
  const used = new Set(active.map((f) => f.column));
  const offered = fields.filter((f) => !used.has(f.column) && f.column.toLowerCase().includes(query.trim().toLowerCase()));
  const others = (column: string) => filters.filter((f) => f.column !== column && isActiveFilter(f));

  // An editor builds a fresh filter; the `required` flag is not its to change
  // (BI-18 round two), so it is carried over from the filter it replaces.
  const replace = (old: FilterDef, next: FilterDef) =>
    onChange(filters.map((f) => (f === old ? { ...next, ...(old.required ? { required: true } : {}), ...(old.tabs?.length ? { tabs: old.tabs } : {}) } : f)));
  const setTies = (old: FilterDef, chosen: string[]) =>
    onChange(filters.map((f) => (f === old ? withTies(f, tabs ?? [], chosen) : f)));
  const setRequired = (old: FilterDef, required: boolean) =>
    onChange(filters.map((f) => (f === old ? { ...f, required: required || undefined } : f)));
  const closeAdd = () => { setAddOpen(false); setPicked(null); setQuery(""); setNewRequired(false); setNewTies([]); };

  const changedFrom = (f: FilterDef) => {
    const saved = savedFilters.find((x) => x.column === f.column && isActiveFilter(x));
    return !saved || !filtersEqual([f], [saved]);
  };

  return (
    <>
      {active.map((f) => (
        <FilterChip
          changed={changedFrom(f)}
          key={`${f.column}:${f.op ?? "in"}`}
          filter={f}
          kind={kindOf(f.column)}
          board={board}
          others={others(f.column)}
          onApply={(next) => replace(f, next)}
          // No remove control while the saved default requires this column.
          onRemove={isRequiredColumn(f.column, savedFilters) ? undefined : () => onChange(filters.filter((x) => x !== f))}
          onToggleRequired={canSaveDefault ? (required) => setRequired(f, required) : undefined}
          tabs={tieable ? tabs : undefined}
          onTies={(chosen) => setTies(f, chosen)}
        />
      ))}

      {canAdd ? (
      <Popover open={addOpen} onOpenChange={(o) => { if (o) setAddOpen(true); else closeAdd(); }}>
        <PopoverTrigger asChild>
          <button type="button" aria-label="Add filter" disabled={fields.length === 0}
            className={cn("inline-flex h-8 shrink-0 items-center gap-1 rounded-full px-3 text-sm font-medium text-primary hover:bg-primary/10 disabled:pointer-events-none disabled:opacity-50 dark:text-sky-300", EASE, FOCUS)}>
            <Plus className="size-3.5" aria-hidden /> Filter
          </button>
        </PopoverTrigger>
        <PopoverContent align="start" collisionPadding={16} className="w-72">
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
                onApply={(next) => {
                  const tied = withTies(newRequired ? { ...next, required: true } : next, tabs ?? [], newTies);
                  onChange([...filters, tied]);
                  closeAdd();
                }}
              />
              {canSaveDefault ? <RequiredRow column={picked.column} checked={newRequired} onChange={setNewRequired} /> : null}
              {tieable && tabs ? <AppliesToRow tabs={tabs} value={newTies} onChange={setNewTies} /> : null}
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
      ) : null}

      {/* One Reset for the whole row, shown only when something differs from the saved default. */}
      {dirty ? (
        <span className="flex shrink-0 items-center gap-1">
          <button type="button" onClick={onReset}
            className={cn("inline-flex h-8 shrink-0 items-center gap-1 rounded-full px-2.5 text-xs font-medium text-muted-foreground hover:bg-muted hover:text-foreground", EASE, FOCUS)}>
            <RotateCcw className="size-3" aria-hidden /> Reset
          </button>
          {canSaveDefault ? (
            <Button variant="ghost" size="sm" className="h-8 rounded-full px-3 text-primary hover:text-primary dark:text-sky-300" disabled={saving} onClick={onSaveDefault}>
              {saving ? "Saving…" : "Save as default"}
            </Button>
          ) : null}
        </span>
      ) : null}
    </>
  );
}

function FilterChip({
  filter, kind, board, others, onApply, onRemove, onToggleRequired, tabs, onTies, changed,
}: {
  filter: FilterDef;
  /** The filter differs from the saved default's on its column: the chip is tinted. */
  changed: boolean;
  kind: string;
  board: string;
  others: FilterDef[];
  onApply: (next: FilterDef) => void;
  /** Absent when the saved default requires the column. */
  onRemove?: () => void;
  /** Present for a caller who may save the default; the flag takes effect with "Save as default". */
  onToggleRequired?: (required: boolean) => void;
  /** Present for an editor of a dashboard with several tabs (BI-18 part C). */
  tabs?: BoardTab[];
  onTies: (chosen: string[]) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const label = filterLabel(filter, kind);
  const tone = changed ? "changed" : "default";
  return (
    <div data-slot="filter-chip" className={chipShell(tone)}>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <ChipTrigger value={label} tone={tone} title={label} className={cn("max-w-72", onRemove && "pr-1.5")} />
        </PopoverTrigger>
        <PopoverContent align="start" collisionPadding={16} className="w-72">
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
          {open && onToggleRequired ? (
            <RequiredRow column={filter.column} checked={filter.required === true} onChange={onToggleRequired} />
          ) : null}
          {open && tabs ? <AppliesToRow tabs={tabs} value={filter.tabs} onChange={onTies} /> : null}
        </PopoverContent>
      </Popover>
      {onRemove ? (
        <button type="button" aria-label={`Remove ${filter.column} filter`} onClick={onRemove}
          className={cn("mr-1 grid size-6 shrink-0 place-items-center rounded-full opacity-60 hover:bg-foreground/10 hover:opacity-100", EASE, FOCUS)}>
          <X className="size-3.5" aria-hidden />
        </button>
      ) : null}
    </div>
  );
}
