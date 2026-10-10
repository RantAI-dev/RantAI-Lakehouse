"use client";

import * as React from "react";
import { Check } from "lucide-react";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import type { FilterDef } from "@/services/clients/bi-store";
import { useFilterValues } from "./use-filter-values";

/**
 * A searchable multi-select of a column's values, "is" / "is not". Used for
 * text columns and for a number column filtered by a list. Picking only
 * edits the draft; the editor's Apply commits it.
 */
export function ValueList({
  board, column, others, draft, onDraft,
}: {
  board: string;
  column: string;
  others: FilterDef[];
  draft: FilterDef;
  onDraft: (next: FilterDef) => void;
}) {
  const [search, setSearch] = React.useState("");
  const { values, truncated, loading, error } = useFilterValues({ board, column, others, search });
  const negate = draft.op === "not_in";
  const picked = new Set(draft.values);
  // A value picked earlier stays visible and unpickable-by-accident even
  // when the current search no longer lists it.
  const shown = [...draft.values.filter((v) => !values.includes(v) && !search), ...values];

  const toggle = (v: string) => {
    const next = picked.has(v) ? draft.values.filter((x) => x !== v) : [...draft.values, v];
    onDraft({ ...draft, values: next });
  };
  const setNegate = (not: boolean) => onDraft({ ...draft, op: not ? "not_in" : "in" });

  return (
    <div className="flex flex-col gap-2">
      <div className="inline-flex w-fit rounded-md border border-border p-0.5 text-xs" role="group" aria-label="Match mode">
        {([["is", false], ["is not", true]] as const).map(([label, not]) => (
          <button
            key={label}
            type="button"
            aria-pressed={negate === not}
            onClick={() => setNegate(not)}
            className={cn("rounded px-2 py-0.5", negate === not ? "bg-muted font-medium text-foreground" : "text-muted-foreground hover:text-foreground")}
          >
            {label}
          </button>
        ))}
      </div>
      <Input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Search values" aria-label="Search values" />
      <div className="max-h-56 overflow-y-auto" role="listbox" aria-multiselectable="true" aria-label={`Values of ${column}`}>
        {loading && shown.length === 0 ? <p className="px-2 py-2 text-xs text-muted-foreground">Loading…</p> : null}
        {error ? <p className="px-2 py-2 text-xs text-destructive">{error}</p> : null}
        {!loading && !error && shown.length === 0 ? <p className="px-2 py-2 text-xs text-muted-foreground">No values.</p> : null}
        {shown.map((v) => {
          const on = picked.has(v);
          return (
            <button
              key={v}
              type="button"
              role="option"
              aria-selected={on}
              onClick={() => toggle(v)}
              className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted"
            >
              <span className={cn("grid size-4 shrink-0 place-items-center rounded border", on ? "border-foreground bg-foreground text-background" : "border-border")}>
                {on ? <Check className="size-3" /> : null}
              </span>
              <span className="truncate">{v}</span>
            </button>
          );
        })}
      </div>
      {truncated ? <p className="px-1 text-[11px] text-muted-foreground">Showing the first 200, search to find more.</p> : null}
    </div>
  );
}
