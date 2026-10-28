"use client";

import * as React from "react";

/** One editable filter as `POST /api/embed/data` describes it (`BI-26` plan D4). */
export type EmbedFilter = {
  column: string;
  kind?: string | null;
  /** The filter now applied to the column (the saved one, or the viewer's). */
  filter?: { values?: string[] } | null;
};

/** What the viewer chose, in the dashboard page's filter shape. */
export type ViewerFilter = { column: string; op: "in"; values: string[] };

/**
 * The controls of the filters a manager made editable (`BI-26` plan D4/D5): one
 * dropdown per filter, listing the values `POST /api/embed/values` allows for
 * this token, with "All" to clear it. A change is reported as the dashboard
 * page's filter shape, and the embed asks for its data again.
 */
export function EmbedFilterBar({
  jwt, filters, onChange,
}: {
  readonly jwt: string;
  readonly filters: readonly EmbedFilter[];
  readonly onChange: (chosen: ViewerFilter[]) => void;
}) {
  const [chosen, setChosen] = React.useState<Record<string, string>>(() =>
    Object.fromEntries(filters.map((f) => [f.column, f.filter?.values?.[0] ?? ""])),
  );
  const [options, setOptions] = React.useState<Record<string, string[]>>({});
  const [failed, setFailed] = React.useState(false);
  const columns = filters.map((f) => f.column).join("\u0000");

  React.useEffect(() => {
    let alive = true;
    (async () => {
      for (const f of filters) {
        try {
          // Deliberately NOT `apiFetch`: the viewer has no session, only the token.
          const res = await fetch("/api/embed/values", {
            method: "POST", headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ jwt, column: f.column }), cache: "no-store",
          });
          if (!alive) return;
          if (!res.ok) { setFailed(true); continue; }
          const json = await res.json();
          setOptions((cur) => ({ ...cur, [f.column]: Array.isArray(json?.values) ? json.values : [] }));
        } catch {
          if (alive) setFailed(true);
        }
      }
    })();
    return () => { alive = false; };
    // The list of columns is the dependency; `filters` is a new array on every payload.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jwt, columns]);

  function pick(column: string, value: string) {
    const next = { ...chosen, [column]: value };
    setChosen(next);
    onChange(filters.map((f) => ({ column: f.column, op: "in" as const, values: next[f.column] ? [next[f.column]] : [] })));
  }

  if (filters.length === 0) return null;
  return (
    <div className="mb-2 flex flex-wrap items-center gap-2" role="group" aria-label="Filters">
      {filters.map((f) => {
        const list = options[f.column] ?? [];
        const current = chosen[f.column] ?? "";
        return (
          <label key={f.column} className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <span>{f.column}</span>
            <select
              aria-label={f.column}
              value={current}
              onChange={(e) => pick(f.column, e.target.value)}
              className="h-7 rounded-md border border-border bg-background px-2 text-xs text-foreground"
            >
              <option value="">All</option>
              {current && !list.includes(current) ? <option value={current}>{current}</option> : null}
              {list.map((v) => <option key={v} value={v}>{v}</option>)}
            </select>
          </label>
        );
      })}
      {failed ? <span role="alert" className="text-xs text-destructive">Some filter values could not be loaded.</span> : null}
    </div>
  );
}
