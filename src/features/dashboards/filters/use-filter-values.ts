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

/**
 * The relation a chart's own filter reads its values from (`BI-47`, QA
 * `LH-BI-6-01`): the chart's mart (with its joins) or its saved SQL source.
 * `unsaved` is SQL that is new or edited and not saved yet: no list is offered
 * then, because the list would be read from the saved text and show the old
 * values (plan D10: unsupported, honestly).
 */
export type OwnSource = { mart: string; joins?: unknown[] } | { sqlSource: string } | { unsaved: true };

/** What an unsaved SQL source's list says instead of values (plan D10, D20). */
export const UNSAVED_SOURCE_SENTENCE = "Save the SQL source to list this column's values.";

/**
 * A saved source as the segment, metric and value-group pickers hold it
 * (`{mart}` or `{source: <SQL source id>}`), as an [`OwnSource`]. With no
 * source picked yet nothing can be listed, so it answers `unsaved` — no
 * request, and no list that belongs to another table.
 */
export function ownSourceOf(source: { mart?: string; source?: string } | null | undefined): OwnSource {
  if (source?.mart) return { mart: source.mart };
  if (source?.source) return { sqlSource: source.source };
  return { unsaved: true };
}

const SEARCH_DEBOUNCE_MS = 250;

/**
 * The values of `column` for a filter's value list. With `source` they come
 * from the chart's own relation and from nothing else (`BI-47` D8: the mart
 * with its joins, or the saved SQL source); without one, from the relations
 * the board's charts read (SQL sources included), as a dashboard filter does.
 * Either way the OTHER active filters narrow the list, and a debounced search
 * narrows it further. A newer request aborts the older one so a slow answer
 * can never overwrite a fresher list.
 */
export function useFilterValues(
  { board, column, others, search, source }: {
    board: string;
    column: string;
    others: FilterDef[];
    search: string;
    source?: OwnSource;
  },
): FilterValuesState {
  const [state, setState] = React.useState<FilterValuesState>({ values: [], truncated: false, loading: true, error: null });
  // The filters as text, so a new array with the same content is no reason to refetch.
  const othersKey = JSON.stringify(others.filter((f) => f.column !== column));
  // The source in plain text, so its own object identity is no reason either.
  const unsaved = source !== undefined && "unsaved" in source;
  const ownMart = source && "mart" in source ? source.mart : "";
  const ownJoins = source && "mart" in source && source.joins?.length ? JSON.stringify(source.joins) : "";
  const ownSqlSource = source && "sqlSource" in source ? source.sqlSource : "";

  React.useEffect(() => {
    if (unsaved) {
      setState({ values: [], truncated: false, loading: false, error: UNSAVED_SOURCE_SENTENCE });
      return;
    }
    const ctl = new AbortController();
    setState((s) => ({ ...s, loading: true, error: null }));
    const timer = setTimeout(async () => {
      try {
        const q = new URLSearchParams({ column });
        if (ownMart) {
          q.set("mart", ownMart);
          if (ownJoins) q.set("joins", ownJoins);
        } else if (ownSqlSource) {
          q.set("source", ownSqlSource);
        } else {
          q.set("board", board);
        }
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
  }, [board, column, othersKey, search, unsaved, ownMart, ownJoins, ownSqlSource]);

  return state;
}
