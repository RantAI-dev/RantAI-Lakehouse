"use client";

import * as React from "react";
import type { FilterDef } from "@/services/clients/bi-store";
import { apiFetch } from "@/services/http";

export type FilterValuesState = {
  values: string[];
  /** The list stopped at the server's cap; searching finds the rest. */
  truncated: boolean;
  loading: boolean;
  /** A fixed sentence; never the server's text. */
  error: string | null;
};

const SEARCH_DEBOUNCE_MS = 250;

/**
 * The values of `column` for a filter's value list: read from the relations
 * the board's charts read (SQL sources included), narrowed by the OTHER
 * active filters, with a debounced search. A newer request aborts the older
 * one so a slow answer can never overwrite a fresher list.
 */
export function useFilterValues(
  { board, column, others, search }: { board: string; column: string; others: FilterDef[]; search: string },
): FilterValuesState {
  const [state, setState] = React.useState<FilterValuesState>({ values: [], truncated: false, loading: true, error: null });
  // The filters as text, so a new array with the same content is no reason to refetch.
  const othersKey = JSON.stringify(others.filter((f) => f.column !== column));

  React.useEffect(() => {
    const ctl = new AbortController();
    setState((s) => ({ ...s, loading: true, error: null }));
    const timer = setTimeout(async () => {
      try {
        const q = new URLSearchParams({ column, board });
        if (search.trim()) q.set("q", search.trim());
        if (othersKey !== "[]") q.set("filters", othersKey);
        const res = await apiFetch(`/api/dashboard/values?${q.toString()}`, { cache: "no-store", signal: ctl.signal });
        if (!res.ok) throw new Error("values");
        const json = (await res.json()) as { values?: unknown[]; truncated?: boolean };
        if (ctl.signal.aborted) return;
        setState({
          values: (json.values ?? []).map(String),
          truncated: json.truncated === true,
          loading: false,
          error: null,
        });
      } catch {
        if (ctl.signal.aborted) return;
        setState({ values: [], truncated: false, loading: false, error: "Could not load values." });
      }
    }, search ? SEARCH_DEBOUNCE_MS : 0);
    return () => { clearTimeout(timer); ctl.abort(); };
  }, [board, column, othersKey, search]);

  return state;
}
