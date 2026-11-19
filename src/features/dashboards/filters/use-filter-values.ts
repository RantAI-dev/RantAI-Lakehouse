"use client";

import * as React from "react";
import { useService } from "@/hooks/use-service";
import { dashboardService, type FilterDef, type FilterValuesScope } from "@/services";

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
 *
 * `BI-29` D7: read through `dashboardService.filterValues` and `useService`
 * (the abort-on-newer-request is the hook's), not a hand-rolled effect that
 * called `apiFetch` itself.
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
  // The filters as text, so a new array with the same content is no reason to refetch.
  const othersKey = JSON.stringify(others.filter((f) => f.column !== column));
  // The source in plain text, so its own object identity is no reason either.
  const unsaved = source !== undefined && "unsaved" in source;
  const ownMart = source && "mart" in source ? source.mart : "";
  const ownJoins = source && "mart" in source && source.joins?.length ? JSON.stringify(source.joins) : "";
  const ownSqlSource = source && "sqlSource" in source ? source.sqlSource : "";

  // A typed search waits for a pause before it asks; clearing it asks at once.
  const [asked, setAsked] = React.useState(search);
  React.useEffect(() => {
    if (search === asked) return;
    const timer = setTimeout(() => setAsked(search), search ? SEARCH_DEBOUNCE_MS : 0);
    return () => clearTimeout(timer);
  }, [search, asked]);

  const result = useService(async (signal) => {
    // No request for an unsaved source: its list would be the old text's values.
    if (unsaved) return { values: [] as string[], truncated: false };
    const scope: FilterValuesScope = ownMart
      ? { mart: ownMart, ...(ownJoins ? { joins: JSON.parse(ownJoins) as unknown[] } : {}) }
      : ownSqlSource ? { sqlSource: ownSqlSource } : { board };
    return dashboardService.filterValues(
      { column, scope, others: JSON.parse(othersKey) as FilterDef[], search: asked },
      signal,
    );
  }, [board, column, othersKey, asked, unsaved, ownMart, ownJoins, ownSqlSource]);

  // The last list stays on screen while a newer one loads, as the list did
  // before this read went through `useService`, so typing does not flash it.
  const [last, setLast] = React.useState<{ values: string[]; truncated: boolean }>({ values: [], truncated: false });
  React.useEffect(() => {
    if (result.status === "success") setLast(result.data);
  }, [result]);

  if (unsaved) return { values: [], truncated: false, loading: false, error: UNSAVED_SOURCE_SENTENCE };
  if (result.status === "error") return { values: [], truncated: false, loading: false, error: "Could not load values." };
  if (result.status === "loading" || search !== asked) {
    return { ...last, loading: true, error: null };
  }
  return { values: result.data.values, truncated: result.data.truncated, loading: false, error: null };
}
