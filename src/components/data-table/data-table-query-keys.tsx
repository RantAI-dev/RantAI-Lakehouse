"use client";

import * as React from "react";

import type { QueryKeys, TablePaginationMode } from "@/types/data-table";

/** What a table's state is called in the URL when nothing renames it. */
export const DEFAULT_QUERY_KEYS: QueryKeys = {
  page: "page",
  perPage: "perPage",
  sort: "sort",
  filters: "filters",
  joinOperator: "joinOperator",
  search: "search",
  groupBy: "groupBy",
  view: "view",
};

interface DataTableQueryKeysContextValue {
  keys: QueryKeys;
  paginationMode: TablePaginationMode;
  /** Extra URL keys cleared by Reset (e.g. page-specific `memoryKeys`). */
  resetExtraKeys: string[];
}

const QueryKeysContext = React.createContext<DataTableQueryKeysContextValue>({
  keys: DEFAULT_QUERY_KEYS,
  paginationMode: "pages",
  resetExtraKeys: [],
});

/**
 * Carries a table's own param names down to the controls that write them.
 *
 * The search box and the reset button sit anywhere inside the toolbar and never
 * receive the table, so passing the names down by prop would mean threading
 * them through every page that renders a toolbar. The default keeps a control
 * used outside a toolbar working on the plain names.
 */
export function DataTableQueryKeysProvider({
  keys,
  paginationMode = "pages",
  resetExtraKeys = [],
  children,
}: {
  keys?: QueryKeys;
  paginationMode?: TablePaginationMode;
  resetExtraKeys?: string[];
  children: React.ReactNode;
}) {
  const value = React.useMemo(
    () => ({
      keys: keys ?? DEFAULT_QUERY_KEYS,
      paginationMode,
      resetExtraKeys,
    }),
    [keys, paginationMode, resetExtraKeys]
  );

  return (
    <QueryKeysContext.Provider value={value}>{children}</QueryKeysContext.Provider>
  );
}

export function useDataTableQueryKeys() {
  return React.useContext(QueryKeysContext);
}
